# Audio mixing & playback

> Status: draft · Crates: freecut-audio (owner); contracts with freecut-timeline, freecut-media, freecut-project, freecut-app

This document designs FreeCut's audio engine: everything between an immutable timeline
snapshot and samples at the output device or the export encoder. It covers:
- the mix graph that is active at each timeline time;
- sample-rate and channel conversion;
- clip gain, fades, crossfades and keyframed volume, and track mute and solo;
- the master bus and limiter, meters and loudness;
- the cpal output path and the audio side of the playback clock;
- scrubbing and offline rendering for export, stems and analysis;
- the waveform-data contract.

It does not design the following. It only places them in the signal chain:
- the DSP inside noise reduction, voice polish, voice effects and EQ (AU-018, AU-019,
  AU-022, AU-034);
- the time-stretch algorithm (AU-023);
- the recording UI (AU-027);
- the music library (AU-030, AU-031).

Transport, the clock's reader math and the ring flush protocol are specified together with
video in [playback-engine.md](playback-engine.md). This document adopts that document's
`Epoch`, `PlayPlan`, feeder thread, `AudioRing` and seqlock `Stamp`. It defines what the audio
side must guarantee so that clock is correct. Where the two documents disagree, the
disagreement is listed under Risks & open questions.

---

## What FreeCut needs

### Requirements from the feature specs

| ID | Requirement (paraphrased) | Consequence for the engine |
|---|---|---|
| AU-001 | At least 32 overlapping clips mixed in real time at 48 kHz on a mid-range laptop. Overlapping clips on different tracks are summed. No overlap within one track. Offline media and disabled clips are silent. A shorter real length clamps the clip (logged correction). | Flat summing (no pairwise cascade). Same-rate 1× clips bypass resampling. One deterministic render function. The engine also supplies the exact stream length that drives TL's correction. |
| AU-002, PM-035 | Embedded audio follows its video through every edit. A/V alignment comes from PTS, including encoder priming and MP4 edit lists. Media time 0 is the first presentable frame and the A/V offset is kept. Audio shorter than the clip → silence. Audio longer → cut at the clip edge. | PCM sources are aligned to media time 0. Reads outside the stream return zeros. The clip span bounds every read. |
| AU-005, AU-007 | Gain is stored as `GainDb(i32)` in hundredths of a dB, with a −∞ sentinel. Live drags are smoothed over about 10 ms. Mute keeps the level. Floats appear only in `freecut-audio` and at the UI edge. | Conversion to linear happens once, in the engine. Live parameter smoothers. Mute is a gain stage, never a model edit. |
| AU-006 | Fade curves: Linear, Smooth (½−½cos πp), Fast (sin πp/2), Slow (1−cos πp/2). Lengths are in output time. Effective lengths are clamped (`in_eff = min(in, dur)`, `out_eff = min(out, dur − in_eff)`). A crossfade wins over a stored fade. | Fades are evaluated analytically per sample. Their endpoints are exact. Crossfades override fades on that edge. |
| AU-008, TR-027 | Volume keyframes interpolate in dB. A segment touching −∞ interpolates in linear amplitude. Keyframes are anchored in source time. Keyframes denser than one per 10 ms are smoothed. | A per-sample envelope evaluator, with an equivalence test against TL's reference evaluator. |
| AU-009, TL-042, PM-072 | Waveforms show the source signal. The overlay comes from the model, with no decoding. The peak pyramid is min/max at 256 samples per bin, ×4 per level, per stream. Bins are aggregated and never under-sampled. | Waveform data comes from `freecut-media`. The gain envelope helper lives in `freecut-timeline`. |
| AU-010 | The 16-step chain. A 32-bit float stereo master. Master gain −∞, then −60 to +12 dB. A safety limiter at −1 dBFS sample peak with about 5 ms look-ahead. Buffer 256–4096 frames (default 1024). Project rate 48 kHz (44.1 kHz in v1). High-quality sinc resampling. ITU downmix (C and surrounds −3 dB, LFE dropped, mono at unity on both sides). The equal-power law ships in the MVP. `sample = floor(ticks × rate / ticks_per_second)` in `i128`. The audio clock is master, with latency compensation. The callback never blocks, allocates or decodes. Underrun → silence plus a counter. Unplugged device → system default. No device → system clock. Device rate ≠ project rate → resample at the boundary. Seek start under 150 ms. | The whole architecture below. |
| AU-011, AU-012, TL-008 | Track mute ramps over about 10 ms. Decoding pauses for muted tracks. Solo is view state that affects preview only. Export ignores solo unless "Export as heard" is set. Muted beats soloed. | `MonitorState` is separate from the compiled plan. Dormant voices. |
| AU-013, AU-014 | Track gain −∞, then −60 to +12 dB. Track pan and clip pan use a balance law. Channel modes: Stereo, Mono mix, Left only, Right only, Swap. Multichannel is downmixed first. | 2×2 matrices per clip, balance per bus. |
| AU-015, EX-019 | Peak and RMS meters, a 1.5 s peak hold, a 20 dB/s fall, a clip LED at ≥ −0.1 dBFS **before** the limiter. M, S and I LUFS plus true peak via `ebur128`. Meters freeze on pause. Preview mute applies after the master bus and after metering, and the clock keeps running. | Meter taps on the feeder, tagged with play time. A loudness worker. |
| AU-016, EX-004, EX-007 | Scrub grains of 40–80 ms, crossfaded. Grains are dropped on fast drags. Heavy stages are used only if cached. Shuttle at 2–8× is pitch-preserved, 1/2× is slowed. Reverse shuttle is silent. | Scrub mode and shuttle mode in the renderer. |
| AU-017 | Normalization is measured offline through chain step 7. Gain = target − I, reduced so that TP + gain ≤ ceiling, then clamped to −30…+24 dB. Below −70 LUFS no gain is applied. Preview plays without the gain until analysis finishes. Export waits. | An analysis render mode and blocking sources for export. |
| AU-023, TR-036, TR-038 | Audio follows TR's exact time map per processing block and never drifts. Varispeed vs keep-pitch. Silent below 0.05×. Reverse reads the source backwards. | Closed-loop resampler ratio, anchored to the exact map every block. |
| AU-026 | Output latency is exposed for tap compensation. | Read from cpal timestamps (see the clock contract). |
| AU-028, AU-029, FX-007 | The duck envelope is a separate gain layer (offline). Audio crossfades are equal power (default) or equal gain, use handles, and fall back to a dip. Embedded audio under a video transition uses equal power in the MVP. | Plan compile resolves overlaps into per-voice crossfade segments. |
| AU-032, AU-033, EX-031 | Offline master render over exactly the export range. Stems are post-track and pre-limiter. Another output rate goes through a resampler. | `OfflineRenderer` shares the same engine. |
| EX-002 | Played samples map to ticks exactly. Video frames drop, audio is never stretched. Buffering holds the clock. Edits are re-planned "within one buffer". | The epoch and ring protocol (playback-engine.md) and plan hot-swap. |
| TL-001 | Ticks are 1/705,600,000 s. That divides every common sample rate. | Integer tick↔sample conversion. |
| TL-030, TL-040 | Only `Active` embedded audio is mixed. A compound clip's audio is the nested mix. Nesting depth ≤ 8. | Nested voices (a child engine with no master stage). |
| PM-007 | 48 kHz default. Stereo only. Changing the rate moves no clip. | Caches must not depend on the project rate. The engine restarts on a rate change. |

### Engineering requirements derived from the above

1. **Determinism.** An exported sample `n` depends only on the timeline snapshot and `n`. It
   must not depend on the block size, the start position, thread timing or wall time. Preview
   output equals export output, apart from monitor state (solo, preview mute) and smoothing
   while the user is actively editing.
2. **Real-time safety.** The device callback does no allocation, locking, I/O, logging, Arc
   dropping or FFmpeg work.
3. **Testability.** The full mixer runs in `cargo test -p freecut-audio` with synthetic
   sources, no FFmpeg and no device.
4. **Budget.** 32 stereo voices at 48 kHz with sinc resampling from 44.1 kHz, plus the master
   limiter, render at ≥ 20× real time on one core of a mid-range laptop in release builds.
   That leaves headroom for v1 DSP.

### Spec inconsistencies found while designing

- AU-023 places the keep-pitch flag in `Retime.preserve_pitch`. TR-038 places it in
  `ClipAudio.preserve_pitch`. Pick one.
- EX-002 says video shows the newest frame ≤ `t` "minus the reported output latency". AU-026
  subtracts device latency and the buffer duration from the playhead. The clock defined in
  playback-engine.md already returns the tick that is audible now. Subtracting latency again
  would double-compensate.
- TR-036 requires that a split never change any audio sample. That holds exactly only if the
  engine treats a source-continuous split as one voice. This design does that (see
  continuity chains).

---

## How the reference editors do it

All paths are relative to each repository root. The code was studied read-only. Algorithms
are described in our own words, and excerpts are limited to a few lines.

### Olive (C++/Qt, GPL-3.0-or-later)

**Structure.** Audio flows through Olive's node graph like video.
- `Track::ProcessAudioTrack` (`app/node/output/track/track.cpp`) allocates a `SampleBuffer`
  for the requested range. It copies each block's samples into it and applies speed or
  reverse per block.
- Speed goes through `SampleBuffer::speed` (varispeed) or, with "maintain pitch", through an
  FFmpeg `atempo` graph (`app/audio/audioprocessor.cpp`). That graph chains several `atempo`
  filters because one filter only accepts 0.5–2.0.
- The authors flag the problem themselves (`track.cpp`, GPL-3.0):

  ```
  // FIXME: This is not the best way to do this, the TempoProcessor works best
  //        when it's given a continuous stream of audio, which is challenging
  //        in our current "modular" audio system. ...
  //        users may hear clicks and pops in the audio during preview due to this
  ```

  A stateful DSP stage (a stretcher) run on independent chunks re-initializes at every chunk
  edge. **Lesson L1:** stateful stages need persistent per-voice state across blocks.

**Keyframed parameters.**
- `RenderProcessor::ProcessSamples` (`app/render/renderprocessor.cpp`) re-evaluates every
  non-sample input of a node **once per sample**, through a full input traversal.
- It builds the sample time as
  `rational::fromDouble(range.in().toDouble() + i / sample_rate)`, a round trip through
  `double`. That is slow, and it gives up exactness at exactly the place the rational type
  was meant to keep it.
- **L2:** compile envelopes into a cheap per-sample evaluator, from integer sample indices.

**Gain units.**
- `VolumeNode` (`app/node/audio/volume/volume.cpp`) stores **linear** amplitude (default 1.0)
  and only displays it in dB (`FloatSlider::kDecibel`). Keyframes therefore interpolate in
  amplitude, which sounds uneven.
- `app/common/decibel.h` maps silence to −200 dB and treats linear values under 1e-6 as 0.
- **L3:** store dB, interpolate in dB, and special-case −∞.

**Pan and transitions.**
- `PanNode` (`app/node/audio/pan/pan.cpp`) is a balance law for stereo only. Other channel
  counts pass through.
- `TransitionBlock::TransformCurve` (`app/node/block/transition/transition.cpp`) offers
  linear, exponential (p²) and "logarithmic" (√p). Applied to both sides, √p gives constant
  power.
- `CrossDissolveTransition::SampleJobEvent` evaluates progress per sample in `double`.

**Sources: the "conform" cache.**
- `FFmpegDecoder::ConformAudioInternal` (`app/codec/ffmpeg/ffmpegdecoder.cpp`) decodes the
  **whole stream once**. It runs it through `swr_convert` to the project format and writes
  planar float files.
- `Decoder::RetrieveAudioFromConform` (`app/codec/decoder.cpp`) then reads any range by
  random access. It shifts by `GetAudioStartOffset()` (stream start minus container start)
  and pads with silence before 0 and after the end.
- Until the conform finishes, `RetrieveAudio` returns `kWaitingForConform` and nothing plays.

Pitfalls in that loop:
- It never flushes the resampler at EOF (no `swr_convert(…, NULL, 0)` drain), so the
  resampler's buffered tail is lost.
- It appends decoded frames back to back with no PTS check. A stream with timestamp gaps
  therefore shifts everything after the gap.
- **L4:** a conform cache gives sample-exact random access, but it must fill gaps by PTS,
  drain every stateful converter, and play something before the cache is ready.

**Playback path.**
- `ViewerWidget::QueueNextAudioBuffer` (`app/widget/viewer/viewer.cpp`) requests rendered
  audio in 250 ms ranges (`kAudioPlaybackInterval = 1/4`) and pre-queues two. That is about
  0.5 s of start latency.
- `AudioPlaybackCache` (`app/render/audioplaybackcache.h`) stores pre-rendered audio on disk
  in segments, so it can be "shifted" cheaply after edits.
- The PortAudio callback `OutputCallback` (`app/audio/audiomanager.cpp`) reads from
  `PreviewAudioDevice::readData` (`app/render/previewaudiodevice.cpp`), which:
  - takes a `QMutex`;
  - copies;
  - then does `buffer_ = buffer_.mid(copy_length)`, which reallocates;
  - and may `emit Notify()`, all on the real-time thread.
- **L5:** a lock, an allocation and a signal in the device callback.

**Clock.**
- `ViewerPlaybackTimer::GetTimestampNow` (`app/widget/viewer/viewerplaybacktimer.cpp`)
  derives the video position from `QDateTime::currentMSecsSinceEpoch()`. That is wall-clock
  time, neither monotonic nor tied to the audio device. Audio is pushed independently, so the
  two drift apart over long playback and jump when the system clock is adjusted.
- **L6:** the audio device is the only clock, read through a monotonic mapping.

**Scrubbing.** `PushScrubbedAudio` renders a hard-coded 20 ms window at the playhead. It
calls `ClearBufferedOutput()` and pushes, with no windowing, so every grain has hard edges.

**Waveforms.** `AudioVisualWaveform` (`app/audio/audiovisualwaveform.h`) keeps per-channel
`{min, max}` mipmaps at power-of-two rates from 1/8 to 1024 summaries per second. Each level
is built from the one above it, and `GetSummaryFromTime` folds a range.

### MLT framework (C, LGPL-2.1-or-later; some modules GPL-2.0-or-later)

**Frame-based audio.**
- Each video frame carries a variable number of samples.
- `mlt_audio_calculate_frame_samples` (`src/framework/mlt_audio.c`) avoids accumulated
  rounding by differencing cumulative counts. It takes `float fps`, so NTSC rates are not
  exact (LGPL-2.1):

  ```c
  return mlt_audio_calculate_samples_to_position(fps, frequency, position + 1)
         - mlt_audio_calculate_samples_to_position(fps, frequency, position);
  ...
  samples = (int64_t) ((double) position * (double) frequency / (double) fps
                       + (position < 0 ? -0.5 : 0.5));
  ```

  The cumulative-difference idea is right. FreeCut gets it for free from integer ticks.

**`transition_mix`** (`src/modules/core/transition_mix.c`) mixes track B into A.
- Laws:
  - `mix_audio` uses `sin`/`cos` of `mix·π/2` when power is requested;
  - `sum_audio` adds;
  - `combine_audio` adds through a one-pole low-pass.
- The magic numbers are documented in `transition_mix.yml`: `start = −1` is a linear
  crossfade and `start = −2` is equal power.
- Each frame ramps from `audio.previous_mix` to `audio.mix` across the frame's samples. That
  is per-frame zipper suppression, reset when frame positions are not consecutive.
- Because A and B frames can carry different sample counts, the transition keeps two FIFOs
  per instance (`float src_buffer[192000*6]`, `dest_buffer[...]`, about 9 MB per transition).
  It drains them with a 1 ms latency target. The file records what happens without them
  (LGPL-2.1):

  ```c
  // However, the simple and stupid approach drops samples. Over time, this
  // can accumulate and cause an A/V sync drift, which addressed in b2640656
  // by saving the unused samples in a buffer and then using them first on the
  // next iteration.
  ```

  **L7:** never let block or frame boundaries decide sample counts. Mix on one absolute
  sample grid.
- Ducking (`apply_ducking`) also lives here. It takes the RMS of the A buffer against a
  threshold and smooths B's gain with `exp(-samples/time_samples)`. It resets on any
  discontinuity, so its output depends on where playback started.

**Volume** (`src/modules/normalize/filter_volume.c`, GPL-2.0-or-later).
- `gain` is accepted in dB or linear. The animated `level` property is in dB.
- Each frame ramps linearly from `_previous_gain`, which resets on non-consecutive positions.
- An optional soft `tanh` limiter is available.
- The 16-bit "normalize" branch appears to read and write `p[0]` for every sample (lines
  ~255–262). The float branch indexes correctly.
- **L8:** a rarely used branch went untested. Every mode needs a golden test.

**Panning and channel conversion.**
- `filter_panner.c` (`src/modules/core/`, LGPL) implements "front L/R balance" as a balance
  law, with per-frame ramps.
- `filter_audiochannels.c` (LGPL) downmixes 5.1 → stereo as `front + 0.707·C + 0.5·S`,
  assuming WAV channel order. It does not normalize and clamps only in the 16-bit path.
- Missing channels are filled by duplicating the existing channels round-robin.
- **L9:** downmix by the decoder's reported layout, never by assumed position, and specify
  headroom handling.

**Time remap** (`src/modules/core/link_timeremap.c`, LGPL).
- Speeds outside 0.1–10× return silence.
- Varispeed works by relabelling the frame's sample rate (`out.frequency = frequency ×
  sample_count / link_sample_count`) and then resampling.
- Pitch compensation hands off to the Rubber Band filter (`src/modules/rubberband/`,
  GPL-2.0-or-later).
- Because source time is a `double`, the link needs a ±4-sample "continuity margin"
  heuristic (`SAMPLE_CONTINUITY_ERROR_MARGIN`) to avoid gaps and doubles between frames.
- **L10:** exact rational positions remove the need for continuity heuristics.

**Resampling** (`src/modules/avformat/link_swresample.c`, LGPL).
- The `swr` context is rebuilt whenever the frame position is not the expected one.
- When the input frame cannot fill the output, future frames are pulled in.
- If the output is still short, existing samples are **duplicated** to fill it
  (`mlt_audio_copy(&out, &out, …)`).
- `filter_swresample.c` returns silence while "precharge" is needed.
- **L11:** a resampler has a delay. Prime it with real preceding input and discard exactly
  its reported delay. Never pad or duplicate.

**Source sync** (`src/modules/avformat/producer_avformat.c`, LGPL).
- `seek_audio` reads forward instead of seeking when the target is less than 12 frames
  ahead. Otherwise it seeks backward.
- The decode path then discards ("ignore") samples until the packet PTS reaches the request.
  If it lands ahead of the request, it seeks one second earlier and retries.
- Audio PTS is offset by the **video's** `first_pts` so the two streams align.
- WMA Pro gets a wider tolerance.
- **L12:** sample-exact streaming reads need pre-roll before the target, then a PTS-based
  discard, with per-codec tolerances.

**Loudness.**
- `filter_loudness.c` (`src/modules/plus/`, LGPL) is two-pass with libebur128. The analysis
  aborts on any skipped frame ("Bad frame sequence"). It applies `target − measured` with no
  peak ceiling and records sample peak only.
- `filter_dynamic_loudness.c` (LGPL) adapts in real time:
  - it uses window loudness for the first 400 ms;
  - it limits the gain change rate (`max_rate`);
  - it interpolates the coefficient geometrically per sample (linear in dB);
  - it resets on discontinuity.
  Its result depends on where playback started.
- **L13:** loudness gain must be a fixed per-clip value from an offline pass with a
  true-peak ceiling, not an adaptive filter.

**Seams.** `filter_audioseam.c` (`src/modules/core/`, LGPL) runs at clip boundaries in a
playlist. If the level jump between the last sample of one clip and the first of the next
exceeds a dB threshold, it crossfades 1000 samples of the **reversed** previous frame into
the new clip. **L14:** hard cuts click, and a declick at discontinuous edges is worth having.

**Meters.** `filter_audiolevel.c` (`src/modules/normalize/`, GPL-2.0-or-later) has IEC 268
meter scaling. Its level is computed from **at most the first 200 samples** of each frame
(`num_samples = *samples > 200 ? 200 : *samples`). **L15:** meters must see every sample.

**Consumers.**
- `consumer_sdl2_audio.c` and `consumer_rtaudio.cpp` (LGPL): the device callback
  (`sdl_fill_audio`, `callback`) locks a pthread mutex, copies, and `memmove`s the remaining
  buffer down. That is O(buffer) work in the callback, with a lock (L5 again).
- Sample counts per frame come from a function-static `counter`, not from the frame's
  position.
- Non-1× speeds are written as silence unless `scrub_audio` is set.
- Video is scheduled against `gettimeofday` using accumulated audio durations
  (`playtime += duration`). It re-baselines when the queue empties. Audio backpressure paces
  the producer, which is effectively an audio clock read through the wall clock.

### Shotcut (C++/Qt on MLT, GPL-3.0-or-later)

**Track summing.**
- Every new track gets an MLT `mix` transition planted from track 0 to it, with
  `always_active=1` and `sum=1` (`MultitrackModel::addAudioTrack` / `addVideoTrack` in
  `src/models/multitrackmodel.cpp`).
- N tracks therefore mean N−1 cascaded pairwise mixes, each with its own FIFOs.
- Order matters (ducking reads the mix so far), so `reorderMixTransitions`
  (`src/models/mixtransitionorder.cpp`) re-plants them in UI order.
- **L16:** sum buses in one pass with a fixed, documented order. Do not chain mixers.

**Ducking UI.** `TrackPropertiesWidget::on_duck*` (`src/widgets/trackpropertieswidget.cpp`)
writes the threshold, attenuation and fade times onto the track's mix transition. That makes
it a live sidechain against the accumulated upper tracks.

**Fades.**
- Fade-in is a `volume` filter whose dB `level` animates from **−60 dB** to 0 dB
  (`multitrackmodel.cpp`, the fade-in update path using `anim_set("level", -60, 0)`).
- The fade therefore starts at about 0.001 amplitude, not at silence.
- Its shape is linear in dB unless the keyframe type changes it.
- **L17:** fades must reach exact silence and have the spec's defined amplitude curves.

**Player.**
- `Controller::setVolume` (`src/mltcontroller.cpp`) sets the consumer's linear `volume` and
  mutes while paused.
- On seek, `refreshConsumer(scrubAudio)` sets `scrub_audio` and refreshes, which plays one
  frame of audio per seek.

**Waveforms.**
- `AudioLevelsTask::run` (`src/models/audiolevelstask.cpp`) stores **one 8-bit level per
  channel per video frame**, taken from the `audiolevel` filter (with the 200-sample limit
  above). It is cached as an image in the thumbnail database.
- That is too coarse to zoom into, and transients that fall outside the measured samples are
  missed.

### Kdenlive (C++/Qt on MLT, GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)

**Mixer strips.**
- `MixerWidget` (`src/audiomixer/mixerwidget.cpp`) attaches three MLT filters to each track
  tractor: `volume` (dB `level`, range −50…+24), `panner` and `audiolevel`.
- Master mute sets `level` to −1000 dB.

**Meter alignment.** Levels arriving from the `audiolevel` filter are stored by **frame
position** (`m_levels[pos]`). `updateAudioLevel(pos)` shows the entry for the frame being
displayed. Meters therefore line up with what is seen and heard, however deep the pipeline
is. **L18:** tag meter data with play time and display it by clock.

**Solo.**
- `MixerManager` (`src/audiomixer/mixermanager.cpp`) implements solo by writing the
  persistent track `hide` property (`"3"`) on every other track. It remembers what it
  changed in `m_soloMuted`.
- Shift-click adds a long branch of special cases. Monitoring state leaks into the saved
  document.
- **L19:** solo is a monitoring mask evaluated in the mixer, never a document edit.

**Same-track audio mixes.** `TrackModel` (`src/timeline2/model/trackmodel.cpp`) plants a
`mix` transition with `start = -1`, which is a **linear** (equal-gain) crossfade. Equal
power is only available through `start = -2`.

**Clip normalize.** The checkbox in clip properties
(`src/mltcontroller/clippropertiescontroller.cpp`) adds `dynamic_loudness`. That is the
adaptive filter from L13.

**Waveforms.**
- `computePeaks` (`src/jobs/audiolevels/generators.cpp`) stores 5 points per frame
  (`AUDIOLEVELS_POINTS_PER_FRAME` in `src/definitions.h`) of `abs`-max as `int16`.
- `std::abs` of an `int16_t` −32768 is 32768, which converts back to −32768 in `int16_t`. A
  full-scale negative sample therefore never wins the `max`, so clipped audio shows smaller
  peaks than it has.
- **L20:** compute min and max separately, never `abs` in the narrow type.

### libopenshot (C++, LGPL-3.0-or-later)

**Time and mixing.**
- Clip positions are `double` seconds converted to frames with `llround`.
- `Frame::GetSamplesPerFrame` (`src/Frame.cpp`) uses floating cumulative counts. It then
  forces them to be divisible by the channel count, which mixes up frames and interleaved
  samples.
- `Timeline::add_layer` (`src/Timeline.cpp`) mixes per video frame (LGPL-3.0):

  ```cpp
  float previous_volume = source_clip->volume.GetValue(clip_frame_number - 1);
  float volume = source_clip->volume.GetValue(clip_frame_number);
  ...
  source_frame->ApplyGainRamp(channel_mapping, 0, source_frame->GetAudioSamplesCount(), previous_volume, volume);
  ```

  The ramp across frame *n* runs from the value at *n−1* to the value at *n*. The envelope is
  therefore sampled at frame rate and lags one frame behind the picture (L2).

**Overlap handling.**
- `VOLUME_MIX_AVERAGE` divides every overlapping clip by the **sum of their volumes**.
  `VOLUME_MIX_REDUCE` multiplies by 0.77.
- A clip's loudness therefore changes when an unrelated clip overlaps it.
- There is no master limiter.
- A frame whose channel count differs from the timeline's is silently skipped.
- **L21:** sum plainly, and protect the output with a limiter.

**Transitions.** `ResolveTransitionAudioGains` applies equal-power fades to clips covered by
a mask transition flagged for audio.

**Playback.**
- `AudioPlaybackThread` (`src/Qt/AudioPlaybackThread.cpp`) connects a JUCE
  `AudioTransportSource` with **read-ahead 0**. `AudioReaderSource::getNextAudioBlock`
  (`src/AudioReaderSource.cpp`) therefore runs on the device callback.
- That function can call `reader->GetFrame()` (decode and composite) unless the playback
  cache path is active.
- It also contains a good pattern: a seek generation counter that is odd while a seek is
  being written. The callback discards work from older generations with no lock. That is a
  seqlock in all but name.
- Non-1× speeds output silence.
- Device opening falls back through the rates {requested, 48000, 44100, 22050}.

**Clock.** `PlayerPrivate::run` (`src/Qt/PlayerPrivate.cpp`) paces video with
`std::chrono::system_clock` sleeps. It re-baselines after stalls while audio free-runs, so
there are two clocks.

**Waveforms.** `AudioWaveformer` (`src/AudioWaveformer.h`) keeps max and RMS per bucket.

### OpenTimelineIO (C++, Apache-2.0)

- There is no audio mixing schema. A track is just `Track::Kind::audio`
  (`src/opentimelineio/track.h`).
- `RationalTime` holds a `double` value and a `double` rate (`src/opentime/rationalTime.h`),
  so it is not exact.
- For interchange (PM-103), `ClipAudio` must travel in a FreeCut metadata namespace. The
  engine takes nothing from OTIO.

### Comparison

| Concern | Olive | MLT / Shotcut | Kdenlive | libopenshot | FreeCut |
|---|---|---|---|---|---|
| Mix structure | Node graph per range | Pairwise `mix` per track | MLT plus per-track filters | Per-frame `add_layer` | Compiled flat plan: voices → buses → master |
| Unit of work | 250 ms ranges | 1 video frame (variable samples) | Same as MLT | 1 video frame | Absolute sample grid; any block size, same result |
| Time | Rational, but per-sample doubles | Integer frames, `float` fps | Same as MLT | `double` seconds | `i64` ticks, exact sample indices |
| Sources | Full conform to disk, silence until done | Stream, seek + discard | Same as MLT | Reader + FrameMapper per frame | Native-rate stereo PCM cache plus a streaming fill |
| Gain automation | Linear amplitude, per-sample graph | Per-frame dB ramps | Same as MLT | Per-frame ramp, one frame late | Per-sample, dB domain, −∞ linear |
| Crossfade law | lin / p² / √p | lin (−1), equal power (−2) | Linear | Equal power (mask) | Equal power default, equal gain option |
| Output protection | None | `tanh` in volume normalize | None | Average/reduce hacks | Look-ahead sample-peak limiter |
| Callback | Mutex + realloc + signal | Mutex + memmove | Same as MLT | May decode | Wait-free SPSC, atomics only |
| Clock | Wall clock | Audio backpressure + `gettimeofday` | Same as MLT | `system_clock` | Device frame stamps (seqlock) |
| Scrub | 20 ms unwindowed | 1 frame per refresh | Same as MLT | None | 40–80 ms windowed, crossfaded, latest wins |
| Normalize | — | Two-pass (no ceiling) or adaptive | Adaptive | — | Offline two-pass with TP ceiling |
| Solo | — | — | Rewrites persistent mute | — | Monitoring mask |
| Waveform | min/max mipmaps | 8 bits per frame | `abs`-max ×5 per frame (int16 bug) | max + RMS | min/max pyramid 256×4, native rate |

---

## Recommended FreeCut design

### 1. Overview

```
 UI thread (freecut-app)                                   freecut-audio
 ┌─────────────────────────────┐ publish(Arc<SequenceSnapshot>, version)
 │ edits → new snapshot        │──────────────┐
 │ transport (playback-engine) │ AudioCmd     ▼
 │ reads Clock / MeterReader   │─────────▶ ┌────────────────────┐ latest-wins ┌──────────────────────┐
 └─────────────────────────────┘           │ plan compiler      │────────────▶│ feeder thread         │
                                           │ (worker, pure)     │ ArcSwap     │  MixEngine::render    │
                                           │ MixView → MixPlan  │ <MixPlan>   │  voices→buses→master  │
                                           └────────────────────┘             │  device-rate stage    │
                                                                              └──────┬─────┬──────────┘
                    freecut-media                                    AudioRing (rtrb)│     │ MeterFrame ring
 ┌───────────────────────────────────────────────┐   PcmReader                       ▼     ▼
 │ AudioSession (streaming fill, PTS-exact)      │◀──── try_read / prefetch   cpal callback   UI meters,
 │ PcmCache (conformed native-rate stereo f32)   │                            (RT, atomics)   loudness worker
 │ PeakPyramid (built in the conform pass)       │                            Stamp seqlock ─▶ AudioClock
 └───────────────────────────────────────────────┘

 Export / stems / analysis: OfflineRenderer → the same MixEngine, blocking sources, no ring, no device.
```

There is one render function, `MixEngine::render(plan, pos, mode, out)`. It is used by:
- the feeder, for playback;
- the scrub path;
- `OfflineRenderer`, for export, stems and analysis.

Every difference between preview and export is a `RenderMode` value. None is a separate code
path.

### 2. Module layout (`freecut-audio`)

```
freecut-audio/src/
  lib.rs            AudioEngine (control handle), OfflineRenderer, re-exports
  error.rs          AudioError, PlanError, SourceError (thiserror)
  time.rs           SampleRate, SampleSpan, tick↔sample (exact), SourceCursor math
  units.rs          gain_lin(GainDb), balance law, cents → ratio   (floats start here)
  plan/
    mod.rs          MixPlan, BusSpec, VoiceSpec, Segment, MasterSpec (immutable)
    compile.rs      compile(&MixView, &CompileOptions) -> Result<MixPlan, PlanError>   (pure)
    chains.rs       continuity chains (merge source-continuous clips into one voice)
    xfade.rs        crossfade / transition overlap → per-segment ramps (AU-029, FX-007)
    index.rs        SpanIndex: voices overlapping a sample span, O(log n + k)
  engine/
    mod.rs          MixEngine::render, RenderMode, MonitorState, RenderReport
    voice.rs        Voice: source cursor, resampler, stateful DSP, smoothers
    bus.rs          TrackBus (gain, balance, mute/solo ramps)
    master.rs       MasterBus (gain, pre/post meter taps, limiter)
    nested.rs       compound-clip voices (child engine without a master stage)
  dsp/
    envelope.rs     per-sample gain (keyframes in dB, −∞ in linear, duck layer)
    fade.rs         FadeCurve, XfadeLaw, edge declick
    matrix.rs       ChannelMap → 2×2, balance pan
    resample.rs     VoiceResampler (rubato wrapper, bypass, priming, closed loop)
    stretch.rs      TimeStretch trait (AU-023, v1; backends behind features)
    limiter.rs      look-ahead sample-peak limiter
    smooth.rs       ParamSmoother (10 ms), sanitize (NaN/Inf → 0)
    meter.rs        PeakRms accumulators, MeterBallistics (pure, used by the UI)
  source/
    mod.rs          PcmProvider / PcmReader traits, PcmKey, ReadMode
    media.rs        adapter over freecut-media (feature "media", default on)
    synthetic.rs    sine, noise, impulse, ramp, ones (tests and benches)
  playback/         jointly specified with playback-engine.md
    clock.rs        PlaybackClock, AudioClock, MonotonicClock, Stamp seqlock
    ring.rs         AudioRing + EpochMark ring, SharedOutput atomics
    feeder.rs       feeder thread: PlayPlan → MixEngine → device stage → ring
    output.rs       cpal device policy, callback body `fill()` (cpal-free, testable)
    scrub.rs        scrub grains (AU-016)
    record.rs       capture → ring → WAV writer thread (AU-027)
  analysis/
    loudness.rs     clip measurement for AU-017 (ebur128), live loudness worker
    duck.rs         speech detection → DuckEnvelope points (AU-028)
    feed.rs         decode-and-downmix feed for beat analysis (AU-025)
  offline.rs        OfflineRenderer: export, stems, analysis renders
```

Dependencies:
- `freecut-audio` → `freecut-timeline`, for `MixView`, `Tick`, `ClipAudio`, `PlayPlan`, and
  the time-map functions `source_time_at` and `local_rate`.
- `freecut-audio` → `freecut-media`, behind the default feature `media`, for the adapter in
  `source/media.rs`.
- `cargo test -p freecut-audio --no-default-features` builds without FFmpeg.

> **Revised (review, 2026-10-12; README §1.2, §4.1).** An optional edge cannot work: the
> public API names `freecut_media::cancel::CancelToken` (`OfflineRenderer::render`,
> `ReadMode::Blocking`) and `freecut_media::AudioBlock` (`OfflineMix`), so without the feature
> the offline mixer would not compile. `freecut-media` now keeps `cancel` and the plain wire
> types unconditional and puts FFmpeg and every decode, encode and probe module behind its
> default feature `ffmpeg`. `freecut-audio` depends on it unconditionally with
> `default-features = false`; audio's `media` feature forwards to `freecut-media/ffmpeg` and
> enables `source/media.rs`.

### 3. Time and units

Rules:
- Positions are TL-001 ticks.
- Sample indices are `i64`, derived exactly.
- Floats appear only for signal values, gains, and the progress fraction `p`, which is
  computed from integers and then converted.

```rust
// freecut-audio/src/time.rs (sketch; Tick and TICKS_PER_SECOND come from freecut-timeline)
use freecut_timeline::time::{Tick, TICKS_PER_SECOND};

/// A mixing rate that divides the tick rate, so tick <-> sample conversion is exact.
/// Accepted: 8k, 11.025k, 16k, 22.05k, 24k, 32k, 44.1k, 48k, 88.2k, 96k, 176.4k, 192k.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SampleRate(u32);

impl SampleRate {
    pub fn new(hz: u32) -> Result<Self, AudioError> {
        if hz == 0 || TICKS_PER_SECOND % i64::from(hz) != 0 {
            return Err(AudioError::UnsupportedRate(hz));
        }
        Ok(Self(hz))
    }
    pub fn hz(self) -> u32 { self.0 }
    pub fn ticks_per_sample(self) -> i64 { TICKS_PER_SECOND / i64::from(self.0) }

    /// The sample whose interval [k·T, (k+1)·T) contains `t`. Floor also holds for
    /// negative ticks. This equals AU-010's floor(ticks × rate / ticks_per_second).
    pub fn sample_at(self, t: Tick) -> i64 { t.get().div_euclid(self.ticks_per_sample()) }

    pub fn tick_of(self, sample: i64) -> Result<Tick, AudioError> {
        sample.checked_mul(self.ticks_per_sample()).map(Tick::new).ok_or(AudioError::TimeOverflow)
    }

    /// Half-open sample span of [start, end). Flooring both edges means abutting clips
    /// tile the sample line with no gap and no overlap, including at 30000/1001 fps,
    /// where frame edges fall between samples (1601.6 samples per frame at 48 kHz).
    pub fn span_of(self, start: Tick, end: Tick) -> SampleSpan {
        SampleSpan { start: self.sample_at(start), end: self.sample_at(end) }
    }
}
```

**Rates that do not divide the tick rate.**
- Odd source rates (for example 37.8 kHz) never meet a tick directly. Source positions are
  computed as `s_ticks × rate / TPS` in `i128`, giving an integer frame plus a fractional
  phase. The resampler absorbs the phase.
- Odd device rates (for example 384 kHz) go through the generic `i128` path in the clock
  (playback-engine.md step 4).

**Gain units.**
- `GainDb(i32)` in hundredths of a dB stays the stored form (AU, shared definitions).
- `units::gain_lin(g)` returns `0.0` for `GainDb::NEG_INF` and **exactly `1.0` for 0 dB**,
  through a special case and not `powf`. A default clip is then a bit-exact pass-through,
  which null tests rely on.
- Pan is an integer from −100 to +100. Cents are `i32`.

### 4. The mix graph per timeline time: `MixPlan`

FreeCut has no general node graph (compare Olive). The chain is fixed by AU-010, so the
"graph" is a **compiled, immutable plan**:
- a list of **voices** (one per sounding source run);
- **buses** (one per audio-bearing track);
- one **master**.

It is compiled from TL's read-only `MixView` (TL-008) by a pure function. At any sample span,
the active graph is `index.overlapping(span)` → voices → their buses → master.

```rust
// freecut-audio/src/plan/mod.rs (sketch)
pub struct MixPlan {
    pub doc_version: u64,
    pub rate: SampleRate,
    pub master: MasterSpec,          // gain (linear), limiter on/off, ceiling
    pub buses: Vec<BusSpec>,         // fixed order = sequence track order (deterministic sums)
    pub voices: Vec<VoiceSpec>,      // sorted by (bus, span.start, key)
    pub end: i64,                    // sequence end, in samples
    index: SpanIndex,
}

pub struct BusSpec {
    pub track: TrackId,
    pub gain: f32,                   // linear; −∞ → 0.0
    pub balance: Balance,            // AU-013 balance law
    pub muted: bool,                 // TrackFlags.muted (model state; solo is not here)
}

pub struct VoiceSpec {
    pub key: VoiceKey,               // stable across edits: (seq path, chain head clip, stream)
    pub bus: BusIdx,
    pub span: SampleSpan,            // audible span, including crossfade handles
    pub source: SourceSpec,
    pub map: SourceMap,              // exact t → s for the whole chain (TR-036)
    pub segments: SmallVec<[Segment; 1]>, // one per clip in the continuity chain
    pub continuity: u64,             // hash of everything that state depends on
    pub source_dsp: SourceDspSpec,   // steps 1–4 (isolation, channel map, clarity, NR, polish)
    pub timeline_dsp: TimelineDspSpec, // step 7 (voice effect, filters)
}

pub enum SourceSpec {
    Media { key: PcmKey },                      // conformed, processed or stem PCM
    Nested { plan: std::sync::Arc<MixPlan> },   // compound clip: nested mix before the master stage
    Silence,                                    // offline media, or a stream that is missing
}

pub struct Segment {                            // one clip's contribution inside the voice
    pub clip: ClipId,
    pub span: SampleSpan,                       // the clip's own samples (abutting segments tile)
    pub gain: GainSpec,                         // normalize × envelope × duck × fades × xfade × mute/enable
    pub matrix: ChannelMatrix,                  // channel map ∘ clip balance (2×2)
}
```

**Compile rules** (`plan::compile`, pure, unit-tested):

1. **Audible voices.** A voice is created for:
   - each audio clip on an audio track;
   - each video clip whose embedded audio is `EmbeddedAudio::Active` (TL-030), one voice per
     enabled stream (AU-002 v1 "all streams mixed");
   - each compound clip that has audio (TL-040).

   Disabled clips (TL-036) and muted clips still get a voice whose gain is 0. The renderer
   skips voices that are silent for a whole span (see dormancy), but keeping them in the plan
   means unmuting needs no recompile.
2. **Spans.** `span = rate.span_of(clip.start, clip.end)`, then extended into handles by
   crossfades (step 6). Reads are clamped to the clip's source range plus handles. Past
   `out`, the source returns silence (TR-036's rounded last frame).
3. **Continuity chains (exact split invariance).** Adjacent clips on the same track are
   merged into **one** `VoiceSpec` when all of these hold:
   - they share the asset and stream;
   - they share the same retime rate and direction;
   - the right clip's source in-point equals the left clip's source out-point mapped through
     the time map;
   - their source-time DSP settings (steps 1–6) are identical.

   Each clip stays a `Segment` with its own gain. The resampler and stateful DSP therefore
   run uninterrupted across a split. TR-036's "split never changes any audio sample" then
   holds bit for bit, and no declick is applied inside a chain.
4. **Gain spec.**
   - Effective fade lengths are computed with TL/AU-006 clamping helpers from
     `freecut-timeline`, which the UI overlay also uses (one source of truth). The results
     are converted to sample counts.
   - Keyframes are converted to `(sample-in-source-time, GainDb, Interpolation)` lists.
   - The duck envelope is converted to clip-local samples.
   - The normalize gain is taken from the analysis cache. It is absent until measured
     (AU-017).
   - The mute and enable flags are applied.
5. **Matrix.** `ChannelMap` (Stereo, MonoMix, LeftOnly, RightOnly, Swap) is composed with the
   clip balance into one 2×2 matrix.
6. **Crossfades.**
   - AU-029 track crossfades: the effective start, length and handle extension come from a
     TL helper `effective_audio_crossfade(track, left)`, so the inspector and the mixer
     agree.
   - FX-007 transition overlaps come from TL's `audio_overlaps()`.
   - The outgoing segment's span is extended past its out-point, and the incoming segment's
     span starts before its in-point. Both get an `Xfade { start, len, law, side }`.
   - The dip fallback (no handles) becomes two half-length fades.
   - Where a crossfade exists, it replaces the stored fade on that edge (AU-006).
7. **Edge declick** (FreeCut default, see Open questions):
   - A voice edge is a hard edge when the next or previous sample on the same bus does not
     continue the same source. At a hard edge with no fade and no crossfade, a 64-sample
     (about 1.3 ms) raised-cosine ramp is applied inside the clip.
   - Chain-internal edges are never hard edges.
   - Golden tests use `CompileOptions { declick: false }` for exactness.
8. **Dormancy hints.** Voices on muted buses are marked `dormant_when_muted`. Read-ahead
   pauses for them (AU-011). Solo is applied at render time from `MonitorState`, not compiled
   in, so toggling solo needs no recompile.
9. **Determinism.** `voices` are sorted by `(bus, span.start, key)`, and buses keep track
   order. Float summation order is then fixed, independent of hash-map iteration, and
   identical between preview and export.

**Compile cost and hand-off.**
- Compiling is O(clips). For a 5 000-clip project it takes a few milliseconds.
- It runs on a worker, not on the UI thread or the feeder. It is latest-wins: when several
  document versions queue up, only the newest is compiled.
- The result is published through `arc_swap::ArcSwap<MixPlan>`. The feeder loads it once per
  render call. Old plans are dropped on the feeder or compile thread, never in the callback.

**Requirement on TL.** `MixView` must be readable from an `Arc<SequenceSnapshot>` that is
cheap to produce per edit (structural sharing). Slider drags at 60 Hz must not deep-clone
large timelines.

### 5. Sources, sample-rate and channel conversion

#### 5.1 The PCM source contract (`freecut-media`, consumed through a trait)

```rust
// freecut-audio/src/source/mod.rs
/// Random-access stereo PCM at the stream's native rate. Frame 0 = media time 0 (PM-035).
pub trait PcmReader: Send {
    fn native_rate(&self) -> u32;
    /// Exact length once known (conform finished, or a full scan done); feeds AU-001's correction,
    /// which the app commits as one undoable `CorrectMediaLength` entry (README §5.11).
    fn exact_len(&self) -> Option<u64>;
    /// Fills `out` (interleaved L,R) starting at `frame`. Frames outside [0, len) are zeros.
    /// In ReadMode::Playback this never waits on I/O or decoding: missing blocks → NotReady.
    fn read(&mut self, frame: i64, out: &mut [f32]) -> Result<Fill, SourceError>;
    /// Non-blocking hint: keep these frames resident soon (the feeder looks about 2 s ahead).
    fn prefetch(&mut self, frames: std::ops::Range<i64>);
}

pub enum Fill { Full, NotReady { valid_frames: usize } }

pub enum ReadMode { Playback, Blocking /* export, analysis: wait, with a CancelToken */ }

pub trait PcmProvider: Send + Sync {
    /// Playback: never blocks; returns a handle at once, reads are NotReady until the
    /// look-ahead thread has opened and positioned the source (revised in review).
    fn open(&self, key: &PcmKey, mode: ReadMode) -> Result<Box<dyn PcmReader>, SourceError>;
}

/// Identifies decoded PCM content, independent of the project rate.
pub enum PcmKey {
    Stream { media: MediaFingerprint, stream: u32, downmix_rev: u16 },
    Processed { base: Box<PcmKey>, settings_hash: u64 },   // AU-018/019/020 processed cache
    Stem { base: Box<PcmKey>, stem: StemKind },             // AU-021, from AI-023
}
```

**Media-side obligations** (designed in `freecut-media`; stated here because the engine's
correctness depends on them):

1. **One canonical decode.** A background **conform** job decodes each used stream once, from
   start to end, into native-rate stereo `f32`, stored in blocks (for example 1 s) with a
   validity bitmap.
   - The partly written file is usable.
   - Peaks (§13) and the exact length come out of the same pass.
   - Conform priority follows the playhead.
   - The cache key does not include the project rate. Changing the rate (PM-007) therefore
     invalidates nothing.
   - Lesson from Olive (L4): conform at the **native rate**, not the project rate. That
     avoids an extra resampling stage and a resampler drain, and it lets varispeed and rate
     conversion share one interpolation (§5.3).
2. **Streaming fill before the conform is ready** (`AudioSession`, playback-engine.md):
   - Seek to `target − preroll(codec)` and decode, then discard by PTS to the exact frame
     (MLT, L12).
   - Pre-roll must cover the codec's state: at least 2 AAC frames, about 10 MP3 frames for
     the bit reservoir, 80 ms for Opus, and 2 packets for Vorbis.
   - Reads that continue less than about 0.5 s ahead decode forward instead of seeking.
   - Results go into the same block cache.
   - A property test checks that streaming output equals the conform output after pre-roll.
     It is bit-exact for PCM, FLAC, AAC and MP3, and within −90 dB for Opus.
3. **Alignment (PM-035, AU-002).**
   - Encoder priming and edit lists are honored, using FFmpeg's skip-samples side data and
     container edit lists.
   - Samples before media time 0 are dropped. A late audio start is filled with zeros.
   - PTS gaps are filled with zeros and overlaps are dropped by PTS. Back-to-back
     concatenation is never used (Olive, L4).
4. **Downmix (AU-010).**
   - Coefficients come from the decoder's channel layout (FFmpeg `AVChannelLayout` order),
     never from position (MLT, L9).
   - L = FL + 0.7071·FC + 0.7071·(SL or BL); R mirrors it. LFE is dropped. Mono goes to both
     sides at unity.
   - The matrix is not normalized. The `f32` headroom is kept and the master limiter guards
     the output.
   - Verify: libswresample's automatic rematrix treats mono as front centre and normalizes
     downmix matrices by default. If swr is used, it must be given this explicit matrix.
     Otherwise the downmix is done in our code. A fixture test with per-channel tones decides
     which.
5. **No blocking in playback mode.** `try_read` may take a short, uncontended lock on the
   feeder thread, but never waits for disk or the decoder. The device callback never touches
   sources at all.

#### 5.2 Voice source cursor and retime

For output sample `n` (project rate) inside a voice, the source time is
`s(t_n) = source_time_at(clip, t_n − clip.start)`. That is TR's exact function, and it is the
only place the mapping lives (TR-038). Per render call the voice computes:
- `x0 = s(t_{n0}) × src_rate / TPS` and `x1 = s(t_{n1}) × src_rate / TPS`, as `i128`
  rationals → integer frame plus fractional phase;
- direction from `reverse`. Reverse reads blocks backward and then reverses each block.

Stages (AU-010 chain steps 1–6):

| Step | Where | Notes |
|---|---|---|
| 1 Decode / stem selection | `PcmKey` choice in the plan | Isolation picks a `Stem` key when ready; otherwise the original plays with a badge (AU-021). |
| 2 Downmix → channel map | media (downmix), voice matrix (map) | The map is applied as a 2×2 matrix at timeline step 13 together with pan. That is linear, so it commutes with steps 3–12 when the source-time DSP is linked-stereo. If a future DSP is not linked, apply the map before it. |
| 3–4 Clarity, NR, polish | voice, or the processed cache | Real time when possible. Otherwise the plan selects `PcmKey::Processed` (AU-018). Each stage declares `latency()` and `history()` (§15). |
| 5 Retime | `VoiceResampler` / `TimeStretch` | See §5.3. |
| 6 To project rate | the same resampler pass | Varispeed and rate conversion are one interpolation. |

#### 5.3 Resampling (`dsp/resample.rs`)

- **Bypass.** When `src_rate == project_rate` and the rate is exactly 1/1 (most phone
  footage), samples are copied. This is bit-exact, and split and seek invariance are exact
  with no work.
- **Fixed ratio** (constant speed, or a rate conversion such as 44.1 → 48 kHz):
  - `rubato::Async::new_sinc(ratio, max_rel, &params, chunk, 2, FixedAsync::Output)`;
  - `params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2)` with
    linear interpolation for playback and export alike (one quality, so preview equals
    export);
  - `ratio = project_rate / (src_rate × speed)`.
- **Priming (L11).**
  - When a voice starts at output sample `n0`, the resampler is fed the source frames that
    precede the mapped position. These are real handle media, or zeros before media start.
  - Exactly `output_delay()` output frames are then discarded.
  - Output frame `n0` is therefore aligned to `x0` to within one source sample.
  - At the end of the clip span the voice simply stops. Nothing is drained, because the next
    sample belongs to another voice.
- **Speed curves (closed loop, TR-038 "never drift").** Each render call:
  - computes the exact target source position `x1` at its last sample;
  - sets `set_resample_ratio(N / (x1 − x_consumed), ramp = true)`;
  - `x_consumed` is the exact count of input frames fed, corrected by the resampler's
    constant delay.

  The error is bounded by about one sample per block and cannot accumulate. A property test
  enforces "within one sample" against TR's map.
- **Extreme rates.**
  - Below 0.05× the voice is silent (AU-023).
  - Above `max_resample_ratio_relative` (configure 16), a cheap decimate-by-2 stage is
    cascaded before the sinc stage. Quality may drop, as AU-023 allows. The audio never
    mutes.
- **Keep pitch (AU-023, AU-024, v1).** `TimeStretch` runs on the native-rate source before
  the resampler. It takes a time ratio and a pitch shift (semitones from AU-024 plus the
  voice-effect pitch), so the audio is stretched once. Backends are discussed under Rust
  ecosystem.

```rust
// freecut-audio/src/dsp/stretch.rs (v1 contract)
pub trait TimeStretch: Send {
    /// Constant processing latency in input frames; the voice primes and discards it.
    fn latency(&self) -> usize;
    /// time_ratio = output duration / input duration for the next block; pitch in semitones.
    fn set_params(&mut self, time_ratio: f64, pitch_semitones: f64);
    /// Consumes `input` (interleaved stereo) and writes exactly `output.len() / 2` frames.
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), StretchError>;
    fn reset(&mut self);
}
```

#### 5.4 Device and export rates

- **Device boundary (AU-010).** If the output device cannot run at the project rate, the
  feeder converts master blocks project → device with one more `rubato::Async` at a fixed
  ratio, after the master stage and before the ring.
  - The ring therefore carries **device-rate** frames. That is consistent with
    playback-engine.md's clock math, which counts device frames.
  - The converter is primed at each epoch start. Its delay is compensated inside the feeder,
    so no latency is reported for it (§9).
- **Export rate (EX-031).** Another output rate goes through rubato's synchronous `Fft`
  resampler (exact rational ratio, such as 160/147) on the master output in
  `OfflineRenderer`. It is primed, then drained with `process_all_into_buffer`, so the
  output length is exactly `rate_out × duration`.

### 6. The per-voice chain (timeline-time steps 7–13)

```rust
// freecut-audio/src/engine/voice.rs (sketch of one render call for one voice)
fn render_into(&mut self, v: &VoiceSpec, span: SampleSpan, mode: &RenderMode,
               bus: &mut StereoBuf, rep: &mut RenderReport) {
    let live = span.intersect(v.span);                 // sample-accurate start and end inside the block
    if live.is_empty() || self.is_dormant(v, mode) { return; }
    let n = live.len();
    let (src, gain) = self.scratch.split(n);           // preallocated per voice; no allocation here
    self.pull_source(v, live, mode, src, rep);         // steps 1–6 (NotReady: see the Revised note below)
    sanitize(src, &mut rep.non_finite);                // NaN/Inf from a corrupt decode → 0 (protects IIR state)
    self.timeline_dsp.process(&v.timeline_dsp, src);   // step 7 (v1)
    for seg in v.segments.iter().filter(|s| s.span.overlaps(live)) {
        let part = seg.span.intersect(live);
        fill_gain(seg, part, &mut gain[part.offset_in(live)..][..part.len()]); // steps 8–12
        let smoothing = self.smoother.for_segment(seg.clip);  // 10 ms live-edit ramps; idle in export
        smoothing.apply(&mut gain[part.offset_in(live)..][..part.len()]);
        mix_matrix_into(&seg.matrix, &src[2 * part.offset_in(live)..][..2 * part.len()],
                        &gain[part.offset_in(live)..][..part.len()],
                        &mut bus.frames_mut(part.offset_in_block(span), part.len()));  // step 13
    }
}
```

> **Revised (review, 2026-10-12; README §2.5, C41).** This sketch originally rendered
> `NotReady` as zeros, counted `rep.starved` and kept writing. In playback that advances the
> clock and loses the audio for good, and it contradicts playback-engine.md ("the feeder writes
> less, and the callback underruns, which holds the clock"). With no conform in the MVP,
> streaming fill is `NotReady` at almost every play start, seek and cut. Now, in
> `RenderMode::Playback`, the engine renders only up to the first sample at which any
> non-dormant voice returns `Fill::NotReady { valid_frames }` and reports that length; the
> feeder writes that ready prefix and retries on its next wake, so the ring drains and the
> clock holds (`Buffering` after 1 s). Zeros remain only for `Offline`, missing-stream and
> decode-error sources and for silent shuttle; `Scrub` still drops a grain that is not ready.
> Export and analysis use `ReadMode::Blocking` and never see `NotReady`.

**Gain evaluation (`dsp/envelope.rs`, `dsp/fade.rs`).** The composite linear gain at sample
`n` is:

`g(n) = normalize · static_or_keyframes(s(n)) · duck(t(n)) · fade(n) · xfade(n) · on`

- `static_or_keyframes`:
  - With no keyframes it is the static `GainDb` converted to linear.
  - With keyframes it is evaluated at the source position (TR-027 anchoring):
    - between two finite keys, interpolate in **dB** with the earlier key's easing, then
      convert to linear;
    - if either key is −∞, interpolate in **linear amplitude** (AU-008);
    - `hold` steps are replaced by a 5 ms ramp, linear in dB, starting at the key. This is
      the "dense keyframes are smoothed" rule, and it is still a pure function of `n`.
- `duck`: the `DuckEnvelope` points are in clip-local ticks and interpolate in dB (AU-028).
- `fade`: analytic. For a fade-in over `[s0, s0+L)`, `p = (n − s0)/L`. For a fade-out over
  `[e−L, e)`, `p = (e − 1 − n)/L`. The gain is `curve.gain(p)`. The first fade-in sample and
  the last fade-out sample are exactly `0.0` (L17), and fade-out is the mirror of fade-in.
- `xfade`: `p = (n − x0)/L` on the shared crossfade span.
  - Equal power: out = `cos(πp/2)`, in = `sin(πp/2)`.
  - Equal gain: out = `1 − p`, in = `p`.
  - Both sides compute `p` from the same integers, so equal gain sums to 1 within one `f32`
    ulp.
- `on`: 0 for a muted or disabled clip. That is a gain, not a model change (AU-007,
  TL-036).
- **Fast path.** If a segment has no keyframes, no duck, and no fade or crossfade
  intersecting the block, `g` is constant over the block: one multiply per sample.
- **Reference equivalence.** `freecut-timeline` provides
  `gain_envelope_at(clip, t) -> GainDb`, used by the UI overlay (AU-009). A property test
  checks that `fill_gain` agrees with it within 0.01 dB at random sample positions, outside
  the 5 ms hold-smoothing windows.

```rust
// freecut-audio/src/dsp/fade.rs
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeCurve { Linear, Smooth, Fast, Slow }          // AU-006 (Fast = equal-power quarter sine)

impl FadeCurve {
    /// p ∈ [0, 1]; 0 = the silent edge, 1 = full level.
    pub fn gain(self, p: f64) -> f64 {
        use std::f64::consts::{FRAC_PI_2, PI};
        let p = p.clamp(0.0, 1.0);
        match self {
            FadeCurve::Linear => p,
            FadeCurve::Smooth => 0.5 - 0.5 * (PI * p).cos(),
            FadeCurve::Fast => (FRAC_PI_2 * p).sin(),
            FadeCurve::Slow => 1.0 - (FRAC_PI_2 * p).cos(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XfadeLaw { EqualPower, EqualGain }                  // AU-029, FX-007 (EqualPower in the MVP)

impl XfadeLaw {
    /// Returns (outgoing, incoming) gains at progress p.
    pub fn gains(self, p: f64) -> (f64, f64) {
        let p = p.clamp(0.0, 1.0);
        match self {
            XfadeLaw::EqualPower => {
                let a = std::f64::consts::FRAC_PI_2 * p;
                (a.cos(), a.sin())
            }
            XfadeLaw::EqualGain => (1.0 - p, p),
        }
    }
}
```

**Balance law (AU-013, AU-014).** For `pan > 0`, `gL = 1 − pan/100` and `gR = 1`. For
`pan < 0`, `gL = 1` and `gR = 1 + pan/100`. Centre is unity on both sides, and the law
applies to mono-origin material too. That is the same law as Olive's `PanNode` and MLT's
"front balance". A constant-power option comes later.

### 7. Buses and master (steps 14–15)

**Track bus.**
- Gain and balance come from `BusSpec`, smoothed over 10 ms on live changes.
- Audibility for preview is `!muted && (solo.is_empty() || solo.contains(track))`. Muted
  beats soloed (AU-012).
- Export uses `!muted`, or the preview rule when "Export as heard" is set.
- A change in audibility ramps over 10 ms (AU-011).
- A bus that is inaudible for a whole block renders none of its voices (dormancy). On wake,
  those voices re-prime from the PCM cache, which is cheap, then ramp in.
- Bus sums are added into the master in track order (L16).

**Master stage** (`engine/master.rs`):
1. master gain (smoothed);
2. **pre-limiter meter tap**, for the clip LED at ≥ −0.1 dBFS (AU-015);
3. limiter;
4. post-limiter meter tap;
5. output.

Preview mute is applied after this, in the callback (EX-019).

**Limiter (`dsp/limiter.rs`).** A look-ahead sample-peak brickwall, built in our own Rust from
a well-known construction:
- The required gain per sample is `min(1, ceiling / max(|L|, |R|))`, stereo-linked.
- A **peak-hold minimum** over a window of `L + 1` samples (`L = round(5 ms)`, a monotonic
  deque in a preallocated ring) feeds a **box filter** with the same `L + 1` window. The audio
  is delayed by exactly `L` samples.
- Together they make the gain reach the required value no later than the moment the peak
  leaves the delay. Mind the off-by-one: when both windows and the delay have the same
  length, the guarantee fails by one sample. The no-overs property test catches this.
- Gain rises back by a one-pole release (about 60 ms). A final clamp at the ceiling is a
  belt-and-braces guarantee that no sample exceeds it.
- **Constant latency.** The delay line stays in the path even when the limiter is off.
  Toggling the limiter therefore never moves audio relative to video. With it off, the
  stage passes samples through and the export clips hard at conversion (AU-010).
- **Latency compensation.** The feeder renders master input `L` samples ahead of the play
  position. Ring frame `k` is then exactly the sound of play time `k` (§9). Export does the
  same: it pre-rolls `L`, discards `L`, and renders `L` extra at the end.
- Sample peak only (as AU-010 specifies). Inter-sample overs after lossy encoding are
  possible. See Risks.

```rust
// freecut-audio/src/dsp/limiter.rs (sketch)
pub struct Limiter {
    ceiling: f32,
    delay: DelayLine<[f32; 2]>,        // exactly L samples, preallocated
    hold: SlidingMin,                  // min of required gain over the last L + 1 inputs
    smooth: BoxFilter,                 // moving average over L + 1 values
    release: f32,                      // one-pole coefficient
    gain: f32,
    enabled: bool,
}

impl Limiter {
    #[inline]
    pub fn process(&mut self, x: [f32; 2]) -> [f32; 2] {
        let peak = x[0].abs().max(x[1].abs());
        let need = if self.enabled && peak > self.ceiling { self.ceiling / peak } else { 1.0 };
        let target = self.smooth.push(self.hold.push(need));
        self.gain = if target < self.gain { target } else { target + self.release * (self.gain - target) };
        let [l, r] = self.delay.push(x);
        if !self.enabled { return [l, r]; }
        let c = self.ceiling;
        [(l * self.gain).clamp(-c, c), (r * self.gain).clamp(-c, c)]
    }
}
```

**Engine entry point.**

```rust
// freecut-audio/src/engine/mod.rs (sketch)
pub enum RenderMode<'a> {
    Playback { monitor: &'a MonitorState },
    Scrub { monitor: &'a MonitorState },             // heavy stages only if cached (AU-016)
    Export { as_heard: Option<&'a MonitorState> },   // solo ignored unless "Export as heard"
    Stem { bus: BusIdx },                             // AU-033: post-track, pre-master
    Analysis { clip: ClipId, upto: ChainStep },       // AU-017 (through step 7), AU-028 feeds
}

#[derive(Clone, Debug, Default)]
pub struct MonitorState { pub solo: SmallVec<[TrackId; 4]>, pub silence_timeline: bool /* AU-027 */ }

impl MixEngine {
    /// Renders out.frames() frames of the master starting at project sample `pos`.
    /// The result depends only on (plan, pos, mode). Partitioning a span into calls of any
    /// sizes yields the same samples, apart from live-edit smoothing.
    pub fn render(&mut self, plan: &MixPlan, pos: i64, mode: &RenderMode<'_>, out: &mut StereoBuf)
        -> RenderReport
    {
        self.reconcile(plan);                                    // §8 voice lifecycle
        let span = SampleSpan::new(pos, out.frames());
        self.buses.clear_all(plan.buses.len(), out.frames());    // reuse capacity
        for vi in plan.index.overlapping(span) {                  // fixed order
            let spec = &plan.voices[vi];
            let voice = self.pool.prepare(spec, span.start, &*self.sources, &mut self.report);
            voice.render_into(spec, span, mode, self.buses.acc(spec.bus), &mut self.report);
        }
        self.buses.finish_into(plan, mode, out);                 // track stage → master sum
        self.master.process(&plan.master, mode, out, &mut self.meters);
        self.report.take()
    }
}
```

**Compound clips (TL-040).**
- A `SourceSpec::Nested` voice owns a child `MixEngine` with no master gain and no limiter.
  The child renders the nested plan's bus sum at the nested sequence's source positions.
- If the compound clip is not retimed, its map is a pure offset, so no resampling happens.
- If it is retimed, the child renders the source frames the resampler or stretcher asks for.
- Depth is at most 8 (TL), so recursion is bounded. Child engines are pooled per voice key.

**Denormals and invalid values.** The feeder thread runs inside
`no_denormals::no_denormals(|| …)`, which sets flush-to-zero for the release smoothers,
biquads and reverbs in v1. Each voice input and the master output are sanitized (NaN/Inf →
0, counted), so one corrupt packet cannot poison filter state for the rest of the session.

### 8. Live edits, voice lifecycle and smoothing

When the feeder loads a plan with a new `doc_version`, `reconcile` walks the new voices in
order:
- **Same `key` and same `continuity`.** Keep all DSP and resampler state. Only the
  parameters change: gains and pans glide over 10 ms (AU-005, AU-011, AU-013).
- **Same `key`, different `continuity`.** The clip was moved, slipped, retimed, or its
  source-time DSP changed. The old voice gets a 5 ms release ramp. The new voice is primed
  (§5.3) and ramps in over 5 ms. That is a short crossfade, never a click.
- **New key.** Prime and play from the exact sample, with no ramp unless the edge is hard and
  declick applies.
- **Gone.** A 5 ms release, then the voice returns to the pool.

Allocation happens only here and only on the feeder thread: a pool growing, or a `Box<dyn
PcmReader>` opening. It never happens in the callback.

> **Revised (review, 2026-10-12; README §2.5).** "A `Box<dyn PcmReader>` opening" must not
> mean file I/O on the feeder: for a media stream, `open` is `avformat_open_input` +
> `find_stream_info` (tens of ms for a large `moov`, seconds on a network volume), while the
> ring holds about two device buffers (~43 ms). `PcmProvider::open` in `ReadMode::Playback`
> therefore returns a reader handle at once (allocating only the handle); media's look-ahead
> thread opens and seeks the file, and `read` returns `NotReady` until it is positioned. Voices
> added by an edit during playback or reached by look-ahead then hold the clock briefly instead
> of stalling the feeder. A test with a fake provider that sleeps in `open` shows the feeder
> never stalls.

With a constant plan (export, analysis), reconciliation is a no-op. Export output is
therefore exactly the pure function of positions required by determinism rule 1.

### 9. Playback output: ring, callback and the audio clock

This section restates the audio obligations behind playback-engine.md's protocol. Read the
two together.

- **Feeder thread** (`playback/feeder.rs`, priority above normal through `thread-priority`):
  - loads the current `PlayPlan` and `MixPlan`;
  - renders up to 1024-frame blocks with `MixEngine::render` (internally split at event
    boundaries);
  - applies the device-rate stage;
  - pushes into `AudioRing`, keeping the target fill (2 device buffers by default, adaptive
    up to 8 after underruns);
  - issues `prefetch` about 2 s ahead for voices that will start soon;
  - parks with `park_timeout` of about a quarter of a buffer when full. It never relies on
    the callback to wake it.
- **Positions.** The feeder maps play time to timeline time through `PlayPlan` (shuttle rate,
  loop wrap). At a loop wrap, the feeder renders up to `Out` and continues at `In` within the
  same ring stream. A 2 ms crossfade is applied at the seam because the waveform jumps.
- **Latency is compensated where it arises**, so ring frame `k` of an epoch is the audio of
  play time `k`:
  - limiter look-ahead: render ahead;
  - stateful voice DSP latency: voices read ahead by `latency()`;
  - device-rate converter delay: primed and discarded.

  The only latency the clock adds is the device's own, from
  `OutputStreamTimestamp::{callback, playback}` (playback-engine.md "Latency").
- **The callback** (`playback/output.rs`). Its body is a plain function `fill(...)` over
  slices and atomics. It does not depend on cpal types, so tests call it directly. It:
  - copies ring frames to the device layout;
  - handles channel counts: stereo goes to channels 1–2 and the rest are zeroed; a mono
    device gets (L+R)/2;
  - converts the sample format when the device is not `f32`, using `cpal::Sample`
    conversions;
  - applies 2 ms fades at flush, hold and stop;
  - updates the master post-limiter peak meter;
  - applies preview mute after metering;
  - publishes the `Stamp`;
  - counts underruns.

  It never allocates, locks, logs, drops an `Arc`, or calls into sources.
- **Device policy.**
  - Open the chosen device, or the system default, at the project rate if supported.
    Otherwise use the device's default rate with the converter.
  - Request `BufferSize::Fixed(n)` when `SupportedBufferSize` allows it, and read back
    `stream.buffer_size()`.
  - The error callback maps `cpal::Error::kind()` to a `try_send` on a bounded channel:
    - `Xrun` → counter only;
    - `DeviceChanged` → keep the stream and take the next latency;
    - `DeviceNotAvailable` / `StreamInvalidated` → rebuild on the default device and start a
      new epoch at the current clock position (AU-010);
    - `PermissionDenied`, `DeviceBusy` or no device → banner plus `MonotonicClock`.
  - The cpal `Stream` lives on one owner thread and is never moved.
- **Scrub grains (AU-016)** (`playback/scrub.rs`):
  - Requests are latest-wins (an `AtomicI64` target plus a generation).
  - Each grain is 64 ms rendered in `RenderMode::Scrub` at the target, with 4 ms
    raised-cosine edges. It is its own short epoch.
  - A new target that arrives while a grain is playing crossfades into the new grain at the
    next ring block. Intermediate targets are dropped.
  - After about 80 ms of no motion the grain is allowed to finish.
  - Positions that are not conformed or streamed yet are silent rather than late.
- **Shuttle (EX-007).**
  - Forward 2–8×: the feeder renders `r·N` timeline samples per `N` output frames and
    compresses them pitch-preserved with `TimeStretch` on the master.
  - 1/2×: renders `N/2` and stretches by 2.
  - Without a stretch backend (MVP), non-1× shuttle is silent, as in MLT and libopenshot.
    Silent output still advances the clock.
  - Reverse shuttle is silent (EX-007): the feeder writes silent frames so the clock keeps
    running.

### 10. Meters and loudness (AU-015)

- **On the feeder**, per rendered block of at most 256 frames, the feeder computes:
  - peak and sum of squares per channel for each bus, the master pre-limiter, and the master
    post-limiter;
  - limiter gain reduction.

  It pushes `MeterFrame { epoch, play_frame, … }` into an SPSC ring. The ring is lossy: when
  full, the feeder skips a frame. Every sample is measured (L15).
- **On the UI**, the reader drains the ring and **selects the newest frame whose
  `play_frame` ≤ the clock**, so the meters show what is heard (Kdenlive's idea, L18).
  - `MeterBallistics` is a pure struct in `dsp/meter.rs`: 1.5 s peak hold, 20 dB/s fall, and
    a latching clip LED.
  - When paused, the meters freeze at their last values and decay after 1 s.
- **Loudness worker.** Post-limiter, pre-mute master samples are copied by the feeder into a
  second lossy SPSC ring. A worker thread feeds `ebur128::EbuR128` with
  `Mode::M | Mode::S | Mode::I | Mode::TRUE_PEAK | Mode::HISTOGRAM`.
  - `HISTOGRAM` keeps integrated loudness at constant memory. Without it the crate's block
    history grows without bound.
  - It resets on play start and on the user's reset.
  - If the ring ever drops samples, the integrated value is marked approximate.
  - Integrated loudness under 400 ms shows "—".

### 11. Offline rendering: export, stems and analysis

```rust
// freecut-audio/src/offline.rs (sketch)
pub struct OfflineRenderer<'a> {
    engine: MixEngine,                 // same type as playback
    plan: &'a MixPlan,
    out_rate: Option<SampleRate>,      // EX-031 "other rate" → Fft resampler on the master
}

impl OfflineRenderer<'_> {
    /// Renders exactly sample_at(range.end) − sample_at(range.start) frames (project rate),
    /// pre-rolling every stateful stage (limiter, DSP history) and discarding the pre-roll.
    pub fn render(&mut self, range: TimeRange, mode: RenderMode<'_>,
                  sink: &mut dyn FnMut(&[f32]) -> Result<(), SinkError>,
                  cancel: &CancelToken, progress: &mut dyn FnMut(f32))
        -> Result<RenderSummary, AudioError>;
}
```

- **Sources.** Sources are opened in `ReadMode::Blocking`, so export waits for conforms,
  processed caches, stems and normalization analysis (AU-017, AU-020, AU-021, AU-032).
  Decode failures carry the `ClipId` into the error (AU-032 "name the failing clip").
- **Exact length.** A range that starts or ends inside a fade is simply cut. Nothing is added
  (EX-031). The frame count is exact, so muxed audio and video durations agree.
- **Stems (AU-033).** `RenderMode::Stem { bus }` outputs one bus after the track stage. The
  master and limiter are skipped. The same voices and caches are reused.
- **Analysis (AU-017).** `RenderMode::Analysis { clip, upto: Step7 }` renders one clip's own
  signal (no envelopes) into `ebur128` with `Mode::I | Mode::TRUE_PEAK`. Then:
  - `gain = clamp(min(target − I, ceiling − TP), −30, +24)`;
  - no gain when I < −70 LUFS;
  - short clips (< 400 ms) fall back to the maximum short-term or momentary loudness (spec
    rule).

  The cache key is the one AU-017 defines.
- **Ducking (AU-028).** The analysis renders the selected speech buses with clip mute applied
  but track mute and solo ignored. Then:
  - band-pass 100 Hz–4 kHz (two RBJ-cookbook biquads, public formulas);
  - 10 ms RMS;
  - threshold with hysteresis;
  - bridge gaps shorter than the setting;
  - shape the envelope with the fade-down starting **before** each speech onset (possible
    because the pass is offline).

  The result is `DuckEnvelope` points in clip-local ticks, applied through
  `SetDuckEnvelope`. Live sidechain ducking stays a "later" item (see Risks), because MLT's
  version shows it is position-dependent.
- **Beat feed (AU-025).** The decode-and-downmix feed reads the original `PcmKey::Stream`,
  takes the mono sum, and converts to the analysis rate with rubato `Fft`, before any clip
  processing.

### 12. Recording boundary (AU-027, brief)

- A cpal input stream pushes into an `rtrb` ring.
- A writer thread drains it into a 24-bit WAV (FreeCut's own writer, see the revision note
  below and README C53), checking free disk space between blocks.
- `InputCallbackInfo` timestamps give the input latency. The app combines it with the output
  clock and the user offset to place the clip.
- "Mute timeline while recording" sets `MonitorState::silence_timeline`. Like preview mute,
  it does not touch the project.

> **Revised (toolbar round, 2026-10-13; AU-027's build contract, timeline-editing.md slice
> T6).** The writer is FreeCut's own (`record/wav.rs`), not `hound`: it has to switch to RF64
> on the fly and refresh the header sizes every second so a crash leaves a readable file, and
> both are simpler in a 200-line writer than around a library. `hound` stays a dev-dependency
> for golden fixtures and debug dumps.
>
> ```text
> record/
>   mod.rs        Recorder, RecordConfig, RecorderState, RecorderEvent, TakeInfo, EndReason
>   source.rs     CaptureSource trait, CaptureSink (ring producer + meter taps)
>   cpal_in.rs    CpalInput, list_inputs()            (feature "record", default on)
>   synthetic.rs  SyntheticInput for tests (sine, silence, impulse; fail_after)
>   wav.rs        24-bit RIFF/RF64 writer with periodic header refresh
>   meter.rs      lock-free peak / RMS / held peak / clip latch (atomics)
>   monitor.rs    input → output monitoring path (bypasses MixEngine and the file)
> ```
>
> Threads: the capture callback (cpal's, real-time: converts to `f32`, applies the input gain
> through a 10 ms smoother, updates the meter atomics, pushes into an `rtrb` ring of 2 s, never
> allocates or locks); the writer thread (drains the ring in blocks of 4 096 frames, converts
> to 24-bit with TPDF dither, writes, checks free space every block against
> `RecordConfig::min_free_bytes`, refreshes the header every second, enforces
> `stop_after_frames`); and the caller's thread, which only sends commands and reads atomics
> and `RecorderEvent`s from a `crossbeam-channel`. A ring overrun (writer stalled for 2 s)
> ends the take with `EndReason::Overrun`: the captured part is kept and placed like a
> disk-low stop, with a "Recording fell behind" notice, rather than writing a gap.
>
> Monitoring (`RecordConfig::monitor`) pushes the gained input into a second small ring read
> by the output callback after the master stage, so it is never mixed into an export, never
> written to the take, and is silent when the output is not running at the input's rate (the
> panel then turns monitoring off, AU-027).
>
> Placement: `TakeInfo::started_at` is the output clock's `ClockStamp` at the first captured
> frame; the app computes the timeline start with `freecut_timeline`'s pure
> `recorded_start(start, input_latency, user_offset_ms, rate)` and sends
> `InsertRecordedClip` with `handle_in = preroll_frames` converted to ticks.

### 13. Waveform data (TL-042, AU-009, PM-072)

- **Produced by `freecut-media`** in the conform pass. Each stream gets a peak pyramid at the
  **native rate**, from the post-downmix L and R:
  - level 0 = 256 frames per bin;
  - each level up merges 4 bins.
- **Bin format.** A bin is `{min: i16, max: i16}` per channel. Values are converted with a
  saturating `(x * 32767.0).round().clamp(-32768.0, 32767.0) as i16`. Min and max are kept
  separately, with no `abs` in the narrow type (L20).
- **Size.** At 48 kHz this is about 1.5 KB/s per stream for level 0, about 7 MB per hour
  for the whole pyramid.
- **Drawing query** (`freecut-app`, pure helper in `freecut-media::peaks`):
  1. Map a pixel column's timeline interval through TR's map to source frames `[a, b)`. A
     reversed clip swaps the ends.
  2. Pick the coarsest level with at least 2 bins per column (PM-072).
  3. **Fold min and max over every bin intersecting `[a, b)`**, never point-sample. This
     keeps transients visible at 100×.
  4. At very high speed one column covers seconds, and level selection bounds the work to a
     few bins.
- **Overlay.** The gain overlay line is `gain_envelope_at` from the model, one value per
  pixel column (AU-009). It needs no audio data, so it draws even for offline media. The
  over-0 dBFS tint is `peak × gain > 1` per column.

### 14. Errors

```rust
// freecut-audio/src/error.rs
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no audio output device is available")]
    NoOutputDevice,                                    // → MonotonicClock + banner (AU-010)
    #[error("audio device error")]
    Device(#[from] cpal::Error),                       // kind() distinguishes the cases
    #[error("sample rate {0} Hz is not supported for mixing")]
    UnsupportedRate(u32),
    #[error("time overflow while converting to samples")]
    TimeOverflow,
    #[error("cannot read audio for clip {clip:?}")]
    Source { clip: ClipId, #[source] source: SourceError },
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error("render cancelled")]
    Cancelled,                                         // control flow; never shown to users
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("time overflow while compiling clip {0:?}")]
    TimeOverflow(ClipId),
    #[error("compound nesting deeper than {0}")]
    NestingTooDeep(u8),
}
```

Run-time conditions on the real-time path are **counters in `RenderReport` or atomics**, never
`Result`s:
- `starved` (NotReady blocks);
- `non_finite`;
- underruns, xruns and limiter activity.

`freecut-app` turns them into banners and logs with `anyhow` context.

### 15. Invariants (each has a test, see Test strategy)

1. **Partition.** For abutting clips, every project sample belongs to exactly one clip span
   (floor on both edges).
2. **Block-size independence.** `render` over `[a, b)` in one call equals any partition into
   smaller calls, bit for bit, with a constant plan.
3. **Seek invariance.** Rendering `[p, p+N)` from a fresh engine equals the same slice of a
   render from 0:
   - bit-exact for bypass and rubato voices, because priming uses real history;
   - within −100 dB for stateful DSP. Every stateful stage declares `history()`, and the
     engine pre-rolls at least that much, so output at `n` depends only on input inside
     `[n − history, n + latency]`.
4. **Split invariance.** Splitting a clip changes no output sample (continuity chains,
   TR-036).
5. **Identity.** A clip at 0 dB, centre pan, Stereo map, no fades, same rate and 1× outputs
   its source samples exactly. With the limiter off, the master output equals the sum.
6. **Silence.** −∞, mute, disable, an inaudible bus or offline media produce exact zeros. The
   first fade-in sample and the last fade-out sample are exactly 0.
7. **Crossfade sums.** Equal gain sums to 1 within 1 ulp. Equal power keeps
   `out² + in² = 1` within 1e-6.
8. **No overs.** With the limiter on, `|y| ≤ ceiling` for every output sample of any input,
   including +40 dB square waves and single-sample spikes.
9. **Sync.** Each voice reads, at output sample `n`, the source position TR's map gives for
   `t_n`, within one source sample, for constant speeds and curves (TR-038 property).
10. **Real-time.** `fill()` and a steady-state `MixEngine::render` (after warm-up) perform no
    allocation.
11. **Model untouched.** The engine has no mutable access to the timeline. Solo, preview mute
    and scrub are view state.

### 16. How this meets the project conventions

| Convention (CLAUDE.md, as restated in the task) | How |
|---|---|
| Rational time, floats only at the UI edge | `Tick` and `i64` samples everywhere. `i128` for products. Floats only for signal and gain values and for `p` built from integers. |
| No decode, render or I/O on the UI thread | The UI publishes snapshots and reads atomics or rings. Compile runs on a worker. Decode, conform and peaks run in `freecut-media` workers. |
| Every edit through undo, reversible, tested | The engine never edits. Master gain and the limiter are TL ops (`SetMasterGain`, `SetLimiter`). Duck envelopes go through `SetDuckEnvelope`. Measurements are caches with no undo entry. |
| `thiserror` in libraries, `anyhow` only in the app | `AudioError`, `PlanError`, `SourceError` and `StretchError` use `thiserror`. |
| No `unwrap`/`expect` outside tests | Fallible conversions return errors. Real-time code uses saturating or clamped arithmetic. |
| `unsafe` only in media/render with `// SAFETY:` | `freecut-audio` has no `unsafe`. The seqlock uses atomics, `rtrb` is a safe API, and FFI stays in `freecut-media` (mmap was dropped in review; README §4.4). Third-party crates (`no_denormals`, `rubato` SIMD) keep their own `unsafe`. |
| `freecut-timeline` stays pure | The plan compiler lives in `freecut-audio`. TL only adds pure helpers: `gain_envelope_at`, `effective_audio_crossfade`, `audio_overlaps`, the time map. |

---

## Rust ecosystem

Versions, licenses and dates were checked on 2026-10-11 with `cargo info` and the crates.io
API. The rubato, cpal, rtrb and ebur128 APIs were read in local sources under
`~/.cargo/registry`. The existing stack (cpal 0.18, ffmpeg-next 9.0, thiserror 2) is assumed.

**Use**

| Crate | Version | License | Why |
|---|---|---|---|
| `cpal` | 0.18.2 (2026-08-16) | Apache-2.0 | Device I/O. `build_output_stream(config, data_cb, err_cb, timeout)` takes `StreamConfig` by value. Streams need an explicit `play()`. `OutputCallbackInfo::timestamp()` gives `callback` and `playback` instants. `ErrorKind` has `DeviceChanged`, `DeviceNotAvailable`, `StreamInvalidated` and `Xrun`. `StreamTrait::buffer_size()`. The `realtime` feature promotes the callback thread. |
| `rubato` | 5.0.1 (2026-10-01) | MIT OR Apache-2.0 | `Async` sinc (variable ratio, `set_resample_ratio(…, ramp)`) for voices and the device boundary. `Fft` (exact fixed ratio) for export rate and analysis. `output_delay()` for priming. No allocation in `process_into_buffer`. The 5.x I/O goes through `audioadapter` traits. Wrap it behind `dsp::resample` because the API changed across majors. |
| `audioadapter-buffers` | 5.2.0 | MIT OR Apache-2.0 | Adapters for rubato input and output (interleaved slices, no copy). |
| `ebur128` | 0.1.10 (2024-10-26) | MIT | EBU R128 / BS.1770: M, S, I, LRA, sample and true peak. Use `Mode::HISTOGRAM` for constant memory. Pure Rust (a port of libebur128). |
| `rtrb` | 0.4.0 | MIT OR Apache-2.0 | Wait-free SPSC rings: audio ring, epoch marks, meter frames, loudness feed, recording capture. Same choice as playback-engine.md. |
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | Publishes the latest `MixPlan` to the feeder without locks. |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | Control commands and device events (`try_send` from the cpal error callback). Never used inside the data callback. |
| `smallvec` | 1.16 (1.x line) | MIT OR Apache-2.0 | Inline segment lists and solo sets with no heap allocation in the common case. |
| `no_denormals` | 0.3.0 | MIT | Flush-to-zero scope for the feeder and offline renderer. |
| `thread-priority` | 3.1.1 | MIT | Raises the feeder thread (shared with playback-engine.md). |
| `hound` (dev) | 3.5.1 | Apache-2.0 | Golden fixtures and debug dumps. Recording (AU-027) uses FreeCut's own WAV/RF64 writer (§12, README C53). |
| `realfft` | 3.5.0 | MIT | v1 spectral DSP (AU-018 noise reduction, STFT). Rubato's `Fft` already depends on it. |
| `proptest` (dev) | 1.x | MIT OR Apache-2.0 | The invariants in §15. |
| `approx` (dev) | 0.5.1 | Apache-2.0 | Tolerance comparisons in goldens. |
| `assert_no_alloc` (dev) | 1.1.2 | BSD-1-Clause | Fails tests if `fill()` or a steady-state `render` allocates. |
| `loom` (dev) | 0.7.2 | MIT | Model-checks the seqlock and the ring flush (shared with playback-engine.md). |

**Time-stretch backends (AU-023, v1). Decision open.**

| Option | License | Notes |
|---|---|---|
| Our own WSOLA, phase-locked phase vocoder later | GPL-3.0-or-later (ours) | No C++ toolchain. Good enough for shuttle and speech. Weaker on polyphonic music at large ratios. |
| `signalsmith-stretch` 0.1.3 | MIT (wrapper of the MIT C++ library) | High quality and pitch shift, with formant options in the upstream library. Needs a C++ compiler in the build. The wrapper is young. |
| `rubberband` 0.1.5 (bindings) | GPL-2.0-or-later | Compatible with GPL-3.0-or-later. Needs the system `librubberband`. Very little crate usage. Rubber Band's own licensing forbids App Store distribution without a commercial licence. |

**Avoid**

| Crate / approach | Why |
|---|---|
| `symphonia` (MPL-2.0) as a second decoder | Compatible, but two decoders means two sets of priming, edit-list and gap rules, and the two can disagree. FFmpeg in `freecut-media` stays the only decoder. |
| FFmpeg `swresample` per voice for playback | Stateful C context that needs explicit draining (Olive's bug) and an FFI call per block from the feeder. Rubato gives priming delays and ratio ramps natively. swr is fine inside the one-shot conform pass if its matrix is set explicitly. |
| FFmpeg `atempo` for keep-pitch | Chunk-wise state resets (Olive's FIXME), and a 0.5–2.0 range that needs chaining. |
| `dasp` 0.11 | Last release in 2020. The few helpers we need (sample conversions) come from `cpal::Sample`. |
| `fundsp` | A synthesis graph DSL. Our chain is fixed and compiled, so a second graph abstraction adds cost without value. |
| `rodio`, `kira` | They own the clock and the mixer, but we need sample-exact control of both. |
| `ringbuf` 0.5 alongside `rtrb` | Fine on its own, but one SPSC crate is enough. `rtrb` was already chosen in playback-engine.md. |
| `audio_thread_priority` (MPL-2.0) | cpal's `realtime` feature handles the callback thread, and `thread-priority` handles the feeder. A third mechanism adds risk (also rejected in playback-engine.md). |
| Mutex-based audio FIFOs (`Mutex<VecDeque<f32>>`) | Priority inversion in the callback (Olive, MLT consumers). |

---

## Test strategy

**Pure unit tests** (no FFmpeg, no device; `--no-default-features`):
- `time.rs`:
  - `sample_at` and `span_of` for every accepted rate × every TL frame rate, including
    30000/1001, 24000/1001 and 60000/1001;
  - negative ticks;
  - `i64` extremes (overflow returns an error);
  - property: abutting spans tile.
- `units.rs`: `gain_lin(0 dB) == 1.0` exactly, `NEG_INF → 0.0`, ±0.01 dB round trips, and
  the AU-005 percent conversion examples (50 % ↔ −6.02 dB).
- `fade.rs`:
  - all four curves at p = 0, 0.5 and 1;
  - monotonic;
  - fade-out mirrors fade-in sample for sample;
  - the first fade-in sample and the last fade-out sample equal 0;
  - effective-length clamping cases from AU-006.
- `XfadeLaw`: equal gain sums to 1 within 1 ulp and equal power within 1e-6 at 10⁵ random
  `p`. FX-007 one-sided cases.
- `envelope.rs`: dB interpolation, −∞ segments linear, hold ramps, keyframes outside the
  trimmed range still shaping the edge. Property: agreement with TL's `gain_envelope_at`.
- `matrix.rs`: every `ChannelMap` on a (L = 1, R = 0) and a (L = 0, R = 1) input, and the
  balance law at −100, −50, 0, 50 and 100.
- `limiter.rs`:
  - no sample above the ceiling for adversarial inputs (+40 dB square, isolated full-scale
    spikes, alternating ±, a DC step, random noise at +24 dB);
  - latency is exactly `L` whether on or off;
  - transparent (bit-exact after the delay) below the ceiling.
- `meter.rs` ballistics:
  - hold 1.5 s, fall 20 dB/s, clip LED latching at −0.1 dBFS pre-limiter;
  - a −20 dBFS sine gives RMS −23.01 dBFS and peak −20.
- `plan::compile` (table-driven from small hand-built `MixView`s):
  - continuity chains form only under the exact conditions;
  - crossfade resolution: handles, shift, shorten, dip fallback;
  - mute, enable and disable become gains;
  - deterministic voice order;
  - compound nesting depth ≤ 8;
  - embedded `Detached` → no voice.

**Property tests on the engine** (synthetic sources: ramps, sines, white noise with a fixed
seed, "all ones"):
- Block-size independence: random partitions of a 10 s render compared bit for bit.
- Split invariance: random split points on random clips (1×, 44.1 → 48 kHz, 1.37×, curve,
  reversed) give equal output bit for bit.
- Seek invariance: random start positions equal slices of a full render (§15.3 tolerances).
- Partition: two abutting "all ones" clips on one track sum to exactly 1.0 on every sample,
  with no 0 or 2 at the cut, at 29.97 fps × 44.1/48 kHz.
- Sync: for random retime maps (TR generators), the source position consumed by voice `v` at
  output `n` is within one sample of `source_time_at`. Checked with a ramp source, where the
  sample value encodes its own index.
- Drift: 3 h of a 44.1 kHz source in a 48 kHz project; at the end, the consumed source frames
  are within ±2 of the exact rational expectation.
- Determinism across runs: the same plan rendered twice in different thread interleavings
  (offline and through the feeder with a virtual output) gives identical bytes.

**Golden tests** (`tests/golden/*.ron` timeline fixtures → WAV, compared with tolerance):
- Fixtures:
  - fades × 4 curves;
  - an equal-power transition under video (FX-007);
  - an audio crossfade with and without handles;
  - keyframed ducking dip;
  - 5.1 downmix;
  - a mono source with each channel map;
  - track mute and solo in each `RenderMode`;
  - limiter on and off at +12 dB;
  - a nested compound clip;
  - a stem render.
- Tolerance: bit-exact for identity paths. Elsewhere a max abs error ≤ 1e-6 (−120 dBFS),
  because `sin`, `cos` and `pow` and rubato's SIMD (AVX vs NEON) differ by an ulp across
  platforms.
- Goldens are regenerated only by an explicit `UPDATE_GOLDEN=1`, and the diff report shows
  peak, RMS and LUFS deltas.

**Real-time tests:**
- `fill()` is driven with fake buffers inside `assert_no_alloc`. So is
  `MixEngine::render` after one warm-up pass, with 32 voices including resampling and the
  limiter.
- `loom` covers the stamp seqlock and the epoch flush (shared with playback-engine.md).
- A `VirtualOutput` harness has random block sizes (cpal may vary them), injected underruns,
  holds and a device-rate change. It checks:
  - ring frames map to the correct play time;
  - clock readings never exceed what was written;
  - an underrun yields silence, never repeats.

**Media fixtures** (owned by `freecut-media` tests, needed by this design). Generated with the
system FFmpeg 8.1.1 by a script and committed (small):
- AAC in MP4 with 2112-sample priming and an edit list;
- audio starting 0.5 s after video;
- VBR MP3 with no Xing header (exact-length correction, AU-001);
- 44.1 kHz MP3; 96 kHz FLAC;
- Opus (pre-skip 312, 80 ms pre-roll);
- 5.1 AC-3 and 5.1 AAC with a distinct tone per channel (layout-driven downmix check);
- mono WAV;
- 16-bit WAV containing −32768 (peak pyramid L20);
- a stream with a deliberate PTS gap (filled with zeros);
- a 10 ms file (shorter than the resampler delay);
- a two-stream camera file;
- a truncated file.

Checks:
- streaming fill equals the conform after pre-roll;
- sample 0 alignment matches `ffmpeg -i x -f f32le` with the same trimming, which
  documents the edit-list behaviour;
- peaks equal brute-force min and max.

**Loudness:**
- Synthetic signals with known BS.1770 results: 1 kHz stereo sine at −20 dBFS → −23.0 LUFS
  ±0.1; a gated silence + tone mix.
- Normalization picks `ceiling − TP` when the target would overshoot.
- −80 dBFS noise → "no gain".
- Short clips use the short-term fallback.
- Synthetic signals are used rather than the EBU test files, whose redistribution terms are
  unclear.

**Performance (benchmarks, not CI-gating):**
- `render` throughput for 32 and 64 voices with 44.1 → 48 kHz resampling plus the limiter.
  Target ≥ 20× real time on one core (AU-001 budget).
- Plan compile for 5 000 clips: under 10 ms.
- Scrub grain latency from request to ring: under 20 ms with conformed media.

**Manual QA:**
- Unplugging USB and Bluetooth devices mid-play.
- A 44.1 kHz-only device with a 48 kHz project.
- An 8-channel interface.
- A mono USB headset.
- Sleep and wake.
- Very long timelines (3 h) for drift by ear.
- Dragging a fader during playback (no zipper).
- Hard cuts between unrelated clips (declick on and off).

---

## Risks & open questions

1. **Keep-pitch flag location.** AU-023 puts it in `Retime`, TR-038 in `ClipAudio`. TR
   (retime ownership) and AU must decide. The engine only needs it in `MixView`.
2. **Edge declick default.** A 1.3 ms ramp at hard, discontinuous edges is proposed and on by
   default. It changes output slightly compared with the "pure" model and must stay off
   inside continuity chains. Should AU make it a visible project setting?
3. **Latency subtraction.** EX-002 and AU-026 talk about subtracting output latency.
   playback-engine.md's clock already returns the audible tick. The specs should state that
   compensation happens once, inside the clock. Tap markers are `clock.now() − user_offset`.
4. **Ring rate.** playback-engine.md describes `AudioRing` as project-rate frames, but its
   clock counts device frames. This document converts in the feeder, so the ring carries
   device-rate frames. The two documents must be reconciled.
5. **Edit latency** is "ring target fill + device latency" (about 43 ms + device latency at
   the defaults), not strictly one buffer (EX-002). The same caveat appears in
   playback-engine.md.
6. **Conform cache cost.** Stereo `f32` at 48 kHz is about 1.4 GB per hour of source audio.
   - Options: 24-bit integer storage (−25 %), a per-project LRU cap (PM-074), or
     conforming only used ranges plus handles.
   - Cold start for a large file depends on the streaming fill working well. That is the
     riskier code path, so it has its own equality tests.
7. **Closed-loop varispeed** on rubato `Async` is an inference from its API (ratio ramp,
   exact output counts), not a documented use case. If the drift and sync properties fail,
   write our own polyphase sinc interpolator with explicit fractional positions (about 300
   lines), which gives exact position control.
8. **True-peak overs.** The AU-010 limiter is sample peak. AAC and MP3 encoding of a −1 dBFS
   sample-peak master can exceed 0 dBTP. Consider an export-only 4× oversampled true-peak
   detector feeding the same limiter, and AU-017's −1 dBTP ceiling for consistency.
9. **Live sidechain ducking** (AU-028 "later") would make output depend on playback start,
   as MLT's does. If it is added, it should run as a look-ahead analysis over
   `[n − history, n + latency]` so seek invariance holds.
10. **Time-stretch engine choice** (MVP shuttle silence vs v1 quality). This blocks AU-023,
    AU-024, AU-022 pitch effects and shuttle audio.
11. **`ebur128` maintenance.** The last release was 2024-10. The crate is small, pure Rust and
    MIT. If it stalls, vendor it or port libebur128 (MIT) behind `analysis::loudness`.
12. **Rubato 5 is two weeks old** at the time of writing (5.0.1, 2026-10-01). Pin the minor
    version and keep it behind `dsp::resample`.
13. **Snapshot cost in TL.** Hot-swapping plans on every slider step needs cheap
    `Arc<SequenceSnapshot>` from `freecut-timeline`. If TL clones deeply, coalesce
    publications to about 30 Hz while dragging. The 10 ms smoothers hide the steps.
14. **Device-specific behaviour.** cpal 0.18 notes per-backend differences in xrun and
    latency reporting. For example, its changelog notes that iOS and JACK timestamps now
    include hardware latency. Bluetooth latency may still be under-reported. playback-engine.md
    risk 1 proposes a user A/V offset, and AU-026's offset preference could be shared.
15. **Spec gap: sub-frame trimming** (AU-001 open request). The engine is already
    sample-exact, so TL can allow sub-frame audio edits without engine changes.

---

## References

Reference implementations, studied read-only. Algorithms are described in our own words.
Excerpts above are at most 10 lines.

- **Olive**: GPL-3.0-or-later (`LICENSE`). Compatible, but nothing is ported.
  - `app/audio/audiomanager.cpp`, `app/audio/audioprocessor.cpp`, `app/audio/audiovisualwaveform.h`
  - `app/render/previewaudiodevice.cpp`, `app/render/audioplaybackcache.h`, `app/render/audioplaybackcache.cpp`, `app/render/renderprocessor.cpp`
  - `app/codec/decoder.cpp`, `app/codec/ffmpeg/ffmpegdecoder.cpp`
  - `app/node/output/track/track.cpp`, `app/node/audio/volume/volume.cpp`, `app/node/audio/pan/pan.cpp`
  - `app/node/block/transition/transition.cpp`, `app/node/block/transition/crossdissolve/crossdissolvetransition.cpp`
  - `app/widget/viewer/viewer.cpp`, `app/widget/viewer/viewerplaybacktimer.cpp`, `app/common/decibel.h`
- **MLT**: LGPL-2.1-or-later unless noted (file headers). Compatible.
  - `src/framework/mlt_audio.c`
  - `src/modules/core/transition_mix.c`, `src/modules/core/transition_mix.yml`
  - `src/modules/core/filter_panner.c`, `src/modules/core/filter_audiochannels.c`, `src/modules/core/filter_audioseam.c`, `src/modules/core/link_timeremap.c`
  - `src/modules/avformat/link_swresample.c`, `src/modules/avformat/filter_swresample.c`, `src/modules/avformat/producer_avformat.c`
  - `src/modules/plus/filter_loudness.c`, `src/modules/plus/filter_dynamic_loudness.c`
  - `src/modules/sdl2/consumer_sdl2_audio.c`, `src/modules/rtaudio/consumer_rtaudio.cpp`
  - GPL-2.0-or-later (compatible): `src/modules/normalize/filter_volume.c`, `src/modules/normalize/filter_audiolevel.c`, `src/modules/rubberband/filter_rbpitch.cpp`
- **Shotcut**: GPL-3.0-or-later (file headers). Compatible.
  - `src/models/multitrackmodel.cpp`, `src/models/mixtransitionorder.cpp`, `src/models/audiolevelstask.cpp`
  - `src/widgets/trackpropertieswidget.cpp`, `src/mltcontroller.cpp`, `src/player.cpp`
- **Kdenlive**: GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL (SPDX headers). Compatible with
  GPL-3.0, but porting would pin the ported code to GPL-3.0 terms. Nothing is ported.
  - `src/audiomixer/mixerwidget.cpp`, `src/audiomixer/mixermanager.cpp`
  - `src/timeline2/model/trackmodel.cpp`, `src/mltcontroller/clippropertiescontroller.cpp`
  - `src/jobs/audiolevels/generators.cpp`, `src/definitions.h`
- **libopenshot**: LGPL-3.0-or-later (SPDX headers). Compatible.
  - `src/Timeline.cpp`, `src/Frame.cpp`, `src/AudioReaderSource.cpp`, `src/AudioWaveformer.h`
  - `src/Qt/AudioPlaybackThread.cpp`, `src/Qt/PlayerPrivate.cpp`, `src/Enums.h`
- **OpenTimelineIO**: Apache-2.0 (`LICENSE.txt`).
  - `src/opentimelineio/track.h`, `src/opentime/rationalTime.h`

Ideas worth reimplementing, never copied:
- cumulative-difference sample counts (MLT);
- a reversed-tail seam crossfade (MLT audioseam), as background for the declick;
- meters keyed by play position (Kdenlive);
- a seek-generation seqlock (libopenshot);
- min/max mipmaps (Olive).

All sources are compatible with GPL-3.0-or-later. FreeCut writes its own implementations.

Crate sources read locally (`~/.cargo/registry/src/…`):
- `rubato-5.0.1/src/{lib.rs, asynchro.rs, asynchro_sinc.rs, synchro.rs, slip.rs}`
- `cpal-0.18.2/src/{timestamp.rs, error.rs, traits.rs}`, `CHANGELOG.md`
- `ebur128-0.1.10/src/{ebur128.rs, history.rs}`
- `rtrb-0.4.0/src/lib.rs`

FreeCut documents:
- `docs/features/audio.md`: AU-001…AU-035, Shared definitions, Signal chain
- `docs/features/timeline-editing.md`: TL-001, TL-004, TL-008, TL-011, TL-030, TL-036, TL-040, TL-042, TL-043
- `docs/features/clip-transform-and-speed.md`: TR-027, TR-036, TR-037, TR-038
- `docs/features/effects-transitions-animations.md`: FX-007
- `docs/features/preview-and-export.md`: EX-002, EX-004, EX-007, EX-019, EX-031, EX-044
- `docs/features/project-and-media.md`: PM-007, PM-035, PM-072
- `docs/architecture/playback-engine.md`: clock, ring, transport (companion document)
- Inventory notes (paraphrased behaviour, not a spec): `scratchpad/inventory/audio.md` sections 1–3, 6 and 10
