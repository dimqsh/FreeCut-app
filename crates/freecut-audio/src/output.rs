//! Playback on an output device through `cpal`, and the device list
//! (AU-010, UI-029).

use std::sync::Arc;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, Host, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize, SupportedStreamConfig, SupportedStreamConfigRange,
};
use freecut_timeline::time::{SampleRate, Tick};

use crate::playback::{
    Backend, DeviceInfo, DeviceSample, Engine, EngineConfig, EngineStatus, LostHandler,
    OutputDeviceInfo, OutputError, OutputTap,
};
use crate::source::{Schedule, SourceOpener};

/// Rates tried, in order, when the device's default configuration can't be
/// used (after its own rate, when that is on the tick grid).
const FALLBACK_RATES: [u32; 2] = [48_000, 44_100];

/// The output devices the system offers, by name, with the current system
/// default marked (UI-029). Names that appear twice are listed once (a name
/// picks the first device that has it).
///
/// This asks the audio server and may block for a while on some systems:
/// call it off the UI thread.
pub fn list_output_devices() -> Result<Vec<OutputDeviceInfo>, OutputError> {
    let host = cpal::default_host();
    let default = host.default_output_device().as_ref().and_then(device_name);
    let names = host
        .output_devices()
        .map_err(device_error)?
        .filter_map(|d| device_name(&d));
    Ok(device_list(names, default.as_deref()))
}

/// The device's name, as `Display` shows it, or `None` when the system
/// can't give it (a device being unplugged). `to_string()` panics then, and
/// on the feeder thread a panic would end playback for good.
fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

fn device_list(
    names: impl IntoIterator<Item = String>,
    default: Option<&str>,
) -> Vec<OutputDeviceInfo> {
    let mut list: Vec<OutputDeviceInfo> = Vec::new();
    for name in names {
        if list.iter().any(|d| d.name == name) {
            continue;
        }
        list.push(OutputDeviceInfo {
            is_default: default == Some(name.as_str()),
            name,
        });
    }
    list
}

/// Real-time playback of a [`Schedule`] on an output device: the one chosen
/// in the [`EngineConfig`], or the system default.
///
/// Mixing runs at the device's own sample rate (sources are resampled
/// straight to it), on a feeder thread that renders about 200 ms ahead. The
/// device callback only copies samples. All methods are cheap and may be
/// called from the UI thread.
///
/// The engine is also the master clock for A/V sync: [`AudioEngine::position`]
/// is the timeline time being heard right now.
#[derive(Debug)]
pub struct AudioEngine {
    engine: Engine,
}

impl AudioEngine {
    /// Opens the configured output device (or the system default) at its
    /// default configuration and starts the feeder thread, paused and
    /// silent. May block while the device opens: call it off the UI thread.
    ///
    /// A configured device that is missing, busy or denied gives way to the
    /// system default ([`AudioEngine::status`] says why); one that fails
    /// otherwise is tried again after 1 s, the engine starting without a
    /// device meanwhile. Fails with [`OutputError::NoDevice`] when no device
    /// opens at all; the app then plays video on the system clock and shows
    /// its banner.
    pub fn start(config: EngineConfig) -> Result<AudioEngine, OutputError> {
        let engine = Engine::start(Box::new(CpalBackend { stream: None }), config)?;
        Ok(AudioEngine { engine })
    }

    /// Replaces what plays (the document changed). Playback, if running,
    /// continues without a gap: the audio already queued plays out and the
    /// new content is heard after the read-ahead (about 200 ms), with volume
    /// and mute changes ramped. Only [`AudioEngine::play`] and
    /// [`AudioEngine::seek`] flush.
    ///
    /// Open readers stay cached while `opener` is the same `Arc` as before;
    /// with a different opener every source is reopened in the background,
    /// each old reader playing on until its replacement is ready.
    pub fn set_source(&self, schedule: Arc<dyn Schedule>, opener: Arc<dyn SourceOpener>) {
        self.engine.set_source(schedule, opener);
    }

    /// Starts (or restarts) playback at timeline time `from`.
    pub fn play(&self, from: Tick) {
        self.engine.play(from);
    }

    /// Stops playback at once; buffered audio is discarded.
    pub fn pause(&self) {
        self.engine.pause();
    }

    /// Jumps to `to`. While playing, buffers flush and playback continues
    /// there; while paused this does nothing until [`AudioEngine::play`].
    pub fn seek(&self, to: Tick) {
        self.engine.seek(to);
    }

    /// The timeline time being heard now, compensated for the device's
    /// output latency; monotonic during playback. `None` when paused.
    ///
    /// If the device is lost and can't be reopened, the position keeps
    /// advancing on the system clock.
    pub fn position(&self) -> Option<Tick> {
        self.engine.position()
    }

    pub fn is_playing(&self) -> bool {
        self.engine.is_playing()
    }

    /// Master volume as linear amplitude (before the limiter). Takes effect
    /// on audio rendered from now on, within the ~200 ms read-ahead.
    pub fn set_master_gain(&self, linear: f32) {
        self.engine.set_master_gain(linear);
    }

    /// "Hear yourself" while recording (AU-027): plays the recorder's
    /// [`MonitorFeed`](crate::record::MonitorFeed) after the master stage,
    /// never into an export; `None` stops it. Returns whether the feed's
    /// rate is the device's: otherwise it stays silent and the panel turns
    /// monitoring off.
    pub fn set_monitor(&self, feed: Option<crate::record::MonitorFeed>) -> bool {
        self.engine.set_monitor(feed)
    }

    /// Preview mute (EX-019): silences the output immediately, with a short
    /// ramp. Export is unaffected.
    pub fn set_muted(&self, muted: bool) {
        self.engine.set_muted(muted);
    }

    /// How many times the output ran dry (or the device reported lost
    /// samples) since the engine started.
    pub fn underruns(&self) -> u64 {
        self.engine.underruns()
    }

    /// The mixing rate, which is the device's rate.
    pub fn sample_rate(&self) -> SampleRate {
        self.engine.sample_rate()
    }

    pub fn device_name(&self) -> String {
        self.engine.device_name()
    }

    /// Which device plays and, when it isn't the chosen one, why (for the
    /// banner and Preferences).
    pub fn status(&self) -> EngineStatus {
        self.engine.status()
    }

    /// Switches to another output device or buffer size (Preferences). The
    /// switch happens on the feeder thread; playback continues from where it
    /// is, or stays paused at the same position. The same fallback rules as
    /// [`AudioEngine::start`] apply, reported by [`AudioEngine::status`].
    pub fn reconfigure(&self, config: EngineConfig) {
        self.engine.reconfigure(config);
    }

    /// Set when the device was lost and reopening failed; cleared once a
    /// device works again.
    pub fn last_error(&self) -> Option<String> {
        self.engine.last_error()
    }
}

/// Output devices through `cpal`.
struct CpalBackend {
    stream: Option<Stream>,
}

impl Backend for CpalBackend {
    fn open(
        &mut self,
        device: Option<&str>,
        tap: OutputTap,
        buffer_frames: u32,
        on_lost: LostHandler,
    ) -> Result<DeviceInfo, OutputError> {
        self.close();
        let host = cpal::default_host();
        let device = match device {
            None => host.default_output_device().ok_or(OutputError::NoDevice)?,
            Some(name) => find_output_device(&host, name)?,
        };
        let name = device_name(&device)
            .ok_or_else(|| OutputError::Device("the device has no name".to_owned()))?;
        let (supported, rate) = choose_config(&device)?;
        let mut config = supported.config();
        config.buffer_size = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => {
                BufferSize::Fixed(buffer_frames.clamp(*min, (*max).max(*min)))
            }
            SupportedBufferSize::Unknown => BufferSize::Default,
        };
        let opened_buffer = match config.buffer_size {
            BufferSize::Fixed(n) => Some(n),
            BufferSize::Default => None,
        };
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(&device, config, tap, on_lost),
            SampleFormat::F64 => build::<f64>(&device, config, tap, on_lost),
            SampleFormat::I16 => build::<i16>(&device, config, tap, on_lost),
            SampleFormat::U16 => build::<u16>(&device, config, tap, on_lost),
            SampleFormat::I32 => build::<i32>(&device, config, tap, on_lost),
            SampleFormat::I8 => build::<i8>(&device, config, tap, on_lost),
            SampleFormat::U8 => build::<u8>(&device, config, tap, on_lost),
            other => return Err(OutputError::UnsupportedFormat(other.to_string())),
        }
        .map_err(device_error)?;
        stream.play().map_err(device_error)?;
        tracing::info!(device = %name, rate = rate.hz(), "audio output started");
        self.stream = Some(stream);
        Ok(DeviceInfo {
            rate,
            name,
            buffer_frames: opened_buffer,
        })
    }

    fn close(&mut self) {
        self.stream = None;
    }
}

/// The first output device named `name`.
fn find_output_device(host: &Host, name: &str) -> Result<Device, OutputError> {
    let devices = host.output_devices().map_err(device_error)?;
    first_named(devices, name, device_name)
        .ok_or_else(|| OutputError::DeviceNotFound(name.to_owned()))
}

/// The first of `devices` named `name`; devices without a name are skipped.
fn first_named<D>(
    devices: impl IntoIterator<Item = D>,
    name: &str,
    name_of: impl Fn(&D) -> Option<String>,
) -> Option<D> {
    devices
        .into_iter()
        .find(|d| name_of(d).as_deref() == Some(name))
}

fn device_error(e: cpal::Error) -> OutputError {
    match e.kind() {
        ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable => OutputError::NoDevice,
        ErrorKind::DeviceBusy => OutputError::Busy(e.to_string()),
        ErrorKind::PermissionDenied => OutputError::Denied(e.to_string()),
        _ => OutputError::Device(e.to_string()),
    }
}

/// The device's default configuration when FreeCut can play it, else the
/// closest supported one (see [`pick_config`]).
fn choose_config(device: &Device) -> Result<(SupportedStreamConfig, SampleRate), OutputError> {
    let default = device.default_output_config().map_err(device_error)?;
    if handled(default.sample_format())
        && let Ok(rate) = SampleRate::new(default.sample_rate())
    {
        return Ok((default, rate));
    }
    let ranges: Vec<_> = device
        .supported_output_configs()
        .map_err(device_error)?
        .collect();
    pick_config(&default, &ranges)
}

/// Sample formats the output tap writes.
fn handled(format: SampleFormat) -> bool {
    HANDLED_FORMATS.contains(&format)
}

/// Every format the tap writes, in the order a fallback prefers them.
const HANDLED_FORMATS: [SampleFormat; 7] = [
    SampleFormat::F32,
    SampleFormat::I16,
    SampleFormat::I32,
    SampleFormat::F64,
    SampleFormat::U16,
    SampleFormat::I8,
    SampleFormat::U8,
];

/// A supported configuration for a device whose default can't be used as
/// is: its rate is off the tick grid, or its sample format (24-bit packed,
/// unsigned 32-bit, DSD, …) isn't one the tap writes. Tries the default rate
/// when it is on the grid, then 48 and 44.1 kHz; at each rate the default
/// format if it is handled, then F32, I16, I32 and the rest.
fn pick_config(
    default: &SupportedStreamConfig,
    ranges: &[SupportedStreamConfigRange],
) -> Result<(SupportedStreamConfig, SampleRate), OutputError> {
    let own_rate = SampleRate::new(default.sample_rate())
        .ok()
        .map(|_| default.sample_rate());
    let formats = handled(default.sample_format())
        .then_some(default.sample_format())
        .into_iter()
        .chain(HANDLED_FORMATS);
    let formats: Vec<SampleFormat> = formats.collect();
    for hz in own_rate.into_iter().chain(FALLBACK_RATES) {
        let Ok(rate) = SampleRate::new(hz) else {
            continue;
        };
        for format in &formats {
            let found = ranges
                .iter()
                .filter(|r| r.sample_format() == *format)
                .find_map(|r| r.try_with_sample_rate(hz));
            if let Some(config) = found {
                return Ok((config, rate));
            }
        }
    }
    if ranges.iter().any(|r| handled(r.sample_format())) {
        Err(OutputError::UnsupportedRate(default.sample_rate()))
    } else {
        Err(OutputError::UnsupportedFormat(
            default.sample_format().to_string(),
        ))
    }
}

fn build<T: SizedSample + DeviceSample + Send + 'static>(
    device: &Device,
    config: StreamConfig,
    mut tap: OutputTap,
    on_lost: LostHandler,
) -> Result<Stream, cpal::Error> {
    let channels = usize::from(config.channels);
    let shared = tap.shared();
    device.build_output_stream::<T, _, _>(
        config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            let ts = info.timestamp();
            let latency = ts
                .playback
                .checked_duration_since(ts.callback)
                .unwrap_or_default();
            tap.fill(data, channels, latency, Instant::now());
        },
        move |err: cpal::Error| match err.kind() {
            ErrorKind::DeviceNotAvailable | ErrorKind::StreamInvalidated => {
                on_lost(err.to_string());
            }
            ErrorKind::Xrun => shared.note_xrun(),
            // The stream follows the new default device by itself.
            ErrorKind::DeviceChanged => tracing::info!("audio output rerouted: {err}"),
            _ => tracing::warn!("audio output error: {err}"),
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn range(format: SampleFormat, min: u32, max: u32) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(2, min, max, SupportedBufferSize::Unknown, format)
    }

    fn default_config(format: SampleFormat, hz: u32) -> SupportedStreamConfig {
        SupportedStreamConfig::new(2, hz, SupportedBufferSize::Unknown, format)
    }

    #[test]
    fn an_unhandled_default_format_falls_back_to_a_supported_one() {
        // An ALSA hw device whose default is packed 24-bit, which also
        // offers 16-bit.
        let ranges = [
            range(SampleFormat::I24, 44_100, 48_000),
            range(SampleFormat::I16, 44_100, 48_000),
        ];
        let (config, rate) =
            pick_config(&default_config(SampleFormat::I24, 48_000), &ranges).unwrap();
        assert_eq!(config.sample_format(), SampleFormat::I16);
        assert_eq!((config.sample_rate(), rate.hz()), (48_000, 48_000));
        // F32 is preferred when offered.
        let ranges = [
            range(SampleFormat::U24, 48_000, 48_000),
            range(SampleFormat::I16, 48_000, 48_000),
            range(SampleFormat::F32, 48_000, 48_000),
        ];
        let (config, _) = pick_config(&default_config(SampleFormat::U24, 48_000), &ranges).unwrap();
        assert_eq!(config.sample_format(), SampleFormat::F32);
        // Nothing handled at all.
        let ranges = [range(SampleFormat::I24, 48_000, 48_000)];
        assert!(matches!(
            pick_config(&default_config(SampleFormat::I24, 48_000), &ranges),
            Err(OutputError::UnsupportedFormat(_))
        ));
    }

    #[test]
    fn an_off_grid_default_rate_falls_back_to_48_or_44_1_khz() {
        let ranges = [range(SampleFormat::I16, 8_000, 44_100)];
        let (config, rate) =
            pick_config(&default_config(SampleFormat::I16, 37_800), &ranges).unwrap();
        assert_eq!((config.sample_rate(), rate.hz()), (44_100, 44_100));
        let ranges = [range(SampleFormat::F32, 8_000, 37_800)];
        assert!(matches!(
            pick_config(&default_config(SampleFormat::F32, 37_800), &ranges),
            Err(OutputError::UnsupportedRate(37_800))
        ));
    }

    #[test]
    fn device_lists_mark_the_default_and_drop_repeated_names() {
        let names = ["Speakers", "Headphones", "Speakers", "HDMI"].map(String::from);
        let list = device_list(names, Some("Headphones"));
        let got: Vec<(&str, bool)> = list
            .iter()
            .map(|d| (d.name.as_str(), d.is_default))
            .collect();
        assert_eq!(
            got,
            [("Speakers", false), ("Headphones", true), ("HDMI", false)]
        );
        assert!(device_list(Vec::new(), None).is_empty());
        assert!(
            device_list(["A".to_owned()], None)
                .iter()
                .all(|d| !d.is_default)
        );
    }

    #[test]
    fn devices_without_a_name_are_skipped_when_looking_one_up() {
        // A device being unplugged can't say its name.
        let devices = [None, Some("Speakers"), None, Some("Headphones")];
        let name_of = |d: &Option<&str>| d.map(str::to_owned);
        assert_eq!(
            first_named(devices, "Headphones", name_of),
            Some(Some("Headphones"))
        );
        assert_eq!(first_named(devices, "HDMI", name_of), None);
    }

    /// Plays a second of silence on the real default device. Needs sound
    /// hardware, so it only runs on request.
    #[test]
    #[ignore = "needs an audio output device"]
    fn plays_on_the_default_device() {
        let engine = AudioEngine::start(EngineConfig::default()).unwrap();
        engine.set_source(Arc::new(crate::Silence), Arc::new(crate::Silence));
        engine.play(Tick::ZERO);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let p = engine.position().unwrap();
        assert!(p > Tick::ZERO, "{p:?} on {}", engine.device_name());
        engine.pause();
        assert_eq!(engine.position(), None);
    }
}
