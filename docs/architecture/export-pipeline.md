# Export / encode pipeline

> Status: draft · Crates: freecut-media, freecut-render, freecut-audio, freecut-app (pure helpers in freecut-timeline and freecut-project)

This document covers everything between pressing **Export** and having a
finished file on disk:

- stepping the timeline frame by frame;
- compositing at output size;
- converting to Y′CbCr on the GPU and reading the result back;
- encoding (software and hardware);
- mixing and encoding audio;
- muxing, fast start and color tags;
- the GIF and audio-only variants;
- progress, cancel, background jobs and the queue;
- finalizing the file.

The export dialog layout, the preset UI, caption content and analysis jobs
belong to other areas. This document only defines the hooks they plug into.

---

## What FreeCut needs

Requirements are distilled from `docs/features/preview-and-export.md` (EX),
`audio.md` (AU), `effects-transitions-animations.md` (FX),
`color-and-filters.md` (CL), `timeline-editing.md` (TL) and the paraphrased
inventory `scratchpad/inventory/preview-and-export.md` (§ numbers).

### N1. Deterministic frame stepping (EX-022, EX-026, TL-001, FX-001)
- Time is integer ticks of 1/705,600,000 s (TL-001). Output frame `n` of an
  export with rate `R = num/den` and range start `S` is evaluated at
  `t_n = S + n/R`, exactly. `N = ceil(duration × R)`.
- Output is always constant frame rate. A lower rate skips frames and a
  higher rate repeats them (no interpolation). NTSC rates stay exact in the
  container time base and are never rounded to 23.98.
- **Export never drops frames.** A slow decode makes the job slower, never
  different. Preview and export use the same shaders, and anything random is
  seeded by a stored seed hashed with the frame index (FX-001).
- A range of exactly one frame is valid. A range shorter than one output
  frame rounds up to one frame (EX-035).

### N2. Render at output size (EX-024, EX-025, EX-042)
- The compositor renders straight at the output size. It never renders at
  canvas size and then rescales.
- Fit/Fill framing is folded into the canvas-to-output transform, so each
  source is resampled once.
- Sizes are even. Sizes above `max_texture_dimension_2d` are refused.
- The GPU limit decides whether 8K is offered.

### N3. Output color (EX-032, EX-030, CL-002)
- WGSL converts the `rgba16float` working texture to Y′CbCr: BT.709 matrix,
  limited range and 4:2:0 chroma with left siting.
- A deterministic dither, seeded by the frame index, is applied when
  quantizing to 8 bits.
- 10-bit output goes straight from float to `yuv420p10`/`yuv422p10`, with no
  8-bit step in between.
- Streams and containers are tagged 709/709/709/limited (`colr` atom in
  MP4/MOV). GIF and image outputs bypass Y′CbCr.

### N4. Readback and back-pressure (EX-022 step 5)
- GPU-to-CPU readback is double buffered, with up to 3 frames in flight.
- Frames go to the encoder through a bounded channel (capacity 8).
- Audio is mixed offline in 1024-sample blocks on its own thread (AU-010) and
  is interleaved by timestamp.

### N5. Encoders and fallback (EX-029, EX-030, EX-034)
- **MVP:** H.264 High through libx264 plus AAC-LC through FFmpeg's native
  encoder, in MP4.
- **v1:**
  - HEVC (libx265, tagged `hvc1`);
  - AV1 (SVT-AV1, with libaom as fallback);
  - VP9 (libvpx);
  - ProRes (`prores_ks`);
  - Opus, FLAC and PCM audio;
  - MOV, MKV and WebM containers.
- **Hardware:** VideoToolbox, NVENC, QSV, AMF and VAAPI, with an Auto/On/Off
  setting.
  - Auto restarts from scratch in software when the hardware encoder fails to
    open or fails mid-export.
- Encoders are detected at runtime by a background probe that is cached.
  FDK-AAC is never used.

### N6. Rate control (EX-027, EX-028)
- **Tiers:** a FreeCut formula gives the bitrate in kbps. Encoding is VBR,
  with the peak at 1.5× the target and a buffer of 2× the target.
- **GOP:** 2 s, closed.
- **Custom mode (v1):**
  - VBR, CBR, constant quality or target file size;
  - two-pass (software encoders only);
  - keyframe interval from 0.5 to 10 s;
  - speed: faster, balanced or slower.

### N7. Audio (EX-031, EX-044, AU-010, AU-012, AU-032, AU-033)
- **Mix:** offline master mix through the same mixer code as preview. The
  length is exactly the range duration.
- **Video exports:**
  - AAC at 96–320 kbps, 48 kHz or 44.1 kHz, stereo (mono in v1);
  - solo is ignored unless "Export as heard" is ticked;
  - an optional "Add silent track" for silent timelines.
- **Audio-only exports:**
  - formats MP3, WAV (16-bit, 24-bit or 32-bit float), FLAC and AAC/.m4a;
  - WAV files over 4 GB switch to RF64;
  - no GPU work at all.

### N8. Muxing (EX-029, EX-040, EX-045, EX-046, EX-049)
- MP4/MOV get fast start.
- The container is named explicitly and is never guessed from the
  extension.
- Files over 4 GB get 64-bit offsets.
- Metadata is technical only by default (privacy, EX-040).
- Optional extras:
  - cover art and title/artist/comment/copyright/year tags;
  - embedded subtitle streams and sidecars;
  - chapters (later).

### N9. GIF (EX-043)
- Settings: size, fps, loop, colors (32–256), dithering, and a global or
  per-frame palette.
- Delays are in 1/100 s and spread so that the total duration is correct.
  For example, 15 fps gives 7, 6, 7…
- There is no audio. Long GIFs get a warning.

### N10. Job behavior (EX-022, EX-023, EX-039, inventory §12, §14)
- **Phases:** Preparing, Analyzing, Rendering (Pass k/2) and Finishing.
- **Progress:**
  - 0–99 % is frames muxed out of `N`, and the final 1 % is finishing;
  - the ETA uses the last 10 s and appears after 3 s;
  - speed is shown as "×real time";
  - a 1 s live thumbnail.
- **Cancel:**
  - takes effect within 1 s and leaves no partial file;
  - a confirmation is asked only after 30 s of running.
- **Output file:** written to `<final>.part`, synced to disk, then renamed
  atomically.
- **Keep-awake:** the computer is kept awake while a job runs.
- **Failures the job must report, each with its own message and code:**
  - disk full;
  - destination removed;
  - FAT32 4 GB limit (FAT16 2 GB, §14);
  - source decode error, naming the clip and time, with a "retry with
    slates" option;
  - GPU device lost;
  - out of memory;
  - a stall, offering a retry with hardware acceleration off (§14);
  - quit during export;
  - crash leftovers.
- **Names and preflight:**
  - names are sanitized; Replace and Keep both are offered;
  - export is refused when the target is a project source (EX-023);
  - preflight problems and warnings are computed from data gathered off
    the UI thread (EX-036);
  - a size estimate is shown (EX-037).

### N11. Background export and queue (EX-038, inventory §12.2)
- Each job holds an immutable snapshot taken at queue time. Later edits never
  affect it.
- One job runs at a time by default.
- Preview keeps playing, and the export runs at below-normal priority.
- The queue is persisted. Unfinished jobs show as "Interrupted" after a
  restart, with Retry.
- Closing the project while its job runs is allowed. Quitting the app asks
  first.

### N12. Same code for draft and CLI (EX-051, EX-052)
- Draft export and `freecut export …` reuse the job code.
- The CLI exit codes are 0, 2, 3 and 4.

### N13. Conventions (as restated in the task)
- Rational time everywhere; floats only for display and non-time quantities.
- No decode, render or file I/O on the UI thread.
- `thiserror` in libraries and `anyhow` only in `freecut-app`.
- `unsafe` only in `freecut-media`/`freecut-render`, each block with a
  `// SAFETY:` comment.
- No `unwrap`/`expect` outside tests unless the invariant is commented.
- Export is not an edit and has no undo entry. `SetCover` is the one undoable
  op (EX-046), and it lives in TL.

---

## How the reference editors do it

The five codebases fall into two families:

- **In-process writers:** Olive and libopenshot. The editor process renders
  frames and calls an FFmpeg wrapper.
- **The MLT family:** MLT's `avformat` consumer, driven by Shotcut and
  Kdenlive. These run in a separate `melt` process on a serialized snapshot
  of the project.

Paths below are relative to each repository root.

### Olive (GPL-3.0-or-later, C++/Qt)

**Key files:**
- `app/task/export/export.cpp`, `app/task/export/export.h`: export task;
- `app/task/render/render.cpp`: shared ticket-driven render loop;
- `app/codec/ffmpeg/ffmpegencoder.cpp`, `.h`: FFmpeg writer;
- `app/codec/exportformat.cpp`, `app/codec/exportcodec.cpp`: container and
  codec tables;
- `app/dialog/export/codec/h264section.cpp`, `av1section.cpp`: rate-control
  UI;
- `app/render/renderprocessor.cpp`, `app/render/opengl/openglrenderer.cpp`:
  readback;
- `app/common/filefunctions.cpp`: temp-file helpers.

**Snapshot.**
- `ExportTask`'s constructor copies the project with `ProjectCopier` and
  renders only the copy.
- It also forces the divider to 1, so export always renders at full
  resolution (`export.cpp`).

**Frame driving.**
- `RenderTask::Render` (`render.cpp`) keeps
  `maximum_rendered_frames = QThread::idealThreadCount()` frame tickets in
  flight and starts a new one each time one completes. Frames therefore
  finish out of order.
- `ExportTask::FrameDownloaded` parks them in `QHash<rational, FramePtr>` and
  writes them strictly in order.
- The comment notes this "can't be done in another thread since the frames
  need to be sent one after the other chronologically".
- Bounding in-flight work by thread count is what keeps memory flat.

**Audio.**
- One audio ticket covers the whole export range (`render.cpp`).
- `RenderProcessor` (`renderprocessor.cpp`, `kTypeAudio`) returns a single
  `SampleBuffer` for that range and hard-clips it with `samples.clamp()`.
- `FFmpegEncoder::WriteAudio` then slices it into chunks of 48,000 samples.
- The whole mix of a long timeline is therefore held in RAM at once.
- Audio progress is deliberately not counted, because "it's generally a lot
  faster than video" (comment in `render.cpp`).

**Readback.**
- `RenderProcessor` applies the OCIO output transform on the GPU, blits to
  the output size and calls `DownloadFromTexture`.
- That is a synchronous `glReadPixels` (`openglrenderer.cpp`). The stall is
  hidden only by having many tickets in flight.

**Y′CbCr conversion.**
- This happens on the CPU in a libavfilter graph:
  `buffer → scale(in_range=full:out_range=…) → format(<encoder pix_fmt>)`
  (`FFmpegEncoder::Open`).
- Only the range is specified. The RGB→Y′CbCr matrix is left to the scale
  filter's defaults.
- The codec context is tagged BT.709, or given an sRGB transfer when the OCIO
  output name contains "sRGB".
- The pixel matrix and the tags are set independently, so nothing guarantees
  they agree.

**Timestamps.**
- `WriteFrame` sets `pts = qRound64(time.toDouble() / av_q2d(time_base))`.
  This goes through floating point.
- Audio pts come from an integer sample counter, which is correct.

**Streams.**
- The encoder is found by name: `libx264`, or `libsvtav1` falling back to the
  generic AV1 encoder (`GetEncoder`).
- `AV_CODEC_FLAG_GLOBAL_HEADER` is set before `avcodec_open2` when the muxer
  needs it (`SetupCodecContext`). This is correct and required for
  MP4/MKV.
- The format is guessed from the file name (`avformat_alloc_output_context2`
  with only a filename). That is why the temp name keeps the extension:
  `name.tmp0.mp4` (`FileFunctions::GetSafeTemporaryFilename`).

**Overwrite.**
- Olive writes to a temp file only when the target already exists.
- At the end it calls `RenameFileAllowOverwrite`, which *removes the target
  and then renames*. That is not atomic: if the rename fails, the old file is
  already gone.
- On failure it says "Export has been saved as … instead" (`export.cpp`).

**Cancel.**
- `Run()` still calls `encoder_->Close()`, which flushes the encoders and
  writes the trailer, and only then deletes the file. The time spent
  flushing is wasted.

**Rate control.**
- The H.264 section offers CRF, target bitrate or target file size. Target
  size sets `bitrate = size / length` with min = max = target and a fixed
  `bufsize` of 2,000,000.
- The code still carries `// FIXME: Implement two-pass` (`h264section.cpp`).

**Bugs and pitfalls seen:**
- `FFmpegEncoder::Close` calls `swr_init(audio_resample_ctx_)` where it means
  `swr_free`, which leaks the resampler.
- A single `audio_frame_` is reused for every `avcodec_send_frame` without
  `av_frame_make_writable`. This relies on encoders copying the input rather
  than keeping a reference.
- `ExportFormat::GetAudioCodecs(kFormatWebM)` offers AAC, MP2, MP3 and PCM.
  The WebM muxer only accepts Opus and Vorbis, so these fail at
  `write_header` time.
- There is no `faststart` and no `hvc1` tag anywhere (grep of `app/`).

### MLT `avformat` consumer (LGPL-2.1-or-later, C)

**Key files:**
- `src/modules/avformat/consumer_avformat.c`: the whole writer;
- `src/modules/avformat/common.c`: swscale setup;
- `src/framework/mlt_consumer.c`: frame threading and dropping;
- `src/framework/mlt_audio.c`: samples per frame;
- `src/modules/avformat/blacklist.txt`: filters excluded from the avfilter
  wrapper;
- `presets/consumer/avformat/*`: presets.

**Threading.**
- `consumer_thread` pulls frames from `mlt_consumer`. With
  `real_time = -N`, `worker_get_frame` (`mlt_consumer.c`) renders with N
  worker threads but returns frames in order.
- It also fetches audio **serially** in the feeder as frames are queued.
- With `real_time > 0` the consumer **drops frames** when it falls behind
  (`drop_max`, `consecutive_dropped` in `mlt_consumer.c`). File export must
  run with `real_time ≤ 0`, and both Shotcut and Kdenlive pass negative
  values.

**Audio per frame.**
- `mlt_audio_calculate_frame_samples` (`mlt_audio.c`) gives the sample count
  for frame `p` as `round(S(p+1)) − round(S(p))`, where
  `S(p) = p × frequency / fps`.
- This "cumulative rounding difference" never drifts. It is the right
  algorithm.
- But `fps` is passed as a `float`. 30000/1001 as `f32` is about 4.7×10⁻⁹
  off, which is roughly 1 sample per hour at 48 kHz. FreeCut does the same
  computation in integers.

**Sample FIFO and interleaving.**
- Audio samples go into a byte FIFO and are encoded in `frame_size` chunks
  (`encode_audio`).
- The main loop alternates streams with `if (audio_pts < video_pts)`, using
  `double` seconds.
- Video `pts = frame_count`, which is exact integer frames in a 1/fps time
  base (`encode_video`).

**Color.**
- Each frame is converted with `sws_scale`. The `SwsContext` is created and
  freed for **every frame** (`encode_video`).
- The matrix is explicit: `mlt_set_luma_transfer` (`common.c`) calls
  `sws_setColorspaceDetails` with the BT.601/709/2020 coefficients for both
  source and destination.
- RGB→YUV uses `SWS_BICUBIC` with accurate rounding (`mlt_get_sws_flags`).
- `open_video` switches `colorspace` to RGB when the encoder's pixel format is
  RGB.

**Pixel format default.**
- With no `pix_fmt` set, `add_video_stream` uses the encoder's *first*
  supported format.
- For FFmpeg's `gif` encoder the list starts with `rgb8`
  (`ffmpeg -h encoder=gif` on this machine), a fixed 3-3-2 palette.
- `palettegen`/`paletteuse` are on the avfilter blacklist
  (`blacklist.txt`), so the `GIF` preset (`presets/consumer/avformat/GIF`)
  cannot use an adaptive palette.

**Failure policy (pitfalls):**
- If the video encoder fails to open, MLT logs "Unable to encode video -
  disabling video output" and **continues without video** (`open_video`).
  Audio works the same way (`open_audio`, then the stream is removed in
  `consumer_thread`). The job fails only if *both* streams are gone.
- Encode errors are tolerated until `++error_count > 2` (`encode_video`,
  `encode_audio`). Up to two consecutive frames can be lost silently.
- `open_audio` sets `AV_CODEC_FLAG_GLOBAL_HEADER` **after**
  `avcodec_open2`, which is too late to have any effect. The video path sets
  it before opening.
- The last partial audio frame is sent with
  `nb_samples = FFMAX(samples, audio_input_frame_size)`, so it is padded to
  a full encoder frame. On the planar path the padding is silence
  (`interleaved_to_planar` memsets). The audio stream ends up to one encoder
  frame longer than the content.
- Two-pass logs default to `<target>_2pass.log`. For x264, cleanup also has
  to delete `x264_2pass.log*` from the current working directory (end of
  `consumer_thread`), because libx264's stats file was not redirected.

**Flush order.**
- At the end MLT drains the audio FIFO, then flushes the audio encoders (the
  comment warns about "untimestamped packets"), then flushes video.
- The trailer is written only if at least one frame was processed.

**Cover art.** `add_attached_pic_stream`:
- checks container support with `avformat_query_codec`;
- uses an *attachment* stream with `filename`/`mimetype` metadata for
  Matroska;
- uses a video stream with `AV_DISPOSITION_ATTACHED_PIC` and one packet
  written right after the header for MP4/M4A/MP3/FLAC.

**Hardware.** VAAPI and Vulkan encoders get a `hwupload` filter graph
(`setup_hwupload_filter`). Other encoders take system-memory frames.

**Presets** (`presets/consumer/avformat/`):
- `YouTube`: MP4, `movflags=+faststart`, AAC 384k, x264 `crf=23 g=15 bf=2`;
- `x264-medium`: `crf=18`;
- `SVT-AV1`: WebM, Opus, `crf=32`, `preset=5` via `svtav1-params`;
- `ten_bit/DNxHR-HQ`: `movflags=+write_colr+faststart`;
- `ten_bit/ProRes 422`: sets `vendor=apl0`, which makes the file claim to
  come from Apple's encoder.

**Porting.** LGPL-2.1+ is compatible, but there is nothing worth copying
verbatim. The cumulative-rounding and timestamp-ordered interleave ideas are
reimplemented below with integer math.

### Shotcut (GPL-3.0-or-later, Qt on MLT)

**Key files:**
- `src/docks/encodedock.cpp`: dialog, settings, encoder mapping, hardware
  detection;
- `src/jobs/encodejob.cpp`, `src/jobs/meltjob.cpp`,
  `src/jobs/abstractjob.cpp`: jobs;
- `src/jobqueue.cpp`: queue;
- `src/util.cpp`: disk-space check.

**Snapshot and isolation.**
- `createMeltJob` serializes the producer to MLT XML in a temporary file and
  runs a separate `melt` process on it (`MeltJob::start`).
- The UI survives codec or driver crashes, and the job input is immutable.

**Checks before export:**
- `checkForMissingFiles` uses `MltXmlChecker`;
- `Util::warnIfLowDiskSpace` (`util.cpp`) checks free space;
- "You cannot write to a file that is in your project" is a **substring
  search of the XML** for the target path (`createMeltJob`). This is fragile
  with symlinks and case-insensitive file systems;
- `JOBS.targetIsInProgress` rejects a second job writing the same file;
- pending analysis filters are offered as jobs first (`enqueueAnalysis`).

**Parallelism.**
- `real_time = -min(cores − 1, 4)`, or `-1` in GPU mode
  (`on_encodeButton_clicked`).
- If a parallel job fails, `EncodeJob::onFinished` rewrites
  `real_time="-1"` and restarts the whole job without parallelism. This shows
  that out-of-order rendering exposed thread-safety bugs.

**Progress and ETA.**
- Progress is parsed from `melt -progress2` stderr ("percentage:",
  `MeltJob::onReadyRead`).
- `AbstractJob::estimateRemaining` gives the average time per percent since
  the first percent.
- Pause is `SIGSTOP`/`DebugActiveProcess`. Priority is `nice -n 3` or
  `SetPriorityClass(BELOW_NORMAL)` (`abstractjob.cpp`).

**Pitfall: success heuristic.**
- `MeltJob::onFinished` sets `exitCode = 0` whenever progress reached 99 %
  ("Prevent false negative …").
- A crash during the trailer or fast-start pass is then reported as
  *success* with an unplayable file.

**Rate-control mapping per encoder family** (`collectProperties`). This is
the most complete map among the references:
- **libx265:** everything goes through `x265-params` (bitrate and
  `vbv-maxrate`/`vbv-bufsize` in kbps, `crf`, `keyint`, `bframes`,
  `scenecut=0` for a strict GOP, and `max-cll`/`master-display` for HDR).
- **libsvtav1:** `rc=2:tbr=…:buf-sz=<ms>` for CBR and `crf` for quality;
  `scd`, `enable-dg` and `pred-struct` emulate the GOP and B-frame
  controls.
- **NVENC:** `cbr=1`, `rc=vbr`+`cq` for quality, `qmin` for constrained.
- **AMF:** `rc=cbr|cqp|vbr_peak`.
- **VAAPI:** `vglobal_quality`.
- **QSV:** `qscale`.
- **VideoToolbox:** `qscale` and `pix_fmt=nv12`.
- **libvpx:** `crf` plus `vb=0`. The comment says VP9 "needs 0 to prevent
  constrained quality mode".
- Two-pass is enqueued as two jobs, and only for software encoders in
  non-quality mode (`enqueueMelt`).

**Hardware detection.**
- `detectHardwareEncoders` spawns the bundled `ffmpeg` once per candidate
  encoder: a 640×360 `lavfi` color source, one frame, a 2 s start timeout and
  a 4 s finish timeout.
- VideoToolbox and MF get `nv12`. VAAPI gets `hwupload`.
- Probing in a child process keeps driver crashes out of the app.

**Other.**
- `checkFrameRate` warns when the export rate is higher than the project
  rate.
- After a job, actions include Open, Show in folder, measure quality (VQM),
  and embed chapters through an ffmpeg remux (`EncodeJob::onEmbedChapters`).

### Kdenlive (GPL-3.0-only OR KDE-accepted GPL, Qt on MLT)

**Key files:**
- `src/dialogs/renderwidget.cpp`: dialog and parameters;
- `src/render/renderrequest.cpp`: job construction;
- `renderer/renderjob.cpp`: the `kdenlive_render` helper process;
- `data/profiles.xml`: presets;
- `src/renderpresets/renderpresetmodel.cpp`: preset parameters.

**Jobs.**
- `RenderRequest::process` writes the playlist, which is the snapshot, and
  creates jobs per pass, per guide section and per audio track
  (`createRenderJobs`, `prepareMultiAudioFiles`).
- `setDocGeneralParams` adds the consumer with in/out, rescale and
  deinterlacer settings.

**Two-pass** (`setDocTwoPassParams`):
- pass 1 gets `pass=1`, `passlogfile=<out>_2pass.log`, `fastfirstpass=1` and
  **`an=1`** (no audio in pass 1);
- x265 instead gets `pass=N:stats=<file>` inside `x265-params`, with `:` in
  the path escaped as `\:`. That escaping matters for Windows drive letters.

**Process and progress** (`renderjob.cpp`):
- `RenderJob` wraps melt and sends JSON progress over a `QLocalSocket`;
- pass 1 maps to 0–50 % and pass 2 to 50–100 % (`receivedStderr`);
- updates are throttled to one per second.

**Pitfalls:**
- Output goes directly to the destination. `slotAbort` deletes `m_dest`, so
  overwriting an existing file destroys it as soon as rendering starts.
- Success is judged by "the output file exists" (`slotIsOver`).
- Embedded subtitles are added afterwards by an `ffmpeg … -c copy -f matroska`
  remux into a temp file, which is then renamed over the destination
  (`slotCheckSubtitleProcess`). The container may no longer match the
  extension.

**Threads.**
- `real_time = -processingthreads`.
- Encoder threads come from a setting, with **libx265 capped at 16**
  (`renderwidget.cpp`, "libx265 encoding is limited to 16 threads max").

**Presets** (`data/profiles.xml`): x264 MP4 with `movflags=+faststart`
`crf=%quality g=15`; NVENC, VideoToolbox (`pix_fmt=yuv420p`) and VAAPI
variants.

**License.** GPL-3.0-only code. Reading it for ideas is fine. Copying it would
make that part GPL-3.0-only, so reimplement instead.

### libopenshot (LGPL-3.0-or-later, C++)

**Key files:**
- `src/FFmpegWriter.cpp`, `src/FFmpegWriter.h`: writer;
- `src/Frame.cpp` (`GetSamplesPerFrame`): samples per frame.

**API.**
- An in-process writer: `SetVideoOptions`/`SetAudioOptions`/`SetOption`, then
  `PrepareStreams`, `WriteHeader`, `WriteFrame`… and `WriteTrailer`.
- The format comes from the file extension (`auto_detect_format`).
- Fast start is the `muxing_preset` option `mp4_faststart` (`SetOption`).

**Audio per frame.**
- Each `WriteFrame` encodes that frame's audio and then its video
  (`write_frame`).
- `GetSamplesPerFrame` (`Frame.cpp`) uses doubles and also rounds the
  cumulative totals down to a multiple of the *channel count*. This mixes up
  per-channel and interleaved counts and adds up to (channels − 1) samples
  of jitter per frame.

**Color pitfall.**
- `process_video_packet` creates one persistent `SwsContext` (RGBA →
  encoder format, `SWS_FAST_BILINEAR` by default).
- It never calls `sws_setColorspaceDetails`, so swscale's default matrix
  (`SWS_CS_DEFAULT`, which is BT.601; see `sws_getCoefficients(SWS_CS_DEFAULT)`
  in MLT's `common.c`) is used.
- The writer never sets `color_primaries`/`color_trc`/`colorspace` (grep of
  `FFmpegWriter.cpp`). HD players assume BT.709 for untagged streams. This is
  the classic "export looks slightly different" shift.

**Audio precision.** `write_audio_packets` converts float samples to int16
with saturation *before* resampling to the encoder format (usually `fltp`).
That is undithered 16-bit truncation of the master mix.

**Quality defaults.**
- `open_video` forces `max_b_frames = 0` for most codecs unless
  `allow_b_frames` is set.
- `crf=0` on H.264 silently switches to `yuv444p` (`SetOption`), and High
  4:4:4 does not play on most devices.
- In the `qp` switch, the AV1 case falls through into the HEVC case (there is
  no `break`).

**Hardware.**
- `av_hwdevice_ctx_create` uses an adapter index taken from settings.
- Hardware H.264 is forced to Constrained Baseline + `zerolatency` with no
  B-frames.
- VAAPI VBR needs `rc_max_rate = bit_rate`, or else "a small low quality file
  is generated".
- Frame upload errors are only printed (`write_video_packet`).
- `hvc1` is tagged only for **software** HEVC (`open_video`). Hardware HEVC
  in MP4 keeps FFmpeg's default `hev1`, which Apple players refuse.

**Flush pitfall.**
- `flush_encoders` begins with `if (… audio … frame_size <= 1) return;`. With
  PCM audio the **video** encoder is never flushed, so frames held in the B-frame
  or lookahead queue are at risk.
- Errors during flush are only logged.

### OpenTimelineIO (Apache-2.0)

- OTIO has no export pipeline. It is relevant here only as a contrast for
  time math.
- `RationalTime` stores `double _value, _rate`, and `to_frames()` truncates
  with `int(_value)` (`src/opentime/rationalTime.h`). A value computed as
  2.9999999 frames becomes frame 2.
- This is why FreeCut keeps integer ticks all the way to the encoder time
  base.

### Comparison

| Concern | Olive | MLT / Shotcut / Kdenlive | libopenshot | FreeCut (below) |
|---|---|---|---|---|
| Process model | in-process tasks | separate `melt` process per job | in-process, called by app | in-process threads (MVP); job API is process-agnostic so the CLI (EX-052) can become an out-of-process worker |
| Snapshot | `ProjectCopier` deep copy | XML file written at enqueue | caller's responsibility | `Arc` document snapshot + serialized copy for retry |
| Frame order | out-of-order tickets, reorder map | in-order queue, N workers | caller loop, sequential | strictly sequential composite, pipelined decode/readback/encode |
| Never drop frames | yes | only with `real_time ≤ 0` | yes | yes (no drop path exists in export code) |
| Video pts | float → int | exact frame counter | exact frame counter | exact frame index in time base `den/num` |
| Audio slicing | whole range in RAM | per-frame cumulative rounding (`float` fps) | per-frame, doubles, channel-rounded | fixed 1024-sample blocks, exact integer sample spans |
| A/V interleave | encoder order | `double` pts compare | per-frame | exact rational compare + `av_interleaved_write_frame` |
| RGB→Y′CbCr | CPU avfilter, matrix default | CPU swscale, explicit matrix | CPU swscale, default (601) matrix | GPU WGSL, explicit BT.709, dithered, tags derived from same struct |
| Color tags | BT.709 (or sRGB trc) | from profile | none | 709/709/709/tv + chroma loc |
| Temp file | only when overwriting; remove+rename | none (direct write) | none | always `.part`, fsync, atomic rename |
| Encoder open failure | error | silently drops that stream | exception | error; Auto→next candidate |
| Encode errors | error | 2 tolerated | exception | first error fails the job |
| HW fallback | n/a | Shotcut: retry w/o parallel | n/a | restart from scratch with software |
| GIF | n/a | fixed `rgb8` palette | n/a | 2-pass `palettegen`/`paletteuse` |
| Cancel | flush+trailer, then delete | kill process, delete | n/a | abandon (no flush), delete `.part`, ≤ 1 s |

### Lessons carried into the design (P-list)

- **P1 Dropping frames:** never possible in export. Export code has no
  "late frame" path; MLT's `real_time > 0` mode is the counter-example.
- **P2 Parallel out-of-order rendering:** it forces random-access decoding
  (seeks on long-GOP sources). It also exposed thread-safety bugs (Shotcut's
  retry without parallelism). FreeCut composites sequentially and gets its
  parallelism from pipelining instead.
- **P3 Float timestamps:** pts are always integers derived from integers
  (Olive's `qRound64(time.toDouble()/…)` and MLT's `float fps` are the
  counter-examples).
- **P4 Matrix and tags must come from one value** (libopenshot's untagged
  601, Olive's unset matrix).
- **P5 Silently dropping a stream or tolerating encode errors** turns bugs
  into corrupt deliverables (MLT).
- **P6 "99 % means success":** success requires the trailer to be written, the
  file closed with no error, and a verification probe (Shotcut's heuristic is
  the counter-example).
- **P7 Remove-then-rename or writing directly to the destination** loses the
  user's old file (Olive, Kdenlive).
- **P8 GLOBAL_HEADER must be set before `avcodec_open2`** (MLT's audio path
  sets it after).
- **P9 HEVC in MP4/MOV needs `hvc1`** whatever the encoder, hardware included
  (libopenshot tags only software HEVC).
- **P10 The container/codec table must be validated against the muxer**
  (Olive offers AAC in WebM).
- **P11 Two-pass stats must go to an explicit temp path.** Paths inside
  `x265-params` need `\:` escaping (MLT's CWD cleanup, Kdenlive's escaping).
- **P12 The audio mix stays `f32` until the encoder's own sample format.** Any
  reduction to integers is dithered (libopenshot's int16 detour).
- **P13 Encoder input frames come from a pool or are made writable** before
  reuse (Olive's reused `audio_frame_`).
- **P14 A whole-range audio buffer** grows linearly with timeline length
  (Olive).
- **P15 Video and audio must both be flushed regardless of codec**
  (libopenshot's early return).
- **P16 Never force High 4:4:4 or Baseline profiles implicitly** (libopenshot).
- **P17 The x265 thread count is capped at 16** (Kdenlive).
- **P18 Child-process hardware probe with a timeout** (Shotcut).
- **P19 Don't spoof vendor IDs** (MLT's `vendor=apl0` ProRes preset).
- **P20 A muxer remux must not change the container behind the user's
  back** (Kdenlive's subtitle remux to Matroska).

---

## Recommended FreeCut design

### 1. Shape of the pipeline

```
 UI thread (freecut-app)          ExportJob orchestrator thread (freecut-app::jobs)
 ┌──────────────────────┐  start  ┌─────────────────────────────────────────────────┐
 │ Export dialog / Jobs │ ──────▶ │ 1 Prepare: snapshot, resolve plan, claim .part, │
 │ panel, status bar    │         │   keep-awake, (Analyze)                         │
 │                      │ ◀────── │ 2 Run attempt(s) — scoped threads:              │
 │ reads ProgressCell   │ progress│                                                 │
 └──────────────────────┘ (atomics│  render thread ──VideoFrame (bounded, bytes)──┐ │
                           + thumb)│  (freecut-render::export; owns its wgpu Device │ │
                                  │   + decode readers from freecut-media)         ▼ │
                                  │  audio thread ──AudioBlock (bounded 64)──▶ mux  │
                                  │  (freecut-audio::offline)               thread │
                                  │                       (freecut-media::encode:   │
                                  │                        EncodeSession owns       │
                                  │                        AVFormatContext + codecs)│
                                  │ 3 Finalize: fsync, atomic rename, sidecars,     │
                                  │   verify, report                                │
                                  └─────────────────────────────────────────────────┘
```

**Design principles:**
- **One writer thread** owns every FFmpeg context of a job: the muxer and all
  encoders. FFmpeg contexts are not thread-safe. Encoders do their own
  internal threading.
- **Producers never wait for each other.**
  - The mux thread pulls from whichever stream is *behind* in presentation
    time, comparing exactly as rationals.
  - A producer blocks only when its own bounded channel is full.
  - So the mux thread never waits on a full channel, and the pipeline cannot
    deadlock.
  - Memory is bounded by the channel capacities. This is MLT's
    `audio_pts < video_pts` idea, done in integers.
- **Sequential composite, pipelined everything else:**
  - frame `n+1` is composited while frame `n` is read back and frame `n−k` is
    encoded;
  - decoders read ahead sequentially, which is fast on long-GOP sources (P2).
- **The job owns a snapshot.** It never reads the live document (EX-022
  step 1).

### 2. Crate and module layout

```
freecut-timeline/src/export.rs        (pure) ExportClock, frame/sample spans, range
                                       resolution (EX-035), marker→chapter (EX-049)
freecut-project/src/export/           (pure + serde) ExportSettings, presets, compat
  settings.rs  presets.rs              table, validate(), output_dimensions(),
  compat.rs    estimate.rs  names.rs   recommended_kbps(), estimate_size(), names,
  gif.rs                               GIF timing helpers
freecut-media/src/frame.rs            (pure) PixelLayout, FrameGeometry, VideoFrame,
freecut-media/src/audio_block.rs      AudioBlock, BufferPool  — the wire types
freecut-media/src/cancel.rs           Cancel (atomic flag + select-able receiver)
freecut-media/src/encode/
  caps.rs        EncoderCaps, probe protocol, cache key
  spec.rs        (pure) EncodeSpec, EncoderImpl, Container, ColorTags
  options.rs     (pure) rate control → EncoderSetup per encoder (snapshot-tested)
  session.rs     EncodeSession: open order, push, finish/abandon, MuxStats
  video.rs       VideoStreamWriter: frame pool, pts, send/receive, codec_tag
  audio.rs       AudioStreamWriter: SampleFifo, sample-format conversion, last frame
  gif.rs         palettegen/paletteuse graphs (two-pass), GIF timing
  mux.rs         container options: faststart, rf64, metadata whitelist, cover art
  ffi.rs         the few raw calls ffmpeg-next lacks; every block has // SAFETY:
  error.rs       EncodeError
freecut-render/src/output/
  layout.rs      FrameGeometry → GPU dispatch geometry (tile sizes, bands)
  convert.rs     OutputConverter (+ convert.wgsl): rgba16f → planes, dither
  readback.rs    ReadbackRing (3 slots, timed waits, device-lost)
  reframe.rs     (pure) Fit/Fill canvas→output transform, even-snapped content rect
  export.rs      ExportRenderer: the frame loop, thumbnails
freecut-audio/src/offline.rs          OfflineMix: Iterator<Item = Result<AudioBlock>>
freecut-app/src/jobs/
  export.rs      ExportJob (orchestrator), attempts & HW fallback, finalize
  queue.rs       JobQueue, persistence, interrupted jobs, leftovers registry
  progress.rs    ProgressCell, ETA window, stall watchdog
  platform.rs    keep-awake, thread priority, reveal/open (worker thread)
```

Dependency direction follows the crate rules: `render`/`audio` → `media` +
`timeline`, and `app` → everything. The wire types (`VideoFrame`,
`AudioBlock`, `PixelLayout`, `Cancel`) live in `freecut-media`, the lowest
crate that both producers and the encoder depend on. They are plain Rust with
no FFI, so the render and audio crates never touch FFmpeg types.

### 3. Exact frame stepping (freecut-timeline)

```rust
// freecut-timeline/src/export.rs — pure, no I/O. Written for FreeCut.
use crate::time::{FrameRate, Tick, TimeRange, TICKS_PER_SECOND}; // TL-001 TimeBase

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExportTimeError {
    #[error("export range is empty")]
    EmptyRange,
    #[error("time arithmetic overflow")]
    Overflow,
}

/// Output grid of one export: frame `n` shows timeline time `start + n/R`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExportClock {
    range: TimeRange,
    rate: FrameRate,
    frames: u64,
}

impl ExportClock {
    pub fn new(range: TimeRange, rate: FrameRate) -> Result<Self, ExportTimeError> {
        if range.duration.0 <= 0 {
            return Err(ExportTimeError::EmptyRange);
        }
        // N = ceil(dur * num / (TPS * den)); i128 so 8K@120 for days cannot overflow.
        let n = i128::from(range.duration.0) * i128::from(rate.num);
        let d = i128::from(TICKS_PER_SECOND) * i128::from(rate.den);
        let frames = u64::try_from((n + d - 1) / d).map_err(|_| ExportTimeError::Overflow)?;
        Ok(Self { range, rate, frames })
    }

    pub fn frame_count(&self) -> u64 { self.frames }

    /// Timeline time of output frame `n`. Floor onto the tick grid; exact for every
    /// rate TL-001 accepts (their frame duration is a whole number of ticks).
    pub fn frame_time(&self, n: u64) -> Result<Tick, ExportTimeError> {
        let off = i128::from(n) * i128::from(TICKS_PER_SECOND) * i128::from(self.rate.den)
            / i128::from(self.rate.num);
        let off = i64::try_from(off).map_err(|_| ExportTimeError::Overflow)?;
        self.range.start.0.checked_add(off).map(Tick).ok_or(ExportTimeError::Overflow)
    }

    /// Samples covering exactly the range: AU-010's `floor(ticks * sr / TPS)` at both
    /// ends, so consecutive ranges tile without gaps or overlaps.
    pub fn sample_count(&self, sample_rate: u32) -> Result<u64, ExportTimeError> {
        let at = |t: i64| i128::from(t) * i128::from(sample_rate) / i128::from(TICKS_PER_SECOND);
        let end = self.range.start.0.checked_add(self.range.duration.0).ok_or(ExportTimeError::Overflow)?;
        u64::try_from(at(end) - at(self.range.start.0)).map_err(|_| ExportTimeError::Overflow)
    }
}
```

**Invariants (unit-tested):**
- `frame_time(0) == range.start` and `frame_time(N−1) < range.end`.
- Successive frame times are strictly increasing. At the project rate they
  are exactly one frame duration apart.
- For 25→30 fps the cadence repeats every 6 output frames.
- Over 1 h at 30000/1001 there is no lost or duplicated tick (EX-026).

**Video timestamps.**
- The encoder time base is `den/num` (for example 1001/30000) and
  `pts = n`. They are integers end to end (P3).
- Packets are rescaled to the stream time base only with
  `av_packet_rescale_ts`, *after* `avformat_write_header`. The muxer
  finalizes `st->time_base` there.

**Audio timestamps.**
- The encoder time base is `1/sample_rate` and `pts` is the running sample
  count.
- The first AAC/Opus packets carry the encoder's priming delay as a negative
  pts. That is correct: the MP4 muxer writes an edit list. Never "fix" it.

**Output timeline starts at 0.** Captions, chapters and cover times are
rebased by subtracting `range.start` (EX-035).

### 4. The export plan (freecut-app, from freecut-project settings)

`ExportSettings` is owned by `freecut-project::export` (EX-021) and keeps the
spec's type names. At job start the app resolves it into an immutable plan
that holds only concrete choices:

```rust
pub struct ExportPlan {
    pub snapshot: Arc<DocumentSnapshot>,       // TL's cheaply-cloned document
    pub clock: ExportClock,                    // range + output rate
    pub video: Option<VideoPlan>,              // None for audio-only
    pub audio: Option<AudioPlan>,              // None if excluded / GIF
    pub target: FinalTarget,                   // folder, stem, ext, conflict policy
    pub container: Container,                  // explicit; never guessed from extension
    pub passes: PassPlan,                      // Single | TwoPass { stats_dir } | GifTwoPass
    pub encoder_candidates: Vec<EncoderImpl>,  // e.g. [HevcVideoToolbox, X265] for Auto
    pub extras: Extras,                        // metadata whitelist, cover, sidecars, chapters
}

pub struct VideoPlan {
    pub width: u32, pub height: u32,           // even; ≤ GPU and encoder limits
    pub framing: CanvasToOutput,               // Fit/Fill folded into the transform
    pub color: OutputColor,                    // matrix/range/depth (EX-032)
    pub rate_control: ResolvedRateControl,     // kbps, peak, buffer, gop, speed, quality
    pub draft: bool,                           // EX-051 (proxies, preview scale)
}
```

**Resolution steps, all pure and testable:**
- `validate(&settings, &caps, &seq_info)`: EX-036 problems block the export.
- `output_dimensions(canvas, size)`: EX-024.
- `recommended_kbps`: EX-027.
- `resolve_rate_control`: EX-028.
- `encoder_candidates(codec, hw_pref, caps, size)`: EX-034.

### 5. Render side (freecut-render)

#### 5.1 ExportRenderer and its own GPU device

> **Revised (review, 2026-10-12; README C19).** A second device from the shared adapter is not
> allowed by wgpu 30: "an Adapter may only be used once to create a device", with "request_device()
> was already called on this Adapter" listed under Panics and the rule only "not currently
> enforced" (`wgpu-30.0.1/src/api/adapter.rs:44-57`). A fallback cannot catch that panic. The
> MVP (M4) therefore renders export on the **shared** device with its own `Compositor`, pools
> and caches; `ExportGate` throttles and preview is idle or paused during export, and there is
> no second pipeline compile, text atlas or `SourceCache`. The shared device's descriptor
> already requests the adapter's real texture and buffer limits (README §2.3), so the limit
> notes below still hold. If isolation is wanted after measuring, export gets a **fresh
> adapter** (`instance.enumerate_adapters(backends)`, matched on `get_info()` vendor, device,
> backend and driver) and calls `request_device` on that adapter only. The original text
> follows as the research record.

- The job creates its **own `wgpu::Device` from the shared `Adapter`**. It
  does not reuse the preview device. This gives three things:
  1. a device-lost event in either one doesn't kill the other;
  2. export allocations are freed in full when the job ends;
  3. preview textures cannot leak into export by accident.
- The cost is a second pipeline compile, about 1 s once per job.
- Request the adapter's real limits, not the defaults. wgpu's defaults are
  8192 px textures, a 256 MiB `max_buffer_size` and a 128 MiB
  `max_storage_buffer_binding_size`.
  - An 8192×8192 10-bit 4:2:2 frame is 268 MB, which is over the default
    storage limit.
  - When the adapter can't raise the limit, the converter dispatches in
    horizontal **bands**, each writing its own buffer range.
- `device.set_device_lost_callback` raises a flag. The loop turns it into
  `RenderError::DeviceLost`. The job fails with "Retry" and never resumes
  mid-file (EX-022).

```rust
pub struct ExportRenderer {
    gpu: ExportGpu,                     // own Device + Queue
    compositor: Compositor,             // the FX-001 pipeline, same shaders as preview
    converter: OutputConverter,         // rgba16f → planes (or rgba8 for GIF/stills)
    ring: ReadbackRing,                 // 3 slots
    clock: ExportClock,
    framing: CanvasToOutput,
    sources: SourceReaders,             // sequential, frame-accurate decoders (freecut-media)
    pool: BufferPool,                   // recycled Vec<u8> sized to FrameGeometry
}

impl ExportRenderer {
    /// Composites every output frame in order and sends it to `out`.
    /// Returns when all N frames were handed over, or on the first error / cancel.
    pub fn run(
        mut self,
        out: &crossbeam_channel::Sender<VideoFrame>,
        cancel: &Cancel,
        thumbs: &dyn Fn(Thumbnail),
    ) -> Result<(), RenderError> {
        for n in 0..self.clock.frame_count() {
            cancel.check()?;                                   // ≤ one frame of latency
            let t = self.clock.frame_time(n)?;
            let plan = self.snapshot().frame_plan(t);          // FX-001 query, pure
            let tex = self.compositor.render(&plan, &self.framing, &mut self.sources)?;
            let mut enc = self.gpu.encoder();
            self.converter.record(&mut enc, &tex, self.ring.slot_buffer(n), n);
            self.ring.submit(&self.gpu, enc, n)?;
            if self.ring.is_full() {
                let frame = self.ring.harvest_oldest(&self.gpu, &self.pool)?;
                send_or_cancel(out, frame, cancel)?;           // crossbeam select! on cancel
            }
            if thumbs_due(n) { thumbs(self.compositor.thumbnail(&self.gpu, &tex, 320)?); }
        }
        while let Some(frame) = self.ring.harvest_remaining(&self.gpu, &self.pool)? {
            send_or_cancel(out, frame, cancel)?;
        }
        Ok(())
    }
}
```

**Source readers.**
- `SourceReaders` are export-only decoders from `freecut-media`. They read
  sequentially, frame-accurately and ahead, one per active clip.
- They never use proxies, except in draft mode (EX-051), and never use preview
  caches. They open their own file handles.
- Default: **software decoding** for export, so the result is reproducible
  across machines. Hardware decode is an opt-in, the same as Shotcut's
  separate export setting that removes `MLT_AVFORMAT_HWACCEL` in
  `meltjob.cpp` (see Open questions).
- A decode error becomes `RenderError::Decode { clip, at: Tick }`. With
  "Retry using slates", the reader returns the EX-005 error slate for
  unreadable frames instead.

#### 5.2 Output conversion (WGSL compute, EX-032)

A compute shader reads the composited `rgba16float` texture and writes
**tightly specified planes into a storage buffer**. It does not write to
`R8`/`R16` render targets, because:
- `R16Unorm` is not core WebGPU;
- `copy_texture_to_buffer` forces 256-byte row padding;
- P010 needs its values in the high bits.

The buffer is then copied into the slot's `MAP_READ` staging buffer.

**Layouts** (`freecut_media::frame::PixelLayout`):

| Layout | Used by | Planes | Sample storage |
|---|---|---|---|
| `Yuv420p` | x264, x265, SVT-AV1, VP9, OpenH264 (8-bit) | Y, Cb, Cr | `u8` |
| `Nv12` | VideoToolbox, NVENC, AMF, QSV (8-bit) | Y, CbCr interleaved | `u8` |
| `Yuv420p10` | x265, SVT-AV1, VP9 (10-bit) | Y, Cb, Cr | `u16` LE, value in **bits 0–9** |
| `P010` | HEVC VideoToolbox/NVENC/AMF (10-bit) | Y, CbCr | `u16` LE, value in **bits 6–15** (`v << 6`) |
| `Yuv422p10` | `prores_ks` | Y, Cb, Cr (full-height chroma) | `u16` LE, bits 0–9 |
| `Rgba8` | GIF, image sequence, thumbnails | RGBA | `u8`, sRGB-encoded working values, no matrix |

Mixing up `Yuv420p10` (low bits) and `P010` (high bits) is a classic bug. It
gives a near-black or garbage picture. The tests check both.

**Tiling and strides.**
- One invocation owns an **8×2 pixel tile** for 8-bit layouts and a 4×2 tile
  for 16-bit ones. It writes whole `u32` words, so no two invocations ever
  write to the same word.
- Strides are rounded up so that tiles stay word-aligned:
  - Y stride is `align_up(width × bytes, 8)`;
  - chroma stride is half of that.
- Widths like 854 (480p at 16:9) work. The padding bytes are ignored when
  copying into the encoder's frame.

**Math.** This is FreeCut's own statement of the standard BT.709 equations
(ITU-R BT.709):

```
R′G′B′ = clamp(rgb_working, 0, 1)           // preview shows the same clamp (8-bit swapchain)
Y′  = 0.2126 R′ + 0.7152 G′ + 0.0722 B′
Cb  = (B′ − Y′) / 1.8556      Cr = (R′ − Y′) / 1.5748
8-bit limited:  Y = 16 + 219·Y′    C = 128 + 224·C      (10-bit: ×4 → 64+876·Y′, 512+896·C)
full range (v1 toggle): Y = 255·Y′  C = 128 + 255·C (8-bit), tags flip to "pc"
code = floor(value + d), d ∈ [0,1): dither threshold; d = 0.5 when dithering is off
clamp codes to [1, 254] (8-bit) / [4, 1019] (10-bit): 0/255 are reserved timing codes
```

**Chroma 4:2:0, left-sited (MPEG-2 style):**
- Each chroma sample sits horizontally on even luma columns and vertically
  between two rows.
- The filter is a horizontal `[1, 2, 1]/4` at columns `2i−1, 2i, 2i+1`
  (clamped at the edges), averaged over rows `2j` and `2j+1`.
- The spec's "box filter" wording conflicts with left siting (see Risks). This
  filter is the siting-correct choice.
- 4:2:2 uses the same horizontal filter with no vertical averaging.

**Dither:**
- a 64×64 blue-noise (or 8×8 Bayer) threshold texture;
- it is offset per frame by `hash(frame_index)`;
- it is the same per plane position on every run, so the output is
  deterministic (EX-032);
- it is off for 10-bit, where rounding is enough.

**Tags** come from the same `OutputColor` value that picks the shader
constants (P4):
- `color_primaries = BT709`;
- `color_trc = BT709`;
- `colorspace = BT709`;
- `color_range = MPEG` (`JPEG` for full range);
- `chroma_sample_location = LEFT`.

These are set on the codec context **before** `avcodec_open2`, so the
encoders write them into the VUI or sequence header. The mov muxer writes
`colr` from the same values.

#### 5.3 Readback ring

- There are 3 slots. Each slot has `{ storage: Buffer(STORAGE|COPY_SRC),
  staging: Buffer(MAP_READ|COPY_DST), frame: u64, submission:
  SubmissionIndex, mapped: Arc<Mutex<Option<Result<(), BufferAsyncError>>>> }`.
- **Per frame:**
  1. record composite + convert + `copy_buffer_to_buffer`;
  2. `queue.submit`;
  3. `map_async` on the staging slice, or wgpu 30's
     `CommandEncoder::map_buffer_on_submit`.
- **Harvest the oldest slot:**
  1. `device.poll(wgpu::PollType::Wait { submission_index:
     Some(idx), timeout: Some(GPU_FRAME_TIMEOUT) })`;
  2. check the map result;
  3. `get_mapped_range()` (which returns `Result` in wgpu 30);
  4. `copy_from_slice` into a pooled `Vec<u8>`;
  5. `unmap`.
- `PollError::Timeout` becomes `RenderError::GpuTimeout { frame }`. The
  default timeout is 30 s per frame, which covers 8K stacks on slow iGPUs.
- With 3 slots the GPU composites `n` while the CPU copies `n−2`. Olive
  instead stalls on a synchronous `glReadPixels` per frame.
- A copy into the pool is made on purpose. Encoders may keep a reference to a
  frame, and FFmpeg allows that (P13). The staging buffer must be unmapped
  and reused right away.
- `VideoFrame` returns its buffer to the pool on drop, so the steady state
  allocates nothing.

**Memory budget.**
- The frame channel capacity is
  `clamp(512 MiB / frame_bytes, 2, 8)`, so it is bounded by **bytes**, not
  only by frames.
- Eight 8K 10-bit frames (≈ 100 MB each) plus the ring plus encoder lookahead
  would otherwise exceed the RAM of the machines the "Not enough memory"
  error in inventory §14 is about.

#### 5.4 Framing (EX-042) and draft (EX-051)

- `CanvasToOutput::fit/fill(canvas, out)` is a pure function. It returns the
  canvas→output affine transform that the compositor uses as its root
  projection, so there is still only one resample.
- With **Fit**, the content rectangle is snapped to **even** pixel bounds.
  Otherwise the 4:2:0 chroma at the bar edge mixes bar color into the image
  edge as a faint colored line.
- **Draft** changes only the inputs: preview scale, proxies and the fastest
  encoder speed. It never changes shaders.

### 6. Audio side (freecut-audio)

```rust
// freecut-audio/src/offline.rs
pub struct OfflineMixSpec {
    pub range: TimeRange,
    pub sample_rate: u32,          // the OUTPUT rate; sources are resampled once (see Risks)
    pub channels: OutputChannels,  // Stereo | Mono (mono = (L+R)/2, AU-032)
    pub solo: SoloPolicy,          // Ignore (default) | AsHeard(solo set)  — AU-012
    pub block_frames: u32,         // 1024 (AU-010)
}

pub struct OfflineMix { /* graph built from the snapshot's MixView (TL-008) */ }

impl Iterator for OfflineMix {
    type Item = Result<AudioBlock, AudioError>;  // blocks are contiguous; the last may be short
}
```

- **The same mixer graph as playback** (AU-010) runs offline: the per-clip
  chain, the track stage and the master limiter. Only the scheduling
  differs. The offline path never touches `cpal`.
- **The exact length** is `ExportClock::sample_count(sr)`. The final block is
  short, and the total equals the range duration exactly (EX-031).
- **Silence is real data.** An empty timeline or "Add silent track" produces
  zero blocks of the right length through the same path.
- **Clipping:**
  - with the limiter on, the limiter handles it;
  - with the limiter off, samples are clamped to [−1, 1] before encoding, as
    EX-031 and AU-010 require.
- **Opus** supports 48 kHz but not 44.1 kHz (`ffmpeg -h encoder=libopus` on
  this machine). The plan forces 48 kHz for Opus, and the dialog says so.

### 7. Encode side (freecut-media)

#### 7.1 EncodeSession: one owner, strict open order

```rust
pub struct EncodeSession { /* Output + VideoStreamWriter + AudioStreamWriter + stats */ }

impl EncodeSession {
    /// Opens every encoder, adds streams, opens `part` and writes the header.
    pub fn open(spec: &EncodeSpec, part: &Path) -> Result<Self, EncodeError>;
    pub fn video_frame(&mut self, f: &VideoFrame) -> Result<(), EncodeError>;
    pub fn audio_block(&mut self, b: &AudioBlock) -> Result<(), EncodeError>;
    /// Flush every encoder (video AND audio, P15), write trailer (faststart pass happens
    /// here), close the AVIOContext and CHECK its result, then return stats.
    pub fn finish(self) -> Result<MuxStats, EncodeError>;
    /// Cancel/failure path: no flush, no trailer; frees contexts quickly.
    pub fn abandon(self);
    pub fn frames_muxed(&self) -> u64;
}
```

**Open order (each step's failure is an `EncodeError`; nothing is
silently dropped, P5):**

1. **Check the path.**
   - Check that `part` is valid UTF-8. `ffmpeg-next` 9.0's `from_path`
     calls `to_str().unwrap()`, which would panic.
   - Prefix it with `file:` so FFmpeg never interprets a `scheme:`-looking
     name as a protocol.
2. **Allocate the output context with the explicit muxer name**
   (`format::output_as(path, "mp4")`).
   - Read `AVFMT_GLOBALHEADER` from the output format.
   - Note that `output_as` also opens the file, which is why the job claims
     `.part` first (§9).
3. **Set up each stream:**
   1. find the encoder **by name** (`EncoderImpl::ffmpeg_name()`);
   2. allocate its context;
   3. set size, `pix_fmt`, the time base, `framerate`, the color tags, GOP,
      `bit_rate`/`rc_max_rate`/`rc_buffer_size` and `thread_count`;
   4. set `AV_CODEC_FLAG_GLOBAL_HEADER` **before** opening (P8);
   5. call `avcodec_open2` with the private options from
      `options::encoder_setup`.
4. **Treat leftover options as errors.**
   - `ffmpeg-next`'s `open_as_with` drops the leftover option dictionary, so a
     misspelled key like `x265-param` is silently ignored.
   - `ffi::open_encoder_strict` calls `avcodec_open2` itself and returns
     `EncodeError::UnknownOption(key)` when the dictionary still has
     entries.
   - This happens in debug and test builds always. In release it is a logged
     warning plus a telemetry-free note in "Copy details".
5. **Add the streams.**
   - Copy parameters into each stream and set the stream time-base hint.
   - For HEVC in MP4/MOV set `codecpar->codec_tag = MKTAG('h','v','c','1')`
     for **every** HEVC encoder, hardware included (P9; `ffi.rs`, because
     ffmpeg-next doesn't expose `codec_tag`).
   - Add the cover stream (§7.6).
6. **Write the header** with `write_header_with(dict)`.
   - The dictionary holds the muxer options: `movflags=+faststart`,
     `rf64=auto` for WAV, `loop=N` for GIF, and so on.
   - A non-empty returned dictionary is an error, as in step 4.
   - Then write the cover packet, if there is one.

**`finish` sequence:**
1. `send_eof` + drain each encoder until `EOF` (video, then audio). Every
   packet goes through `av_interleaved_write_frame`.
2. `av_write_trailer`. FFmpeg flushes and reports buffered write errors here.
   For MP4/MOV with `faststart`, this is when the file is rewritten in place.
3. `ffi::close_checked`: `avio_closep(&pb)` and **check its return value**.
   `ffmpeg-next`'s `Output` destructor calls `avio_close` and ignores the
   result, so a write error at the final flush would otherwise be lost.

**Packets.** The packet loop is the standard FFmpeg one:
- `send_frame`, then `receive_packet` until `EAGAIN`;
- `av_packet_rescale_ts(enc_tb → st_tb)`, then set `stream_index`;
- `av_interleaved_write_frame`.

`frames_muxed` counts **video packets written**, clamped to `N`. Delayed
packets arrive during the flush, so progress keeps moving through the
x264/x265/SVT-AV1 lookahead drain instead of freezing at "100 %".

#### 7.2 Mux loop (timestamp-ordered pull)

```rust
/// Runs on the job's mux thread. Pulls from the stream that is behind in time.
pub fn mux_loop<M: Muxer>(
    m: &mut M,
    video: Option<&Receiver<VideoFrame>>, audio: Option<&Receiver<AudioBlock>>,
    expect: Expect,                     // N frames, S samples, rate, sample_rate
    cancel: &Cancel,
) -> Result<(), MuxLoopError> {
    let (mut next_v, mut next_a) = (0u64, 0u64);
    while next_v < expect.frames || next_a < expect.samples {
        let take_video = next_v < expect.frames
            && (next_a >= expect.samples
                || video_before_audio(next_v, expect.rate, next_a, expect.sample_rate));
        if take_video {
            let f = recv_or_cancel(video, cancel)?.ok_or(MuxLoopError::VideoEndedEarly(next_v))?;
            debug_assert_eq!(f.index, next_v); // invariant: contiguous, in order
            m.video_frame(&f)?;
            next_v += 1;
        } else {
            let b = recv_or_cancel(audio, cancel)?.ok_or(MuxLoopError::AudioEndedEarly(next_a))?;
            if b.first_sample != next_a { return Err(MuxLoopError::AudioGap(next_a)); }
            m.audio_block(&b)?;
            next_a += u64::from(b.frames);
        }
    }
    Ok(())
}

/// n/R < s/sr  ⇔  n·den·sr < s·num   (exact; FFmpeg's av_compare_ts does the same in C)
fn video_before_audio(n: u64, r: FrameRate, s: u64, sr: u32) -> bool {
    i128::from(n) * i128::from(r.den) * i128::from(sr) < i128::from(s) * i128::from(r.num)
}
```

- `Muxer` is a small trait implemented by `EncodeSession` and by test doubles:
  a recording muxer that hashes planes, and a failing muxer that returns
  `ENOSPC` at frame *k*.
- The pull order keeps the libavformat interleaving queue small. Without it,
  an audio thread far ahead would have its packets buffered in memory until
  `max_interleave_delta` (10 s) forced a flush.
- `recv_or_cancel` is a `crossbeam_channel::select!` over the data receiver
  and the cancel receiver.

#### 7.3 Video stream writer

- **Frame pool.** A small pool of `AVFrame`s is allocated with
  `av_frame_get_buffer` (aligned) for the encoder's `pix_fmt`.
  - Before each use the writer calls `av_frame_make_writable` (P13).
  - It copies rows from `VideoFrame` (our stride) into the frame
    (FFmpeg's `linesize`), sets `pts = n`, and sends it.
- **Zero-copy.** Wrapping our `Vec` with `av_buffer_create` and a
  return-to-pool free callback is a later optimization. It needs an
  ownership-heavy `unsafe` block and saves about 2 ms per 4K frame.
- **Threads:**
  - `threads = min(cores, 16)` for x264 and x265 (P17);
  - `lp` for SVT-AV1;
  - in **background** mode (v1), `cores − 2`, minimum 2 (§10).

#### 7.4 Audio stream writer

- **`SampleFifo`** is plain Rust, an interleaved `f32` ring. It re-chunks the
  1024-sample mixer blocks into the encoder's `frame_size`:
  - 1024 for AAC;
  - 1152 for MP3;
  - 960 for Opus at 20 ms;
  - any size for PCM and FLAC (`frame_size == 0` → 1024).
- **Sample format conversion is done in Rust:**
  - `f32` interleaved → `fltp` (AAC, MP3) by deinterleaving;
  - → `flt` (Opus) by copying;
  - → `s16`/`s32` (FLAC, PCM) with **TPDF dither** from a fixed-seed PRNG, so
    the output is deterministic (P12).
  - `swresample` is not needed, because the mixer already runs at the output
    rate.
- **The last frame** is sent **short**. All the encoders FreeCut uses
  advertise `AV_CODEC_CAP_SMALL_LAST_FRAME` (`ffmpeg -h encoder=aac|libopus|libmp3lame|flac`
  lists "small"). The stream therefore ends exactly at the range end instead
  of being padded to a full frame as MLT does. If an encoder lacks both
  `SMALL_LAST_FRAME` and `VARIABLE_FRAME_SIZE`, the frame is padded with
  silence and the overshoot is logged.

#### 7.5 Encoder selection, capabilities and hardware fallback

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EncoderImpl {
    X264, OpenH264, X265, SvtAv1, LibAom, Vp9, ProresKs, Gif,
    H264VideoToolbox, HevcVideoToolbox,
    H264Nvenc, HevcNvenc, Av1Nvenc,
    H264Amf, HevcAmf, Av1Amf,
    H264Qsv, HevcQsv, Av1Qsv,        // need hw frames (hwupload) — v1.x
    H264Vaapi, HevcVaapi,            // need hw frames (hwupload) — v1.x
}
impl EncoderImpl {
    pub fn ffmpeg_name(self) -> &'static str;          // "libx264", "hevc_videotoolbox", …
    pub fn is_hardware(self) -> bool;
    pub fn input_layout(self, depth: BitDepth) -> Option<PixelLayout>; // None = unsupported depth
}
```

**Capability probe** (`EncoderCaps::probe`, at startup, on a worker thread,
never on the UI thread):

1. **Software encoders** are probed in-process. `avcodec_find_encoder_by_name`
   plus an open at 64×64 with the FreeCut pixel layout is cheap and safe.
2. **Hardware encoders are probed in a child process**: the FreeCut binary
   with a hidden `--probe-encoders` subcommand that prints JSON lines. This
   follows Shotcut's `detectHardwareEncoders` (P18), but without depending on
   an external `ffmpeg` executable.
   - Each candidate opens at 256×256, encodes one frame, flushes, and must
     produce at least one packet.
   - There is a 10 s timeout per candidate. A hung or crashed driver marks
     that encoder unavailable and leaves the app untouched.
3. **The cache key** is FreeCut version + `av_version_info()` + the wgpu
   `AdapterInfo` (name, driver, driver_info). Any change re-probes in the
   background. Options that need a missing encoder are shown disabled
   (EX-021).

**Auto order:**
- the hardware encoder matching the **render adapter's vendor** (0x10DE →
  NVENC, 0x1002 → AMF, 0x8086 → QSV, Apple → VideoToolbox);
- then the software encoder of the same codec.

`On` uses the hardware list only. `Off` uses software only.

**Hardware specifics, from the references and the local `ffmpeg -h` output:**
- **VideoToolbox:**
  - `pix_fmt nv12`, or `p010le` for HEVC 10-bit; there is no 10-bit H.264;
  - `allow_sw=0` (the default), so a missing hardware unit fails instead of
    quietly using Apple's software path;
  - `realtime=0`;
  - quality mode `q:v` only on Apple silicon;
  - `constant_bit_rate` needs macOS 13.
- **NVENC:**
  - system-memory `nv12`/`p010le` input (no hwcontext needed);
  - `preset p4`, `tune hq`;
  - consumer GPUs limit concurrent sessions. An open error caused by that
    triggers fallback.
- **AMF:** `nv12`/`p010le`, `rc=vbr_peak|cbr|cqp`.
- **QSV and VAAPI** need `hw_frames_ctx` + `av_hwframe_transfer_data`. This is
  FFI in `freecut-media::encode::ffi` and is deferred until after the first
  hardware release (MLT uses a `hwupload` graph, `setup_hwupload_filter`).
- **Never** imitate libopenshot's forced Baseline/`zerolatency`/no-B-frames
  for hardware H.264 (P16). Use High profile and the encoder's own B-frame
  default.

**Fallback (EX-034):**
- An open failure on a hardware candidate goes straight to the next
  candidate. The session is abandoned, and the `.part` file is truncated and
  re-claimed.
- A mid-stream hardware failure, an `EncodeFailed { hw: true }` error,
  **restarts the job from frame 0** with the next candidate. The render is
  deterministic, so the frames are identical.
- The result view notes "Hardware encoder failed; finished with software
  encoder" (EX-039).

#### 7.6 Rate control → encoder options (pure, `options.rs`)

```rust
pub struct EncoderSetup {
    pub bit_rate: Option<u64>, pub max_rate: Option<u64>, pub min_rate: Option<u64>,
    pub buffer_bits: Option<u64>, pub gop: u32, pub threads: u32,
    pub private: Vec<(&'static str, String)>,  // validated strictly at open (§7.1 step 4)
}
pub fn encoder_setup(enc: EncoderImpl, rc: &ResolvedRateControl, fps: FrameRate,
                     speed: EncoderSpeed, pass: Pass) -> EncoderSetup;
```

**Terms used in the table:**
- **T** is the target bitrate (tier or custom).
- **GOP** `g = round(2 s × fps)`, or the custom interval.
- "VBR" is EX-027's single-pass VBR: `maxrate = 1.5 T`, `bufsize = 2 T`.
- "CBR" is VBV-constrained: `minrate = maxrate = T`, `bufsize = T`.
  - True HRD-CBR padding is not used for files. libx264 itself documents
    `nal-hrd` "cbr not allowed in .mp4".
- **Q** is the constant-quality value from EX-028.

| Encoder | VBR (tiers) | CBR | Constant quality | GOP / closed | Speed (faster / balanced / slower) |
|---|---|---|---|---|---|
| `libx264` | ctx `bit_rate`, `rc_max_rate`, `rc_buffer_size` | same with `min=max` | `crf=Q` (default 20) | `g`; x264 GOPs are closed by default | `preset` veryfast / medium / slow |
| `libx265` | `x265-params bitrate=…:vbv-maxrate=…:vbv-bufsize=…` (kbps, Shotcut's choice) | `vbv-maxrate=bitrate` | `crf=Q` (24) | `x265-params keyint=…:open-gop=0`; **x265 defaults to open GOP**, so set it explicitly | `preset` fast / medium / slow |
| `libsvtav1` | `bit_rate` (+ `svtav1-params rc=1`) | `svtav1-params rc=2:tbr=…:buf-sz=<ms>` (Shotcut) | `crf=Q` (32, 0–63) | `g`; set `irefresh-type=2` explicitly for closed GOP | `preset` 10 / 8 / 5 |
| `libvpx-vp9` | `bit_rate`, `maxrate`, `bufsize`, `deadline=good`, `row-mt=1` | `minrate=maxrate=bit_rate` | `crf=Q` (31) **and `bit_rate=0`** (else constrained quality) | `g` | `cpu-used` 4 / 2 / 1 |
| `prores_ks` | — profile sets the rate | — | — | intra | `profile` 0–3 (Proxy / LT / 422 / HQ); **no `vendor=apl0`** (P19) |
| `*_videotoolbox` | `bit_rate` (+ `rc_max_rate`) | `constant_bit_rate=1` (macOS ≥ 13) | `global_quality` (q:v), Apple silicon only | `g` | `prio_speed` 1 / auto / 0 |
| `*_nvenc` | `rc=vbr`, `bit_rate`, `maxrate`, `bufsize` | `rc=cbr` | `rc=vbr`, `cq=Q`, `bit_rate=0` | `g`, `strict_gop` with custom GOP | `preset` p2 / p4 / p6 |
| `*_amf` | `rc=vbr_peak`, `bit_rate`, `maxrate` | `rc=cbr` | `rc=cqp`, `qp_i/p/b=Q` | `g` | `quality` speed / balanced / quality |
| `gif` | n/a | n/a | n/a | n/a | n/a |

**Two-pass (software only, EX-028):**
- **x264:** private `stats=<tmp>/<job>.x264.log` (the encoder option is
  confirmed locally), plus `flags +pass1/+pass2`.
- **x265:** `x265-params pass=N:stats=<path>`, with `:` and `\` escaped as
  Kdenlive does (P11).
- **libvpx:** collect `stats_out` from pass 1 in memory and hand it back as
  `stats_in`. This is MLT's pattern for non-x264 codecs.
- Other encoders don't offer two-pass until a test proves they work.
- **Pass 1:**
  - video only, written to the `null` muxer (no file);
  - audio thread not started, the same as Kdenlive's `an=1`;
  - progress is 0–50 %, and pass 2 is 50–100 % (Kdenlive's mapping).

  Pass 2 must produce the identical frame sequence. Any nondeterminism, such
  as a slate appearing in only one pass, makes the encoder reject its stats.
  The job treats that as `EncodeError::InconsistentPasses`, not as a crash.

**Target file size:** `kbps = bytes × 8 / seconds / 1000 − audio_kbps`,
clamped to ≥ 300. This is pure math in `freecut-project` (EX-028). It forces
two-pass for software encoders.

#### 7.7 Containers and muxer options

| Container | Muxer name | Video | Audio | Options / notes |
|---|---|---|---|---|
| MP4 | `mp4` | H.264 (`avc1`), HEVC (`hvc1` forced), AV1 | AAC (Opus allowed, flagged "less compatible") | `movflags=+faststart`; set `creation_time` explicitly (UTC export time) |
| MOV | `mov` | H.264, HEVC (`hvc1`), ProRes | AAC, PCM s16/s24 | `movflags=+faststart` |
| MKV | `matroska` | all | AAC, Opus, FLAC, PCM | cover = attachment stream with `filename`/`mimetype` (as in MLT) |
| WebM | `webm` | VP9, AV1 | **Opus only** (Vorbis legacy) | AAC/MP3/PCM rejected at plan time (P10) |
| M4A | `ipod` | — | AAC | `movflags=+faststart`; FFmpeg maps `.m4a` to `ipod` |
| MP3 | `mp3` | — | MP3 | ID3v2 tags + Xing/LAME header (needs seekable output, which a file is) |
| WAV | `wav` | — | PCM s16/s24/f32 | **`rf64=auto`**; FFmpeg's default is `never` (local `ffmpeg -h muxer=wav`) |
| FLAC | `flac` | — | FLAC 16/24 | 24-bit = `s32` + `bits_per_raw_sample=24` |
| GIF | `gif` | GIF (`pal8`) | — | `loop` (see §7.8) |
| (pass 1) | `null` | any | — | no file |

**Rules:**
- `freecut-project::export::compat::allowed(codec, container)` is the single
  table behind the UI.
- An integration test checks every `true` cell with
  `avformat_query_codec(oformat, codec_id, FF_COMPLIANCE_NORMAL)` and an
  actual header write, so the table cannot drift from FFmpeg (P10).
- **Metadata whitelist (EX-040):**
  - allowed keys are `title`, `artist`, `comment`, `copyright`, `date`, the
    muxer-generated `encoder` tag and `creation_time`;
  - no user, host or path data, ever;
  - a test asserts the key set read back from the file.
  - x264's settings SEI in the bitstream is technical and allowed.
- **Cover art** (EX-046):
  - a JPEG packet with `AV_DISPOSITION_ATTACHED_PIC` for MP4/MOV/M4A/MP3/FLAC;
  - an attachment for MKV;
  - hidden for WebM/GIF;
  - support is checked with `avformat_query_codec` before adding the stream,
    as MLT's `add_attached_pic_stream` does.
- **Embedded subtitles** (EX-045) are muxed in the **same** session: a
  `mov_text` stream for MP4/MOV, SRT/ASS for MKV, WebVTT for WebM. There is
  never a post-hoc remux that changes the container (P20). Sidecars are
  written after the video is finalized.
- **The 4 GB limit:**
  - MP4/MOV switch to 64-bit chunk offsets automatically;
  - WAV uses `rf64=auto`;
  - the job tracks `avio_tell` and fails early with `FileTooLarge` on
    FAT/FAT32 (4 GiB − 1) and FAT16 (2 GiB) volumes (inventory §14).

#### 7.8 GIF (EX-043)

Frames are rendered in `Rgba8` (sRGB-encoded working values, no matrix), at
the GIF size and GIF rate, through the same `ExportRenderer`.

**Global palette (default): two render passes, bounded memory.**
1. **Pass 1:** `buffer → palettegen=max_colors=C:stats_mode=full:reserve_transparent=0
   → buffersink`. This produces one palette image at EOF and stores no
   frames.
2. **Pass 2:** `buffer + buffer(palette) → paletteuse=dither=<d>:diff_mode=rectangle
   → buffersink` → `gif` encoder (`pal8`) → `gif` muxer.

**Single-pass alternative.** The common one-pass graph,
`split → palettegen → paletteuse`, makes `paletteuse` **queue every frame**
until the palette arrives at EOF. For a 15 s 480p GIF at 15 fps that is
already about 370 MB. Two passes cost one extra render and keep memory
flat.

**Per-frame palettes:** a single pass with `palettegen=stats_mode=single` +
`paletteuse=new=1`.

**Dither mapping:**
- none → `dither=none`;
- ordered → `bayer` (`bayer_scale=2`);
- error diffusion → `sierra2_4a` (FFmpeg's default) or `floyd_steinberg`.

**Do not** let FFmpeg choose the GIF pixel format. Its first supported format
is `rgb8`, the fixed palette the MLT preset ends up with.

**Timing:**
- the time base is 1/100;
- `pts_n = round_half_up(n × 100 × den / num)` computed in `i128`;
- delays are the pts differences, so 15 fps gives 7, 6, 7, …, and the total
  is exact;
- the last frame's duration is `round(total) − pts_{N−1}`;
- rates are ≤ 25 fps, so every delay is ≥ 4 cs. Browsers clamp delays below
  2 cs.

**Loop mapping** (the Netscape loop count counts *extra* repeats):

| User choice | Muxer `loop` |
|---|---|
| forever | `0` |
| play once | `−1` |
| N times | `N − 1` |

`gif_pts_cs` and `gif_loop_value` are pure and tested.

**Fallback.** If libavfilter isn't available in a build, a pure-Rust path
works instead: `imagequant` (GPL-3.0-or-later, compatible) builds the
histogram over pass 1 and remaps in pass 2, and the `gif` crate writes the
file. Both give the same timing math.

#### 7.9 Audio-only (EX-044, AU-032)

- **No GPU device is created.** The plan has `video: None`. The session opens
  only the audio stream, and the mux loop pulls only audio.
- **Formats:**
  - MP3: `libmp3lame` CBR, `b=` 128–320k;
  - WAV: `pcm_s16le`/`pcm_s24le`/`pcm_f32le`, `rf64=auto`;
  - FLAC: `flac`, `compression_level=5`;
  - AAC: native `aac` in the `ipod` muxer.
- **Mono** is mixed in `freecut-audio`: `(L + R) / 2`.
- **Progress** is samples muxed out of the total. Cover art becomes album art
  (APIC, PICTURE or `covr`, depending on the format).
- **Stems (AU-033, later)** are N parallel audio-only sessions fed by
  per-track taps of the same offline graph, taken before the master limiter.

#### 7.10 Errors (thiserror, library crates)

```rust
// freecut-media/src/encode/error.rs
#[derive(Debug, thiserror::Error)]
pub enum EncodeError {
    #[error("no encoder available: {0}")]
    EncoderUnavailable(&'static str),
    #[error("encoder {name} failed to open")]
    OpenFailed { name: &'static str, hw: bool, #[source] source: ffmpeg_next::Error },
    #[error("encoder {name} failed at frame {frame}")]
    EncodeFailed { name: &'static str, hw: bool, frame: u64, #[source] source: ffmpeg_next::Error },
    #[error("unknown encoder/muxer option `{0}`")]
    UnknownOption(String),
    #[error("not enough space on the destination volume")]
    DiskFull,                                   // AVERROR(ENOSPC) anywhere, incl. avio_closep
    #[error("file too large for the destination file system")]
    FileTooLarge,                               // AVERROR(EFBIG) or our FAT guard
    #[error("two-pass statistics do not match the second pass")]
    InconsistentPasses,
    #[error("output path is not valid UTF-8")]
    NonUtf8Path,
    #[error("muxer error while {stage}")]
    Mux { stage: &'static str, #[source] source: ffmpeg_next::Error },
}

// freecut-render: RenderError { DeviceLost, GpuTimeout{frame}, OutOfMemory, Decode{clip, at},
//                               SizeExceedsLimit{w,h,max}, Cancelled, ... }
// freecut-audio:  AudioError  { Decode{clip, at}, Cancelled, ... }
```

**Error handling in `freecut-app`:**
- The app wraps these with `anyhow::Context` and maps them to
  **stable error codes** plus plain-language messages (inventory §14), for
  example `E-DISK-FULL` "Not enough space on <volume>". "Copy details"
  includes the source chain and the encoder log.
- **First error wins.** When the mux thread fails, the producers then see
  "receiver disconnected" or `Cancelled`.
- The orchestrator ranks errors: `Cancelled` and `Disconnected` are the lowest
  priority, so the root cause is what gets reported, not a secondary
  "channel closed".
- A panic in a worker is caught by `JoinHandle::join` and becomes
  `JobError::Internal`. It never takes down the UI.

### 8. Progress, ETA, cancel and the stall watchdog (freecut-app)

**`ProgressCell`** is shared and lock-free:
- `phase: AtomicU8`, `pass: AtomicU8`, `frames_muxed: AtomicU64`,
  `samples_muxed: AtomicU64`;
- `thumb: Mutex<Option<Thumbnail>>`, updated at most once per second.

The UI reads it on repaint (`request_repaint_after(250 ms)`), with no channel
backlog. Phases are Preparing, Analyzing, Rendering (pass k/m), Finishing and
Done/Failed/Cancelled.

**Progress** (EX-022): `p = 0.99 × frames_muxed / N`, or samples for audio-only.
Finishing then fills the last 1 %. Finishing covers:
- the encoder drain tail;
- the trailer and the fast-start rewrite;
- fsync;
- the rename;
- sidecars and the verification probe.

**ETA:**
- a ring of `(Instant, frames_muxed)` samples gives the average rate over the
  last 10 s;
- it is shown after 3 s;
- `speed = (frames/s) / R`, displayed as "1.8× real time";
- for two-pass, the remaining-pass estimate is included.

**Cancel:**
- `Cancel::cancel()` sets an `AtomicBool` and drops a sender, so every
  `select!` wakes at once;
- the render loop checks it once per frame and audio once per block;
- the mux thread calls `session.abandon()`, which skips the flush and the
  trailer (unlike Olive);
- threads are joined, and the `.part` file is deleted;
- the target is ≤ 1 s;
- one 8K frame with heavy effects may exceed that, so the UI shows
  "Stopping…" immediately.

**Stall watchdog** (inventory §14):
- it fires when `frames_muxed` hasn't moved for `max(60 s, 50 × mean frame
  time)`;
- the job shows "Export seems stuck" with **Keep waiting** and **Retry with
  hardware acceleration off**;
- it never auto-cancels.

### 9. Files: claim, finalize, overwrite and crash recovery

**Claiming the `.part` file:**
1. Write `<app-data>/jobs/<job-id>.json` with `{part, final, started}` and
   hold an exclusive `fs4` lock on it while the job runs.
2. Create `<final>.part` with `OpenOptions::create_new(true)`. This is an
   atomic claim. Another FreeCut instance writing the same name gets
   `AlreadyExists` and uses `<final>.<id8>.part`.
   - FFmpeg's `avio_open` uses `O_TRUNC` and would silently clobber, so the
     claim must happen before FFmpeg opens the file.
3. Hand the path to `EncodeSession::open`. FFmpeg reopens and truncates the
   claimed file.

**Finalize (after `finish()` succeeded):**

```rust
fn finalize(part: &Path, target: &FinalTarget) -> Result<PathBuf, FinalizeError> {
    std::fs::File::open(part)?.sync_all()?;          // std uses F_FULLFSYNC on Apple targets
    verify_output(part, &target.expect)?;            // demux: streams, frame count, duration
    let dest = match target.on_conflict {
        Conflict::Replace => target.path.clone(),    // atomic replace on the same volume
        Conflict::KeepBoth => next_free_name(&target.path)?, // re-checked now, not at dialog time
    };
    std::fs::rename(part, &dest).map_err(|e| FinalizeError::Rename { kept_as: part.into(), source: e })?;
    sync_parent_dir(&dest);                          // best effort on Unix
    Ok(dest)
}
```

**Rename and verification rules:**
- The rename is always on the same volume, because `.part` sits next to the
  target. It is an atomic replace, so the user's old file survives until the
  new one is complete (P7).
- If the rename fails, for example because Windows has the old file open in a
  player, the job keeps the finished file and reports it. Olive does the same
  ("saved as … instead").
- `verify_output` is the success criterion (P6). Instead of "exit at ≥ 99 %",
  it reopens the file with `ffmpeg-next` and checks:
  - the expected streams and codecs are present;
  - the video frame count is N, from the packet count or `nb_frames`;
  - the duration is within one frame;
  - the color tags are present;
  - for MP4/MOV, `moov` comes before `mdat`.
  It reads only headers and packet headers and never decodes, so it costs
  about 100 ms.
- **Fast-start space.** FFmpeg's fast-start pass rewrites the file **in
  place**, shifting `mdat` by the size of `moov`. It needs extra space of
  about the `moov` size, not a second copy. This corrects EX-022 (see Risks),
  but it does re-read the whole file, so it is part of "Finishing".

**Crash recovery:**
- On launch, the app scans `<app-data>/jobs/`.
- Entries whose lock can be acquired belong to dead processes. The app offers
  to delete their recorded `.part` files (EX-022).
- It never deletes anything it didn't record, and never globs.

**Target is a project source** (EX-023):
- Compare the canonicalized destination folder and name, case-insensitively
  on case-insensitive volumes, against every media path in the snapshot.
- If the target exists, also compare file identity (dev+inode, or the Windows
  file index).
- This replaces Shotcut's substring search of the XML.

### 10. Background export and the queue (EX-038)

**Snapshot:**
- At queue time the UI thread clones the `Arc` document. This is
  O(changed nodes) thanks to TL's structural sharing.
- A worker serializes the snapshot to
  `<app-data>/jobs/<id>/snapshot.freecut` with `freecut-project` serde. This
  is Shotcut's and Kdenlive's "XML at enqueue" idea.
- Retry after a crash or restart ("Interrupted") reloads that file. A running
  job uses the in-memory `Arc`.

**`JobQueue`:**
- `JobState { Queued, Running(progress), Done(report), Failed(error),
  Cancelled, Interrupted }`;
- one job runs at a time by default, or two with the preference; inventory
  §12.2 shows the reference editor runs one;
- preflight runs at queue time and again at job start (EX-036);
- the queue file is JSON in app data, written on every state change, off the
  UI thread.

**Coexisting with preview:**
- Export threads run at **below-normal priority** (`thread-priority` crate):
  utility QoS on macOS, `nice` on Linux, `THREAD_PRIORITY_BELOW_NORMAL` on
  Windows.
- The mux thread sets its priority *before* opening encoders. On Linux and
  macOS the encoder's own worker threads inherit it. On Windows they don't,
  so background jobs also open encoders with `threads = cores − 2`.
- While the player is playing and reports dropped frames, the export render
  loop yields up to 100 ms between frames (`ExportGate`, an atomic set by the
  player). This is cooperative. Export never *drops* frames. It only goes
  slower.

**Platform:**
- Keep-awake is held while any job runs. The `keepawake` crate uses
  `IOPMAssertionCreateWithName` on macOS, `SetThreadExecutionState` on
  Windows, and a systemd-logind inhibitor lock plus
  `org.freedesktop.ScreenSaver` on Linux.
  - Windows execution state is **per thread**, so the guard is created and
    dropped on the orchestrator thread, which lives for the whole job.
  - Use `.idle(true)` only. `.sleep(true)` maps to Windows Away Mode
    (`ES_AWAYMODE_REQUIRED`), which export doesn't need.
- Quitting with jobs running or queued prompts Wait / Cancel-and-quit.

### 11. How the design meets the project conventions

**Rational time.**
- All frame and sample positions are integers from `ExportClock` (`i128`
  intermediates, checked conversions).
- Floats appear only:
  - in shader uniforms;
  - in the bitrate formula, stored as integer kbps;
  - in ETA and speed display;
  - inside DSP.

**Nothing heavy on the UI thread.**
- Snapshot cloning is O(1)-ish on the UI thread.
- Serialization, probing, free-space checks, preflight I/O, rendering,
  decoding, mixing, encoding, renaming, reveal/open and leftover cleanup all
  run on workers.

**Undo.** Export has no undo entry. `SetCover` stays a TL document op
(EX-046).

**Errors.** `thiserror` enums in media, render, audio, timeline and project;
`anyhow` only in `freecut-app::jobs` and the CLI.

**`unsafe`.**
- Only in `freecut-media::encode::ffi`:
  - `codec_tag`;
  - strict `avcodec_open2`;
  - `avio_closep` with its result checked;
  - `avformat_query_codec`;
  - hwframes;
  - optionally `av_buffer_create` for zero-copy.
- Each call is a small function with a `// SAFETY:` comment stating the
  pointer and lifetime invariants.
- `freecut-render` needs none: wgpu is safe.

**No `unwrap`.**
- Path UTF-8 is validated before calling `ffmpeg-next`, which would panic on
  it.
- Channel sends and receives are `Result`s that are handled.
- Poisoned mutexes map to `JobError::Internal`.

**Testability.**
- The time math, plan resolution, option mapping, GIF timing, names,
  estimates and the compatibility table are pure functions.
- The I/O seams are the `Muxer` trait, the encoder factory and the GPU
  context, so each can be replaced by a fake in tests.

---

## Rust ecosystem

Versions were verified with `cargo info` on 2026-10-11.

### Use

| Crate | Version | License | Why |
|---|---|---|---|
| `ffmpeg-next` / `ffmpeg-sys-next` | 9.0.0 / 9.0.1 | WTFPL | Already chosen. Output contexts, encoders, packets, filter graphs (palettegen/paletteuse). Gaps (`codec_tag`, strict option check, checked `avio_closep`, `avformat_query_codec`, hwframes) are filled in `ffi.rs`. |
| `wgpu` | 30.0.1 | MIT OR Apache-2.0 | Already chosen. Compute conversion, `map_async`/`map_buffer_on_submit`, `PollType::Wait { submission_index, timeout }` for timed readback. |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | Bounded channels with `select!`, so producers blocked on a full channel wake on cancel. std `sync_channel` has no select. |
| `keepawake` | 0.6.1 | MIT | Keep-awake on macOS (IOKit), Windows (per-thread execution state) and Linux (logind/ScreenSaver over D-Bus) behind one guard (`Builder::default().idle(true).reason("Exporting").create()`, held on the orchestrator thread). |
| `thread-priority` | 3.1.1 | MIT | Below-normal priority for background export threads. |
| `fs4` | 1.1.0 | MIT OR Apache-2.0 | Exclusive lock on job registry files (crash detection). Its free-space helper is an alternative to a platform call; confirm the API name at implementation time. |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | Safe casts for uniform structs and readback words. |
| `thiserror` / `anyhow` / `serde` | 2 / 1 / 1 | MIT OR Apache-2.0 | Already chosen (convention). |
| `proptest` (dev) | 1.12.0 | MIT OR Apache-2.0 | Property tests for the clock, the FIFO and cancel points. |
| `insta` (dev) | 1.49.0 | Apache-2.0 | Snapshot tests of `encoder_setup()` output per encoder × mode. |
| `tempfile` (dev) | 3.27.0 | MIT OR Apache-2.0 | Scratch export folders in tests. |

### Optional / fallback

| Crate | Version | License | Use |
|---|---|---|---|
| `imagequant` | 4.4.1 | **GPL-3.0-or-later** (compatible) | Pure-Rust GIF palette (histogram over pass 1, remap in pass 2) if libavfilter is unavailable. |
| `gif` | 0.14.2 | MIT OR Apache-2.0 | GIF writer for that fallback (exact per-frame delays). |
| `rubato` | 5.0.1 | MIT OR Apache-2.0 | Owned by `freecut-audio` for source resampling; listed because the export rate may differ from the project rate. |
| `ffmpeg-the-third` | 6.0.0+ffmpeg-9.0 | WTFPL | Maintained fork of `ffmpeg-next`; keep in mind if `ffmpeg-next` lags behind FFmpeg 9. |

### Avoid

- **`gifski` 1.34.0** (AGPL-3.0-or-later). It adds network-use obligations
  and brings its own pipeline. Use FFmpeg or imagequant instead.
- **Spawning the `ffmpeg` CLI to encode** (the Kdenlive and Shotcut remux
  style). It adds an executable dependency, parses stderr for progress and
  hides errors. The only child process we spawn is *our own binary*, for the
  hardware probe.
- **FDK-AAC** is non-free and GPL-incompatible. Use the native `aac`
  encoder. On macOS, `aac_at` (AudioToolbox, a system framework) is a
  quality option worth evaluating.
- **`openh264` (0.9.8, BSD-2) and `rav1e` (0.8.1, BSD-2) as separate Rust
  encoders.** Keep a single encode path through FFmpeg. OpenH264 is reached as
  FFmpeg's `libopenh264` when the build has it (EX-029 fallback), and SVT-AV1
  is faster than rav1e.
- **`sysinfo` 0.39** for disk space alone. It is heavy, with MSRV 1.95.
- **Rust-side `swresample` use** in the export path. The mixer already renders
  at the output rate, and format conversion is ~20 lines of safe Rust
  (§7.4).

### Toolchain notes

- The local FFmpeg 8.1.1 (Homebrew) is configured with
  `--enable-gpl --enable-version3`, which makes it GPLv3. That is fine for
  FreeCut.
- It has libx264, libx265, libsvtav1, libvpx-vp9, libopus, libmp3lame,
  VideoToolbox (H.264/HEVC/ProRes), `prores_ks`, `aac` and `aac_at`.
- It has **no libaom encoder** (only dav1d for decoding). The AV1 fallback
  must therefore be capability-driven.

---

## Test strategy

### Pure unit tests (no FFmpeg, no GPU)

**`ExportClock`** (freecut-timeline):
- `N` for 1-frame ranges and for ranges shorter than one frame (rounds up to
  1);
- 30000/1001 over 1 h and 24 h: the sum of frame durations equals
  `t_N − t_0`, and there are no duplicate ticks;
- 25→30 and 30→25 cadence patterns;
- 60→24;
- `i128` overflow guard at extreme durations (`Overflow`, never a panic);
- `sample_count` with 44.1k/48k on NTSC ranges: adjacent ranges tile
  exactly.
- **Property:** for random ranges and rates,
  `frame_time(n) ≤ frame_time(n+1)` and `frame_time(N−1) < end`.

**`video_before_audio`**: property test against a `num_rational` / `BigInt`
oracle (dev-dependency) or a high-precision reference.

**Mux loop** with fake producers:
- the order of pulls is non-decreasing in presentation time;
- a video-only plan, an audio-only plan, and a producer that ends early
  (`VideoEndedEarly`);
- an audio gap (`AudioGap`);
- cancel at a random step (proptest): returns within one iteration and never
  calls `finish`.

**`SampleFifo`:** any sequence of block sizes in → encoder frames out, with
total samples conserved, every frame `frame_size` long except the last, and
no frame empty.

**Sample conversion:**
- `fltp` deinterleave round-trips;
- TPDF dither is deterministic for a seed, and its mean error is ≈ 0;
- clipping to [−1, 1] when the limiter is off.

**`encoder_setup`:** `insta` snapshots for every encoder × {VBR, CBR, CQ,
target size} × speed × pass. Also check:
- x265 always contains `open-gop=0`;
- VP9 CQ always has `bit_rate=0`;
- ProRes never sets `vendor`;
- x264/x265 threads are ≤ 16;
- Windows paths in `x265-params stats` are escaped.

**`freecut-project::export`** (as listed in the EX model notes):
- `output_dimensions` (landscape, portrait, square, 1081×1921);
- the `recommended_kbps` table;
- target-size math;
- `estimate_size`;
- `sanitize_file_stem` (reserved Windows names, trailing dots, all-invalid →
  "Untitled");
- `resolve_conflict` with a fake `exists` callback (case-insensitive);
- `allowed(codec, container)`, checked exhaustively against a literal
  table.

**GIF timing:**
- for every offered rate, the delay sum equals `round(total × 100)`;
- 15 fps gives the pattern 7, 6, 7;
- no delay < 2;
- the loop mapping forever/once/N.

**Reframe:**
- Fit and Fill transforms for 16:9 → 9:16, 9:16 → 1:1 and 4:5;
- the content rectangle is even-aligned;
- an identity canvas gives the identity transform.

**FrameGeometry:**
- strides and offsets for every layout at widths 854, 1080, 1920 and
  7680/8192;
- every tile write stays inside its plane;
- `P010` vs `Yuv420p10` bit positions.

### GPU tests (freecut-render; run on a software adapter in CI)

These use lavapipe/llvmpipe on Linux and WARP on Windows, with Metal locally.
Tolerance is ±1 code value on the dither-off path, because GPU float
differences across vendors make bit-exact cross-vendor tests unrealistic.
Tests with the same adapter must be bit-exact run to run.

- **Matrix golden.**
  - Render solid patches (black, white, 75 % bars, pure R/G/B, mid-grey) and
    a horizontal RGB ramp.
  - Convert to each layout.
  - Compare with a **CPU reference implementation in the test**, using f64
    BT.709 formulas.
  - Limited-range white must be exactly Y=235 / Cb=Cr=128, and black exactly
    16/128/128.
  - 10-bit gives 940/64/512.
- **Chroma siting.** Single-pixel vertical lines at even and odd columns
  check that the `[1, 2, 1]` horizontal weights land on the expected chroma
  samples. A horizontal line checks the vertical averaging.
- **Dither:**
  - the same frame index gives identical bytes;
  - different indices give different patterns;
  - the mean over a flat 50 % grey field equals the undithered value
    ±0.05;
  - a 0→1 gradient shows no runs longer than X columns (banding check).
- **Readback ring:**
  - frames come out in order with no loss for ring sizes 1, 2 and 3;
  - odd widths (854);
  - banded dispatch with a forced low `max_storage_buffer_binding_size`;
  - timeout handling with a fake poll.
- **Preview = export (EX-040).** For a fixed fixture project, the compositor
  output fed to a `RecordingMuxer` (hashes of the YUV planes) must equal the
  hashes from rendering the same frame times through the preview code path at
  output size. There are no extra frames and no overlay.

### FFmpeg integration tests (freecut-media)

Synthetic sources are generated in the test: Rust-generated frames with the
frame number encoded as a pixel barcode, plus a tone with sample-index
clicks.

**Per encoder actually present** (capability-gated, so CI and the dev Mac
differ gracefully), × container cell:
- write a 2 s file, then demux it with `ffmpeg-next` and assert:
  - codec, `pix_fmt` and profile (High for x264, never 4:4:4);
  - color tags 1/1/1/tv and chroma location left;
  - the `colr` atom (MP4/MOV);
  - `hvc1` for HEVC;
  - the packet count equals N;
  - the stream time base allows exact NTSC timestamps (constant pts
    deltas);
  - for MP4/MOV, `moov` comes before `mdat` (parse the top-level atom
    list);
  - the metadata key set ⊆ the whitelist.
- **Decode back and read the barcode:**
  - frame `k` of the file shows source frame `k`, so no frames are dropped,
    duplicated or reordered;
  - for 25→30 exports, the expected repeat pattern holds.

**A/V sync:**
- A timeline with a white flash frame and a click, both at t = 10 s, exported
  at 23.976, 25, 29.97 and 59.94 with 44.1k and 48k audio.
- Decode both streams, find the flash frame index and the click sample,
  correct the click by the edit list or `skip_samples`, and assert alignment
  ≤ 1 ms.
- A nightly variant runs at 1 h.

**Audio exactness:**
- The decoded audio sample count, after priming and edit-list trimming,
  equals `sample_count(sr)` within the codec's documented tolerance.
  For PCM and FLAC it is **exact**.
- FLAC STREAMINFO MD5 equals the MD5 of the PCM produced by `OfflineMix`.
  That is a bit-exact check of the offline mix through the encoder.

**Containers:**
- WAV with `rf64=auto` writes an RF64 header when `data` > 4 GiB. Test this
  with a sparse fake `AVIOContext` that only counts bytes, or by unit-testing
  the option wiring.
- The GIF palette has ≤ the requested color count.
- GIF delays read back match `gif_pts_cs`.
- The loop extension value matches.

**`compat` table vs FFmpeg:** for every allowed cell, `avformat_query_codec`
== 1, and a real `write_header` succeeds (P10).

**Strict options:** `encoder_setup` output for every encoder opens with no
leftover options (`UnknownOption` never fires).

**Failure injection** (a fake muxer or a small RAM-disk volume in nightly
runs):
- `ENOSPC` at frame k → `DiskFull`, the `.part` file is deleted, and no
  final file exists;
- `avio_closep` failure → the error surfaces;
- encoder open failure on a hardware candidate → the next candidate is used,
  and the result note is recorded;
- mid-stream `EncodeFailed { hw: true }` → a full restart in software, and
  the output is identical to a software-only export;
- GPU device-lost via a fake context → `RenderError::DeviceLost` and Retry.

**Two-pass:**
- the pass-1 and pass-2 frame hashes are identical;
- x264 and x265 two-pass reach the target bitrate within ±10 % on a 10 s
  fixture;
- a deliberately nondeterministic fake renderer → `InconsistentPasses`.

**Reproducibility mode for golden files:**
- `fflags=+bitexact` and `flags=+bitexact`;
- fixed `creation_time`;
- `threads=1` for x264.

The same inputs must then produce byte-identical files on one machine. The
hashes are stored per FFmpeg major version.

### Job / app tests (freecut-app)

- **Cancel:** at random points (property test over phases), every thread
  joins within 1 s (CI timeout 5 s), no `.part` file is left, and the registry
  entry is removed.
- **Finalize:**
  - Replace keeps the old file until the rename;
  - Keep both re-checks at finish time;
  - a rename failure keeps the finished file and reports it;
  - the crash leftover scan only touches recorded paths.
- **Queue:**
  - persistence round-trip;
  - Interrupted after a simulated restart;
  - Retry from the serialized snapshot produces the same frames as the
    original job.
- **Progress:**
  - monotonic;
  - ≤ 99 % until finalize;
  - the ETA window math;
  - the stall watchdog fires on a frozen fake muxer and never cancels.

### Fixtures needed

Generated at test time, with no binary blobs in the repo:
- a barcode video source at 30000/1001 and at 25;
- a VFR clip;
- a rotated phone-style clip;
- 10-bit HEVC;
- stereo and mono tone/click WAVs at 44.1k and 48k;
- a small FreeCut project with a transition, text, an effect layer and an
  in/out range.

Large-file tests (> 4 GB, 8K, 1 h) are **nightly only**.

---

## Risks & open questions

1. **Fast-start disk space (spec correction).** EX-022 says fast start
   "briefly needs free space about equal to the file size". FFmpeg's
   implementation shifts the data in place, so the extra space is about the
   `moov` size. It does re-read and rewrite the whole file, which is slow on
   USB disks. Proposal: update EX-022 and EX-037 to say so. Preflight only
   needs a margin above the estimate.
2. **Chroma filter wording.** EX-032 says "left-sited … box filter". A box
   filter is center-sited. Proposal: the `[1, 2, 1]` horizontal filter in
   §5.2. Confirm with CL.
3. **BT.709 transfer tag vs Apple players.** Tagging 1-1-1 as CL-002 requires
   makes QuickTime/macOS apply BT.709-camera-OETF handling. Files can look
   slightly lighter there than in browsers or VLC. This is a known industry
   issue, not a FreeCut bug. Decide whether to document it only, or to offer
   a "match macOS QuickTime" tag option later.
4. **Separate wgpu `Device` for export.** This costs a second pipeline compile
   and extra VRAM while preview is open. Alternative: share the preview device
   and serialize submissions. Recommendation: separate device. Measure on
   8 GB iGPU Macs.
5. **In-process vs out-of-process worker.** Shotcut and Kdenlive isolate
   crashes in `melt`. FreeCut starts in-process (MVP). The job API takes a
   plan and progress sink so that `freecut export --worker` (EX-052) can run
   it in a child process later, if driver or encoder crashes show up in the
   field.
6. **Software vs hardware decode for export.** Hardware decode is faster but
   can differ by ±1 code value from software decoding (chroma upsampling),
   and that difference can break "preview = export" if preview uses hardware
   decode (EX-018). Recommendation: software decode for export by default,
   with an opt-in. Needs a decision with PM/EX-018.
7. **Mixer at output sample rate.** Rendering the offline mix directly at
   44.1k when the project is 48k means each source is resampled once.
   Resampling the master afterwards would mean resampling twice. This needs
   AU's agreement, since AU-010 talks about the "project rate".
8. **Audio length vs video length.** Audio is exactly the range. The last
   video frame can extend up to one frame past it (`ceil`). EX-026 accepts
   this. Some platforms trim to the shortest stream. Confirm.
9. **Thread priority on Windows.** Encoder-created threads don't inherit
   priority there. The mitigation is fewer encoder threads plus
   `ExportGate`. Measure UI smoothness during a 4K x265 background job.
10. **Hardware encoder fidelity.** VideoToolbox quality mode exists only on
    Apple silicon. NVENC session limits vary by driver. AMF and VAAPI rate
    control varies by driver (libopenshot's VAAPI "maxrate" workaround).
    Bitrate tests for hardware encoders run only on matching CI runners.
11. **8K and large frames.** Banded conversion is needed when
    `max_storage_buffer_binding_size` < plane size. Encoder 8K support (x264
    level 6.x, NVENC/VideoToolbox limits) is found by opening the encoder,
    which triggers fallback.
12. **FAT detection.** Identifying FAT/FAT32/FAT16 volumes needs per-OS calls:
    `statfs` `f_fstypename`/`f_type`, or `GetVolumeInformationW`. If
    detection fails, the job relies on `EFBIG`/`ENOSPC` mapping, which on
    Windows may arrive as a generic I/O error. Message: "file too large for
    this drive's file system (FAT32)".
13. **Two-pass for SVT-AV1/AMF/NVENC.** It is not offered until tests prove
    the FFmpeg wrappers support it in the linked FFmpeg (EX-028 says "software
    encoders only").
14. **QSV/VAAPI hwframes.** These need `unsafe` hwcontext FFI. They are
    scheduled after the VideoToolbox/NVENC/AMF system-memory path ships.
15. **Packaging and patents.** Distribution builds may lack x264/x265/AAC
    (EX-029, clean-room notes). Every choice above is capability-driven,
    including the AV1 fallback, given the local FFmpeg has no libaom. Which
    FFmpeg build ships on each OS is open.
16. **ProRes `vendor` field.** MLT's preset sets `vendor=apl0` for
    compatibility with picky tools. FreeCut leaves FFmpeg's default, in line
    with the clean-room "no claim of Apple certification" rule. Revisit only
    with evidence and a legal opinion.
17. **HDR (EX-033, later).** The converter takes `OutputColor`, so
    PQ/HLG/BT.2020 matrices and x265/SVT-AV1 mastering metadata (Shotcut's
    `master-display`/`max-cll` injection) can be added without restructuring.
    It is blocked on CL's working-space change.
18. **Shared GPU float determinism.** Bit-exact goldens hold per adapter
    only. CI uses a software adapter with a ±1 tolerance across vendors.
19. **Process note.** The relayed request ("open it i allow u for one time")
    was ambiguous. Nothing under `/Applications/CapCut.app` was opened, and
    this design did not need it. The design was written from the FreeCut
    specs, the inventory and the open-source references only.

---

## References

FreeCut specs (this repo):
- `docs/features/preview-and-export.md` — EX-021 … EX-052
- `docs/features/audio.md` — AU-010, AU-011, AU-012, AU-032, AU-033
- `docs/features/effects-transitions-animations.md` — FX-001
- `docs/features/color-and-filters.md` — CL-002
- `docs/features/timeline-editing.md` — TL-001
- `scratchpad/inventory/preview-and-export.md` §§ 6–14 (paraphrased behavior
  notes)

Olive — GPL-3.0-or-later:
- `app/task/export/export.cpp`, `app/task/export/export.h`
- `app/task/render/render.cpp`
- `app/render/renderprocessor.cpp`, `app/render/opengl/openglrenderer.cpp`
- `app/codec/ffmpeg/ffmpegencoder.cpp`, `app/codec/ffmpeg/ffmpegencoder.h`
- `app/codec/exportformat.cpp`, `app/codec/exportcodec.cpp`
- `app/dialog/export/codec/h264section.cpp`, `app/dialog/export/codec/av1section.cpp`
- `app/common/filefunctions.cpp`

MLT — LGPL-2.1-or-later:
- `src/modules/avformat/consumer_avformat.c`
- `src/modules/avformat/common.c`
- `src/modules/avformat/blacklist.txt`
- `src/framework/mlt_consumer.c`
- `src/framework/mlt_audio.c`
- `presets/consumer/avformat/YouTube`, `x264-medium`, `SVT-AV1`, `GIF`, `AAC`,
  `webm`, `ten_bit/DNxHR-HQ`, `ten_bit/ProRes 422`, `ten_bit/AV1`

Shotcut — GPL-3.0-or-later:
- `src/docks/encodedock.cpp`
- `src/jobs/encodejob.cpp`, `src/jobs/meltjob.cpp`, `src/jobs/abstractjob.cpp`
- `src/jobqueue.cpp`
- `src/util.cpp`

Kdenlive — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL (ideas only):
- `src/dialogs/renderwidget.cpp`
- `src/render/renderrequest.cpp`
- `renderer/renderjob.cpp`
- `src/renderpresets/renderpresetmodel.cpp`
- `data/profiles.xml`

libopenshot — LGPL-3.0-or-later:
- `src/FFmpegWriter.cpp`, `src/FFmpegWriter.h`
- `src/Frame.cpp`

OpenTimelineIO — Apache-2.0:
- `src/opentime/rationalTime.h`

Rust crates (sources inspected locally in `~/.cargo/registry`):
- `ffmpeg-next` 9.0.0: `src/format/mod.rs` (`output_as`, `from_path`
  unwraps), `src/format/context/output.rs` (`write_header_with`,
  `write_trailer`), `src/format/context/destructor.rs` (`avio_close` result
  ignored), `src/codec/encoder/video.rs` (`open_as_with` drops leftover
  options)
- `wgpu` 30.0.1: `src/api/device.rs` (`poll`), `src/api/buffer.rs`
  (`map_async`, `get_mapped_range` → `Result`),
  `src/api/command_buffer_actions.rs` (`map_buffer_on_submit`)
- `keepawake` 0.6.1: `src/lib.rs` (Builder API), `src/sys/windows.rs`
  (`SetThreadExecutionState`; `sleep` → `ES_AWAYMODE_REQUIRED`),
  `src/sys/macos.rs`, `src/sys/linux.rs`

Local tool facts (FFmpeg 8.1.1 on this machine, `ffmpeg -h encoder=…` and
`-h muxer=…`):
- the `gif` encoder's pixel formats start with `rgb8`;
- `libopus` supports 48/24/16/12/8 kHz only;
- the `wav` muxer has `rf64` default `never`;
- `libx264` has a `stats` option and says `nal-hrd` "cbr not allowed in .mp4";
- `h264_videotoolbox` takes `nv12`/`yuv420p` (no `p010`), and
  `hevc_videotoolbox` adds `p010le`;
- `aac`, `libopus`, `libmp3lame` and `flac` report the "small" (short last
  frame) capability.
