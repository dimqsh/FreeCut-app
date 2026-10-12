//! The feeder, ring and clock driven by hand through a fake device.

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;

use super::*;
use crate::source::MixInput;
use crate::testing::{List, Opener, Wave, input, secs};

type Slot<T> = Arc<Mutex<Option<T>>>;

/// How a named fake device fails to open.
#[derive(Debug, Clone, Copy)]
enum Fail {
    Busy,
    Denied,
    /// A generic device error (or a timeout).
    Error,
}

/// A named device of the fake system.
#[derive(Debug, Clone, Copy)]
struct Named {
    rate: u32,
    fail: Fail,
    /// How many more opens fail.
    failures: usize,
}

impl Named {
    fn working(rate: u32) -> Named {
        Named {
            rate,
            fail: Fail::Error,
            failures: 0,
        }
    }

    fn failing(fail: Fail, failures: usize) -> Named {
        Named {
            rate: 48_000,
            fail,
            failures,
        }
    }
}

/// Every open attempt: the device asked for and the buffer size.
type OpenLog = Arc<Mutex<Vec<(Option<String>, u32)>>>;

/// A device whose callback the test calls: the system default (`rate`,
/// `fail`) and any named devices in `named`.
#[derive(Clone)]
struct FakeDevice {
    tap: Slot<OutputTap>,
    lost: Slot<LostHandler>,
    rate: Arc<AtomicU32>,
    fail: Arc<AtomicBool>,
    opens: Arc<AtomicUsize>,
    named: Arc<Mutex<HashMap<String, Named>>>,
    log: OpenLog,
}

impl FakeDevice {
    fn new() -> FakeDevice {
        FakeDevice {
            tap: Arc::default(),
            lost: Arc::default(),
            rate: Arc::new(AtomicU32::new(48_000)),
            fail: Arc::default(),
            opens: Arc::default(),
            named: Arc::default(),
            log: Arc::default(),
        }
    }

    fn with(self, name: &str, device: Named) -> FakeDevice {
        self.named.lock().unwrap().insert(name.to_owned(), device);
        self
    }

    fn unplug(&self, reason: &str) {
        let lost = self.lost.lock().unwrap();
        (lost.as_ref().unwrap())(reason.to_owned());
    }

    fn last_open(&self) -> (Option<String>, u32) {
        self.log.lock().unwrap().last().cloned().unwrap()
    }
}

impl Backend for FakeDevice {
    fn open(
        &mut self,
        device: Option<&str>,
        tap: OutputTap,
        buffer_frames: u32,
        on_lost: LostHandler,
    ) -> Result<DeviceInfo, OutputError> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.log
            .lock()
            .unwrap()
            .push((device.map(str::to_owned), buffer_frames));
        let (hz, name) = match device {
            None => {
                if self.fail.load(Ordering::SeqCst) {
                    return Err(OutputError::NoDevice);
                }
                let hz = self.rate.load(Ordering::SeqCst);
                (hz, format!("Fake {hz}"))
            }
            Some(name) => {
                let mut named = self.named.lock().unwrap();
                let Some(d) = named.get_mut(name) else {
                    return Err(OutputError::DeviceNotFound(name.to_owned()));
                };
                if d.failures > 0 {
                    d.failures -= 1;
                    return Err(match d.fail {
                        Fail::Busy => OutputError::Busy("held".to_owned()),
                        Fail::Denied => OutputError::Denied("no".to_owned()),
                        Fail::Error => OutputError::Device("timed out".to_owned()),
                    });
                }
                (d.rate, name.to_owned())
            }
        };
        *self.tap.lock().unwrap() = Some(tap);
        *self.lost.lock().unwrap() = Some(on_lost);
        Ok(DeviceInfo {
            rate: SampleRate::new(hz).unwrap(),
            name,
            buffer_frames: Some(buffer_frames),
        })
    }

    fn close(&mut self) {
        *self.tap.lock().unwrap() = None;
    }
}

struct Rig {
    engine: Engine,
    device: FakeDevice,
    /// The fake device clock.
    now: Instant,
}

impl Rig {
    fn new() -> Rig {
        Rig::on(FakeDevice::new(), EngineConfig::default())
    }

    fn on(device: FakeDevice, config: EngineConfig) -> Rig {
        let engine = Engine::manual(Box::new(device.clone()), config).unwrap();
        Rig {
            engine,
            device,
            now: Instant::now(),
        }
    }

    /// A ramp source on the whole first minute: sample `k` of the timeline
    /// plays `k * 1e-6` (left) and its negation (right).
    fn with_ramp() -> Rig {
        let rig = Rig::new();
        rig.set_wave(Wave::Counter, 1e-6);
        rig
    }

    fn set_wave(&self, wave: Wave, gain: f32) {
        let mut i = input(1, 1, Tick::ZERO, secs(60.0), Tick::ZERO);
        i.gain = gain.into();
        self.engine.set_source(
            Arc::new(List(vec![i])),
            Arc::new(Opener::with(&[(1, wave)])),
        );
    }

    /// Runs the feeder, then one device callback of `frames` stereo frames
    /// at the fake clock, which then advances by the buffer's duration.
    fn pull(&mut self, frames: usize) -> Vec<f32> {
        self.engine.pump();
        let mut out = vec![f32::NAN; frames * 2];
        let mut tap = self.device.tap.lock().unwrap();
        tap.as_mut()
            .unwrap()
            .fill(&mut out, 2, Duration::ZERO, self.now);
        let hz = u64::from(self.engine.sample_rate().hz());
        self.now += Duration::from_nanos(frames as u64 * 1_000_000_000 / hz);
        out
    }

    fn position(&self) -> Option<Tick> {
        self.engine.position_at(self.now)
    }
}

fn ramp(k: i64) -> f32 {
    k as f32 * 1e-6
}

fn samples(n: i64) -> Tick {
    Tick::from_raw(n * 14_700)
}

#[test]
fn plays_from_the_requested_time() {
    let mut rig = Rig::with_ramp();
    assert_eq!(rig.engine.device_name(), "Fake 48000");
    assert_eq!(rig.engine.sample_rate().hz(), 48_000);
    assert_eq!(rig.position(), None);
    rig.engine.play(secs(1.0));
    assert!(rig.engine.is_playing());
    assert_eq!(rig.position(), Some(secs(1.0)));
    let out = rig.pull(480);
    for i in 0..480 {
        assert_eq!(out[2 * i], ramp(48_000 + i as i64), "frame {i}");
        assert_eq!(out[2 * i + 1], -ramp(48_000 + i as i64));
    }
    assert_eq!(rig.engine.underruns(), 0);
}

#[test]
fn position_follows_the_device_clock() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(secs(1.0));
    let start = rig.now;
    rig.pull(480);
    // Right after the callback, its first frame is being heard.
    assert_eq!(rig.engine.position_at(start), Some(secs(1.0)));
    // Half-way through the buffer.
    let mid = start + Duration::from_millis(5);
    assert_eq!(
        rig.engine.position_at(mid),
        Some(secs(1.0).checked_add(samples(240)).unwrap())
    );
    // Never beyond what was handed to the device, and never backwards.
    let late = start + Duration::from_secs(5);
    assert_eq!(
        rig.engine.position_at(late),
        Some(secs(1.0).checked_add(samples(480)).unwrap())
    );
    assert_eq!(
        rig.engine.position_at(mid),
        Some(secs(1.0).checked_add(samples(480)).unwrap())
    );
    // The next callback continues the count exactly.
    let second = rig.now;
    rig.pull(480);
    assert_eq!(
        rig.engine.position_at(second),
        Some(secs(1.0).checked_add(samples(480)).unwrap())
    );
}

#[test]
fn output_latency_is_compensated() {
    let rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.engine.pump();
    let at = rig.now;
    let mut out = vec![0.0; 960];
    rig.device.tap.lock().unwrap().as_mut().unwrap().fill(
        &mut out,
        2,
        Duration::from_millis(20),
        at,
    );
    // 20 ms of latency: nothing of this buffer is heard yet.
    assert_eq!(
        rig.engine.position_at(at + Duration::from_millis(10)),
        Some(Tick::ZERO)
    );
    assert_eq!(
        rig.engine.position_at(at + Duration::from_millis(25)),
        Some(samples(240))
    );
}

#[test]
fn seek_flushes_buffered_audio() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.engine.seek(secs(5.0));
    assert_eq!(rig.position(), Some(secs(5.0)));
    let out = rig.pull(480);
    assert_eq!(out[0], ramp(240_000));
    assert_eq!(out[958], ramp(240_000 + 479));
    assert_eq!(rig.engine.underruns(), 0);
}

#[test]
fn stale_frames_are_dropped_even_before_the_feeder_catches_up() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    // Seek without running the feeder: the callback must not play the old
    // read-ahead, and the wait for new frames is not an underrun.
    rig.engine.seek(secs(5.0));
    let mut out = vec![1.0; 960];
    let now = rig.now;
    rig.device
        .tap
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fill(&mut out, 2, Duration::ZERO, now);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.underruns(), 0);
    assert_eq!(rig.pull(480)[0], ramp(240_000));
}

#[test]
fn pause_silences_and_stops_the_clock() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.engine.pause();
    assert!(!rig.engine.is_playing());
    assert_eq!(rig.position(), None);
    let out = rig.pull(4800);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.underruns(), 0);
    rig.engine.seek(secs(2.0));
    assert_eq!(rig.position(), None);
    rig.engine.play(secs(2.0));
    assert_eq!(rig.pull(10)[0], ramp(96_000));
}

#[test]
fn set_source_during_playback_keeps_queued_audio_and_switches_after_the_read_ahead() {
    let mut rig = Rig::new();
    rig.set_wave(Wave::Dc(0.5), 1.0);
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.pull(480);
    // A new document (and a new opener): nothing restarts.
    rig.set_wave(Wave::Dc(0.25), 1.0);
    assert_eq!(rig.position(), Some(samples(960)));
    let mut heard = Vec::new();
    while heard.len() < 24_000 {
        heard.extend(rig.pull(480).iter().step_by(2).copied());
    }
    // What was queued plays out: no silence, no repeat, no ramp.
    assert_eq!(heard[0], 0.5);
    assert!(heard.iter().all(|&v| v == 0.5 || v == 0.25), "a gap");
    let switch = heard.iter().position(|&v| v == 0.25).unwrap();
    assert!(heard[switch..].iter().all(|&v| v == 0.25));
    // Heard within about 250 ms of the change.
    assert!(switch <= 12_000, "switched after {switch} frames");
    assert_eq!(rig.position(), Some(samples(960 + 24_000)));
    assert_eq!(rig.engine.underruns(), 0);
}

/// A dc source on `[0, 60 s)` at `gain` through `opener`.
fn dc_input(key: u64, asset: u64, start: f64, gain: f32) -> MixInput {
    let mut i = input(key, asset, secs(start), secs(60.0), Tick::ZERO);
    i.gain = gain.into();
    i
}

#[test]
fn edits_committed_every_frame_during_playback_never_silence_it() {
    // A volume drag commits on every UI frame (60 Hz or more; here every
    // 10 ms device buffer).
    let mut rig = Rig::new();
    let opener = Arc::new(Opener::with(&[(1, Wave::Dc(0.5))]));
    let source = |gain| {
        let schedule: Arc<dyn Schedule> = Arc::new(List(vec![dc_input(1, 1, 0.0, gain)]));
        schedule
    };
    rig.engine.set_source(source(1.0), opener.clone());
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    let mut pulled = 480;
    for k in 0..200 {
        let gain = if k % 2 == 0 { 0.5 } else { 1.0 };
        rig.engine.set_source(source(gain), opener.clone());
        let out = rig.pull(480);
        assert!(out.iter().all(|&v| v > 0.0), "silence after commit {k}");
        pulled += 480;
    }
    // The clock ran with the audio.
    assert_eq!(rig.position(), Some(samples(pulled)));
    assert_eq!(rig.engine.underruns(), 0);
    // One reader, read sequentially throughout: a commit never seeks back.
    assert_eq!(opener.opens(), 1);
    let reads = opener.reads();
    for w in reads[1..].windows(2) {
        let (_, at, frames) = w[0];
        assert_eq!(w[1].1.raw(), at.raw() + frames as i64 * 14_700);
    }
}

#[test]
fn the_feeder_never_opens_a_source_itself() {
    let mut rig = Rig::new();
    let opener = Arc::new(Opener::with(&[(1, Wave::Dc(0.5)), (2, Wave::Dc(0.25))]));
    let a = dc_input(1, 1, 0.0, 1.0);
    rig.engine
        .set_source(Arc::new(List(vec![a.clone()])), opener.clone());
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    assert_eq!(opener.opens(), 1);
    // A clip at 0.3 s is added while its file is slow to open.
    rig.engine.hold_prefetch(true);
    let b = dc_input(2, 2, 0.3, 1.0);
    rig.engine
        .set_source(Arc::new(List(vec![a, b])), opener.clone());
    for _ in 0..60 {
        let out = rig.pull(480);
        assert!(out.iter().all(|&v| v == 0.5), "the rest keeps playing");
    }
    assert_eq!(opener.opens(), 1, "nothing opened on the feeder");
    assert_eq!(rig.engine.underruns(), 0);
    // Once open, it joins (ramping in) after the read-ahead.
    rig.engine.hold_prefetch(false);
    let mut heard = Vec::new();
    for _ in 0..40 {
        heard.extend(rig.pull(480).iter().step_by(2).copied());
    }
    assert!(heard.iter().all(|&v| (0.5..=0.75).contains(&v)));
    assert_eq!(*heard.last().unwrap(), 0.75);
    assert_eq!(opener.opens(), 2);
}

#[test]
fn a_reopened_device_waiting_for_audio_counts_underruns() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.device.unplug("device unplugged");
    // The new device calls back before the feeder rendered anything for
    // it: that gap in playback is an underrun (only play and seek wait for
    // free).
    rig.engine.pump_commands();
    let mut out = vec![1.0; 960];
    let now = rig.now;
    rig.device
        .tap
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fill(&mut out, 2, Duration::ZERO, now);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.underruns(), 1);
    // And playback continues right after the frames already played.
    assert_eq!(rig.pull(1)[0], ramp(480));
}

#[test]
fn every_callback_publishes_the_clock_even_while_it_is_read() {
    let rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.engine.pump();
    let shared = rig.device.tap.lock().unwrap().as_ref().unwrap().shared();
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                let _ = rig.engine.position();
            }
        });
        let mut handed = 0;
        let mut out = [0.0f32; 8];
        for _ in 0..20_000 {
            let mut guard = rig.device.tap.lock().unwrap();
            let Some(tap) = guard.as_mut().filter(|t| t.ring.slots() >= 4) else {
                drop(guard);
                rig.engine.pump();
                continue;
            };
            tap.fill(&mut out, 2, Duration::ZERO, Instant::now());
            drop(guard);
            handed += 4;
            assert_eq!(
                shared.clock.load().frames,
                handed,
                "a callback went unpublished"
            );
        }
        stop.store(true, Ordering::Relaxed);
    });
}

#[test]
fn a_slow_source_open_does_not_starve_the_device() {
    use std::sync::Condvar;

    /// Opens asset 2 only once the gate opens.
    struct Gated {
        inner: Opener,
        gate: Arc<(Mutex<bool>, Condvar)>,
    }
    impl SourceOpener for Gated {
        fn open(
            &self,
            source: &crate::source::SourceKey,
            rate: SampleRate,
        ) -> Option<Box<dyn crate::source::SourceReader>> {
            if source.asset == 2 {
                let (open, cv) = &*self.gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = cv.wait(open).unwrap();
                }
            }
            self.inner.open(source, rate)
        }
    }

    let device = FakeDevice::new();
    let engine = Engine::start(Box::new(device.clone()), EngineConfig::default()).unwrap();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let opener = Gated {
        inner: Opener::with(&[(1, Wave::Dc(0.5)), (2, Wave::Dc(0.25))]),
        gate: gate.clone(),
    };
    let schedule = List(vec![dc_input(1, 1, 0.0, 1.0), dc_input(2, 2, 0.3, 1.0)]);
    engine.set_source(Arc::new(schedule), Arc::new(opener));
    engine.play(Tick::ZERO);
    // Pull a whole second while asset 2 can't open; the feeder must keep
    // rendering clip 1 through the point where clip 2 starts.
    let started = Instant::now();
    let mut sounding = 0;
    let mut out = vec![0.0f32; 2048];
    while sounding < 48_000 {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the feeder stalled after {sounding} frames"
        );
        device.tap.lock().unwrap().as_mut().unwrap().fill(
            &mut out,
            2,
            Duration::ZERO,
            Instant::now(),
        );
        let got = out.iter().step_by(2).filter(|&&v| v != 0.0).count();
        if got == 0 {
            std::thread::sleep(Duration::from_millis(1));
        }
        sounding += got;
    }
    let (open, cv) = &*gate;
    *open.lock().unwrap() = true;
    cv.notify_all();
}

#[test]
fn a_dry_ring_is_an_underrun_and_plays_silence() {
    let rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.engine.pump();
    let now = rig.now;
    let mut tap = rig.device.tap.lock().unwrap();
    let tap = tap.as_mut().unwrap();
    // The read-ahead is 200 ms (9600 frames); ask for more without refilling.
    let mut out = vec![1.0; 2 * 12_000];
    tap.fill(&mut out, 2, Duration::ZERO, now);
    // The very first callback isn't counted (playback hadn't started).
    assert_eq!(rig.engine.underruns(), 0);
    tap.fill(&mut out[..200], 2, Duration::ZERO, now);
    assert!(out[..200].iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.underruns(), 1);
}

#[test]
fn startup_wait_is_not_an_underrun() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    let now = rig.now;
    let mut out = vec![1.0; 960];
    rig.device
        .tap
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .fill(&mut out, 2, Duration::ZERO, now);
    assert!(out.iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.underruns(), 0);
    assert_eq!(rig.position(), Some(Tick::ZERO));
    // The first real frames are the requested start.
    assert_eq!(rig.pull(1)[0], 0.0);
    assert_eq!(rig.pull(1)[0], ramp(1));
}

#[test]
fn preview_mute_ramps_to_silence() {
    let mut rig = Rig::new();
    rig.set_wave(Wave::Dc(0.5), 1.0);
    rig.engine.play(Tick::ZERO);
    assert_eq!(rig.pull(10)[0], 0.5);
    rig.engine.set_muted(true);
    let out = rig.pull(960);
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    assert!(left[0] < 0.5 && left[0] > 0.49);
    assert!(left.windows(2).all(|w| w[1] <= w[0]));
    assert_eq!(left[480], 0.0);
    assert_eq!(left[959], 0.0);
    // The clock keeps running while muted.
    assert_eq!(rig.position(), Some(samples(970)));
    rig.engine.set_muted(false);
    let out = rig.pull(960);
    assert_eq!(out[2 * 959], 0.5);
}

#[test]
fn master_gain_reaches_the_mix() {
    let mut rig = Rig::new();
    rig.set_wave(Wave::Dc(0.5), 1.0);
    rig.engine.set_master_gain(0.5);
    rig.engine.play(Tick::ZERO);
    assert_eq!(rig.pull(10)[0], 0.25);
}

#[test]
fn device_formats_and_channel_layouts() {
    let rig = Rig::new();
    rig.set_wave(Wave::Dc(0.5), 1.0);
    rig.engine.play(Tick::ZERO);
    rig.engine.pump();
    let now = rig.now;
    let mut guard = rig.device.tap.lock().unwrap();
    let tap = guard.as_mut().unwrap();

    let mut mono = [0i16; 4];
    tap.fill(&mut mono, 1, Duration::ZERO, now);
    assert_eq!(mono, [16_384; 4]);

    let mut quad = [0u16; 8];
    tap.fill(&mut quad, 4, Duration::ZERO, now);
    assert_eq!(
        quad,
        [
            49_152, 49_152, 32_768, 32_768, 49_152, 49_152, 32_768, 32_768
        ]
    );

    // A partial trailing frame is silent.
    let mut odd = [1.0f32; 5];
    tap.fill(&mut odd, 2, Duration::ZERO, now);
    assert_eq!(odd, [0.5, 0.5, 0.5, 0.5, 0.0]);

    assert_eq!(i16::from_f32(2.0), i16::MAX);
    assert_eq!(i16::from_f32(-2.0), -i16::MAX);
    assert_eq!(u8::from_f32(0.0), 128);
    assert_eq!(i32::from_f32(1.0), i32::MAX);
}

#[test]
fn a_lost_device_is_reopened_and_playback_continues() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.device.unplug("device unplugged");
    // The replacement device runs at 44.1 kHz.
    rig.device.rate.store(44_100, Ordering::SeqCst);
    rig.engine.pump();
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 2);
    assert_eq!(rig.engine.sample_rate().hz(), 44_100);
    assert_eq!(rig.engine.device_name(), "Fake 44100");
    assert_eq!(rig.engine.last_error(), None);
    // Continues right after the 10 ms already played: sample 441 at 44.1 kHz.
    assert_eq!(rig.position(), Some(secs(0.01)));
    let out = rig.pull(441);
    assert_eq!(out[0], ramp(441));
    assert_eq!(rig.position(), Some(secs(0.02)));
}

#[test]
fn errors_from_a_replaced_stream_are_ignored() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    let old = rig.device.lost.lock().unwrap().take().unwrap();
    old("device unplugged".to_owned());
    rig.engine.pump();
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 2);
    old("late error".to_owned());
    rig.engine.pump();
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 2);
}

#[test]
fn without_a_device_the_clock_runs_on_wall_time() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.device.fail.store(true, Ordering::SeqCst);
    rig.device.unplug("device unplugged");
    // First attempt, then the single retry (immediate in manual mode).
    rig.engine.pump();
    rig.engine.pump();
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 3);
    let err = rig.engine.last_error().unwrap();
    assert!(err.contains("device unplugged"), "{err}");
    assert!(rig.engine.is_playing());
    let now = Instant::now();
    let p0 = rig.engine.position_at(now).unwrap();
    let p1 = rig
        .engine
        .position_at(now + Duration::from_secs(1))
        .unwrap();
    assert_eq!(p1.checked_sub(p0).unwrap(), secs(1.0));

    // Pressing play again tries the device again; it works now.
    rig.device.fail.store(false, Ordering::SeqCst);
    rig.engine.play(secs(3.0));
    rig.engine.pump();
    assert_eq!(rig.engine.last_error(), None);
    rig.now = Instant::now();
    // It continues from the wall-clock position, a moment after 3 s.
    let first = rig.pull(10)[0];
    assert!(
        first >= ramp(144_000) && first < ramp(144_000 + 4_800),
        "{first}"
    );
}

#[test]
fn the_feeder_thread_renders_ahead_and_stops_on_drop() {
    let device = FakeDevice::new();
    let engine = Engine::start(Box::new(device.clone()), EngineConfig::default()).unwrap();
    let i = input(1, 1, Tick::ZERO, secs(60.0), Tick::ZERO);
    engine.set_source(
        Arc::new(List(vec![i])),
        Arc::new(Opener::with(&[(1, Wave::Dc(0.5))])),
    );
    let started = Instant::now();
    engine.play(Tick::ZERO);
    let mut out = [0.0f32; 2];
    while out[0] != 0.5 {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "nothing rendered"
        );
        std::thread::yield_now();
        let mut tap = device.tap.lock().unwrap();
        tap.as_mut()
            .unwrap()
            .fill(&mut out, 2, Duration::ZERO, Instant::now());
    }
    drop(engine);
    // Shutdown closed the device.
    assert!(device.tap.lock().unwrap().is_none());
}

// ------------------------------------------------------- devices (AU-010)

fn chosen(name: &str) -> EngineConfig {
    EngineConfig {
        device: Some(name.to_owned()),
        ..EngineConfig::default()
    }
}

fn status(device: &str, requested: Option<&str>, reason: Option<DeviceIssue>) -> EngineStatus {
    EngineStatus {
        device_name: device.to_owned(),
        requested: requested.map(str::to_owned),
        using_fallback: reason.is_some() && !device.is_empty(),
        reason,
    }
}

#[test]
fn the_chosen_device_plays() {
    let device = FakeDevice::new().with("USB", Named::working(44_100));
    let mut rig = Rig::on(device, chosen("USB"));
    assert_eq!(rig.engine.status(), status("USB", Some("USB"), None));
    assert_eq!(rig.engine.sample_rate().hz(), 44_100);
    assert_eq!(rig.device.last_open(), (Some("USB".to_owned()), 1024));
    rig.set_wave(Wave::Counter, 1e-6);
    rig.engine.play(secs(1.0));
    assert_eq!(rig.pull(10)[0], ramp(44_100));
}

#[test]
fn buffer_sizes_are_clamped_to_the_preference_range() {
    for (asked, used) in [(10, 256), (256, 256), (2_048, 2_048), (100_000, 4_096)] {
        let config = EngineConfig {
            device: None,
            buffer_frames: asked,
        };
        assert_eq!(config.buffer(), used);
        let rig = Rig::on(FakeDevice::new(), config);
        assert_eq!(rig.device.last_open(), (None, used));
    }
    assert_eq!(EngineConfig::default().buffer_frames, 1_024);
    assert_eq!(EngineConfig::default().device, None);
}

#[test]
fn a_missing_busy_or_denied_device_falls_back_to_the_default_at_once() {
    let cases = [
        (None, DeviceIssue::NotConnected),
        (Some(Fail::Busy), DeviceIssue::Busy),
        (Some(Fail::Denied), DeviceIssue::AccessDenied),
    ];
    for (fail, issue) in cases {
        let mut device = FakeDevice::new();
        if let Some(fail) = fail {
            device = device.with("Chosen", Named::failing(fail, usize::MAX));
        }
        let mut rig = Rig::on(device, chosen("Chosen"));
        assert_eq!(
            rig.engine.status(),
            status("Fake 48000", Some("Chosen"), Some(issue)),
            "{fail:?}"
        );
        // One try at the chosen device, then the default; no retry.
        rig.engine.pump();
        assert_eq!(rig.device.opens.load(Ordering::SeqCst), 2);
        rig.set_wave(Wave::Counter, 1e-6);
        rig.engine.play(Tick::ZERO);
        assert_eq!(rig.pull(10)[10], ramp(5));
        assert!(!issue.to_string().is_empty());
    }
}

#[test]
fn a_device_that_does_not_respond_is_retried_once_before_falling_back() {
    assert_eq!(RETRY_DELAY, Duration::from_secs(1));
    // Fails twice: the engine starts without a device, retries (at once in
    // manual mode, after 1 s for real), then plays on the default.
    let device = FakeDevice::new().with("Flaky", Named::failing(Fail::Error, 2));
    let rig = Rig::on(device, chosen("Flaky"));
    assert_eq!(
        rig.engine.status(),
        status("", Some("Flaky"), Some(DeviceIssue::NotResponding))
    );
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 1);
    rig.engine.pump();
    assert_eq!(
        rig.engine.status(),
        status(
            "Fake 48000",
            Some("Flaky"),
            Some(DeviceIssue::NotResponding)
        )
    );
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 3);

    // Fails once: the retry opens it.
    let device = FakeDevice::new().with("Flaky", Named::failing(Fail::Error, 1));
    let rig = Rig::on(device, chosen("Flaky"));
    rig.engine.pump();
    assert_eq!(rig.engine.status(), status("Flaky", Some("Flaky"), None));
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), 2);
}

#[test]
fn reconfigure_switches_device_and_keeps_the_position() {
    let device = FakeDevice::new().with("USB", Named::working(44_100));
    let mut rig = Rig::on(device, EngineConfig::default());
    rig.set_wave(Wave::Counter, 1e-6);
    rig.engine.play(secs(1.0));
    rig.pull(480);
    rig.engine.reconfigure(EngineConfig {
        device: Some("USB".to_owned()),
        buffer_frames: 512,
    });
    rig.engine.pump();
    assert_eq!(rig.device.last_open(), (Some("USB".to_owned()), 512));
    assert_eq!(rig.engine.status(), status("USB", Some("USB"), None));
    assert!(rig.engine.is_playing());
    // Continues right after the 10 ms already played, at 44.1 kHz.
    assert_eq!(rig.position(), Some(secs(1.01)));
    let out = rig.pull(441);
    assert_eq!(out[0], ramp(44_541));
    assert_eq!(rig.position(), Some(secs(1.02)));
    // A deliberate switch isn't an underrun.
    assert_eq!(rig.engine.underruns(), 0);

    // The same settings again change nothing.
    let opens = rig.device.opens.load(Ordering::SeqCst);
    rig.engine.reconfigure(EngineConfig {
        device: Some("USB".to_owned()),
        buffer_frames: 512,
    });
    rig.engine.pump();
    assert_eq!(rig.device.opens.load(Ordering::SeqCst), opens);

    // Back to the system default.
    rig.engine.reconfigure(EngineConfig::default());
    rig.engine.pump();
    assert_eq!(rig.engine.status(), status("Fake 48000", None, None));
    assert_eq!(rig.position(), Some(secs(1.02)));
    assert_eq!(rig.pull(10)[0], ramp(48_960));
}

#[test]
fn reconfigure_while_paused_stays_paused() {
    let device = FakeDevice::new().with("USB", Named::working(44_100));
    let mut rig = Rig::on(device, EngineConfig::default());
    rig.set_wave(Wave::Counter, 1e-6);
    rig.engine.reconfigure(chosen("USB"));
    rig.engine.pump();
    assert!(!rig.engine.is_playing());
    assert_eq!(rig.position(), None);
    assert!(rig.pull(480).iter().all(|&v| v == 0.0));
    assert_eq!(rig.engine.sample_rate().hz(), 44_100);
    rig.engine.play(secs(2.0));
    assert_eq!(rig.pull(10)[0], ramp(88_200));
}

#[test]
fn reconfigure_to_a_missing_device_falls_back_and_keeps_playing() {
    let mut rig = Rig::with_ramp();
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.engine.reconfigure(chosen("Gone"));
    rig.engine.pump();
    assert_eq!(
        rig.engine.status(),
        status("Fake 48000", Some("Gone"), Some(DeviceIssue::NotConnected))
    );
    assert_eq!(rig.position(), Some(samples(480)));
    assert_eq!(rig.pull(10)[0], ramp(480));
}

#[test]
fn a_lost_chosen_device_falls_back_to_the_default() {
    let device = FakeDevice::new().with("USB", Named::working(48_000));
    let mut rig = Rig::on(device, chosen("USB"));
    rig.set_wave(Wave::Counter, 1e-6);
    rig.engine.play(Tick::ZERO);
    rig.pull(480);
    rig.device.named.lock().unwrap().remove("USB");
    rig.device.unplug("device unplugged");
    rig.engine.pump();
    assert_eq!(
        rig.engine.status(),
        status("Fake 48000", Some("USB"), Some(DeviceIssue::NotConnected))
    );
    assert_eq!(rig.engine.last_error(), None);
    assert_eq!(rig.position(), Some(samples(480)));
    assert_eq!(rig.pull(10)[0], ramp(480));
}

#[test]
fn a_device_buffer_longer_than_the_read_ahead_never_runs_dry() {
    // A narrowband output (Bluetooth hands-free) with the largest buffer:
    // 4096 frames at 16 kHz is 256 ms, more than the 200 ms read-ahead.
    for hz in [8_000, 16_000] {
        let device = FakeDevice::new();
        device.rate.store(hz, Ordering::SeqCst);
        let config = EngineConfig {
            buffer_frames: EngineConfig::MAX_BUFFER_FRAMES,
            ..EngineConfig::default()
        };
        let mut rig = Rig::on(device, config);
        rig.set_wave(Wave::Dc(0.5), 1.0);
        rig.engine.play(Tick::ZERO);
        for _ in 0..20 {
            let out = rig.pull(4_096);
            assert!(out.iter().all(|&v| v != 0.0), "{hz} Hz: silent frames");
        }
        assert_eq!(rig.engine.underruns(), 0, "{hz} Hz");
    }
}

// ------------------------------------------- re-keyed inputs (AU-038, TL-030)

/// One millisecond at 48 kHz: the window the gap checks measure in.
const MS: usize = 48;

/// RMS of `w`.
fn rms(w: &[f32]) -> f32 {
    (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt()
}

/// The longest run of (all but) silent samples in `l`.
fn longest_silence(l: &[f32]) -> usize {
    let (mut run, mut longest) = (0, 0);
    for &v in l {
        run = if v.abs() < 1e-4 { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// Checks that `heard` (one channel of a 1 kHz tone at 48 kHz, so every
/// 1 ms window holds one whole period) never drops out: no silent run
/// longer than 1 ms, no 1 ms window quieter than `floor` RMS, neighbouring
/// windows within `max_rms_step` RMS of each other (a ramp, not a cut), and
/// no sample-to-sample jump beyond `max_jump` (the tone itself moves up to
/// 0.08 per sample at amplitude 0.57).
fn assert_seamless(heard: &[f32], floor: f32, max_rms_step: f32, max_jump: f32, what: &str) {
    let silent = longest_silence(heard);
    assert!(silent <= MS, "{what}: {silent} silent samples in a row");
    let levels: Vec<f32> = heard.as_chunks::<MS>().0.iter().map(|w| rms(w)).collect();
    for (i, &level) in levels.iter().enumerate() {
        assert!(level >= floor, "{what}: {level} RMS in ms {i}");
    }
    for (i, w) in levels.windows(2).enumerate() {
        let step = (w[1] - w[0]).abs();
        assert!(
            step <= max_rms_step,
            "{what}: RMS steps by {step} at ms {i}"
        );
    }
    let jump = heard
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump <= max_jump, "{what}: a step of {jump}");
}

/// Clip [`CLIP`] of asset 1 on `[0, 60 s)` playing media from 0, keyed
/// `key`, as stream `stream` of `mixed`.
fn stream_input(key: u64, stream: u32, mixed: usize) -> MixInput {
    MixInput {
        clip: CLIP,
        source: SourceKey { asset: 1, stream },
        stream_mix: crate::stream_mix_factor(mixed),
        ..input(key, 1, Tick::ZERO, secs(60.0), Tick::ZERO)
    }
}

/// Asset 1 with two audio streams carrying a 1 kHz tone in phase: stream 1
/// at amplitude 0.5, stream 2 at 0.3.
fn two_stream_opener() -> Arc<Opener> {
    let tone = |amp| Wave::Sine { amp, hz: 1_000.0 };
    Arc::new(Opener {
        streams: [((1, 1), tone(0.5)), ((1, 2), tone(0.3))].into(),
        ..Opener::default()
    })
}

/// Plays `steps[0]` from the start, then switches to each following
/// schedule during playback, 400 ms apart (twice the read-ahead, so each
/// is heard), through one opener (as the app does after an edit). Returns
/// the left channel heard from 100 ms in.
fn play_switching(opener: &Arc<Opener>, steps: &[Vec<MixInput>]) -> (Rig, Vec<f32>) {
    let mut rig = Rig::new();
    let mut heard = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        rig.engine
            .set_source(Arc::new(List(step.clone())), opener.clone());
        if i == 0 {
            rig.engine.play(Tick::ZERO);
        }
        for _ in 0..40 {
            heard.extend(rig.pull(480).iter().step_by(2).copied());
        }
    }
    (rig, heard.split_off(4_800))
}

/// The clip id the tests' clip has, and the one its detached sound gets.
const CLIP: u64 = 5;
const DETACHED: u64 = 6;

fn mixed_key(stream: u32) -> u64 {
    use freecut_timeline::ids::ClipId;
    let clip = ClipId::from_raw(std::num::NonZeroU64::new(CLIP).unwrap());
    crate::stream_input_key(clip, stream)
}

#[test]
fn toggling_all_streams_mixed_during_playback_leaves_no_gap() {
    let opener = two_stream_opener();
    let one = vec![stream_input(CLIP, 1, 1)];
    let all = vec![
        stream_input(mixed_key(1), 1, 2),
        stream_input(mixed_key(2), 2, 2),
    ];
    let (rig, heard) = play_switching(&opener, &[one.clone(), all.clone(), one, all]);
    // Stream 1 alone is 0.354 RMS, both mixed 0.4. Switching on, stream 1
    // keeps its level until stream 2's reader, opened for it, joins about
    // 50 ms later: never lower, and every change ramps over 10 ms (less
    // than 0.03 per ms).
    assert_seamless(&heard, 0.35, 0.03, 0.1, "All streams mixed on and off");
    // Each switch was heard: the mix ends with both streams.
    let last = rms(&heard[heard.len() - MS..]);
    let both = 0.8 * crate::stream_mix_factor(2) / 2f32.sqrt();
    assert!((last - both).abs() < 1e-3, "{last} RMS at the end");
    assert_eq!(rig.engine.underruns(), 0);
    // Stream 1 kept its one reader throughout; stream 2 was opened each
    // time it joined.
    assert_eq!(opener.opens(), 3);
}

#[test]
fn switching_all_streams_mixed_on_during_playback_never_dips_the_clip() {
    let opener = two_stream_opener();
    let one = vec![stream_input(CLIP, 1, 1)];
    let all = vec![
        stream_input(mixed_key(1), 1, 2),
        stream_input(mixed_key(2), 2, 2),
    ];
    let (rig, heard) = play_switching(&opener, &[one, all]);
    let levels: Vec<f32> = heard.as_chunks::<MS>().0.iter().map(|w| rms(w)).collect();
    // The steady states: stream 1 alone (0.5 / √2, 0.354 RMS) before the
    // switch, both streams at 1/√2 each (0.4 RMS) once it was heard.
    let (before, after) = (levels[0], levels[levels.len() - 1]);
    assert!(
        (before - 0.5 / 2f32.sqrt()).abs() < 1e-3,
        "{before} RMS before"
    );
    let both = 0.8 * crate::stream_mix_factor(2) / 2f32.sqrt();
    assert!(
        (after - both).abs() < 1e-3,
        "{after} RMS after: not switched"
    );
    // Stream 1 keeps its level until stream 2's reader, opened ~50 ms
    // ahead, joins; then both ramp together. No 1 ms window across the
    // switch is more than 0.5 dB below the steady state.
    let floor = before.min(after) * 10f32.powf(-0.5 / 20.0);
    for (i, &level) in levels.iter().enumerate() {
        assert!(level >= floor, "{level} RMS in ms {i}, below {floor}");
    }
    // No silent run: only the tone's own zero crossings, a sample each.
    let silent = longest_silence(&heard);
    assert!(silent <= 1, "{silent} silent samples in a row");
    assert_eq!(rig.engine.underruns(), 0);
    // Stream 1 kept its reader; stream 2 got its own.
    assert_eq!(opener.opens(), 2);
}

#[test]
fn detaching_the_sound_during_playback_leaves_no_gap() {
    let opener = two_stream_opener();
    let attached = vec![stream_input(CLIP, 1, 1)];
    // The detached audio clip is a new clip with its own key, playing the
    // same stream at the same place.
    let mut detached = stream_input(DETACHED, 1, 1);
    detached.clip = DETACHED;
    detached.track.id = 2;
    let (rig, heard) = play_switching(&opener, &[attached.clone(), vec![detached], attached]);
    assert_seamless(&heard, 0.3, 1e-3, 0.1, "detach and undo");
    assert_eq!(rig.engine.underruns(), 0);
    // One reader played throughout: nothing was reopened.
    assert_eq!(opener.opens(), 1);
}

#[test]
fn a_stream_switch_during_playback_leaves_no_gap() {
    let opener = two_stream_opener();
    let first = vec![stream_input(CLIP, 1, 1)];
    let second = vec![stream_input(CLIP, 2, 1)];
    let (rig, heard) = play_switching(&opener, &[first.clone(), second, first]);
    // A cut between the streams (0.354 and 0.212 RMS), never a gap.
    assert_seamless(&heard, 0.2, 0.15, 0.3, "stream switch");
    assert_eq!(rig.engine.underruns(), 0);
}

#[test]
fn a_recorders_monitoring_plays_after_mute_at_its_own_rate_only() {
    let mut rig = Rig::new();
    rig.engine.set_muted(true);
    let (mut tap, feed) = crate::record::monitor_channel(48_000);
    assert!(rig.engine.set_monitor(Some(feed)));
    for _ in 0..4 {
        tap.push(&[0.25]);
    }
    // Paused and muted: only the monitored input is heard.
    let out = rig.pull(8);
    assert_eq!(&out[..8], &[0.25; 8]);
    assert!(out[8..].iter().all(|&s| s == 0.0), "{out:?}");
    // A feed at another rate stays silent.
    let (mut tap, feed) = crate::record::monitor_channel(44_100);
    assert!(!rig.engine.set_monitor(Some(feed)));
    tap.push(&[0.5]);
    assert!(rig.pull(4).iter().all(|&s| s == 0.0));
    assert!(!rig.engine.set_monitor(None));
    assert!(rig.pull(4).iter().all(|&s| s == 0.0));
}
