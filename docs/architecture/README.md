# FreeCut architecture

> Status: draft · 2026-10-12 · Scope: the whole workspace. This page combines the ten
> subsystem studies in this folder into one design and makes the decisions they leave open.

FreeCut is a free, open-source (GPL-3.0-or-later) desktop video editor in Rust for macOS,
Windows and Linux. It covers the everyday workflow of CapCut-class editors. This page says how
the six crates fit together, which threads exist, what each crate exposes, which dependencies
we take, the decisions that apply to every crate, how the studies' disagreements are settled,
and the order in which to build the MVP.

| Study | Covers | Main crates |
|---|---|---|
| [timeline-model.md](timeline-model.md) | Rational time, document model, invariants, transition geometry | timeline |
| [edit-ops-and-undo.md](edit-ops-and-undo.md) | Planners, change sets, history, coalescing, snapping | timeline (driven by app) |
| [keyframes-and-speed.md](keyframes-and-speed.md) | `Animatable<T>`, easing, exact time maps, frame sampling, retimed audio | timeline, render, audio |
| [media-io.md](media-io.md) | Probe, decode, seek, hardware decode, caches, thumbnails, peaks, proxies | media |
| [playback-engine.md](playback-engine.md) | Clock, preview scheduler, decode-ahead, frame caches, present ring, transport | app, render, media, audio |
| [render-compositor.md](render-compositor.md) | FX-001 pipeline, color, blend modes, transitions, texture pools | render |
| [audio-engine.md](audio-engine.md) | Mix plan, DSP chain, audio side of the clock, offline render | audio |
| [export-pipeline.md](export-pipeline.md) | Export job, Y′CbCr output, encoders, muxing, finalize, queue | media, render, audio, app |
| [project-format.md](project-format.md) | File format, migrations, save/open, lock, recovery, paths, OTIO | project |
| [text-and-subtitles.md](text-and-subtitles.md) | Text model, fonts, layout, SDF raster, captions, subtitle files and streams | timeline, render, project, media |
| [speech-and-captions.md](speech-and-captions.md) | Captions drawer, local speech-to-text engine choice, `freecut-analysis`, model downloads, filler words, lyrics, second-line sources, captions build slices CS1–CS4 | analysis, timeline, project, render, media, app |
| [cloud-generation.md](cloud-generation.md) | Opt-in generation with the user's own Higgsfield account: MCP client, OAuth sign-in, catalogue, cost preflight, job ledger and the unknown-outcome rule, uploads and downloads, provenance, error mapping, build slices H1–H4 | cloud (proposed), timeline, media, project, app |

**How to read this.** Each study contains the research: how the open-source reference editors
work, the pitfalls in their code, and file-path citations. This README records the decisions.
When this page and a study disagree, §6 decides. The studies are the research record; the
2026-10-12 review pass (§9) added short **"Revised"** notes where a study states something this
page now overrides, and left the rest of each study as written. Feature IDs (TL-, PM-, EX-,
AU-, TR-, FX-, CL-, OV-, TX-, UI-, AI-) refer to `docs/features/*.md`.

---

## 1. System overview

### 1.1 The six crates

| Crate | Responsibility | Owns | Must never |
|---|---|---|---|
| `freecut-timeline` | The pure edit model and every pure function over it | `Tick` and the time math; `Document` (settings, library as plain data, sequences, tracks, clips, transitions, markers, groups, links, compound clips); keyframes and easing; retime time maps; text and caption model; edit planners, `ChangeSet`, `History`, `Editor`; snapping; `validate()`; queries; `FramePlan` and `MixView` builders; `PlayPlan`; `ExportClock`; `render_key` | Do I/O, link FFmpeg, touch the GPU, spawn threads, or use `unsafe` (`#![forbid(unsafe_code)]`) |
| `freecut-media` | The only crate that talks to FFmpeg | Behind the default feature `ffmpeg`: probe, fingerprint, decoders and `DecoderPool`, seeking, hardware decode, stills (`image` crate), image sequences, audio readers, the derived-data cache (thumbnails, filmstrips, peaks, GOP index, PCM conform), proxies, encoders and muxing (`EncodeSession`), subtitle stream writer. Always built, with no FFmpeg: `CancelToken`, the plain wire types (`AudioBlock`, `PooledBuf`, `PixelLayout`, `EncodeFrame`) and the pure colour helpers (`color::hdr_to_sdr`) | Know about subtitle file formats, GPU types or the project file; run on the UI thread (debug assertion) |
| `freecut-render` | GPU compositing and everything drawn into a video frame | wgpu pipelines and WGSL; `FramePlan` → pass graph; color conversion math; effect and transition registries; texture pools; text layout and glyph raster (parley, swash, SDF); preview scheduler, `PresentRing` and frame caches; export renderer and output converter; slates; scopes | Depend on egui or `freecut-project`; evaluate keyframes or time maps (it receives evaluated values) |
| `freecut-audio` | Mixing and audio playback | `MixPlan` compiler, `MixEngine`, DSP (gain, fades, crossfades, limiter), the audio clock, `AudioRing`, the cpal output stream and its feeder, scrub grains, meters and loudness, `OfflineRenderer` for export, stems and analysis | Allocate, lock or decode in the device callback; use `unsafe` |
| `freecut-project` | Persistence and interchange | Private serde DTOs, canonical JSON, migrations, open and save pipelines, atomic replace, lock, backups and recovery, portable media paths, view-state sidecar types, export settings, presets and the codec/container table, subtitle parsers and writers, OTIO import/export | Depend on media, render or audio; touch the UI thread; use `unsafe` |
| `freecut-app` | The egui/eframe binary that wires everything together | Window and panels; the single `Editor`; snapshot publication; `Player` (transport state machine); job system (import, background media jobs, save coordinator, export queue); wgpu device configuration (eframe creates the device, §2.3); egui paint callback; native file dialogs; `MemoryBudget` (§5.12); string catalog `i18n` (§5.13); preferences; mapping errors to user text with `anyhow`; CLI subcommands | Decode, render or do file I/O on the UI thread; mutate the document except through `Editor` |

### 1.2 Dependency direction

```
                    ┌────────────────────────────────────────┐
                    │              freecut-app               │   depends on all five crates
                    └────────────────────────────────────────┘
   ┌───────────────────┐        ┌───────────────────┐
   │  freecut-render   │        │   freecut-audio   │
   └────┬─────────┬────┘        └────┬─────────┬────┘
        │         │                  │ feature │
        │         ▼                  ▼ "media"  │
        │     ┌──────────────────────────┐     │      ┌───────────────────┐
        │     │      freecut-media       │     │      │  freecut-project  │
        │     └────────────┬─────────────┘     │      └─────────┬─────────┘
        ▼                  ▼                   ▼                ▼
   ┌───────────────────────────────────────────────────────────────────┐
   │                         freecut-timeline                          │   no internal deps
   └───────────────────────────────────────────────────────────────────┘
```

Allowed edges between workspace crates, and no others:

| Crate | May depend on |
|---|---|
| `freecut-timeline` | nothing internal |
| `freecut-media` | timeline (FFmpeg only behind its default feature `ffmpeg`) |
| `freecut-project` | timeline |
| `freecut-render` | media (default features), timeline |
| `freecut-audio` | timeline; media **always**, with `default-features = false` (cancel token and wire types only). Audio's default feature `media` forwards to `freecut-media/ffmpeg` and adds the `PcmProvider` adapter over media's readers, so `cargo test -p freecut-audio --no-default-features` builds the whole mixer, `OfflineRenderer` included, without FFmpeg |
| `freecut-app` | everything |

The audio API is written in media's types (`CancelToken` in `OfflineRenderer::render` and
`ReadMode::Blocking`, `AudioBlock` from `OfflineMix`), so an optional edge would make the
FFmpeg-free build fail on audio's own public API. The split by feature keeps those types and
drops only FFmpeg. The dependency guard checks both the edge and the feature: `cargo tree -p
freecut-audio --no-default-features` must not contain `ffmpeg-sys-next`.

This is the project rule (app → everything; render and audio → media + timeline; timeline →
nothing internal). The studies add two edges that respect it: media → timeline for `Tick` and
the plain media types, and project → timeline for the model it maps to. Notably forbidden:
render ↔ audio, render → project, media → project, project → media.

"Nothing internal" does not mean "no crates at all". The timeline crate may use pure, I/O-free
external crates. They are on an allowlist checked by the dependency guard test (M0):
`thiserror`, `serde` (optional feature), `smallvec`, `glam`, `xxhash-rust`, `num-rational`,
`num-integer`, `ethnum`, `libm`, `unicode-segmentation`, `unicode-normalization`,
`icu_segmenter`, `icu_casemap`. This replaces timeline-model.md's "thiserror and serde only"
(§6, C30).

### 1.3 Data flow

**Edits, snapshots and consumers.**

```
┌──────────────────────────────────── UI thread (freecut-app, egui) ─────────────────────────────────────┐
│ pointer / keys ─► px → Tick (UI edge) ─► quantize to frame grid ─► snap (SnapIndex) ─► clamp (limits)  │
│                                              │                                                         │
│                                              ▼                                                         │
│        Editor::plan(EditRequest) ── Tx on an Arc-clone of the Document (freecut-timeline, pure)        │
│            │ Err(EditError) → red ghost or toast; document untouched                                   │
│            ▼ Ok(Plan { changes: ChangeSet, preview })                                                  │
│   pointer move: draw plan.preview() as ghost        pointer up: Editor::commit(plan)                   │
│                                                        │  History: coalescing, 200 entries / 512 MB     │
│                                                        │  undo/redo = mechanical inverse of ChangeSet   │
│                                                        ▼                                               │
│                          publish DocSnapshot { Arc<Document>, version } into ArcSwap                   │
└─────────┬────────────────────────┬─────────────────────────┬───────────────────────┬──────────────────┘
          │ latest wins            │ latest wins             │ Arc clone              │ Arc clone at queue time
          ▼                        ▼                         ▼                        ▼
  PREVIEW SCHEDULER         AUDIO PLAN COMPILER        SAVE WORKER              EXPORT JOB
  (render thread)           (audio worker)             (freecut-project)        (orchestrator + render,
  frame path below          audio path below           DTOs → canonical JSON    audio and mux threads)
                                                       → temp + fsync + rename
                                                       → <Name>.freecut
```

**Video frame at time t (preview and export share it).**

```
 DocSnapshot ─► frame_plan(seq, t) ─► FramePlan      freecut-timeline: coverage, transitions, time maps,
                                         │             keyframes, transform matrix, text items (pure)
                                         ├─► render_key(plan, inputs) ─► RamFrameCache hit? (v1) ─► upload to slot
                                         ▼ miss
                  Compositor::render (freecut-render, wgpu)
                    L1 source ◄── FrameSource::frame(asset, t, mode) ◄── freecut-media DecodeQueue + DecoderPool
                    L2 normalize (YUV affine, geometry) … L8 place … L10 blend; transitions; effect layers
                                         │
                     ┌───────────────────┴──────────────────────┐
                     ▼ preview                                   ▼ export
        PresentRing slot (Rgba8Unorm, 4–14 slots)    OutputConverter: BT.709 limited, 4:2:0, dither
                     │ UI: take_due(clock + frame time)          │ ReadbackRing (3) → EncodeFrame (bounded by bytes)
                     ▼                                           ▼
        egui paint callback (freecut-app)            mux thread: EncodeSession (freecut-media)
                                                     → <final>.part → fsync → verify → atomic rename
```

**Audio.**

```
 DocSnapshot ─► MixView (timeline) ─► compile (freecut-audio, worker, latest wins) ─► ArcSwap<MixPlan>
 PlayPlan (timeline, pure: shuttle rate, loop wrap, end) ─────────────────────────────────┐
                                                                                          ▼
 feeder thread: MixEngine::render(plan, pos) ◄── PcmReader::read, non-blocking (freecut-media:
     voices → track buses → master (gain, limiter)      streaming fill and block cache; PCM conform for
     writes only the ready prefix (§2.5)                seek-unreliable streams from M2, the rest in v1)
     → device-rate stage → AudioRing (rtrb) + EpochMark ring
                                                                                          ▼
 cpal callback (real time): copy ring → device, 2 ms fades, meters, preview mute, publish Stamp (seqlock)
                                                                                          ▼
 AudioClock ─► Transport on the UI thread and the PreviewScheduler (frames become due at clock time)

 export: OfflineRenderer (same MixEngine, ReadMode::Blocking, plan compiled at the export rate)
         ─► AudioBlock channel ─► mux thread
```

**Media workers.**

```
 import (dialog / drop) ─► job pool: probe (optionally in a `freecut --media-worker` child) ─► MediaInfo
     ─► Editor::plan(ImportMedia) ─► commit: one undo entry
 background jobs (yield while PlaybackActivity != Idle; background permits from DecodeGate):
     poster + hover tiles, filmstrip chunks L0/L1/L2, per-stream audio pass (peaks + exact length,
     plus PCM conform for seek-unreliable streams; other conform cases in v1), GOP index scan, proxies
     ─► CacheStore (<cache>/v1, temp file + atomic rename) ─► app loads tiles as egui textures
 playback decode: DecodeQueue workers (Exact > Playback > Approximate > Lookahead > Background)
     lease decoders from DecoderPool; frames are Arc<VideoFrame> (ref-counted AVFrame, no copy)
```

---

## 2. Threading model

### 2.1 Threads

| Thread | Crate | Count | Priority | Owns | Talks through | Never |
|---|---|---|---|---|---|---|
| UI (egui main) | app | 1 | normal | `Editor`, selection, view state, `Transport`, snapshot publisher | `ArcSwap` stores, crossbeam commands, reads of atomics and `PresentRing` | decode, render, file I/O, run a modal OS loop (synchronous file dialogs), hold a lock longer than 1 ms (debug assertion) |
| Preview scheduler | render | 1 | above normal | preview `Compositor`, `PresentRing` writes, RAM cache fill (v1) | `ArcSwap<PreviewRequest>`, wake channel (capacity 1), `DecodeTicket`s | mutate the document |
| Decode workers | media | clamp(cores/2, 2, 6) | normal | leased decoders | priority job queue, one-shot `DecodeTicket` | share an FFmpeg context with another thread |
| Prefetch and look-ahead | media | one per active (source, lane) | normal | one lease each | generation counters; bounded queue (8 frames or 0.5 s); `rtrb` blocks for audio | — |
| Audio plan compiler | audio | 1 worker | normal | — | latest-wins slot → `ArcSwap<MixPlan>` | — |
| Audio feeder | audio | 1 | above normal | `MixEngine`, voices, resamplers, DSP state | `AudioRing` + `EpochMark` ring to the callback; crossbeam `AudioCmd` | block on decode or disk; open or seek a file (readers arrive already opening, §2.5) |
| cpal callback | audio | host-owned | real time | preallocated callback state | `rtrb` consumer, atomics, seqlock `Stamp` | allocate, lock, log, drop an `Arc`, touch sources |
| Loudness worker | audio | 1 | normal | `ebur128` state | lossy `rtrb` feed | — |
| Font discovery | render | 1 | normal | fontique collection build | `ArcSwap<FontSnapshot>` | — |
| TextService | render | 1 | normal | `TextEngine` #0, `EditActor`, font-picker thumbnails | crossbeam commands; latest-wins replies | GPU work |
| Disk cache I/O (v1) | render | 1 | below normal | `DiskSegmentCache` | bounded crossbeam (8) | — |
| Job pool (CPU and I/O) | app | pool | lowest | probe, thumbnails, peaks, proxies, subtitle read, save worker (one in flight, one queued), path resolution (one thread per volume), memory-pressure poll (§5.12), awaiting file-dialog futures (§2.3) | crossbeam results to the UI | — |
| Export job | app, with render, audio and media components | orchestrator + render + audio + mux | below normal | `ExportRenderer` (on the shared device in the MVP, C19), `OfflineRenderer`, `EncodeSession` | bounded channels, `ProgressCell` atomics, `CancelToken` | drop a frame |
| Child processes | the app binary | on demand | — | `--probe-encoders`; `--media-worker` (optional, see §8) | JSON lines on stdin/stdout | — (they are the only things ever killed) |

No async runtime is used. FFmpeg and GPU work is blocking and CPU-bound, and the pipeline needs
priorities and real-time guarantees that `tokio` and `rayon` do not give (playback-engine.md,
"Avoid").

### 2.2 Channels and shared state

| Mechanism | Used for |
|---|---|
| `arc_swap::ArcSwap` (latest wins) | `DocSnapshot`, `PreviewRequest`, `PlayPlan`, `MixPlan`, `FontSnapshot` |
| `rtrb` wait-free SPSC rings | `AudioRing` + `EpochMark`, meter frames, loudness feed, recording capture, per-source audio blocks into the feeder |
| `crossbeam-channel` bounded | decode job queue and tickets; wake-ups (`try_send`, capacity 1); audio commands; device errors (`try_send` from the cpal error callback); job results; export frames (capacity `clamp(512 MiB / frame_bytes, 2, 8)`); export audio blocks (64); `select!` on cancel |
| Atomics and a seqlock | clock `Stamp` history (64 entries), `SharedOutput` flags, `PlaybackActivity`, `ExportGate`, `ProgressCell` |
| `parking_lot::Mutex`, held for microseconds, never across I/O or GPU calls | `PresentRing` slot states, decoder pool state, hardware registry, cache index |
| `DecodeGate` semaphore | playback and scrub decode take priority over background jobs |

### 2.3 Ownership rules

1. **The document is immutable once published.** Only `Editor` (UI thread) changes it, by
   swapping in a new `Arc<Document>`. Readers on other threads `load()` a snapshot per frame or
   per block and never see a half-applied edit. Structural sharing (`Arc` per sequence, track,
   clip and property bag) makes a snapshot O(1) and a commit O(touched tracks).
2. **FFmpeg contexts are used by one thread at a time.** Decoders are leased exclusively. One
   mux thread owns every FFmpeg context of an export job. Decoded `AVFrame`s are immutable and
   shared through `Arc`.
3. **One `Compositor` per thread** (preview scheduler, export, thumbnails). It is `Send`, not
   `Sync`. Pipelines and shader sources are shared read-mostly.
4. **The cpal `Stream`** is created, owned and dropped on one owner thread.
5. **The GPU device.** eframe creates the instance, adapter, device and queue through
   `egui_wgpu::WgpuSetup::CreateNew(WgpuSetupCreateNew { device_descriptor, native_adapter_selector, .. })`,
   because only that path keeps what eframe does for us: the adapter is chosen with
   `compatible_surface`, so it can present (`egui-wgpu-0.36.2/src/lib.rs`, `request_adapter` and
   `RenderState::create`); eframe injects the display handle that some platforms require
   ("Wayland with GLES", `egui-wgpu-0.36.2/src/setup.rs:165,186`), which is exactly the GL
   fallback; and egui's limits are applied (`max_texture_dimension_2d: 8192`, WebGL2 downlevel
   limits on GL, `setup.rs:261-273`). An app-created device handed over with
   `WgpuSetup::Existing` would lose all three on multi-GPU and Wayland systems.
   - FreeCut's `device_descriptor` closure receives the adapter and requests
     `TEXTURE_FORMAT_16BIT_NORM` (L2's `R16Unorm`/`Rg16Unorm`) and `TIMESTAMP_QUERY` when
     `adapter.features()` has them. Limits start from `Limits::default()`, or the downlevel
     WebGL2 limits on GL, then take the adapter's own `max_texture_dimension_2d` (never below
     8192) and buffer sizes, which export on the shared device needs.
   - The app builds `GpuContext` from `CreationContext::wgpu_render_state` in the app creator,
     then installs `device.on_uncaptured_error(..)` (the default handler panics) and
     `device.set_device_lost_callback(..)`. Both work after creation in wgpu 30
     (`wgpu-30.0.1/src/api/device.rs:506,681`).
   - `native_adapter_selector` implements the UI-028 GPU choice and must only pick adapters
     that pass the surface-compatibility check.

   Preview, thumbnails, the TextService raster and, in the MVP, export share that device, so
   present slots are sampled by the egui paint callback without copies, and the single queue
   orders all their work (§6, C19). Device loss sets a flag; the next `render` returns
   `RenderError::DeviceLost`, caches are dropped, and v1 offers autosave plus "restart
   renderer" (§8).
6. **OS dialogs never block the UI thread.** File dialogs (Import, Open, Save As, the export
   folder) use `rfd::AsyncFileDialog` with the main window as parent. On macOS that path opens a
   window-modal sheet and returns at once (`rfd-0.17.2/src/backend/macos/modal_future.rs`); the
   synchronous API calls `NSSavePanel::runModal` (`.../macos/file_dialog/panel_ffi.rs:76`), a
   nested modal loop that would stop egui updates and so video presentation. The UI thread
   creates the future and hands it to a job thread that awaits it with `pollster::block_on`;
   the result comes back over the job channel with a `request_repaint`. The `Transport` pauses
   when a dialog opens.
7. **Presentation follows the clock, not render completion.** eframe repaints reactively, and
   the scheduler renders frames ahead of their deadline, so a repaint requested only when a
   frame becomes Ready usually finds nothing due. While the `Transport` is `Prerolling`,
   `Playing` or `Buffering`, every update calls `ctx.request_repaint()` (or
   `request_repaint_after(next_ready_deadline − now)`), so `take_due` runs every vsync. The
   `PresentRing` has `slots = clamp(128 MiB / slot_bytes, 4, 14)` slots (1080p: 14; 4K: 4),
   and the planner's depth is clamped to `slots − 2` (one slot is Shown, one is being
   written), so the adaptive pre-roll of up to 12 frames is reachable where memory allows.

### 2.4 Cancellation

- **One type:** `freecut_media::cancel::CancelToken` (media is the lowest crate that render,
  audio and export all depend on). A token fires on any of:
  - an explicit `cancel()`;
  - a newer request generation (latest wins: `(Arc<AtomicU64> current, my_generation, Epoch)`);
  - a deadline or an I/O stall.

  `is_cancelled()` is one atomic load. `receiver()` gives a crossbeam receiver for `select!`,
  so a producer blocked on a full channel wakes at once.
- **Checked at every stage boundary:** between packets (decode), between layers
  (compositor), per frame (export render loop), per block (audio). The same token is installed
  as the FFmpeg AVIO interrupt callback, so a stalled network read aborts too. A single
  `send_packet` or `receive_frame` call cannot be interrupted, which bounds cancel latency to
  one decode call.
- **Stale results are discarded, not prevented.** Every result carries `(epoch, generation)`
  and is checked when decoding finishes, when rendering finishes, and in `take_due` /
  `take_still`. A stale result is released and never shown.
- **Threads are never killed.** Shutdown stores `Idle`, sets a stop flag, drops senders, wakes
  every thread and joins with a 2 s watchdog that only logs. Only child processes (encoder
  probe, optional media worker) are killed.
- **Export cancel** abandons the `EncodeSession` (no flush, no trailer), joins the threads and
  deletes the `.part` file, within 1 s.
- **After an FFmpeg cancel** the AVIO context has `AVERROR_EXIT` latched. The reader calls
  `clear_interrupt()`, marks its position unknown, and its next use starts with a seek.

### 2.5 Real-time audio rules

The callback body is a plain function `fill(...)` over slices and atomics. It does not depend
on cpal types, so tests call it directly inside `assert_no_alloc`. It copies ring frames, fades
2 ms at flush, hold and stop, meters, applies preview mute after metering, publishes the
`Stamp`, and counts underruns. An underrun outputs silence and the clock **holds**, which keeps
A/V sync and never repeats audio. Map evaluation, BigInt fallbacks, plan drops and source reads
all happen on the feeder.

**A source that is not ready is never skipped in playback** (§6, C41). The MVP has no conform
for ordinary files (C11), so streaming fill returns `NotReady` at almost every play start, seek
and cut; rendering those samples as zeros would advance the clock and lose that audio for good.
- In `RenderMode::Playback` the feeder renders only up to the first sample at which any
  non-dormant voice returns `Fill::NotReady { valid_frames }`, writes that ready prefix to the
  ring, and retries on its next wake. The ring drains, the callback underruns, and the clock
  holds. After 1 s of starvation the `Transport` enters `Buffering` (the same 1 s / 0.5 s rule
  as for video).
- Zeros are written only for sources that are `Offline`, a missing stream or a decode error
  (counted as `starved` and reported), and for silent shuttle (C9).
- `Prerolling` opens the audio gate when the first video frame is Ready **and** every voice
  active at the start position has returned `Full` for its first block, or after 250 ms; the
  "ring full enough" test alone is not used, because a ring can be full of zeros.
- `Scrub` keeps "silent rather than late": a grain whose source is not ready is dropped.
- `PcmProvider::open` in `ReadMode::Playback` never blocks: it returns a reader handle at
  once, media's look-ahead thread opens and seeks the file (`avformat_open_input` +
  `find_stream_info` can take seconds on a network volume, while the ring holds about 43 ms),
  and reads return `NotReady` until the reader is positioned. The feeder never performs file
  I/O, including for voices added by an edit during playback.

---

## 3. Public API sketch per crate

Sketches only: field lists and signatures are abbreviated (`..`). Names are final unless §6
says a study disagreed; then the name below is the resolution.

### 3.1 `freecut-timeline`

```text
time/      Tick, TICKS_PER_SECOND, Rounding, Rational, FrameRate, SampleRate, SampleSpan, TimeRange,
           MarkRange, StreamTimeBase, ticks_from_stream, stream_ts_at_or_before, Timecode, TimeError
id.rs      id newtypes, IdAlloc, IdCursor, IdRemap
model/     Document, ProjectSettings, library (MediaItem, MediaInfo, StoredPath, Fingerprint), Sequence,
           Track, Clip, ClipContent, Transition, Marker, relations, props/ (typed property bags)
anim/      Animatable<T>, KeyframeTrack<T>, Keyframe<T>, Interpolation, Interpolate, PropPath, PropBag
retime/    Retime, TimeMap, SourceInstant, Speed, SpeedCurve, CompiledCurve, SourceFrameGrid, SamplePlan
text/      RichText, Span, TextStyle, TextClipData, FontRef, text ops
caption/   CaptionTrackData, CaptionSegment, caption ops, re-flow, retime
edit/      EditRequest, EditCtx, Tx, Plan, Change, ChangeSet, policy/ (magnet, ripple, attach, place, finish), ops/
history.rs editor.rs snap.rs validate.rs query.rs
plan/      FramePlan (+ TextItemPlan), MixView, gain_envelope_at, render_key
effects/   EffectManifest, ParamSpec, ParamSet, EffectCatalog::migrate (pure; no GPU types; v1 content, C37)
playback.rs PlayPlan, PlayTime, Epoch, ShuttleRate     export.rs ExportClock
testkit/   (feature "testkit", dev only) ASCII timeline DSL, proptest strategies, frame-array oracle
```

```rust
// ---- time (timeline-model §3) ----
pub const TICKS_PER_SECOND: i64 = 705_600_000; // 2^9·3^2·5^5·7^2, the "flicks" constant
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(i64);                                    // no Add/Sub impls: checked math only
impl Tick {
    pub const fn from_raw(v: i64) -> Tick;
    pub const fn raw(self) -> i64;
    pub fn checked_add(self, d: Tick) -> Result<Tick, TimeError>;
    pub fn checked_sub(self, d: Tick) -> Result<Tick, TimeError>;
    pub fn mul_rational(self, r: Rational, mode: Rounding) -> Result<Tick, TimeError>;
    pub fn as_secs_f64(self) -> f64;                     // UI edge only; never fed back
}
pub enum Rounding { Floor, Ceil, NearestTiesDown, NearestTiesEven }
pub struct Rational { num: i64, den: i64 }               // reduced, den > 0, no From<f64>
pub struct FrameRate { num: u32, den: u32, frame: i64 }  // new() refuses rates whose frame isn't whole ticks
impl FrameRate {
    pub fn new(num: u32, den: u32) -> Result<Self, TimeError>;
    pub fn quantize(self, t: Tick) -> Result<Tick, TimeError>;     // nearest, ties to the earlier frame
    pub fn frame_index(self, t: Tick) -> i64;                      // floor
    pub fn frame_start(self, k: i64) -> Result<Tick, TimeError>;
}
pub struct SampleRate(u32);                              // must divide TICKS_PER_SECOND
impl SampleRate {
    pub fn new(hz: u32) -> Result<Self, TimeError>;
    pub fn sample_at(self, t: Tick) -> i64;              // floor, also for negative ticks
    pub fn span_of(self, start: Tick, end: Tick) -> SampleSpan; // abutting spans tile exactly
}
pub struct TimeRange { start: Tick, duration: Tick }     // half-open; duration > 0; end() cannot overflow
pub struct StreamTimeBase { num: i32, den: i32 }
pub fn ticks_from_stream(ts: i64, tb: StreamTimeBase) -> Result<Tick, TimeError>;      // the only stream → tick
pub fn stream_ts_at_or_before(t: Tick, tb: StreamTimeBase) -> Result<i64, TimeError>; // its exact inverse

// ---- ids and document ----
// SeqId, TrackId, ClipId, TransitionId, MarkerId, GroupId, LinkId, MediaId, BinId, EffectId:
// `struct X(NonZeroU64)`, all allocated by ONE document-wide counter, always < 2^53.
pub struct IdAlloc { next: NonZeroU64 }   // owned by Editor; persisted as ids.next; never rewound
pub struct IdCursor { .. }                // plan-time allocation; ghosts burn nothing

pub struct Document { settings: Arc<ProjectSettings>, library: Arc<Library>,
                      sequences: BTreeMap<SeqId, Arc<Sequence>>, root: SeqId }  // no public mutators
pub struct Sequence { id: SeqId, flags: SequenceFlags,
                      visual: Vec<Arc<Track>>,       // bottom → top; visual[0] IS the main track
                      audio: Vec<Arc<Track>>, markers: Arc<[Marker]> }
pub struct Track { id: TrackId, kind: TrackKind, header: TrackHeader /* name, muted, hidden, locked */,
                   payload: TrackPayload /* None | Caption(CaptionTrackData) */,
                   clips: Vec<Arc<Clip>>,            // sorted by (start, id), never overlapping
                   transitions: Vec<Transition> }    // at most one per cut
#[non_exhaustive] pub enum TrackKind { Video, Text, Caption, Sticker, Effect, Adjustment, Audio }
pub struct Clip { id: ClipId, range: TimeRange, content: ClipContent, enabled: bool,
                  name: Option<String>, markers: Vec<Marker>, group: Option<GroupId>,
                  attach: Option<Attachment>, link: Option<LinkId>,
                  content_offset: Tick,              // key clock offset for non-media content (0 for media)
                  props: PropBags }                  // Arc'd typed bags: transform, audio, color, effects, ...
pub enum ClipContent {
    Media { asset: MediaId, streams: StreamSel, source: SourceRange, retime: Retime, embedded_audio: EmbeddedAudio },
    Still { from: StillSource },
    Generated { kind: GeneratedKind },
    Text(Arc<TextClipData>),
    Caption(Arc<CaptionSegment>),                    // caption segments are clips (C5)
    Compound { seq: SeqId, source: SourceRange, retime: Retime },
    Unavailable { kind: String },                    // unknown kind from a newer file; read-only placeholder
}
pub struct Transition { id: TransitionId, from: ClipId, to: ClipId, in_offset: Tick, out_offset: Tick,
                        placement: Placement /* Overlap | Handles(Alignment) */, payload: TransitionPayload }
pub enum Coverage<'a> {
    Empty,
    Clip { clip: &'a Clip, local: Tick },
    Transition { tr: &'a Transition, from: &'a Clip, from_local: Tick, to: &'a Clip, to_local: Tick,
                 elapsed: Tick, duration: Tick },   // two integers; render divides
}
impl Track { pub fn coverage_at(&self, t: Tick) -> Coverage<'_>;   // O(log n), no allocation
             pub fn clips_in_range(&self, r: TimeRange) -> impl Iterator<Item = &Clip>; }
pub fn validate(doc: &Document, limits: Option<&dyn SourceLimits>) -> Result<(), Vec<InvariantError>>;

// ---- retime and keyframes (keyframes-and-speed §4–§5) ----
pub struct Retime { mode: RetimeMode, reverse: bool, pub frame_sampling: FrameSampling, map: TimeMap }
pub enum RetimeMode { Normal(Speed), Curve(Arc<SpeedCurve>) }
pub fn source_time_at(clip: &Clip, t: Tick) -> Result<SourceInstant, TimeError>;   // the only mapping
pub fn source_span(clip: &Clip, t0: Tick, t1: Tick) -> Result<(SourceInstant, SourceInstant), TimeError>;
pub fn timeline_len(clip: &Clip) -> Tick;
pub fn timeline_offset_of(clip: &Clip, s: Tick) -> Option<Tick>;
pub fn local_rate(clip: &Clip, t: Tick) -> Rational;                               // display and decisions only
pub trait SourceFrameGrid: Send + Sync { fn len(&self) -> usize; fn start(&self, i: usize) -> Tick; }
pub fn plan_samples(clip: &Clip, t: Tick, frame: Tick, grid: &dyn SourceFrameGrid) -> Result<SamplePlan, TimeError>;
pub enum SamplePlan { Single(FrameIdx), Blend(SmallVec<[(FrameIdx, f32); 16]>),
                      Synthesize { a: FrameIdx, b: FrameIdx, phase: f32 } }    // renamed from FramePlan (C6)
pub struct Animatable<T> { static_value: T, keys: Option<Arc<KeyframeTrack<T>>> }
pub trait Interpolate: Clone + PartialEq + Send + Sync + 'static {
    fn interpolate(a: &Self, b: &Self, e: f64) -> Self;
}
pub fn key_time_at(clip: &Clip, t: Tick) -> Result<Tick, TimeError>;

// ---- editing (edit-ops-and-undo) ----
#[non_exhaustive]
pub enum EditRequest { Insert(..), Overwrite(..), Move(..), Trim(TrimReq), Split(..), Delete(..), Range(..),
    CloseGaps(..), Replace(..), Freeze(..), Paste(..), Duplicate(..), Group(..), Ungroup(..),
    DetachAudio(..), RestoreAudio(..), AddTrack(..), RemoveTrack(..), MoveTrack(..), SetTrackHeader(..),
    SetSeqFlag(..), SetRetime(..), SetEnabled(..), AddTransition(..), RemoveTransition(..),
    CreateCompound(..), BreakApart(..), Conform(..), Props(PropsReq), Keyframes(KeyReq),
    Text(TextReq), Captions(CaptionReq), Library(LibraryReq /* ImportMedia, RemoveMedia, RelinkMedia, .. */) }
pub enum TrimMode { Plain, Ripple, Roll, Slip, Slide }
pub struct EditCtx<'a> { pub rate: FrameRate, pub prefs: &'a EditPrefs, pub limits: Limits,
                         pub sources: &'a dyn SourceLimits, pub busy: &'a BTreeSet<ClipId>,
                         pub playhead: Option<Tick> }
pub trait SourceLimits { fn media_duration(&self, m: MediaId) -> Option<Tick>; } // None while probing
pub enum Change {
    InsertClip { .. }, RemoveClip { .. }, ReplaceClip { seq, track, before: Arc<Clip>, after: Arc<Clip> },
    ShiftClips { seq, track, ids: Arc<[ClipId]>, delta: Tick },
    InsertTrack { .. } /* may carry clips */, RemoveTrack { .. }, MoveTrack { .. }, SetTrackHeader { .. },
    InsertTransition { .. }, RemoveTransition { .. }, ReplaceTransition { .. },
    UpsertMarker { owner: MarkerOwner, before: Option<Marker>, after: Option<Marker> },
    SetSeqFlags { .. }, InsertSequence { .. }, RemoveSequence { .. }, SetSettings { .. },
    Library(LibraryChange),
}
pub struct ChangeSet(Vec<Change>);     // inverse() = reversed list of Change::inverse(); compact()
pub struct Plan { pub label: Label, pub changes: ChangeSet, pub notices: Vec<Notice>,
                  pub selection_after: Option<SelectionSnapshot>, pub primary_seq: SeqId,
                  /* private: result, base_revision, id_high_water */ }
impl Plan { pub fn preview(&self) -> &Document; pub fn is_noop(&self) -> bool; }
pub struct Editor { /* doc, ids: IdAlloc, history: History, revision: u64 */ }
impl Editor {
    pub fn open(doc: Document, ids_next: u64) -> Result<Self, InvariantError>;  // last non-undoable step (§5.11)
    pub fn doc(&self) -> &Document;  pub fn snapshot(&self) -> DocSnapshot;  pub fn ids_next(&self) -> u64;
    pub fn plan(&self, req: &EditRequest, ctx: EditCtxArgs<'_>) -> Result<Plan, EditError>;
    pub fn commit(&mut self, plan: Plan, meta: CommitMeta) -> Result<Committed, CommitError>;
    pub fn undo(&mut self) -> Result<Applied, HistoryError>;
    pub fn redo(&mut self) -> Result<Applied, HistoryError>;
    pub fn begin_gesture(&mut self, key: CoalesceKey) -> GestureId;
    pub fn end_gesture(&mut self, g: GestureId);
    pub fn cancel_gesture(&mut self, g: GestureId);          // applies the inverse; leaves no entry
    pub fn mark_saved(&mut self);  pub fn is_dirty(&self) -> bool;
}
pub struct DocSnapshot { pub doc: Arc<Document>, pub version: u64 }
pub struct SnapIndex { .. }
impl SnapIndex { pub fn build(seq: &Sequence, extra: &[(Tick, SnapKind)], exclude: &BTreeSet<ClipId>) -> Self;
                 pub fn snap(&self, edges: &[Tick], delta: Tick, threshold: u64) -> Option<Snap>; }

// ---- pure inputs for render, audio, playback and export ----
pub fn frame_plan(doc: &Document, seq: SeqId, t: Tick, inputs: &PlanInputs<'_>) -> Result<FramePlan, TimeError>;
pub struct FramePlan { pub seq_version: u64, pub time: Tick, pub frame: Tick, pub frame_index: i64,
                       pub canvas: CanvasSpec, pub background: BackgroundPlan, pub nodes: Vec<PlanNode> }
// render_key does not hash seq_version, time or frame_index; nodes contribute the times and
// seeds they consume (§5.5), so a ripple that only shifts frames keeps their keys.
pub enum PlanNode { Layer(LayerPlan), Transition(TransitionPlan), EffectLayer(EffectLayerPlan), Adjustment(AdjustmentPlan) }
pub enum LayerSource { Video { asset: MediaId, samples: SmallVec<[(Tick, f32); 2]> }, Still { asset: MediaId },
                       Raster(RasterRequest /* Text(TextItemPlan), Shape, Sticker */), Slate { kind: SlateKind, file_name: String } }
// LayerPlan carries layer_to_canvas: glam::DMat3 (a homography), evaluated values and exact ticks.
pub fn mix_view(doc: &Document, seq: SeqId) -> MixView<'_>;
pub fn gain_envelope_at(clip: &Clip, t: Tick) -> GainDb;    // used by the UI overlay and the audio equivalence test
pub fn render_key(plan: &FramePlan, inputs: &RenderKeyInputs<'_>) -> RenderKey;   // 128-bit XXH3
pub struct PlayPlan { pub epoch: Epoch, pub start: Tick, pub rate: ShuttleRate, pub bounds: TimeRange,
                      pub at_end: EndBehavior, pub frame: FrameRate }
// bounds = [0, duration) or the loop region. A commit that changes the sequence duration while
// playing applies plan.amended(effective_from, new_bounds), as loop edits do, or starts a new
// epoch at the current position when amending is impossible (playback-engine.md, Revised note).
impl PlayPlan { pub fn timeline_at(&self, played: PlayTime) -> PlanPos;
                pub fn frames_from(&self, from: PlayTime, display_interval: PlayTime)
                    -> impl Iterator<Item = (PlayTime, Tick)> + '_; }
pub struct ExportClock { .. }
impl ExportClock { pub fn new(range: TimeRange, rate: FrameRate) -> Result<Self, ExportTimeError>;
                   pub fn frame_count(&self) -> u64;
                   pub fn frame_time(&self, n: u64) -> Result<Tick, ExportTimeError>;
                   pub fn sample_count(&self, sr: SampleRate) -> Result<u64, ExportTimeError>; }
```

Errors: `TimeError`, `EditError`, `ApplyError` (always a bug), `InvariantError`, `HistoryError`,
`CommitError`, `RetimeError`, `KeyframeError`, `RichTextError`, `ExportTimeError`.

### 3.2 `freecut-media`

```rust
// Always built (no FFmpeg): cancel, wire types (EncodeFrame, AudioBlock, PooledBuf, PixelLayout), color.
// Everything else is behind the default feature "ffmpeg" (§1.2).
pub fn init(cfg: MediaConfig) -> Result<MediaRuntime, MediaInitError>; // libav major + minor check (§5.7), PM-036 table
pub fn mark_ui_thread();                                               // blocking APIs debug_assert!(!on_ui_thread())

pub struct CancelToken { .. }   // cancel(), is_cancelled(), generation, deadline/stall, receiver()
pub mod color { pub fn hdr_to_sdr(rgb: [f32; 3], tf: HdrTransfer, peak_nits: f32) -> [f32; 3]; } // pure BT.2390 path (§5.4)

// probe (MediaInfo and its stream types are plain data in freecut-timeline::model::library)
pub fn probe(path: &Path, cancel: &CancelToken) -> Result<MediaInfo, ProbeError>;
pub fn fingerprint(path: &Path) -> std::io::Result<Fingerprint>;   // size + xxh3(first+last 1 MiB) (+ duration)

// video
pub enum SeekMode { Exact, Approximate }   // decoder skip settings only in Approximate and L0 filmstrips (§5.5)
pub enum DecoderProfile { Playback, Scrub, Thumbnail, Export, Analysis, Reverse }
pub enum HwDecodePolicy { Off, Auto }      // Auto for preview profiles from M3a; export stays software
pub struct DecoderKey { pub source: MediaId, pub stream: u32, pub variant: SourceVariant /* Original | Proxy */ }
pub struct DecoderPool { .. }   // leases; count + byte budget; idle close
impl DecoderPool {
    pub fn lease(&self, key: &DecoderKey, near: Tick, profile: DecoderProfile, cancel: &CancelToken)
        -> Result<Lease<'_>, DecodeError>;
    pub fn invalidate_source(&self, m: MediaId);
}
impl VideoDecoder {
    pub fn frame_at(&mut self, t: Tick, mode: SeekMode, cancel: &CancelToken) -> Result<Arc<VideoFrame>, DecodeError>;
}
pub enum Priority { Exact, Playback, Approximate, Lookahead, Background }
pub struct DecodeQueue { .. }   // priority workers on top of the pool; identical jobs merged
impl DecodeQueue { pub fn request(&self, job: DecodeJob) -> DecodeTicket;   // never blocks
                   pub fn prefetch(&self, schedule: PrefetchSchedule); }
pub struct VideoFrame {
    pub start: Tick, pub duration: Tick, pub layout: PixelLayout, pub size: (u32, u32),
    pub planes: Planes,                 // Arc'd AVFrame + plane views; no copy
    pub geometry: DisplayGeometry,      // rotation_cw, mirrored, sar, display size
    pub color: ColorInfo,               // resolved: tags → CL-002 defaults → PM-040 overrides
    pub alpha: AlphaMode, pub quality: FrameQuality /* Exact | Approximate | Concealed | Truncated */,
    pub variant: SourceVariant,
}
pub enum PixelLayout {                  // one enum for decode and encode layouts (C16)
    Yuv420P8, Yuv422P8, Yuv444P8, Nv12,
    Yuv420P16 { bits: u8 }, Yuv422P16 { bits: u8 }, Yuv444P16 { bits: u8 },   // LSB-aligned
    P016 { bits: u8 },                                                         // MSB-aligned (P010 = bits 10)
    Yuva420P8, Yuva444P16 { bits: u8 }, Rgba8, Rgba16,
}
pub struct ReverseReader { .. }   pub struct GopIndex { .. }
pub fn to_rgba(frame: &VideoFrame, max: Size) -> Result<RgbaImage, DecodeError>; // CPU path; HDR via color::hdr_to_sdr

// audio
pub struct AudioReader { .. }
impl AudioReader { pub fn read(&mut self, start: i64, out: &mut PlanarF32, cancel: &CancelToken)
                       -> Result<AudioReadReport, DecodeError>; }

// derived data (all regenerable, content-keyed)
pub struct CacheStore { .. }   // <cache>/v1, atomic publish, verified reads, LRU by category, pins
pub mod thumbs { pub fn poster(..); pub fn hover(..); }
pub mod strip  { pub fn build_chunk(..); }                // L0 / L1 / L2 filmstrip chunks
pub mod audio_pass { pub fn run(..) -> Result<AudioPassResult, DecodeError>; } // peaks + exact length (+ PCM for seek-unreliable streams)
pub mod peaks  { pub fn level_for(samples_per_px: u64) -> u8; }
pub mod proxy  { pub fn encode(..); pub fn verify(..); pub fn find_proxy(..); }
pub mod status { pub fn check(..) -> MediaStatus; pub fn validate_candidate(..) -> Match; }
pub fn detect_sequences(paths: &[PathBuf]) -> Vec<SequenceCandidate>;   // pure

// encode and mux (export-pipeline §7)
pub mod encode {
    pub struct EncodeSpec { .. }   pub enum EncoderImpl { X264, X265, SvtAv1, Vp9, ProresKs, Gif, H264VideoToolbox, .. }
    pub fn encoder_setup(enc: EncoderImpl, rc: &ResolvedRateControl, fps: FrameRate, speed: EncoderSpeed, pass: Pass) -> EncoderSetup;
    pub struct EncoderCaps { .. }   impl EncoderCaps { pub fn probe(..) -> Self; }   // hw in a child process
    pub trait Muxer { fn video_frame(&mut self, f: &EncodeFrame) -> Result<(), EncodeError>;
                      fn audio_block(&mut self, b: &AudioBlock) -> Result<(), EncodeError>; }
    pub struct EncodeSession { .. }
    impl EncodeSession { pub fn open(spec: &EncodeSpec, part: &Path) -> Result<Self, EncodeError>;
                         pub fn finish(self) -> Result<MuxStats, EncodeError>;   // flush all, trailer, checked avio_closep
                         pub fn abandon(self);                                    // cancel: no flush, no trailer
                         pub fn frames_muxed(&self) -> u64; }
    pub fn mux_loop<M: Muxer>(m: &mut M, video: Option<&Receiver<EncodeFrame>>, audio: Option<&Receiver<AudioBlock>>,
                              expect: Expect, cancel: &CancelToken) -> Result<(), MuxLoopError>;
    pub struct SubtitleStreamWriter { .. }   // owned by EncodeSession; advance_to(video_pts)
    pub fn verify_output(path: &Path, expect: &Expect) -> Result<(), EncodeError>;
}
pub struct EncodeFrame { pub index: u64, pub layout: PixelLayout, pub planes: PooledBuf, pub geometry: FrameGeometry }
pub struct AudioBlock  { pub first_sample: u64, pub frames: u32, pub data: PooledBuf /* f32 interleaved */ }
```

Errors: `MediaInitError`, `ProbeError` (PM-037 variants plus `NotMedia`), `DecodeError`,
`CacheError`, `ThumbError`, `ProxyError`, `EncodeError`, `MuxError`, `MuxLoopError`.

### 3.3 `freecut-render`

```rust
pub struct GpuContext { pub device: wgpu::Device, pub queue: wgpu::Queue, pub caps: GpuCaps } // from eframe's RenderState (§2.3)
pub struct ShaderLibrary;  pub struct EffectRegistry;  pub struct TransitionRegistry;
pub struct Compositor { .. }
impl Compositor {
    pub fn new(gpu: GpuContext, cfg: RenderConfig, lib: Arc<ShaderLibrary>) -> Result<Self, RenderError>;
    pub fn render(&mut self, plan: &FramePlan, target: &RenderTarget, out: Output<'_>,
                  frames: &dyn FrameSource, cancel: &CancelToken) -> Result<FrameReport, RenderError>;
}
pub enum Output<'a> { Present(&'a PresentSlot), Export(&'a mut ExportFrame), Still(&'a mut StillImage),
                      Texture(&'a wgpu::TextureView) }
pub trait FrameSource: Send + Sync {          // implemented over DecodeQueue; tests use synthetic frames
    fn frame(&self, asset: MediaId, at: Tick, mode: SeekMode) -> Result<Arc<VideoFrame>, DecodeError>;
}
pub struct RenderTarget { pub size: (u32, u32), pub proxy_policy: ProxyPolicy, pub quality: Quality,
                          pub captions: CaptionBurnPolicy }
pub struct FrameReport { pub warnings: SmallVec<[RenderWarning; 2]>, pub gpu_time: Option<Duration> }
pub trait Effect: Send + Sync + 'static {
    fn manifest(&self) -> &'static EffectManifest;                   // type from freecut_timeline::effects
    // no migrate here: parameter migration is pure and runs in freecut-project's open pipeline (C37)
    fn encode(&self, b: &mut PassBuilder<'_>, input: ResId, fx: &EffectInput<'_>) -> Result<ResId, RenderError>;
}
pub fn yuv_to_rgb_affine(m: YuvMatrix, r: YuvRange, bits: u32, p: Packing) -> [[f64; 4]; 3];
pub fn transition_progress(elapsed: Tick, frame: Tick, duration: Tick) -> f64;  // (2e + f) / 2D, exact

pub mod preview {
    pub struct PreviewEngine { .. }
    impl PreviewEngine {
        pub fn spawn(gpu: GpuContext, frames: Arc<dyn FrameSource>, clock: Arc<dyn PlaybackClockRead>,
                     repaint: Box<dyn Fn() + Send + Sync>, cfg: PreviewConfig) -> Result<Self, RenderError>;
        pub fn set_document(&self, snap: DocSnapshot);
        pub fn request(&self, req: PreviewRequest);                  // Idle | Still(FrameRequest) | Play { plan, target }
        pub fn take_due(&self, present_at: PlayTime) -> Option<PresentedFrame>;   // UI thread, non-blocking
        pub fn take_still(&self) -> Option<PresentedFrame>;
        pub fn ui_frame_finished(&self);
        pub fn stats(&self) -> PlaybackStats;
    }
    pub enum PreviewScale { Auto, Full, Half, Quarter }
    pub fn preview_target_size(canvas: (u32, u32), viewer_px: (u32, u32), s: PreviewScale) -> (u32, u32);
}
pub mod export { pub struct ExportRenderer;      // shared device in the MVP (C19); run(out, cancel, thumbs)
                 pub struct OutputConverter;     // OutputColor → planes + tags
                 pub struct ReadbackRing;        // 3 slots, timed waits
                 pub struct CanvasToOutput; }    // Fit/Fill folded into the root transform
pub mod text   { pub struct TextEngine; pub struct TextService; pub struct FontSnapshot;
                 pub struct EditActor; pub struct LaidOutText; }
```

The clock trait itself lives in `freecut-audio`, which render may not depend on. Render takes a
narrow `PlaybackClockRead` trait object that `freecut-app` adapts from the audio clock (§6, C36).

Errors: `RenderError` (superset of the studies' variants, C29), `RenderWarning` (non-fatal:
bypassed effect, dissolve fallback, missing LUT, slate used), `FontError`, `TextRenderError`.

### 3.4 `freecut-audio`

```rust
pub fn compile(view: &MixView<'_>, opts: &CompileOptions) -> Result<MixPlan, PlanError>; // opts.rate = project or export rate
pub struct MixPlan { pub doc_version: u64, pub rate: SampleRate, pub master: MasterSpec,
                     pub buses: Vec<BusSpec>, pub voices: Vec<VoiceSpec>, pub end: i64, .. }
pub struct MixEngine { .. }
impl MixEngine {
    pub fn render(&mut self, plan: &MixPlan, pos: i64, mode: &RenderMode<'_>, out: &mut StereoBuf) -> RenderReport;
}
pub enum RenderMode<'a> { Playback { monitor: &'a MonitorState }, Scrub { monitor: &'a MonitorState },
                          Export { as_heard: Option<&'a MonitorState> }, Stem { bus: BusIdx },
                          Analysis { clip: ClipId, upto: ChainStep } }
pub struct MonitorState { pub solo: SmallVec<[TrackId; 4]>, pub silence_timeline: bool }   // view state

pub trait PcmProvider: Send + Sync {
    // Playback: returns at once; media's look-ahead thread opens and seeks, reads are NotReady until then (§2.5).
    fn open(&self, key: &PcmKey, mode: ReadMode /* Playback | Blocking */) -> Result<Box<dyn PcmReader>, SourceError>;
}
pub trait PcmReader: Send {
    fn native_rate(&self) -> u32;
    fn exact_len(&self) -> Option<u64>;
    fn read(&mut self, frame: i64, out: &mut [f32]) -> Result<Fill /* Full | NotReady */, SourceError>;
    fn prefetch(&mut self, frames: std::ops::Range<i64>);
}

pub trait PlaybackClock: Send + Sync { fn reading(&self) -> ClockReading; fn now(&self) -> Tick; }
pub struct ClockReading { pub epoch: Epoch, pub played: PlayTime, pub timeline: Tick, pub state: ClockState }
pub struct AudioClock;  pub struct MonotonicClock;     // device-frame stamps / no-device fallback
pub struct AudioEngine { .. }   // control handle for the feeder and the device
impl AudioEngine {
    pub fn start(cfg: OutputConfig, pcm: Arc<dyn PcmProvider>) -> Result<Self, AudioError>;
    pub fn publish(&self, snap: DocSnapshot);           // compile worker, latest wins
    pub fn play(&self, plan: Arc<PlayPlan>);  pub fn open_gate(&self);  pub fn hold(&self);  pub fn release(&self);
    pub fn stop(&self);  pub fn set_preview_mute(&self, on: bool);  pub fn set_monitor(&self, m: MonitorState);
    pub fn clock(&self) -> Arc<dyn PlaybackClock>;  pub fn meters(&self) -> MeterReader;
}
pub struct OfflineRenderer<'a> { .. }   // export, stems, analysis: same MixEngine, blocking sources
pub struct OfflineMix { .. }            // impl Iterator<Item = Result<AudioBlock, AudioError>>
pub trait TimeStretch: Send {           // v1 keep-pitch and shuttle (C9)
    fn reset(&mut self);  fn input_latency(&self) -> usize;  fn output_latency(&self) -> usize;
    fn preroll(&mut self, history: &[f32], time_ratio: f64);
    fn set_params(&mut self, time_ratio: f64, pitch_semitones: f64);
    fn process(&mut self, input: &[f32], output: &mut [f32]) -> Result<(), StretchError>;
}
```

Errors: `AudioError` (includes `NoOutputDevice` and `Device(cpal::Error)`), `PlanError`,
`SourceError`, `StretchError`. Run-time conditions on the real-time path are counters in
`RenderReport` or atomics, never `Result`s.

### 3.5 `freecut-project`

```rust
pub fn open(path: &Path) -> Result<Opened, OpenError>;   // read → migrate → typed decode → validate/repair → map
pub fn read_header(path: &Path) -> Result<ProjectHeader, OpenError>;   // 64 KiB prefix
pub struct Opened { pub document: Document, pub ids_next: u64, pub unknown: UnknownFields,
                    pub mode: OpenMode, pub warnings: Vec<OpenWarning>, pub provenance: Provenance,
                    pub recovery: Option<RecoveryInfo> }
pub struct ProjectSnapshot { pub doc: Arc<Document>, pub ids_next: u64, pub unknown: Arc<UnknownFields>,
                             pub generation: u64, pub resolved: Arc<BTreeMap<MediaId, PathBuf>> }
pub fn save(snap: &ProjectSnapshot, target: &SaveTarget<'_>, clock: &dyn Clock) -> Result<Saved, SaveError>;
pub fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<FileStamp, WriteError>;
pub mod migrate { pub const CURRENT_FORMAT_VERSION: u32 = 1; pub const WRITE_COMPAT: u32 = 1;
                  pub fn migrate_to_current(doc: serde_json::Value, from: u32) -> Result<Migrated, MigrationError>;
                  pub struct Chain<T>; }      // reused by looks, templates, presets, the export queue
pub mod lock { pub fn inspect(..) -> Result<LockStatus, LockError>; pub fn acquire(..) -> Result<ProjectLock, LockError>; }
pub mod backup { pub fn prune(policy: &Policy, now: OffsetDateTime, entries: &[BackupEntry]) -> Vec<BackupId>; }
pub mod recovery { /* write, find, assess (revision-based), discard */ }
pub mod paths { pub fn relative_to(base: &PathRepr, target: &PathRepr, same_volume: bool, max_up: usize) -> Option<RelPath>;
                pub fn resolve(sp: &StoredPath, cx: &ResolveContext<'_>, fs: &dyn FileSystem) -> Resolution; }
pub mod view { pub struct ViewState; pub struct PlayerUiState; }        // sidecar in app data, never in the project
pub mod export { pub struct ExportSettings; pub mod presets; pub mod compat { pub fn allowed(c: Codec, k: Container) -> bool; }
                 pub fn validate(..) -> Vec<Preflight>; pub fn output_dimensions(..); pub fn recommended_kbps(..);
                 pub fn sanitize_file_stem(..); pub fn estimate_size(..); }
pub mod subtitles { pub fn sniff(text: &str, ext_hint: Option<&str>) -> Option<SubtitleFormat>;
                    pub fn parse(text: &str, f: SubtitleFormat) -> Result<(SubtitleDoc, ParseReport), SubtitleError>;
                    pub fn write(doc: &SubtitleDoc, f: SubtitleFormat, o: &WriteOptions) -> Result<String, SubtitleError>; }
pub mod interop { pub struct InterchangeTimeline; pub struct InterchangeReport;
                  pub mod otio { pub fn read(..); pub fn write(..); } }   // later
```

The `StoredPath`, `PathRepr`, `Fingerprint` and `MediaInfo` types are plain data in
`freecut-timeline`; the project crate only maps, stores and compares them. Errors: `OpenError`,
`SaveError`, `MigrationError`, `LockError`, `WriteError`, `SubtitleError`, `OtioError`.

### 3.6 `freecut-app`

```rust
fn main() -> anyhow::Result<()>;   // eframe window; subcommands: `export` (later), `--probe-encoders`, `--media-worker`
struct App {
    editor: Editor,                                  // the only mutable document owner
    published: Arc<ArcSwap<DocSnapshot>>,
    player: Player,                                  // Transport + PreviewEngine + AudioEngine
    jobs: JobSystem,                                 // import/probe, media jobs, save coordinator, export queue
    gpu: GpuContext,                                 // from CreationContext::wgpu_render_state (WgpuSetup::CreateNew)
    memory: MemoryBudget,                            // one owner of every cache cap (§5.12)
    i18n: Localizer,                                 // Fluent catalog; tr!/tr_args! (§5.13)
    prefs: Preferences, view: ViewState,
}
pub enum Transport { Paused { at: Tick }, Prerolling { .. }, Playing { plan: Arc<PlayPlan> },
                     Buffering { .. }, Scrubbing { target: Tick, still_since: Option<Instant> } } // generic over PlaybackClock in tests
pub struct ExportJob; pub struct JobQueue; pub struct ProgressCell;   // export orchestration, persistence, progress
pub struct AutosaveScheduler;     // v1, pure state machine tested with a fake clock
// Error mapping: library errors → anyhow::Context → stable codes ("E-DISK-FULL") → Fluent message ids
// (`error-disk-full`) for the plain-language text; history labels map OpKind → `op-<kind>` with a count (§5.13).
```

---

## 4. Workspace `Cargo.toml` and dependencies

### 4.1 Manifest skeleton

**Current scaffold vs this design.** A workspace scaffold appeared in the repository while this
page was being written (root `Cargo.toml`, `clippy.toml`, `crates/*`, `.github/workflows/ci.yml`).
It matches the crate list, edges, edition, licence and test-time `unwrap` allowances. M0 closes
these differences:

| Item | Scaffold today | This design | Why |
|---|---|---|---|
| `rust-version` | `1.95` (was `1.88` when this page was first written) | `1.95` | eframe and egui-wgpu 0.36.2 declare 1.95, and so does `sysinfo` 0.39.6 |
| `freecut-audio` → `freecut-media` | optional (`dep:freecut-media` behind `media`) | unconditional with `default-features = false`; audio's `media` feature = `["freecut-media/ffmpeg"]`; media gets a default feature `ffmpeg` that gates `ffmpeg-next` and every FFI, decode, encode and probe module | `cargo test -p freecut-audio --no-default-features` must build without FFmpeg, and audio's public API names `CancelToken` and `AudioBlock` (§1.2) |
| `unsafe` lints | `unsafe_code = "deny"` workspace-wide | keep it; add `#![forbid(unsafe_code)]` in timeline, project, audio, app; `#[allow(unsafe_code)]` only on media's FFI modules; add `clippy::undocumented_unsafe_blocks = "deny"` | enforce `// SAFETY:` comments mechanically |
| `unwrap_used`, `expect_used`, `dbg_macro` | `warn` | `warn` is acceptable, because CI runs clippy with `-D warnings`; `deny` in this sketch is equivalent | — |
| `[profile.dev.package."*"] opt-level = 2` | present | keep | decoders and wgpu are unusable unoptimized |

```toml
[workspace]
resolver = "3"
members = [
  "crates/freecut-timeline", "crates/freecut-media", "crates/freecut-render",
  "crates/freecut-audio", "crates/freecut-project", "crates/freecut-app",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.95"          # eframe and egui-wgpu 0.36.2 declare 1.95 (read with `cargo info --offline`)
license = "GPL-3.0-or-later"

[workspace.dependencies]
# internal
freecut-timeline = { path = "crates/freecut-timeline" }
freecut-media    = { path = "crates/freecut-media" }
freecut-render   = { path = "crates/freecut-render" }
freecut-audio    = { path = "crates/freecut-audio" }
freecut-project  = { path = "crates/freecut-project" }

# fixed stack
eframe          = "0.36.2"
egui            = "0.36"
egui-wgpu       = "0.36.2"
wgpu            = "30.0.1"
ffmpeg-next     = { version = "9.0.0", default-features = false, features = ["codec", "format", "software-scaling", "software-resampling", "filter"] }
ffmpeg-sys-next = "9.0.1"
cpal            = "0.18.2"         # per-OS features in freecut-audio, see §4.2; macOS floor in §5.14
serde           = { version = "1.0.229", features = ["derive"] }
serde_json      = { version = "1.0.154", features = ["float_roundtrip"] }  # never arbitrary_precision / raw_value
thiserror       = "2.0.21"
anyhow          = "1"

# pre-1.0 stacks are pinned exactly and wrapped behind our own modules
parley          = { version = "=0.12.0", features = ["system", "complex-scripts"] }
rubato          = "~5.0.1"
# ... the rest of §4.2, one line each
```

```toml
# crates/freecut-media/Cargo.toml
[features]
default = ["ffmpeg"]
ffmpeg  = ["dep:ffmpeg-next", "dep:ffmpeg-sys-next", "dep:image", "dep:fast_image_resize"]

# crates/freecut-audio/Cargo.toml
[dependencies]
freecut-media = { workspace = true, default-features = false }   # CancelToken, AudioBlock, PooledBuf
[features]
default = ["media"]
media   = ["freecut-media/ffmpeg"]                               # + the PcmProvider adapter over media
```

```toml
# Every member crate:
[lints]
workspace = true

[workspace.lints.rust]
unsafe_op_in_unsafe_fn = "deny"

[workspace.lints.clippy]
undocumented_unsafe_blocks = "deny"   # every unsafe block carries `// SAFETY:`
unwrap_used = "deny"                  # allowed only with #[allow] + an `// INVARIANT:` comment
expect_used = "deny"
dbg_macro = "deny"
```

- `clippy.toml` at the root sets `allow-unwrap-in-tests = true` and
  `allow-expect-in-tests = true`. `crates/freecut-project/clippy.toml` repeats those keys and
  adds `disallowed-types` for `std::collections::HashMap` and `HashSet`, so DTOs stay
  deterministic (project-format.md).
- `#![forbid(unsafe_code)]` at the root of `freecut-timeline`, `freecut-project`,
  `freecut-audio`, `freecut-app` and, in v1, `freecut-render` (it needs no `unsafe`).
- `freecut-media` has `#![deny(unsafe_code)]` at the root and `#[allow(unsafe_code)]` only on
  `av/`, `encode/ffi.rs` and `encode/subtitle.rs`.

### 4.2 Dependency table

Only crates whose version and license a study checked are listed (plus the fixed stack).
"Used by": tl = timeline, md = media, rd = render, au = audio, pj = project, app.

**Fixed stack**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `eframe` | 0.36.2 | MIT OR Apache-2.0 | app | rust-version 1.95 |
| `egui` | 0.36 | MIT OR Apache-2.0 | app | same release line as eframe |
| `egui-wgpu` | 0.36.2 | MIT OR Apache-2.0 | app | `CallbackTrait`, `WgpuSetup::CreateNew` with our device descriptor (§2.3); built on wgpu 30.0 |
| `wgpu` | 30.0.1 | MIT OR Apache-2.0 | rd, app | same major as egui-wgpu, so textures are shared; one device per adapter (C19) |
| `ffmpeg-next` | 9.0.0 | WTFPL | md (feature `ffmpeg`) | see the avoid-list for its unsafe-to-use APIs; its enum-returning getters are not used on enum fields (§5.7) |
| `ffmpeg-sys-next` | 9.0.1 | WTFPL | md (feature `ffmpeg`) | raw FFI for the gaps; knows the `ffmpeg_8_1` cfg; bindgen turns C enums into Rust enums (`build.rs:1724-1726`), so enum fields are read as `c_int` (§5.7) |
| `cpal` | 0.18.2 | Apache-2.0 | au | output timestamps, `ErrorKind`. CoreAudio needs macOS 14.2 per its README (§5.14). Features: macOS none; Windows `realtime`; Linux `pipewire` + `realtime-dbus` (ALSA is always built), confirmed by the M3a latency and A/V test on a PipeWire and a PulseAudio desktop |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | pj; tl (optional feature) | |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | pj; dev elsewhere | `float_roundtrip` |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | all libraries | |
| `anyhow` | 1 | MIT OR Apache-2.0 | app only | |

**Concurrency and scheduling**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | app, rd, au | latest-wins publication |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | md, rd, au, app | `select!`, `recv_timeout` |
| `rtrb` | 0.4.0 | MIT OR Apache-2.0 | au, md | wait-free SPSC |
| `parking_lot` | 0.12.5 | MIT OR Apache-2.0 | md, rd | non-real-time mutexes, no poisoning |
| `quick_cache` | 0.7.0 | MIT | rd, md | byte-weighted concurrent caches |
| `thread-priority` | 3.1.1 | MIT | rd, au, app | best effort |
| `keepawake` | 0.6.1 | MIT | app | playback and export; held on the orchestrator thread |
| `pollster` | 1.0.1 | Apache-2.0 OR MIT | app (normal), rd (dev) | blocks on `rfd` dialog futures on a job thread, and on `enumerate_adapters`/`request_device` if export ever gets its own device (C19); no async runtime |

**Hashing, math, data**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `xxhash-rust` (`xxh3`) | 0.8.19 | BSL-1.0 | tl, md, rd, pj | render key, fingerprints, file stamps, the colour stack's `ColorDigest` (C52), the `LutRef` hash of LUT files (pj, CL-016) |
| `smallvec` | 1.16.3 | MIT OR Apache-2.0 | tl, md, rd, au | avoid the 2.0 beta |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | md, rd | `Pod` uniforms and planes |
| `glam` | 0.34.1 | MIT OR Apache-2.0 | tl, rd | `DMat3` homography in `FramePlan` |
| `half` | 2.7.1 | MIT OR Apache-2.0 | rd | f16 uploads |
| `num-rational` | 0.4.2 | MIT OR Apache-2.0 | tl | curve compilation and test oracle only; never for `Tick` |
| `num-integer` | 0.1.47 | MIT OR Apache-2.0 | tl | lcm, div_floor |
| `ethnum` | 1.5.3 | MIT OR Apache-2.0 | tl | I256 curve evaluation (0.11 µs measured) |
| `libm` | 0.2.16 | MIT | tl, rd | bit-identical easing across OSes; the CPU colour stack's transcendentals (C52) |

**Media, images, logging**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `image` | 0.25.10 | MIT OR Apache-2.0 | md; rd (dev) | stills, EXIF orientation, JPEG/PNG tiles; only needed formats |
| `fast_image_resize` | 6.1.0 | MIT OR Apache-2.0 | md | still downscale |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | md, pj; dev | atomic publish |
| `tracing` | 0.1.44 | MIT | all | logging (§5.6) |
| `tracing-subscriber` | 0.3.23 | MIT | app | the subscriber: `fmt` layer + `EnvFilter` |
| `tracing-appender` | 0.2.5 | MIT | app | rolling log file in app data (daily, keep 7); non-blocking writer |

**Audio**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `rubato` | 5.0.1 | MIT OR Apache-2.0 | au, md | device boundary, export and analysis rates; behind `dsp::resample`; new major, pin `~5.0` |
| `audioadapter-buffers` | 5.2.0 | MIT OR Apache-2.0 | au | rubato I/O adapters |
| `ebur128` | 0.1.10 | MIT | au | loudness; last release 2024-10, vendor if it stalls |
| `no_denormals` | 0.3.0 | MIT | au | flush-to-zero scope |
| `hound` | 3.5.1 | Apache-2.0 | au (dev) | test fixtures and debug dumps only; recording uses FreeCut's own WAV/RF64 writer (C53) |
| `realfft` | 3.5.0 | MIT | au | v1 spectral DSP |

**Text and Unicode**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `parley` (`system`, `complex-scripts`) | =0.12.0 | Apache-2.0 OR MIT | rd | layout, shaping, editing cursor; fontique via `parley::fontique` |
| `swash` | 0.2.10 | Apache-2.0 OR MIT | rd | unhinted coverage, colour glyphs |
| `skrifa` / `read-fonts` | 0.44 / 0.41 | MIT OR Apache-2.0 | rd | pinned to parley's versions |
| `sdf_glyph_renderer` (no default features) | 1.1.0 | BSD-3-Clause | rd | glyph SDFs |
| `etagere` | 0.3.0 | MIT OR Apache-2.0 | rd | atlas allocation |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 | tl | grapheme boundaries |
| `icu_segmenter` | 2.3.0 | Unicode-3.0 | tl | caption re-flow, same engine as parley |
| `icu_casemap` | 2.3.0 | Unicode-3.0 | tl, rd | locale-aware display case |
| `icu_properties` | 2.3.0 | Unicode-3.0 | rd | joining type, script, emoji properties |
| `unicode-normalization` | 0.1.25 | MIT OR Apache-2.0 | tl, rd, pj | NFC for search, font names, relink |
| `encoding_rs` | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause | app | subtitle file decoding |
| `chardetng` | 1.0.0 | Apache-2.0 OR MIT | app | encoding detection over the whole file |
| `isolang` | 2.4.0 | Apache-2.0 | md | BCP 47 → ISO 639-2 stream tags |

**Project format and app**

| Crate | Version | License | Used by | Notes |
|---|---|---|---|---|
| `serde_path_to_error` | 0.1.20 | MIT OR Apache-2.0 | pj | JSON path in schema errors |
| `uuid` (`v4`, `serde`) | 1.28.0 | Apache-2.0 OR MIT | pj | project and session ids |
| `time` | 0.3.55 | MIT OR Apache-2.0 | pj | RFC 3339 timestamps, backup day buckets |
| `base64` | 0.23.1 | MIT OR Apache-2.0 | pj | non-UTF-8 path encodings |
| `url` | 2.5.8 | MIT OR Apache-2.0 | pj | OTIO file URLs |
| `semver` | 1.0.28 | MIT OR Apache-2.0 | pj | writer quirk ranges |
| `gethostname` | 1.1.0 | Apache-2.0 | pj | lock metadata |
| `dunce` | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 | pj | Windows path comparison |
| `directories` | 6.0.0 | MIT OR Apache-2.0 | app | app-data and cache roots, passed into libraries |
| `notify` | 8.2.0 | CC0-1.0 | app | file watching (PM-093, v1); 9.0 is still a release candidate |
| `walkdir` | 2.5.0 | Unlicense OR MIT | app | folder import |
| `rfd` | 0.17.2 | MIT | app | native file dialogs, `AsyncFileDialog` only (§2.3 rule 6); default features (`xdg-portal`, `wayland`) on Linux |
| `sysinfo` (`default-features = false`, `features = ["system"]`) | 0.39.6 | MIT | app | total and available RAM for `MemoryBudget` (§5.12); MSRV 1.95; queried on a job thread (it reads `/proc` on Linux) |
| `fs4` (`default-features = false`) | 1.1.0 | MIT OR Apache-2.0 | app | `fs4::available_space` for EX-036 preflight and PM-113 only; file locks stay on `std` (C22) |
| `fluent-bundle` | 0.16.0 | Apache-2.0 OR MIT | app | UI-044 string catalog; `concurrent::FluentBundle`; bidi isolation off for LTR |
| `fluent-langneg` | 0.13.1 | Apache-2.0 OR MIT | app | language negotiation; the version `fluent-bundle` 0.16.0 itself depends on (0.14.2 moved to `icu_locid` 1.x, a second locale type and a second ICU4X major next to our 2.3 crates) |
| `unic-langid` | 0.9.6 | MIT OR Apache-2.0 | app | the `LanguageIdentifier` type both Fluent crates above use |
| `zip` | 9.0.1 | MIT | pj | package archive (v1, optional) |
| `quick-xml` | 0.42.0 | MIT | pj | FCPXML (later) |

**Dev-only**

| Crate | Version | License | Used by |
|---|---|---|---|
| `proptest` | 1.12.0 | MIT OR Apache-2.0 | all |
| `insta` | 1.49.0 | Apache-2.0 | all |
| `pretty_assertions` | 1.4.1 | MIT OR Apache-2.0 | tl |
| `approx` | 0.5.1 | Apache-2.0 | tl, au |
| `loom` | 0.7.2 | MIT | au, rd |
| `assert_no_alloc` | 1.1.2 | BSD-1-Clause | au |
| `naga` | 30.0.1 | MIT OR Apache-2.0 | rd (WGSL validation, uniform layout tests) |
| `image-compare` | 0.5.0 | MIT | rd |
| `schemars` | 1.2.2 | MIT | pj (schema-hash lock test) |
| `similar-asserts` | 2.0.0 | Apache-2.0 | pj |
| `arbitrary` / `cargo-fuzz` | 1.5.0 / 0.13.2 | MIT OR Apache-2.0 | pj (subtitle parsers), md (probe), tl (later) |
| `test-strategy` | 0.4.5 | MIT OR Apache-2.0 | optional proptest derives |

**Optional or later, adopted only when the trigger happens**

| Crate | Version | License | Trigger |
|---|---|---|---|
| `signalsmith-stretch` | 0.1.3 | MIT (C++ build) | v1 keep-pitch default candidate, after a listening test |
| `imbl` | 7.0.2 | MPL-2.0+ | profiling shows `Vec<Arc<Clip>>` copy-on-write cost on very large tracks |
| `rustc-hash` | 2.1.3 | Apache-2.0 OR MIT | profiling of `TimelineIndex` |
| `naga_oil` | 0.23.0 | MIT OR Apache-2.0 | the `concat!` shader prelude becomes unwieldy |
| `lyon_tessellation` | 1.0.22 | MIT OR Apache-2.0 | Bézier masks (TR-021) |
| `imagequant` + `gif` | 4.4.1 + 0.14.2 | GPL-3.0-or-later; MIT OR Apache-2.0 | a build without libavfilter needs a GIF fallback |
| `rubberband` | 0.1.5 | GPL-2.0-or-later | only as an optional engine; see licence notes |
| `bungee-rs` | 0.1.1 | MPL-2.0 | benchmark candidate for keep-pitch |
| `whisper-rs` (+ `whisper-rs-sys`) | =0.16.0 (=0.15.0, bundles whisper.cpp 1.8.3) | Unlicense (whisper.cpp MIT) | captions slice CS2: `freecut-analysis` feature `ml-whisper` (C47); C++ build with CMake; features `metal` on macOS, `tracing_backend`; `vulkan` only in release builds that have the SDK |
| `ureq` | 3.4.2 | MIT OR Apache-2.0 | model downloads in the app (C49, CS2) and the assistant's `http` feature (C45); rustls TLS; MSRV 1.85 |
| `sha2` | 0.11.1 | MIT OR Apache-2.0 | model file verification in `freecut-analysis` (C49, CS2); MSRV 1.88 |

The 2026-10-12 review added `rfd`, `pollster` (as a normal dependency), `tracing-subscriber`,
`tracing-appender`, `sysinfo`, `fs4`, `fluent-bundle`, `fluent-langneg` and `unic-langid`; their
versions, licences and MSRVs were read with `cargo info` (the two `tracing-*` crates and the
`fluent-langneg` 0.13 line online, the rest from the local registry). A licence-audit tool (for
example `cargo-deny`) is still chosen and verified at M0.

### 4.3 License review for GPL-3.0-or-later

- **Compatible without remarks:** MIT, Apache-2.0 (GPLv3 is compatible with Apache-2.0),
  BSD-1/3-Clause, Zlib, BSL-1.0, CC0, Unicode-3.0, Unlicense, MIT-0. Keep their notices in the
  generated third-party notice file.
- **WTFPL** (`ffmpeg-next`, `ffmpeg-sys-next`): permissive and compatible. The wrapper is
  community-maintained, so all use stays behind `freecut_media::av` (`rsmpeg` and
  `ffmpeg-the-third` are the fallback plans).
- **FFmpeg itself.** The local Homebrew 8.1.1 build is `--enable-gpl --enable-version3`, i.e.
  GPLv3, which is fine for FreeCut. Shipping it brings source-offer obligations, and libx264,
  libx265 and HEVC raise patent questions per platform. **FDK-AAC is never used** (non-free,
  GPL-incompatible); the native `aac` encoder is used. Every codec choice is capability-driven
  (§8).
- **MPL-2.0** (`imbl`, `bungee-rs`): compatible through MPL's secondary-licence clause;
  file-level copyleft applies to modifications of those files.
- **GPL-2.0-or-later** (`rubberband` bindings and librubberband): compatible, but it needs the
  system library, and Rubber Band's own terms forbid App Store distribution without a
  commercial licence. Optional only.
- **GPL-3.0-or-later** (`imagequant`): compatible; optional fallback.
- **Avoid:** `gifski` (AGPL-3.0-or-later, network-use obligations).
- **Studied code, not dependencies.** Kdenlive is GPL-3.0-only OR KDE-accepted later versions.
  Porting its code would pin those parts to GPL-3.0-only, so it is used for ideas only. Shotcut
  and Olive (GPL-3.0-or-later), MLT (LGPL-2.1-or-later; `movit` and `interp.h`
  GPL-2.0-or-later), libopenshot (LGPL-3.0-or-later) and OTIO (Apache-2.0) are compatible, but
  no code is ported. OTIO test vectors may be adapted with attribution (not its roll, ripple and
  slide vectors). Kdenlive's CC0 luma maps and effect XML data may be reused with attribution.
- **Fonts.** Bundled Noto fonts are OFL. Modified test subsets must not use a reserved font name.

### 4.4 Not adopted

| Crate or approach | Reason (study) |
|---|---|
| `fs2`; `fs4`'s lock traits | `std::fs::File::lock`/`try_lock` are stable since Rust 1.89 (project-format; C22). `fs4` is adopted only for `available_space` |
| `memmap2` | `Mmap::map` is an `unsafe fn` (`memmap2-0.9.11/src/lib.rs:762`), and its safety condition (the file is not truncated or changed while mapped) depends on other FreeCut processes evicting the cache; deleting a mapped file also fails on Windows. Peak pyramids (about 7 MB per hour) are read into memory; conform PCM uses positioned reads (`std::os::unix::fs::FileExt::read_at`, `std::os::windows::fs::FileExt::seek_read`) |
| Pinning `cpal` 0.17.3 for older macOS | 0.17.3 references `AudioHardwareCreateProcessTap` too (`cpal-0.17.3/src/host/coreaudio/macos/loopback.rs:11,106`), so it does not lower the floor (§5.14) |
| `lru` | `quick_cache` covers byte-weighted caches; frame rings are a `VecDeque` (C23) |
| `slotmap`, random UUIDs for entity ids | keys can't be re-inserted, so undo would change ids; nondeterministic goldens (timeline-model) |
| `undo` crate, `im` | command-pattern history; `im` superseded by `imbl` (edit-ops) |
| `vtc`, `timecode`, any f64-seconds time crate | own `Tick`-based timecode, ~100 lines (timeline-model) |
| `tokio`, `rayon` for playback | no priorities or real-time guarantees (playback-engine) |
| `rodio`, `kira`, `dasp`, `fundsp`, `ringbuf`, `audio_thread_priority` | own the clock or duplicate chosen crates (audio-engine) |
| `symphonia` | a second decoder with different priming and edit-list rules (media-io, audio-engine) |
| `triple_buffer` | holds one value; present ring needs N slots with deadlines (playback-engine) |
| `glyphon`, `cosmic-text`, `fdsm`, `vello` (now) | no per-glyph effects or union outlines; second shaper; MSDF breaks union (text) |
| `rand` distributions | not reproducible across releases (text) |
| `subparse` and other subtitle crates | missing formats and reports; use only as test oracles (text) |
| `chrono`, `atomic-write-file`, binary project formats, `json5` | (project-format, export-pipeline). `sysinfo` was on this list; it is now adopted in the app for RAM queries only (§5.12) |
| `openh264`, `rav1e` as separate encoders, the `ffmpeg` CLI | one encode path through FFmpeg in-process (export-pipeline) |
| OCIO via FFI, OpenCV, frei0r, avfilter for rendering | breaks preview = export determinism (render-compositor) |

---

## 5. Cross-cutting decisions

### 5.1 Time

- **One time type.** `Tick(i64)` at 1/705,600,000 s for every instant and span in sequence,
  source and play time. Every common frame duration (23.976 to 120 fps, NTSC included) and every
  common audio sample period (8 kHz to 192 kHz) is a whole number of ticks. `i64` spans about
  414 years; JSON values stay below 2^53 (about 148 days).
- **No `Add`/`Sub` on `Tick`.** All arithmetic is `checked_*` and returns
  `TimeError::Overflow`; products use `i128`. Speed curves are evaluated exactly in 256-bit
  integers (`ethnum::I256`) with a BigInt fallback, because `i128` is not enough (C3 in
  keyframes-and-speed.md).
- **Ratios** (speed, aspect) are an exact `Rational`; it never has a constructor from `f64`.
- **One rounding rule per conversion**, named at the call site:

  | Conversion | Rule |
  |---|---|
  | Edit point → frame grid | nearest, ties to the earlier frame |
  | Stream timestamp → tick (`ticks_from_stream`) | nearest, ties to even; inverted only by `stream_ts_at_or_before` |
  | Subtitle ms/cs → tick | exact (1 ms = 705,600 ticks) |
  | Tick → frame index, tick → sample index | floor |
  | Trim: timeline length → source length | floor |
  | Speed change: source length → timeline length | `max(1 frame, nearest_frame(D / r))` |
  | Export frame `n` | `start + floor(n · TPS · den / num)` in `i128` |
  | Subtitle export | total rounded half up to ms first, then split into fields |
- **Media identity is the PTS converted to ticks.** Frame choice compares ticks, never stream
  timestamps; timebases such as 1/15360 and `AV_TIME_BASE` do not divide the tick rate.
- **Play time is not timeline time.** `PlayTime` counts real playback since an epoch began;
  `PlayPlan` maps it to `Tick` (shuttle, loop wrap, end). Loops are planned, never detected.
- **Floats** appear only at the UI edge (pixels, `as_secs_f64`), in signal and gain values,
  in shader uniforms, and in progress fractions computed from integers (`transition_progress`,
  keyframe progress).

### 5.2 Ids

- One document-wide monotonic allocator for every entity kind (`SeqId`, `TrackId`, `ClipId`,
  `MediaId`, …). Ids are `NonZeroU64` newtypes, always below 2^53. Caption segment ids are
  `ClipId`s.
- The allocator (`IdAlloc`) lives in `Editor`, not in `Document`, so structural equality ignores
  it. Ids are allocated at **planning** time through an `IdCursor` and replayed exactly on redo.
  Undo never rewinds the counter, so ids are never reused.
- `freecut-project` persists `ids.next`. On load it uses `max(stored next, max id + 1)`.
- Split keeps the original id on the **left** part. Paste, duplicate, OTIO import and template
  merges remap every id through `IdRemap`. Compound clips deep-copy nested sequences with fresh
  ids.
- Each relation is stored once (`Clip.group`, `Clip.link`, `Clip.attach`); reverse maps are
  derived. Selection is app state, recorded on history entries but never in the document.
- The project id is a UUID v4, regenerated for copies, packages and duplicates.

### 5.3 Errors

| Crate | Error types | Notes |
|---|---|---|
| timeline | `TimeError`, `EditError`, `ApplyError`, `InvariantError`, `HistoryError`, `CommitError`, `RetimeError`, `KeyframeError`, `RichTextError`, `ExportTimeError` | `ApplyError` and `InvariantError` mean a bug: logged, document untouched |
| media | `MediaInitError`, `ProbeError`, `DecodeError`, `CacheError`, `ThumbError`, `ProxyError`, `EncodeError`, `MuxError`, `MuxLoopError` | `DecodeError::Cancelled` is control flow, never shown |
| render | `RenderError`, `RenderWarning`, `FontError`, `TextRenderError` | per-item problems degrade (bypass, dissolve, identity LUT, slate) and are warnings |
| audio | `AudioError`, `PlanError`, `SourceError`, `StretchError` | real-time conditions are counters |
| project | `OpenError`, `SaveError`, `MigrationError`, `LockError`, `WriteError`, `SubtitleError`, `OtioError` | errors name the JSON path |
| app | `anyhow` with context, `JobError` | mapped to stable codes and plain text; toast wording lives here |

Rules: `thiserror` in every library, `anyhow` only in `freecut-app`. No `unwrap`/`expect`
outside tests unless the invariant is commented (clippy enforces it). Nothing panics on user
input or file content: paths are validated as UTF-8 before the `ffmpeg-next` calls that would
panic, and FFmpeg's leaky or panicking subtitle wrappers are bypassed. A no-op edit is not an
error: it returns a plan with no changes and creates no history entry. First error wins in
multi-threaded jobs; `Cancelled` and "disconnected" rank lowest so the root cause is reported.

### 5.4 Color pipeline

1. **Input** (`freecut-media`). Each frame carries a resolved `ColorInfo` (matrix, range,
   transfer, primaries, chroma location, alpha): container and frame tags, then CL-002
   defaults (BT.601 below 720 lines, BT.709 otherwise; limited range for YUV, full for JPEG and
   PNG; left chroma siting for 4:2:0), then PM-040 overrides. `DisplayGeometry` (rotation,
   mirroring, SAR) travels with it. Media computes them once; it never rotates or converts 4K
   frames on the CPU.
2. **Normalize (L2, GPU).** Planes are uploaded as `R8Unorm`/`Rg8Unorm`, or `R16Unorm` when
   available (`R16Uint` with manual bilinear otherwise). One 3×4 affine computed and
   unit-tested in Rust handles matrix, range, bit depth and packing; the frame's own chroma
   siting is honoured. wgpu's `NV12`/`P010` formats are not used (unavailable on Metal).
   Straight alpha is premultiplied once, here.
3. **Working space.** Premultiplied `Rgba16Float`, Rec.709 primaries, display-referred
   (gamma-encoded) code values. Linear light is used only inside exposure, white balance,
   gamut and HDR conversion, and frame blending (TR-041), with a pure γ 2.4 decode/encode.
   Dissolves, wipes, blend modes, resampling, keys, LUTs and grades work on code values.
   `*Srgb` texture formats are never used in the pipeline.
4. **Blend modes.** Normal is fixed-function premultiplied source-over. The 15 others follow
   the W3C Compositing Level 1 formulas on un-premultiplied, clamped colour, applied exactly once
   at L10.
5. **HDR input** (PQ/HLG): EOTF to linear, divide by 203 cd/m², BT.2390 EETF tone map, 2020 →
   709 gamut matrix with soft clip, γ 2.4 encode, all in the L2 variant. This is CL-002 (mvp):
   iPhone's default capture is 10-bit HLG HEVC, so the variant ships in M3a with a CPU
   reference (`freecut_media::color::hdr_to_sdr`, pure Rust) and GPU goldens, and M2 generates
   HLG and PQ fixtures. An HDR working space and HDR export come later through a `WorkingSpace`
   enum read by three shader helpers.
6. **Preview output.** Accumulator → `Rgba8Unorm` present slot with ordered dither. The egui
   paint callback has a second pipeline variant that linearizes when the swapchain is sRGB, so
   the image isn't encoded twice. The RAM frame cache (v1) stores the same RGBA8 values.
7. **Export output.** BT.709 matrix, limited range, 4:2:0 left-sited chroma filtered `[1,2,1]/4`
   horizontally and averaged vertically, a deterministic dither seeded by the frame index
   (8-bit only), codes clamped to [1, 254]. The stream tags (1/1/1, tv, chroma location left)
   and the `colr` atom come from the **same** `OutputColor` value that picks the shader
   constants, so pixels and tags cannot disagree.
8. **CPU paths** (thumbnails, AI): `freecut_media::to_rgba` uses FFmpeg 8's dynamic
   `sws_scale_frame` with the resolved colour fields (the legacy ffmpeg-next scaler defaults to
   BT.601 and cannot be configured). For PQ/HLG sources swscale does **only** the matrix and
   chroma step: the destination keeps the source's primaries and transfer (RGB48, still
   PQ/HLG-encoded), and `color::hdr_to_sdr`, the same function the shader's golden is checked
   against, does EOTF → EETF → gamut → γ 2.4. swscale's `intent = Perceptual` tone mapping is not
   used, so the media panel and the timeline match the viewer and the export (CL-002 names
   thumbnails as following the same rule).
9. **Text and UI colours** are sRGB `Rgba8` with straight alpha in the model and are converted
   into the working space once, when uniforms are built.
10. **CPU renderer (C44, C52).** Until the GPU path exists, frames are straight-alpha RGBA8
    code values in the same Rec.709 display-referred space; the colour stack reads them as
    `f32`, uses linear light (γ 2.4) only where item 3 says, bakes its per-pixel stages into a
    lattice applied with tetrahedral interpolation, and quantizes once. Its results are the
    goldens the GPU working space above is checked against, within tolerance.

### 5.5 Determinism and caching

- **Preview = export.** The same `FramePlan`, shaders and mixer run in both. Preview may only
  lower resolution, drop frames, use proxies, use hardware decode (from M3a, §7), and use the
  `Approximate` scrub decode. Export decodes in software by default so files are reproducible
  across machines.
- **Decoder shortcuts never touch `Exact`.** `skip_loop_filter` and `skip_frame` are set only in
  `SeekMode::Approximate` and on the L0 filmstrip path, never in `Exact` mode and never in the
  `Export` or `Analysis` profiles. libavcodec applies the discard setting per packet when it
  is sent: frame threads copy it in `update_context_from_user`, called from `submit_packet`
  (`libavcodec/pthread_frame.c`, LGPL-2.1-or-later, read for behaviour only), and packets go in
  ahead of the frames that come out by the reorder delay plus the frame-thread lag. So "restore
  the filter when the output nears `t`" is too late: the target packet, and with the Export
  profile up to about `thread_count` packets after it, would already be decoded without
  deblocking. If the speed-up is wanted later, the choice is made per packet from
  `packet.pts` before `send_packet`: skip only packets whose pts is below
  `t − (reorder_depth + thread_count) × frame`. M2's plane-hash test (§7) guards this.
- **Content-keyed caches, no epoch flushes.** `render_key` is a stable 128-bit XXH3 over the
  canonical resolved `FramePlan` plus everything else that changes pixels: media fingerprints
  and proxy choice, slate state, analysis result ids, effect and shader versions,
  `RENDER_ABI_VERSION`, `TEXT_RASTER_VERSION`, the font environment digest, colour settings,
  the caption burn policy, live edit overrides, the decode path where it is not bit-exact
  (hardware vs software), and the **resolved pixel size**.
  - The key hashes only what affects pixels. `FramePlan.seq_version`, `time` and `frame_index`
    are **excluded** from the canonical serialization. Each node contributes the times it
    consumes: source sample ticks for video layers, transition `elapsed`/`duration`,
    clip-local ticks only for nodes whose output depends on local time (generated content,
    time-driven effects; `LayerPlan` marks them), and `frame_index` only when a node consumes a
    seed (noise, grain, random text order; adjustment layers seed by timeline frame index,
    CL-011). On a magnetic main track nearly every edit ripples; with `time` in the key, every
    shifted frame would miss the cache, and the same still at two sequence times would never
    share a key.
  - Undo, redo and unrelated edits (ripples included) then reuse cached frames automatically.
    A missing input would show stale frames, so a property test mutates every pixel-affecting
    plan field and checks the key changes, a second property checks that rippling an earlier
    clip leaves the shifted frames' keys unchanged, and a debug mode re-renders random cache
    hits and compares them.
  - The RAM frame cache that consumes these keys is v1 (EX-016); the MVP uses `render_key` for
    the paused re-request only, and releases Ready frames whose snapshot version changed.
- **Media-derived data** (thumbnails, filmstrips, peaks, GOP index, conform, proxies) is keyed
  by fingerprint + stream + native timestamps or sample indices, never by project frame rate or
  timing overrides.
- **Portable math.** Easing uses `libm`, never the platform libm; random animation order uses
  our own SplitMix64 with checked-in golden vectors; noise and grain hash
  `(seed, frame index, normalized cell)`. Golden tests run on all three OSes.
- **Audio.** Output sample `n` depends only on the plan and `n`: block-size, seek and split
  invariance are tested properties; summation order is fixed (bus order, then voice order).

### 5.6 Logging and diagnostics

- Libraries log through `tracing` and never print. `freecut-app` installs the subscriber,
  writes a rotating log file in app data, and offers "Copy details" on error dialogs.
- FFmpeg's `av_log` is routed into `tracing` by `freecut_media::av::log`. Messages from a
  probe's own format context are captured into that probe's diagnostics (how "moov atom not
  found" becomes `ProbeError::Incomplete`).
- Repeated problems are logged once per (source, 1 s range).
- The real-time callback never logs; it increments counters that the feeder or UI reports.
- No telemetry. Logs stay local. Exported files carry only whitelisted technical metadata
  (EX-040) — no user, host or path data.

### 5.7 `unsafe` and FFI policy

- `unsafe` exists only in `freecut-media`: `av/` (custom I/O, per-stream seek, index entries,
  side data, hardware decode and `get_format`, `sws_scale_frame`, `av_log`, filter graphs,
  capability table), `encode/ffi.rs` (codec tag, strict `avcodec_open2`, checked `avio_closep`,
  `avformat_query_codec`, hardware frames) and `encode/subtitle.rs`. Each block has a
  `// SAFETY:` comment; clippy's `undocumented_unsafe_blocks` is denied.
- `freecut-render` needs no `unsafe` in v1. Zero-copy hardware-decode interop (later) would add
  it there, measured first and with `// SAFETY:` comments.
- Every other crate is `#![forbid(unsafe_code)]`. Third-party crates keep their own `unsafe`.
- **libav version check.** `freecut_media::init()` compares the runtime `avcodec_version()`,
  `avformat_version()`, `avutil_version()`, `swscale_version()`, `swresample_version()` and
  `avfilter_version()` with the build-time `LIBAV*_VERSION_INT`. A different major, or a
  runtime **minor older** than the headers, refuses to start media with a clear error: the
  bindings would read struct fields the older library does not have.
- **Enum-typed fields are read as integers.** ffmpeg-sys-next 9.0.1 generates every C enum as
  a Rust enum (`.default_enum_style(bindgen::EnumVariation::Rust { non_exhaustive })`,
  `ffmpeg-sys-next-9.0.1/build.rs:1724-1726`; the `non-exhaustive-enums` feature only adds
  `#[non_exhaustive]`). A value added in a newer libav minor (a new `codec_id`, `color_trc`,
  `color_primaries`, `colorspace`, `chroma_location` or side-data type) is then an invalid
  discriminant, which is undefined behaviour as soon as it is read, including through
  ffmpeg-next getters such as `Parameters::id()` (`ffmpeg-next-9.0.0/src/codec/parameters.rs:54-55`).
  Inside `av/` every such field is read as a raw `c_int`
  (`ptr::addr_of!((*par).codec_id).cast::<c_int>().read()`) and mapped to our own enums with
  an `Unknown(i32)` arm; ffmpeg-next's enum-returning getters are not called on those fields.
  A unit test feeds out-of-range integers through each mapper.
- **Which FFmpeg runs.** Official builds bundle one pinned FFmpeg build per OS and require the
  exact major.minor they were built against (this settles part of §8 Q9). Distribution and
  source builds may run a newer minor than their headers, which the raw-integer rule above
  makes safe.

### 5.8 Testing approach

| Layer | What | Where |
|---|---|---|
| Pure unit and table tests | time and rounding tables, timecode vectors, YUV affine, blend formulas, fades, limiter, ExportClock, encoder option mapping, GIF timing, path math, backup pruning | every crate, no I/O |
| The op triple | every planner: planning doesn't mutate its input; result validates; undo restores the original exactly (ids included); redo reproduces the result | timeline |
| Property tests (`proptest`) | random op sequences against a full-snapshot oracle; frame-array oracle for off-by-one bugs; split invariance (video and audio); seek and block-size invariance; `PlayPlan` laps; snapping; project round trip and unknown-field survival; RichText invariants | timeline, audio, render, project |
| Golden snapshots (`insta`) | ASCII timelines and change sets, easing tables as f64 bits, `MediaInfo` per fixture, encoder setups, canonical project files (byte-identical on 3 OSes), writer output | all |
| Harnesses without devices | `FakeClock` transport, `FakeRenderer`, `VirtualOutput` (2 h at 29.97 fps: A/V within 1 frame) | app, render, audio |
| Concurrency | `loom` for the clock seqlock, present ring and ring flush; `assert_no_alloc` on the callback and steady-state mixing | audio, render |
| GPU goldens | 16–64 px scenes against the CPU reference (≤ 2/1023) and PNG goldens; lavapipe on Linux CI, Metal on macOS; skipped without an adapter unless `FREECUT_REQUIRE_GPU=1` | render |
| FFmpeg fixtures | generated at test time by our own encode path: frame-number barcode video, sample-index click audio, rotation, VFR, priming and edit lists, broken files; a few tiny CC0 "wild" files committed with a manifest; no third-party or CapCut material | media, export |
| Fuzzing | subtitle parsers, probe + first decode, edit-request interpreter (later) | project, media, timeline |
| Crash and fault | kill-during-save harness (200 kills), injected `atomic_replace` failures, `ENOSPC` muxer, cancel at random points | project, app |
| Nightly only | 1 h sync and drift, > 4 GB files, 8K, hardware decode/encode (`FREECUT_TEST_HW=1`) | media, export |
| Benchmarks (not gating) | `plan_move` on 10k clips < 2 ms; exact scrub p95 < 150 ms; mixer ≥ 20× real time for 32 voices; curve eval < 1 µs; 4K30 HEVC Main10 HLG at 1× on 8 GB machines (M3a) | per crate |

### 5.9 CI commands

Every change runs, on macOS, Windows and Linux:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p freecut-audio --no-default-features   # the mixer builds and tests without FFmpeg
```

- **FFmpeg is pinned per runner, never taken from the distro.** Distro packages do not match
  the design: Ubuntu 24.04's apt FFmpeg is 6.1 (libavcodec 60), which fails the init check
  (§5.7) and lacks the `ffmpeg_8_1` cfg the dynamic `sws_scale_frame` path relies on, and
  Windows has no pkg-config FFmpeg at all. M0 installs a GPL **shared** FFmpeg 8.1 build with
  headers on each runner (a cached source build, or for example BtbN's `gpl-shared` 8.1 builds
  on Linux and Windows), cached by version, and exports `FFMPEG_DIR` (read by
  `ffmpeg-sys-next-9.0.1/build.rs:1159`) or `PKG_CONFIG_PATH`. macOS uses the same pinned
  build rather than whatever Homebrew bottle is current (Homebrew FFmpeg 8.1.1 locally).
- Windows installs LLVM, because ffmpeg-sys-next runs `bindgen`, which needs libclang.
- Linux installs `libasound2-dev` (cpal needs ALSA headers even when another backend is used,
  `cpal-0.18.2/README.md`, "Linux Build Dependencies"), plus `libpipewire-0.3-dev` and
  `libdbus-1-dev` for the chosen cpal features (§4.2), next to the existing xcb, xkb and Wayland
  packages.
- A CI step on every OS asserts that `freecut_media::init()` reports libavcodec 62.x with the
  expected minor.
- The workflow file itself (`.github/workflows/ci.yml`) still installs only the GUI packages; M0
  makes it match this list.
- A Linux job runs the GPU goldens on lavapipe with `FREECUT_REQUIRE_GPU=1`; macOS runs them on
  Metal. A GL-backend job that keeps the GLES fallback working (render-compositor risk 10) is
  v1 (§7).
- The dependency guard test (M0) fails on any edge outside §1.2.
- WGSL validation and uniform-layout checks are ordinary tests (naga).
- Nightly: the long and hardware jobs above, and the fuzz targets.

Locally, before calling a change done: `cargo fmt --all`, then clippy and tests as above.

### 5.10 Clean-room rules

FreeCut reimplements behaviour described in our own words in `docs/features/`. No CapCut code,
assets, presets, fonts, icons, models or screenshots are used, and nothing under the CapCut
application is opened or traced. The studies read open-source editors for lessons and cite
paths; FreeCut code is written from scratch. Product branding never uses the CapCut name.

### 5.11 Document changes after open

`Editor` is the only mutator, and its API has only `plan`, `commit`, `undo` and `redo`. So:

- **Everything that must not be undoable runs before `Editor::open`.** That is the open
  pipeline in `freecut-project`: migrations, typed decode, load repairs (PM-013) and effect
  parameter migration (C37).
- **Every change after open is an ordinary labelled commit** through `Editor::plan`/`commit`,
  including background corrections: `OpKind::CorrectMediaLength` (AU-001: the exact-length
  scan shortens a source, so the `MediaInfo` duration and the out-points of clips past it are
  clamped in one entry), `UpdateMediaInfo`, and copied-media fix-ups (PM-039). This is
  edit-ops N8 ("background work that changes the document adds exactly one entry").
  Background commits during a gesture are re-planned after it (§8 Q11).
- **Why not a side channel.** A clamp applied outside history to a clip that appears in an
  undo entry makes the next undo's `before` check fail (`edit-ops-and-undo.md`, "Rules for
  `apply`"). That is `HistoryError::Diverged`, which clears the history from that point.
- AU-001 says "non-undoable model correction"; §6.1 asks its owner to change that to "its own
  undo entry". A test covers trim → background clamp → undo → undo → redo → redo, all `Ok`.

### 5.12 Memory budget

On an 8 GB unified-memory machine the per-cache defaults of the studies add up to about
6.6 GB before egui and the OS: `DecoderPool` 2 GiB (C12), `SourceFrameCache` 512 MB,
`RamFrameCache` 25 % of RAM = 2 GB, `ImageCache` 512 MB, GPU `TexturePool` 1.5 GiB.

- **One owner.** `freecut-app::MemoryBudget` derives every cache cap from total RAM and a
  unified-memory flag (the wgpu adapter reports `DeviceType::IntegratedGpu`), and hands the
  caps to the libraries through their configs. Starting split: the cache total is 35 % of RAM
  on unified memory (GPU pool included) and 45 % otherwise (GPU pool counted against VRAM
  separately). Shares of that total: decoders 35 %, `SourceFrameCache` 15 %, `ImageCache`
  10 %, `TexturePool` 25 % (unified memory only), `RamFrameCache` 15 % (v1). At 8 GB unified
  that is about 2.8 GB in all. The numbers are tuned in M3a with the 4K benchmark.
- **Pressure.** A job thread polls available memory every 2 s; below 10 % of total it halves
  every cap until the next poll above 20 %. Caches shrink by evicting idle entries, never by
  dropping frames in use.
- **OS queries.** Total and available RAM come from `sysinfo` (system feature only) and free
  disk space for EX-036 preflight and PM-113 from `fs4::available_space`. Both crates keep
  their FFI to themselves, so `freecut-app` stays `#![forbid(unsafe_code)]`. This replaces C22's
  "small per-OS helper", which could not be written without `unsafe` in the app.

### 5.13 Localization (UI-044)

UI-044 is mvp and must exist before any UI string is written, so it lands in M0.

- `freecut-app::i18n` holds a `Localizer` over `fluent_bundle::concurrent::FluentBundle` with
  `fluent-langneg` negotiation; `locales/en-US/*.ftl` are embedded at build time; `en-US` is the
  source and the fallback. `tr!("id")` and `tr_args!("id", name = value)` are the only way
  text reaches a widget.
- A debug-only pseudo-locale (accented, about 40 % longer, bracketed) shows hard-coded strings
  and truncation.
- Message ids: history labels map `OpKind` to `op-<kind>` with a `count` variable ("Split 3
  clips"); stable error codes map to ids (`E-DISK-FULL` → `error-disk-full`); toasts use
  `toast-<topic>`.
- A CI lint (a small test over the app sources) rejects string literals passed to egui widget
  constructors (`Button::new`, `Label::new`, `ui.label`, `ui.button`, …) outside the helpers.

### 5.14 Platform floors

- **macOS.** The spec floor is macOS 11 (`docs/features/workspace-and-shortcuts.md`,
  "Platform minimums"). cpal 0.18.2 lists CoreAudio at macOS 14.2 (`cpal-0.18.2/README.md`,
  "Minimum Supported Versions"), and its macOS backend references
  `AudioHardwareCreateProcessTap` unconditionally (`src/host/coreaudio/macos/loopback.rs:18-19,115`),
  through a plain `extern` block in `objc2-core-audio` 0.3 (0.3.2 locally,
  `src/generated/AudioHardware.rs:1794`), and Rust does not weak-link such references. On
  macOS 11–14.1 the app may then fail at launch with a missing symbol.
  **Decision:** builds set `MACOSX_DEPLOYMENT_TARGET=11.0`. M0 checks the release binary with
  `nm -m` (the ProcessTap symbols must be `weak external` or absent) and `otool -l`
  (`LC_BUILD_VERSION` minos 11.0). If they are strong, FreeCut carries a `[patch.crates-io]`
  of cpal without the loopback module (FreeCut never records system output); if that patch
  turns out to be unmaintainable, the spec owner is asked to raise the floor to 14.2 (§6.1).
  Pinning cpal 0.17.3 does not help (§4.4). The bundled FFmpeg is built with the same
  deployment target; Homebrew dylibs are built for the build host's macOS and are never
  shipped. A launch smoke test on macOS 11 needs a self-hosted VM or a manual release check,
  because hosted macOS runners are newer than 14.2.
- **Windows 10** and **Linux** (Vulkan 1.1 or GL 3.3 / GLES 3.0) as the spec says; cpal's
  PipeWire backend needs PipeWire 0.3.53, with ALSA as the runtime fallback.

---

## 6. Conflicts between the studies and how they are resolved

Each row names the studies involved and the decision. The decisions bind code; the studies stay
as the research record.

| # | Conflict | Decision |
|---|---|---|
| C1 | **Root type and edit vocabulary.** timeline-model has `Timeline { root, sequences, ids }`, `Txn`, `Change::UpdateClip`, tracks as `Vec<Clip>`, `IdAlloc` inside the model. edit-ops has `Document { settings, library, sequences, root }`, `Tx`, `Plan`, `ReplaceClip`, `Vec<Arc<Clip>>`, `IdAlloc` in `Editor`. | Adopt edit-ops' shape and names (`Document`, `Tx`, `Plan`, `Editor`, `ReplaceClip`, `Arc<Clip>`). Keep timeline-model's time module, invariants T1–C3, transition geometry and queries. Add the transition change variants that edit-ops omitted. The merged `Change` enum is in §3.1. |
| C2 | **Atomic apply.** timeline-model applies in place and rolls back the prefix; edit-ops applies to an `Arc`-clone and swaps on success. | Clone-and-swap: atomic by construction, a divergence leaves the live document untouched. |
| C3 | **Validation in release builds.** timeline-model: full in debug, touched scope in release; edit-ops: `validate_touched` always. | `validate_touched` at the end of every `Tx` in all builds; full `validate()` on load, before save, in debug builds and in property tests. |
| C4 | **Persistence boundary.** timeline-model (and the text model sketch) put serde derives with `try_from` mirrors on model types; project-format keeps the model serde-free and uses private DTOs. | DTOs in `freecut-project` are the only on-disk schema, guarded by the schema-hash lock test. `freecut-timeline` keeps an optional `serde` feature for `EditRequest` repro logs and test fixtures only. Load-time validation happens in the DTO mapper through the timeline constructors (`FrameRate::new`, `RichText::from_parts`, …). The clipboard for cross-project paste goes through the DTOs. |
| C5 | **Caption segments.** timeline-model: segments are clips (`ClipContent::Caption`) on a `Caption` track. text-and-subtitles: a private sorted `Vec<CaptionSegment>` with `SegId` and one `splice` primitive. | Segments are clips. They get move, trim, split, selection, snapping and undo from the common machinery, and `segment_at` is `coverage_at`. The text study's single-mutator idea becomes one planner helper, `caption::ops::splice_plan`, that every caption op uses and that emits ordinary changes. Bulk import creates tracks with one `InsertTrack` per track (the change carries the clips), so 50k cues are one entry. `SegId` = `ClipId`. Track-level caption style is `TrackPayload::Caption`. |
| C6 | **Name clash: `FramePlan`.** render uses it for the composited frame; keyframes uses it for per-clip source sampling. | Render keeps `FramePlan`. The sampling enum becomes `SamplePlan`, produced by `plan_samples`. `LayerSource::Video.samples` carries source frame starts (ticks) with weights. |
| C7 | **Frame-blend colour space.** keyframes: blend in linear light; render: everything in code values except exposure, white balance and HDR. | Frame blending is linear light through the same γ 2.4 helpers (it averages light over time; code-value averaging darkens motion). Dissolves stay in code values (FX-003). |
| C8 | **Per-voice resampling.** keyframes: a stateless table-driven sinc reading exact positions (rubato rejected for varispeed). audio-engine: one rubato `Async` per voice with priming, closed-loop ratio and continuity chains; its own polyphase sinc as the fallback. | The contract is keyframes' exact `BlockPlan` plus audio-engine's invariants (sync within one source sample; split, seek and block-size invariance). Implementation: same-rate 1× voices are bypassed (bit-exact); every other voice — rate conversion in the MVP, varispeed in v1 — uses one stateless position-driven windowed-sinc interpolator, which meets the invariants by construction. Gate: the 32-voice benchmark at ≥ 20× real time; if it misses, fall back to rubato `Async` with priming. Rubato stays for the device boundary (`Async`) and for export and analysis rates (`Fft`). Continuity chains stay for stateful DSP and the stretcher. |
| C9 | **Keep-pitch flag and stretch trait.** AU-023 puts the flag in `Retime`, TR-038 in `ClipAudio`; the trait is `Stretcher` (keyframes) or `TimeStretch` (audio); the backend is `signalsmith-stretch` (keyframes) or open (audio). | `ClipAudio.preserve_pitch` (detached audio can differ from its video). Trait name `TimeStretch` with keyframes' pre-roll and latency methods (§3.4). Backend chosen in v1 after a listening test against Rubber Band and Bungee; `signalsmith-stretch` is the default candidate. Until then non-1× shuttle is silent. |
| C10 | **AudioRing sample rate.** playback-engine: project rate; audio-engine: device rate (the clock counts device frames). | Device-rate frames; the feeder converts project → device after the master stage. |
| C11 | **Source audio policy.** audio-engine: conform every used stream to native-rate f32 on disk, with streaming fill until ready. media-io: live decode is primary; conform only for unreliable-seek sources, reverse and scrub; prefetch rings "into the callback". | `PcmReader` (audio-engine) is the contract; media implements it with a PTS-exact streaming fill and an in-memory block cache (MVP). One background audio pass per stream always produces peaks and the exact length. **From M2** it also writes native-rate PCM for streams the probe flags as seek-unreliable (MP3 without a Xing/VBRI TOC, raw ADTS AAC, Ogg without an index), and reads of those streams are served from it; until it is ready such a stream is never seeked: it is decoded forward from 0, counting samples (in playback that means `NotReady` and the clock holds, §2.5). The other two conform cases (reverse, scrub) stay v1, under PM-074 cache limits. Conform files are read with positioned reads, not mmap (§4.4). Media's audio prefetch rings feed the **feeder**; the cpal callback only reads `AudioRing`. |
| C12 | **Decoder pool shape and limits.** playback: a worker pool with a priority queue; sessions keyed by (media, stream, role, lane); 24 sessions; idle close 10 s paused / 3 s playing. media-io: an exclusive lease pool keyed by (source, stream, variant); `max(4, workers + 2·tracks)`; 2 GiB byte estimate; 3 per source; idle 10 s. | Both layers: media-io's `DecoderPool` of leases, with playback's prioritized `DecodeQueue` workers on top. Limits: `min(24, max(4, workers + 2 × visible tracks))` decoders and a 2 GiB byte budget, 3 per source, idle close 10 s paused and 3 s while playing. `DecoderProfile` is the union of both lists. |
| C13 | **Opening files.** playback-engine: `format::input_with_interrupt`; media-io: `std::fs::File` + `StreamIo` custom I/O (`input(path)` panics on non-UTF-8 paths; avoids the Windows CRT handle limit). | media-io's custom I/O. |
| C14 | **Time function and accessor names.** media-io `from_stream_ts`, `seek_ts_before`, `TICKS_PER_SEC`; audio `Tick::new`/`get`; render `as_i64`; export `.0`. | `ticks_from_stream`, `stream_ts_at_or_before`, `TICKS_PER_SECOND`, `Tick::from_raw`/`raw`. Seek targets take `stream_ts_at_or_before` and then subtract a safety margin; nothing compares stream timestamps for equality. |
| C15 | **`SampleRate` defined twice** (timeline and audio). | One type in `freecut-timeline::time`, with `sample_at` and `span_of`. |
| C16 | **Frame type names.** media-io `VideoFrame` (decoded), playback `DecodedFrame`, export `VideoFrame` (pooled bytes for the encoder); two `PixelLayout` enums. | Decoded frames are `VideoFrame`; encoder input is `EncodeFrame`; one `PixelLayout` enum in media-io's naming covers both (export's `Yuv420p10` = `Yuv420P16 { bits: 10 }`, `P010` = `P016 { bits: 10 }`, `Yuv422p10` = `Yuv422P16 { bits: 10 }`). |
| C17 | **Seek mode names.** playback and render `Exact`/`Approximate`; media-io `Exact`/`Keyframe`. | `SeekMode::{Exact, Approximate}`; `FrameQuality::{Exact, Approximate, Concealed, Truncated}`. |
| C18 | **Cancellation types.** playback `CancelToken` (epoch + generation); media-io `CancelToken` + `Deadline`; export `Cancel` (flag + select receiver). | One `freecut_media::cancel::CancelToken` with all three triggers and a `receiver()` (§2.4). `Epoch` is a pure type in the timeline crate. |
| C19 | **Export GPU device.** render-compositor: one device shared by every compositor, one queue. export-pipeline: its own `Device` from the shared adapter (isolation, VRAM freed, device loss contained). | **Revised 2026-10-12.** The MVP (M4) renders export on the **shared** device. A second device from the same adapter breaks the wgpu 30 contract: "an Adapter may only be used once to create a device", and `request_device` lists "already called on this Adapter" under Panics, a rule only "not currently enforced" (`wgpu-30.0.1/src/api/adapter.rs:44-57`); a panic also cannot be caught by the old "fall back if creating it fails" plan. The shared device costs nothing extra: `ExportGate` already throttles, preview is idle or paused during export, and there is no second pipeline compile, text atlas or `SourceCache`. If isolation is still wanted after measuring (§8 Q2), export gets a **fresh adapter**: `instance.enumerate_adapters(backends)` (the instance is in eframe's `RenderState`), pick the entry whose `get_info()` vendor, device, backend and driver match the preview adapter, and call `request_device` on that adapter only. |
| C20 | **Y′CbCr conversion for export.** render-compositor: fragment passes into `R8Unorm` / `R16Uint` targets, no compute on the core path (GLES fallback). export-pipeline: a compute shader writing tightly packed planes into a storage buffer. | The MVP uses the fragment-pass converter: it runs on every backend, and 8-bit 4:2:0 needs only `R8Unorm` targets; the 256-byte row padding is stripped during the pooled copy that happens anyway. The compute converter (tight packing, banded dispatch, P010 high bits) is a v1 path used when the adapter has compute. Both share `OutputColor` and the same CPU reference golden; fast path = slow path is tested. |
| C21 | **Subtitle stream writer.** text-and-subtitles: `media::mux::subtitle` taking `&mut Output`; export-pipeline: one `EncodeSession` owns every FFmpeg context of the job. | `SubtitleStreamWriter` lives in `media::encode::subtitle`, is owned by `EncodeSession`, gets its cues through `EncodeSpec`, and the mux loop calls `advance_to(video_pts)`. |
| C22 | **File locks.** project-format: `std::fs::File::try_lock`, avoid fs2/fs4. export-pipeline: `fs4` locks on job registry files. | `std` locks for both (stable since 1.89; MSRV is 1.95). `fs4` is adopted only for `available_space` (preflight, PM-113), with `default-features = false`; its lock traits are not used. A per-OS helper would need `unsafe` in the app (§5.12). |
| C23 | **In-memory LRU crate.** media-io `lru`; playback and text `quick_cache`. | `quick_cache` for byte-weighted shared caches; frame rings are `VecDeque`s; `lru` dropped. |
| C24 | **Where `render_key` lives.** playback-engine puts it in the timeline crate; it hashes shader versions and font digests owned by render. | The function lives in `freecut-timeline` (pure). Render supplies `RenderKeyInputs`: `RENDER_ABI_VERSION`, `TEXT_RASTER_VERSION`, effect versions, font environment digest, burn policy, overrides, resolved size. |
| C25 | **Keyframes at a split.** edit-ops left TL-023 (boundary keys) vs TR-027 (no boundary keys) open; keyframes resolved it. | TR-027: both halves share the same `Arc<KeyframeTrack>`, nothing is inserted; generated and still clips use `content_offset` so the right half's clock continues. |
| C26 | **Trimming curve-retimed clips.** edit-ops follows TR-036's Hermite subdivision; keyframes uses windows. | Windows: trim and split move `window_start` only; the authored curve is never cut. Edit-ops' trim and split call `Retime::{split_at, trim_head, trim_tail}`. |
| C27 | **Where the id allocator lives and its range.** timeline-model `NonZeroU64` in the model; edit-ops plain `u64` in `Editor`; project-format `u64` capped at 2^53. | `NonZeroU64` ids, capped at 2^53, allocator in `Editor`; `ProjectSnapshot` and `Opened` carry `ids_next`. |
| C28 | **`EditCtx` fields.** timeline-model `{ rate, sources, limits }`; edit-ops `{ prefs, busy, playhead, next_id }`. | The union (§3.1); the id base comes from `Editor`. |
| C29 | **Error variants.** timeline-model `TrackLocked(TrackId)`, `BelowMinimum`, `SourceExhausted`, `TimeError` inside `EditError`; edit-ops `TrackLocked { track }`, `TooShort`, `SourceLimit`, `TimeOverflow`, plus `ClipBusy` and `Stale`. | edit-ops' set and names, with `EditError::Time(TimeError)` instead of `TimeOverflow`, plus timeline-model's `NestingTooDeep`, `CompoundCycle`, `InsideTransition` and `NotFound`. Likewise `RenderError` is the superset of the playback, render and export variants (`DeviceLost`, `OutOfMemory`, `InvalidSize`, `TooLarge`, `GpuTimeout`, `Decode`, `Readback`, `Cancelled`). |
| C30 | **Timeline crate's external dependencies.** timeline-model says "only `serde` and `thiserror`"; keyframes adds `num-rational`, `num-integer`, `ethnum`, `libm`, `smallvec`; render adds `glam`; playback adds `xxhash-rust`; text adds Unicode crates. | "Nothing internal" stays; pure external crates are allowed through the allowlist in §1.2, checked by the dependency guard. |
| C31 | **Main-track marker in the file.** timeline-model: `visual[0]` is the main track by position, nothing else marks it; project-format's JSON example writes `"main": true` on the track. | Position decides; the DTO has no `main` field (each fact stored once). The loader's repair promotes the lowest Video track when `visual[0]` is not Video. |
| C32 | **Snapshot type names.** `SequenceSnapshot` (playback, audio), `DocumentSnapshot` (export), `Arc<Document>` (edit-ops, project). | `DocSnapshot { doc: Arc<Document>, version: u64 }`; `MixView` and the frame planner borrow it. |
| C33 | **Export audio rate.** export-pipeline: mix directly at the output rate so sources resample once; audio-engine: mix at the project rate, then an `Fft` resampler on the master. | `MixPlan` is compiled at the export rate whenever that rate divides the tick rate (44.1 and 48 kHz both do), so sources resample once; the master `Fft` path is kept for any other rate. |
| C34 | **Requests for decoded frames.** playback's `DecodeJob.at` is a stream timestamp; render's `FrameSource` and media's `frame_at` take a `Tick`. | Media time in ticks across crate boundaries; conversion to stream timestamps happens only inside media. |
| C35 | **Where text code lives.** render-compositor lists `src/text/`; text-and-subtitles puts layout, fonts and raster in `freecut-render::text`, subtitles in `freecut-project`, the stream writer in media. | Agreed as written, with C21 for the writer. |
| C36 | **The clock seen by render.** playback-engine's `PreviewEngine::spawn` takes `Arc<dyn PlaybackClock>`, a trait in `freecut-audio`; render may not depend on audio. | Render defines a minimal `PlaybackClockRead` trait (`reading() -> (Epoch, PlayTime, Tick, state)` with the shared pure types from timeline); `freecut-app` adapts `AudioClock` to it. |
| C37 | **Effect parameter migrations.** `Effect::migrate` is in render; projects load in project, which can't depend on render. | **Revised 2026-10-12.** The pure parts move into `freecut-timeline::effects`: `EffectManifest`, the parameter schema (`ParamSpec`, `ParamKind`, `ParamSet`) and `EffectCatalog::migrate(id, from, ParamSet)`. None of them uses GPU types. `freecut-project`'s open pipeline migrates parameters while it maps DTOs, before `Editor::open` (§5.11). Render keeps only `encode` and refers to the manifest type. This removes the post-load mutation, which the `Editor` API cannot express. No seventh crate. |
| C38 | **Cover image format.** project-format stores covers as WebP; media-io has no WebP encoder (no libwebp; `image-webp` encodes lossless only). | JPEG covers, PNG when alpha is needed, like thumbnails. |
| C39 | **Codec/container table test.** export-pipeline tests `freecut-project::export::compat` against `avformat_query_codec`, but project can't depend on media. | The pure exhaustive table test stays in project; the FFmpeg cross-check (query + real header write per allowed cell) is an integration test in `freecut-app`. |
| C40 | **GOP index naming.** playback `KeyframeIndex`, media-io `GopIndex`. | `GopIndex`. |
| C41 | **A source that is not ready during playback.** playback-engine: the feeder writes less, the callback underruns and the clock holds. audio-engine: `NotReady` renders as zeros, counts `starved` and keeps writing, so the clock advances and the audio is skipped. | playback-engine's behaviour, made precise in §2.5: write only the ready prefix, hold, `Buffering` after 1 s; zeros only for offline, missing-stream and decode-error sources and silent shuttle; `Prerolling` waits for the voices at the start position (or 250 ms). |
| C42 | **Corrections after open.** AU-001 and media-io: "non-undoable" clamp after the exact-length scan; C37: post-load effect migration; edit-ops N8: background changes add one entry; `Editor` exposes no other mutator. | §5.11: non-undoable work happens before `Editor::open`; everything after is a labelled commit (`CorrectMediaLength`, `UpdateMediaInfo`). |
| C43 | **Who creates the GPU device.** README and render-compositor: the app, handed over with `WgpuSetup::Existing`. playback-engine: `GpuContext` is cloned from eframe's `RenderState`. | eframe creates it with `WgpuSetup::CreateNew` and our descriptor and adapter selector; `GpuContext` comes from the `RenderState` (§2.3). |
| C44 | **Preview and export renderer for the M4/M5 slice (2026-10-12).** render-compositor and export-pipeline plan a wgpu compositor for both; it is not built yet. Before this slice the app previewed through a CPU compositor of its own in `freecut-app/src/preview.rs`; that code is now removed and `preview.rs` calls `freecut_render::cpu::Compositor`. | `freecut_render::cpu::Compositor` is the one renderer for preview and export until the wgpu compositor lands: plans are drawn on the CPU in straight-alpha RGBA8, text is laid out with parley and rasterized with swash, `export::ExportRenderer` drives it on the export clock, and `freecut_audio::OfflineMix` renders export sound through the playback `Mixer`. It is the reference the GPU path must match: GPU goldens compare against it. A system font hides a bundled font of the same family name (System > Bundled, as text-and-subtitles.md says). Export runs in `freecut-app::jobs::export` on this compositor, `OfflineMix` and `freecut_media::encode::EncodeSession`. Details: render-compositor.md §17 and the text-and-subtitles.md implementation note. |
| C45 | **The LLM editor goal vs. the local-only, undo-first design (2026-10-12).** The product goal is an open-source, LLM-based editor, but ai-tools.md put a typed-instruction editing assistant out of scope, the specs promise no cloud and no telemetry, and the edit model only accepts stamped plans committed by the `Editor`. A model could be given a second, looser edit path, or bolted into the app. | Design in [llm-editor.md](llm-editor.md), features AS-001…AS-020. A new crate `freecut-assistant` (depends on timeline, project, and media with `default-features = false` for the cancel token only; only app depends on it; networking behind its `http` feature) owns the tool registry, the sans-I/O agent loop, providers and safety rules. Tools are generated from `EditRequest` with a coverage test; a model only *proposes*: its calls are planned into a `PlanChain` on a `Planner` snapshot, shown with `Plan::preview()` and a FreeCut-generated change summary, and committed by the UI thread on Accept as **one** `OpKind::AssistantEdit` entry; stale plans are re-checked, never committed. Providers are pluggable (local OpenAI-compatible servers; hosted APIs such as Anthropic Messages, default `claude-opus-5-5`), off by default, keys in the OS keychain, no telemetry, disclosure before anything leaves the machine, media files never sent. CI replays recorded model replies; no network. Milestones M-AI1–M-AI4. When M-AI1 starts, §1.1, §1.2, §2.1 and CLAUDE.md gain the crate, its edges and its worker thread. |
| C46 | **Text round 2 on the CPU renderer (2026-10-12, accepted 2026-10-12).** The text spec now asks for optional style layers that keep their values while off, a bubble layer beside the background box (TX-054, TX-055), per-unit animation, Pace and a preset manifest (TX-022, TX-023, TX-056, FX-030). text-and-subtitles.md designs those effects as wgpu SDF passes, but C44 makes the straight-alpha RGBA8 CPU compositor the only renderer until the wgpu path lands, and straight alpha cannot carry TX-012's additive glow. | (1) Model: `Layer<T> { on, value }` replaces `Option<T>` for outline, shadow, glow, background, bubble and curve; at most one of background and bubble is on (a pure check in `freecut-timeline`). Nothing has shipped, so no migration. (2) The CPU path implements the effects from glyph coverage: a distance transform gives outline bands (union-correct, as R5), glow and shadow sources; boxes, bubbles (`bubble_path` in `freecut-render::shape`) and decorations are filled with `zeno`. (3) Text layers are composed **premultiplied** inside `freecut-render::text` and drawn with premultiplied *over*, so a glow pixel with α = 0 adds light, as the GPU design says; other layers stay straight alpha. (4) Per-glyph rasters are placed with per-glyph affine or projective (3×3) transforms, so per-unit motion, curves and 3D turns share one path; layout is still done once (R7). (5) Preset manifests (FX-030 channels, curves, amplitudes) are pure data parsed by `freecut-project` and evaluated by pure functions in `freecut-timeline` (`animation_offsets`, `pace_duration`); `unit_windows` and per-glyph application stay in `freecut-render::text::animate`, which needs the layout. (6) The CPU results are the golden reference the wgpu path must match later (C44). Build order: the round-2 build plan, slices S0–S3 (S4 and S5 are the UI concept remainder and assistant M-AI1). Render side built in S1: `freecut_render::text::style`, `shape::bubble`, the premultiplied text layer and the mask effects (render-compositor.md §17, text-and-subtitles.md "Round 2 render implementation"). |
| C47 | **Local speech-to-text engine (2026-10-13).** TX-038, TX-044, TX-045 and AI-009 to AI-012 need local recognition with word timestamps, language detection and voice activity on macOS, Windows and Linux; ai-tools.md assumed whisper.cpp but no crate, build or model files were chosen, and sherpa-onnx, candle, `ort` and Vosk are alternatives. | whisper.cpp 1.8.3 through `whisper-rs` `=0.16.0` / `whisper-rs-sys` `=0.15.0` (Unlicense binding; MIT runtime, MIT Whisper weights, MIT Silero VAD in ggml form), built from the crate's bundled sources with CMake; `WHISPER_DONT_GENERATE_BINDINGS=1` in `.cargo/config.toml` so no libclang is needed (one CI job regenerates and compares). Backends: Metal on macOS, CPU elsewhere, Vulkan in Windows/Linux release builds if the SDK is available (§8 Q16). Tiers Fast/Balanced/Accurate = `ggml-base-q5_1` (60 MB), `ggml-small-q5_1` (190 MB, default), `ggml-large-v3-turbo-q5_0` (574 MB), pinned to a model-repo revision with SHA-256. FreeCut chunks audio at VAD silences itself and maps model time to source ticks in its own code; DTW alignment gives word times. sherpa-onnx is the planned second engine (diarization, fast English-only models) through the future ONNX runtime, never through a build script that downloads binaries; Vosk is not adopted. Evaluation: [speech-and-captions.md](speech-and-captions.md) §2–§5. |
| C48 | **Adopting `freecut-analysis` (2026-10-13).** AI-001 proposed a seventh crate for analysis and inference; §1.1 lists six, and the features README kept its adoption open. Speech recognition needs it first. | Adopted with captions slice CS2. Edges: `freecut-analysis → freecut-timeline` only; only `freecut-app` depends on it (the §1.2 table and the dependency guard gain the row when CS2 starts, as C45 did for the assistant). `#![forbid(unsafe_code)]`; no networking; the only file I/O is opening a model the app names and hashing a file. Features: `ml-whisper` (whisper-rs), `ml-whisper-vulkan`, `ml-whisper-cuda` (never in official builds), later `ml-onnx`. The app's default feature `stt` enables `ml-whisper`, so `cargo run -p freecut-app` has speech recognition; `--no-default-features` builds without CMake or whisper.cpp, and the Captions drawer explains the missing rows. CLAUDE.md's crate list is updated by the maintainer when CS2 starts. |
| C49 | **Model downloads (2026-10-13).** AI-002 promised confirmed, hashed, resumable downloads but left the URLs, the storage layout, cancel semantics and the HTTP stack open. | Manifest compiled into `freecut-analysis` (`models/manifest.json`): id, version, task, file, HTTPS URL on the upstream model host **pinned to a repository revision**, SHA-256, size, licence id and URL. Nothing downloads without AI-002's confirmation (size, licence, origin, destination, free space). Files go to `<app data>/FreeCut/models/<id>/<version>/<file>` (`directories`), written as `.part` with a state file, resumed with a validator-bound range request, hashed over the whole file, then renamed into place; Cancel keeps the part, Discard deletes it, parts older than 30 days are swept. Downloader: a job in `freecut-app` with blocking `ureq` (rustls) and `sha2`, the same `ureq` the assistant uses (C45); never on the UI thread. After install, models load from disk only; the network is touched only on Download, Retry or *Check for model updates*. |
| C50 | **Caption data for generated captions (2026-10-13).** C5 made segments clips, but word timings, filler marks and transcripts were placed differently across TX-038, TX-039, AI-009 and AI-012: AI-009 keeps transcripts in the project yet off the undo stack, which §5.11 forbids; TX-038 stores filler tags as rich-text spans that do not exist yet (TX-015). | `ClipContent::Caption(Arc<CaptionSegment>)` on `TrackKind::Caption`, `TrackPayload::Caption(Arc<CaptionTrackData>)`; word timings `WordTiming { start, end, text: ByteRange, flags: WordFlags }` are **segment-local** (moving a caption never rewrites them; trim-head, split, merge and retime rebase them). Filler marks are `WordFlags::FILLER`, drawn by the look's Filler word style. `Transcript` is pure data in `freecut-timeline::model::library`; attaching it is the labelled change `AttachTranscript`, and an Auto captions run commits one compound entry (transcripts + `ReplaceCaptions`). Format changes are additive bumps in CS1 (caption tracks and clips) and CS2 (transcripts). |
| C51 | **Where second-line text comes from (2026-10-13).** TX-036's second line was typed or imported in v1 and translated only by AI-013's local models (later), which have no runtime yet; the user asked for a second language now. | In order of availability: typed; attached from a subtitle file; Whisper's built-in Translate to English (no extra model; English only; attached by TX-036's overlap rule); the user's assistant provider when AS is configured (opt-in, caption text only, AS-004 disclosure each run, AI-013 windows and split-back, `TranslateCaptions`); local translation models (AI-013) once the ONNX runtime exists, which then become the default. Options 1–3 never leave the machine; option 4 uses the assistant's existing single exception to *Local only*, and the AS and AI owners confirm it before CS4. |
| C52 | **Color stack on the CPU renderer (2026-10-13).** color-and-filters.md designs the stack (CL-001) as fused WGSL passes in `Rgba16Float`, render-compositor.md §6 L5 compiles it into uniforms and 3D textures, and §5.4 puts the working space in premultiplied float. C44 makes the straight-alpha RGBA8 CPU compositor the only renderer for now, its adjustment layers draw a placeholder, and the user's Adjustments request (Basic, HSL, Curves, Wheels, Masks pages, LUTs, adjustment clips) needs grading now. Evaluating every stage per pixel in `f32` costs far more than a frame allows at 1080p. | (1) **Model:** `Clip.color: Option<Arc<ColorStack>>` on video, image and adjustment clips (`None` = identity), fixed-point fields as CL-001 says; keys are `PropPath::Color(ColorParamId)` / `ColorPuck` / `ColorMask` in the existing `ClipKeys`, evaluated by `Clip::color_at`. (2) **Plan:** `LayerPlan.color: Option<Arc<ColorStack>>` (evaluated; `None` for absent, identity or bypassed) and `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })` for adjustment clips, whose transform and opacity are ignored. (3) **Working space:** display-referred Rec.709 code values with straight alpha (CL-002, §5.4), linear light (γ 2.4) only inside exposure, white balance and deflicker gain, Oklab for hue work; the 8-bit input is read as `f32`, every stage runs in `f32` with `libm` transcendentals, and the result is quantized once (no dither). (4) **Bake:** the per-pixel run of stages (4–14, plus 16 and the Amount blend when nothing spatial or masked follows) is evaluated at the nodes of an RGB lattice over the 8-bit input cube (33³; 65³ for steep curves or fine user LUTs) and applied with tetrahedral interpolation; identity stacks are skipped, a LUT-only stack applies the user's table directly, and bakes are cached by an XXH3-128 digest of the evaluated values plus `COLOR_ALGO_VERSION`. Spatial stages, masks and the Amount blend with a base run as later passes. (5) **Placement:** clips are graded in `Compositor::prepare` on the decoded (possibly scaled) picture before mips and placement, cached by picture identity and digest; an adjustment node flushes the layers collected so far into the frame and grades the frame in canvas space at output resolution, blended by Mix. (6) **Budget (1080p, release, 8 threads):** bake ≤ 3 ms, one graded full-frame layer ≤ 4 ms, an adjustment clip ≤ 4 ms, a mask ≤ 2 ms. (7) **Parity:** per-pixel independent, banded arithmetic and portable math make bytes independent of thread count, frame order and preview versus export; `eval_pixel` is the oracle for the lattice and, later, the GPU goldens (C44). The GPU design of render-compositor.md §6 L5 stays the target; its working space becomes premultiplied `Rgba16Float` there, and the CPU goldens are compared within tolerance. Build order: slices K1–K6 of color-and-filters.md › Adjustments round 1. Details: render-compositor.md §17, *Color stack on the CPU path*. *K1 amendment (2026-10-14, measured):* Hue rotation turns the Rec.709 luma/colour-difference plane of code values instead of Oklab (the Oklab turn missed the lattice bound by 19/255); saturation and vibrance mix toward code-value luma; see color-and-filters.md › CL-001 › *K1 as built*. *K2 amendment (2026-10-14, measured):* HSL bands are weighed by the HSV hue of code values and shift hue in the same luma/colour-difference plane (no Oklab), wheel pucks are luma-free colour-difference offsets, curves bake to 1024-entry Fritsch–Carlson tables, and the lattice is 65³ when the steepest Luma × channel curve slope exceeds 4; every K2 corner stack stays within 1.2/255 of `eval_pixel` (color-and-filters.md › CL-001 › *K2 as built*). *K3 amendment (2026-10-14, measured):* the user LUT (stage 13) comes from a `LutSource` handed to `Compositor::with_luts`/`set_luts`; a LUT-only stack samples the user's own table (both interpolations reproduce an identity table exactly and agree with the bake within 1/255); a missing LUT grades as identity and is reported by `take_missing_luts`; skin protection's mask (CL-030) is part of the bake, which is 65³ whenever a skin scope is active (33³ was 8.8/255 off at the mask's hue shoulders, 65³ is 1.5/255) and whenever the user's 3D table is finer than 33³; `COLOR_ALGO_VERSION` is 3 (color-and-filters.md › CL-001 › *K3 as built*). |
| C53 | **Timeline toolbar round on the current stack (2026-10-13).** The toolbar round (timeline-editing.md › Toolbar round, slices T1–T6) needs voiceover capture, markers, linked selection and *Follow main track*. audio-engine.md §12, its crate table and §4 here named `hound` as the recording writer, but AU-027 now needs an on-the-fly RF64 switch and a header refresh every second; timeline-model.md §8 typed marker colours as `MarkerColor`, while TL-037 and TL-049 use one `PaletteColor`; TL-057 changes TL-030's selection default. | (1) **Recorder:** `freecut_audio::record` behind a new default feature `record` of `freecut-audio` (cpal input); a `CaptureSource` trait with `SyntheticInput`, so the recorder's tests need no microphone and `cargo test -p freecut-audio --no-default-features` still builds the mixer. Capture → `rtrb` ring → writer thread → FreeCut's own 24-bit WAV/RF64 writer (`record/wav.rs`) with PM-043's `.recording` marker; `hound` becomes a dev-dependency (fixtures, debug dumps). No `unsafe`. (2) **Markers** use `PaletteColor` (timeline-model.md §8 updated); markers and attachments are persisted through `freecut-project` format records, each taking the next free `CURRENT_FORMAT_VERSION` at merge (C54). (3) **Edits:** cut-away is one `EditRequest::TrimToPlayhead`; linked selection is pure (`expand_selection`, `linked_edges` in `freecut-timeline`) and changes no existing op; *Follow main track* runs `follow_owners` inside every owner-moving op. (4) The tool, skim mode and linked-selection preference are view state or preferences, never document data. Details: timeline-editing.md slices T1–T6, AU-027, audio-engine.md §12. |
| C54 | **Round 3 integration: three areas, one workspace (2026-10-13).** The toolbar (T1–T6), colour (K1–K6, C52) and captions (CS1–CS4, C47–C51) rounds were planned in parallel. Almost every slice edits `freecut-timeline`, `freecut-app`, the workspace `Cargo.toml`/`Cargo.lock` and `CURRENT_FORMAT_VERSION`; K3's draft had `freecut-project` parse LUT files, which needs the forbidden project → media edge; render's colour statistics module was named `analysis`, beside the `freecut-analysis` crate that C48 says owns analysis; caption looks were placed in both `assets/` and `resources/`; and the captions build completes features that were still `later`. | (1) **Order:** toolbar first (smallest, no new crates), then colour, then captions and speech; slices inside a round keep their own order. Crate-disjoint side lanes may run beside the trunk: T6a (`audio` only), the FFmpeg-free `freecut_media::lut` parsers (K3a), the `freecut-project::subtitles` parsers and writers (CS1a), `freecut_media::audio::read_pcm_mono` (CS2a), and the pure core of `freecut-analysis` once CLAUDE.md lists it (CS2b). An app-only slice (T5) may pair with the model half of the next trunk slice. (2) **Format versions** are taken in merge order, one per slice that changes persisted data, each with an additive identity migration and a migration test; nobody hardcodes a number in specs. (3) **LUT import** is the app job `freecut_app::jobs::luts`, which validates with `freecut_media::lut` and stores through `freecut_project::luts::LutLibrary` (index and hashed copies only). (4) Render's pixel statistics for CL-019, CL-020 and CL-034 are `freecut_render::color::stats`: they read render's `Picture`, need no model and stay in render; `freecut-analysis` owns model inference and the audio/text analysis of AI-001. (5) FreeCut's JSON presets (animations, text animations, caption looks) live in `resources/`; binary or attributed assets (fonts, LUT files and their recipes) live in `assets/` and are listed in `assets/ASSETS.md`. (6) **Priorities:** TX-038, TX-039, TX-044, TX-051, AI-002, AI-009 and AI-010 move from later to v1 (M9 → M8); TX-041, AI-012 and AI-013 are built in part and stay later. (7) **CLAUDE.md** gains `freecut-analysis` only when the maintainer approves the proposed lines before CS2 (C48); until then CS2b does not start. |
| C55 | **Cloud generation with the user's own Higgsfield account (2026-10-13, proposed).** The user asked to connect Higgsfield and generate or edit images and video with their own token. The specs promise *Local only* with the assistant as the single exception (C45), and ai-tools.md's AI-026 refuses remote hosts. Higgsfield has two products: an MCP server billed to the user's plan credits, whose documented sign-in is browser OAuth (no key to paste), and a REST API with a key pair, whose docs and terms forbid client-side keys. A paid tool call that times out may already have spent credits. | Design in [cloud-generation.md](cloud-generation.md), features CG-001…CG-026. (1) **Service:** the MCP server at `https://mcp.higgsfield.ai/mcp`, JSON-RPC 2.0 over Streamable HTTP (`initialize`, `tools/list`, `tools/call`, session ids, `notifications/cancelled`), polled with `jobs_wait`; uploads and downloads are plain HTTPS to presigned and result URLs, without the bearer token. "Bring your own token" means OAuth 2.1 in the system browser (PKCE, loopback redirect, dynamic client registration, device code if offered), with *Paste an access token* as an advanced path; tokens live in the OS keychain; the REST key pair waits for Higgsfield's written confirmation. (2) **Crate:** a new `freecut-cloud` (depends on timeline and on media with `default-features = false` for the cancel token; only app depends on it; networking behind `http`; sans-I/O core; `#![forbid(unsafe_code)]`). It is recommended over `freecut-assistant`, whose only-the-configured-provider rule it would blur, and over `freecut-app`, which has no seam for the tests that guard money. The assistant reaches it only through app-routed action tools (CG-024). One cloud worker per app; transfers run on PM-073's I/O pool. (3) **Money rules:** every paid call follows a `get_cost` preflight and an explicit Run on FreeCut's sheet; paid submissions have no retry wrapper and are never resent automatically, not even after a `401` or session `404` (those become *Not accepted*); a request that may have left becomes *Outcome unknown*; the cost preflight shares its tool with the paid run, so a guard checks the `get_cost` flag on every preflight and never retries one that may have been sent; actions whose tool gives no estimate are routed through catalogue models that do, or not offered; free-trial allowances are used only on the user's answer; a fixed allow list of tools (no batch tools, presets, publishing or account changes). (4) **Data:** only the chosen section or frame, re-encoded, metadata stripped, with generic file names; the disclosure is generated from the same `UploadPlan` that drives the upload. (5) **Results:** a ledger in app data, written before and after each submission, keeps runs across project close and crashes; downloads are checked with SHA-256 and go into the generated-media store (or an app-data inbox); items carry `MediaOrigin::Generated { kind: GeneratedMediaKind::Cloud, produced_by: "higgsfield:<model>", recipe, parent, source_range }` (PM-041; the provenance fields are added to the code's `MediaOrigin` in H2, the recipe as JSON text); import and placement are one undo entry. (6) **Tests:** recorded and scrubbed fixtures, an in-memory fake MCP server and `127.0.0.1` listeners; no network in CI. Slices H1–H4 (M-CG1–M-CG4). Before H1 the owners confirm the second *Local only* exception and approve the CLAUDE.md lines in cloud-generation.md §2.4; until then no cloud code starts. |

### 6.1 Spec follow-ups (owners must sign off; specs are not edited here)

| Specs | Issue | Proposed change | Raised by |
|---|---|---|---|
| TL-004 vs FX-002, AU-029 | FX-002 says the clips overlap; TL-004 says clips never overlap | Keep no-overlap; overlap placement = trim + ripple + handles; model `Transition { from, to, in_offset, out_offset, placement, payload }` | timeline-model |
| TL-011 vs TR-036 | which length derives from which | Speed edits keep D and recompute T; trims set T and recompute D | timeline-model, keyframes |
| TR-036 | reverse `s(t) = out − t·r` picks the frame after the range | Left-continuous frame rule for reversed clips | timeline-model, keyframes |
| TR-036 | "i128 is enough"; "Hermite subdivision on trim" | I256 evaluation; windows instead of subdivision | keyframes |
| TL-023, AU-008 vs TR-027 | boundary keyframes on split | No boundary keys (TR-027) | edit-ops, keyframes |
| TR-031, TR-027 | missing mirror variants; generated clips' key clock | Add ElasticIn, BounceIn, OvershootIn, internal HoldAtStart; add `content_offset` | keyframes |
| AU-023 vs TR-038 | keep-pitch flag location | `ClipAudio.preserve_pitch` | keyframes, audio |
| TL-006, TL-023 | per-sequence id counters collide on compound moves | One document-wide allocator | timeline-model, edit-ops, project |
| TL-039, TL-011 | groups stored twice | `Clip.group` only | timeline-model, edit-ops |
| TL-004 | caption track kind missing | Add `TrackKind::Caption`; segments are clips | timeline-model, text |
| FX, TR, TX templates | `RationalTime` | Means `Tick`; no second time type | timeline-model |
| TL-002 | ruler labels (wall clock) vs `HH:MM:SS:FF` drift at NTSC | Both from `Timecode` | timeline-model |
| TL-001, PM-011 | conform may collide when "keep at 1 frame" | Steal from the following gap, else ripple that track | timeline-model |
| PM-021 vs TL-010, TL-038, EX | view state "in the project UI section" | App-data sidecar `view/<project-id>.json`; types in `freecut-project::view` | project-format |
| PM-021, PM-042 | `assets` top-level key | Drop it; the asset index is derived | project-format |
| PM-021 | newer files always read-only | Add `write_compat` for additive bumps | project-format |
| PM-013 | drop clips with unresolvable media | Lossless placeholder offline item | project-format |
| AI-001 | `AnalysisRecord` persisted in the project | Regenerable analysis in the PM-074 cache; only user-edited data (transcripts) in the project | project-format |
| PM-035, FX-001 L2 | where rotation and PAR are applied | Media computes, GPU applies; CPU path via `to_rgba` | media-io |
| PM-080 | proxy geometry baked in | Unrotated uniform scale of the coded picture | media-io |
| PM-080 | proxy audio | Optional AAC fallback track used only when the original is offline | media-io |
| PM-070 | WebP thumbnails | JPEG, PNG with alpha | media-io |
| EX-002, AU-026 | latency subtracted twice; "within one buffer" | Compensation happens once, in the clock; edits reach the ear within ring fill (~43 ms) + device latency | playback, audio |
| EX-002 transport | state sketch | Add `Prerolling` and `Buffering`; add `PlayPlan`, `PlayTime`, `Epoch` | playback |
| EX-022, EX-037 | fast-start needs "a second copy" of disk space | It needs about the `moov` size (in-place shift) | export |
| EX-032 | "left-sited … box filter" | `[1,2,1]/4` horizontal filter for left siting | render, export |
| CL-001 | fused pass order around sharpen (stage 13) | Split passes: 1–12, 13, 14–16 | render (G1) |
| FX-023 | 8-bit luma maps band | Store as `R16Float` | render (G3) |
| FX-001 | layer-space resolution | Size `fit × scale × k`, k from the transform, √2-quantized | render (G4) |
| CL-007, CL-008 | unnamed "linear light" | γ 2.4 (BT.1886, zero black) | render (G5) |
| OV-004 | luminosity weights | W3C 0.3/0.59/0.11 | render (G6) |
| TR-001 | who builds the transform matrix | The timeline builds a 3×3 homography; render scales and inverts it | render (G7) |
| FX-025 | per-pass watchdog | Not possible on wgpu 30; static checks + consent dialog | render |
| TX-002 / UI-046 | egui text widgets can't shape complex scripts | On-canvas editing through our own `EditActor`; egui `TextEdit` only for the MVP inspector mirror | text |
| AU-001 | "clamped by a non-undoable model correction" after the exact-length scan | "clamped in its own undo entry" (§5.11); a non-undoable change after open would diverge the history | review 2026-10-12 |
| EX-018 vs PM-080 | both v1, but 4K HEVC Main10 (often HLG, 60 fps) is the main phone input and software decode will not hold 1× with two layers on 8 GB laptops | Move EX-018 to mvp for preview (export stays software); M3a already builds it behind `HwDecodePolicy::Auto` | review 2026-10-12 |
| UI platform minimums (workspace-and-shortcuts) | "macOS 11 or newer" vs cpal 0.18's macOS 14.2 for CoreAudio | Keep 11 if the M0 symbol check or the cpal patch works (§5.14); otherwise raise the floor to 14.2 | review 2026-10-12 |

---

## 7. Build order to MVP

Every milestone ends with the same definition of done: `cargo fmt`, `cargo clippy -D warnings`
and `cargo test --workspace` green on macOS, Windows and Linux; no `unsafe` outside the allowed
modules; every new edit op has its roundtrip test; the dependency guard passes; this README and
any affected spec follow-up list updated.

### M0 — Workspace scaffold

**Deliverables**
- Cargo workspace per §4.1. Partly present already (six crates, workspace lints, root
  `clippy.toml`, `LICENSE`, CI workflow); M0 applies the deltas in the §4.1 table and adds
  `crates/freecut-project/clippy.toml`, `rustfmt.toml`, `assets/ASSETS.md` for attributions and
  a third-party-notices plan.
- Crate roots with the `unsafe` attributes of §4.1 and empty public modules matching §3.
- `freecut-media` split into the always-built part (cancel, wire types, `color`) and the
  default feature `ffmpeg`; `freecut-audio` depends on it with `default-features = false`
  (§1.2, §4.1).
- `tests/deps_guard.rs` (workspace level, via `cargo metadata`): allowed edges (§1.2), the
  audio → media `default-features = false` rule, and the timeline crate's external allowlist.
- CI on three OSes running §5.9 with the pinned FFmpeg 8.1 shared build per runner
  (`FFMPEG_DIR`/`PKG_CONFIG_PATH`), LLVM on Windows, `libasound2-dev` and the cpal feature
  packages on Linux, plus the lavapipe GPU job (initially only a "create adapter and clear a
  texture" test).
- `freecut-media::init()`: libav major + minor check, raw-integer enum mappers in `av/`
  (§5.7), and the PM-036 capability table.
- `freecut-app`: an eframe window created with `WgpuSetup::CreateNew` and our
  `device_descriptor` (16-bit norm and timestamp features when available, ≥ 8192 px textures,
  downlevel limits on GL) and adapter selector; `GpuContext` built from
  `CreationContext::wgpu_render_state`, then the uncaptured-error handler and the device-lost
  callback installed; a paint-callback quad drawn from a texture the app owns;
  `tracing-subscriber` + `tracing-appender` installed.
- `freecut-app::i18n` (§5.13): `Localizer`, `tr!`/`tr_args!`, embedded
  `locales/en-US/*.ftl`, the debug pseudo-locale, the literal-string lint, and the
  `OpKind` and error-code message-id maps.
- macOS: `MACOSX_DEPLOYMENT_TARGET=11.0` and the ProcessTap symbol check of §5.14.
- Fixture generator skeleton (`tests/support/fixtures.rs` in media) that encodes a 10-frame
  barcode clip at test time.

**Acceptance**
- The three CI commands pass on all three OSes; `deps_guard` fails when a forbidden edge is
  added in a throwaway branch; `cargo tree -p freecut-audio --no-default-features` has no
  `ffmpeg-sys-next`.
- On every CI OS, `freecut_media::init()` reports libavcodec 62.x with the expected minor and
  finds the `h264` decoder and the `libx264` and `aac` encoders. A runtime with an older minor
  is refused (unit test over the version comparison). The enum mappers turn out-of-range
  integers into `Unknown`.
- `cargo run -p freecut-app` opens a window that shows an app-rendered wgpu texture through the
  paint callback (proves device sharing) and closes without validation errors; the GPU
  features and limits actually granted are logged.
- The window's title and one button come from the catalog; with the pseudo-locale on they show
  pseudo text; a test widget with a literal string fails the lint.
- `nm -m` on the macOS release binary shows no strong reference to
  `_AudioHardwareCreateProcessTap` (or the cpal patch of §5.14 is in place).
- The barcode fixture decodes back to indices 0–9.

### M1 — Timeline model and undo, fully tested

**Deliverables** (`freecut-timeline` only; no FFmpeg, no GPU)
- `time/`: `Tick`, `div_round`, `Rounding`, `Rational`, `FrameRate` with the PM-005 presets,
  `SampleRate`, `SampleSpan`, `TimeRange`, `MarkRange`, stream conversions, non-drop
  `Timecode` (drop-frame is v1).
- Ids, `IdAlloc`, `IdCursor`, `IdRemap`; the `Document` model including library plain data
  (`MediaItem`, `MediaInfo`, `StoredPath`, `Fingerprint`); typed property bags with static
  values (keyframe tracks arrive in v1, but `Animatable<T>` exists with `keys: None`).
- `Retime` with the linear `TimeMap` and windows at 1× (speed edits are v1, but split and trim
  already go through `Retime::{split_at, trim_head, trim_tail}`).
- `validate()` (T1–C3), queries (`coverage_at`, `clips_in_range`, gaps, cut points, duration,
  snap targets), `Change`, `ChangeSet` (inverse, compact), `apply_all`, `Tx`, `Plan`,
  `History` (depth, byte budget, saved mark, gestures, repeat coalescing), `Editor`,
  `SnapIndex`.
- MVP planners: insert (drop/add, magnet cut index), move (two-stage, collision → new track),
  plain and ripple trim, split, delete and ripple delete, add/remove/auto-remove tracks, track
  mute and hide, sequence flags (magnet on/off with compaction), conform to frame grid, add and
  remove transition with overlap placement plus `fit_transitions`, library ops (`ImportMedia`,
  `RemoveMedia`, `RelinkMedia`, `UpdateMediaInfo`, `CorrectMediaLength`: new length plus the
  out-point clamp of every clip past it, one entry, §5.11).
- `testkit`: ASCII DSL, proptest strategies, frame-array oracle, `assert_roundtrip`.

**Acceptance** (`cargo test -p freecut-timeline`)
- Time tables: every PM-005 rate and every listed sample rate has whole-tick periods; 11 fps and
  384 kHz are refused with the right `TimeError`; `div_round` tables including negative
  numerators and exact ties; `stream_ts_at_or_before(ticks_from_stream(ts)) == ts` for 1/15360,
  `AV_TIME_BASE`, 1/90000, 1001/30000; timecode vectors including 100 h without wrap; values
  near `i64::MAX` return `Err`, never panic.
- Every MVP planner passes `assert_roundtrip`, its spec edge cases (no-op returns an empty plan,
  locked tracks, whole multi-clip move refused, magnet on and off) and an `insta` golden of its
  ASCII before/after.
- State-machine property test: random sequences of up to 50 ops; after each step `validate()`
  passes and the main track is contiguous with the magnet on; undo-all equals the initial
  document and redo-all the final one; ids are never reused; 256 cases per PR, 10,000 nightly.
- Frame-array oracle agrees with insert, overwrite, ripple delete, trim, split and overlap
  transition on random timelines of ≤ 300 frames; split never changes `coverage_at` at any
  frame; add + remove transition is the identity.
- History tests: 200-entry and 512 MB limits keep the newest entry; dirty flag correct across
  eviction, redo-branch discard and merges; undo refused during a gesture; repeat merge at
  1,000 ms but not 1,001 ms; nudge right then left leaves no entry; trim → background
  `CorrectMediaLength` on the trimmed clip → undo → undo → redo → redo all return `Ok` (no
  `Diverged`) and end at the corrected document.
- Snapping tests: inclusive threshold, priority ties, coincident targets give one guide,
  dragged clips excluded, clamp after snap removes the guide.
- Benchmark recorded (not gating): `plan_move` on 10,000 clips × 20 tracks.

### M2 — Media probe/decode and project save/load

**Deliverables**
- `freecut-media`: custom-I/O `open_input` with interrupt; `probe` (streams, rotation and
  mirroring, VFR, rates, durations, colour tags, alpha kind, fingerprint, error mapping);
  `GopIndex`; `VideoDecoder::frame_at` with the cost planner, per-stream seek, overshoot
  retry, EOF and concealment; `PixelLayout` mapping and the `sws_scale_frame` CPU path;
  `AudioReader` with PTS alignment and priming; `DecoderPool` and `DecodeQueue`; `CancelToken`;
  stills through `image`; `CacheStore`; poster and hover thumbnails; filmstrip L0 chunks; the
  per-stream audio pass (peak pyramid + exact length, plus native-rate PCM for streams the probe
  flags as seek-unreliable, served through positioned reads, C11); forward-only reading of
  such streams until their PCM is ready; `color::hdr_to_sdr` and the HDR branch of `to_rgba`
  (§5.4); UI-thread assertions.
- `freecut-project`: v1 DTOs, canonical writer, open pipeline (parse, migration runner with no
  steps yet, typed decode with JSON paths, validate and repair), save pipeline
  (`atomic_replace`, conflict check, verify-after-write, one-time `.bak` copies), lock
  (metadata + liveness), `StoredPath`, `relative_to`, resolver over `FileSystem`,
  `read_header`, `view` sidecar types.
- `freecut-app` (headless parts): job pool; import → probe → `ImportMedia` commit; save/open
  coordinator; media status check and relink flow (PM-090/091) as functions callable from
  tests.

**Acceptance**
- Test-time fixtures: H.264 with B-frames (MP4, MKV), HEVC, VP9 with alpha, ProRes 4444, 29.97 at
  1/15360 and 1/30000 timebases, VFR, display matrices 90/180/270 + hflip, 1440×1080 SAR 4:3,
  odd sizes, AAC priming and edit lists, early and late audio, zero-byte, random bytes,
  truncated MP4; **10-bit HEVC HLG and PQ** clips from our own libx265 path
  (`colorprim=bt2020:transfer=arib-std-b67` or `smpte2084`, `colormatrix=bt2020nc`,
  `max-cll` set); **VBR MP3 without a TOC** and **raw ADTS AAC**.
- Exact seek: 300 random targets per fixture in random order return the barcode index the
  TR-036 rule expects; a walk over every frame at exact tick boundaries of the 1/15360 fixture
  never shows the previous frame (timeline-model risk 9); sequential equals random access.
- Exact pixels: for the H.264 B-frame and HEVC fixtures, under both the `Scrub` and the
  `Export` profiles, random-access `frame_at` returns frames whose **full plane hashes** (not
  barcode indices; 16 bands cannot see missing deblocking) equal a sequential decode's (§5.5).
- HDR: the HLG and PQ fixtures probe with the right transfer and primaries, and `to_rgba`
  output equals `color::hdr_to_sdr` applied to the RGB48 matrix output.
- `MediaInfo` `insta` snapshots per fixture; error-mapping goldens (`Empty`, `NotMedia`,
  `Incomplete`, `Corrupt`, `UnsupportedCodec`); a non-UTF-8 file name opens on Linux without
  panicking.
- Audio: random `AudioReader` reads equal a sequential decode bit for bit, **including the VBR
  MP3 and ADTS fixtures** (before and after their PCM is ready); the first audible sample is
  within ±1 of expectation for priming, edit-list delay and early audio; peaks equal a
  brute-force min/max including the final samples; the exact-length scan of a shortened file
  reaches the app as one `CorrectMediaLength` commit.
- Cancellation: a slow custom-I/O source aborts within its deadline, and the reader works
  again after `clear_interrupt` + seek. Pool stress: 16 threads for 30 s, no deadlock, every
  frame correct, byte budget respected.
- Project: proptest save → open gives an equal document; `save(open(save(d))) == save(d)` byte
  for byte; injected unknown keys survive a round trip; `kitchen_sink.freecut` golden is
  byte-identical on all three OSes; schema-hash lock test active; kill harness (200 kills
  during saves) always leaves an openable file with one of the two documents; injected
  `atomic_replace` failures keep the old file; lock liveness (killed child → `Stale`);
  relative-path and resolver tables; a non-tick-exact frame rate in a file is refused.

### M3a — Headless playback engine

M3 was split in the 2026-10-12 review: it held the whole engine, the whole UI and several v1
features, so it would have held up M4 and M5. M3a is testable without a window; M3b is the UI
on top of it. `RamFrameCache` with readback, peak meters (AU-015 is v1) and the GL-backend CI
job move to v1.

**Deliverables**
- `freecut-timeline`: `frame_plan` (layers, cross-dissolve transition nodes, stills with alpha,
  slates, Fit as the default framing), `PlayPlan`/`PlayTime`/`Epoch` with `amended(..)`,
  `render_key` (pixel-affecting fields only, §5.5), `MixView`, `gain_envelope_at`,
  fade-clamping helpers, `audio_overlaps`.
- `freecut-render`: `GpuContext`, `Compositor` MVP path (L2 normalize **including the PQ/HLG
  variant** of §5.4, place pass with transform matrix, edge AA, opacity, Normal blend,
  background colour, dissolve, slates), `TexturePool`, `SourceCache`, `PipelineCache`, naga
  validation tests, `PreviewEngine` (scheduler, `plan_due`, `PresentRing` sized by bytes with
  depth ≤ slots − 2, release of Ready frames whose snapshot version changed),
  `preview_target_size`. No RAM frame cache.
- `freecut-media`: playback prefetch, look-ahead opener (opens and seeks readers for the
  feeder, §2.5), `Approximate` scrub decode, and **hardware decode for the preview profiles**
  as media-io designs it (`get_format` with software fallback, `av_hwframe_transfer_data` to
  NV12/P010, which L2 already handles) behind `HwDecodePolicy::Auto`; export stays software.
- `freecut-audio`: `MixPlan` compile (voices, buses, master; fades, clip and track mute, clip
  volume), `MixEngine` with bypass and the stateless rate-conversion interpolator, limiter,
  feeder with the ready-prefix rule (§2.5), `AudioRing`, cpal output, `AudioClock` and
  `MonotonicClock`.
- `freecut-app` (headless): `Transport` (Paused, Prerolling, Playing, Buffering, Scrubbing) with
  `FakeClock`, the `VirtualOutput` and `FakeRenderer` harness, `MemoryBudget` (§5.12).

**Acceptance**
- `Transport` tests with a `FakeClock` cover every row of the EX-002/003 state table: Space at
  the end restarts from 0, pause shows the frame on screen, stepping while playing pauses
  first, buffering enters at 1 s and leaves at 0.5 s. Edits while playing: appending a clip
  extends playback past the old end; a ripple delete that moves the end before the playhead
  stops at the new end (`PlayPlan` amendment, playback-engine.md "Revised" note).
- Engine harness (`FakeRenderer` + `VirtualOutput`): 2 h at 29.97 ends with |video − audio| ≤
  1 frame; a seek never presents an old-epoch frame; an edit during playback releases only
  Ready frames of the old snapshot version; the start gate opens within 250 ms; frames
  rendered 3 frames early are presented on their deadline vsync, not when they finish.
- Starvation: a voice whose source becomes ready 300 ms late loses no samples (the output is
  bit-identical to a run where it was ready) and A/V stays within 1 frame; a fake provider that
  sleeps 2 s in `open` never stalls the feeder.
- `loom` passes for the seqlock, `PresentRing` and ring flush; `assert_no_alloc` passes for
  `fill()` and for steady-state `MixEngine::render` with 32 voices.
- Audio properties: abutting "all ones" clips sum to exactly 1.0 on every sample at 29.97 ×
  44.1/48 kHz; split invariance bit for bit (1× and 44.1 → 48 kHz); block-size and seek
  invariance; limiter never exceeds the ceiling on adversarial input; `fill_gain` agrees with
  `gain_envelope_at` within 0.01 dB.
- GPU goldens on lavapipe and Metal: 8/10-bit planar, NV12 and P010 at odd sizes with left vs
  centre siting; **HLG and PQ frames against the `color::hdr_to_sdr` CPU reference within
  2/1023**; straight-alpha gradient shows no fringe after rotation; rotated quad edge AA;
  dissolve at e ∈ {0, 0.25, 0.5, 1}; rendering twice is bit-identical; ½-scale render vs
  downsampled full render PSNR ≥ 40 dB.
- `render_key` properties: mutating any pixel-affecting field of a generated `FramePlan`
  changes the key; rippling an earlier clip leaves the keys of the shifted frames unchanged;
  an edit followed by undo restores the key.
- Hardware decode: media-io's equality test (I9) on the dev machine; a forced `get_format`
  failure falls back to software at the same position.
- Benchmarks recorded, not gating: start latency from cached media < 150 ms; exact scrub p95 <
  150 ms on 1080p long-GOP H.264; **4K30 HEVC Main10 HLG plays at 1× without planned drops**
  on a base 8 GB Apple Silicon machine and a mid-range Windows laptop (threshold recorded
  here once measured).

### M3b — App shell with timeline UI and preview

**Deliverables**
- `freecut-app`: main window layout and home screen (UI-001/002/016); project new/open/save/
  recent and unsaved-changes prompt (PM-001/002/012/013/017, UI-025); media panel with import
  dialog (`rfd` async, §2.3) and drag-and-drop (PM-030/031/050); timeline panel with ruler and
  timecode, zoom and scroll, filmstrip and waveform tiles, selection, drag/trim/split/delete
  with ghosts from `plan.preview()`, snapping guides (TL-002…TL-043 MVP subset); inspector
  with volume and fades; player panel with paint callback, frame step, J/K/L at 1×; command
  system and default shortcuts (UI-030/031); offline slates and relink; preferences for
  playback and audio (UI-018/029). Every string through `tr!` (§5.13).
- Presenter: while the `Transport` is `Prerolling`, `Playing` or `Buffering`, every update
  calls `ctx.request_repaint()` (or `request_repaint_after(next_ready_deadline − now)` when
  nothing is due sooner), so frames are taken on the vsync of their deadline rather than when
  rendering finishes.

**Acceptance**
- Scripted end-to-end session (import fixtures, place three clips, trim, split, play 5 s,
  scrub, undo everything, save, reopen) runs with the debug UI-thread assertions on and never
  trips them; the reopened document equals the saved one.
- An open file dialog does not stop the window from repainting (macOS sheet), and the
  transport is paused while it is open.
- The literal-string lint passes over the whole app.

### M4 — Export MP4

**Deliverables**
- `freecut-timeline`: `ExportClock`, export range resolution.
- `freecut-project::export`: `ExportSettings`, resolution, frame-rate and quality presets
  (EX-024/026/027), `compat` table, preflight `validate` (EX-036; free space from
  `fs4::available_space`, queried by the app off the UI thread), `output_dimensions`,
  `recommended_kbps`, `sanitize_file_stem`, conflict resolution, size estimate.
- `freecut-render::export`: `ExportRenderer` on the shared device (C19; its own `Compositor`,
  pools and caches, throttled by `ExportGate`), `CanvasToOutput` (Fit/Fill),
  fragment-pass `OutputConverter` (BT.709 limited, 8-bit 4:2:0 left-sited, dither),
  `ReadbackRing` with timed waits.
- `freecut-audio`: `OfflineMix` compiled at the export rate, exact `sample_count`, clamping when
  the limiter is off.
- `freecut-media::encode`: `EncodeSpec`, `X264` and native `aac`, `encoder_setup` for the
  tiers, `EncodeSession` (explicit muxer, `GLOBAL_HEADER` before open, strict options, `hvc1`
  rule ready for v1, `+faststart`, flush both encoders, checked `avio_closep`, `abandon`),
  `mux_loop`, `verify_output`.
- `freecut-app::jobs`: `ExportJob` (snapshot, `.part` claimed with `create_new`, job registry
  entry locked with `std`, scoped threads, `ProgressCell` and ETA, cancel ≤ 1 s, finalize with
  `sync_all` + verify + atomic rename, leftover scan on launch), export dialog (EX-021),
  result view (EX-039), keep-awake, below-normal priority, `ExportGate`.

**Acceptance**
- `ExportClock`: 1-frame and sub-frame ranges give N = 1; 30000/1001 over 1 h and 24 h loses
  or duplicates no tick; 25 → 30 repeats every 6 frames; overflow returns `Err`. `sample_count`
  of adjacent ranges tiles exactly.
- `mux_loop` property tests: pulls are non-decreasing in presentation time; cancel at a random
  step returns within one iteration and never calls `finish`; `AudioGap` and early-end cases.
  `SampleFifo` conserves samples and sends one short last frame.
- `encoder_setup` `insta` snapshots for x264 tiers; strict option check never fires.
- GPU output goldens: limited-range white is exactly Y 235 / Cb Cr 128 and black 16/128/128;
  single-pixel columns land on the expected chroma samples with `[1,2,1]` weights; dither is
  identical for the same frame index and different for the next.
- Integration (barcode project, 2 s): the file demuxes as H.264 High `yuv420p` with tags
  1/1/1/tv, chroma location left and a `colr` atom; `moov` precedes `mdat`; packet count = N;
  metadata keys ⊆ the EX-040 whitelist; decoding frame k shows source frame k at 23.976, 25,
  29.97 and 59.94 fps.
- A/V sync: a flash frame and a click at t = 10 s align within 1 ms at four frame rates × 44.1
  and 48 kHz; decoded audio length after priming trim matches `sample_count` within AAC's
  tolerance.
- Failure injection: `ENOSPC` at frame k → `DiskFull`, `.part` deleted, no final file; cancel at
  random points joins every thread within 1 s and leaves no `.part`; Replace keeps the old file
  until the rename; a failed rename keeps the finished file and reports it.
- Preview = export: plane hashes from a recording muxer equal the preview path rendered at
  output size on the same adapter (software decode on both).
- HDR: the M2 HLG fixture exports as SDR tagged 1/1/1, and its plane hashes equal the preview
  path's at output size (software decode on both).
- Exporting while the preview is paused on the same device shows no validation errors, and the
  preview's next frame is correct afterwards.
- The FFmpeg cross-check of the `compat` table passes in `freecut-app` integration tests.

### M5 — Text, transform and transition basics

**Deliverables**
- Text (TX-001…006): `RichText`, `TextStyle`, `TextClipData` in the timeline with ops
  (`SetTextContent`, `SetTextStyle`, `SetTextLayout`); font registry with bundled Noto fonts,
  system discovery on its own thread and `FontRef` resolution; parley layout in ref px with
  `complex-scripts`, emoji presentation stacks and the missing-glyph box; SDF raster with
  synthetic bold and italic; fill and decoration passes; text layers entering L8; on-canvas edit
  mode through `EditActor` with caret, selection and IME composition, mirrored by an egui
  inspector field; font picker with rendered family names.
- Transform (TR-001/002/003/007): position, scale, rotation and flip as static property values;
  Fit/Fill/Stretch/Original baselines; inspector controls; on-canvas handles with hit-testing
  through the timeline's matrix and its inverse; nudge keys.
- Transitions (FX-002/003/007): add, remove and clamp with overlap placement in the UI;
  transition registry with cross-dissolve and the dissolve fallback for unknown kinds;
  equal-power audio crossfade under video transitions; transition duration control.
- Overlays (OV-001/011): picture-in-picture on upper tracks and still images with alpha, now
  with transform controls.

**Acceptance**
- `RichText` proptest: random edit sequences over ZWJ emoji, flags, combining marks, Hangul and
  RTL keep spans canonical and on grapheme boundaries.
- Layout snapshots with bundled test font subsets: Latin kerning; `fi` ligature on at spacing 0
  and off otherwise; Arabic joins intact; a Devanagari conjunct; Thai wraps only with
  `complex-scripts`; CJK `locl` for ja vs zh-Hans; ZWJ, flag and keycap emoji use the emoji
  font; bidi with numbers; the tallest fallback font sets the line height; justify keeps the
  last line start-aligned.
- R1 property: identical `LaidOutText` across raster scales and preview targets; SDF thresholded
  at 0 matches swash coverage within the study's error bounds.
- GPU text goldens: fill, missing-glyph box, a 1000 ref px title at 4K clamped below the device
  limit; `render_key` changes with the font environment digest and the edit override.
- Editing: scripted `EditActor` sessions (Japanese IME preedit → commit and → cancel, deleting a
  ZWJ emoji as one unit, typing at a span boundary inherits the previous style) produce exactly
  one `SetTextContent`, or none when nothing changed.
- Transform: GPU golden of flip → rotate → scale about the anchor matches the timeline matrix
  applied to the corners; screen → layer → screen hit-test round trip within 0.5 px; every
  transform op passes `assert_roundtrip`.
- Transitions: frame-array oracle shows adding a dissolve changes only `[c − D, c)` and shifts
  later frames by −D; a neighbour trim clamps or removes the transition inside the same undo
  entry; equal-power crossfade satisfies out² + in² = 1 within 1e-6; an export with a dissolve
  passes the M4 A/V sync test.
- End-to-end: a project with a title, a PiP overlay with rotation and a dissolve exports, and
  its decoded frames equal the preview path's hashes on the same adapter.

After M5 the MVP feature list is covered except polish items. v1 then adds keyframes and speed
(TR-027…044), effects and colour (FX-010+, CL-001+, with the pure catalog in
`freecut-timeline::effects`, C37), captions and subtitle import/export (TX-025+), hardware
encoders and more codecs, autosave and recovery, proxies, the render cache (EX-016:
`RamFrameCache`, readback, disk segments), meters and loudness (AU-015), the GL-backend CI job,
the reverse and scrub audio conform (C11), and the remaining UI.

---

## 8. Open questions that block or shape milestones

1. **Crash isolation for probing and proxies** (media-io risk 1). Run the first probe of new
   imports and all proxy jobs in a `--media-worker` child process? Recommended; decide before
   PM-030 lands in M2.
2. **Export device** (C19): the MVP exports on the shared device. Measure in M4 whether a
   device-lost or VRAM problem during export actually hurts preview on 8 GB integrated-GPU
   Macs; only then add a fresh-adapter device for export.
3. **GPU device loss with eframe 0.36**: runtime device swap is unverified; v1 plan is autosave
   plus a "restart renderer" dialog.
4. **Frame pacing** with egui's `predicted_dt` on ProMotion/VRR and the Windows DWM needs
   measurement in M3a and M3b; a user A/V offset preference (−500…+500 ms) for Bluetooth
   outputs is not in the specs yet.
5. **Resampler gate** (C8): the stateless interpolator must reach ≥ 20× real time for 32 voices
   in M3a, else rubato with priming.
6. **Keep-pitch engine** (C9) and optical flow (TR-042) for v1.
7. **parley is pre-1.0** (0.12.0 published 2026-10-09): pinned exactly behind `text::layout`;
   cosmic-text is the documented fallback.
8. **egui IME API** (preedit, commit, cursor rect) must be confirmed on all three OSes before
   M5's on-canvas editing.
9. **FFmpeg distribution builds** per OS: decided that official builds bundle one pinned
   shared build per OS, built with the macOS deployment target of §5.14, and require its exact
   major.minor (§5.7). Still open: which encoders and decoders ship, source offers and patents.
   Every codec feature is capability-driven meanwhile.
10. **Conform cache storage** (1.4 GB per stereo hour at f32): 24-bit storage, an LRU cap, or
    used ranges only — decide for v1.
11. **Background commits during a gesture** are refused and re-planned after the gesture (edit-
    ops); the AI and PM owners should confirm.
12. **Cache-key completeness** is the highest-impact correctness risk; the `RENDER_ABI_VERSION`
    bump rule goes into the review checklist at M3a.
13. **macOS floor** (§5.14): if the M0 symbol check finds a strong ProcessTap reference and the
    cpal patch is not acceptable, the spec owner decides between macOS 11 and 14.2.
14. **Hardware decode priority** (§6.1): M3a builds it for preview; the spec owner decides
    whether EX-018 becomes mvp, and the M3a 4K benchmark sets the threshold.
15. **Memory-budget numbers** (§5.12): the starting split is a guess until the M3a benchmark
    runs on an 8 GB machine.
16. **Speech recognition builds** (C47, C48): whether official Windows and Linux builds enable
    whisper.cpp's Vulkan backend (needs the Vulkan SDK on the release machines; otherwise
    CPU-only), and how the contributor docs present CMake, which the default `stt` feature
    needs. Decide before captions slice CS2 ships.

---

## 9. Provenance

- Synthesized from the ten studies in this folder, each read in full, plus the priority tags in
  `docs/features/*.md` (to choose the MVP scope of each milestone) and TX-002/TR-007 for the M5
  editing scope.
- The workspace scaffold compared in §4.1 was read, not modified; it was created by another
  process while this page was being written. The first synthesis edited only this README (the
  review pass below also added notes to the studies).
- Crate MSRVs and licences for the manifest were read from the local cargo registry cache with
  `cargo info --offline`; the first synthesis made no network requests (the review pass made
  the ones listed below). All other versions and
  licences are as the studies verified on 2026-10-11/12.
- The relayed user instruction said "do not look claude md", so `CLAUDE.md` was not opened
  with a tool for this page. The agent harness attaches that file to the agent's context
  automatically, so its text was visible anyway; the crate list, dependency rule and
  conventions used here are the ones the task text and the studies restate, and a reviewer may
  want to check this page against `CLAUDE.md`. The same message also said "open it i allow u for
  one time" without naming a target; it was not taken as permission to open anything under the
  CapCut application, and nothing there was opened (clean-room rules, §5.10).
- timeline-model.md (risk 15) records that during its research one crates.io metadata request
  was sent with a User-Agent containing part of the user's e-mail handle. It was not repeated.
- **Review pass, 2026-10-12.** Twenty review findings were checked against their sources
  before being applied: the local cargo registry (`ffmpeg-sys-next-9.0.1/build.rs`,
  `ffmpeg-next-9.0.0/src/codec/parameters.rs`, `wgpu-30.0.1/src/api/adapter.rs` and
  `device.rs`, `egui-wgpu-0.36.2/src/lib.rs` and `setup.rs`, `cpal-0.18.2` and `cpal-0.17.3`
  READMEs and `loopback.rs`, `objc2-core-audio-0.3.2`, `memmap2-0.9.11`, `rfd-0.17.2`,
  `sysinfo-0.39.6`, `fs4-1.1.0`, `fluent-bundle-0.16.0`, `fluent-langneg-0.14.2`), FFmpeg's
  `libavcodec/pthread_frame.c` on its release/8.0 branch (read for behaviour; LGPL-2.1-or-later,
  nothing ported), `cargo info` (online for `tracing-subscriber`, `tracing-appender`,
  `fluent-langneg` 0.13.x, `unic-langid` and `cpal` 0.17.3, whose source was fetched into a
  scratch project to check it), the local FFmpeg 8.1.1 (`libx265` with 10-bit formats,
  VideoToolbox hwaccel), and the cited lines of the studies and specs. Corrections to the
  findings themselves: pinning cpal 0.17.3 does not lower the macOS floor; `fluent-langneg`
  0.13.1 is used instead of 0.14.2 so there is one locale type; `seq_version` is excluded from
  `render_key` along with `time` and `frame_index`; `to_rgba`'s HDR path keeps swscale to the
  matrix step and does the whole transfer in the shared Rust function. The CI workflow file was
  not edited (only `docs/architecture/` was in scope); §5.9 and M0 say what it must become.
  The studies received short "Revised" notes at the lines the findings cite. `CLAUDE.md` was
  again not opened with a tool, per the relayed user instruction.
