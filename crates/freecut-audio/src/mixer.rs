//! The pull-based block mixer shared by playback and export (AU-010).
//!
//! [`Mixer::render`] produces any span of output samples at a fixed rate.
//! Rendering a span in one call or in blocks of any sizes gives identical
//! samples, so export (large blocks) and playback (small blocks) agree.
//!
//! Per block: ask the [`Schedule`] what overlaps the block's time window;
//! for each input, read its samples from a cached [`SourceReader`], apply
//! its volume (static or keyframed), fades, pan, stream-mix factor and track
//! stage, and sum onto a stereo `f32` bus; then apply the master gain and
//! the safety limiter. Input `i` covers output samples
//! `[S(range.start), S(range.end))` where `S(t) = floor(t·rate / TPS)`, and
//! output sample `n` reads media time
//! `source_start + tick(n) − tick(S(range.start))`, which keeps every source
//! sample-exact across blocks.
//!
//! The limiter looks ahead, so the mixer internally runs its bus that many
//! samples ahead of the output; callers never see the delay. A render that
//! doesn't continue the previous one (a seek) resets the limiter and primes
//! it from the new position.
//!
//! Changes while rendering continuously never step (AU-005, AU-011,
//! AU-013): a clip's static volume, its pan, its stream-mix factor and its
//! track's volume and pan ramp to new values over about 10 ms, a clip that
//! leaves the schedule mid-way (muted, deleted) ramps out over as long, and
//! one that joins mid-way (unmuted) ramps in. Volume keyframes are not
//! ramped but evaluated at every sample (with the centred smoothing of
//! [`crate::envelope`], which doesn't lag); when a clip's keyframes change
//! mid-way, the level heard at that moment ramps to the new envelope over
//! the same 10 ms. Clips that start or end where they are placed, and every
//! render after a seek, play their exact samples. A clip that an edit
//! re-keys while it plays (switched to or from "All streams mixed",
//! AU-038; its sound detached, TL-030) keeps its reader: the new key takes
//! over the old key's reader of the same source on the same media axis and
//! ramps any change of level, so it neither stops nor restarts. While other
//! streams of the same clip ([`MixInput::clip`]) wait for their readers,
//! that change of level waits too, and starts at the sample they join at:
//! a clip switched to "All streams mixed" keeps its level until all its
//! streams sound, then ramps to the mix as they ramp in, so it never dips.
//!
//! **Readers.** By default a missing reader is opened inline, at the block
//! that first needs it (export). Playback instead calls
//! [`Mixer::defer_opens`] and hands readers in, already opened and
//! positioned on another thread ([`Mixer::offer`]); an input whose reader
//! isn't there yet plays silence and joins when it arrives.

use std::collections::HashMap;

use freecut_timeline::time::{SampleRate, Tick, TimeRange};

use crate::envelope::{GainKeys, Smoother};
use crate::gain::Fades;
use crate::limiter::Limiter;
use crate::samples::{sample_at, sample_start, sample_ticks};
use crate::source::{MixInput, Schedule, SourceKey, SourceOpener, SourceReader};

/// Length of the master gain ramp in seconds (avoids zipper noise).
const MASTER_RAMP_SECS: f32 = 0.01;
/// Length of a clip volume ramp, and of the ramp in or out of a clip that
/// joins or leaves the mix mid-way (AU-005, AU-011).
const CLIP_RAMP_SECS: f32 = 0.01;

/// A cached reader for one scheduled input.
struct Slot {
    source: SourceKey,
    /// `None` when the source isn't open (yet), couldn't be opened or failed
    /// to read.
    reader: Option<Box<dyn SourceReader>>,
    /// Opening or reading failed; tried again after the next jump.
    failed: bool,
    /// The reader was opened by an opener that has been replaced, or for the
    /// input's previous source; it plays on until the new reader for this
    /// input takes over.
    stale: bool,
    /// The slot took over the key from one reading another source: its
    /// reader starts at full volume rather than ramping in.
    replaced: bool,
    /// The media time a sequential read would continue at.
    next_at: Option<Tick>,
    /// The mixing pass that last mixed this slot.
    pass: u64,
    /// The sample its reader started sounding at, when that was in the
    /// span just mixed.
    started: Option<i64>,
    /// The input as last scheduled, for a ramp-out after it leaves.
    last: Option<MixInput>,
    /// Smoothed static volume ([`MixInput::static_volume`]), ramping
    /// towards `target` by `step` per sample. With keyframes it multiplies
    /// their envelope.
    gain: f32,
    target: f32,
    step: f32,
    /// Smoothed left and right gains (clip and track pan), ramping towards
    /// `pan_target` by `pan_step` per sample.
    pan: [f32; 2],
    pan_target: [f32; 2],
    pan_step: [f32; 2],
    /// The volume keyframes the slot was last mixed with.
    keys: Option<GainKeys>,
    /// A keyframe change crossfade: the volume (static volume times
    /// envelope) heard when the keys changed, held and faded out as the new
    /// volume fades in over `blend_len` samples; `blend_pos` counts the
    /// samples done. Done when `blend_pos == blend_len`.
    hold: f64,
    blend_pos: u32,
    blend_len: u32,
}

impl Slot {
    fn new(source: SourceKey) -> Slot {
        Slot {
            source,
            reader: None,
            failed: false,
            stale: false,
            replaced: false,
            next_at: None,
            pass: 0,
            started: None,
            last: None,
            gain: 0.0,
            target: 0.0,
            step: 0.0,
            pan: [1.0; 2],
            pan_target: [1.0; 2],
            pan_step: [0.0; 2],
            keys: None,
            hold: 1.0,
            blend_pos: 0,
            blend_len: 0,
        }
    }

    /// Whether the slot is sounding: a reader read it last block at a
    /// volume above zero.
    fn audible(&self) -> bool {
        self.reader.is_some() && self.next_at.is_some() && self.gain > 0.0
    }

    /// Starts the volume at `from`, ramping to the target over `ramp` samples.
    fn ramp_from(&mut self, from: f32, ramp: f32) {
        self.gain = from;
        self.step = (self.target - from).abs() / ramp;
    }

    /// Heads the volume for `target`, ramping from where it is over `ramp`
    /// samples.
    fn retarget(&mut self, target: f32, ramp: f32) {
        if target != self.target {
            self.target = target;
            self.step = (target - self.gain).abs() / ramp;
        }
    }

    /// The volume to play for `volume`, the static volume times the
    /// envelope at this sample: itself, or its crossfade from the held one.
    fn blended(&self, volume: f64) -> f64 {
        if self.blend_pos >= self.blend_len {
            return volume;
        }
        let w = f64::from(self.blend_pos) / f64::from(self.blend_len);
        self.hold + (volume - self.hold) * w
    }

    /// Ends any keyframe crossfade (a seek, a reader starting).
    fn end_blend(&mut self) {
        self.blend_pos = self.blend_len;
    }

    /// Advances the smoothed volume and pan by one sample.
    fn tick(&mut self) {
        self.blend_pos = (self.blend_pos + 1).min(self.blend_len);
        self.gain += (self.target - self.gain).clamp(-self.step, self.step);
        for c in 0..2 {
            let d = self.pan_target[c] - self.pan[c];
            self.pan[c] += d.clamp(-self.pan_step[c], self.pan_step[c]);
        }
    }
}

/// The span of bus samples one mixing pass covers.
#[derive(Debug, Clone, Copy)]
struct Span {
    first: i64,
    /// One past the last sample.
    end: i64,
    /// The pass follows a seek: nothing ramps.
    fresh: bool,
}

/// A reader opened and positioned ahead of time (playback).
struct Prepared {
    source: SourceKey,
    /// The media time its next read is sequential at.
    at: Tick,
    /// `None`: the source couldn't be opened.
    reader: Option<Box<dyn SourceReader>>,
}

/// What the mixer holds for one input, for the playback prefetcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReaderState {
    /// A reader is open (or the source failed until the next jump): nothing
    /// to fetch.
    Held,
    /// A reader positioned at this media time waits to be taken.
    Prepared(Tick),
    /// Nothing: a reader has to be opened.
    Missing,
}

/// Renders the timeline's audio as interleaved stereo `f32` at one rate.
pub struct Mixer {
    rate: SampleRate,
    sample_ticks: i64,
    slots: HashMap<u64, Slot>,
    /// Readers handed in by [`Mixer::offer`], by input key.
    prepared: HashMap<u64, Prepared>,
    /// Missing readers are not opened inline (playback).
    deferred: bool,
    pass: u64,
    bus: Vec<f32>,
    scratch: Vec<f32>,
    /// One block of a keyframed input's envelope.
    curve: Vec<f64>,
    smoother: Smoother,
    keys: Vec<u64>,
    limiter: Limiter,
    /// The `start_sample` that would continue the previous render.
    next: Option<i64>,
    master_target: f32,
    master: f32,
    master_step: f32,
    /// [`CLIP_RAMP_SECS`] in samples.
    clip_ramp: f32,
}

impl std::fmt::Debug for Mixer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mixer")
            .field("rate", &self.rate)
            .field("readers", &self.slots.len())
            .field("prepared", &self.prepared.len())
            .field("next", &self.next)
            .field("master", &self.master_target)
            .finish_non_exhaustive()
    }
}

impl Mixer {
    /// A mixer producing samples at `rate`, with the limiter on and the
    /// master gain at unity.
    pub fn new(rate: SampleRate) -> Mixer {
        let hz = rate.hz() as f32;
        Mixer {
            rate,
            sample_ticks: sample_ticks(rate),
            slots: HashMap::new(),
            prepared: HashMap::new(),
            deferred: false,
            pass: 0,
            bus: Vec::new(),
            scratch: Vec::new(),
            curve: Vec::new(),
            smoother: Smoother::new(rate),
            keys: Vec::new(),
            limiter: Limiter::new(rate.hz()),
            next: None,
            master_target: 1.0,
            master: 1.0,
            master_step: 1.0 / (MASTER_RAMP_SECS * hz),
            clip_ramp: (CLIP_RAMP_SECS * hz).max(1.0),
        }
    }

    pub fn sample_rate(&self) -> SampleRate {
        self.rate
    }

    /// Master volume as linear amplitude, applied before the limiter.
    /// Changes ramp over about 10 ms; negative or non-finite is silence.
    pub fn set_master_gain(&mut self, linear: f32) {
        self.master_target = if linear.is_finite() && linear > 0.0 {
            linear
        } else {
            0.0
        };
    }

    /// Turns the safety limiter on or off (on by default). Off, overs pass
    /// through unclipped.
    pub fn set_limiter(&mut self, on: bool) {
        self.limiter.set_enabled(on);
    }

    /// Drops every cached reader and forgets the playback position.
    pub fn reset(&mut self) {
        self.slots.clear();
        self.prepared.clear();
        self.next = None;
    }

    /// Never opens a reader while rendering: an input plays once a reader
    /// for it was handed in with [`Mixer::offer`] (playback, where opening a
    /// file on the rendering thread would starve the device).
    pub(crate) fn defer_opens(&mut self) {
        self.deferred = true;
    }

    /// The samples the mix runs ahead of the output (the limiter's
    /// look-ahead).
    pub(crate) fn latency(&self) -> usize {
        self.limiter.latency()
    }

    /// Hands in a reader for input `key`, positioned so that reading at `at`
    /// is sequential (`None`: the source can't be opened). The input takes
    /// it when its playback reaches `at`, or at once if it is past that.
    pub(crate) fn offer(
        &mut self,
        key: u64,
        source: SourceKey,
        at: Tick,
        reader: Option<Box<dyn SourceReader>>,
    ) {
        self.prepared.insert(key, Prepared { source, at, reader });
    }

    /// The readers came from an opener that was replaced: each keeps
    /// playing until a reader from the new one is handed in for its input,
    /// and readers handed in before are dropped.
    pub(crate) fn invalidate_readers(&mut self) {
        for slot in self.slots.values_mut() {
            slot.stale = true;
        }
        self.prepared.clear();
    }

    /// What the mixer holds for input `key` playing `source`.
    pub(crate) fn reader_state(&self, key: u64, source: &SourceKey) -> ReaderState {
        if let Some(p) = self.prepared.get(&key)
            && p.source == *source
        {
            return ReaderState::Prepared(p.at);
        }
        match self.slots.get(&key) {
            Some(s) if s.source == *source && !s.stale && (s.reader.is_some() || s.failed) => {
                ReaderState::Held
            }
            _ => ReaderState::Missing,
        }
    }

    /// Drops the readers handed in for inputs not in `wanted`, or no longer
    /// needed because their input holds a reader.
    pub(crate) fn prune_prepared(&mut self, wanted: impl Fn(u64) -> bool) {
        let slots = &self.slots;
        self.prepared.retain(|key, p| {
            let held = slots
                .get(key)
                .is_some_and(|s| s.source == p.source && !s.stale && s.reader.is_some());
            wanted(*key) && !held
        });
    }

    /// Renders output samples `[start_sample, start_sample + out.len() / 2)`
    /// into `out` (interleaved stereo). A trailing odd value is zeroed.
    ///
    /// Continuing exactly where the previous call ended reads every source
    /// sequentially; any other `start_sample` is a seek. Spans whose time
    /// can't be represented in ticks are silent.
    pub fn render(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
        start_sample: i64,
        out: &mut [f32],
    ) {
        let frames = out.len() / 2;
        out[frames * 2..].fill(0.0);
        let lookahead = self.limiter.latency();
        let fresh = self.next != Some(start_sample);
        if fresh {
            self.limiter.reset();
            self.master = self.master_target;
            self.mix(schedule, opener, start_sample, lookahead, true);
            for &frame in self.bus.as_chunks::<2>().0 {
                self.limiter.process(frame);
            }
        }
        // `lookahead` is a few hundred samples; the sum can't overflow for
        // any start whose ticks are representable, and others are silent.
        let ahead = start_sample.saturating_add(lookahead as i64);
        self.mix(schedule, opener, ahead, frames, fresh);
        let (bus, _) = self.bus.as_chunks::<2>();
        for (o, &frame) in out.as_chunks_mut::<2>().0.iter_mut().zip(bus) {
            *o = self.limiter.process(frame);
        }
        self.next = start_sample.checked_add(frames as i64);
    }

    /// Mixes samples `[first, first + frames)` onto `self.bus[..2 * frames]`,
    /// master gain included. `fresh`: this follows a seek, so nothing ramps.
    fn mix(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
        first: i64,
        frames: usize,
        fresh: bool,
    ) {
        self.bus.clear();
        self.bus.resize(frames * 2, 0.0);
        self.pass += 1;
        self.mix_inputs(schedule, opener, first, frames, fresh);
        self.slots.retain(|_, s| s.pass == self.pass);

        for frame in self.bus.as_chunks_mut::<2>().0 {
            let delta = self.master_target - self.master;
            self.master += delta.clamp(-self.master_step, self.master_step);
            frame[0] *= self.master;
            frame[1] *= self.master;
        }
    }

    fn mix_inputs(
        &mut self,
        schedule: &dyn Schedule,
        opener: &dyn SourceOpener,
        first: i64,
        frames: usize,
        fresh: bool,
    ) {
        let Some(end) = first.checked_add(frames as i64) else {
            return;
        };
        let (Ok(t0), Ok(t1)) = (sample_start(first, self.rate), sample_start(end, self.rate))
        else {
            return;
        };
        let Ok(window) = TimeRange::from_start_end(t0, t1) else {
            return;
        };

        let span = Span { first, end, fresh };
        let inputs = schedule.inputs(window);
        let scheduled = |key: u64| inputs.iter().any(|i| i.key == key);
        // Slots whose change of level waits for the other streams of their
        // clip: mixed after those, once it is known when they sound.
        let mut waiting = Vec::new();
        for input in &inputs {
            let slot = match self.slots.remove(&input.key) {
                Some(slot) if slot.source == input.source => slot,
                // Another source on a live key (a stream switch, AU-038)
                // during playback: the old reader plays on, like one of a
                // replaced opener, until the new source's reader takes over
                // at its position, so the switch is a cut, not a gap.
                Some(slot) if self.deferred && slot.reader.is_some() => Slot {
                    source: input.source,
                    stale: true,
                    ..slot
                },
                Some(_) => Slot {
                    replaced: true,
                    ..Slot::new(input.source)
                },
                None => self
                    .adopt(input, scheduled)
                    .unwrap_or_else(|| Slot::new(input.source)),
            };
            if waits_for_streams(&slot, input, &inputs, fresh) {
                waiting.push((input, slot));
                continue;
            }
            self.mix_scheduled(slot, input, opener, span, i64::MIN);
        }
        for (input, slot) in waiting {
            let from = self.streams_sound_from(input, &inputs);
            self.mix_scheduled(slot, input, opener, span, from.unwrap_or(i64::MAX));
        }

        // Inputs that left the schedule while sounding (a mute, a delete)
        // ramp out instead of stopping dead. One that simply ended doesn't.
        if fresh {
            return;
        }
        let pass = self.pass;
        let mut keys = std::mem::take(&mut self.keys);
        keys.clear();
        keys.extend(
            self.slots
                .iter()
                .filter(|(_, s)| s.pass != pass && s.audible())
                .map(|(k, _)| *k),
        );
        for key in &keys {
            let Some(mut slot) = self.slots.remove(key) else {
                continue;
            };
            let Some(last) = slot.last.take() else {
                continue;
            };
            if sample_at(last.range.end(), self.rate) <= first {
                // It ended where it was placed.
                continue;
            }
            let mixed = self.mix_slot(&mut slot, &last, 0.0, opener, span, i64::MIN);
            if mixed && slot.audible() {
                slot.pass = pass;
                slot.last = Some(last);
                self.slots.insert(*key, slot);
            }
        }
        self.keys = keys;
    }

    /// Mixes scheduled `input`'s part of `span` with `slot` (see
    /// [`Mixer::mix_slot`]) and keeps the slot for the next pass if it is
    /// worth it.
    fn mix_scheduled(
        &mut self,
        mut slot: Slot,
        input: &MixInput,
        opener: &dyn SourceOpener,
        span: Span,
        level_from: i64,
    ) {
        let target = input.static_volume();
        if self.mix_slot(&mut slot, input, target, opener, span, level_from) {
            slot.pass = self.pass;
            slot.last = Some(input.clone());
            self.slots.insert(input.key, slot);
        }
    }

    /// The first sample of this pass from which every other stream of
    /// `input`'s clip sounds, now that they were mixed (see
    /// [`waits_for_streams`]); `None` while one waits for its reader.
    /// Streams that are silent, failed, or sounding and waiting to change
    /// level themselves hold nothing back.
    fn streams_sound_from(&self, input: &MixInput, inputs: &[MixInput]) -> Option<i64> {
        let mut from = i64::MIN;
        for other in inputs.iter().filter(|i| is_sibling(i, input)) {
            // Not here: silent (not kept), or still waiting to be mixed.
            let Some(slot) = self.slots.get(&other.key) else {
                continue;
            };
            if slot.reader.is_some() {
                from = from.max(slot.started.unwrap_or(i64::MIN));
            } else if !slot.failed {
                return None;
            }
        }
        Some(from)
    }

    /// Mixes one input's part of `span` onto the bus at static volume
    /// `target` (ramped). The volume heads for `target` from sample
    /// `level_from` on, at once when that is at or before the span; until
    /// then it keeps its current target. Returns whether the slot is worth
    /// keeping.
    fn mix_slot(
        &mut self,
        slot: &mut Slot,
        input: &MixInput,
        target: f32,
        opener: &dyn SourceOpener,
        span: Span,
        level_from: i64,
    ) -> bool {
        let Span { first, end, fresh } = span;
        slot.started = None;
        let st = self.sample_ticks;
        let s0 = sample_at(input.range.start(), self.rate);
        let s1 = sample_at(input.range.end(), self.rate);
        let (mut lo, hi) = (s0.max(first), s1.min(end));
        if lo >= hi {
            return true;
        }
        if level_from <= lo {
            slot.retarget(target, self.clip_ramp);
        }
        let pan = input.channel_gains();
        if pan != slot.pan_target {
            slot.pan_target = pan;
            let steps = slot.pan_step.iter_mut().zip(pan.iter().zip(&slot.pan));
            for (step, (to, from)) in steps {
                *step = (to - from).abs() / self.clip_ramp;
            }
        }
        if fresh {
            slot.gain = target;
            slot.pan = pan;
            slot.end_blend();
        }
        if slot.gain == 0.0 && target == 0.0 {
            // Silent and not read: a later audible block starts afresh.
            return false;
        }

        let Some(offset) = lo.checked_sub(s0).and_then(|k| k.checked_mul(st)) else {
            return true;
        };
        let Ok(mut at) = input.source_start.checked_add(Tick::from_raw(offset)) else {
            return true;
        };
        let jump = slot.next_at != Some(at);
        // `hi - lo` is at most the block length.
        slot.next_at = at.checked_add(Tick::from_raw((hi - lo) * st)).ok();
        if jump {
            slot.failed = false;
        }

        let had_reader = slot.reader.is_some();
        if self.deferred {
            if let Some(start) = self.take_prepared(slot, input, at, hi - lo) {
                // A reader waiting at a later sample of this block starts
                // there; the samples before it stay silent.
                lo += (start.raw() - at.raw()) / st;
                at = start;
            }
        } else if slot.reader.is_none() && !slot.failed {
            slot.reader = opener.open(&input.source, self.rate);
            slot.failed = slot.reader.is_none();
        }
        if slot.reader.is_none() {
            // Silent until a reader comes; one that does mid-way ramps in.
            if !fresh {
                slot.gain = 0.0;
            }
            return true;
        }
        if !had_reader {
            // The reader starts sounding: at once at the clip's start or
            // after a seek, ramping in when it joins mid-way.
            slot.started = Some(lo);
            let joins = !fresh && lo > s0 && !slot.replaced;
            slot.ramp_from(if joins { 0.0 } else { target }, self.clip_ramp);
            slot.pan = pan;
            slot.replaced = false;
            slot.end_blend();
        }
        let keys = input.gain.keys();
        if slot.keys.as_ref() != keys {
            if had_reader && !fresh {
                self.rebase(slot, at);
            }
            slot.keys = keys.cloned();
        }

        let n = (hi - lo) as usize;
        self.scratch.clear();
        self.scratch.resize(n * 2, 0.0);
        let Some(reader) = slot.reader.as_mut() else {
            return true;
        };
        if let Err(e) = reader.read(at, n, &mut self.scratch) {
            tracing::warn!(key = input.key, error = %e, "audio source read failed; playing silence");
            slot.reader = None;
            slot.failed = true;
            return true;
        }

        let fades = Fades::new(input);
        let curve = match &slot.keys {
            Some(keys) => {
                self.curve.resize(n, 0.0);
                self.smoother.fill(keys, at, &mut self.curve[..n]);
                Some(&self.curve[..n])
            }
            None => None,
        };
        let start = (lo - first) as usize;
        // A change of level held back until a later sample of this span
        // starts there (`level_from - lo` is below the span's length).
        let retarget = (level_from > lo && level_from < hi).then(|| (level_from - lo) as usize);
        let ramp = self.clip_ramp;
        // `first * st` and `end * st` were representable (t0, t1), so
        // every tick in between is too.
        let mut t = lo * st;
        let bus = &mut self.bus[start * 2..(start + n) * 2];
        let samples = self.scratch.as_chunks::<2>().0;
        let frames = bus.as_chunks_mut::<2>().0.iter_mut().zip(samples);
        for (i, (o, s)) in frames.enumerate() {
            if retarget == Some(i) {
                slot.retarget(target, ramp);
            }
            slot.tick();
            let volume = slot.blended(f64::from(slot.gain) * curve.map_or(1.0, |c| c[i]));
            let g = if fades.is_flat() {
                volume as f32
            } else {
                fades.apply(volume, t, st)
            };
            o[0] += s[0] * (g * slot.pan[0]);
            o[1] += s[1] * (g * slot.pan[1]);
            t += st;
        }
        true
    }

    /// The slot's keyframes change at media time `at` while it sounds: holds
    /// the volume heard right now (a crossfade in progress included) and
    /// crossfades from it to the new volume over the usual 10 ms, which
    /// also covers the static volume's change to or from keyframes, so that
    /// jumps to its target. Unlike rescaling the volume, this works down to
    /// silence, so dragging the line to −∞ doesn't click (AU-008).
    fn rebase(&mut self, slot: &mut Slot, at: Tick) {
        let old = slot.keys.as_ref().map_or(1.0, |k| self.smoother.at(k, at));
        slot.hold = slot.blended(f64::from(slot.gain) * old);
        slot.gain = slot.target;
        slot.blend_pos = 0;
        // `clip_ramp` is at least 1 and about 10 ms of samples.
        slot.blend_len = self.clip_ramp as u32;
    }

    /// The key of a slot whose reader can carry on as `input`'s, for an
    /// input the mixer holds nothing for: one whose key is leaving the
    /// schedule (`scheduled` says it isn't in it) and which reads the same
    /// source on the same media axis (the same media time at every
    /// timeline sample), so its next read continues where it is. That is a
    /// clip re-keyed by an edit: switched to or from "All streams mixed"
    /// (AU-038), its sound detached or that undone (TL-030). The smallest
    /// such key, when there are several.
    fn adoptable(&self, input: &MixInput, scheduled: impl Fn(u64) -> bool) -> Option<u64> {
        let axis = self.media_axis(input);
        self.slots
            .iter()
            .filter(|(key, s)| {
                **key != input.key
                    && s.source == input.source
                    && s.reader.is_some()
                    && s.last.as_ref().is_some_and(|l| self.media_axis(l) == axis)
                    && !scheduled(**key)
            })
            .map(|(key, _)| *key)
            .min()
    }

    /// Media time minus timeline time at every sample of `input`, in
    /// ticks: inputs with the same value read their source in step.
    fn media_axis(&self, input: &MixInput) -> i128 {
        let s0 = sample_at(input.range.start(), self.rate);
        i128::from(input.source_start.raw()) - i128::from(s0) * i128::from(self.sample_ticks)
    }

    /// Takes over the slot [`Mixer::adoptable`] finds for `input`: its
    /// reader, position, volume and pan carry on under the new key, and
    /// any change of level (the stream-mix factor) or pan ramps over the
    /// usual 10 ms, so a re-keyed clip neither stops nor restarts. The old
    /// key is gone from then on, so it doesn't ramp out.
    fn adopt(&mut self, input: &MixInput, scheduled: impl Fn(u64) -> bool) -> Option<Slot> {
        let key = self.adoptable(input, scheduled)?;
        let slot = self.slots.remove(&key)?;
        if !slot.stale
            && self
                .prepared
                .get(&input.key)
                .is_some_and(|p| p.source == input.source)
        {
            // Its reader is current: one opened for the new key isn't needed.
            self.prepared.remove(&input.key);
        }
        Some(slot)
    }

    /// Whether `input`, which the mixer holds nothing for, will carry on
    /// with a current reader of a key that is leaving the schedule (see
    /// [`Mixer::adoptable`]), so the playback prefetcher needn't open one.
    pub(crate) fn can_adopt(&self, input: &MixInput, scheduled: impl Fn(u64) -> bool) -> bool {
        !self.slots.contains_key(&input.key)
            && self
                .adoptable(input, scheduled)
                .and_then(|key| self.slots.get(&key))
                .is_some_and(|s| !s.stale)
    }

    /// Gives `slot` the reader handed in for `input`, if there is one and it
    /// is due: when the slot has no reader, or its stale reader has reached
    /// the new one's position. Returns the media time the new reader starts
    /// at when that is later in this block of `n` samples from `at`.
    fn take_prepared(
        &mut self,
        slot: &mut Slot,
        input: &MixInput,
        at: Tick,
        n: i64,
    ) -> Option<Tick> {
        let p = self.prepared.get(&input.key)?;
        if p.source != input.source {
            return None;
        }
        let st = self.sample_ticks;
        let ahead = p.at.raw() - at.raw();
        let later = (ahead > 0 && ahead % st == 0).then_some(ahead / st);
        if slot.reader.is_some() {
            // A current reader of this source (one handed over from another
            // key) keeps playing; a stale one plays on until the new one's
            // position.
            if !slot.stale || ahead > 0 {
                return None;
            }
        } else if later.is_some_and(|k| k >= n) {
            return None;
        }
        let p = self.prepared.remove(&input.key)?;
        slot.stale = false;
        slot.failed = p.reader.is_none();
        slot.reader = p.reader;
        slot.reader.as_ref()?;
        later.map(|_| p.at)
    }
}

/// Whether `other` is another stream of `input`'s clip.
fn is_sibling(other: &MixInput, input: &MixInput) -> bool {
    other.clip == input.clip && other.key != input.key
}

/// Whether `slot`, sounding as `input` while it plays on, changes level
/// while `input`'s clip has other streams scheduled in `inputs`. That change
/// waits until they sound too ([`Mixer::streams_sound_from`]): a clip
/// switched to "All streams mixed" (AU-038) keeps its stream's reader and
/// level while the others' readers are on their way, then ramps to the mix
/// as they ramp in, so the clip never dips.
fn waits_for_streams(slot: &Slot, input: &MixInput, inputs: &[MixInput], fresh: bool) -> bool {
    !fresh
        && slot.reader.is_some()
        && input.static_volume() != slot.target
        && inputs.iter().any(|i| is_sibling(i, input))
}

#[cfg(test)]
mod tests;
