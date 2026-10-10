# Media I/O: probe, decode, seek, proxies, thumbnails, waveforms

> Status: draft · Crates: freecut-media (fills plain-data types owned by `freecut-timeline::media` / `freecut-timeline::library`; consumed by freecut-render, freecut-audio, freecut-app)

This document covers everything between a file path and:

1. a `MediaInfo` record (probe),
2. decoded video frames and audio samples at exact media times (decode and seek),
3. derived, regenerable data: posters, filmstrips, waveform peaks, keyframe indexes and proxies.

Out of scope here: encoding and export muxing (export design), compositing and YUV→RGB shaders (render, FX-001 L2, CL-002), mixing and resampling to the project rate (freecut-audio, AU-010), and the job queue and its UI (freecut-app, PM-073). This document defines the work functions those parts call.

Paths to reference code are written as `<repo>/<path inside that repo>`. Licenses are listed in [References](#references).

---

## What FreeCut needs

### Functional requirements by feature

| Feature | What media I/O must provide |
|---|---|
| PM-030 / PM-031 / PM-032 | Background probe per file. Up to max(2, cores/2) probes at a time, 30 s timeout. Works on Unicode, emoji and long (> 260 chars, Windows) paths. Files still being written are probed as they are. Cloud placeholders and network shares may stall, so they must time out instead of hanging. |
| PM-035 | The probe extracts: container, duration (if missing, a background exact scan), start offset, start timecode, size, mtime and fingerprint. Per video stream: codec, profile, bit depth, pixel format, size, SAR, rotation **and mirroring** (display matrix), nominal and average rate, VFR flag, field order, alpha (including VP8/VP9 alpha side data), and color tags. Per audio stream: codec, rate, layout, duration, language. Subtitle streams are listed. Attached pictures are used as tile art. Normalization rules: rotation, square pixels, odd sizes, VFR delivered by timestamp, media time 0 = first presentable frame, the A/V offset is preserved. VFR is detected from rate disagreement or > 1 % spread of timestamp deltas over the first 300 packets. MP4 edit lists and negative timestamps are honored. A one-frame video is valid. Panics are caught at the job boundary. |
| PM-036 | Content decides, not the extension. A startup capability table is built from the linked FFmpeg. DRM-protected files are refused. Hardware decode is optional and software decode is always the fallback. |
| PM-037 | `ProbeError` variants: `NotFound`, `PermissionDenied`, `Empty`, `NoStreams`, `UnsupportedCodec { codec }`, `Corrupt { detail }`, `Incomplete`, `Protected`, `Timeout`, `Io`. |
| PM-038 | `Fingerprint { size, head_tail: xxh3-64(first+last 1 MiB), duration }` is computed for every import. It is the cache key for all derived data and drives relink and change detection. |
| PM-033 | Image sequences: detection (pure `detect_sequences`), per-frame decode, alpha kept, gaps hold the previous frame, mixed sizes fitted to the first frame, lazy for more than 10,000 frames. |
| PM-034 | Stills: EXIF orientation applied. A large image is decoded once and downscaled to min(2 × canvas long side, 8192). Alpha is kept for PNG, WebP, TIFF and GIF. CMYK becomes RGB. In the mvp, 16-bit becomes 8-bit and animated images show their first frame (moving in v1, OV-012). |
| PM-040 | Decode honors the interpretation overrides: frame rate (VFR is conformed by frame count), field order, PAR, rotation and mirror, alpha mode, audio stream, color. |
| PM-060 | 16 cached hover frames per item. Hovering never triggers a decode. |
| PM-070 | Poster at min(1 s, 10 % of duration), at most 256 px tall, with rotation, PAR, interpretation and HDR tone mapping applied. Key: (fingerprint, time, size, interpretation hash). On failure, fall back to 0 s, then to the type icon. A corrupt cache entry is regenerated silently. |
| PM-071 / TL-042 | Filmstrip: a base strip of 1 frame per 2 s at 64 px from the nearest keyframe, then denser frames on demand for the visible range only. Stored per media item, not per clip. Built from the proxy when there is one. Built lazily for 3-hour files. Requests that are no longer visible are cancelled. |
| PM-072 / AU-009 | Min/max peak pyramid: 256 samples per bin, ×4 per level, one per stream. Key: (fingerprint, stream, format version). Built progressively. At high speeds bins are aggregated, never undersampled. |
| PM-073 | Work functions are cancellable and report progress. They are throttled during playback, and panics stay contained. |
| PM-074 | One cache root with size limits (20 GB total, 2 GB thumbnail sub-limit) and LRU eviction. Entries visible in the open project are pinned. Proxies are evicted last. Writes are atomic, so two FreeCut processes can share the cache. |
| PM-080 / PM-081 | Proxy format: H.264 8-bit 4:2:0, keyframe every 10 frames, CRF 23, libx264. Timestamps and cadence are kept exactly, VFR included, so proxy frame N is original frame N. Video only. Alpha sources get an alpha-capable proxy. Images never get proxies. Image sequences are proxied as video. Proxies are found by naming convention. They go stale when the fingerprint or the resolution setting changes. The decoder factory chooses proxy or original. If the original is offline and a proxy exists, preview still works. |
| PM-082 (later) | Optimized media reuses the proxy infrastructure. |
| PM-090 / PM-091 / PM-093 | `check(&StoredPath, &Fingerprint) -> MediaStatus` with a 2 s timeout per volume. `validate_candidate(&MediaInfo, &Path) -> Match`: same kind, duration within 1 frame (or 100 ms for audio), same display resolution, same channel count. Change detection by fingerprint. |
| TL-001 | Ticks are 1/705,600,000 s. Stream timestamps are converted to ticks by exact rational multiplication, rounded to nearest with ties to even. Frame choice is media's job. |
| TR-036 / TR-041 / TR-043 | Return the frame whose display interval contains `s(t)`, chosen by PTS (VFR-safe). Frame blend needs up to 16 consecutive source frames per output frame. Reverse play uses a GOP-wise reverse cache, and export never depends on that cache. |
| EX-002 / EX-004 / EX-007 | The decoder pool and its read-ahead live here. Latest-wins requests. A keyframe-approximate frame during fast drags, and the exact frame within 150 ms (1080p long-GOP). Reverse shuttle decodes GOPs into a bounded RAM cache (512 MB). |
| EX-005 | Offline and decode-error states are reported per frame. Errors are logged once per (source, range). |
| EX-015 | A plain source player for media preview mode, on top of the same readers. |
| EX-018 | Hardware decode set to Auto or Off. VideoToolbox, D3D11VA, NVDEC, VAAPI and QSV. A stream the hardware can't decode falls back to software. Device loss recreates the decoder in software and shows one toast. Streams beyond the hardware's limit use software. v1 copies frames through CPU memory. |
| CL-002 | Expose the color tags and apply the defaults: untagged SD → BT.601, untagged HD → BT.709, untagged YUV → limited range, JPEG/PNG → full range. The shader conversion and HDR tone mapping are render's job. |
| AU-001 / AU-002 / AU-010 / AU-016 / AU-023 | Exact sample positions. A/V aligned by PTS, including AAC priming and edit lists. An exact-length scan for inexact durations. Worker threads read ahead into lock-free rings, and the real-time callback never decodes. Reverse and scrub reads. |
| OV-012 / OV-014 | Animated images: frames with rational start and duration, decode by index, an LRU of 256 MB per asset, disposal modes honored. Alpha video: ProRes 4444, QuickTime Animation, PNG-in-MOV, VP8/VP9 alpha (libvpx decoders). HEVC alpha that this build can't decode is reported, not silently dropped. |
| AI-001 and others | Decode audio to 16 kHz mono, 44.1 kHz stereo or 48 kHz. Decode frames for analysis. Analysis may run on proxies when originals are offline. |

### Non-functional requirements

- **Never on the UI thread.** Every blocking API in this crate asserts this in debug builds (see [Threading](#decoder-pool-read-ahead-and-threading)).
- **Latency.** Exact frame within 150 ms after the pointer stops, for 1080p long-GOP. Playback starts in under 150 ms from cached or proxy media. At least 32 audio clips mixed in real time.
- **Bounded memory.** Decoder count, frame queues, reverse caches and image caches all have byte budgets.
- **Robustness.** A missing, truncated, corrupt or hostile file must not hang the app. Every read is interruptible. As far as possible it must not crash the app either; see the subprocess open question. It must never leave a half-written cache entry or proxy that later looks valid.
- **Determinism.** The same file and time always give the same frame. Hardware and software decode give the same frame.

### Spec tensions this design resolves (for spec owners)

1. **Where normalization happens.** PM-035 says rotation and square pixels are applied "in decode", but FX-001 L2 and CL-002 put rotation, PAR and the YUV→RGB matrix in the render shader. **Resolution:** media *computes* the normalization exactly once and attaches it to every frame, as `DisplayGeometry` plus a resolved `ColorInfo` (tags, CL-002 defaults and PM-040 overrides). The GPU applies it in L2. CPU consumers (thumbnails, AI) get it applied by `freecut-media::convert::to_rgba`. There is one source of truth, and 4K frames are never rotated on the CPU.
2. **Proxy geometry.** PM-080 says rotation and pixel aspect are baked into proxies. With PM-040 overrides, that would force regenerating the proxy on every interpretation change. It would also make proxy and original geometry differ, which breaks "proxy frame N == original frame N" checks. **Recommendation:** a proxy is a *uniformly scaled copy of the coded picture*: not rotated, same SAR, same color tags. The decoder applies the original's geometry and color to both. Shotcut does the same: it transcodes with `-noautorotate` and re-injects rotation metadata afterwards (`shotcut/src/proxymanager.cpp`, `shotcut/src/jobs/postjobaction.cpp`).
3. **WebP thumbnails (PM-070).** The FFmpeg 8.1 build on the dev machine has no libwebp, and the `image-webp` encoder is lossless only. **Recommendation:** JPEG for opaque tiles, PNG for alpha tiles.
4. **Resampling ownership.** AU-010 puts project-rate resampling in freecut-audio. The AI specs ask media for 16 kHz mono. **Recommendation:** media always decodes at the native rate. One resampler utility (rubato) lives in `freecut-media::audio::resample`, so analysis paths that don't link freecut-audio can use it, and freecut-audio may reuse it.

---

## How the reference editors do it

### Olive — `olive/` (GPL-3.0-or-later, C++/Qt)

**Key files:**
- `olive/app/codec/decoder.{h,cpp}`
- `olive/app/codec/ffmpeg/ffmpegdecoder.cpp`
- `olive/app/codec/conformmanager.cpp`
- `olive/app/render/rendercache.h`, `olive/app/render/renderprocessor.cpp`, `olive/app/render/rendermanager.{h,cpp}`
- `olive/app/audio/audiovisualwaveform.{h,cpp}`
- `olive/app/common/ffmpegutils.cpp`, `olive/app/common/filefunctions.cpp`
- `olive/app/task/project/import/import.cpp`

**Structure.** `Decoder` is an abstract class with one `QMutex` per instance and `Open / RetrieveVideo / RetrieveAudio / Close`. It caches the last texture by (time, divider), so a paused redraw is free (`decoder.cpp`). `FFmpegDecoder::Instance` wraps one format context and one codec context per stream (`ffmpegdecoder.cpp`).

**Probe.** `FFmpegDecoder::Probe` opens a second `Instance` per video stream and decodes two frames, to learn interlacing, `av_guess_sample_aspect_ratio` and `av_guess_frame_rate`. If `duration_estimation_method == AVFMT_DURATION_FROM_BITRATE`, or no duration exists, it decodes the *whole stream* to find the last `best_effort_timestamp`. A video stream whose second read returns EOF is treated as a still. A file with audio and stills but no real video (album art) gets its stills disabled.

**Seek (`FFmpegDecoder::RetrieveFrame`).**
- The target is converted to a stream timestamp, plus the *format* `start_time`.
- A deque of decoded frames is kept (`MaximumQueueSize() == 2`). Forward decoding is reused while the target lies within `[front, back + 2 s]`. Otherwise the decoder is flushed and `av_seek_frame(stream, ts, AVSEEK_FLAG_BACKWARD)` is called.
- If the first frame after a seek is already later than the target, it seeks one more second back and retries until it reaches 0 (`cache_at_zero_`).
- On overshoot it returns the *previous* frame, giving "frame at or before" semantics. At EOF it returns the last frame.

Excerpt (`olive/app/codec/ffmpeg/ffmpegdecoder.cpp`, GPL-3.0-or-later):
```cpp
if (!cache_at_zero_ && (ret == AVERROR_EOF || filtered->best_effort_timestamp > target_ts)) {
  seek_ts = qMax(min_seek, seek_ts - second_ts_);
  instance_.Seek(seek_ts);
  if (seek_ts == min_seek) { cache_at_zero_ = true; }
  continue;
}
```

**Pixels.** Planar YUV at 8, 10 and 12 bits is uploaded as three textures and converted in GLSL. The matrix comes from `sws_getCoefficients(colorspace)` and the range from the frame. Every other format goes through `sws_scale` to RGBA or RGBA64 with `SWS_POINT`, and the preview "divider" downscale also happens on the CPU (`ProcessFrameIntoTexture`, `PreProcessFrame`). Deinterlacing is done in a shader.

**Audio.** Olive "conforms" audio. The whole stream is decoded and resampled with swr to project parameters, into one planar PCM file per channel. The file is written as `.working` and renamed when done (`conformmanager.cpp`). Playback then reads by byte offset. Until the conform exists, `RetrieveAudio` returns `kWaitingForConform` (`decoder.cpp`).

**Decoder lifetime.**
- `DecoderCache` is a `QHash<CodecStream, DecoderPair{decoder, last_modified}>` (`rendercache.h`).
- A decoder is reopened if the file's mtime changed (`RenderProcessor::ResolveDecoderFromInput`).
- A timer closes decoders idle for more than 5 s, or 1 s in aggressive mode (`rendermanager.h`: `kDecoderMaximumInactivity`).

**Waveform.** `AudioVisualWaveform` keeps per-channel `{min, max}` at power-of-two summary rates from 1/8 to 1024 per second. Each coarser level is built from the finer one (`audiovisualwaveform.cpp`).

**Image sequences.** Detection requires the basename to end in digits. The neighbours N−1 and N+1 are probed, then the sequence limits are walked until the first missing file (`import.cpp: ValidateImageSequence`, `GetImageSequenceLimit`).

**Pitfalls seen in the code:**
- No rotation handling anywhere in `olive/app/codec/ffmpeg/ffmpegdecoder.cpp`: there is no display-matrix or `rotate` lookup, so phone portrait clips come out sideways.
- `RetrieveFrame` mixes `pts` and `best_effort_timestamp`, and compares `pts == target_ts` after a rational→timestamp conversion. This is fragile when the timebase doesn't divide the time grid.
- `Instance::GetPacket` reads and throws away packets of every other stream. `AVStream.discard` is never set, which wastes I/O on multi-track files.
- Image-sequence frames open a fresh decoder for every frame (`renderprocessor.cpp`, "we don't engage the decoder cache"). Sequence detection stops at the first gap.
- No audio at all until the whole file is conformed. The conform key is SHA-1 of path + mtime (`filefunctions.cpp: GetUniqueFileIdentifier`), so it breaks when a file moves.
- Waveform time→index uses `double`.
- No hardware decode.

### MLT `producer_avformat` — `mlt/` (LGPL-2.1-or-later, C)

**Key files:**
- `mlt/src/modules/avformat/producer_avformat.c` (about 4,700 lines)
- `mlt/src/modules/avformat/common.{c,h}`
- `mlt/src/framework/mlt_cache.c`, `mlt/src/framework/mlt_service.c`

**Contexts.**
- For seekable files, `producer_open` opens *separate* `AVFormatContext`s for audio and for video, so the two seek independently.
- `set_up_discard` sets `AVDISCARD_ALL` on every stream that isn't selected.
- After validation the file is closed again, "to release resources for large playlists" (`producer_avformat_init`). Live decoder state then sits in a per-class LRU, `mlt_service_cache_put(..., "producer_avformat")`.
- Apps size that LRU as max(4, threads + 2 × tracks): Shotcut in `shotcut/src/mltcontroller.cpp: updateAvformatCaching`, Kdenlive in `kdenlive/src/timeline2/model/timelinemodel.cpp`.
- A packet thread (`packets_worker`) keeps one video packet prefetched.

**Seek (`seek_video`).**
- It seeks only when moving backwards, or when the target is at least `seek_threshold` (64) frames ahead. Otherwise it decodes forward.
- "Preseek" subtracts 2 s for codecs with B-frames.
- During pre-roll it sets `skip_loop_filter = AVDISCARD_NONREF`, and resets it to `AVDISCARD_NONE` once the target is reached (`producer_get_image`). This is a cheap speed-up because skipped frames are never shown.

**Timestamps.** `best_pts()` chooses pts or dts by counting invalid values. Excerpt (`mlt/src/modules/avformat/producer_avformat.c`, LGPL-2.1-or-later):
```c
self->invalid_pts_counter += pts == AV_NOPTS_VALUE;
self->invalid_dts_counter += dts == AV_NOPTS_VALUE;
if ((self->invalid_pts_counter <= self->invalid_dts_counter || dts == AV_NOPTS_VALUE)
    && pts != AV_NOPTS_VALUE)
    return pts;
else
    return dts;
```
- `find_first_pts()` scans up to 500 packets for the first keyframe PTS. It special-cases negative DTS (a B-frame MP4 "shows the third frame first") by assuming a first PTS of 0.
- VFR is declared when at least 3 distinct packet durations appear among the first 20 frames (`VFR_THRESHOLD`), or when avg ≠ r rate, ignoring denominators 1, 2, 125 and 1001.
- Positions are frame indexes computed as `(pts − first_pts) × tb × fps + 0.5` in doubles. There is a "WILD TIMESTAMP" clamp when |requested − decoded| > 999, and duplicate positions are bumped by +1.

**Geometry.** `get_rotation()` prefers the legacy `rotate` tag. Otherwise it uses `-av_display_rotation_get()` on the display matrix from `codecpar->coded_side_data`. For 90° and 270°, width/height are swapped and the aspect ratio is inverted. There is a workaround for "1088" encodings that lack cropping info (`producer_set_up_video`). Mirroring isn't handled.

**Frame rate (`guess_frame_rate`).**
- Starts from `av_guess_frame_rate`.
- Ignores `r_frame_rate ≥ 1000`.
- Snaps Matroska's 1 ms-rounded 29.97, 59.94, 23.976 and 47.952 back to the 1001 rates.
- Honours Android's `com.android.capture.fps` (slow-motion capture).

**Alpha.** VP8 and VP9 are forced onto the `libvpx` / `libvpx-vp9` decoders, because the native decoders report `yuv420p` for alpha streams. If the pixel format turns out not to be `yuva420p`, it switches back to the native decoder (`find_default_streams`, `video_codec_init`).

**Hardware decode (`video_codec_init`, `producer_get_image`).**
- `avcodec_get_hw_config` is matched against the device type, then `av_hwdevice_ctx_create` is called, and frames are copied with `av_hwframe_transfer_data`.
- Optional GPU scale filters (`scale_vt`, `scale_vaapi`, `scale_d3d11`) are not used for interlaced input.
- **D3D11 HEVC texture padding** (black bars) is detected and the filter disabled (`try_setup_hwaccel_filters`).
- A pixels-per-second threshold (environment variable) can disable hwaccel.

**Errors.** The decoder keeps a clone of the last good frame and returns it on decode failure ("error concealment"). It tolerates 10 consecutive decode errors. MPEG files often fail at the start because of B-frames that reference frames before the first I-frame.

**Audio (`seek_audio`, `decode_audio`).**
- Seeks when the position jumps by at least 12 frames.
- Aligns by comparing packet pts (minus the first *video* pts) with the request, in doubles. It skips samples when behind, and seeks back 1 s when ahead. WMA Pro gets extra tolerance.

**Conversion flags (`common.c: mlt_get_sws_flags`).**
- YUV→RGB at the same size: `SWS_POINT | ACCURATE_RND | FULL_CHR_H_INT`.
- YUV→YUV with the same subsampling: `POINT`.
- Any resize: `BICUBIC` plus the full-chroma flags.

**Pitfalls.**
- The frame-index model quantizes everything to the profile frame rate through doubles. This is why both Shotcut (`shotcut/src/util.cpp: getConversionAdvice`, "variable frame rate, which is not reliable for editing") and Kdenlive (`kdenlive/src/jobs/cliploadtask.cpp`) push users to transcode VFR files.
- Five recursive mutexes per producer.
- One shared decoder per bin clip makes same-media overlaps thrash. Kdenlive works around it with per-track producers (`kdenlive/src/bin/projectclip.cpp: getTimelineProducer`, `m_videoProducers`).

### Shotcut — `shotcut/` (GPL-3.0-or-later, Qt on MLT)

**Key files:**
- `shotcut/src/proxymanager.cpp`
- `shotcut/src/jobs/postjobaction.cpp`
- `shotcut/src/util.cpp`
- `shotcut/src/mltcontroller.cpp`

**Proxy job (`generateVideoProxy`).** Runs an external `ffmpeg` with:
- `-noautorotate` and the video+audio maps;
- optional `yadif`, then `scale=-2:<res>` with *explicit* `in_range`/`out_range`;
- *explicit* `-color_primaries/-color_trc/-colorspace` chosen per source colorspace;
- libx264 `-preset veryfast -crf 23`, or a hardware encoder (nvenc, qsv, amf, vaapi, videotoolbox, mf);
- `-g 1 -bf 0`, which makes **all-intra proxies**. On Apple Silicon, VideoToolbox is skipped for full-range sources.

**Rotation.** Rotation is injected afterwards by a `-c copy` remux with `-display_rotation`. A comment notes the sign flip: "-display_rotation is counter-clockwise, while m_rotation is the clockwise rotate convention" (`ProxyFinalizePostJobAction::doAction`).

**Crash-safe naming.** The job writes `<hash>.pending.mp4` and renames it on success. `removePending()` deletes leftover pending files.

**Where proxies live.** In the project's `proxies/` folder or a global folder. `utime()` touches a proxy on use, a cheap LRU signal.

**What is never proxied (`isValidVideo`, `isValidImage`).**
- Pixel formats with alpha (a long explicit list).
- Streams with the `alpha_mode` tag.
- Cover art (`mjpeg` at 90000 fps).
- Sources smaller than 1.3 × the proxy resolution.

Images get JPEG proxies, but only when they have no alpha. Camera-made proxies are used when present: GoPro `GL*.LRV`, DJI `*.LRF` (`GoProProxyFilePath`, `DJIProxyFilePath`).

**Hash.** `Util::getFileHash` is MD5 of the first 1 MB + last 1 MB, or the whole file at 2 MB or less. The source comments it as "intentionally copied from Kdenlive" (`shotcut/src/util.cpp`). The file size isn't part of the hash.

### Kdenlive — `kdenlive/` (per-file SPDX `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`, Qt on MLT)

**Key files:**
- `kdenlive/src/jobs/proxytask.cpp`, `kdenlive/src/jobs/cachetask.cpp`
- `kdenlive/src/jobs/audiolevels/{audiolevelstask.cpp,generators.cpp}`
- `kdenlive/src/utils/thumbnailcache.{hpp,cpp}`
- `kdenlive/src/bin/projectclip.cpp`
- `kdenlive/src/jobs/cliploadtask.cpp`

**Proxies (`ProxyTask::run`).**
- External ffmpeg with user-editable parameter profiles, sanitized against a list of forbidden arguments.
- A separate alpha profile, which forces the `libvpx` decoders for VP8/VP9 alpha.
- `-noautorotate`, and `-map 0`, so **proxies keep audio**.
- NVDEC/NVENC eligibility is checked by codec and pixel format.
- Progress is parsed from ffmpeg's stderr `time=` lines.
- Camcorder proxies are checked for audio-stream count. The comment says Sony FX6 proxies carry 1 stream; a mismatch triggers a rebuild (`cliploadtask.cpp`).

**Decoders per track.** Each (bin clip, track) gets its own producer (`projectclip.cpp: getTimelineProducer`).

**Thumbnails.**
- `CacheTask` seeks a dedicated *thumbnail producer* with `consumer.rescale=nearest` and deinterlacer `onefield`, both cheap settings.
- `ThumbnailCache` combines a volatile in-memory LRU (cost cap 10,000,000 bytes) with persistent JPEGs named `<clipHash>#<frame>.jpg`. The frame number is in *project* frames.

**Audio levels.**
- The libav fast path `generateLibav` discards other streams and converts to s16 with swr.
- It stores **5 peak points per project frame** (`AUDIOLEVELS_POINTS_PER_FRAME`) in `<hash>_<stream>_<int(fps)>_audio.dat`, written with `QDataStream` and no header (`audiolevelstask.cpp`, `projectclip.cpp: getAudioThumbPath`).
- It falls back to the slow MLT path when `stream->start_time > 0`.

**Pitfalls in the code:**
- **A truncated proxy can be accepted forever.** The proxy is written straight to its final name, and `run()` treats any existing non-empty file as done (`fInfo.exists() && fInfo.size() > 0`). A crash during transcoding leaves a short proxy that is then trusted.
- **The peak cache is tied to the project frame rate** through `int(fps)` in the file name. 29.97 and 29 collide. There is no header or version, and the write isn't atomic.
- `computePeaks` uses `std::abs` on `int16_t` and narrows the result back to `int16_t`, so |−32768| wraps to −32768 and full-scale negative peaks are under-reported. It also stores only max(|x|), not min and max. Excerpt (`kdenlive/src/jobs/audiolevels/generators.cpp`):
  ```cpp
  int16_t maxValue = std::abs(pIn[start * nChannels]);
  for (size_t inIdx = start; inIdx < end; ++inIdx) {
      maxValue = std::max(maxValue, static_cast<int16_t>(std::abs(pIn[inIdx * nChannels])));
  }
  ```
- `generateLibav` never sends EOF to the decoder, so the tail samples still buffered in the decoder are dropped.

### libopenshot — `libopenshot/` (LGPL-3.0-or-later, C++)

**Key files:** `libopenshot/src/FFmpegReader.{h,cpp}`, `libopenshot/src/FFmpegUtilities.h`, `libopenshot/src/AudioWaveformer.h`.

**State.** `PacketStatus` counts packets read vs. decoded per stream, for EOF detection. A working cache and a final cache hold assembled `Frame`s (`FFmpegReader.h`).

**Seek (`FFmpegReader::Seek`).**
- For targets below about frame 32, it **closes and reopens the file** instead of seeking ("more reliable than seeking").
- Otherwise it seeks to `requested − 12` frames on the video stream, with the audio stream as fallback.
- If the seek fails, it reopens the file and sets `enable_seek = false` for good.

**Timestamps (`UpdatePTSOffset`, `ConvertVideoPTStoFrame`).**
- The *video* start anchors frame 1. The comment says this "avoids AAC priming / audio preroll shifting video frame 1 to frame 2".
- Timestamps more than 10 s from zero are ignored as "wildly invalid".
- Frame numbers are `round(seconds × fps) + 1` in doubles, and duplicates map to −1.

**Bad frame rates (`CheckFPS`).** It counts packets in seconds 0–2 and **truncates the average fps to an integer**: `info.fps = Fraction(int(avg_fps), 1)`, so 29.97 becomes 29.

**Durations (`PickDurationSeconds`).** Strategies VideoPreferred, AudioPreferred and LongestStream, over the stream, format and bitrate-inferred durations. Stills are forced to 1 hour.

**Orientation.** Read from the `rotate` tag, else from the display matrix, with a NaN guard (`Open`). It is applied after decode by `QImage::transformed` (`ApplyFrameOrientation`). The decode size is bounded by the preview size, with the bounds swapped for quarter turns (`ProcessVideoPacket`).

**Hardware decode.**
- `get_hw_dec_format` writes the chosen format into **global variables**, and **returns `AV_PIX_FMT_NONE`** when the preferred hardware format isn't offered. That makes the open fail instead of falling back to software.
- Hardware is enabled only for H.264, MPEG-2, VC-1 and WMV (`IsHardwareDecodeSupported`).
- Failure is detected as 3 INVALIDDATA errors before the first frame, which triggers `ReopenWithoutHardwareDecode`.

**Pitfalls in the code:**
- `GetAVFrame` has a "TODO also handle possible further frames". Only the first frame of a receive loop is used, and every frame is copied with `av_image_copy`.
- **Wrong color matrix for HD.** `sws_setColorspaceDetails` is called with `sws_getCoefficients(SWS_CS_DEFAULT)` (BT.601) on both sides, so HD sources get BT.601 colors. The default scaler is `SWS_FAST_BILINEAR`.

**Waveform.** `AudioWaveformer` keeps max *and* RMS per bucket. RMS is a nice inner band for drawing.

### OpenTimelineIO — `opentimelineio/` (Apache-2.0)

`opentimelineio/src/opentimelineio/imageSequenceReference.{h,cpp}` describes image sequences with:
- `start_frame`, `frame_step`, zero padding;
- negative frame numbers;
- `rate` as a `double`;
- `MissingFramePolicy { error, hold, black }`.

It is useful vocabulary for PM-033 and for interchange (PM-103). FreeCut keeps the policy names but stores the rate as a rational.

### Comparison

| Concern | Olive | MLT | libopenshot | Shotcut / Kdenlive (on MLT) | FreeCut (this design) |
|---|---|---|---|---|---|
| Frame identity | rational time → ts, `pts == ts` | frame index from double fps | frame number from double seconds | MLT frame index | PTS converted once to ticks; choice compared in ticks |
| VFR | follows PTS (by accident) | flagged; index math drifts | `CheckFPS` truncates to int | warn and suggest transcode | first-class: PTS-based, no transcode needed |
| Seek | flush + `av_seek_frame`, retry 1 s back | threshold 64 frames, 2 s preseek, skip loop filter | `requested−12`, reopen near start, give up on failure | via MLT | keyframe index + cost planner, retry back, skip loop filter during pre-roll, reopen fallback |
| Rotation | none | tag or display matrix, no mirror | tag or matrix, QImage rotate | via MLT; proxies inject rotation | stream and **frame** display matrix, rotation + mirror, applied on GPU |
| Audio | full conform to PCM files | live decode, double alignment | per-frame audio location | via MLT | live decode, exact sample alignment, optional conform cache |
| Decoder reuse | per (file, stream), idle 5 s | global LRU of producers | one reader per clip | per track (Kdenlive) | pool with cost-based affinity and byte budget |
| HW decode | none | device ctx + transfer, padding fix | globals, NONE on miss | settings | per-decoder state, software fallback inside `get_format`, failure detector |
| Waveform | float min/max, time mipmaps | — | max + RMS | int16 |max|, per project frame | int16 min/max per channel, sample-based pyramid, versioned file |
| Proxy safety | — | — | — | Shotcut: `.pending` rename; Kdenlive: direct write | temp + verify + atomic rename, embedded metadata |

### Lessons learned → FreeCut rules

These pitfalls are taken from the code above; each one becomes a rule in the design.

| # | Lesson (source) | FreeCut rule |
|---|---|---|
| L1 | Frame indexes from double fps drift and break on VFR (MLT; libopenshot `CheckFPS` integer truncation) | Identity is the PTS. A frame is chosen by comparing tick times converted with one function (I1). |
| L2 | Timebases like 1/15360, 1/12288 and AV_TIME_BASE don't divide 705,600,000 (checked numerically) | Never convert a tick target to a stream timestamp and compare for equality. Compare in tick space; stream timestamps are used only for seek targets with a margin. |
| L3 | Negative DTS on B-frame MP4 (MLT `find_first_pts`). AAC priming packet `pts=-1024` flagged discard (verified with FFmpeg 8.1 `ffprobe` on a generated fixture). | Use *decoded frame* PTS, keep libavcodec's default skip-samples and discard handling, and anchor media time 0 on the first presentable video frame (libopenshot's rationale). |
| L4 | Seek lands after the target on broken indexes and open GOPs (Olive retry loop) | After a seek, if the first decoded frame is later than the target, step back one keyframe (or 1 s, doubling) and retry. Reopening and decoding linearly is the last resort (libopenshot). |
| L5 | Decoding from far away is slow; tiny jumps shouldn't seek (MLT threshold 64; Olive 2 s window) | A cost planner using the keyframe index: decode forward when no keyframe lies between the current position and the target. |
| L6 | Pre-roll frames are never shown (MLT `skip_loop_filter`) | Use `skip_loop_filter = NonReference` during pre-roll. Thumbnails use `skip_frame = NonKey`. |
| L7 | Unused streams waste I/O (Olive). MLT and Kdenlive discard them. | Set `discard = All` on every stream except the one being decoded, with one format context per reader. |
| L8 | Rotation ignored (Olive). The legacy tag is gone in modern FFmpeg. EXIF orientation arrives as *frame* side data in FFmpeg 8.1 (verified). Sign conventions are confusing (Shotcut comment; `av_display_rotation_set(90)` reads back as −90). | Read the stream `coded_side_data` and the first frame's `AV_FRAME_DATA_DISPLAYMATRIX`. Clockwise quarter turns = round(−get/90). Mirrored = det < 0. Pin the convention with golden fixtures. |
| L9 | Native VP8/VP9 decoders drop alpha (MLT) | Probe the `alpha_mode` tag. Prefer `libvpx`/`libvpx-vp9` for alpha streams; if they're missing, report "alpha not supported". |
| L10 | Matroska 1 ms timestamps make 29.97 jitter 33/34 ms (MLT normalization) | VFR test tolerates ±1 timebase unit. The nominal rate snaps to standard rates within 0.1 %. |
| L11 | HW `get_format` returning NONE, and global state (libopenshot) | `get_format` returns the first *software* format when the wanted hardware format isn't offered. State goes through `AVCodecContext.opaque`, never globals. |
| L12 | HW failures surface as INVALIDDATA before the first frame (libopenshot). D3D11 HEVC padding (MLT). Hardware can't crop top/left with opaque frames (FFmpeg `apply_cropping` docs). | Failure detector, then a transparent software reopen at the same position. Call `av_frame_apply_cropping` after transfer, and test against software. |
| L13 | BT.601 default matrix for HD (libopenshot) | The CPU path uses FFmpeg 8's dynamic `sws_scale_frame` with explicit frame color properties. The GPU path uses render's shader with the resolved `ColorInfo`. |
| L14 | Every decoded frame copied (libopenshot `av_image_copy`) | Frames are ref-counted `AVFrame`s shared through `Arc`; zero copy until GPU upload. |
| L15 | Only the first frame per receive loop used (libopenshot TODO) | Drain `receive_frame` until EAGAIN. Every frame enters the frame ring. |
| L16 | Last samples lost without a decoder flush (Kdenlive `generateLibav`) | Every sequential job sends EOF and drains the decoder. |
| L17 | `abs(int16)` overflow and |x|-only peaks (Kdenlive) | Store min and max separately, computed in f32, quantized with saturation. |
| L18 | Peaks and thumbnails keyed by project frames or `int(fps)` (Kdenlive) | Derived data is keyed by *native* stream timestamps or sample indexes, independent of project settings and interpretation. |
| L19 | Truncated proxy trusted forever (Kdenlive). Shotcut's `.pending` rename works. | Write to a temp name, verify by probing and comparing timing with the source, then rename atomically. Embed a version and the source fingerprint in proxy metadata. Delete stale temp files on startup. |
| L20 | All-intra proxies scrub best (Shotcut `-g 1 -bf 0`) | No B-frames. GOP of 10 per the spec, with all-intra (`g=1`) as a "smoothest scrubbing" preference. |
| L21 | A 540p proxy of untagged HD would get BT.601 under the "SD → 601" default; Shotcut writes explicit tags | The decoder resolves color from the *original's* info and overrides, never from the proxy. Proxies also carry explicit tags. |
| L22 | One decoder per clip thrashes when the same media is used twice (Kdenlive per-track producers) | Several decoders per source in the pool, chosen by position affinity. |
| L23 | Decoder count must scale with tracks and threads (Shotcut/Kdenlive max(4, threads + 2·tracks)) | Pool caps by count *and* estimated bytes. Idle decoders close (Olive 5 s). |
| L24 | Seek near the start is unreliable (libopenshot reopens) | Seek to the start with `avformat_seek_file(stream, i64::MIN, first_ts, first_ts)`; if the first decoded PTS is still wrong, reopen. |
| L25 | Duration from bitrate is a guess (Olive full scan) | Mark `duration_exact = false` and schedule a background exact scan (AU-001). |
| L26 | Decode errors early in MPEG files are normal (MLT tolerance 10) | Tolerate up to N errors per run, conceal with the last good frame within one frame interval, else report an error frame. |
| L27 | Camera proxies may lack audio streams (Kdenlive FX6 check) | Only use camera-made proxies after the same timing verification. Proxies never supply audio unless the original is offline. |
| L28 | `ffmpeg-next` `format::input(path)` calls `path.to_str().unwrap()` (panics on non-UTF-8 paths). `Input::seek` uses `stream_index = -1`, which means AV_TIME_BASE units. `PacketIter` hides I/O errors as end of stream. The legacy sws wrapper ignores the return value and can't set colorspace (all verified in ffmpeg-next 9.0.0 source). | Open through custom I/O (`StreamIo`) from a Rust `File`. Our own stream-specific seek. Drive `Packet::read` directly. Our own sws FFI. |

---

## Recommended FreeCut design

### Invariants

- **I1. One rounding.** `freecut_timeline::time::from_stream_ts` is the only stream-timestamp→tick conversion. Media time = that value minus the media origin, in ticks. Frame choice compares ticks only.
- **I2. Media origin.** Media time 0 is the first *presentable* frame of the default video stream after edit lists. With no video, it is the first decoded audio sample. Every other stream keeps its offset relative to that origin.
- **I3. PTS identity.** A decoded frame is identified by its stream PTS. A frame index is used only for interpretation conform (PM-040) and image sequences.
- **I4. Interruptible.** Every blocking call takes a `CancelToken`. The same token is installed as the FFmpeg AVIO interrupt callback, so a stalled network read aborts too.
- **I5. Exclusive use.** No FFmpeg context is used by two threads at once. Decoders are leased exclusively. Decoded `AVFrame`s are immutable and shared read-only.
- **I6. Content-keyed derived data.** Derived data is keyed by `Fingerprint` + stream + native timestamps, is immutable once published, is published by atomic rename, and carries a format version.
- **I7. Geometry and color travel with the frame.** They are computed once in media, including CL-002 defaults and PM-040 overrides.
- **I8. `unsafe` only in `freecut_media::av`.** Each block has a `// SAFETY:` comment.
- **I9. Software is the reference.** Hardware output must match software. A mismatch is a bug, and the hardware path gets disabled for that codec and profile.
- **I10. Never on the UI thread.** `freecut_media::mark_ui_thread()` is called once by the app. Every blocking API does `debug_assert!(!on_ui_thread())`.

### Module layout

```
crates/freecut-media/src/
  lib.rs            MediaRuntime (pool + caches + hw registry), init(), mark_ui_thread()
  error.rs          ProbeError, DecodeError, CacheError, ProxyError (thiserror)
  cancel.rs         CancelToken, Deadline
  av/               the ONLY module with `unsafe` (FFI boundary over ffmpeg-next / ffmpeg-sys-next)
    open.rs         open_input(): File + StreamIo + interrupt
    seek.rs         seek_stream(): avformat_seek_file on a stream index
    index.rs        read AVIndexEntry table (avformat_index_get_entry)
    side_data.rs    display matrix (stream + frame), alpha_mode, encryption info
    hw.rs           device registry, get_format callback, transfer + cropping
    sws.rs          dynamic sws_scale_frame wrapper (FFmpeg 8 API)
    filter.rs       deinterlace graph (bwdif), v1
    log.rs          av_log routing to `tracing`, per-context capture for probe diagnostics
    caps.rs         runtime capability table (PM-036)
  time.rs           StreamClock, media-origin math, sample-index math
  fingerprint.rs    Fingerprint computation (xxh3)
  probe/            mod.rs probe(), video.rs, audio.rs, still.rs, vfr.rs, duration.rs, errors.rs
  source.rs         ResolvedSource, SourceVariant (original / proxy), Interpretation application
  geometry.rs       DisplayGeometry, Rotation math, PAR
  color.rs          ColorInfo resolution (tags → CL-002 defaults → overrides)
  video/            decoder.rs (VideoDecoder), plan.rs (pure seek planner), ring.rs, frame.rs,
                    layout.rs (PixelLayout), reverse.rs (ReverseReader), concealment.rs
  still/            image.rs (stills via `image`), animated.rs (OV-012), sequence.rs (PM-033)
  audio/            reader.rs (AudioReader), prefetch.rs, convert.rs (to f32 planar), resample.rs (rubato)
  pool.rs           DecoderPool, Lease, budgets, idle close
  prefetch.rs       video read-ahead workers, generations
  convert.rs        to_rgba() CPU path (thumbnails, analysis)
  cache/            store.rs (CacheStore), key.rs, lru.rs, files.rs (atomic publish)
  thumbs/           poster.rs, hover.rs, strip.rs (filmstrip chunks), codec.rs (JPEG/PNG tiles)
  peaks/            build.rs (job), file.rs (format), query.rs (level choice)
  gop.rs            GopIndex (keyframe table) build/persist
  proxy/            naming.rs, encode.rs, verify.rs, camera.rs (v1+)
  status.rs         check(), validate_candidate()
  formats.rs        SUPPORTED table, dialog_filters()
  sequence.rs       detect_sequences() (pure)
```

Dependency direction follows the project layout: `freecut-media` depends on `freecut-timeline` (plain types and time math) and on nothing else internal.

### Plain-data additions requested in `freecut-timeline::media`

PM-035 defines `MediaInfo`, `VideoStreamInfo` and `AudioStreamInfo`. Decoding and caching also need a few more fields. These are proposed additions; the spec owner decides:

```rust
// freecut-timeline::media (plain data, no I/O). Additions to PM-035's types.
pub struct VideoStreamInfo {
    // ... PM-035 fields ...
    pub index: u32,                       // container stream index (selection survives remux)
    pub time_base: (i32, i32),            // native timebase, for native-timestamp cache keys
    pub start: Tick,                      // first presentable PTS relative to media origin (0 for default video)
    pub duration: Tick,
    pub frame_count: Option<u64>,         // exact when known (index or scan)
    pub capture_rate: Option<FrameRate>,  // e.g. Android slow-motion markup; suggestion only
    pub intra_only: bool,                 // AV_CODEC_PROP_INTRA_ONLY → seeking is always exact
    pub max_keyframe_interval: Option<Tick>, // drives the "long GOP → suggest proxy" hint (EX-004)
    pub alpha: AlphaKind,                 // None | Native (yuva/rgba) | SideChannel (VP8/9) | Unsupported
    pub attached_pic: bool,
    pub timestamps: TimestampQuality,     // Reliable | Repaired | Synthesized
}
pub struct AudioStreamInfo {
    // ... PM-035 fields ...
    pub index: u32,
    pub time_base: (i32, i32),
    pub start: Tick,                      // may be negative (audio before first video frame)
    pub duration_exact: bool,             // false when FFmpeg estimated it from bitrate
    pub frame_size: Option<u32>,          // samples per packet when constant (AAC 1024)
}
pub enum AlphaKind { None, Native, SideChannel, Unsupported }
pub enum TimestampQuality { Reliable, Repaired, Synthesized }
```

### Time inside media

```rust
/// Maps one stream's native timestamps to media time (I1, I2).
#[derive(Clone, Copy, Debug)]
pub(crate) struct StreamClock {
    tb_num: i32,
    tb_den: i32,
    origin: Tick, // media origin expressed in absolute ticks (same rounding function)
}

impl StreamClock {
    pub fn media_time(&self, ts: i64) -> Result<Tick, TimeError> {
        from_stream_ts(ts, self.tb_num, self.tb_den).checked_sub(self.origin)
    }
    /// A native timestamp guaranteed to be <= every ts whose media_time >= t.
    /// Used only as a seek target, never for equality.
    pub fn seek_ts_before(&self, t: Tick) -> i64 {
        // floor((t + origin) * den / (num * TICKS_PER_SEC)) - 1, in i128
        todo!()
    }
}

/// Audio: exact sample index in the stream's native rate (AU-010 formula).
/// TICKS_PER_SEC is divisible by every common sample rate, so this is exact for them.
pub(crate) fn sample_at(t: Tick, rate: u32) -> i64 { /* floor(t * rate / TPS) in i128 */ todo!() }
```

Why tick space matters: with the timebase 1/15360 (FFmpeg's default MP4 timescale for 30 fps), one timestamp unit is 45,937.5 ticks. Frame 3's start rounds (ties-even) to 137,812. Converting the request `t = 137,812` back to a timestamp gives 2.99999…, which floors to 2: the wrong frame. Comparing `media_time(pts) <= t` in ticks is always consistent.

### Opening an input

```rust
pub(crate) fn open_input(
    path: &Path,
    io_buf: usize,              // 256 KiB video, 64 KiB audio/probe
    cancel: CancelToken,
    stall: Duration,            // max time without I/O progress (2 s playback, 30 s probe)
) -> Result<ffmpeg_next::format::context::Input, OpenError>;
```

- Open the file with `std::fs::File`. This handles Unicode, Windows verbatim long paths and permission errors with a real `io::ErrorKind`. Wrap it in `format::context::StreamIo::from_read_seek_with_capacity` and pass it to `format::input_from_stream_with_interrupt`. The filename hint is the file name, converted lossily, and it is only used for format guessing.
- **Do not use `format::input(&path)`.** In ffmpeg-next 9.0.0, `from_path` does `to_str().unwrap()`, which panics on non-UTF-8 paths (Linux). Custom I/O also avoids the Windows CRT descriptor limit (512 by default) that FFmpeg's `file:` protocol hits when many decoders are open.
- The interrupt closure returns `true` when the token is cancelled, or when no read made progress within `stall`. The 30 s PM-030 timeout and the 2 s volume check (PM-090) are both implemented this way.
- **After a cancel, the AVIO context is poisoned:** `AVERROR_EXIT` is latched into `pb->error`, as ffmpeg-next's own comments explain. A reader that was cancelled sets its position to `Unknown` and calls `Input::clear_interrupt()`. The next use must start with a seek.
- Known limit: custom I/O can't follow MOV external data references or HLS-style relative URLs. Those inputs fail with `UnsupportedCodec`/`Corrupt`, which is acceptable for a desktop editor.

### Probe

`pub fn probe(path: &Path, cancel: &CancelToken) -> Result<MediaInfo, ProbeError>`, in order:

1. **stat.** Missing → `NotFound`. EACCES/EPERM → `PermissionDenied` (on macOS, also for privacy-protected folders). Size 0 → `Empty`. Record the mtime.
2. **Sniff the magic bytes before FFmpeg.** `.freecut` projects, zip, m3u and Windows shortcuts are refused with `ProbeError::NotMedia { kind }`. This is a proposed variant that covers the PM-036 hints.
3. **Hash.** `head_tail` = xxh3-64 of (first 1 MiB ‖ last 1 MiB), or of the whole file if it is 2 MiB or less, so the head and tail never overlap.
4. **Open** (see above) and run `find_stream_info` with the default `probesize` and `analyzeduration`. If an expected stream reports no parameters (TS audio that starts late), retry once with 4× `analyzeduration`.
5. **Classify streams.**
   - Video with `AV_DISPOSITION_ATTACHED_PIC` → cover art.
   - Data streams: read `tmcd` for the timecode.
   - Subtitle streams are listed.
   - Encrypted streams → `Protected`. Signs: codec tag `encv`/`enca`/`drmi`/`drms`, or `AV_PKT_DATA_ENCRYPTION_INIT_INFO` in `coded_side_data`.
   - A stream with no decoder → recorded as unsupported. If *no* A/V stream is decodable → `UnsupportedCodec { codec }` (the name comes from `avcodec_get_name`). If no A/V stream exists at all → `NoStreams`.
6. **Default streams.** Use `av_find_best_stream`, excluding attached pictures, and pick audio related to the chosen video.
7. **Read the first frame of each default stream**, with a dedicated decoder at `threads = 1`. The first frame gives:
   - the real output pixel format (alpha with `libvpx-vp9`);
   - size after codec cropping;
   - field flags (`AV_FRAME_FLAG_INTERLACED`, `AV_FRAME_FLAG_TOP_FIELD_FIRST`; FFmpeg 8.1 has removed the old `interlaced_frame`, `top_field_first` and `key_frame` fields);
   - color tags missing from the container;
   - **the frame display matrix** (EXIF orientation and H.264/HEVC display-orientation SEI);
   - the first presentable PTS;
   - for audio: the first PTS after priming is discarded, and the actual sample format and layout.

   If the first frame doesn't decode → `Corrupt`.
8. **Scan packets** of the video stream (up to 300 packets or 10 s), with every other stream discarded. Collect PTS, keyframe flags and durations. The results feed:
   - **VFR:** sort the PTS (packet order isn't presentation order when B-frames are present) and take the deltas in ticks. A stream is VFR if `(max − min) / median > 1 %` *and* `max − min > 2` timebase units (which absorbs the Matroska 1 ms quantization, L10), or if the avg and r rates disagree by more than 0.1 % after snapping.
   - **Nominal rate:** `av_guess_frame_rate` → snap to {23.976, 24, 25, 29.97, 30, 47.952, 48, 50, 59.94, 60, 100, 119.88, 120, 240} within 0.1 %. Otherwise keep the reduced rational. Ignore `r_frame_rate ≥ 1000` (MLT).
   - **GOP statistics** (`max_keyframe_interval`, `intra_only` from `AV_CODEC_PROP_INTRA_ONLY`).
9. **Durations** (PM-035): the media duration is the longer of the video end and the audio end, where video end = last PTS + its duration, from stream durations or the index. If `duration_estimation_method == AVFMT_DURATION_FROM_BITRATE` (raw field, read through `av`), set `duration_exact = false` and the app schedules `duration_scan` (AU-001). One-frame video: duration = one nominal frame.
10. **Rotation and mirroring.** Take the matrix from the stream `coded_side_data` (`AV_PKT_DATA_DISPLAYMATRIX`), else from the first frame (`AV_FRAME_DATA_DISPLAYMATRIX`). `rotation_cw = round(−av_display_rotation_get(m) / 90) mod 4`, treating NaN as 0 (libopenshot guards this). `mirrored = m[0]·m[4] − m[1]·m[3] < 0`. The convention was verified on FFmpeg 8.1: `av_display_rotation_set(90)` reads back as −90, and an EXIF orientation of 6 ("rotate 90° clockwise to display") reports −90. Non-quarter angles are rounded to the nearest quarter and a warning is logged.
11. **Alpha.** `AlphaKind::Native` when the decoded pixel format has an alpha component. `SideChannel` when Matroska `alpha_mode=1` (the stream metadata tag Shotcut checks). If `libvpx-vp9`/`libvpx` is missing, report `Unsupported`. HEVC alpha: v1, build-dependent → `Unsupported` with a message (OV-014).
12. **Color.** Store the raw tags. `color.rs` resolves the CL-002 defaults when frames are delivered, not at probe time, because the overrides can change.
13. **Stills.** A stream is a still when its codec is in {mjpeg, png, webp, bmp, tiff, gif, jpegxl, …} *and* (the demuxer is `image2`/`*_pipe` *or* `nb_frames ≤ 1`), as in libopenshot's `UpdateVideoInfo` heuristic. For stills, the authoritative decode path switches to the `image` crate (see [Stills](#stills-animated-images-and-image-sequences)).
14. **Fingerprint** = `{ size, head_tail, duration }`.

**Error mapping** (`probe/errors.rs`, kept as a pure function where possible):

| Condition | Detected by | `ProbeError` |
|---|---|---|
| file missing / deleted between pick and probe | `metadata` → `NotFound` | `NotFound` |
| permission | `io::ErrorKind::PermissionDenied` | `PermissionDenied` |
| 0 bytes | size | `Empty` |
| moov atom missing (recording not finalized) | mov/mp4 demuxer + `AVERROR_INVALIDDATA` + log line captured for this context ("moov atom not found") | `Incomplete` |
| interrupt fired by the deadline | `AVERROR_EXIT` with deadline passed | `Timeout` |
| encrypted | step 5 | `Protected` |
| no decoder | step 5 | `UnsupportedCodec { codec }` |
| other `AVERROR_INVALIDDATA`, first-frame failure | | `Corrupt { detail }` |
| other `io::Error` | | `Io` |

`av/log.rs` installs one global `av_log` callback. It formats with `av_log_format_line2`, passing the `va_list` through untouched, and routes by the `AVClass` context pointer: messages from a format context that a probe has registered are copied into that probe's diagnostics buffer. That is how "moov atom not found" is recognized without parsing global stderr. Everything else goes to `tracing` at a mapped level.

### Video decoding and accurate seeking

```rust
pub enum SeekMode { Exact, Keyframe } // Keyframe = EX-004 fast-drag approximation, filmstrip L0

pub enum DecoderProfile {
    Playback,   // frame+slice threads, hw allowed
    Scrub,      // slice threads only (frame threading adds a frame of delay per thread on every refill)
    Thumbnail,  // threads = 1, software only (don't consume scarce hw sessions)
    Export,     // max threads, hw per EX-018 setting
    Analysis,   // threads = 2
}

pub struct VideoDecoder {
    input: Input,                 // own format context, every other stream discard=All
    codec: decoder::Video,        // opened with libvpx-* when AlphaKind::SideChannel
    stream: usize,
    clock: StreamClock,
    gops: Arc<GopIndex>,          // keyframe table: AVIndexEntry (mp4/mov/mkv cues) or scanned + cached
    ring: FrameRing,              // last N decoded frames (N = 4, or 16 while blending)
    pos: DecoderPos,              // Unknown | At { last: Tick, eof: bool }
    hw: Option<HwSession>,
    profile: DecoderProfile,
    errors: ErrorWindow,          // consecutive decode errors, last-good frame for concealment
}

impl VideoDecoder {
    pub fn open(src: &ResolvedSource, stream: u32, profile: DecoderProfile,
                hw: &HwRegistry, cancel: &CancelToken) -> Result<Self, DecodeError>;
    pub fn frame_at(&mut self, t: Tick, mode: SeekMode, cancel: &CancelToken)
        -> Result<Arc<VideoFrame>, DecodeError>;
    pub fn position(&self) -> Option<Tick>;
}
```

**Planner** (`video/plan.rs`, pure, property-tested):

```rust
pub enum Plan { Hit, DecodeForward, SeekTo { keyframe: Tick } }

pub fn plan(pos: Option<Tick>, ring: RingSpan, t: Tick, gops: &GopIndex, max_forward: Tick) -> Plan {
    if ring.contains(t) { return Plan::Hit; }
    let k = gops.keyframe_at_or_before(t);           // None → seek to media start
    match pos {
        Some(p) if p <= t && (gops.no_keyframe_in(p, t) || t - p <= max_forward) => Plan::DecodeForward,
        _ => Plan::SeekTo { keyframe: k.unwrap_or(Tick::ZERO) },
    }
}
```

`max_forward` is 2 s of media, or 64 frames, whichever is smaller (from MLT's threshold and Olive's window). Without an index, the planner assumes a keyframe every `max_keyframe_interval`.

**The `frame_at` loop:**

1. `plan`. On `Hit`, return the frame from the ring.
2. On `SeekTo(k)`: `avformat_seek_file(fmt, stream, i64::MIN, ts(k), ts(k), 0)` on *this stream's* index, in the stream timebase (`av/seek.rs`). Don't use ffmpeg-next's `Input::seek`, which passes −1 and so uses AV_TIME_BASE (µs) units against the default stream. Then flush the codec and clear the ring.
   > **Revised (review, 2026-10-12; README §5.5).** This step used to set `skip_loop_filter = NonReference` after every seek and restore it near `t` (step 4). That is not exact: libavcodec applies the discard setting per packet when the packet is sent (frame threads copy it in `update_context_from_user`, called from `submit_packet`, `libavcodec/pthread_frame.c`), and packets go in ahead of the output by the reorder delay plus the frame-thread lag. The target packet, and with the Export profile up to about `thread_count` packets after it, would be decoded without deblocking. `skip_loop_filter` and `skip_frame` are now set only in `SeekMode::Approximate` and on the L0 filmstrip path (step 7), never in `Exact` mode or the `Export`/`Analysis` profiles. A later speed-up must decide per packet from `packet.pts` before `send_packet`, skipping only packets below `t − (reorder_depth + thread_count) × frame`.
3. Read packets with `Packet::read`, never `packets()`, which hides I/O errors as end of stream. Skip packets of other streams; with discard set, there are almost none. `send_packet`, then drain `receive_frame` until EAGAIN (L15).
4. For each frame:
   - `start = clock.media_time(best_effort_timestamp)`.
   - If start ≤ the previous start (non-monotonic PTS: AVI packed B-frames, broken muxers), repair it to previous + nominal duration and mark `TimestampQuality::Repaired` once.
   - Push the frame onto the ring with `end = next.start`. The end of the last frame is known from `frame.duration`, or else the nominal duration.
   - (Removed in review: no loop-filter toggling in `Exact` mode; see the note under step 2.)
   - Return when `start ≤ t < end` is decidable: if `start == t`, return at once. If `start + duration > t` and the duration is reliable (CFR, `frame.duration > 0`), return without peeking. Otherwise decode one more frame.
5. **Overshoot after a seek** (the first frame decoded has `start > t` and the seek target was > 0): step back to the previous keyframe (or 1 s, doubling each time) and repeat. Once at the start, return the first frame. This is clamping for `t <` the first frame (L4, Olive).
6. **EOF.** Send EOF and drain. If `t` ≥ the last frame's start, return the last frame (EX-004: "scrubbing past the end shows the last frame"). If the file ended earlier than the probe said (truncated, or still recording), set `quality = Truncated` and return the last frame.
7. **Keyframe mode** (`SeekMode::Approximate`). Seek as in step 2, set `skip_loop_filter = NonReference`, then return the first decoded frame whose start ≤ t (`FrameQuality::Approximate`). For L0 filmstrips, also set `skip_frame = NonKey`. This is not used for intra-only codecs, where every frame is a keyframe, so seeking per tile is cheaper.
8. **Errors.**
   - `AVERROR_INVALIDDATA` from send or receive increments `errors`. While under the tolerance (10, from MLT), keep reading.
   - If the target can't be produced but a good frame within one frame interval exists, return it with `FrameQuality::Concealed`. Otherwise return `DecodeError::Corrupt { at: t }`, and render draws the error slate (EX-005).
   - Errors are logged once per (source, range of 1 s).
   - If seeking fails twice for one source, reopen it, set `seekable = false` for the session and decode linearly from the start (libopenshot's fallback, but bounded per session, not forever).
9. **Cancellation.** Check the token between packets. FFmpeg's internal reads poll it through the interrupt callback. A cancelled decoder returns `DecodeError::Cancelled`, and its position becomes `Unknown`.

**Deinterlacing (v1).** A `bwdif=mode=send_frame:deint=interlaced` graph (`av/filter.rs`) runs after decode for frames flagged interlaced, or when the interpretation forces a field order. Two pitfalls:
- bwdif needs the *next* frame to output the current one, so the loop must decode one frame past the target, and flush the graph at EOF.
- The graph is reset after every seek, so the first frame after a seek has less temporal context. That is acceptable.

The deinterlace filter isn't applied to proxies that are flagged progressive (see [Proxies](#proxies-pm-080-pm-081)). MLT doesn't use its GPU scale filter on interlaced input for the same reason: the field structure has to survive until deinterlacing.

### Frames and pixel formats

Render converts YUV to RGB in its shader (CL-002, FX-001 L2). Media therefore hands over planes in a **small closed set of layouts** that render supports natively, and converts the long tail of formats once on the CPU, keeping matrix and range unchanged.

```rust
pub enum PixelLayout {
    Yuv420P8, Yuv422P8, Yuv444P8,                 // 3 planes u8
    Nv12,                                          // hw output (VT/D3D11/VAAPI)
    Yuv420P16 { bits: u8 }, Yuv422P16 { bits: u8 }, Yuv444P16 { bits: u8 }, // LSB-aligned (yuv4xxp10/12)
    P016 { bits: u8 },                             // P010/P012/P016: semi-planar, MSB-aligned
    Yuva420P8, Yuva444P16 { bits: u8 },            // + alpha plane (VP9 alpha, ProRes 4444)
    Rgba8, Rgba16,                                 // straight alpha, sRGB-encoded (PNG, qtrle, gbrap→rgba)
}

pub struct VideoFrame {
    pub start: Tick,
    pub duration: Tick,
    pub layout: PixelLayout,
    pub size: (u32, u32),            // decoded (after codec crop), before geometry
    pub planes: Planes,              // Arc'd AVFrame ref + plane views (stride, rows); no copy (L14)
    pub geometry: DisplayGeometry,   // rotation_cw, mirrored, sar, display_size (from original + interp)
    pub color: ColorInfo,            // resolved: tags → CL-002 defaults → PM-040 overrides
    pub alpha: AlphaMode,            // Straight | Premultiplied | Ignore (PM-040); render premultiplies
    pub field: FieldInfo,
    pub quality: FrameQuality,       // Exact | Keyframe | Concealed | Truncated
    pub variant: SourceVariant,      // Original | Proxy → "Proxy" tag in the player (PM-081)
}
```

**Mapping rules (`video/layout.rs`):**
- The `yuvj*` formats are treated as their `yuv*` equivalents, with full range (the deprecated "JPEG" formats, as in Olive's `ConvertJPEGSpaceToRegularSpace`).
- `gray` and `gray10` → `Yuv444P*` with neutral chroma, so the matrix path stays uniform.
- `pal8`, `rgb24`, `bgr0`, `gbrp*` → `Rgba8` or `Rgba16`.
- `yuyv422`, `uyvy422`, `v210` → `Yuv422P8` or `Yuv422P16`.
- `yuv410p` and `yuv411p` → `Yuv420P8` (chroma resampled).
- `xyz12` → `Rgba16`.
- Any format FFmpeg itself flags `AV_PIX_FMT_FLAG_HWACCEL` is never seen here; it is transferred first.

**CPU conversion (`av/sws.rs`).** Use FFmpeg 8's **dynamic** API: `sws_alloc_context` + `sws_scale_frame`. Every property (size, format, colorspace, range, primaries, transfer, chroma location) comes from the `AVFrame` itself, and the context reconfigures on its own when they change. On the dev build (FFmpeg 8.1.1, swscale 9.5.101) this was checked with a C test program: PQ and HLG input transfers and BT.2020/P3 primaries are supported, with `SwsContext.intent` (perceptual, relative colorimetric, …) controlling gamut and tone mapping. `sws_isSupportedInput(AV_PIX_FMT_VIDEOTOOLBOX) == 0`, so hardware frames must be transferred first.

ffmpeg-next's own `software::scaling::Context` uses the legacy `sws_getContext` / `sws_scale` path, ignores `sws_scale`'s return value and can't set colorspace details. Without colorspace details it falls into libopenshot's BT.601 bug (L13), so FreeCut doesn't use it.

Flags follow MLT's `mlt_get_sws_flags`:
- format-only conversions: `SWS_POINT | SWS_ACCURATE_RND` (plus `FULL_CHR_H_INT` for YUV→RGB);
- downscales for thumbnails: `SWS_AREA` (filmstrip) or `SWS_BICUBIC` (poster).

```rust
/// CPU path for thumbnails, hover frames and AI: applies geometry and color, outputs sRGB RGBA8.
pub fn to_rgba(frame: &VideoFrame, max: Size, intent: ToneMapIntent) -> Result<RgbaImage, DecodeError>;
```

`to_rgba` sets the *source* frame's color fields to the resolved `ColorInfo`, because CL-002 defaults have to beat UNSPECIFIED. The destination is RGBA, BT.709 primaries and sRGB transfer. Rotation and mirroring are then applied to the small result (cheap). HDR sources use `intent = Perceptual`. That differs slightly from CL-002's filmic shader curve; see the open questions.

> **Revised (review, 2026-10-12; README §5.4 item 8).** Using swscale's perceptual intent would make thumbnails and filmstrips disagree with the viewer and the export. For PQ/HLG sources swscale now does only the matrix and chroma step (destination primaries and transfer = the source's, RGB48), and the pure-Rust `freecut_media::color::hdr_to_sdr` (EOTF → /203 cd/m² → BT.2390 EETF → 2020→709 soft clip → γ 2.4), which is also the shader golden's CPU reference, does the rest. The `intent` parameter of `to_rgba` is dropped.

> **Implementation note (2026-10-12, CL-002).** Built in `freecut_media::color` (pure Rust, always compiled) and used by `av::sws::Scaler`, so `VideoDecoder::frame_at`/`frame_at_size` and posters all return SDR Rec.709 for HDR sources. A frame whose transfer is PQ (16) or HLG (18), or whose primaries are BT.2020 (9), is converted by swscale to RGBA64LE in its own transfer and primaries, then `ToneMapper` (lookup tables matching `color::hdr_to_sdr` within one code value, rows spread over up to 8 threads, deterministic) does: EOTF (PQ; HLG inverse OETF + BT.2100 OOTF at 1000 cd/m², γ 1.2) → BT.2020→709 matrix with soft gamut compression (per-channel distance curve, threshold 0.8, limits at the 2020 primaries) → BT.2390 EETF on max(R,G,B) with light ÷ 203 cd/m² (target peak = reference white), hue kept → γ 1/2.4. Gamut comes before the tone map because it is scale-invariant and the tone map on max(R,G,B) then keeps every channel ≤ 100 %. FreeCut defaults: HLG and untagged PQ peak 1000 cd/m²; PQ peak from MaxCLL, else the mastering display's max luminance, clamped to 203–10 000. Results: HLG 75 % (reference white) → code 230 (90 %), PQ 203 cd/m² at a 1000 cd/m² peak → the same, about 83 % at 4000 cd/m²; SDR BT.2020 sources get the gamut step only (γ 2.4 in and out, neutrals unchanged). Not built yet: the one-time "converted to SDR" notice (app) and CL-041's per-media options.

### Hardware decode (EX-018)

```rust
pub enum HwDecodePolicy { Off, Auto }
pub enum HwKind { VideoToolbox, D3d11va, Cuda, Vaapi, Qsv }

pub struct HwRegistry {               // one per process, shared via Arc
    devices: Mutex<HashMap<HwKind, HwDeviceState>>, // Untried | Ready(HwDevice) | Failed(reason)
    sessions: AtomicU32,               // active hw decoders, capped (excess → software)
}
```

**Order per platform:**
- macOS: VideoToolbox.
- Windows: D3D11VA, then CUDA when an NVIDIA adapter is present.
- Linux: VAAPI, then CUDA.
- QSV only when explicitly detected.

A device (`av_hwdevice_ctx_create`) is created lazily, once per kind. If creation fails, the kind is marked `Failed` for the session.

**Open:**
1. Walk `avcodec_get_hw_config(codec, i)` for a `AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX` entry whose `device_type` matches (MLT's loop).
2. Set `hw_device_ctx = av_buffer_ref(device)`.
3. Set `extra_hw_frames = ring_len + 2`. Hardware frame pools are fixed-size, and holding decoded surfaces starves the decoder. Frames are also transferred immediately, so references are short-lived.
4. Point `opaque` at a boxed `HwWanted { pix_fmt }` owned by the decoder, and set `get_format`. Never use globals (L11).

`get_format` (`av/hw.rs`): return `wanted.pix_fmt` if it is in the offered list; otherwise return the first offered format whose descriptor *lacks* `AV_PIX_FMT_FLAG_HWACCEL`. libavcodec calls `get_format` again when a hardware accelerator fails to initialize for a profile (H.264 4:2:2 10-bit on VideoToolbox, for example), so falling back here gives the "that stream silently uses software" behaviour EX-018 asks for.

**Transfer.** `av_hwframe_transfer_data` → `av_frame_copy_props` → `av_frame_apply_cropping(AV_FRAME_CROP_UNALIGNED)`. libavcodec's `apply_cropping` can't crop top/left on opaque hardware frames (FFmpeg 8.1 `avcodec.h`). A golden test compares the size and checksum of the hardware frame with the software frame for 1088-coded H.264 and D3D11 HEVC (MLT's padding issue, L12).

**Failure detector.** If 3 or more `INVALIDDATA` errors happen before the first good frame, or a transfer error or device loss happens at any time, the decoder reopens *in software at the same position*: the planner's next call seeks. The source and codec are marked `hw_failed` for the session, and the app gets `MediaEvent::HwFallback` so it can show the one-time toast.

**Equality test (I9).** For each fixture codec, decode N frames with hardware and with software and compare the planes. 8-bit H.264 and HEVC must match exactly (decoding is bit-exact by spec). Others get a PSNR floor.

**Not in v1:** zero-copy into wgpu (EX-018 says "later"). MLT-style GPU scale filters are also out, because render scales anyway.

> **Revised (review, 2026-10-12; README §7 M3a, §6.1).** This path (`get_format` with software fallback, transfer to NV12/P010) is built in M3a for the **preview** profiles behind `HwDecodePolicy::Auto`, although EX-018 is tagged v1: 4K HEVC Main10 phone footage will not play at 1× with software decode on 8 GB laptops, and preview scale does not reduce decode cost. Export keeps software decode by default. Whether EX-018 becomes mvp is raised with its owner.

### Decoder pool, read-ahead and threading

```rust
pub struct DecoderKey { pub source: SourceId, pub stream: u32, pub variant: SourceVariant }

pub struct DecoderPool { state: Mutex<PoolState>, cfg: PoolConfig, hw: Arc<HwRegistry> }

pub struct PoolConfig {
    pub max_decoders: usize,       // default max(4, workers + 2 × visible_tracks) (Shotcut/Kdenlive formula)
    pub max_bytes: u64,            // estimated: w·h·bpp·(refs + threads + ring), default 2 GiB (revised: set by MemoryBudget, README §5.12)
    pub per_source: usize,         // 3: same media used on several tracks at once
    pub idle_close: Duration,      // 10 s (Olive closes after 5 s idle)
}

impl DecoderPool {
    /// Exclusive lease, best positioned decoder for `near`. Blocks (with cancel) if at capacity.
    pub fn lease(&self, key: &DecoderKey, near: Tick, profile: DecoderProfile,
                 cancel: &CancelToken) -> Result<Lease<'_>, DecodeError>;
    pub fn invalidate_source(&self, source: SourceId); // offline / changed / relinked / proxy switch
}
pub struct Lease<'p> { /* Deref<Target = VideoDecoder>; returns to pool on Drop */ }
```

**Selection.** Among the idle decoders for a key, take the one with the lowest `cost(pos, near)`:
- 0 when the target is in its ring;
- the forward distance when there is no keyframe between `pos` and `near`;
- otherwise a constant seek cost.

If every candidate is busy, or the best cost is a seek while fewer than `per_source` decoders exist, open a new one, as long as the global count and byte budget allow. Otherwise evict the least-recently-used idle decoder of *another* source. Failing that, wait. This generalizes Kdenlive's per-track decoders (L22) without hard-wiring tracks.

**Read-ahead (EX-002).** `prefetch.rs` runs one worker per active (source, lane) during playback:
- The app gives it a schedule: the next source times from TL's map, produced by render or the app each time the document version changes.
- The worker holds a lease and decodes ahead into a bounded queue (8 frames, or 0.5 s).
- Each scheduled item carries a `generation: u64`. A seek, a document edit or a cancel bumps the generation, and stale frames are dropped (EX-002 "read-ahead after the playhead is discarded").
- Render's request first checks the prefetch queue, then leases a decoder itself.

**Priorities and throttling (PM-073).**
- A process-wide `DecodeGate` semaphore gives Playback/Scrub requests absolute priority.
- Thumbnail, filmstrip, peak and proxy jobs acquire a background permit that is withheld while the transport is playing.
- Proxy encoding also pauses during export.

**Threading model:**
- The UI thread calls nothing blocking here (I10). It sends requests over channels (app) and receives results.
- Render worker threads and prefetch workers lease decoders and call `frame_at`.
- Audio prefetch threads own `AudioReader`s (below).
- The app's CPU and I/O job pools call the job functions.
- `VideoDecoder: Send + !Sync`.
- `Arc<VideoFrame>: Send + Sync`. ffmpeg-next 9 already implements `Send + Sync` for `Frame`, and a published frame is never written again.
- `HwRegistry: Send + Sync`. Hardware device contexts are thread-safe for decoding; D3D11VA installs its own lock through `AVD3D11VADeviceContext.lock`.
- The `av/sws.rs` context is `Send`, but each worker uses its own.
- FFmpeg ≥ 4 needs no global lock around `avcodec_open2`, so MLT's `open_mutex` isn't needed.

**Offline sources.** If opening fails with `NotFound` (or a volume error), the pool marks the source offline. Leases then fail fast with `DecodeError::Offline` until `invalidate_source` is called. The app calls it when the watcher (PM-093) or a status check (PM-090) changes the source's state, so the pool never retries on every frame.

### Reverse decoding (EX-007, TR-043)

`ReverseReader` serves descending targets:
1. For the GOP containing `t`, starting at keyframe `k`, decode forward from `k` up to `t` into a reverse cache (budget 512 MB, EX-007). Serve frames from the cache in descending order.
2. While serving, a second lease on the same source (the pool allows 3) decodes the previous GOP `[k_prev, k)` into a staging buffer. This is double buffering.
3. If a GOP is larger than the budget, decode it as sliding windows: decode from `k` and keep only the last W frames that fit before the target, then repeat for the next window down. This costs O(GOP²/W) decode work and is bounded. The app suggests proxies for such files, and EX-007 already expects stutter for them.

**Export** uses the same reader with the export profile and never depends on the preview cache (TR-043).

### Stills, animated images and image sequences

**Stills (PM-034).**
- Decode with the `image` crate: JPEG (zune-jpeg 0.5; `image` requests RGB output, so zune-jpeg converts CMYK/YCCK, and the CMYK fixture checks this), PNG (8/16-bit), WebP, GIF, BMP, TIFF, TGA and QOI. This matches the references: Olive decodes stills through OIIO (`olive/app/codec/decoder.cpp: ReceiveListOfAllDecoders`), and MLT/Kdenlive/Shotcut use `qimage`/`pixbuf` producers. It also keeps the most common untrusted input inside memory-safe Rust.
- Orientation comes from `ImageDecoder::orientation()` and is applied with `DynamicImage::apply_orientation` (image 0.25.10).
- HEIC/HEIF, AVIF (unless the `image` crate's AVIF path is enabled), JPEG XL, EXR and DPX go through FFmpeg, if the build has those decoders. There, orientation comes from the frame display matrix (verified for MJPEG in FFmpeg 8.1).
- A decoded still is downscaled with `fast_image_resize` to min(2 × canvas long side, 8192) on its long side. It is kept in a RAM `ImageCache` (LRU by bytes, default 512 MB; revised in review: the cap comes from the app's `MemoryBudget`, README §5.12) as `Rgba8`/`Rgba16` frames with full range and sRGB. In the mvp, 16-bit is reduced to 8-bit (PM-034).

**Animated (OV-012, v1).**
- The `image` crate's `AnimationDecoder` handles GIF, APNG and WebP, and composites frames, including disposal.
- Each frame gets a rational start and duration. Delays of 0 or ≤ 10 ms become 100 ms (OV spec).
- Frames decode by index into a per-asset LRU (256 MB).
- If trailing frames are corrupt, play up to the last good frame and warn.

**Image sequences (PM-033).**
- `detect_sequences(&[PathBuf]) -> Vec<SequenceCandidate>` is pure:
  - The frame number is the *last* numeric run before the extension.
  - Group by (dir, prefix, suffix, ext); the comparison is case-insensitive on Windows and macOS.
  - Width may vary (`f9`, `f10`).
  - At least 3 frames.
  - `missing: Vec<u32>` lists the gaps.
  - Unlike Olive's walk, which stops at the first gap, this works from a directory listing, so no stat call is needed per candidate.
- `SequenceReader` decodes frame files *in parallel*. They are independent, which is a big win for PNG/EXR sequences.
  - It prefetches the next N frames in playback direction and keeps an LRU.
  - Frame i starts at `i × rate.frame_duration()`, which is exact because TL rates divide the tick rate.
  - A missing frame holds the previous one (`MissingFramePolicy::Hold`, the OTIO term).
  - Frames of a different size are fitted to the first frame's size inside the reader (PM-033), so consumers always see one size.

### Audio decoding

```rust
pub struct AudioReader {
    input: Input, codec: decoder::Audio, stream: usize, clock: StreamClock,
    rate: u32, layout: ChannelLayout,   // native; ch_layout (FFmpeg ≥ 7 API), unspecified order → default layout (LFE kept, AU-010)
    next: Option<i64>,                  // next media sample index this reader will produce
    carry: PlanarF32,                   // decoded but not yet consumed samples
}

pub struct AudioReadReport { pub silent_ranges: SmallVec<[(i64, i64); 2]>, pub concealed: bool }

impl AudioReader {
    /// Fills `out` (n samples, native rate, f32 planar) starting at media sample `start`.
    /// Silence before the stream start, after its end, across gaps and undecodable packets.
    pub fn read(&mut self, start: i64, out: &mut PlanarF32, cancel: &CancelToken)
        -> Result<AudioReadReport, DecodeError>;
}
```

**Alignment (replaces MLT's double-based ahead/behind logic):**
1. If `start != next`, seek to `start − preroll`, where preroll = max(frame_size × 2, 100 ms). That gives codecs with overlap (AAC, Opus pre-skip) clean output. Flush.
2. For every decoded frame: `first = sample_at(clock.media_time(pts), rate)`. Within a contiguous run, trust sample counting. Re-anchor to the PTS only when |pts-derived − counted| > max(20 ms, 2 frames), since TS/AAC timestamps jitter. On re-anchoring:
   - a positive difference inserts silence (a real gap);
   - a negative one drops the overlap.

   Log once.
3. Copy `[start, start+n)` out of `carry`. Samples before media 0 (audio that begins before the first video frame) are never returned.
4. Convert the sample format to f32 planar in `audio/convert.rs`: u8, s16, s32, s64, flt and dbl, planar or interleaved, in plain safe Rust. NaN and ±Inf from corrupt float streams become 0. swr isn't needed for this. Downmixing and channel maps are AU's job (chain step 2).
5. Priming and edit lists: keep libavcodec's default skip-samples handling (do *not* set `AV_CODEC_FLAG2_SKIP_MANUAL`) and the mov demuxer's default advanced edit lists. With the FFmpeg 8.1 fixture, the priming packet (`pts=-1024`, flag `D`) produced no output, and the first frame was at `pts=0`.

**Prefetch (AU-010).** `audio::prefetch` gives one thread per active (source, stream, lane). It reads forward (or backward, for reverse) in source time and pushes `AudioBlock { start_sample, generation, data }` into an `rtrb` SPSC ring. The real-time mixer callback only pops blocks. A seek or scrub bumps the generation and the ring is drained. Resampling, time-stretch and reverse ordering happen in freecut-audio, which reads blocks in source time.

**Optional conform cache.** This keeps Olive's good idea without its "no sound until done" flaw. A background job writes `f32` planar PCM at the *native* rate to the cache for three cases:
- sources whose seeking proved unreliable (VBR MP3 without a TOC, raw ADTS AAC, some Ogg);
- reversed clips (TR-043);
- scrubbing (AU-016).

The key is (fingerprint, stream, format version). Once the conform exists, `AudioReader` serves reads from a `memmap2` mapping. Until then, live decode serves them.

> **Revised (review, 2026-10-12; README C11, §4.4).** (1) The first case is no longer v1: in M2 the per-stream audio pass (which already decodes these streams end to end for peaks and the exact length) also writes the native-rate PCM for streams the probe flags as seek-unreliable (MP3 without a Xing/VBRI TOC, raw ADTS AAC, Ogg without an index), and reads of those streams are served from it. Until it is ready, such a stream is **never seeked**: the reader decodes forward from 0 and counts samples, because the post-seek PTS of these demuxers is an estimate and the re-anchor check in step 2 above has nothing to compare against (music would be offset by tens to hundreds of ms after every seek, scrub, split or play-from-middle). In playback that shows as `NotReady` and the clock holds (README §2.5). Reverse and scrub conform stay v1. (2) `memmap2` is dropped: `Mmap::map` is `unsafe` outside the allowed modules. Conform files are read with positioned reads (`FileExt::read_at` on Unix, `FileExt::seek_read` on Windows), and peak pyramids are read into memory.

**Exact duration scan (AU-001).** `duration_scan(src, stream)` decodes the whole stream, with every other stream discarded. It sends EOF and drains (L16). It returns the exact sample count. The app commits TL's `CorrectMediaLength` op: the new length and the clamp of every clip past it, as **one undo entry** (revised in review; a non-undoable clamp after open would make a later undo diverge, README §5.11).

**Resampling for analysis.** `audio::resample::Resampler` wraps rubato's synchronous sinc resampler for offline work: 16 kHz mono (AI-007 and others), 44.1 kHz and 48 kHz. Streaming variable-ratio resampling for speed curves is freecut-audio's.

### Derived-data cache (PM-074)

Root: the platform cache directory from PM (`~/Library/Caches/FreeCut`, …), versioned as `v1/`:

```
<root>/v1/
  thumbs/<fp:2>/<fp>/poster-s<stream>-<interp>-h<h>-t<pts>.jpg|png
  thumbs/<fp:2>/<fp>/hover-s<stream>-<interp>.fcs        (16 tiles, PM-060)
  strips/<fp:2>/<fp>/s<stream>-<interp>-h<h>/L<level>/<bucket>.fcs
  peaks/<fp:2>/<fp>/s<stream>.fcp
  index/<fp:2>/<fp>/s<stream>.gop                         (keyframe table for formats without one)
  audio/<fp:2>/<fp>/s<stream>.f32                         (conform cache)
  proxies/<fp>-<short>p.v<N>.mp4|.mov                     (default proxy location)
  analysis/...                                            (AI-001 owns keys)
  tmp/                                                    (temp files; cleaned on startup)
```

- `<fp>` is the hex of xxh3-64 over (size, head_tail, duration ticks). `<interp>` is a short hash of the interpretation fields that change pixels: rotation, mirror, PAR, alpha mode, color override, field order. Timing overrides are **excluded**, because tiles are keyed by native PTS, not media time (L18). Changing a frame-rate override therefore never invalidates thumbnails or peaks.
- **Atomic publish.** Write into `tmp/<random>`, `fsync`, then `rename` into place, using a no-replace rename where the OS supports one; when the target already exists, keep the existing file, since both copies have the same content. Two processes can race safely. Readers verify the header magic, version and trailing xxh3. If a check fails, they delete the file and regenerate it (PM-070 "corrupt entry silently regenerated").
- **Eviction.**
  - An in-memory index is built by a background directory scan at startup (size, mtime).
  - A cache hit touches the mtime, at most once an hour per file (Shotcut uses `utime` on proxies). atime isn't used because volumes are often mounted noatime or relatime.
  - Eviction is LRU by mtime, per category, under the limits: total 20 GB, thumbnail sub-limit 2 GB, proxies evicted last.
  - The app passes a pin set (the fingerprints visible in the open project).
  - When the disk is nearly full, the effective limit shrinks (PM-074).
- `tmp/` files older than one hour are deleted at startup. That covers crashed jobs, as Shotcut's `removePending` does.

```rust
pub struct CacheStore { root: PathBuf, limits: CacheLimits, index: Mutex<LruIndex> }
impl CacheStore {
    pub fn path_for(&self, key: &CacheKey) -> PathBuf;
    pub fn get(&self, key: &CacheKey) -> Result<Option<CachedFile>, CacheError>;   // validated
    pub fn put_atomic(&self, key: &CacheKey,
                      write: impl FnOnce(&mut BufWriter<File>) -> io::Result<()>) -> Result<(), CacheError>;
    pub fn evict(&self, pinned: &HashSet<FingerprintKey>) -> Result<EvictReport, CacheError>;
}
```

### Thumbnails and filmstrips

**Poster (PM-070).**
- Default time = min(1 s, 10 % of duration), or the poster frame set by PM-057.
- Exact seek with `DecoderProfile::Thumbnail`, then `to_rgba` at ≤ 256 px tall with geometry applied.
- Encode as JPEG (quality 85); PNG when the frame has alpha.
- Fallbacks: if decoding fails, try t = 0, then return `ThumbError::Unavailable`, and the app shows the type icon.
- Audio-only files with cover art use the attached picture.

**Hover frames (PM-060).** 16 evenly spaced frames, generated by the poster job right after the poster, at 160 px tall, in one `.fcs` chunk. Hovering only reads cached tiles.

**Filmstrip chunks (PM-071, TL-042).**

| Level | Spacing | Decode | Chunk span |
|---|---|---|---|
| L0 | one tile per 2 s | `Keyframe` mode (`skip_frame = NonKey`) for long-GOP sources, exact seek per tile for intra-only ones | 128 tiles (256 s) |
| L1 | one tile per 0.5 s | exact, sequential within the chunk | 64 tiles (32 s) |
| L2 | every source frame | exact, sequential | 2 s of media |

- A chunk is generated *as a unit* by one job, from one sequential decode, which is the cheapest access pattern. It is immutable once published.
- Each tile records its **actual** native PTS and media time. A keyframe tile can be up to a GOP away from its grid point, and TL-042's "nearest frame already generated" logic needs the true time.
- L0 for the whole file is built visible range first; for a 3-hour file that is 5,400 tiles in about 43 chunks. L1 and L2 are built only for visible ranges. Their requests carry a generation, and the app cancels requests for chunks that scroll out of view.
- When a Ready proxy exists, tiles are decoded from the proxy (PM-071). They are still keyed by the *original's* fingerprint and native PTS, which is valid because proxy timestamps equal the original's (verified when the proxy is accepted).
- Height is 64 px by default. The app requests other heights by key.

`.fcs` file layout, little-endian, versioned:

```
magic "FCST" | u16 version | u16 flags | u64 fp | u32 stream | u64 interp | u16 height | u8 level | u32 bucket
u32 n | n × { i64 native_pts, i64 media_ticks, u16 w, u16 h, u8 codec(JPEG|PNG), u32 offset, u32 len }
blobs... | u64 xxh3 of everything above
```

### Waveform peaks (PM-072, AU-009)

Job: `peaks::build(src, stream, cancel, progress) -> PeakPyramid`.
- Decode sequentially with every other stream discarded (Kdenlive's fast path). Send EOF and drain (L16).
- Align by timestamps exactly like `AudioReader`, so gaps become zero-bins and the A/V offset is kept.
- Level 0 has 256 samples per bin; each level above aggregates 4 bins of the level below (`min` of mins, `max` of maxes), up to the level where the whole stream is one bin.
- Values are per channel, computed in f32 and quantized to i16 Q15 with saturation. min and max are kept separately (L17).
- Bins are indexed by the stream's own sample index from media sample 0. Samples before 0 are excluded, and `first_sample_offset` is recorded.
- Partial pyramids go to the app through the progress callback every ~0.5 s of work, which gives the "progressive, show what's ready" behaviour for 10-hour files. Only the complete pyramid is published to disk (atomic). A cancelled build leaves nothing, and the next request restarts it.
- Pure query helper, used by the app's drawing and by AU-009's speed mapping:
  ```rust
  /// Coarsest level with at least 2 bins per pixel column (PM-072 tile rule; AU-009 never undersamples).
  pub fn level_for(samples_per_px: u64) -> u8;
  ```

`.fcp` layout:
```
magic "FCPK" | u16 version | u16 flags(COMPLETE) | u64 fp | u32 stream | u32 rate | u16 channels
u16 bin0 (=256) | u8 fan (=4) | u8 levels | i64 first_sample_offset | u64 total_samples
per level: u64 bins, then bins × channels × (i16 min, i16 max)
u64 xxh3 trailer
```

Size: at 48 kHz, level 0 is 187.5 bins/s × 4 B ≈ 2.7 MB per channel-hour, plus a third of that for the upper levels.

### Proxies (PM-080, PM-081)

**Format.**

| Source | Proxy |
|---|---|
| opaque video | H.264 High, 8-bit 4:2:0, libx264 `preset=veryfast crf=23 g=10 bf=0`, MP4 (spec). Preference "smoothest scrubbing" → `g=1` (Shotcut's choice). |
| alpha video (`AlphaKind::Native/SideChannel`) | ProRes 4444 (`prores_ks`, intra, FFmpeg-native, fast decode) in MOV. VP9 alpha via libvpx is smaller but inter-frame and needs libvpx to decode (open question). |
| HDR / 10-bit | 8-bit H.264 with the source's transfer/primaries tags (banding is acceptable for preview) |
| interlaced | deinterlaced with bwdif at frame rate, marked progressive in proxy metadata |
| images | none (PM-034's downscale covers them) |
| image sequences | proxied as video, same per-frame timestamps |

**Geometry (tension 2).**
- The proxy scales the *coded* picture uniformly (even dimensions) so its short display side is ≤ the chosen height, and keeps the source SAR.
- No rotation is applied (`autorotate` off, as with `-noautorotate` in the references), and the display matrix is not copied.
- Interpretation and geometry are applied at decode from the original's info.

**Color.** Write explicit primaries, transfer, matrix and range tags (Shotcut). Full-range sources stay full range. The decoder still resolves color from the original (L21). Hardware encoders are optional. VideoToolbox H.264/HEVC isn't used for full-range input (Shotcut excludes it on Apple Silicon).

**Timing — "proxy frame N is original frame N".**
- Each decoded frame's PTS is copied into the encoder unchanged, with no `fps` filter: passthrough, VFR kept.
- The output stream timebase equals the input's. The mov muxer gets `video_track_timescale = tb.den` so the MP4 track keeps it.
- `bf=0`, so there is no B-frame delay, no negative DTS and no edit list in the proxy.
- If the original is VFR, the proxy is VFR with the same PTS.
- The audio of the original stays authoritative. As a recommended addition (see the open questions), a mono AAC 96 kb/s fallback track is written but used only when the original is offline. PM-080 currently says video-only.

**Generation job (`proxy::encode`).**
1. Decode with the Export profile; hardware is allowed.
2. Scale through `av/sws.rs` (or the bwdif graph), encode, and mux to `<root>/v1/tmp/<random>.mp4`. Progress comes from the PTS against the duration.
3. Check free space first and abort cleanly when the disk is full (`ProxyError::DiskFull`).
4. **Verify** (`proxy::verify`): probe the result and compare frame count (±0), first PTS, last PTS and duration with the source, plus sampled PTS equality at 32 points. A mismatch → `ProxyError::TimingMismatch`, and the file is deleted. If the source was truncated, the proxy covers what decoded and gets `flagged_truncated` (PM-080).
5. Embed metadata tags: `freecut_proxy=1; v=<format version>; src=<fp>; h=<height>; progressive=<bool>`.
6. Rename atomically to `proxies/<fp>-<short>p.v<N>.<ext>`, or to the project's `<Name> Proxies/` folder when PM-080 storage is "with project".

**Lookup (`proxy::find_proxy(fp, height)`).**
- Check both storage locations by naming convention.
- Read the embedded tags and require a matching `fp` and version. A mismatch means stale, regenerate.
- No project-file state is kept, as PM-080 requires.

**Selection (decoder factory, PM-081).** `ResolvedSource` picks `Proxy` when the toggle is on and a proxy is Ready, or when the original is offline and a proxy exists ("Original offline" badge). Export always resolves `Original` and refuses offline sources unless the user chooses "Export anyway" (PM-090).

**Process isolation.** Proxy jobs are long and CPU-heavy, and they decode arbitrary files, so they are the easiest place to isolate crashes. The job function is a plain library call. The app may run it in a child process (`freecut --media-worker proxy …`) and kill it to cancel, like the ffmpeg subprocesses Shotcut and Kdenlive use. This is a recommendation; see the open questions.

**Camera proxies (v1+).** GoPro `GL*.LRV` and DJI `*.LRF` (Shotcut) are used only if `proxy::verify` passes against the original. Kdenlive's FX6 case shows camera proxies can differ in streams.

### Status, relink and change detection

- `status::check(path, fp, timeout) -> MediaStatus`: `metadata()` with a per-volume timeout, run on the I/O pool.
  - `NotFound` → `Offline`.
  - A missing mount root (`/Volumes/X`, a drive letter) → `VolumeUnavailable`.
  - `PermissionDenied`.
  - A size or mtime difference → recompute `head_tail` → `Changed` only if the fingerprint differs (PM-093).
- `status::validate_candidate(old: &MediaInfo, path) -> Match`:
  - probe the candidate;
  - an exact fingerprint match → `Exact`;
  - same kind, duration within 1 nominal frame (or 100 ms for audio-only), same display size after geometry, same channel count → `MetadataMatch`;
  - else `Mismatch { diffs }` (PM-091).

### Errors

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("file not found")] NotFound,
    #[error("permission denied")] PermissionDenied,
    #[error("file is empty")] Empty,
    #[error("no video or audio streams")] NoStreams,
    #[error("unsupported codec {codec}")] UnsupportedCodec { codec: String },
    #[error("damaged file: {detail}")] Corrupt { detail: String },
    #[error("recording looks unfinished")] Incomplete,
    #[error("copy-protected")] Protected,
    #[error("timed out reading the file")] Timeout,
    #[error("not a media file ({kind:?})")] NotMedia { kind: NotMediaKind }, // proposed addition (PM-036 hints)
    #[error(transparent)] Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("media offline")] Offline,
    #[error("cancelled")] Cancelled,
    #[error("timed out")] Timeout,
    #[error("frame at {at:?} could not be decoded")] Corrupt { at: Tick },
    #[error("hardware decoder failed")] HwFailed,            // internal; triggers software reopen
    #[error("{op} failed: {msg} ({code})")] Ffmpeg { op: &'static str, code: i32, msg: String },
    #[error(transparent)] Io(#[from] std::io::Error),
}
// CacheError, ProxyError { DiskFull, TimingMismatch, Encoder(..), Cancelled, .. }, ThumbError similarly.
```

The app maps these to the user-facing text (PM-037 table). Library code never shows UI and never uses `anyhow`.

### The `unsafe` boundary (`freecut_media::av`)

Everything that ffmpeg-next 9.0 doesn't wrap goes through `ffmpeg_next::ffi` (`ffmpeg-sys-next` 9.0.1). Each item has a small safe wrapper and a `// SAFETY:` comment stating its obligations:

| Item | Why it needs FFI | Main safety obligation |
|---|---|---|
| `seek_stream` | per-stream `avformat_seek_file` | context is exclusively borrowed; stream index valid |
| `index_entries` | `avformat_index_get_entry` | returned pointer is only valid until the next call on the context → copied into `GopIndex` immediately |
| display matrix, encryption, `alpha_mode` | raw side-data parsing | length checked (≥ 9 × i32) before reading |
| `duration_estimation_method` | not exposed | read-only field access |
| hw device, `get_format`, `opaque` | no hwaccel API in ffmpeg-next | `opaque` points to a heap `HwWanted` that outlives the codec context (dropped after `avcodec_free_context`) |
| hw transfer + `av_frame_apply_cropping` | not exposed | destination frame freshly allocated; source is a hw frame of this decoder |
| dynamic `sws_scale_frame` | legacy wrapper unusable | frames are valid and not aliased; context used by one thread |
| `av_log` callback | global hook | `va_list` passed through untouched to `av_log_format_line2`; callback never unwinds (catch_unwind → abort, as ffmpeg-next's interrupt shim does) |
| bwdif graph (v1) | filter graph | standard buffersrc/buffersink lifetimes |

Startup checks (`av/caps.rs`):
- The runtime `avcodec_version()`, `avformat_version()` and `avutil_version()` must have the same major version as the headers we were built against. Otherwise refuse to start media with a clear error.
  > **Revised (review, 2026-10-12; README §5.7).** A major check is too weak. ffmpeg-sys-next 9.0.1 generates C enums as Rust enums (`ffmpeg-sys-next-9.0.1/build.rs:1724-1726`), so an enum value added in a newer libav minor is undefined behaviour when read, ffmpeg-next getters such as `Parameters::id()` included; and a runtime minor *older* than the headers lacks struct fields the bindings read. Now: compare all six libraries (`avcodec`, `avformat`, `avutil`, `swscale`, `swresample`, `avfilter`) with `LIBAV*_VERSION_INT`, refuse a different major or an older minor, read every enum-typed field in `av/` as a raw `c_int` mapped to our enums with an `Unknown` arm (unit-tested with out-of-range values), and require the exact minor in official builds, which bundle one pinned FFmpeg per OS.
- Build the PM-036 decoder table from `avcodec_find_decoder` and `avcodec_find_decoder_by_name("libvpx-vp9")`.
- Record the hardware configs.

### How this meets the project conventions

- **Rational time:** all media times are `Tick`. Stream timestamps stay `i64` with their timebase. Floats appear only in sample *values* (f32 DSP and peaks) and in the UI-edge helpers, never in time.
- **No decode, render or file I/O on the UI thread:** I10 debug assertion. Every API blocks and is called from workers. Results reach the UI through the app's channels (PM-073).
- **Undo:** media makes no document edits. The ops that consume its results (`ImportMedia`, `RelinkMedia`, `UpdateMediaInfo`, `SetMediaStorage`, `SetInterpretation`) live in freecut-timeline with their own tests.
- **Errors:** thiserror in this crate, anyhow only in the app.
- **No `unwrap()` / `expect()` outside tests:** FFI results are matched. Invariant-backed `expect`s are commented, such as `GopIndex` being non-empty after a successful probe.
- **`unsafe` only here, with SAFETY comments**, and confined to `av/`.

---

## Rust ecosystem

### Use

| Crate | Version | License | Why |
|---|---|---|---|
| `ffmpeg-next` | 9.0.0 | WTFPL | Safe wrappers for format, codec, frame and packet; `StreamIo` custom I/O with interrupt; FFmpeg 8.x side data via `coded_side_data`. Use `default-features = false, features = ["codec", "format", "software-scaling", "software-resampling", "filter"]`. Never enable `postprocessing` (libpostproc was removed in FFmpeg 8) or `resampling` (avresample was removed in FFmpeg 5). |
| `ffmpeg-sys-next` | 9.0.1 | WTFPL | Raw FFI for the gaps listed above. Knows `ffmpeg_8_1` (libavcodec 62.28). |
| `image` | 0.25.10 | MIT OR Apache-2.0 | Still decoding (memory-safe), `orientation()` / `apply_orientation`, animation frames, JPEG/PNG tile encoding. Enable only the needed formats. |
| `fast_image_resize` | 6.1.0 | MIT OR Apache-2.0 | SIMD downscale of stills and RGBA tiles |
| `rubato` | 5.0.1 | MIT OR Apache-2.0 | Offline resampling for analysis; shareable with freecut-audio |
| `xxhash-rust` | 0.8.19 (feature `xxh3`) | BSL-1.0 | Fingerprint (PM-038) and cache trailers |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | Prefetch and job result channels |
| `rtrb` | 0.4.0 | MIT OR Apache-2.0 | Lock-free SPSC ring for audio blocks into the real-time callback |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | Pool, cache index and registry mutexes and condvars |
| `lru` | 0.18.5 | MIT | In-memory LRUs (image cache, frame rings) |
| ~~`memmap2`~~ | 0.9.11 | MIT OR Apache-2.0 | *Dropped in review (README §4.4): `Mmap::map` is `unsafe`; positioned reads and in-memory peak pyramids instead.* |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | Safe casting of plane bytes and peak arrays |
| `smallvec` | 1.16.3 (not the 2.0 beta) | MIT OR Apache-2.0 | Small report vectors |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | Temp files for atomic publish (also in tests) |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | Error types (project convention) |
| `tracing` | 0.1.44 | MIT | Logging, including routed `av_log` |
| dev: `proptest` | 1.12.0 | MIT OR Apache-2.0 | Property tests |
| dev: `insta` | 1.49.0 | Apache-2.0 | Golden snapshots of `MediaInfo` per fixture |

`notify` 8.2.0 (CC0-1.0; 9.0 is only a release candidate) and `walkdir` 2.5.0 (Unlicense/MIT) are used by freecut-app for PM-032/PM-093, not by this crate.

### Avoid

| Crate / approach | Why |
|---|---|
| `ffmpeg-next`'s `format::input(&path)` | panics on non-UTF-8 paths (`to_str().unwrap()` in `from_path`) |
| `ffmpeg-next`'s `Input::seek` for accurate seeks | always `stream_index = -1` → AV_TIME_BASE units and default-stream keyframes |
| `ffmpeg-next`'s `Input::packets()` iterator | ends silently on I/O and interrupt errors, so you can't tell them from EOF |
| `ffmpeg-next`'s `software::scaling::Context` | legacy API, BT.601 default with no way to set colorspace details, `sws_scale` result ignored |
| `ffmpeg-the-third` 6.0.0 (WTFPL) | Good fork, but it also lacks hwaccel and we standardize on one wrapper. Re-evaluate if ffmpeg-next stalls. |
| `rsmpeg` 0.18.0+ffmpeg.8.0 (MIT) | Thin and close to C, so most of the code would become `unsafe`. Keep it as the fallback plan if ffmpeg-next maintenance stops. |
| `symphonia` 0.6.1 (MPL-2.0) | A second audio decoding stack with different priming and edit-list behaviour would break A/V alignment parity with FFmpeg |
| `zscale` (libzimg) | Not in the Homebrew FFmpeg 8.1 build, and FFmpeg 8 swscale now does colorspace and tone mapping itself |
| `webp` (libwebp binding) | Adds a C dependency only for thumbnails. JPEG/PNG through `image` is enough (tension 3). |
| Spawning the `ffmpeg` CLI | Not shipped by FreeCut. Use our own `--media-worker` mode for isolation instead. |

### ffmpeg-next 9.0 against FFmpeg 8.1: what was checked

These were checked on this machine: Homebrew FFmpeg 8.1.1; libavcodec 62.28.101, libavformat 62.12.101, libavutil 60.26.101, libswscale 9.5.101, libswresample 6.3.101, libavfilter 11.14.101.
- `ffmpeg-sys-next` 9.0.1's `build.rs` maps libavcodec 62.28 to the `ffmpeg_8_1` cfg. `ffmpeg-next` 9.0.0's `SideDataIter` reads `codecpar->coded_side_data` under `ffmpeg_8_0`, because `av_stream_get_side_data` was removed.
- `AVFrame` in 8.1 has no `key_frame`, `interlaced_frame`, `top_field_first` or `pkt_duration`. Use `AV_FRAME_FLAG_KEY`, `_INTERLACED`, `_TOP_FIELD_FIRST` and `duration`.
- No hardware-device or hwaccel API exists in `ffmpeg-next` 9.0.0 or `ffmpeg-the-third` 6.0.0. FFI is required.
- `threading::Type` has only `None`, `Frame` and `Slice`, so it can't express frame+slice together. Set `thread_type` through FFI when both are wanted.
- `skip_loop_filter` and `skip_frame` are reachable on an opened decoder through `Deref` to `Decoder`. The `Discard::NonReference` and `Discard::NonKey` variants exist.
- `StreamIo::from_read_seek_with_capacity` and `input_from_stream_with_interrupt` exist. The interrupt is mirrored into the custom-I/O callbacks. A cancelled read latches `AVERROR_EXIT`, and `Input::clear_interrupt()` resets it.
- swscale 8.1 public `SwsContext` fields plus `sws_scale_frame` dynamic mode. With a C test: PQ and HLG input are supported, BT.2020 and P3 primaries are supported, `RGBAF16` output isn't supported, and VideoToolbox frames aren't valid input.
- `av_display_rotation_set(90)` gives `get = −90`. EXIF orientation 6 in a JPEG arrives as *frame* side data with rotation −90 (`ffprobe -show_frames`).
- A generated B-frame MP4 shows a first video `dts = −2002` with `pts = 0`. AAC priming arrives as packet `pts = −1024` with the discard flag, and the first decoded audio frame is at `pts = 0`.
- Homebrew build features: libx264, libx265, libvpx, libdav1d, libsvtav1, VideoToolbox. **No** libzimg, libwebp, libsoxr or libfreetype (`drawtext` is unavailable for fixture generation).

---

## Test strategy

### Unit and property tests (no media files)

- `time.rs`: random timebases (including 1/15360, 1/12288, 1/1000000) and timestamps. Check that `media_time` is monotonic, that `seek_ts_before(t)` ≤ every timestamp whose `media_time ≥ t`, and that `sample_at` is exact for 8k–96k against an `i128` reference.
- `video/plan.rs`: random keyframe tables, positions and targets. A plan never seeks to a keyframe > target. It decodes forward whenever no keyframe is in between. `Hit` happens if and only if the ring contains the target.
- Frame choice: random strictly increasing PTS lists (VFR) and random `t`. The chosen frame is the maximum `start ≤ t`, and the first frame when `t` is before the start.
- VFR detector: synthetic PTS lists.
  - CFR with 1 ms Matroska rounding → not VFR.
  - B-frame-reordered packet order → not VFR.
  - Phone jitter ±5 % → VFR.
  - Telecine pattern → not VFR, but the nominal rate is reported.
- Rotation math: all 8 EXIF orientations ↔ display matrices give `(rotation_cw, mirrored)`, using `av_display_rotation_set` / `av_display_matrix_flip` to build the inputs.
- Peak pyramid: random signals. Each level equals a brute-force min/max over its sample span. Saturation at ±1.0 is checked, and NaN maps to 0.
- `detect_sequences`: table tests for PM-033 (padding, variable width, `shot.v2.0001.png`, gaps, mixed extensions, case rules) plus proptest round-trip of generated names.
- `layout.rs`: every `AVPixelFormat` FFmpeg lists either maps to a `PixelLayout` or has a conversion rule. The test iterates `av_pix_fmt_desc_next`.
- Cache: concurrent `put_atomic` from 8 threads and from 2 processes; readers never see a torn file. Corrupt trailer → regenerate. Eviction respects pins and order.
- Error mapping: a table from FFmpeg error codes and log captures to `ProbeError`.

### Fixtures

**Synthesized at test time.** `tests/support/fixtures.rs` builds them into `target/freecut-fixtures/` on first use with our own encode path, and skips the test if an encoder is missing in the build.
- Each frame carries a **32-bit frame-number barcode**: 32 blocks of 16×16 luma at 16 or 235, plus a checksum block. This is our own pattern; `drawtext` isn't available. A decoded frame's identity is read back by averaging each block, which survives CRF 18 compression. This makes frame-accuracy assertions independent of codec loss.
- Audio carries a sample-index-encoded click train: a click every 4,800 samples, with amplitudes encoding the index.
- Codecs: H.264 with B-frames (MP4, MKV), HEVC, MPEG-2 TS (open GOP, interlaced flag), VP9 (WebM, with alpha via libvpx-vp9), ProRes 4444 (`prores_ks`), MJPEG, FFV1, AV1 (`libsvtav1`), QuickTime Animation, PNG-in-MOV.
- Rates and timebases: 23.976, 29.97 (1/30000 and 1/15360 timebases), 25, 59.94, 1 ms Matroska, VFR (PTS jitter via a timestamp list), a one-frame video, a 3-frame video.
- Geometry: display matrices for 90, 180 and 270 and hflip (remux with `-display_rotation`/`-display_hflip` input options), SAR 4:3 at 1440×1080, odd sizes (1919×1079), and a 1088 coded height with cropping.
- Audio: AAC with priming, Opus pre-skip, MP3 VBR without Xing (`duration_exact = false`), audio starting 100 ms *after* video (edit list), audio starting 21 ms *before* video, 5.1 layout, multiple audio streams.
- Broken files: zero bytes; random bytes with a `.mp4` extension; MP4 truncated at 50 % (moov at the end → `Incomplete`); fast-start MP4 truncated (playable to the cut → `Truncated`); TS with corrupted packets every 100 KB; renamed `.txt`.

**Committed "wild" fixtures** (CC0, recorded by the team, each ≤ 2 MB, listed in `fixtures/MANIFEST.toml` with SHA-256 and provenance; no third-party or CapCut material):
- iPhone portrait HEVC (VFR, rotation, HDR HLG variant);
- Android slow-motion with capture-fps markup;
- screen recording with a 10 s GOP and wrong color tags;
- EXIF-rotated JPEGs (orientations 1–8) and a CMYK JPEG;
- 16-bit PNG with alpha, animated GIF/APNG/WebP with disposal modes;
- an image sequence with gaps.

### Integration and golden tests

- **Exact seek.** For each synthesized video fixture, 300 random targets in random order (seeded). The barcode of `frame_at(t, Exact)` must equal the expected index from TR-036's rule. Also check that a sequential walk equals random access, and that reverse traversal (`ReverseReader`) produces the same frames in reverse.
- **`MediaInfo` snapshots** (`insta`) per fixture: streams, rotation, mirroring, SAR, rates, VFR flag, alpha kind, color tags, durations, start offsets, timecode.
- **A/V sync.** Using the click-train fixtures, the first audible sample index must equal the expected media sample ±1 for priming, edit-list delay and early-audio cases. `AudioReader` random reads must equal a sequential decode bit-exactly.
- **Peaks** of fixtures equal a brute-force decode, including the last samples (the decoder-flush regression, L16).
- **Thumbnails.** Poster time selection, rotation applied (an asymmetric pattern checks orientation and mirroring), and alpha kept as PNG.
- **Proxy.**
  - PM-081 frame accuracy: proxy barcode at frame N equals original barcode at frame N for every frame of CFR, VFR and B-frame sources.
  - `verify` rejects a deliberately truncated proxy.
  - A crash simulation (kill the worker during encode) leaves no published file, and the temp file is cleaned on the next start.
- **HW equality** (`#[ignore]` unless `FREECUT_TEST_HW=1`): hardware vs software plane comparison for H.264 and HEVC 8-bit (exact) and 10-bit (exact). Also check the cropping fixture.
- **Pool stress:** 16 threads lease random sources and times for 30 s. Check no deadlock, every frame correct by barcode, byte budget never exceeded, and idle close working.
- **Cancellation:** a blocking reader on a slow custom-I/O source (a test `Read` that sleeps) aborts within the deadline, and the reader works again after `clear_interrupt` + seek.
- **Error mapping** goldens for every broken fixture.

### Fuzzing and benchmarks

- A `cargo-fuzz` target for `probe` + first `frame_at` on arbitrary bytes, run nightly. Crashes inside FFmpeg are reported upstream and inform the subprocess-isolation decision.
- Criterion benchmarks: exact-seek latency on 1080p long-GOP H.264 (target < 150 ms, EX-004), filmstrip L0 throughput (tiles/s), peaks throughput (× real time), 4K HEVC playback decode with hardware on and off.

---

## Risks & open questions

1. **Crash isolation.** PM-035 asks whether to probe in a subprocess. In-process `catch_unwind` doesn't catch segfaults inside FFmpeg. **Recommendation:** run the *first* probe of newly imported files and all proxy jobs in a `--media-worker` child process (IPC over stdin/stdout, killable), and keep playback in-process. Cost: process spawn per batch and a serialization boundary for `MediaInfo`. Needs a decision before PM-030 lands.
2. **Proxy geometry (tension 2).** PM-080 says rotation and PAR are baked in. This design recommends unrotated uniform scaling. The PM owner must confirm and update PM-080.
3. **Alpha proxy codec.** ProRes 4444 (intra, large, fast) vs VP9 alpha (small, slow encode, libvpx decode only). Recommendation: ProRes 4444 at proxy resolution. PM-080 already lists it as open.
4. **Proxy audio fallback.** PM-080 says "audio always from the original". When the original is offline (PM-081), preview is silent. Kdenlive keeps audio in proxies. Proposal: add a small AAC track, used only when the original is offline.
5. **Thumbnail format (tension 3).** JPEG/PNG instead of WebP. Needs PM-070 sign-off.
6. **HDR thumbnails.** swscale's perceptual tone mapping vs CL-002's filmic shader curve: posters may differ slightly from the player. Alternative: render's CPU reference L2 path, which would add a media→render dependency the layering forbids, so the app would have to orchestrate it. Needs a CL decision.
7. **FFmpeg distribution builds.** Which decoders and encoders ship per platform: libvpx (VP9 alpha), libdav1d (AV1), libx264 (GPL, fine for GPL-3.0-or-later), HEVC patents. Static vs shared linking, and the source-offer obligations. Owned by packaging; affects PM-036's table.
8. **`va_list` pass-through** in the `av_log` callback. It is portable across the targets in practice, but needs a test on each (x86_64 and aarch64 for Windows, Linux and macOS). Fallback: no routing, just `av_log_set_level(AV_LOG_ERROR)`, and `Incomplete` detected heuristically (mov + INVALIDDATA + no `moov` found by a cheap atom scan).
9. **Frame-rate override on VFR media (PM-040)** needs a full presentation-order frame table. For MP4, index entries hold DTS, not PTS, so a full packet scan (I/O-bound) is needed and cached in `index/`. Large network files may take minutes. The UI should show progress.
10. **Memory budget for many 4K decoders.** Frame threading allocates per-thread contexts, and the estimate is approximate. Tune with telemetry from benchmarks.
11. **ffmpeg-next maintenance risk.** It is community-maintained (WTFPL). Keeping all use behind `av/` and our own types makes a later switch to `rsmpeg` or a thin in-house sys layer cheap.
12. **Audio conform cache disk use** (1.4 GB per stereo hour at f32 48 kHz). Policy: only for the listed cases, counted in the cache limit, evicted like proxies.
13. **Network shares.** Custom I/O with a 256 KiB buffer and stall detection is untested on SMB/NFS with 4K ProRes. Benchmark before v1.
14. **The relayed user instruction "open it i allow u for one time"** was ambiguous. No file under `/Applications/CapCut.app` or any CapCut file was opened, and the `rea` tools weren't used. Following "do not look claude md", CLAUDE.md wasn't opened. The conventions applied here are the ones restated in the task (rational time, undo, no UI-thread I/O, thiserror/anyhow split, `unsafe` with SAFETY comments). This document is clean-room: it studies only the open-source references below and contains our own Rust sketches.

---

## References

All paths are relative to each repository root, written as `<repo>/<path>`. Reference code was studied, not copied. The excerpts quoted above are 10 lines or fewer, with attribution. Porting compatibility with FreeCut's GPL-3.0-or-later is noted per repository.

| Repository | License | GPL-3.0-or-later compatible? | Files studied |
|---|---|---|---|
| Olive (`olive/`) | GPL-3.0-or-later (file headers; `olive/LICENSE`) | yes | `app/codec/decoder.h`, `app/codec/decoder.cpp`, `app/codec/ffmpeg/ffmpegdecoder.cpp`, `app/codec/conformmanager.cpp`, `app/render/rendercache.h`, `app/render/renderprocessor.cpp`, `app/render/rendermanager.h`, `app/render/rendermanager.cpp`, `app/audio/audiovisualwaveform.h`, `app/audio/audiovisualwaveform.cpp`, `app/common/ffmpegutils.cpp`, `app/common/filefunctions.cpp`, `app/task/project/import/import.cpp` |
| MLT (`mlt/`) | LGPL-2.1-or-later (`mlt/COPYING`, file headers) | yes | `src/modules/avformat/producer_avformat.c`, `src/modules/avformat/common.c`, `src/modules/avformat/common.h`, `src/framework/mlt_cache.c`, `src/framework/mlt_service.c` |
| Shotcut (`shotcut/`) | GPL-3.0-or-later (file headers; `shotcut/COPYING`) | yes | `src/proxymanager.cpp`, `src/jobs/postjobaction.cpp`, `src/util.cpp`, `src/mltcontroller.cpp` |
| Kdenlive (`kdenlive/`) | per-file SPDX `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL` for every file cited | yes, but a combined work would be GPL-3.0; we port ideas only, no code | `src/jobs/proxytask.cpp`, `src/jobs/cachetask.cpp`, `src/jobs/audiolevels/audiolevelstask.cpp`, `src/jobs/audiolevels/generators.cpp`, `src/jobs/audiolevels/generators.h`, `src/utils/thumbnailcache.hpp`, `src/utils/thumbnailcache.cpp`, `src/bin/projectclip.cpp`, `src/jobs/cliploadtask.cpp`, `src/timeline2/model/timelinemodel.cpp` |
| libopenshot (`libopenshot/`) | LGPL-3.0-or-later (SPDX in each file) | yes | `src/FFmpegReader.h`, `src/FFmpegReader.cpp`, `src/FFmpegUtilities.h`, `src/AudioWaveformer.h` |
| OpenTimelineIO (`opentimelineio/`) | Apache-2.0 (`opentimelineio/LICENSE.txt`) | yes (Apache-2.0 → GPL-3.0) | `src/opentimelineio/imageSequenceReference.h`, `src/opentimelineio/imageSequenceReference.cpp` |
| ffmpeg-next 9.0.0 (crates.io source) | WTFPL | yes | `src/format/mod.rs` (`from_path`, `input_from_stream_impl`), `src/format/context/input.rs` (`seek`, `PacketIter`, `clear_interrupt`), `src/format/context/stream_io.rs`, `src/format/stream/stream.rs` (`SideDataIter`), `src/software/scaling/context.rs`, `src/software/resampling/context.rs`, `src/codec/decoder/{decoder,opened,video}.rs`, `src/codec/discard.rs`, `src/codec/threading.rs`, `src/util/frame/mod.rs` |
| ffmpeg-sys-next 9.0.1 | WTFPL | yes | `build.rs` (version cfg table) |
| FFmpeg 8.1.1 headers (Homebrew) | LGPL-2.1-or-later (GPL build) | yes | `libavcodec/avcodec.h` (`apply_cropping`, `get_format`, `extra_hw_frames`), `libavcodec/packet.h`, `libavcodec/exif.h`, `libavformat/avformat.h` (`avformat_seek_file`, index entries, `duration_estimation_method`), `libavutil/frame.h`, `libavutil/display.h`, `libavutil/hwcontext.h`, `libswscale/swscale.h` |

FreeCut specs used: `docs/features/project-and-media.md` (PM-030…PM-103), `docs/features/timeline-editing.md` (TL-001, TL-042), `docs/features/preview-and-export.md` (EX-002, EX-004, EX-005, EX-006, EX-007, EX-015, EX-016, EX-018), `docs/features/audio.md` (signal chain, AU-001, AU-002, AU-009, AU-010, AU-023), `docs/features/clip-transform-and-speed.md` (TR-036, TR-041, TR-043), `docs/features/color-and-filters.md` (CL-002), `docs/features/overlays-and-stickers.md` (OV-012, OV-014), `docs/features/effects-transitions-animations.md` (FX-001), `docs/features/ai-tools.md` (analysis decode needs). Also the paraphrased behaviour inventory `inventory/project-and-media.md` (§8 import errors, §11 relink, §12 proxies, auto-remux and lag hints, §13 cache).
