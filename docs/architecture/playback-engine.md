# Preview playback, scheduling & A/V sync

> Status: draft · Crates: freecut-app, freecut-render, freecut-media, freecut-audio (reads pure types from freecut-timeline)

This document designs the engine behind the player panel. It covers the playback
clock, how video frames are scheduled against it, decode-ahead, frame caches,
seek and scrub latency, frame dropping, preview resolution, the background
threads and their channels, cancellation, and how finished frames reach the egui
UI thread.

Out of scope here (owned elsewhere, only the interfaces are fixed here):
- the compositor internals (FX-001 pass graph) in `freecut-render`;
- the mixer graph and DSP (AU-010 signal chain) in `freecut-audio`;
- the export job (EX-022). Export never uses the real-time scheduler. It shares the
  decoder pool and compositor, but it renders every frame and never drops one.

---

## What FreeCut needs

### Requirements from the feature specs

IDs refer to `docs/features/preview-and-export.md` (EX), `audio.md` (AU),
`timeline-editing.md` (TL), `effects-transitions-animations.md` (FX),
`clip-transform-and-speed.md` (TR), `project-and-media.md` (PM) and `ai-tools.md` (AI).

| Area | Requirement | Numbers / defaults |
|---|---|---|
| Clock | Audio output is the master clock. Time is computed from samples played with integer math, compensating the device's reported output latency (EX-002, AU-010). | `t = start + samples × 705_600_000 / sample_rate` ticks (TL-001) |
| Clock | No output device: a monotonic system clock drives playback, with a banner (AU-010). | — |
| Clock | Preview mute still runs the clock from the device, which outputs silence (EX-019). | — |
| Sync | Video shows the newest rendered frame whose time is ≤ the clock. If rendering falls behind, video frames are skipped. Audio is never stretched or paused for video (EX-002). | 24 fps on 60 Hz uses an uneven 3:2 cadence (accepted) |
| Sync | The renderer always works on the frame for "now + pipeline latency", never on stale ones (EX-002). | — |
| Start | Read-ahead starts at the playhead when play is requested (EX-002, AU-010). | Start latency < 150 ms for cached or proxy media |
| Buffering | No new frame for more than 1 s: pause the clock and audio, show "Buffering…". Resume when 0.5 s of frames are ready (EX-002). | 1 s / 0.5 s |
| Edits | An edit during playback discards read-ahead past the playhead, and the new content shows from the next rendered frame. Audio is re-planned within about one buffer (EX-002). | — |
| End | At the end, stop on the last frame. Play at `duration` restarts from 0 (EX-002). | — |
| Pause | Pausing stops on the frame showing at that moment (EX-002). | — |
| Step | Frame step moves exactly one sequence frame. Each key repeat requests an exact frame. Stepping while playing pauses first and steps from the frame on screen (EX-003). | — |
| Scrub | Latest request wins. Requests that have not started are dropped; a request in progress finishes only if it is still the newest (EX-004). | — |
| Scrub | Two-step delivery: an approximate frame (nearest keyframe) during a fast drag, then the exact frame after 80 ms of stillness or on release (EX-004). | Exact frame in < 150 ms for 1080p long-GOP |
| Scrub | Requests throttled to display refresh. Decoded frames go into the frame cache, so scrubbing back over a region is served from it (EX-004, TL-014). | — |
| Viewer | A frame still rendering keeps the previous frame on screen. Never flash blank. A spinner appears after 250 ms (EX-001). | 250 ms |
| Viewer | The viewer holds only a texture handle and the frame's `(seq_version, Tick)` tag. The UI thread never decodes or renders (EX-001). | — |
| Slates | Offline and undecodable media render procedural slates in the clip's layer rectangle. Errors are logged once per source and range (EX-005). | — |
| Resolution | Auto / Full / 1/2 / 1/4. The target is `canvas × factor`, even pixels, short side ≥ 64 px. Cache entries are keyed by scale. "Sharpen when paused" re-renders the paused frame at full quality (EX-006). | Pure `preview_target_size(canvas, viewer_px, scale)` |
| Shuttle | J/K/L at ±1, 2, 4, 8×. Above 1×, only the frames due at display cadence are rendered. Reverse decodes GOPs backwards into a bounded cache (EX-007). Forward 2–8× audio is time-stretched with pitch kept (AU-016, AU-023). Reverse shuttle is silent. | Reverse cache 512 MB |
| Loop | Loop the in/out range or the whole timeline. The loop start is prefetched, so the wrap gap is ≤ one audio buffer. Out is exclusive (EX-008). | ≈ 21 ms at 1024 frames, 48 kHz |
| Cache | RAM frame cache keyed by a 128-bit render key that covers every input (EX-016). Disk pre-render as 2 s segments (v1). | RAM 25 % of system RAM, max 8 GB. Disk 20 GB, LRU |
| Stats | Presented vs. due frames over a 1 s window. Shown automatically while frames drop. At ≥ 2× the target is display cadence (EX-017). | Green ≥ 95 %, amber ≥ 75 % |
| HW decode | VideoToolbox / D3D11VA / NVDEC / VAAPI / QSV through FFmpeg, with silent per-stream software fallback (EX-018). The v1 path copies through CPU memory. | — |
| Modes | Media preview mode plays one source at its own rate and aspect, with its own audio and no project effects (EX-015). | — |
| Viewports | Fullscreen and a second display share the rendered texture (EX-010, EX-020). | — |
| Audio RT | The real-time callback never blocks, allocates or decodes. Underrun outputs silence and counts it, and audio is never repeated (AU-010). | Buffer 256–4096 frames, default 1024 |
| Audio device | Device unplugged: switch to the system default and continue. Device rate ≠ project rate: resample at the device boundary (AU-010). | — |
| Scrub audio | 40–80 ms grains with short crossfades. Very fast drags drop grains (AU-016). | — |
| Determinism | Preview and export use one pipeline. Preview may only lower resolution, drop frames and use proxies (FX-001, EX conventions). | — |
| Cooperation | Background decoding (thumbnails, proxies) and analysis jobs are throttled while the preview plays (PM-073, AI-001). | — |
| Model | Unit tests in `freecut-app` drive the transport with a fake clock (EX-002). | — |

### Engineering requirements derived from the above

- **R1 One clock per playback run.** Every consumer reads the same clock: the
  scheduler, the presenter, the transport and the auto-scrolling timeline. Each
  run is identified by an `Epoch`.
- **R2 Exact time.** All positions are `Tick(i64)`. Products use `i128`.
  Positions are always computed from absolute counters, never accumulated, so
  nothing drifts. Floats appear only in cost estimates (`Duration`) and UI
  pixels.
- **R3 Latest wins, everywhere.** Requests are state ("show this"), not a queue
  of commands.
- **R4 The UI thread never waits.** No lock it takes is held across decode,
  render, GPU readback or I/O.
- **R5 Real-time-safe audio callback.** No allocation, no lock, no logging, no
  decode, no unbounded loop.
- **R6 Content-addressed caches.** Undo, redo and unrelated edits reuse cached
  frames automatically. A cache hit is pixel-identical to a fresh render at the
  same size.
- **R7 Cooperative cancellation.** Stale work is detected at every stage
  boundary and discarded. Threads are never killed.
- **R8 Graceful degradation.** Missing device, lost GPU, offline media,
  hardware-decoder failure and full disks each have a defined fallback.

---

## How the reference editors do it

All paths are relative to each repository root. Licenses are noted because they
decide whether an algorithm may be ported. Olive (GPL-3.0-or-later), Shotcut
(GPL-3.0-or-later), MLT (LGPL-2.1-or-later), libopenshot (LGPL-3.0-or-later) and
OpenTimelineIO (Apache-2.0) are all compatible with FreeCut's GPL-3.0-or-later.
Kdenlive's files are `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`. That is
compatible too, but ported code would be fixed to GPL-3.0 unless KDE e.V.
accepts a later version. The algorithms below are described in our own words.
We re-derive them in Rust; we do not translate them line by line.

### Olive (C++/Qt, GPL-3.0-or-later)

Key files:
- `app/widget/viewer/viewer.cpp`, `viewerdisplay.cpp`, `viewerplaybacktimer.cpp`, `viewerqueue.h`;
- `app/render/previewautocacher.cpp`, `rendermanager.cpp`, `renderjobtracker.h`, `projectcopier.h`;
- `app/render/framehashcache.cpp`, `playbackcache.h`, `audioplaybackcache.h`, `previewaudiodevice.cpp`, `cancelatom.h`, `videoparams.cpp`;
- `app/audio/audiomanager.cpp`, `app/codec/ffmpeg/ffmpegdecoder.cpp`.

**Clock.** `ViewerPlaybackTimer` (`viewerplaybacktimer.cpp`) is a wall clock. It
stores the epoch milliseconds at start and returns
`start + floor(elapsed_ms / frame_ms) × speed`. Audio is not the master. Audio
is rendered in 250 ms chunks (`kAudioPlaybackInterval` in `viewer.cpp`). Each
chunk is pushed into a byte buffer when the device's "notify" signal fires.
Video and audio are only aligned at start, so the two clocks can drift apart
during long playback.

**Audio output.** In `audiomanager.cpp` the PortAudio callback reads from
`PreviewAudioDevice` (`previewaudiodevice.cpp`). That read takes a `QMutex`,
reallocates the buffer (it keeps the tail with `mid`), and can emit a Qt signal
from the audio thread. The callback's `timeInfo` is ignored, so output latency
is not compensated.

**Frame queue and presentation.**
- `PlayInternal` (`viewer.cpp`) pre-queues 0.5 s of frames (`kVideoPlaybackInterval`)
  and 2 audio chunks. `FinishPlayPreprocess` starts playback only when both have
  arrived.
- The display widget consumes the queue on every buffer swap
  (`ViewerDisplayWidget::UpdateFromQueue`, `viewerdisplay.cpp`). It shows the
  frame whose timestamp equals the timer's frame. When more than one frame is
  popped in one paint, it counts a skipped frame.
- `ViewerQueue::AppendTimewise` (`viewerqueue.h`) orders frames by timeline time,
  and the comparison flips with the sign of the speed.

**Starvation recovery.** In `QueueStarved` (`viewer.cpp`), if the queue has been
empty for 250 ms, `ForceRequeueFromCurrentTime` throws away pending requests.
It restarts the request pointer one second ahead of the current time. This is
an early version of "render for now + pipeline latency".

**Render threads and snapshots.**
- `RenderManager` (`rendermanager.cpp`) runs one video render thread with an
  OpenGL context, one audio thread and waveform threads, all started at
  `QThread::IdlePriority`.
- Renders work on a **copy of the node graph** (`ProjectCopier`). Edits are
  queued and applied to the copy only when no render task is running
  (`PreviewAutoCacher::TryRender`).
- Each queued range carries a `JobTime` (`RenderJobTracker`). A finished frame
  is accepted only if its job time is still current for that time
  (`VideoRendered`).

**Latest-wins scrubbing.** `GetSingleFrame` (`previewautocacher.cpp`) first
cancels any single-frame render that is queued but not yet started (it is
finished with no value). `UpdateTextureFromNode` (`viewer.cpp`) also clears
not-yet-running single-frame renders. Cancellation is a mutex-guarded flag with a
"heard" bit (`CancelAtom`, `cancelatom.h`), polled in the decoder loop.

**Decoder** (`FFmpegDecoder::RetrieveFrame`, `ffmpegdecoder.cpp`):
- It keeps a tiny list of decoded frames (`MaximumQueueSize` returns 2).
- It does not seek if the target lies between the oldest cached frame and two
  seconds past the newest. It decodes forward instead.
- Seeks use `AVSEEK_FLAG_BACKWARD`. If the first decoded frame is already past
  the target, or EOF comes first, it moves the seek point one second earlier and
  retries, down to 0.
- It returns the last frame whose pts ≤ target. At EOF it returns the last
  decoded frame.
- Decoders unused for 5 s are closed, or after 1 s ("aggressive") during
  playback (`rendermanager.h`).
- A "dry run" walks up to 10 s ahead during playback, so decoders for upcoming
  clips stay open and warm (`RequestNextDryRun`, `viewer.cpp`;
  `kDryRunInterval`, `rendermanager.cpp`).

**Caches.**
- `FrameHashCache` (`framehashcache.cpp`) writes **one file per frame**. Float
  frames go to EXR with DWAA compression, 8-bit frames to JPEG. Files live in a
  per-cache UUID folder and are named by timestamp.
- Validity is tracked as a `TimeRangeList` of validated ranges. Graph changes
  invalidate ranges (`playbackcache.h`).
- A cache file that fails to load is deleted.
- Audio is fully pre-rendered into **segmented PCM files**, so edits can shift
  the audio without re-rendering everything (`audioplaybackcache.h`, header
  comment). Source audio is "conformed" to the project sample rate by background
  tasks.

**Preview resolution.**
- A divider in {1, 2, 3, 4, 6, 8, 12, 16} (`videoparams.cpp`). The auto divider
  aims at about 1280×720.
- Changing it from the viewer menu pushes an **undoable** command
  (`ContextMenuSetPlaybackRes`). FreeCut treats this as view state instead
  (EX-006).

**Pitfalls visible in the code:**
- wall-clock video with no latency compensation;
- a lock and an allocation in the audio callback;
- the queue order depends on the direction of playback;
- render threads at idle priority can be starved by other processes;
- an edit waits until no render job is running before it reaches the render
  copy, so a long render delays every edit.

### MLT framework (C, LGPL-2.1-or-later)

Key files:
- `src/framework/mlt_consumer.c`, `src/framework/mlt_audio.c`;
- `src/modules/sdl2/consumer_sdl2_audio.c`;
- `src/modules/avformat/producer_avformat.c`.

**Consumer threading (`mlt_consumer.c`).** The `real_time` property chooses the
model:
- `0` renders synchronously in the caller;
- `1` uses one read-ahead thread **that may drop frames**;
- `-1` uses the same thread but never drops;
- `|n| > 1` uses `n` worker threads (negative means never drop).

Defaults are `buffer = 25`, `drop_max = 5`, `real_time = 1`
(`mlt_consumer_init`). `prefill` sets how many frames must be ready before the
first one is shown.

**Drop algorithm** (`consumer_read_ahead_thread`). Audio is always processed for
every frame. Only image rendering may be skipped. The thread keeps a running
average of the per-frame processing time:
- A sample more than 20× the average is ignored as an outlier.
- Nothing is skipped in the first `buffer/5 + 1` frames after a start, resume or
  seek (pre-roll).
- Skipping is considered only when the queue has drained to `buffer/5 + 1` or
  fewer frames. Then the next image is skipped if the average cost exceeds the
  frame duration.
- After `drop_max` consecutive skips, a render is forced, so the picture never
  freezes completely.

In worker mode (`worker_get_frame`), a `process_head` index moves toward the
tail of the queue as drops increase. Workers then skip frames that cannot finish
before playout. When drops exceed `drop_max` with low-latency settings, the
buffer grows by the thread count.

**Flush.** `mlt_consumer_purge` empties the queue on a seek and sets an
`is_purge` flag, so the frame that is in flight is discarded too.

**Audio-paced output** (`consumer_sdl2_audio.c`):
- `consumer_play_audio` blocks while the SDL FIFO is full. Audio consumption
  therefore paces the whole pipeline.
- Each frame is stamped with a `playtime`, the running sum of audio durations.
- The video thread compares wall-clock time since its start with that stamp. It
  sleeps if the frame is more than 20 ms early. It drops the frame if it is more
  than 10 ms late, unless fewer than two frames are queued. When the queue
  empties, it re-bases its start time.
- A user `video_delay` offset is added.
- At speeds other than 1×, audio is written as silence unless `scrub_audio` is
  set.
- The SDL fill callback takes a mutex and moves the remaining bytes with
  `memmove`.

**Samples per frame** (`mlt_audio_calculate_frame_samples`, `mlt_audio.c`). The
sample count for a frame is the difference of two rounded cumulative positions.
This prevents drift at rates like 29.97 (1601/1602 samples). However, it does
the math with a `float` fps.

**Decoder seeks** (`producer_avformat.c`):
- `seek_video` seeks only when the target is behind the decoder or at least
  `seek_threshold` frames ahead (default **64**). Otherwise it decodes forward.
- For codecs with B-frames it seeks **2 s earlier** ("preseek").
- While catching up after a seek, it sets `skip_loop_filter = AVDISCARD_NONREF`
  and restores it before the target frame.
- A `packets_worker` thread demuxes ahead of the decoder.
- Seekable files open **two `AVFormatContext`s**, so audio and video seek
  independently. The code comments question whether this is needed.
- A "wild timestamp" guard replaces positions more than 999 frames off.
- An `mlt_cache` keeps recently decoded images.

### Shotcut (C++/Qt on MLT, GPL-3.0-or-later)

Key files: `src/videowidget.cpp`, `src/mltcontroller.cpp`, `src/player.cpp`, `src/util.cpp`.

**Consumer setup.**
- `VideoWidget::reconfigure` uses MLT's `sdl2_audio` consumer (or `rtaudio`) for
  audio, with video drawn by the widget.
- It sets `buffer = max(25, fps)`, `prefill = 8`, `drop_max = fps/4` and a user
  `video_delay`.
- `Controller::realTime` returns `1` (drop frames) when the "realtime" setting
  is on. Otherwise it returns `-min(cores-1, 4)` (parallel, never drop), or
  `-1` with GPU effects.

**Frame hand-off.**
- The `consumer-frame-show` handler runs on the MLT thread
  (`VideoWidget::on_frame_show`).
- It converts 10-bit frames there, using a pool of reusable buffers.
- It posts the frame to the GUI thread through a **semaphore of 3** in-flight
  frames (`FrameRenderer`, `m_semaphore(3)`). In real-time mode it waits zero
  milliseconds, so a busy UI makes frames drop instead of piling up.

**Preview scale.** `Controller::setPreviewScale` picks a height (capped at the
profile height). It derives the width from the display and sample aspect, and
rounds both to even numbers (`Util::coerceMultiple`, default multiple 2).

**Seeking.**
- `Controller::seek` seeks the producer, purges the consumer and asks for a
  refresh.
- Refreshes are coalesced by a 10 ms single-shot timer (`m_refreshTimer`).

**Pause.** The comment in `Controller::pause` records a real bug. A frame was
dropped, so the UI did not know the latest position. A later parameter change
then created a keyframe one frame off. The fix seeks to the consumer's position
and refreshes. **Lesson: the paused playhead must be the frame on screen.**

**Loop.** `Player::onFrameDisplayed` sets the playhead from the frame actually
displayed. Looping is reactive: once the displayed position reaches
`loopEnd - 1`, it seeks and purges. The wrap therefore costs a flush and a
pre-roll.

**Shuttle.** `Controller::rewind` and `fastForward` double or halve the speed
and purge the consumer.

### Kdenlive (C++/Qt on MLT, GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)

Key files: `src/monitor/videowidget.cpp`, `src/monitor/monitorproxy.cpp`,
`src/timeline2/view/previewmanager.cpp`, `src/kdenlivesettings.kcfg`.

**Consumer setup** (`VideoWidget` in `videowidget.cpp`):
- `real_time = ±1`, depending on the "drop frames" setting;
- `buffer = max(25, fps)`, `prefill = 6`, `drop_max = fps/4`;
- an optional `scale` for preview scaling.

**Starting playback.** `switchPlay` drops `prefill` to 1 when fewer than 6
frames remain before the end. Otherwise the pre-roll would stall at the end. A
speed change purges the consumer and re-seeks.

**Loop and zone playback.** `checkFrameNumber` detects the out point from the
consumer position. Its comment reads "not sure why we need to check against
pos + 1" (Kdenlive, `src/monitor/videowidget.cpp`). That off-by-one is the cost
of detecting the wrap after read-ahead instead of planning it.

**Seek feedback.** `MonitorProxy` (`monitorproxy.cpp`) keeps a `seekFinished`
flag. It is cleared when a seek is requested and set again once the consumer
reports the requested position. The UI uses it to show that a seek is in
progress.

**Timeline preview (pre-render)** (`previewmanager.cpp`):
- The timeline is cut into chunks of `timelinechunks` frames (default 25,
  `kdenlivesettings.kcfg`). Chunks are rendered to files by a separate process.
- Chunks are named by frame index. When an edit happens,
  `PreviewManager::invalidatePreviews` moves dirty chunk files into an
  `undo/<stack index>` folder and restores them on undo.
- The cache is keyed by position, not content, so it has to follow the undo
  stack by hand.

### libopenshot (C++, LGPL-3.0-or-later)

Key files:
- `src/Qt/PlayerPrivate.cpp`, `src/Qt/VideoCacheThread.cpp`, `src/Qt/AdaptivePreroll.h`, `src/Qt/AudioPlaybackThread.cpp`;
- `src/AudioReaderSource.cpp`, `src/CacheMemory.cpp`, `src/CacheDisk.cpp`, `src/Timeline.cpp`, `src/FFmpegReader.cpp`, `src/PreviewSize.h`.

**Clock.** `PlayerPrivate::run` paces video with `system_clock`. It sleeps until
`start + n × frame_duration`. If it falls behind, it **re-bases** the start time
instead of bursting through late frames. Audio runs separately through JUCE and
is re-seeked to the video position whenever playback pauses or changes speed.
There is no audio-master sync.

**Seek generation.**
- Every seek increments a `seek_generation` counter.
- `getFrame` decodes without holding the lock. It publishes the result only if
  the generation is unchanged.
- `stopPlayback` signals every worker and joins it. A comment warns against
  JUCE's finite-timeout `stopThread`, which can kill a decoder while it holds
  locks.

**Read-ahead cache thread** (`VideoCacheThread::run` and `prefetchWindow`):
- It keeps a directional window ahead of the playhead. The window size is
  `capacity × VIDEO_CACHE_PERCENT_AHEAD`, where capacity is the byte budget
  divided by the estimated bytes per frame.
- A `request_generation` counter and a timeline `CacheEpoch` abort stale fills.
- During playback it fetches at most 8 frames per pass, so the player thread is
  not starved.
- Eviction can leave holes behind the prefetch mark, and the code repairs them.
- `Seek` juggles several interacting flags (`userSeeked`, `scrub_active`,
  `preroll_on_next_fill`, `clear_cache_on_next_fill`) to tell scrubs, commits
  and same-frame refreshes apart.

**Adaptive pre-roll** (`AdaptivePreroll.h`):
- It measures decode times with an exponential moving average of the mean and
  the absolute deviation (weight 0.25).
- It turns `mean + 3 × deviation` into a cost relative to the frame budget. Cheap
  frames need 2 frames of pre-roll; expensive ones need `ceil(2 + 4 × cost)`
  frames, scaled by speed.
- The target rises at once but falls only after a two-frame difference.
- Pre-roll gives up waiting after 500 ms (`StartupLimitUs`).

**Audio.**
- `AudioReaderSource::getNextAudioBlock` pulls audio that is attached to each
  video `Frame`. Each `Frame` carries its own audio samples.
- If the frame is not cached, it may call `reader->GetFrame`, which decodes.
  The transport is created with no read-ahead buffer
  (`AudioPlaybackThread.cpp`), so that decode can run on the audio path.
- It outputs silence until the video cache reports ready, and at any speed
  other than 1×.

**Caches.**
- `CacheMemory` (`CacheMemory.cpp`) keys frames by **frame number**, with a
  deque for LRU order. `Touch` is O(n).
- `CleanUp` evicts while `GetBytes()` exceeds the limit. `GetBytes()` sums every
  frame on each call, so `CleanUp` is O(n²). It always keeps at least 20 frames.
- `CacheDisk` (`CacheDisk.cpp`) writes one image file per frame. Audio goes to a
  text file with one float per line.
- `Timeline::GetFrame` (`Timeline.cpp`) holds a recursive `getFrameMutex` for
  the whole composite. JSON edits (`ApplyJsonDiff`) take the same mutex, so an
  edit waits for the frame being rendered.
- An edit removes the affected frame range from the cache, widened by
  **8 frames on each side**, and bumps a cache epoch.

**Decoder** (`FFmpegReader.cpp`):
- `GetFrame` decodes forward if the target is 1–20 frames ahead. Otherwise it
  seeks to `target − 12` frames.
- Near the start of a file it closes and reopens the file instead of seeking.
- `CheckSeek` detects seeks that land too far and retries at
  `target − 10 × attempt²`, up to 5 times, with detection of seeks that make no
  progress.

**Preview size.** `AlignPreviewSize` (`PreviewSize.h`) rounds to multiples of 4.

### OpenTimelineIO (C++, Apache-2.0)

`src/opentime/rationalTime.h` stores time as a `double` value at a `double`
rate. `to_frames()` truncates (`int(_value)`). It is fine for interchange, but
not for a playback clock: a value like 2.999999 becomes frame 2. FreeCut's clock
uses only integer ticks (TL-001).

### Comparison

| Concern | Olive | MLT / Shotcut / Kdenlive | libopenshot | **FreeCut (this design)** |
|---|---|---|---|---|
| Master clock | Wall clock | Audio FIFO back-pressure, with a wall clock for video | Wall clock | **Audio device samples, latency compensated** |
| A/V alignment | Only at start | `playtime` stamp ± 10–20 ms | Re-seek on pause | **Video follows the audio clock every vsync** |
| Queue order | Timeline time, flips with speed sign | Production order | Frame number | **Play time (monotonic per epoch)** |
| Drop policy | Skip at paint; re-queue 1 s ahead after 250 ms starved | Cost average vs. frame duration when the queue is low; `drop_max` forces a frame | Re-base the clock; prefetch burst limit | **Plan only frames that can finish before their deadline. The presenter skips late ones.** |
| Seek decision | Within 2 s → forward | < 64 frames → forward; 2 s preseek | 1–20 frames → forward; −12 frames | **Keyframe index, cost-based, with a previous-keyframe retry** |
| Cancellation | Mutex flag; tickets finished empty | Purge flag | Generation counters | **Epoch + generation tokens, checked at each stage** |
| Cache key | Time range + validity list | Position | Frame number + epoch, ±8 margin | **128-bit content hash (`render_key`)** |
| Disk cache | One EXR/JPEG per frame | Chunks of 25 frames, undo-folder archive (Kdenlive) | One image + text audio file per frame | **2 s ProRes segments keyed by content (v1)** |
| UI hand-off | GL texture in a display widget | Semaphore(3) to the GUI thread | Render thread signal | **wgpu paint callback over a 4-slot GPU present ring** |
| Audio callback | Mutex + realloc | Mutex + memmove | May decode | **Wait-free ring, no allocation** |
| Loop | Detected in the timer update | Detected after display, then seek + purge | — | **Planned in a `PlayPlan`; audio and video prefetch the wrap** |

### Lessons → FreeCut responses

1. **Video on a wall clock drifts from audio.** Consumer audio clocks are often
   tens of ppm off the system clock, which is tens to hundreds of milliseconds
   per hour (Olive `viewerplaybacktimer.cpp`, libopenshot `PlayerPrivate.cpp`).
   → The clock is derived from samples the device has consumed.
2. **Locks, allocation or decoding in the audio callback** cause glitches under
   load (Olive `previewaudiodevice.cpp`, MLT `consumer_sdl2_audio.c`,
   libopenshot `AudioReaderSource.cpp`). → A wait-free SPSC ring, with
   preallocated state only.
3. **Ignoring output latency** shifts audio late by the device buffer, often
   20–200 ms, and more on Bluetooth (Olive `audiomanager.cpp` ignores
   `timeInfo`). → Subtract cpal's `playback − callback` delay, clamped, plus any
   resampler delay.
4. **Float frame rates and float time** (MLT `mlt_audio.c`, OTIO
   `rationalTime.h`). → Integer ticks and `i128` products.
5. **Queues ordered by timeline time break on loops and reverse** (Olive
   `viewerqueue.h`). → Order by monotonic play time.
6. **Reactive loops** cause off-by-one errors and wrap gaps (Kdenlive
   `checkFrameNumber`, Shotcut `onFrameDisplayed`). → Plan the wrap; audio and
   video read ahead across it.
7. **A playhead taken from processed frames** caused off-by-one keyframes
   (Shotcut `Controller::pause`). → On pause, the playhead becomes the frame on
   screen.
8. **Keyframe seeks that land late** (Olive back-off, libopenshot `CheckSeek`,
   MLT preseek). → A keyframe index from probing, plus a retry from the previous
   keyframe when the first decoded pts is past the target.
9. **Fixed forward-decode thresholds** (MLT 64, libopenshot 20, Olive 2 s) are
   wrong for both short and long GOPs. → Compare the estimated decode cost of
   both paths.
10. **One demuxer shared by audio and video** makes them fight over the seek
    position (MLT opens two contexts). → One decoder session per role.
11. **A global render lock** stalls edits (libopenshot `getFrameMutex`). →
    Immutable `Arc` snapshots. The render side never locks the document.
12. **Position-keyed caches** need epoch flushes, ±8 frame margins or undo
    archives (libopenshot, Olive, Kdenlive). → A content-hash render key.
13. **Many interacting flags** (libopenshot `VideoCacheThread::Seek`). → One
    "latest request" value plus a pure planner function.
14. **Killing threads** that hold locks (libopenshot comment). → Cooperative
    tokens, then join.
15. **Idle-priority render threads** (Olive). → Preview threads at normal or
    higher priority. Background jobs get the lowest priority.
16. **Opening a decoder at a cut causes a hitch** (Olive's dry run keeps decoders
    warm). → A look-ahead opener.
17. **Unbounded UI hand-off.** Shotcut bounds frames in flight with a semaphore
    of 3. → A present ring with back-pressure and reclaim of stale frames.
18. **One file per frame on disk** (Olive EXR files, libopenshot images and text
    audio). → 2 s segments (EX-016).
19. **O(n) cache bookkeeping** (libopenshot `CacheMemory`). → An O(1) weighted
    cache (`quick_cache`).
20. **Stalling at the end** because pre-roll waits for frames that do not exist
    (Kdenlive `switchPlay`). → Pre-roll targets are clamped to the frames that
    remain.

---

## Recommended FreeCut design

### Overview

```
 UI thread (freecut-app / egui)                      freecut-audio
 +--------------------------------+   AudioCmd      +-------------------------------+
 | Transport state machine        |---------------->| AudioEngine                   |
 | every frame (vsync):           |                 |  feeder thread: MixView ->    |
 |   r = clock.reading()          |<-- reading -----|  AudioRing (rtrb, wait-free)  |--> cpal callback (RT)
 |   f = preview.take_due(r + dD) |                 |  AudioClock (seqlock stamps)  |<-- stamps (frames, latency)
 |   paint callback samples slot  |                 +-------------------------------+
 +---------------+----------------+
                 | PreviewRequest (ArcSwap, latest wins) + wake
                 v
 freecut-render: PreviewScheduler thread
   due planner -> render_key -> RamFrameCache? -> compositor (wgpu, shared Queue)
        |                                               |
        | DecodeJob (priority, epoch, cancel)           v
        v                                         PresentRing (4 GPU slots) --> UI
 freecut-media: DecoderPool (N workers, sessions per media/role, keyframe index,
                SourceFrameCache, look-ahead opener)
 freecut-timeline (pure): SequenceSnapshot, frame_plan(t), render_key, PlayPlan
```

The engine has four stages:
- **Decode fans out.** Several worker threads, several sessions.
- **Rendering is serial.** One scheduler thread owns preview GPU work.
- **Presentation follows the clock.** The UI picks the newest due frame once per
  vsync.
- **Audio drives the clock.** The feeder thread fills a ring, and the callback
  publishes what it has consumed.

### Crate and module layout

```
freecut-timeline (pure, no I/O)
  time.rs          Tick, FrameRate, TimeRange (TL-001)
  playback.rs      PlayTime, PlayPlan, EndBehavior, ShuttleRate, Epoch      <- new, shared by render/audio/app
  render_key.rs    render_key(seq, Tick, RenderTarget) -> RenderKey (EX-016)

freecut-media
  cancel.rs        CancelToken (epoch + generation), Priority
  decode/pool.rs   DecoderPool, DecodeJob, DecodeTicket, SessionRole
  decode/session.rs VideoSession: demux + decoder + small decoded ring + retargetable decode
  decode/keyframes.rs KeyframeIndex (built at probe, PM-035), pure seek_action()
  decode/reverse.rs ReverseReader (EX-007)
  decode/hw.rs     HwDecodePolicy (EX-018), unsafe FFI with // SAFETY:
  decode/audio.rs  AudioSession + per-source read-ahead (AU-010)
  cache/source.rs  SourceFrameCache (decoded frames, byte-weighted)
  lookahead.rs     LookaheadOpener (warm decoders for upcoming cuts)

freecut-render
  preview/engine.rs    PreviewEngine (public handle), PreviewConfig
  preview/request.rs   PreviewRequest, FrameRequest, SeekMode, PreviewScale
  preview/size.rs      preview_target_size() (pure, EX-006)
  preview/scheduler.rs scheduler thread loop
  preview/plan.rs      plan_due() (pure), CostModel
  preview/present.rs   PresentRing, PresentedFrame, FrameTag, ViewerBlit pipeline (wgpu only)
  preview/stats.rs     PlaybackStats (EX-017)
  cache/ram.rs         RamFrameCache (EX-016)
  cache/disk.rs        DiskSegmentCache (EX-016, v1)
  slate.rs             offline / error slates (EX-005)

freecut-audio
  clock.rs         PlaybackClock trait, AudioClock, MonotonicClock, ClockReading
  output.rs        cpal stream, device policy, callback
  ring.rs          AudioRing (rtrb samples + epoch marks), SharedOutput atomics
  feeder.rs        feeder thread: mixes PlayPlan audio into the ring
  scrub.rs         ScrubVoice (AU-016)

freecut-app
  player/transport.rs  Transport state machine (generic over PlaybackClock)
  player/shuttle.rs    J/K/L step table (EX-007)
  player/mod.rs        Player facade: wires PreviewEngine + AudioEngine + Transport
  player/viewer.rs     egui panel, ViewerPaintCallback (egui_wgpu::CallbackTrait)
  player/activity.rs   PlaybackActivity (throttle signal for background jobs)
```

Dependency direction:
- `freecut-render` and `freecut-audio` depend on `freecut-media` and
  `freecut-timeline`. `freecut-app` depends on all of them.
- `freecut-render` does **not** depend on egui. The egui paint-callback adapter
  lives in `freecut-app` and uses the wgpu objects that `freecut-render`
  exposes.
- `CancelToken` lives in `freecut-media`, the lowest crate that render and audio
  both depend on.
- `PlayPlan` lives in `freecut-timeline`, because it is pure tick math, like
  `loop_region` (EX-008).

### Time types for playback (`freecut-timeline::playback`)

Play time and timeline time are different quantities:
- **Play time** (`PlayTime`) is real elapsed playback time since the epoch's
  first audible sample.
- **Timeline time** (`Tick`) is what the play time maps to. It depends on the
  shuttle rate, the loop wrap and the direction.

Keeping them separate fixes lessons 5 and 6.

```rust
/// Identifies one uninterrupted playback run. Every play, seek, rate change or
/// restarted loop region starts a new epoch. Results from an older epoch are dropped.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(pub u64);

/// Real playback time elapsed in an epoch, in TL-001 ticks (1/705_600_000 s).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayTime(pub i64);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ShuttleRate { pub num: i32, pub den: u32 } // 1/1 = normal, -2/1 = reverse 2x (EX-007)

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndBehavior { Stop, Loop }

/// Pure mapping from play time to timeline time for one epoch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayPlan {
    pub epoch: Epoch,
    pub start: Tick,          // timeline time audible at PlayTime(0); on the frame grid
    pub rate: ShuttleRate,    // never 0 while playing
    pub bounds: TimeRange,    // [0, duration) or the loop region; end is exclusive (EX-008)
    pub at_end: EndBehavior,
    pub frame: FrameRate,     // sequence grid (TL-001)
}

pub enum PlanPos { At(Tick), Ended { last_frame: Tick } }

impl PlayPlan {
    /// offset = played * num / den, using i128. In Loop mode the result is folded into
    /// `bounds` with rem_euclid. Stop mode returns Ended at or past the bound.
    pub fn timeline_at(&self, played: PlayTime) -> PlanPos { /* ... */ }

    /// Iterates (deadline, timeline frame start) in presentation order, starting
    /// at play time `from`. Frames are spaced by `step`: every frame for |rate| <= 1,
    /// otherwise about |rate| x display_interval rounded to whole frames (EX-007).
    /// The iterator crosses loop wraps, so the frame after Out-1 is In (EX-008).
    pub fn frames_from(&self, from: PlayTime, display_interval: PlayTime)
        -> impl Iterator<Item = (PlayTime, Tick)> + '_ { /* ... */ }
}
```

Rules:
- Frame selection quantizes **down** in the playing direction. The timeline
  frame shown at play time `p` is `floor_to_grid(timeline_at(p))` going forward
  and the frame containing that instant going backward.
- At 1×, a loop of `n` frames yields exactly `n` distinct frames per lap, with
  no repeat and no skip at the wrap. This is property-tested.
- A loop-region change while looping (EX-008 "new bounds apply at the next
  wrap") is an **amendment**: `plan.amended(effective_from, new_bounds)`.
  `effective_from` must be at or after both the audio feeder's write head and
  the video scheduler's planned head, so no stage has already produced content
  under the old bounds. If that is impossible, the transport starts a new epoch
  at the current position, which costs one flush.

> **Revised (review, 2026-10-12; README §3.1).** `bounds` was fixed at play start as
> `[0, duration)`, and an edit while playing only called `set_document`, so appending a clip
> (or undoing a ripple delete) stopped playback at the old end, and shortening the sequence
> played black and silence up to the old end. Now every commit while `Playing` that changes
> the sequence duration applies the same amendment, `plan.amended(effective_from,
> [0, new_duration))`, with the same feeder/scheduler head rule, or starts a new epoch at the
> current position when amending is impossible. If the new end is already behind the
> playhead, playback ends at once (`Paused` at the new last frame). Transport `FakeClock`
> tests cover extend-while-playing and shrink-past-playhead.

### The playback clock (`freecut-audio::clock`)

```rust
pub trait PlaybackClock: Send + Sync {
    /// Timeline time audible now (EX-002 model note). Paused: the paused position.
    fn now(&self) -> Tick { self.reading().timeline }
    fn reading(&self) -> ClockReading;
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ClockReading {
    pub epoch: Epoch,
    pub played: PlayTime,  // monotonic within an epoch
    pub timeline: Tick,    // plan.timeline_at(played), on the frame grid
    pub state: ClockState,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClockState { Paused, Starting, Running, Held /* underrun or buffering */, Ended }
```

**AudioClock: what the callback publishes.** At the start of each output block,
the callback writes a `Stamp` through a single-writer **seqlock**. Every field
is an atomic, so a torn read is impossible, and the writer never waits. Fields:
- `epoch`;
- `device_frame`: frames written to the device since the stream opened;
- `played_at_block`: frames of this epoch played before this block;
- `real`: frames in this block that came from the ring;
- `block`: block length in frames;
- `latency`: output latency in frames;
- `wall`: a `std::time::Instant` taken inside the callback.

The last 64 stamps are kept in a fixed array of atomics. 64 blocks covers more
than one second, which is longer than any latency we accept.

**Reader math.** All of this is integer arithmetic; `Duration` is integer
nanoseconds.
1. `elapsed = min(frames_in(now − wall), block)`. This interpolates inside the
   current block, so the clock moves smoothly between callbacks. It never
   extrapolates past what was actually written.
2. `audible_device_frame = device_frame + elapsed − latency`.
3. Map that device frame to played frames through the stamp history:
   - inside a block's real part: `played_at_block + offset`;
   - in the silent tail of a block (underrun or hold): `played_at_block + real`,
     so the clock **holds**;
   - before the epoch's first stamp: 0, and the state is `Starting`.
4. `played = frames_to_ticks(played_frames, device_rate)`, computed as
   `frames × 705_600_000 / rate` in `i128`. This is exact for every common device
   rate (8 k to 192 k all divide the tick rate) and uses floor division
   otherwise. Because it comes from absolute counters, it never drifts.
5. `played = max(played, last_reported[epoch])`, so readings never go backward
   within an epoch.

**Latency.**
- `latency = (info.timestamp().playback − info.timestamp().callback)` converted
  to frames. Both instants are on the stream's own clock (cpal 0.18
  `OutputStreamTimestamp`). Never compare a `StreamInstant` with `Instant`: on
  some hosts each stream has its own origin (cpal `timestamp.rs` docs).
- Add the device-boundary resampler's delay (AU-010, when the device rate
  differs from the project rate).
- Clamp the total to 0–500 ms. If the host reports 0, use one block as a
  conservative estimate.
- On `cpal::ErrorKind::DeviceChanged` (the stream was rerouted), keep the stream
  and treat the next callback's latency as authoritative. On
  `DeviceNotAvailable` or `StreamInvalidated`, rebuild on the system default
  (AU-010) and restart the epoch at the current position.

**Why underruns hold the clock.** The ring contains audio for consecutive play
time. A missing sample means that play time has not been heard yet. If the
clock held still, video stays in sync and the user hears a gap of silence. If
it advanced, audio would jump ahead and skip content. AU-010 requires silence
for the missing block and no repeated audio; holding meets both. Long stalls
become the EX-002 "Buffering" state (see Transport).

**MonotonicClock (no output device).** Same trait. It uses
`played = (Instant::now() − start) × 705_600_000 / 1e9`, computed with integer
nanoseconds, with `hold()` and `resume()` for buffering. Latency is 0. The
transport cannot tell which clock it uses.

**Muted preview (EX-019).** The callback consumes the ring, meters it, then
writes zeros. The clock is unaffected.

### Audio feeder and ring (`freecut-audio::{ring, feeder, output}`)

- `AudioRing` is an `rtrb::RingBuffer<f32>` of interleaved stereo at the project
  rate. Its capacity is 8 × the largest device buffer.
- A second small `rtrb` ring carries `EpochMark { at_ring_frame: u64, epoch }`,
  which tells the callback where a new epoch's samples begin.
- `SharedOutput` holds only atomics:
  - `flush_to_mark` (a pending flush);
  - `hold`, `mute` and `gate_open`;
  - the master gain bits;
  - underrun and over counters;
  - meter peaks.
- The **feeder thread** owns the mixer graph (AU-010). It renders 1024-frame
  blocks of the current `PlayPlan` through the read-only `MixView` and keeps the
  ring at its target fill:
  - default 2 device buffers;
  - raised to as many as 8 after underruns;
  - lowered again after 10 s with no underrun.
- The feeder never blocks on decode. Source audio comes from per-source
  read-ahead buffers in `freecut-media`. If those are short, the feeder writes
  less, and the callback underruns, which holds the clock.
  (Confirmed in review as the rule for both studies, README §2.5 and C41: the feeder
  writes only the prefix up to the first not-ready sample; audio-engine.md's
  zero-fill is limited to offline, missing and broken sources.)

The real-time callback, as a sketch:

```rust
fn on_output(st: &mut CallbackState, out: &mut [f32], info: &cpal::OutputCallbackInfo) {
    // Real-time rules: no allocation, no locks, no logging, no syscalls except Instant::now.
    let wall = std::time::Instant::now();
    let frames = out.len() / CHANNELS;
    if st.shared.take_flush() {
        st.skip_to_latest_mark();                 // drop old-epoch frames; switch epoch
    }
    let real = if st.shared.gate_open() && !st.shared.hold() {
        st.copy_from_ring(out, frames)            // <= frames; fade in 2 ms at epoch start
    } else { 0 };
    st.fade_out_tail_if_stopping(out, real);      // avoid clicks on hold/seek
    out[real * CHANNELS..].fill(0.0);
    st.shared.meter(out);                         // meters before mute (EX-019)
    if st.shared.mute() { out.fill(0.0); }
    st.stamps.publish(Stamp {
        epoch: st.epoch, device_frame: st.device_frame, played_at_block: st.played,
        real: real as u32, block: frames as u32,
        latency: st.latency_frames(info, frames), wall,
    });
    st.device_frame += frames as u64;
    st.played += real as u64;
    if real < frames && st.expecting_audio() { st.shared.count_underrun(); }
}
```

**Discontinuities.**
- **Seek, rate change, play start.** The feeder pushes an `EpochMark` at its
  write position, writes the new epoch's samples, and sets `flush_to_mark`. The
  callback skips to the mark immediately. Nobody waits for anybody.
- **Edits while playing.** The `PlayPlan` is unchanged; only the content
  changes. The feeder keeps writing from the end of the ring using the new
  `SequenceSnapshot`. At most the ring's target fill (2 buffers, about 43 ms)
  plus device latency of pre-edit audio is heard. This is our reading of EX-002
  "re-planned within one buffer"; see Risks.
- **Loop wrap.** The feeder writes past `Out − 1` straight into `In`, because
  `PlayPlan` gives the sample-accurate wrap. Nothing is flushed, so the wrap is
  seamless (EX-008).
- **End (Stop).** The feeder writes up to the end and sets "end of stream". A
  ring that is empty after end of stream is not counted as an underrun.
- **Reverse shuttle (silent, EX-007).** The feeder writes silence that still
  counts as real samples, so the clock runs from the device.
- **Scrub grains (AU-016)** are each a short epoch of their own, played while
  the transport is paused. The video does not read the clock during scrubbing.

### Preview engine (`freecut-render::preview`)

Requests are state, not commands. The latest request wins:

```rust
#[derive(Clone, Debug)]
pub enum PreviewRequest {
    Idle,
    /// Paused, stepping, scrubbing or skimming (EX-003/004, TL-014). `mode` picks the
    /// two-step delivery: Approximate during fast motion, then Exact.
    Still(FrameRequest),
    /// Playing: frames become due according to `plan` and the shared clock.
    Play { seq: SeqVersion, plan: Arc<PlayPlan>, target: RenderTarget },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameRequest {           // EX-004 model note
    pub seq_version: u64,
    pub time: Tick,
    pub target: RenderTarget,       // resolved pixel size + proxy policy, see Preview resolution
    pub mode: SeekMode,             // Exact | Approximate
}

pub struct PreviewEngine { /* Arc<Shared>, JoinHandle */ }

impl PreviewEngine {
    pub fn spawn(gpu: GpuContext, decoders: DecoderPool, clock: Arc<dyn PlaybackClock>,
                 repaint: Box<dyn Fn() + Send + Sync>, cfg: PreviewConfig) -> Result<Self, RenderError>;
    pub fn set_document(&self, snap: Arc<SequenceSnapshot>); // ArcSwap store + wake
    pub fn request(&self, req: PreviewRequest);              // ArcSwap store + generation++ + wake
    /// UI thread, non-blocking: newest Ready frame with deadline <= `present_at`.
    /// Older Ready frames are released and counted as dropped.
    pub fn take_due(&self, present_at: PlayTime) -> Option<PresentedFrame>;
    /// UI thread: the finished frame for the current Still request, if any.
    pub fn take_still(&self) -> Option<PresentedFrame>;
    pub fn ui_frame_finished(&self);                         // releases slots no longer shown
    pub fn stats(&self) -> PlaybackStats;
}
```

- `repaint` is `egui::Context::request_repaint`, supplied by the app.
  `egui::Context` is `Send + Sync`, so `freecut-render` never names egui.
  > **Revised (review, 2026-10-12; README §2.3 rule 7).** This callback alone is not
  > enough: frames are rendered ahead of their deadline, so the repaint it triggers
  > usually finds nothing due, and reactive eframe then waits for the next finished
  > frame, which makes presentation follow render completion instead of the clock. While
  > the transport is `Prerolling`, `Playing` or `Buffering`, the app calls
  > `ctx.request_repaint()` on every update (or `request_repaint_after(next_ready_deadline
  > − now)`), so `take_due` runs every vsync. A `FakeRenderer` test shows frames rendered
  > 3 frames early are presented on their deadline vsync.
- `GpuContext` is a clone of eframe's `wgpu::Device` and `wgpu::Queue` (via
  `egui_wgpu::RenderState`) and the target format.

**Scheduler loop.** One dedicated thread. It owns the compositor's pipelines and
all preview GPU submissions.

```rust
loop {
    let gen = shared.generation();
    let req = shared.request.load_full();               // ArcSwap<PreviewRequest>
    let doc = shared.document.load_full();              // ArcSwap<SequenceSnapshot>
    gpu.device.poll(wgpu::PollType::Poll).ok();         // drives cache readbacks (map_async)
    match &*req {
        PreviewRequest::Idle => { shared.wake.recv_timeout(IDLE_POLL).ok(); }
        PreviewRequest::Still(r) => serve_still(r, &doc, gen)?,  // see "Seek and scrub"
        PreviewRequest::Play { plan, target, .. } => {
            let now = clock.reading();
            if now.epoch != plan.epoch { shared.wake.recv_timeout(MS_2).ok(); continue; }
            ring.reclaim_stale(plan.epoch, now.played);   // never-shown frames past deadline -> Free
            let due = plan_due(plan, &DueInput {
                now: now.played, cost: cost_model.estimate(), // EWMA mean + 3 x deviation
                display_interval: display.interval(), planned_head: planner.head(),
                depth: depth_policy.depth(cost_model.estimate(), plan.frame),
            });
            decoders.prefetch(&doc, plan, &due, Priority::Playback, token(plan.epoch, gen));
            for (deadline, t) in due {
                if shared.generation() != gen { break; }  // newer request: re-plan now
                let Some(slot) = ring.acquire() else { break }; // back-pressure from the UI
                let fplan = doc.frame_plan(t);            // FX-001, pure
                let key = render_key(&doc, t, target);    // EX-016, pure
                let started = Instant::now();
                let res = match ram_cache.get(&key) {
                    Some(img) => blit.upload(&img, slot),
                    None => compositor.render(&fplan, target, slot, &decoders, token(plan.epoch, gen)),
                };
                match res {
                    Ok(()) => {
                        cost_model.observe(started.elapsed());
                        ring.publish(slot, FrameTag { epoch: plan.epoch, seq: doc.version(), time: t, key, approx: false, target: *target }, Some(deadline));
                        planner.advance(deadline);
                        (shared.repaint)();
                        cache_fill.maybe_readback(slot, key); // async copy for RamFrameCache
                    }
                    Err(RenderError::Cancelled) => { ring.release(slot); break; }
                    Err(e) => { ring.release(slot); report_once(e); } // slates come from FX-001 L1
                }
            }
            shared.wake.recv_timeout(half_frame(plan.frame)).ok();
        }
    }
}
```

**Due planner** (`plan_due`, pure, unit-tested):
- `earliest_useful = now + cost + display_interval`. A frame whose deadline is
  closer than its predicted cost would be late, so it is skipped **before** any
  work is done. This combines MLT's "skip if the average cost exceeds the frame
  duration" with Olive's "re-queue ahead after starvation", and it acts per
  frame.
- The planner continues from `max(planned_head + step, earliest_useful)`. It
  returns up to `depth` entries from `plan.frames_from(..)`.
- **Depth.** Cheap frames need 2 frames ahead. Expensive frames need
  `ceil(2 + 4 × cost/frame)` frames (the shape of libopenshot's adaptive
  pre-roll). The depth is clamped to 2–12 and to the frames remaining before the
  end (lesson 20).
  *Revised in review (README §2.3 rule 7):* with a 4-slot `PresentRing`, one slot
  Shown and one being written, a depth above 2–3 was never reachable. The depth is
  also clamped to `slots − 2`, and the ring has
  `slots = clamp(128 MiB / slot_bytes, 4, 14)` slots (14 at 1080p, 4 at 4K).
- **Rate > 1×.** The step is about `rate × display_interval`, so only frames
  that can be seen are rendered. EX-017 counts those as the target.

**Presenter (UI thread).**
- Each egui frame calls
  `take_due(reading.played + present_latency)`, where `present_latency` is
  egui's predicted frame time (`ctx.input(|i| i.predicted_dt)`), converted to
  ticks at the UI edge. A frame drawn now appears at the next vsync.
- `take_due` returns the newest Ready frame with `deadline ≤ present_at` and
  releases older ones. A released frame that was never shown counts as
  `dropped_late`.
- Nothing is shown "from the future". The only exception is the first frame of
  an epoch, which shows during pre-roll; it is the paused frame anyway.

**Drops are counted in three places** (EX-017):
- frames the planner skipped (`dropped_planned`);
- frames rendered but never shown (`dropped_late`);
- frames reclaimed while the UI was not painting (`dropped_unpainted`, for
  example a minimized window).

`presented / due` over a sliding 1 s window drives the green, amber and red
readout.

**Pre-roll (start latency < 150 ms).**
1. Play goes through `Transport::Prerolling`. The scheduler renders the start
   frame and `depth` more.
2. The feeder fills the ring to 2 buffers.
3. The transport opens the audio gate when the first video frame is Ready and
   the ring is full enough. It also opens it after **250 ms** regardless (the
   startup limit, cf. libopenshot's 500 ms), and video catches up by dropping.
   *Revised in review (README §2.5):* "full enough" is replaced by "every voice
   active at the start position has returned `Full` for its first block", since a
   ring can also be full of zero-filled starvation; the 250 ms limit stays.
4. The clock reports `Starting` until the first epoch sample is audible
   (latency). The playhead stays on the start frame until then, so picture and
   sound start together.

**Edits during playback (EX-002).**
- `set_document` swaps the snapshot.
- On its next loop, the scheduler recomputes the `render_key` of every Ready
  frame for this epoch. Frames whose key changed are released. Frames whose key
  is unchanged stay valid, because an edit outside them does not change their
  pixels.
- The planner head is reset to the first released frame. The audio side behaves
  as described above.
- Paused edits re-request the current `FrameRequest` only if the playhead
  frame's key changed. The old frame stays on screen until the new one arrives
  (EX-001).

### Decode (`freecut-media::decode`)

**DecoderPool** runs `N = clamp(cores/2, 2, 6)` worker threads, which pull from a
priority queue built with `crossbeam-channel`. Priority order:
1. `Exact` (scrub, step);
2. `Playback`;
3. `Approximate`;
4. `Lookahead` (opening decoders early);
5. `Background` (thumbnails, filmstrips, proxies — PM-073).

Identical in-flight jobs (same source, same pts) are merged: one decode, many
tickets.

```rust
pub struct DecodeJob {
    pub source: SourceRef,          // MediaId + stream + Original|Proxy(height) + decode variant
    pub at: SourceTime,             // pts in stream timebase, from TR's time map
    pub mode: SeekMode,             // Exact | Approximate
    pub priority: Priority,
    pub cancel: CancelToken,        // epoch + request generation
}

impl DecoderPool {
    pub fn request(&self, job: DecodeJob) -> DecodeTicket;   // never blocks
    pub fn prefetch(&self, /* frames the planner will need */);
    pub fn cached(&self, src: &SourceRef, at: SourceTime) -> Option<Arc<DecodedFrame>>;
}
impl DecodeTicket {
    pub fn wait(&self, deadline: Instant) -> Result<Option<Arc<DecodedFrame>>, DecodeError>;
}
```

`DecodedFrame` holds the refcounted `ffmpeg_next::frame::Video`, which is
`Send + Sync` in ffmpeg-next 9, so no copy is made. It also holds metadata: pts
as `Tick`, color tags, rotation and pixel aspect. Planes are uploaded to GPU
textures as they are (Y plus UV, 8 or 10 bit). YUV→RGB conversion, rotation and
range expansion happen in the FX-001 L2 shader (CL-002), not with `sws_scale`
on the CPU.

**Sessions.** A `VideoSession` owns:
- one `format::context::Input` (opened with `format::input_with_interrupt`, so
  blocking I/O on slow or network drives observes the cancel token);
- one decoder;
- the stream's `KeyframeIndex`;
- a ring of the last 2–4 decoded frames.

Sessions are keyed by `(MediaId, stream, SessionRole, lane)`, where `SessionRole`
is `Playback`, `Scrub`, `Audio`, `Reverse`, `Thumbnail` or `Export`.
- Separate roles mean audio read-ahead, scrub seeks and thumbnail jobs never
  move each other's read position (lesson 10).
- The same file used at two places on the timeline gets up to 3 `Playback`
  lanes. A job goes to the lane whose position is cheapest to continue from.
- Limits: 24 open sessions in total. Idle sessions close after 10 s when
  paused, or 3 s while playing (Olive uses 5 s and 1 s).
- Hardware decode sessions are capped per backend. Extra sessions fall back to
  software (EX-018).

**Seek decision** (pure, unit-tested). The `KeyframeIndex` is built at probe
time (EX-004 model note; PM-035).

```rust
pub enum SeekAction { FromRing, DecodeForward, SeekTo { keyframe: i64 } }

/// Choose the cheaper path to `target` (lesson 9). `pos` is the last pts the decoder output.
pub fn seek_action(idx: &KeyframeIndex, ring: &DecodedRing, pos: Option<i64>, target: i64) -> SeekAction {
    if ring.contains(target) { return SeekAction::FromRing; }
    let kf = idx.at_or_before(target).unwrap_or_else(|| idx.first());
    match pos {
        Some(p) if p < target => {
            let forward = idx.frames_between(p, target);
            let seek = idx.frames_between(kf, target) + SEEK_PENALTY_FRAMES; // flush + demux cost
            if forward <= seek { SeekAction::DecodeForward } else { SeekAction::SeekTo { keyframe: kf } }
        }
        _ => SeekAction::SeekTo { keyframe: kf },
    }
}
```

- **The seek call itself.** Use the video stream's index with a range that ends
  at the keyframe pts, which guarantees "at or before". Watch out:
  `ffmpeg_next::format::context::Input::seek` passes stream index −1, so its
  timestamp is in `AV_TIME_BASE` (microseconds), not the stream timebase. Either
  rescale before calling it, or call `avformat_seek_file` with the stream index
  inside `freecut-media`'s FFI module (`unsafe` with a `// SAFETY:` comment).
  Always `flush()` the decoder after seeking.
- **Seeks that land late (lesson 8).** If the first decoded pts after a seek is
  already past `target` (open-GOP leading pictures, a wrong index, broken
  files), retry from the *previous* keyframe in the index. After 5 failures,
  decode from the start and mark the index entry unreliable.
- **Frame choice.** Return the frame with the largest pts ≤ target (TL-001: "the
  source frame at or before"). At EOF return the last frame.
- **Wild timestamps.** If decoded pts are more than 10 s away from the expected
  pts, the frame is still used. It is tagged `approx` and logged once.
- **Retargetable decode.** A running `DecodeForward` re-reads its target from an
  atomic between packets. A newer scrub request in the same GOP, *ahead* of the
  current position, simply moves the target instead of cancelling and seeking
  again. Slow forward scrubbing becomes continuous decoding instead of thrashing
  between seeks.
- **Approximate mode** (EX-004 step one): seek to the keyframe and return the
  first output frame. Optionally set `skip_frame(Discard::NonKey)` and
  `skip_loop_filter` (MLT's catch-up trick). These change pixels, so they are
  **never** allowed in Exact mode or in export.
- **Threading.** Playback sessions use FFmpeg frame threads (throughput). Frame
  threading delays the first output by about one frame per thread after a flush,
  so Scrub sessions use slice threading or ≤ 2 frame threads (latency). This is
  tuned by the scrub benchmark.

**Look-ahead opener** (lesson 16). While playing, the opener finds clips that
become active within the next 2 s of `PlayPlan` time. It opens their sessions on
`Lookahead` priority and seeks them to their first source frame, so the cut does
not hitch.

**ReverseReader (EX-007).** It decodes the GOP that holds the target forward into
the `SourceFrameCache`, under its own 512 MB sub-budget. It then hands frames
out in descending order and prefetches the previous GOP on a second lane while
the current one plays. For very long GOPs the budget limits how far behind it
can read. Stutter is expected there, and FreeCut suggests proxies.

**Preview scale does not reduce decode cost.** H.264/HEVC/AV1 cannot decode at
reduced resolution (FFmpeg `lowres` covers only a few codecs). A 4K source at a
1/4 preview still decodes in full. Proxies (PM-080) and hardware decode
(EX-018) are the real fixes. The stats tooltip should say so when decode
dominates. *Revised in review (README §7 M3a):* hardware decode for preview is
built in M3a, not v1, with a non-gating 4K30 HEVC Main10 HLG benchmark on 8 GB
machines.

### Caches

| Tier | Owner | Key | Contents | Budget | Eviction |
|---|---|---|---|---|---|
| Decoded ring | `VideoSession` | pts | last 2–4 frames | per session | FIFO |
| `SourceFrameCache` | media | `(SourceRef, pts)` | `Arc<DecodedFrame>` | 512 MB (shared with reverse); revised: a share of `MemoryBudget` (README §5.12) | weighted by bytes (`quick_cache`) |
| GPU hot ring | render | `RenderKey` | last 8 composited textures | 8 slots | LRU |
| `RamFrameCache` (v1, revised in review) | render | `RenderKey` | RGBA8 display-referred, CPU | 25 % of RAM, ≤ 8 GB (EX-016); revised: a share of `MemoryBudget` (README §5.12) | weighted by bytes (`quick_cache`) |
| `DiskSegmentCache` (v1) | render (I/O thread) + media (encode/decode) | segment key = hash of its frames' keys | 2 s ProRes 422 LT (EX-016) | 20 GB | LRU, pinned for the open project |

- **The render key** (`freecut-timeline::render_key`) is a 128-bit XXH3 of:
  - the canonical serialization of the resolved `FramePlan` at `t` (*revised in
    review, README §5.5:* without `seq_version`, `time` and `frame_index`; each node
    adds only the times and seeds it consumes, so a ripple that shifts frames keeps
    their keys and the same still shares a key across sequence times);
  - each source's media fingerprint (PM) and its proxy-or-original choice;
  - the offline or error slate state;
  - the ids of analysis results (AI-001);
  - effect and shader versions plus a global `RENDER_ABI_VERSION`;
  - the color pipeline settings;
  - the **resolved pixel size** of the `RenderTarget` (not the `PreviewScale`
    enum, because Auto depends on the viewer size).

  The hash is stable across runs (XXH3, not `DefaultHasher`), so disk segments
  survive restarts. An input missing from the key would show stale frames,
  which is the most damaging bug class here. It is guarded by property tests
  (see Test strategy).
- **Why RGBA8 in RAM.** Preview display is 8-bit (`Rgba8Unorm` present slots,
  below). Working values are already display-encoded (FX-001 L2), so 8-bit
  storage loses nothing visible on screen. It also halves the memory of
  RGBA16F: a 1080p frame is 8.3 MB.
- **Filling the cache.** The slot texture is copied to a staging buffer, and the
  scheduler's `device.poll` completes the `map_async`. Rows are padded to
  `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256 bytes); the padding is stripped when
  storing.
- **Upload on a hit.** `queue.write_texture` into the slot. Uploads are queued
  and run before the next submit on that queue, whichever thread submits. This
  is harmless, because the slot is not being sampled.
- Disk segments are written by a low-priority job through `freecut-media`
  encoders, and only when "Smart pre-render" or "Pre-render range" asks for it.
  A full disk stops pre-render and shows a toast (EX-016).

### Delivering frames to the UI

Options considered (egui-wgpu 0.36):

| Option | Verdict |
|---|---|
| `egui::TextureHandle::set(ColorImage)` every frame | **Rejected.** CPU RGBA on the UI thread, about 8 MB memcpy plus an upload per 1080p frame. |
| `Renderer::register_native_texture` / `update_egui_texture_from_wgpu_texture` | **Fallback.** The texture must be `Rgba8Unorm`. Each change needs the renderer's `RwLock` write lock and creates a bind group. Usable with ids pre-registered once per slot. |
| `egui_wgpu::CallbackTrait` paint callback | **Chosen.** Our own pipeline. Nothing is locked on the hot path. We control filtering (nearest above 200 % for pixel peeping, EX-009), the checkerboard and later HDR. |

**PresentRing** (`freecut-render::preview::present`):
- 4 `Rgba8Unorm` textures at the current target size. Each has a bind group for
  the `ViewerBlit` pipeline, created when the slot is (re)allocated.
  *Revised in review:* `clamp(128 MiB / slot_bytes, 4, 14)` textures, so the planner
  depth (clamped to `slots − 2`) can reach its adaptive range (README §2.3 rule 7).
- The compositor's output stage (FX-001 step 7) writes the RGBA16F accumulator
  into the slot. RAM cache hits upload straight into it. Both paths end in the
  same format, so the texture-id fallback and the paint callback can share
  slots.
- Slot states:

```rust
enum SlotState {
    Free,
    Rendering,
    Ready { tag: FrameTag, deadline: Option<PlayTime> },
    Shown { ui_frame: u64 },  // sampled by the UI's current frame
}
```

Rules:
- The scheduler writes only `Free` slots.
- `take_due` moves `Ready` → `Shown`. `ui_frame_finished()`, called at the start
  of the next egui `update`, moves the previously `Shown` slot to `Free`. By
  then eframe has submitted the frame that sampled it.
- On a single queue, GPU work runs in submission order, so a later write cannot
  overtake an earlier sample.
- If the UI stops painting (minimized), `Ready` slots past their deadline are
  reclaimed by the scheduler, which counts them as dropped.
- The ring's small state sits behind a `parking_lot::Mutex` held for
  microseconds. No GPU calls happen under it.

**Paint callback** (`freecut-app::player::viewer`):
- `ViewerPaintCallback { bind_group: Arc<wgpu::BindGroup>, uniforms: ViewerUniforms }`
  implements `CallbackTrait`.
- `prepare` writes the zoom/pan/letterbox uniforms to a buffer stored once in
  `CallbackResources`.
- `paint` draws one quad into egui's render pass.
- **sRGB pitfall.** If `RenderState::target_format` is an sRGB format, the
  shader must convert the display-encoded sample to linear before output, or the
  image gets encoded twice. Two pipeline variants are chosen at startup.
- Guides, safe areas and on-canvas handles are drawn afterwards with the egui
  painter (EX-011, TR-007).
- Fullscreen and a second display (EX-010, EX-020) add a paint callback in
  another egui viewport. All viewports share one device, so they sample the
  same slot. The target size is the larger of the viewers.

### Transport state machine (`freecut-app::player::transport`)

This refines the EX-002 model note (`Stopped | Playing | Scrubbing`). It is
generic over `PlaybackClock`, so tests drive it with a fake clock and fake
engines.

```rust
pub enum Transport {
    Paused    { at: Tick },                                    // shows frame `at`
    Prerolling{ plan: Arc<PlayPlan>, since: Instant },
    Playing   { plan: Arc<PlayPlan> },
    Buffering { plan: Arc<PlayPlan>, held_at: Tick, since: Instant },
    Scrubbing { target: Tick, still_since: Option<Instant> },  // drag, skim, step repeat
}
```

| Event | From | To / action |
|---|---|---|
| Space or Play | Paused | If `at == duration`, set `at = 0` (EX-002). With loop on and `at` outside the loop region, set `at = region.start` (EX-008). New epoch → `Prerolling`. Request `Play`. Feeder starts. `PlaybackActivity = Playing`. Keep the display awake. |
| Gate opens (first frame and audio ready, or 250 ms) | Prerolling | `Playing`. `audio.open_gate()`. |
| Every UI frame | Playing | `playhead = shown_frame.tag.time` (lesson 7). Auto-scroll uses the same value (TL-013). |
| No new frame for 1 s | Playing | `Buffering`. `audio.hold()`; the clock holds. Show "Buffering…" (EX-002). |
| 0.5 s of Ready frames ahead | Buffering | `Playing`. `audio.release()`. |
| `ClockState::Ended` | Playing | `Paused { at: last frame }`. |
| Space, K or pause | Playing / Buffering / Prerolling | `Paused { at: frame on screen }`. Flush audio with a 2 ms fade. Request `Still(Exact)` at Full when "Sharpen when paused" is on and the shown frame was lower quality (EX-006). |
| Step ←/→ | any | Pause first, then `at ± 1 frame` from the frame on screen, clamped without wrapping. Request `Still(Exact)` (EX-003). |
| J/L/K (EX-007) | any | New `ShuttleRate` from the step table → new epoch, flush, `Prerolling`. Pre-roll is short because the planner renders sparsely above 1×. |
| Pointer drag or skim | any | `Scrubbing`. Request `Still(Approximate)` throttled to display refresh. After 80 ms still, or on release, request `Still(Exact)`. Audio grains (AU-016). |
| Edit (new snapshot) | any | `set_document`. Playing continues (see edits above); if the sequence duration changed, `plan.amended(..)` with the new bounds or a new epoch (revised in review, see `PlayPlan`). Paused re-requests only if the playhead frame's key changed. |
| Loop region changed | Playing (loop) | `plan.amended(..)`, or a new epoch if amending is impossible (EX-008). |
| Sleep or lid close | any | `Paused`. On wake, reopen the device and stay paused (EX-002). |

The paused frame is always the frame **on screen**. It is never the clock time
and never the newest processed frame. This avoids Shotcut's off-by-one keyframe
bug.

### Seek and scrub path (EX-004), step by step

1. The UI coalesces pointer moves to at most one request per egui frame. It
   publishes `Still(FrameRequest { mode: Approximate, .. })` and increments the
   generation.
2. The scheduler wakes and computes the key.
   - If the key is in the GPU hot ring or the RAM cache, the frame is shown in
     under one frame and the request is done. The cache is exact, so no second
     step is needed.
   - Otherwise it asks the decoder pool. A queued job that has not started and
     belongs to an older generation is dropped. A running job continues only if
     it can be **retargeted** (same GOP, target ahead). Otherwise its token is
     cancelled and it stops at the next packet boundary.
3. The approximate result is published with `approx: true`. The UI shows it.
4. After 80 ms of stillness, `Exact` is requested. Its result replaces the
   approximate frame. If any request is still pending after 250 ms, the viewer
   shows a spinner (EX-001), the same idea as Kdenlive's `seekFinished`.
5. Exact frames enter the caches, so scrubbing back over a region is instant.

### Preview resolution (EX-006)

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum PreviewScale { Auto, Full, Half, Quarter }

/// Pure, unit-tested (EX-006). Even dimensions, short side >= 64 px, never above the canvas,
/// aspect kept within one pixel.
pub fn preview_target_size(canvas: (u32, u32), viewer_px: (u32, u32), scale: PreviewScale) -> (u32, u32);
```

- `RenderTarget { size, proxy_policy, quality: Preview | Sharpen }` is resolved
  on the UI thread and carried in each request.
- With Auto, a viewer resize changes the size only after it has been stable for
  150 ms, so dragging a panel does not re-allocate the slots and throw away
  cache keys every frame.
- A scale change applies from the next frame. Old RAM entries stay, under their
  own keys (EX-006).
- The ring re-allocates its 4 slots when the size changes. Frames already
  `Shown` keep their old texture until released.
- The setting is view state in `PlayerUiState`. It is not an undoable edit,
  unlike Olive's `ContextMenuSetPlaybackRes`.

### Threads, priorities and channels

| Thread | Crate | Count | Priority | Talks through |
|---|---|---|---|---|
| UI (egui) | app | 1 | normal | `ArcSwap` requests, `PresentRing` (short mutex), clock reads (seqlock), `crossbeam` commands to audio |
| Preview scheduler | render | 1 | above normal (`thread-priority`, best effort) | wake channel (`crossbeam`, capacity 1, `try_send`), `ArcSwap` request/document |
| Decode workers | media | clamp(cores/2, 2, 6) | normal | priority job queue (`crossbeam`), `DecodeTicket` (one-shot) |
| Audio feeder | audio | 1 | above normal | `rtrb` to the callback, `crossbeam` commands from the UI |
| cpal callback | audio | host | real-time (host) | `rtrb` consumer, atomics only |
| Disk cache I/O | render | 1 | below normal | bounded `crossbeam` (8) |
| Background jobs | media / analysis | pool | lowest | read the `PlaybackActivity` atomic and yield while it is not `Idle` (PM-073, AI-001) |

- During playback a `keepawake` guard keeps the display awake. Olive does the
  same with `viewerpreventsleep`.
- No async runtime is involved.

### Cancellation and shutdown

- `CancelToken = (Arc<AtomicU64> current_generation, my_generation, epoch)`.
  `is_cancelled()` is a single atomic load.
- Decode loops check it between packets. The compositor checks it between
  layers. The FFmpeg interrupt callback checks it during blocking I/O.
- A single `send_packet` or `receive_frame` call cannot be interrupted. That
  bounds cancel latency to one decode call, typically ≤ 10 ms at 1080p and more
  at 8K.
- Every result carries `(epoch, generation)`. It is checked when decoding
  finishes, when rendering finishes, and in `take_due` / `take_still`. A stale
  result is released, never shown.
- **Shutdown.** Store `Idle`, set a stop flag, drop the senders, wake every
  thread, and join with a 2 s watchdog that only logs. Threads are never killed
  (lesson 14). The cpal stream is dropped on the thread that owns it.

### Errors (thiserror in libraries, anyhow only in the app)

```rust
// freecut-media
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot open {path}")]
    Open { path: std::path::PathBuf, #[source] source: ffmpeg_next::Error },
    #[error("seek failed in {media:?} near pts {pts}")]
    Seek { media: MediaId, pts: i64 },
    #[error("decode error in {media:?} near {at:?}")]
    Decode { media: MediaId, at: Tick, #[source] source: ffmpeg_next::Error },
    #[error("media {0:?} is offline")]
    Offline(MediaId),
    #[error("hardware decoder unavailable: {0}")]
    HwUnavailable(String),       // caller retries in software (EX-018)
    #[error("cancelled")]
    Cancelled,                   // control flow; never shown to users
}

// freecut-render
#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("GPU device lost: {0}")] DeviceLost(String),
    #[error("out of GPU memory")]    OutOfMemory,
    #[error("invalid render size {0}x{1}")] InvalidSize(u32, u32),
    #[error(transparent)]            Decode(#[from] DecodeError),
    #[error("cancelled")]            Cancelled,
}

// freecut-audio
#[derive(Debug, thiserror::Error)]
pub enum AudioOutputError {
    #[error("no audio output device")] NoDevice,     // -> MonotonicClock + banner (AU-010)
    #[error(transparent)] Device(#[from] cpal::Error), // kind(): DeviceNotAvailable, StreamInvalidated, ...
}
```

How errors are handled:
- A per-frame decode error turns into an error slate for that layer (EX-005).
  It is logged once per `(source, 1 s bucket)`.
- `DeviceLost` (from `Device::set_device_lost_callback`) puts the engine in an
  error state. The viewer shows the slate and the app recreates the renderer
  (see Risks).
- `freecut-app` wraps these errors with `anyhow` context for banners and logs.

### Invariants and how the design meets the project conventions

| Invariant | Mechanism / check |
|---|---|
| Time is rational ticks; floats only at the UI edge | `PlayTime` and `Tick` are `i64` with `i128` products. `predicted_dt` (f32) is converted once in `freecut-app`. `Duration` is used only for cost estimates. |
| No decode, render or I/O on the UI thread | The UI calls only `request`, `take_due`, `reading` and `stats`, all non-blocking. A debug-only lock-time assertion on the UI thread fails tests if any lock is held over 1 ms. |
| Audio callback is real-time safe | Atomics, `rtrb` and preallocated state only. `assert_no_alloc` in tests. |
| Clock is monotonic within an epoch | Clamped in `reading()`. Property-tested. |
| Never present a stale epoch, or a frame whose key changed | Tags are checked in `take_due` and `take_still`. Keys are recomputed on a document swap. |
| Preview equals export at the same size | One compositor. Approximate decode tricks are confined to `SeekMode::Approximate`. A golden test compares `render_still` with a preview frame at Full. |
| Every edit goes through undo | The engine never edits. Transport, scale and loop are view state (EX conventions), so they have no undo entries. |
| `unsafe` only in media and render, with `// SAFETY:` | Only the hwaccel and `avformat_seek_file` FFI in `freecut-media`. The seqlock uses atomics only, so it needs no `unsafe`. |
| No `unwrap`/`expect` outside tests unless the invariant is commented | `parking_lot` locks do not poison. Channel disconnects are handled as shutdown. |

### Defaults (FreeCut defaults, to be tuned by benchmarks)

| Setting | Default |
|---|---|
| Device buffer | 1024 frames (AU-010) |
| Ring target fill | 2 buffers (max 8 after underruns, shrinks back after 10 s clean) |
| Latency clamp | 0–500 ms |
| Discontinuity fades | 2 ms |
| Present ring | 4 slots |
| Render-ahead depth | adaptive, 2–12 frames |
| Pre-roll gate | first frame and 2 audio buffers, or 250 ms |
| Spinner | 250 ms (EX-001) |
| Scrub exact delay | 80 ms still (EX-004) |
| Buffering | 1 s with no new frame → hold. Resume at 0.5 s ahead (EX-002) |
| Auto-size debounce | 150 ms |
| Decode workers | clamp(cores/2, 2, 6) |
| Sessions | ≤ 3 playback lanes per media. ≤ 24 open in total. Idle close 10 s paused, 3 s playing |
| Seek penalty | 8 frames equivalent |
| Seek retries | previous keyframe × 5, then from the start |
| Look-ahead opener | clips starting within 2 s |
| SourceFrameCache | 512 MB |
| GPU hot ring | 8 textures |
| RAM frame cache | 25 % of RAM, ≤ 8 GB (EX-016) |
| Stats window | 1 s (EX-017) |

---

## Rust ecosystem

Versions and licenses were checked with `cargo info` on this machine. The
existing stack (ffmpeg-next 9.0, eframe/egui 0.36, wgpu 30, cpal 0.18, serde 1,
thiserror 2, anyhow 1) is assumed.

**Use**

| Crate | Version | License | Why |
|---|---|---|---|
| `cpal` | 0.18.2 | Apache-2.0 | Output stream. Its `OutputCallbackInfo::timestamp()` gives `callback` and `playback` instants for latency. `cpal::Error::kind()` distinguishes `DeviceChanged`, `DeviceNotAvailable`, `StreamInvalidated` and `Xrun`. |
| `egui-wgpu` | 0.36.2 | MIT OR Apache-2.0 | `CallbackTrait` paint callbacks. `RenderState` shares the `Device` and `Queue`. Native texture registration is the fallback. |
| `rtrb` | 0.4.0 | MIT OR Apache-2.0 | Wait-free SPSC ring with a chunk API for the callback (samples + epoch marks). |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | Bounded channels with `select!` and `recv_timeout`. Used for wake-ups, decode job queues and one-shot tickets. |
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | Lock-free publishing of the latest `PreviewRequest`, `SequenceSnapshot` and `PlayPlan`. |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | Small non-real-time mutexes (present ring, session pool) without poisoning. |
| `quick_cache` | 0.7.0 | MIT | Concurrent cache weighted by bytes (`Weighter`) for `RamFrameCache` and `SourceFrameCache`. O(1), unlike the libopenshot pitfall. |
| `xxhash-rust` (feature `xxh3`) | 0.8.19 | BSL-1.0 | Stable 128-bit `render_key`. Fast enough to hash every frame. BSL-1.0 is GPL-compatible. |
| `rubato` | 5.0.1 | MIT OR Apache-2.0 | Device-boundary resampling (AU-010). Its output delay is fed into latency compensation. |
| `keepawake` | 0.6.1 | MIT | Keeps the display awake during playback, and the system awake during export (EX-022). |
| `thread-priority` | 3.1.1 | MIT | Best effort: raise the scheduler and feeder, lower background jobs. |
| `proptest` (dev) | 1.12.0 | MIT OR Apache-2.0 | Property tests for `PlayPlan`, clock, planner, seek decision and render key. |
| `loom` (dev) | 0.7.2 | MIT | Model-checks the seqlock stamp, the present ring and the ring flush protocol. |
| `assert_no_alloc` (dev) | 1.1.2 | BSD-1-Clause | Fails tests if the callback body allocates. |

**Avoid**

| Crate / approach | Why |
|---|---|
| `tokio` / async runtimes for the pipeline | FFmpeg and GPU work is blocking and CPU-bound. `spawn_blocking` pools are unbounded and have no priorities or real-time guarantees. |
| `rayon` for scheduling playback work | A global pool shared with everything, with no priorities and no cancellation, so playback can starve. It is fine inside an offline job. |
| `rodio`, `kira` | Their engines own the clock and mixing, but we need sample-exact control and the AU-010 chain. |
| `triple_buffer` 9.0.0 (MPL-2.0) | The license is compatible, but it holds a single value. We need N GPU slots with deadlines and reclaim. |
| `audio_thread_priority` 0.38 (MPL-2.0) | cpal manages its own callback thread. Adding a second mechanism adds risk for little gain. |
| `crossbeam-utils::AtomicCell<BigStruct>` for the clock stamp | Large types fall back to a global striped lock, which is not wait-free for the callback writer. |
| `std::collections::hash_map::DefaultHasher` for keys | Not stable across Rust versions or runs, which would break disk segment keys. |
| `egui::TextureHandle::set` per video frame | CPU copy and upload on the UI thread (see Delivering frames). |
| GStreamer bindings | A whole second media framework, which duplicates `freecut-media`. |

---

## Test strategy

**Pure unit and property tests** (no threads, no GPU, no FFmpeg):
- `PlayPlan` (proptest over rates, loop bounds and frame rates, NTSC included):
  - `timeline_at` is monotonic within a lap;
  - `frames_from` at 1× yields each frame of a lap exactly once;
  - wraps are exact (Out is exclusive);
  - reverse stops at 0 and forward stops at the end;
  - an amendment takes effect exactly at `effective_from`;
  - no overflow for 24 h at 120 fps and ±8×.
- Clock math, from synthetic stamp sequences with random block sizes, latencies,
  underruns, holds and epoch changes:
  - readings are monotonic and never exceed the real frames written;
  - readings hold through silence;
  - readings equal `played_at_block − latency` at callback instants;
  - `frames_to_ticks` is exact for all listed sample rates.
- `plan_due`:
  - never returns a deadline earlier than `now + cost + display_interval`;
  - at zero cost and 1×, returns every frame;
  - at 4× on a 60 Hz display, returns no more than about 60 frames per second;
  - stays within the remaining-frame clamp.
- `seek_action`, for random GOP layouts and positions: always reaches the target
  with cost ≤ min(forward, seek + penalty), and never seeks backward when the
  ring holds the target.
- `preview_target_size`: even dimensions, short side ≥ 64, ≤ canvas, aspect
  error ≤ 1 px, all EX-006 examples (5K Auto → 1080p; 1/4 of 480p clamps to
  64 px).
- `render_key` (in `freecut-timeline`):
  - mutating any field of a generated `FramePlan` changes the key (a guard
    against missing inputs);
  - an edit outside a frame's layers leaves the key unchanged;
  - an edit followed by undo restores the key exactly.
- `Transport` with a `FakeClock` and fake engine handles: every row of the state
  table, including Space at `duration`, stepping while playing, buffering
  enter/exit thresholds, sleep/wake, and "pause shows the frame on screen".

**Concurrency tests:**
- `loom`: no torn or mixed seqlock reads; `PresentRing` never has a slot both
  `Rendering` and `Shown`; `AudioRing` flush never plays an old-epoch sample
  after the mark.
- The callback body is run offline with fake buffers inside
  `assert_no_alloc::assert_no_alloc(..)` (the allocator wrapper is installed
  only in the test binary).

**Engine harness without GPU or device (deterministic):**
- `freecut-render`'s scheduler is generic over a `FrameRenderer` trait.
  `freecut-audio`'s output over an `OutputBackend` trait.
- Test implementations: `FakeRenderer` (configurable cost distribution, returns
  tagged frames) and `VirtualOutput` (pulls blocks on a virtual timeline with a
  set latency, jitter and forced underruns).
- Scenarios:
  - 2 h at 29.97 with random render costs: final |video − audio| ≤ 1 frame, no
    drift;
  - drop counters match the injected overload;
  - a seek during playback never presents an old-epoch frame;
  - edits during playback keep unchanged-key frames;
  - the loop wrap gap is ≤ one buffer;
  - the start gate opens within 250 ms.

**Decode golden tests** (real FFmpeg, small fixtures):
- Fixtures are generated by `tools/fixtures/gen.sh` with the system FFmpeg and
  committed, about 5 MB total, with the FFmpeg version recorded. Each frame
  carries its **frame index as a binary bar code**: the `geq` filter on `N`
  draws 16 vertical bands (no fonts, survives lossy coding). A decoded frame
  is decoded back to its exact index.
- Fixture set:
  - CFR 24, 25 and 30000/1001;
  - H.264 long GOP (keyint 300) with B-frames;
  - HEVC open GOP;
  - all-intra (MJPEG, ProRes);
  - VFR (irregular timestamps);
  - negative start and MP4 edit list;
  - odd dimensions;
  - rotation metadata;
  - audio longer than video;
  - a truncated file.
- Properties (proptest, 200 cases per fixture):
  - random seek sequences in Exact mode return the exact index;
  - Approximate returns the keyframe at or before the target;
  - `ReverseReader` returns strictly descending exact indices;
  - retargeted decodes return the latest target;
  - a hardware session's output equals the software session's index.

**A/V sync end-to-end** (manual or nightly, real device or virtual loopback):
- The fixture flashes white on frame 0 of every second and clicks on sample 0 of
  the same second.
- A debug overlay logs `(presented tag time, clock reading)` per vsync, and the
  audio click times.
- Assert a mean offset ≤ 1 frame. With a loopback device, also measure the
  photodiode-free timing through the stamp log.

**Performance benchmarks** (not CI-gating; run on reference machines):
- exact scrub p95 < 150 ms on 1080p keyint 250 H.264;
- start latency < 150 ms from a proxy;
- 4K HEVC 10-bit with hardware decode at 1× without drops;
- RAM cache hit-to-screen < 1 frame.

**Manual QA checklist:**
- Bluetooth headphones; unplugging the device mid-play; sleep and wake;
- monitors at 120 Hz, VRR and 50 Hz;
- 1-frame loop; J/K/L at every step; minimized window during play;
- offline media appearing and disappearing mid-play;
- GPU loss (driver reset on Windows).

---

## Risks & open questions

1. **Bluetooth and AirPlay latency** is often under-reported by hosts, so audio
   arrives late. Should FreeCut add an "A/V offset" preference (−500 to +500 ms,
   default 0)? MLT, Shotcut and Kdenlive all expose `video_delay`. The specs do
   not list one; this needs the EX/AU owners.
2. **Underrun semantics.** This design holds the clock during underruns, which
   keeps sync and gives a gap of silence. AU-010 says only "silence … audio is
   never repeated". This needs confirmation from the AU owner.
3. **"Re-planned within one buffer" (EX-002)** is met as "within the ring's
   target fill (about 2 buffers) plus device latency". A strict one-buffer
   guarantee would need a 1-buffer ring and more underruns. To be confirmed.
4. **egui frame pacing.** Is `predicted_dt` accurate enough on ProMotion/VRR and
   under the Windows DWM? If not, presentation may need its own vsync estimate,
   or `request_repaint_after` tuning. This needs measurement.
5. **Shared wgpu queue contention.** A heavy 4K effect pass submitted by the
   scheduler can delay the UI's frame, because wgpu has no queue priorities.
   Mitigation: split large composites into several submissions and cap GPU time
   per submit. A second device cannot share textures, so it is not an option.
6. **GPU device loss with an eframe-owned device.** Recovering may require
   recreating eframe's `RenderState`. The support for that in eframe 0.36 is
   unverified. The fallback is an error slate and a "Restart renderer" action.
7. **Cache-key completeness** is the highest-impact correctness risk. The
   mitigations are the property test, a `RENDER_ABI_VERSION` bump rule in the
   review checklist, and a debug mode that randomly re-renders cache hits and
   compares them.
8. **HW decode bandwidth.** The v1 CPU copy of 4K60 10-bit HEVC is about
   750 MB/s of readback and upload. That may make 1× impossible on some
   machines until zero-copy (later).
9. **FFmpeg is not interruptible** inside a single decode call. 8K or very heavy
   codecs bound the cancel latency to one call.
10. **VFR and broken timestamps.** Index-based seeking assumes probe timestamps
    match decode timestamps. The fallback heuristics (forward thresholds and
    retries) must stay in place for files without a reliable index.
11. **RGBA8 RAM cache.** HDR preview (EX-033, later) will need RGBA16F entries
    and a larger budget per frame.
12. **Spec alignment.** This document refines the EX-002 `Transport` sketch
    (adds `Prerolling` and `Buffering`) and introduces `PlayPlan`, `PlayTime`
    and `Epoch` in `freecut-timeline`. `preview-and-export.md` should reference
    them.

---

## References

Reference implementations (studied read-only; algorithms described in our own
words):

- **Olive** — GPL-3.0-or-later (`LICENSE`)
  - `app/widget/viewer/viewer.cpp`, `viewerdisplay.cpp`, `viewerplaybacktimer.cpp`, `viewerqueue.h`
  - `app/render/previewautocacher.cpp`, `rendermanager.cpp`, `rendermanager.h`, `renderjobtracker.h`, `projectcopier.h`, `cancelatom.h`
  - `app/render/framehashcache.cpp`, `playbackcache.h`, `audioplaybackcache.h`, `previewaudiodevice.cpp`, `videoparams.cpp`
  - `app/audio/audiomanager.cpp`, `app/codec/ffmpeg/ffmpegdecoder.cpp`
- **MLT** — LGPL-2.1-or-later (file headers)
  - `src/framework/mlt_consumer.c`, `src/framework/mlt_audio.c`
  - `src/modules/sdl2/consumer_sdl2_audio.c`, `src/modules/avformat/producer_avformat.c`
- **Shotcut** — GPL-3.0-or-later (file headers)
  - `src/videowidget.cpp`, `src/mltcontroller.cpp`, `src/player.cpp`, `src/util.cpp`, `src/util.h`
- **Kdenlive** — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL (SPDX headers)
  - `src/monitor/videowidget.cpp`, `src/monitor/monitorproxy.cpp`
  - `src/timeline2/view/previewmanager.cpp`, `src/kdenlivesettings.kcfg`
- **libopenshot** — LGPL-3.0-or-later (SPDX headers)
  - `src/Qt/PlayerPrivate.cpp`, `src/Qt/VideoCacheThread.cpp`, `src/Qt/AdaptivePreroll.h`, `src/Qt/AudioPlaybackThread.cpp`
  - `src/AudioReaderSource.cpp`, `src/CacheMemory.cpp`, `src/CacheDisk.cpp`, `src/Timeline.cpp`, `src/FFmpegReader.cpp`, `src/PreviewSize.h`
- **OpenTimelineIO** — Apache-2.0 (`LICENSE.txt`)
  - `src/opentime/rationalTime.h`

Crate APIs checked in local sources:

- cpal 0.18.2: `src/timestamp.rs` (`StreamInstant`, `OutputStreamTimestamp`), `src/error.rs` (`ErrorKind`), `src/traits.rs` (`build_output_stream`)
- egui-wgpu 0.36.2: `src/renderer.rs` (`CallbackTrait`, `register_native_texture` requires `Rgba8Unorm`), `src/lib.rs` (`RenderState`)
- wgpu 30.0.1: `src/api/device.rs` (`poll`, `set_device_lost_callback`), `src/api/queue.rs`
- ffmpeg-next 9.0.0: `src/format/context/input.rs` (`seek` uses stream index −1), `src/format/mod.rs` (`input_with_interrupt`), `src/codec/decoder/decoder.rs` (`skip_frame`, `skip_loop_filter`), `src/util/frame/mod.rs` (`Frame: Send + Sync`)

FreeCut specs:

- `docs/features/preview-and-export.md`: EX-001…EX-020, EX-022
- `docs/features/audio.md`: AU-010, AU-016, AU-023
- `docs/features/timeline-editing.md`: TL-001, TL-013, TL-014, TL-042
- `docs/features/effects-transitions-animations.md`: FX-001
- `docs/features/clip-transform-and-speed.md`: TR-036, TR-041
- `docs/features/project-and-media.md`: PM-035, PM-073, PM-080
- `docs/features/ai-tools.md`: AI-001
