# Smart / AI-assisted tools

> Status: draft · Owns: analysis job kinds and their shared status model, the analysis-result key and records (stored in PM-074's media cache), the smart-tool compatibility rules (AI-001), the local ML inference runtime (`freecut-analysis`) & model manager, word-timed transcripts and transcript search, motion-track and plane-track data, face tracks, AI mattes, depth records for relighting, beat-grid refinement parameters, smart motion plans (reframe pins, camera moves), retouch presets, health-check findings and applied-fix records, and the ML engines behind AU-020, AU-021 and TX's speech features (derived media items themselves: PM-041; beat detection and beat markers: AU-025 / TL-037; the task list: PM-073 / UI-040) · Crates: freecut-analysis (proposed, see AI-001), freecut-timeline, freecut-media, freecut-render, freecut-audio, freecut-project, freecut-app

## Summary

This area covers the "smart" tools that analyze footage or audio and then do something for the user: cut out silences or filler words, split at shot changes, refine beat grids and cut clips to the music, make overlays follow a moving object, write captions from speech, find footage by what is said in it, remove or replace a background, reframe for a new aspect ratio, make slow motion smoother, upscale, colorize old black-and-white footage, retouch faces (with presets, makeup and hair color tints), relight a shot with virtual lights, blur faces, erase objects, pin inserts onto tracked flat surfaces, add subject- and beat-aware camera moves, check a whole project for common picture and sound problems, cut short clips out of long recordings, sort media into bins, and turn a pile of clips plus a song into a quick beat-synced montage. It also supplies the jobs and local models behind smart features that other areas own: beat detection's job slot (audio.md, AU-025), voice isolation, stems and voice clarity (audio.md, AU-020/AU-021), caption translation and read-aloud (text-and-captions.md), and stabilization's interplay with analysis (clip-transform-and-speed.md). Everything runs **on the user's machine** with classic signal-processing algorithms or open-licensed local models. FreeCut never uses a cloud service, an account, or any CapCut model. (The opt-in language-model assistant of [assistant.md](assistant.md) is the only place where a user may connect a hosted service, with their own key; no tool in this spec needs it. Opt-in cloud generation with the user's own Higgsfield account is specified separately in [cloud-generation.md](cloud-generation.md) and is proposed as a second exception; no tool in this spec depends on it either.) Each tool works in two steps. First an *analysis* step runs in the background and stores its result. Then an *apply* step makes a normal, undoable timeline edit, or switches on a non-destructive clip property. Before any tool that cuts material, the user reviews what it found.

## Features

### AI-001 Background analysis jobs & result cache — v1

**What the user sees:** Analysis jobs appear as rows in the app's single background-task list. The list is PM-073's, and its status-bar indicator and toasts are UI-040's; this feature adds no indicator or list of its own. Each analysis row is labelled with its **job kind** (e.g. "Detect scenes", "Transcribe", "Track motion", "Separate voice") and the clip or media name, and uses the row actions PM-073 defines. On the timeline, a clip that is being analyzed shows a thin **progress strip** along its bottom edge. A clip with finished results shows small **badges**, e.g. a speech-bubble icon for a transcript or a person icon for a cutout matte; beat markers draw their own ticks (AU-025). In Preferences → Smart tools there are:
- "Analyze on import" switches for scene detection, beat detection on audio-only files (AU-025 in audio.md), and transcription. They are all **off** by default (FreeCut default).
- The analysis pool's concurrency settings (below).
- A "Clear analysis cache" button that shows the size of the analysis part of the media cache (PM-074).

**Inputs / controls (FreeCut defaults):**
- Analysis pool concurrency:
  - Max concurrent CPU jobs: 1 to logical cores. Default: half the logical cores, at least 1.
  - Max concurrent GPU jobs: 1 (fixed in v1).
  - Throttle during playback: on/off, default **on**. Jobs drop to the lowest priority and yield the GPU while the preview plays.
- Analysis range: when a job starts from the timeline, it covers the clip's source range plus 1 s of handles on each side. When it starts from the media library, it covers the whole file.

**Behavior / output:**
- Every smart tool runs as an `AnalysisJob` on the analysis pool. This pool is separate from PM-073's media-job pools (it has the settings above) but reports its rows, progress and failures into PM-073's list. Nothing decodes, runs inference or touches files on the UI thread.
- **Result key:** PM-038's media `Fingerprint` (the only media content key: size + xxh3 of the first and last 1 MiB + probed duration) + stream index + source range + analysis kind + algorithm or model id and version + a hash of the parameters. The key holds **no media id and no modification time**, so a copied, moved or relinked file with the same content reuses the stored results, in this project and in others. Two requests with the same key share one job, so the work is deduplicated.
- **Where results live** (FreeCut's cache rule, shared with PM-041 and PM-074):
  - **Media cache (PM-074):** deterministic, regenerable outputs. These are analysis records (per-frame scene scores, silence ranges, onset envelopes, face tracks, landmark tracks, propagated masks), cutout mattes, AI-interpolated frames and processed audio. They are evictable. A missing entry is rebuilt on demand, and the clip shows a "re-analyzing" badge meanwhile. **Export waits for regeneration** (below).
  - **Generated-media store (PM-041):** derived files that a clip points at and that the user approved or that are not bit-for-bit reproducible. These are upscaled video (AI-019), inpainted video (AI-022), stems (AI-023), text-to-speech audio (AI-025), colorized video (AI-028) and generated media (AI-026). They are media items with `MediaOrigin::Generated`, never cache entries.
  - **Project document:** results that the user can correct or that an edit is bound to. These are transcripts (AI-009, with the user's corrections), tracking data bound to an overlay (AI-008), refine prompts (AI-015), beat-grid settings (AI-005), and the markers and keyframes that apply steps write. Each carries the key it came from, so it can be checked for staleness.
  - Analyses that other areas define (e.g. TR-026 stabilization, AU-025 beats) keep the storage rule of their own spec, even when they run on this scheduler.
- Results are deterministic for a given key. Export always uses the stored result, so the export matches the preview. If export needs a result that doesn't exist yet or was evicted, export first runs the job, with progress shown in the export dialog (FreeCut default), and only then encodes.
- Analysis never changes the edit model. Results are applied by each feature's own apply step.
- **Shared status model (owned here).** Every analysis row and every per-clip strip moves through the same states:
  - **Queued**: its place in line and an estimated wait ("about 3 min", "over 1 h", or "estimating…").
  - **Waiting to apply**: the result is ready and the tool that will use it is named.
  - **Running**: percent, plus an ETA once 5 % is done.
  - **Done**, **Cancelled**, or **Failed** with the reason and **Retry**.
  Waits and ETAs come from throughput measured on this machine for each job kind (seconds of media processed per second), never from a server. Every smart tool in this spec, and every other area's job on this pool, uses these states, so the reference editor's per-tool queue, "busy" and "try later" messages have no FreeCut equivalent.
- **One job per target.** A second request with the same key joins the running job. A different heavy job started while the GPU slot is busy is **queued, never refused** (FreeCut default). The reference editor runs one heavy smart feature at a time and asks the user to try again later.
- **Compare before/after.** Every inspector section of a smart tool that changes the picture has a compare eye. While CL-023's **Compare** split is active in the preview, the "before" side shows the clip with that tool bypassed. For sound tools, holding the eye plays the clip without the tool. This is view state only and never affects export.
- **Smart-tool compatibility (owned here).** Smart tools are built to compose. Stabilization, tracking, reframing, cutouts, relighting and retouch all read raw source frames and are mapped through one another (AI-007), so applying one tool never removes another. Only two real conflicts exist, and both are handled here:
  - *Motion writers.* AI-016 auto reframe, AI-017 subject follow, AI-030 camera moves and the user's own keyframes all write a clip's position, scale and rotation keyframes (TR-027). Each smart writer records itself on the clip as its `SmartMotion` origin. Applying a writer to a clip whose motion keyframes came from another writer, or from the user, asks "Replace the motion from <writer>?". On yes, only those three properties are replaced, in the same compound op. Keyframes on other properties are never touched.
  - *Source substitutes.* AI-022 object removal, AI-028 colorize and AI-019 enhance each swap the clip to a derived file. They chain in that fixed order: each step takes the previous step's output as input, and its PM-041 item names that output as `parent`. Re-running an earlier step marks the later ones **Out of date** with **Rebuild**. Until they are rebuilt, the clip plays the last valid step.
  - Observed alternative: the reference editor removes a list of other edits when auto reframe is applied (stabilization, crop, camera tracking, custom cutout, automatic camera moves), removes reframe when one of those is applied later, and removes enhancement when certain effects are added. FreeCut keeps all of them, because AI-007 makes them compatible.

**Edge cases:**
- Undo while a job runs: undoing the edit that started the job (for example switching Remove background on) never asks for confirmation. The job is cancelled, its finished part stays in the cache, and redo resumes from there. The reference editor asks before undoing a running cutout.
- Cancelling from the task list is immediate for cache-backed jobs, whose finished chunks are kept anyway. For jobs that write PM-041 derived files (AI-019, AI-022, AI-023, AI-028), it asks first when more than 1 minute of finished work would be lost (FreeCut default).
- GPU initialization fails (no supported adapter, or a graphics driver too old for the runtime): GPU jobs run on the CPU, and UI-040 shows a one-time notice that suggests a driver update. No feature is blocked. The reference editor blocks some tools until the driver is updated.
- Media goes offline during a job: the job fails with "Media missing" and partial results are thrown away. The job can be retried after relinking. A relinked file with the same fingerprint needs no re-run.
- The media file changes on disk: **staleness comes from PM-093**. When PM-093 reports the item as Changed and its recomputed fingerprint differs, cached results under the old key simply stop matching and are recomputed on demand. Document-held results from the old key (transcripts, tracking data) are marked **stale**. They are not used for new apply steps until the user re-runs the job or chooses "Keep anyway". User-corrected transcripts are never thrown away silently.
- The app quits or the project closes during a job: the job is cancelled and its partial cache files are deleted. Jobs that work in chunks (e.g. transcription) may keep finished chunks and resume later.
- Disk full while writing to the cache: the job fails cleanly, and its half-written file is removed (PM-074's atomic writes).
- Several clips use the same media: the analysis runs once, and each clip maps the result through its own in/out points and speed.
- Zero-length or still-image clips: audio and motion analyses are disabled for them, and a tooltip says why.
- The user trims a clip beyond the range that was analyzed: only the new part is analyzed. Tools that need warm-up frames (e.g. recurrent matting) take extra frames before the new part.
- GPU runs out of memory: the job retries once on the CPU and shows a notice through UI-040.
- PM-074 evicts an entry the open project needs while playing: PM-074 pins entries used by the open project's visible items; anything else is rebuilt on demand.

**Undo:** n/a (jobs and caches are not edits). Cancelling a job can't be undone. Each feature's apply step is its own undo entry.

**Model notes:**
- Propose a new crate, **`freecut-analysis`**. It is the single name for FreeCut's analysis and ML runtime, in every spec. It takes buffers in and returns plain data. It holds the classic algorithms (silence, scenes, tracking, caption segmentation, cleanup, transcript search) and the ML inference wrappers, behind cargo features such as `ml-onnx` and `ml-whisper`.
  - It does not depend on `freecut-media`. `freecut-app` streams decoded frames and PCM from `freecut-media` into it.
  - It depends on `freecut-timeline` only for the rational time type and plain data types such as `Fingerprint` and `CaptionDraft`, or on a future tiny shared crate.
  - Results are expressed in **source time** (rational ticks plus the stream's timebase), never `f32` seconds.
  - *Decided (architecture C48, 2026-10-13):* the crate is adopted with the captions build (slice CS2 of [speech-and-captions.md](../architecture/speech-and-captions.md)). It depends on `freecut-timeline` only, forbids `unsafe`, and gates whisper.cpp behind `ml-whisper` (the app's default feature `stt`), so `freecut-app` also builds without it. `ml-onnx` comes with the first ONNX model.
- Background analyses owned by other areas (TR-026, AU-017, AU-025, FX-020/FX-021) may be queued on the same analysis pool so they share throttling, the task-list rows and the export wait. Their owning specs still define the work itself.
- `AnalysisKey { fingerprint: Fingerprint /* PM-038 */, stream: u32, source_range: TimeRange, kind: AnalysisKind, algo_id, algo_version, params_hash: u64 }`.
- `AnalysisJob { id, key: AnalysisKey, target: MediaRef, params, model: Option<ModelRef>, state }` and the analysis pool live in `freecut-app`, which forwards progress into PM-073's `JobQueue` view.
- `AnalysisRecord { key: AnalysisKey, payload }` is serialized with serde and stored through PM-074's cache API (`freecut-media::cache`). Large artifacts (mattes, frames, audio) are encoded and decoded by `freecut-media`.
- `JobState { Queued { position: u32, eta: Option<Duration> }, WaitingToApply { tool }, Running { fraction, eta }, Done, Cancelled, Failed { reason } }` lives in `freecut-app` and is what PM-073's `TaskRow` shows for analysis rows. ETAs come from a per-kind throughput average kept in app preferences.
- `SmartMotion { origin: SmartMotionOrigin /* Reframe | Follow | CameraMoves */, params, seed: Option<u64> }` is an optional field on the clip in `freecut-timeline`, written by the motion writers' ops. A pure `motion_conflict(clip, writer) -> Option<Origin>` decides when to ask, and is unit-tested.
- The substitute chain is read from PM-041's `parent` links. A pure `chain_order(kind)` fixes the order (Inpainted → Colorized → Upscaled), and `stale_after(step)` lists the steps to rebuild.

### AI-002 Local model manager & AI privacy settings — v1

**What the user sees:** Preferences → Smart tools → **Models** lists every optional model pack. Each row shows its name, task (speech recognition, cutout, interpolation, upscaling, …), size on disk, license (short id plus a link to the full text), upstream origin, and status (not installed / downloading x% / installed / update available / damaged). Each row has Install, Remove, Import from file… and Show folder buttons. The first time a user starts a feature that needs a missing model, a dialog explains what will be downloaded, its size, its license, and where it comes from. The choices are **Download**, **Choose file…** and **Cancel**. Nothing is downloaded without this explicit confirmation. The same page has:
- Compute device: Auto / CPU only / a specific GPU adapter.
- CPU thread limit.
- Model folder.
- A switch named "Allow model downloads". Default: on, but downloads only ever happen after a confirmation click.
- A **Face & body analysis** switch. Default: on. The first time a tool detects faces or bodies (AI-016, AI-017, AI-020, AI-021, AI-033, and AI-034's Faces target), a one-time notice explains that the detection runs on this computer only, that its results stay in the project and the media cache, and where to switch it off. When the switch is off, those tools are disabled and a tooltip points to this page. The reference editor asks for consent before face and body detection; FreeCut gives a notice instead, because nothing leaves the machine.

**Inputs / controls:**
- Compute device: default **Auto**. Auto picks the GPU when one is supported, otherwise the CPU.
- Thread limit: 1 to logical cores. Default: all cores minus 1.
- Model folder: default is `<per-user application data directory>/FreeCut/models`.
- Optional quality tiers for a task (e.g. speech recognition Fast / Balanced / Accurate). Each tier maps to a specific model in the manifest.

**Behavior / output:**
- FreeCut ships a **manifest** of known models. Each entry lists id, version, task, download URLs (the upstream project's official release location), SHA-256, size, license id, a URL to the license text, input/output spec, and minimum runtime version.
- A download goes to a temporary file, resumes with HTTP range requests, is checked against the SHA-256, and is then moved into place in one atomic step.
- **Import from file:** the file is hashed and must match a manifest entry. Files that match no entry are rejected. A "custom model" escape hatch is out of scope for now.
- Runtimes:
  - ONNX models go through ONNX Runtime via the `ort` crate. Execution providers: CoreML on macOS, DirectML on Windows, CUDA optional, CPU fallback everywhere.
  - Speech recognition goes through whisper.cpp via Rust bindings. Its own Metal, CUDA, Vulkan and CPU backends are used.
- No telemetry and no background update checks. "Check for model updates" is a manual button.
- All inference is local. Media, transcripts and frames never leave the machine.
- *Download policy (captions build, 2026-10-13; architecture C49):*
  - **Opt-in only.** The confirmation names the pack, its task, the exact size (and the free space at the destination), the licence with a link, the host it comes from and the folder it goes to. A speech tier and the 0.9 MB voice-activity model are one confirmation and one combined size.
  - **Upstream and pinned.** Each manifest URL points at the upstream project's own model host and names a fixed repository revision, never a moving branch, so the bytes behind an entry cannot change; the entry's SHA-256 is checked before the file is used.
  - **Stored per version** under the model folder as `<model-id>/<version>/<file>`; an `installed.json` beside it records the manifest entry it was checked against.
  - **Resumable.** The download writes `<file>.part` plus a small state file (URL, expected size and hash, the server's validator). A later attempt continues with a range request tied to that validator; if the server sends the whole file instead, it starts over. The hash covers the whole file, including the part already on disk.
  - **Cancellable.** **Cancel** stops within one read and keeps the partial file for the next attempt; **Discard partial download** in the Models page deletes it. Partial files untouched for 30 days are removed at start-up (FreeCut default).
  - **Offline afterwards.** Installed models load from disk only. FreeCut makes no network request for models except when the user clicks Download, Retry or **Check for model updates**.
  - The downloader is a background job in `freecut-app` (blocking `ureq` with rustls, `sha2` for hashing); it honours the system proxy settings and never runs on the UI thread.

**Edge cases:**
- Hash mismatch: the file is deleted and "Download damaged — try again" is shown.
- Not enough disk space: the download is refused up front, using the size from the manifest.
- The user removes a model that a running job is using: removal waits until the job ends.
- A newer model version is installed: existing stored results keep the model version that made them and stay valid. Only a re-run uses the new version.
- A project opened on a machine without the model: stored results (transcripts, track data) still work. Only actions that need inference ask to install.
- No supported GPU: the job runs on the CPU. A CPU without the instruction sets an optimized runtime build expects (e.g. AVX2): the runtime's generic CPU path is used. In both cases, if the job would take more than 10× real time, a warning is shown first.
- No network: Download shows "offline" and offers Choose file….

**Undo:** n/a (preferences).

**Model notes:**
- `ModelManifest`, `ModelId`, `ModelVersion`, `ModelStatus` and the runtime-selection logic live in `freecut-analysis`. They are pure data and loaders.
- The downloader (network I/O) and the preferences UI live in `freecut-app`.
- No `unsafe` in our code. FFI stays inside the third-party binding crates.

### AI-003 Silence detection & removal — v1

**What the user sees:** Select one or more clips that have audio, or a whole track, and choose **Remove silences…** from the right-click menu or the Smart tools menu. A dialog shows a strip for each selected clip, with shaded bands over the detected silent ranges. It also shows a list of ranges (start, length, a checkbox each, and click-to-jump-playhead) and a summary such as "Removes 1:42 of 12:03". Changing a slider updates the bands live, because analysis is cheap and cached. **Apply** makes the edit.

**Inputs / controls:**
- Threshold: −80 to −10 dBFS, step 1. Default: **−40 dBFS**.
- "Auto threshold" button: estimates the noise floor (10th percentile of short-term loudness) and adds 6 dB.
- Minimum silence length: 100 ms to 10 s. Default: **500 ms**.
- Keep padding (speech kept on each side of a cut): 0 to 1000 ms. Default: **150 ms**.
- Detection source: this clip's audio (default) / a chosen linked or overlapping audio clip (e.g. a separate microphone recording).
- Action: **Delete and close gaps** (ripple, default) / Delete and leave gaps / Split only / Add markers only.
- All values are FreeCut defaults.

**Behavior / output:**
- The audio is downmixed to mono at its native rate. RMS is measured over 20 ms windows with a 10 ms hop.
- A silent range is a run of windows below the threshold that lasts at least the minimum length. Padding is taken off both ends.
- Range edges are snapped to the project frame grid, rounding **inward** so slightly more material is kept. Ranges shorter than one frame after snapping are dropped.
- The analysis uses the source audio before clip volume, fades and track mute, so a muted clip can still be analyzed.
- Ranges are found in source time and mapped to timeline time through the clip's time map (speed and reverse). That map is owned by clip-transform-and-speed.md.
- The cuts are applied to the clip and its linked clips (video + audio) as splits and deletions. Ripple behavior across other tracks follows timeline-editing.md.

**Edge cases:**
- Clip without audio: the item is disabled with a tooltip.
- The whole clip is below the threshold: a confirmation says the entire clip would be removed.
- Silence at the very start or end of a clip: the in/out point is trimmed instead of splitting off a fragment.
- Selected clip on a locked track: the item is disabled. Other locked tracks are never changed. If a ripple would need to move a locked track, that track is left as is and the user is told.
- A transition overlaps a removed range: it is removed, following the transition rules in timeline-editing.md and effects-transitions-animations.md.
- Clips with keyframes: splits keep the keyframes, following the split rules in timeline-editing.md.
- Speed-ramped (curve) clips: the mapping is evaluated per range edge, so ranges can shrink or grow on the timeline. Reversed clips: each range maps reversed.
- Very long recordings (hours): analysis streams in chunks, and the list is virtualized.
- Steady background music under speech: speech pauses won't fall below the threshold. A hint suggests voice isolation first (AU-021, run by the AI-023 engine) or picking the dry voice track as the detection source.
- Applying is all-or-nothing. If any edit in the batch fails validation, nothing changes.

**Undo:** One compound op, "Remove silences (N ranges)", made of splits and range deletions (ripple or not). One undo restores everything.

**Model notes:**
- `freecut-analysis::silence::detect(pcm, sample_rate, params) -> Vec<SampleRange>` is a pure function. The audio is decoded by `freecut-media`.
- The mapping and op building happen in `freecut-app`, using the `Clip` time mapping and the `Split` / `DeleteRange { ripple }` ops from `freecut-timeline`.
- The parameters persist as the user's last-used settings (app preferences), not in the project.

### AI-004 Scene / shot detection & auto split — v1

**What the user sees:**
- **Timeline:** right-click a video clip → **Split at scene changes…**
- **Media library:** right-click a video → **Detect scenes**. This adds scene markers to the media item. Each detected shot shows as a sub-range thumbnail, and dragging one to the timeline inserts only that range.

The dialog shows a strip of first-frame thumbnails, one per detected shot, each with a checkbox and its timecode. A slider changes sensitivity and the strip re-computes from cached per-frame scores.

**Inputs / controls:**
- Detector: **Content change** (default) / Adaptive (for footage with fast camera moves) / Fades (cuts through black or white).
- Sensitivity: 0–100. Default: **50**. 50 maps to a content-difference threshold of 27 on PySceneDetect's scale; higher sensitivity means a lower threshold and more cuts.
- Minimum shot length: 0.1–10 s. Default: **0.6 s**.
- Fast mode: analyze every 2nd frame. Default: off.
- Two quick presets above the slider: **Fewer, longer pieces** (sensitivity 35, roughly one cut per scene) and **More, shorter pieces** (sensitivity 65, roughly one cut per camera shot). The reference editor offers only these two levels. FreeCut keeps the slider too, for footage where neither fits.
- Action: **Split** (default on the timeline) / Add markers only.

**Behavior / output:**
- Frames are downscaled to at most 256 px wide. For each frame, a weighted difference to the previous frame is computed over hue, saturation and luma (edges optional).
  - Content mode cuts when the score passes the threshold.
  - Adaptive mode compares the score to a rolling average of its neighbors.
  - Fade mode watches mean luma cross a dark/bright threshold.
- A cut is placed at the first frame of the new shot. Splits fall on that frame's timeline position.
- Per-frame scores are stored as an AI-001 analysis record in the media cache (PM-074), so changing sensitivity needs no re-decode.

**Edge cases:**
- No cuts found: the dialog says so and suggests the **More, shorter pieces** preset. Nothing changes.
- Dissolves and other gradual transitions are often missed. The dialog says so, and the user can add a split by hand.
- Camera flashes and very fast pans can cause false cuts. Adaptive mode or a lower sensitivity helps.
- Variable-frame-rate media: frames are handled by their presentation timestamps, kept as rational time.
- A cut exactly at the clip's in or out point is ignored. Existing splits are never duplicated.
- A clip shorter than twice the minimum shot length gives no cuts. Still images: disabled.
- Speed-changed or reversed clips: cuts map through the time map.
- Linked audio is split at the same points (timeline-editing.md linking rules).
- Locked track: disabled.
- Interlaced sources: analysis runs on the deinterlaced frames that `freecut-media` provides.

**Undo:**
- Timeline: one compound op, "Split at scene changes (N)".
- Markers only: one op that adds the timeline markers.
- Media library: one undoable "Add scene markers" on the media item. The media item model is owned by project-and-media.md.

**Model notes:**
- `freecut-analysis::scenes` provides `frame_score(prev, cur, weights)` and `detect(scores, params) -> Vec<FrameIndex>`. It is a fresh Rust implementation of the publicly documented histogram/content approach; PySceneDetect (BSD-3-Clause) is the reference.
- Shot ranges are stored as an `AnalysisRecord`.

### AI-005 Beat grid refinement — v1

**What the user sees:** Beat detection belongs to audio.md (AU-025): the onset, tempo and beat-tracking job (which runs on AI-001's analysis pool), its density and sensitivity controls, the tempo readout, the tick marks and the tap key (with AU-026's latency compensation). Beats are stored only as TL-037 clip markers with `origin = Beat`. This feature does not redefine any of that. It owns only **grid refinement**: a **Beat grid** popover, opened from AU-025's tempo readout on a clip that has auto beat markers. The popover shows the detected tempo with ×2 and ÷2 buttons, a manual BPM field, a meter choice and an offset nudge. Every change redraws the clip's auto beat markers at once from the cached analysis, so the user can line the grid up with the music by ear.

**Inputs / controls (FreeCut defaults):**
- Tempo: ×2 / ÷2 buttons, or a manual BPM from 30 to 300. Default: the tempo AU-025 detected.
- Meter used to place downbeats: **4/4** (default) or 3/4.
- Offset nudge: ±half a beat, in 10 ms steps. Default: **0**.
- **Reset**: returns every value to AU-025's detected result.
- Density and sensitivity are AU-025's controls. The popover shows their current values read-only, with a link to AU-025's controls.

**Behavior / output:**
- AU-025's tracker is re-run on its cached onset envelope with the user's constraints: a fixed beat period when the tempo is forced or halved/doubled, the chosen meter for downbeat spacing, and the offset added to every beat. AU-025's current density and sensitivity apply unchanged.
- The result replaces the clip's **auto** beat markers (TL-037 clip markers, `origin = Beat`, flagged auto) in source time. **Manual** beat markers (flagged manual, e.g. placed with AU-025's tap key) never move.
- The offset is stored in rational ticks; the 10 ms step is only the UI increment.

**Edge cases:**
- Clip without auto beat markers: the popover offers **Detect beats** (AU-025) first.
- No steady pulse (AU-025 reports a variable tempo): the tempo field and ×2/÷2 are disabled; the offset still works.
- A manual BPM more than ±30 % away from the detected one: markers follow the forced grid and may drift from the music; a hint suggests ×2 or ÷2 instead.
- An offset that pushes markers outside the clip's source range: those markers are hidden like trimmed markers (TL-037), not deleted.
- The user later changes AU-025's density or sensitivity: AU-025 re-runs detection, and the stored grid settings are re-applied to the new result.
- The onset envelope was evicted from the media cache (PM-074): it is recomputed before the redraw, and the popover shows AI-001's progress strip meanwhile.
- Speed-changed or reversed clips: markers stay in source time, so TL-037's and AU-025's display mapping applies unchanged.
- Locked track: the popover is read-only.

**Undo:** Each change is one compound op: `SetBeatGridParams { clip, old, new }` plus TL-037's `DeleteMarkers` (the old auto beat markers) and `AddMarker` (the regenerated ones). Undo restores both. A drag of the offset nudge coalesces into one entry.

**Model notes:**
- `BeatGridParams { bpm_override: Option<Rational>, meter: Meter, offset: Tick }` lives on AU-025's **beat-analysis record**, in its document part, so it is undoable and survives cache eviction. It does not live in `ClipAudio`, and there is no `ClipAudio.beats` or `BeatMarker` type.
- The tracker and its onset envelope are AU-025's. This feature only passes constraints. Turning a beat list into TL-037 marker ops is a pure op builder in `freecut-timeline`, with unit tests for "manual markers never move".

### AI-006 Auto beat sync (fit clips to beats) — v1

**What the user sees:** Select several video or image clips on one track, plus a music clip that has beat markers (TL-037 clip markers with `origin = Beat`, placed by AU-025), and choose **Sync to beats**. A small dialog previews the resulting lengths. On Apply, the clips are trimmed and laid end to end so that each cut lands on a beat.

**Inputs / controls:**
- Cut every N beats: 1 / **2** (default) / 4 / 8.
- Start at: **first beat at or after the first selected clip's start** (default) / first beat after the playhead / music start.
- Clips shorter than their slot: **Skip to the next beat after the clip ends** (default; leaves the clip whole) / Hold the last frame to fill the slot.
- Images: take exactly N beats each.
- Order: current timeline order (fixed in v1).

**Behavior / output:**
- Beat times are mapped from the music clip into timeline time.
- Each selected clip, in turn, gets a slot from the current beat to beat + N. Its out point is trimmed to the end of the slot (its in point is never changed), and it is placed at the start of the slot.
- Following clips on the same track ripple. Other tracks follow the ripple rules in timeline-editing.md.

**Edge cases:**
- No beat markers on the chosen music: the dialog offers to run beat detection (AU-025) first.
- More clips than beats left: the extra clips go after the last beat, unchanged, with a warning. Fewer clips: the rest of the music is left alone.
- Clips on several tracks are selected: only the topmost selected track is processed, and the user is told.
- Transitions between clips: each is kept if both neighbors are still adjacent and have enough handle media, otherwise removed (effects-transitions-animations.md).
- The music clip has a speed change: beat times go through its time map.
- Speed-changed video clips: trimming happens in timeline time.
- Locked track: disabled.

**Undo:** One compound op, "Sync N clips to beats" (trims + moves).

**Model notes:** `freecut-timeline::ops::fit_to_grid(clips, grid_times, policy)` is a pure op builder that a unit test can cover in isolation. Beat times are read from the music clip's TL-037 clip markers with `origin = Beat`, auto and manual, after any AI-005 refinement. AI-029 reuses this op builder.

### AI-007 Stabilization-aware analysis — v1

**What the user sees:** Video stabilization itself (the switch, its strength levels, the motion analysis and the correction) is defined in clip-transform-and-speed.md (TR-026). This feature keeps the smart tools in this spec correct on stabilized clips. Tracking paths, face boxes, cutout mattes, refine strokes and reframe keyframes stay lined up with the picture whether stabilization is on or off. Turning stabilization on or off, or changing its level, never makes these tools analyze the clip again.

**Inputs / controls:** None. This is fixed FreeCut behavior.

**Behavior / output:**
- Every pixel analysis in this spec (AI-008, AI-014, AI-015, AI-016, AI-017, AI-020, AI-021, AI-022) runs on the **raw source frame**: the output of FX-001's L2 stage, before L2b Retouch (AI-020), TR-026's correction and the crop at L3. Results are stored per source frame, in source-pixel coordinates. Because analyses read the frame before L2b, changing retouch settings never re-runs them.
- When a result is used, it is mapped into the picture the user sees: through TR-026's per-frame correction and the crop (L3), then through the clip's transform (L8) for anything placed on the canvas.
- Mattes and masks (AI-014, AI-015, AI-022) are warped with the same correction as their frame before they are applied at L4.
- Anything the user draws in the preview (a tracking box, a refine stroke, a custom reframe box) is mapped back the inverse way before it is stored.

**Edge cases:**
- TR-026's analysis isn't finished yet: the identity correction is used, which matches the unstabilized preview TR-026 shows meanwhile. When the correction arrives, overlays move with the picture and no AI analysis re-runs.
- A mapped point lands outside the visible frame: the overlay follows it off-canvas like any off-canvas layer; the stored data is not clipped.
- TR-026's static zoom and the crop scale mapped points exactly as they scale the picture, so a tracked overlay doesn't drift when the strength level changes.
- Still images: the correction is the identity.
- Rolling-shutter wobble that TR-026 leaves uncorrected: results follow the raw frame, so they wobble together with the picture.

**Undo:** n/a. This feature has no edit of its own; stabilization changes are TR-026's `SetStabilize`.

**Model notes:**
- `map_source_to_layer(frame, point)` and its inverse are pure functions over TR-026's correction matrix and the crop rectangle. They live in `freecut-render`, which already reads that correction to draw the frame; `freecut-app` uses the inverse for preview input.
- No new persisted data.

### AI-008 Motion tracking (attach to a moving object) — v1

**What the user sees:** Select an overlay that sits above a video clip: text, sticker, image, picture-in-picture video, a mask, or a blur/mosaic effect region. Open Inspector → **Tracking**. Pick the clip to track. The default is the topmost video clip under the overlay at the playhead. A tracking box appears on the preview at the playhead; drag and resize it around the target and press **Track**.
- Instead of drawing the box, the user can **click an object** in the preview. The click seeds the box from AI-015's click selection when the segmentation model is installed, or from a default box centred on the click when it is not. **Face** and **Person** chips snap the box to the nearest face (AI-021's detector) or person (AI-016's detector).
- Masks track too: a TR-017 shape mask or a CL-031 color mask on a clip can follow that clip's own content. The tracked clip is then the masked clip itself.
- The Tracking section also offers **Plane** mode for flat surfaces (AI-031).
- While tracking, the preview shows the box following the target, with a progress bar and a Stop button.
- Afterwards, the overlay follows the target. A dotted path shows the tracked positions, and a confidence strip under the clip marks weak or lost segments in amber or red.
- If tracking loses the target, the user moves the box to the right place on that frame and presses **Continue**.
- **Bake to keyframes** turns the result into ordinary position/scale/rotation keyframes. **Remove tracking** detaches the overlay.

**Inputs / controls:**
- Direction: **Forward from playhead** (default) / Backward / Both.
- Follow: position (always); scale (default **on**; off keeps the overlay's own size); rotation (default off).
- Quality: Fast / **Accurate** (default).
- Path smoothing: 0–100. Default: **20** (FreeCut default). It is a zero-phase low-pass filter on the samples, applied when the path is evaluated, so changing it never re-tracks.
- Initial box: 120×120 source pixels, centered on the frame (FreeCut default). The search area is 2× the box.

**Behavior / output:**
- The tracker finds the box from one frame to the next with normalized cross-correlation template matching on an image pyramid, refined to sub-pixel accuracy with Lucas–Kanade. Scale is found by a multi-scale search and rotation by an angle search.
- Each frame gets a confidence score. Five frames in a row below 0.5 mark the target as **lost** and tracking stops.
- The result is `TrackData`, sampled per **source frame of the tracked clip**.
- The overlay keeps an **offset** from the tracked box, taken at the reference frame. Dragging the overlay later changes the offset, not the path.
- At any timeline time, the overlay's position is the track sample (mapped through the tracked clip's time map, its stabilization correction (AI-007) and its current transform) plus the offset, plus any keyframes the user added.
- Outside the tracked range, the nearest sample is held.
- **Path corrections:** on a paused frame, the user can drag a point of the dotted path. That moves the sample, and tracking resumes from that frame in the chosen direction with the corrected box.
- **Re-track:** on an overlay that already has a track, **Track** asks to replace the samples in the chosen direction from the playhead. Samples on the other side are kept. Overlays and effects bound to the track stay bound. The reference editor deletes the effects attached to the old track when the user re-tracks.

**Edge cases:**
- Range too short: tracking needs at least 2 frames in the chosen direction. At the clip's first frame Backward is disabled, and at its last frame Forward is disabled.
- Target not visible on the current frame (the box lies outside the picture): Track is disabled, and a hint asks the user to move the playhead to a frame where the target is visible.
- Target occluded or leaving the frame: marked lost. The user continues tracking from a frame where the target is visible again.
- The tracked clip is moved, trimmed or retimed: the binding still holds, because samples are in source time.
- The tracked clip is deleted: the overlay keeps its current position, a warning badge appears, and undo restores the link.
- The tracked clip has its own scale or position: samples go through its current transform, so the overlay stays on target.
- Stabilization on or off: tracking data is kept in raw source coordinates and mapped through TR-026's correction (AI-007), so toggling stabilization needs no re-track.
- Box partly outside the frame: the search is clamped to the frame.
- Overlay longer than the tracked clip: the first or last sample is held.
- Low-contrast or very small targets (< 16 px): a warning before tracking starts.
- The tracked clip is split: TL-023's split copies the binding to the new piece, so the overlay follows whichever piece lies under it. Samples are in source time, so both pieces use the same track.
- Several clips can be tracked at the same time, within AI-001's pool limits. Each clip runs one tracking job at a time.
- Observed alternative: the reference editor drops tracking when the tracked clip is moved, scaled, rotated or retimed, keeps it only on the first piece after a split, and runs one tracking job at a time across the app. FreeCut keeps tracking in all these cases, because samples are in source time and are mapped through the clip's current transform and time map.
- Locked track: disabled.

**Undo:**
- "Track motion" (attach `TrackData` + binding): one op.
- Each path-point drag, and each change of path smoothing: one op.
- Each manual correction plus partial re-track: one op.
- "Bake to keyframes": one op.
- "Remove tracking": one op.

**Model notes:**
- `TrackData { source_clip: ClipId, smoothing: u8, samples: Vec<TrackSample { src_time: RationalTime, center: Vec2, scale: f32, rotation: f32, confidence: f32 }> }`. Spatial values are floats; time is rational. Smoothing is applied at evaluation by a pure function, so stored samples stay raw.
- The overlay binding has a source: `Point(TrackData)` (this feature), `Plane(id)` (AI-031) or `Face(id)` (AI-021's face tracks, for shape covers). Its clip ids are kept current through TL-023's split rule.
- The binding lives on the overlay in `freecut-timeline`. The bound position is evaluated in `freecut-render`, which also reads the stabilization correction. Keyframes and their evaluation are owned by clip-transform-and-speed.md.
- Pinning an overlay or sticker to a moving object in overlays-and-stickers.md (OV-022) uses this tracker and **Bake to keyframes**.
- The tracker is in `freecut-analysis::track`, written in pure Rust. Later upgrade: SAM 2-based object tracking (AI-015) for targets that change shape.

### AI-009 Local speech transcription — v1

**What the user sees:** Right-click a clip or a media item → **Transcribe…**. A small dialog offers:
- Language: **Auto-detect** (default), or a list of about 99 languages.
- Quality: Fast / **Balanced** (default) / Accurate.
- Voice-activity filter: **on** (default).
- Task: Transcribe (default) / Translate to English.
- Optional word hints: names and jargon that bias recognition.

Progress shows per chunk. When it finishes, the transcript is attached to the media and appears in the Transcript panel (AI-011). It also feeds auto captions (AI-010) and spoken-word search in the Media panel (AI-027).

**Inputs / controls:**
- Quality tiers map to manifest models, all quantized (FreeCut default):

  | Tier | Model | Manifest file (download size) |
  |---|---|---|
  | Fast | base | `ggml-base-q5_1.bin` (60 MB) |
  | **Balanced** (default) | small | `ggml-small-q5_1.bin` (190 MB) |
  | Accurate | large-v3-turbo | `ggml-large-v3-turbo-q5_0.bin` (574 MB) |

  Model sizes are shown from the manifest (AI-002). Each tier installs together with the voice-activity model `ggml-silero-v6.2.0.bin` (0.9 MB). Exact sizes, SHA-256 values and the pinned source revision are in [speech-and-captions.md §3](../architecture/speech-and-captions.md#3-speech-model-packs). The Accurate model was trained for transcription only, so **Translate to English** with Accurate runs on the Balanced model, and the dialog says so.
- Audio stream: default is the stream the clip uses. Channel: mix (default) / left / right.
- Word hints: free text, up to 200 characters.

**Behavior / output:**
- `freecut-media` decodes the audio and resamples it to 16 kHz mono.
- A voice-activity detector (Silero VAD, MIT, in the ggml form that whisper.cpp runs) finds speech regions. whisper.cpp then transcribes them in windows of about 30 s, with token-level timestamps and DTW word alignment. FreeCut cuts the audio into chunks of at most 5 minutes at silences and feeds each chunk separately, so the mapping back to source time is FreeCut's own tested arithmetic, and each finished chunk is cached so a cancelled job resumes (pipeline: [speech-and-captions.md §5](../architecture/speech-and-captions.md#5-transcription-pipeline-ai-009)).
- The result is a list of words. Each word has text, start and end (rational, rounded to 10 ms), and a confidence. Words are grouped into sentence segments, and word edges are refined using the VAD boundaries.
- With Auto-detect, the language is detected once from the first 30 s of speech (FreeCut default). An option detects it per segment instead.
- The transcript is document data, saved in the project because the user can correct it (AI-011). It carries its AI-001 key (PM-038 fingerprint + stream + source range + model id and version + params hash). The raw recognizer output is also kept as an analysis record in the media cache (PM-074), so another project that uses the same file, or a copy of it, reuses it without recognizing it again.

**Edge cases:**
- Music-only or silent media: "No speech found". The voice-activity filter guards against invented text.
- Repetition loops in the output: identical n-grams repeated more than 3 times in a row are dropped.
- Mixed-language speech: per-segment detection is offered.
- Very long files: chunked, resumable, with ETA. Memory stays within the model's working set.
- Multiple audio streams: the user picks one. Stereo interviews with one speaker per channel can pick a channel.
- Media relinked to different content, or changed on disk (PM-093): the fingerprint differs, so the transcript is marked stale (AI-001). A relink to an identical copy keeps it valid.
- Re-running over a transcript the user has corrected: confirmation required. The corrected words are kept and only the timings are re-aligned, when the user chooses that.
- Speed-changed clips: the transcript is in source time; consumers map it.
- Model not installed: model manager prompt (AI-002).

**Undo:** The recognition itself is analysis. Attaching its transcript to the project is a labelled entry, `AttachTranscript { media, stream, before, after }` ("Transcribe"), because the transcript is document data (architecture §5.11, C50). Inside an Auto captions run it is part of that run's single "Auto captions" entry (AI-010). Undo removes the transcript; the raw result stays in the media cache, so redo or a re-run is instant. User corrections to transcript text are their own entries (AI-011).

**Model notes:**
- `Transcript { media: MediaId, key: AnalysisKey, stream: u32, language: LangTag, model: ModelRef, words: Vec<Word { text, start: RationalTime, end: RationalTime, confidence: f32, edited: bool }>, segments: Vec<Range<usize>> }` is persisted by `freecut-project`. `media` links it to the media item in this project; `key` is what staleness and reuse are checked against.
- Inference runs in `freecut-analysis` (feature `ml-whisper`).
- *Engine decision (architecture C47, 2026-10-13):* whisper.cpp 1.8.3 through `whisper-rs` `=0.16.0` (Unlicense; `whisper-rs-sys` `=0.15.0` builds the bundled sources with CMake, offline). Metal on macOS, CPU elsewhere, Vulkan in Windows and Linux release builds if the release pipeline can provide it. sherpa-onnx (Apache-2.0) is the planned second engine for diarization and fast English-only models, through the ONNX runtime once it exists. The comparison is in [speech-and-captions.md §2](../architecture/speech-and-captions.md#2-choosing-the-local-speech-recognizer).
- whisper.cpp and the Whisper weights are MIT-licensed. Other open-source editors (Kdenlive, Shotcut) also ship local Whisper-based speech-to-text, which supports this approach.

### AI-010 Auto captions — v1

**What the user sees:** In the Captions panel, **Generate captions from speech…** opens the auto-captions dialog. The caption track it fills, the caption line rules and how results land on the track belong to text-and-captions.md (TX-037, TX-038, TX-039); this feature defines the job and the dialog behind the command. The dialog offers:
- Mode: **Recognize speech** (default) / Recognize song lyrics (isolates the vocals with AI-023 first; results get `origin: Lyrics`, TX-044) / **Align known text** (later; see the mode below).
- Scope: selected clips / **whole timeline** (default) / in–out range.
- Language.
- **Use existing transcripts**: media that already has a transcript (AI-009) is not recognized again.
- Line rules: TX-037's rules for the receiving track, shown and editable in place.
- Keep word timing (for TX-039's active-word highlight), mark filler words, and include muted clips.
- Output: **new caption track** (default, as in TX-038) / replace existing captions in the range.

**Generate** runs any missing transcriptions (AI-009), then builds caption segments. They can then be edited like any other caption.

*Where the dialog lives (captions build, 2026-10-13):* the form is the **From speech** row of the Captions drawer (TX-057); the panel and context-menu entries open it there. Its rows map onto this dialog: Speech language → Language; Replace earlier auto captions → Output (replace mode, default on when the track already holds unedited recognized captions); Quality → AI-009's tier; Apply to → Scope; Mark filler words → the switch below, with **Keep them, dimmed** (default; filler words keep their place and get `WordFlags::FILLER`) or **Leave them out** (filler words are removed from the drafts before packing). The drawer adds **Second line**, which runs a second step after recognition: *English, from the speech* re-runs AI-009 with Translate to English on the same audio and attaches the result by TX-036's overlap rule; other engines are AI-013's. *Recognize song lyrics* and *Align known text* are the **From a song** row.

**Inputs / controls (FreeCut defaults):**
- Scope: **whole timeline**. Language: **Auto-detect**.
- Use existing transcripts: **on**.
- Keep word timing: **on**.
- Mark filler words: **off**. When on, words that match AI-012's filler lists are tagged, not removed.
- Include muted clips: **off**.
- Characters per line, lines, durations and punctuation breaks: TX-037's values. They are not redefined here.
- Keyword emphasis: **off**. When on, the drafts carry TX-040's keyword tags. The emphasis rules themselves are TX-040's.

**Behavior / output:**
- For every clip in scope that has audio: take its media's transcript, keep the words inside the clip's used source range (a word counts if at least 50 % of it is inside), and map them to timeline time through the clip's time map (clip-transform-and-speed.md).
- Words are packed into segments with TX-037's re-flow. Segment edges snap to project frames: start rounds down, end rounds up, and every segment is at least one frame long.
- Segments on one caption track never overlap (TX). When clips on different tracks speak at the same time, the earlier segment is shortened to end where the later one starts (FreeCut default). An option puts the overlapping speech on a second caption track instead.
- Segments land through TX-038's `ReplaceCaptions` op as caption drafts with `origin: Recognized` (or `Lyrics` in song mode), carrying per-word timings (TX-039) when Keep word timing is on.
- **Re-sync captions** rebuilds the recognized, unedited segments in a scope from the transcripts after the speech clips were moved or trimmed. This covers TX-038's case of source clips that move after recognition.

**Align known text mode (captions slice CS3).** The user already has the words, e.g. song lyrics (TX-044's "align pasted lyrics" path) or a script, and wants them timed against the audio.
- *What the user sees:* a text box (paste, or **Load text file…** for TXT), a **Text kind** choice (**Lyrics** / Script), and the audio source: the selected music or speech clip (default) or the clips in scope. One caption is made per line of the pasted text (TX-030's line splitting). **Align** shows a report of lines that could not be placed.
- *Inputs / controls (FreeCut defaults):* Text kind: **Lyrics** when the source clip's audio category (`ClipAudio.origin`, audio.md) is music, otherwise **Script**. Isolate vocals first: **on** for Lyrics (AI-023), off for Script. Language: taken from the text when it can be detected, otherwise the dialog's language. Keep word timing: **on**.
- *Behavior / output:*
  1. The source audio is recognized with the local Whisper model and its word timestamps (AI-009; an existing transcript is reused). Only Whisper weights under the **MIT** license are used for this mode.
  2. The known words and the recognized words are normalized (lowercase, punctuation and accents folded) and matched with a global sequence alignment (edit distance over word tokens), so skipped or misheard words don't derail the rest.
  3. Matched words take the recognized timings. Unmatched known words between two matched anchors share the time between them in proportion to their character counts.
  4. Each line becomes one caption draft spanning its first to its last word, with word timings. Drafts pass through TX-037 re-flow only if a line breaks the track's rules (the user's line breaks are kept otherwise).
  5. The drafts land through TX-038's `ReplaceCaptions` with `origin: Lyrics` or `origin: Script`, per Text kind.
- *Edge cases:* lines that cannot be placed (wrong song, skipped verse) follow TX-044's rule: they are listed, placed after the last aligned line at 2 s each, and flagged. Repeated choruses are matched in order. Text with no matching speech at all: "Text doesn't match the audio", no change. Very long texts are aligned in windows anchored on confident matches. Alignment models with non-commercial weights (e.g. CC-BY-NC alignment checkpoints) are never used.
- *Undo:* one `ReplaceCaptions` op per run, as below.

**Edge cases:**
- No speech found in scope: message, no change.
- Reversed clips: skipped with a warning, because the words would run backwards. Speed-changed clips: segment durations scale with the time map.
- Replace mode never replaces captions the user created or edited by hand. Only untouched recognized segments are replaced.
- Segments past the end of the timeline are clipped to the end.
- RTL and complex scripts: shaping and line breaking are TX's.
- Some clips fail (e.g. model missing, media offline): captions are added for the clips that succeeded, and the failures are listed in a report.
- The timeline is edited while the job runs: handled as TX-038 describes (Insert anyway / Discard).

**Undo:** One entry per run, "Auto captions": TX-038's `ReplaceCaptions { track, created_track, removed, inserted }`, plus the `AttachTranscript` changes of any transcriptions the run made (AI-009). Re-sync and Align known text are one entry of the same kind.

**Model notes:**
- `freecut-analysis::captions::words_for_clip(transcript, used_range, time_map) -> Vec<TimedWord>` is a pure function and can be unit-tested. Packing reuses TX-037's pure `reflow()`.
- `freecut-analysis::captions::align_known_text(known: &[Line], recognized: &[Word]) -> AlignResult { drafts: Vec<CaptionDraft>, unplaced: Vec<usize> }` is pure and unit-tested with fixture word lists (skipped verse, repeated chorus, misheard words). `CaptionDraft` is TX-038's plain-data type.
- Caption segments, caption tracks, `CaptionRules`, word timings and SRT import/export belong to text-and-captions.md.

### AI-011 Transcript-based editing — later

**What the user sees:** A dockable **Transcript** panel shows the speech of the timeline as text, in timeline order.
- The source is the clips on a chosen track. The default is the first track whose clips have transcripts.
- Clip boundaries show as thin dividers labeled with the clip name. Pauses of 1 s or more show as gap chips, e.g. "[1.4 s]". Low-confidence words have a dotted underline.
- Clicking a word moves the playhead there. During playback, the current word is highlighted. Selecting text highlights the matching range on the timeline.
- **Delete** removes the selected words from the video.
- With **Show deleted** on (default), deleted text stays visible with strike-through and can be brought back with **Restore**.
- A search box offers next/previous and **Delete all matches**.
- Double-clicking a word lets the user correct its spelling. This changes text only, not timing or video.
- **Copy text** and **Export transcript** (TXT; SRT through text-and-captions.md) are available.

**Inputs / controls:**
- Source track: dropdown.
- Pause display threshold: 0.3–5 s. Default: **1 s**.
- Cut padding: 0–200 ms. Default: **40 ms** (FreeCut default), to avoid clipping sounds.
- Ripple: **on** (follows timeline-editing.md).

**Behavior / output:**
- Selected words become source ranges: from the start of the first word minus padding to the end of the last word plus padding. The ends are clamped to the midpoint of the gap to the neighboring kept word, and snapped outward to frames.
- Selecting a gap chip includes that pause.
- Ranges are mapped to timeline time and removed from the clip and its linked clips as ripple deletions.
- **Restore** puts a deleted range back. If the neighboring pieces are still adjacent cuts from the same source, it rejoins them. Otherwise it inserts a new clip segment of the missing range at its original position in the sequence.

**Edge cases:**
- Clips reordered outside the panel: the panel is a live view built from the clips and their transcripts, so it shows the new order.
- B-roll on upper tracks over the speech: it isn't cut unless timeline-editing.md ripples all tracks.
- A selection that spans a clip boundary: both clips are cut.
- Deleting every word of a clip removes only the word ranges. Leading and trailing silence stay unless the gap chips are selected too.
- Clips without a transcript show an inline "Transcribe" placeholder.
- Locked source track: the panel is read-only.
- A text correction that adds or removes words: word timings are shared proportionally across the original word's span.
- Auto captions made from the transcript are refreshed only by **Re-sync captions** (AI-010), never silently.

**Undo:**
- Each delete or restore: one op (ripple-delete or insert/rejoin).
- Each text correction: an undoable edit of transcript data.

**Model notes:**
- `TranscriptView` (a derived structure: clip order + mapped words) is built in `freecut-app`.
- Edits use the range-delete, insert and join ops from `freecut-timeline`.
- Corrections write `Word.text` and set `edited = true` in the project's `Transcript`.

### AI-012 Filler-word, pause & repeat cleanup — later

**What the user sees:** A **Clean up** button in the Transcript panel opens a review sheet with these groups and their counts:
- Filler words.
- Long pauses.
- Repeated words (stutters such as "I I think").

Each found item is highlighted in the transcript and listed with a checkbox. **Remove selected** or **Remove all** applies the cuts; **Remove** on a single row removes just that item. The sheet also has:
- A show/hide switch and a count for each group.
- **Previous** / **Next**, which step through the flagged items. The playhead and the transcript follow.
- Unchecking, for false positives.
- **Mark for removal** on any words selected in the transcript, for items the detector missed.

**Inputs / controls (FreeCut defaults):**
- Filler list per language, which the user can edit. The English default enables um, uh, er, ah, hmm, mm. Ambiguous entries ("like", "you know", "I mean", "so") are included but **unchecked**. English ships first, and other languages get a list as contributors add one. The reference editor's version supports English speech only.
- Long pause threshold: 0.1–5 s. Default: **0.8 s**. The lower bound matches the reference editor, which refuses anything below 0.1 s.
- Shorten pauses to: 0–1 s. Default: **0.3 s**. Pauses are shortened, not removed entirely, so speech doesn't run together.
- Repeated words: **off** by default.

**Behavior / output:**
- Fillers are matched on normalized word text: lowercase, punctuation stripped.
- Pause shortening deletes the middle of the pause and leaves half of the target length at each side.
- Removals use the same range pipeline as AI-011. Adjacent removals merge into one cut, and cuts shorter than one frame are skipped.
- Speech recognizers tend to leave out disfluencies. In cleanup mode, FreeCut transcribes in a verbatim style: a word hint containing common fillers biases the recognizer toward writing them down. The UI notes that detection is approximate.
- Running detection again keeps the user's hand marks and unchecked items (FreeCut default). **Start over** discards them after a confirmation. The reference editor replaces hand-picked marks on a re-run, with a warning.
- The list is a live view built from the transcript and the clips. After timeline edits, items whose words are no longer on the timeline drop out, and the rest stay valid. The reference editor instead treats the whole list as out of date once the timeline is edited.
- Removal is all-or-nothing, as in AI-003. If one cut fails validation, nothing changes, and the failing item is named. The reference editor removes what it can and leaves the failed items flagged.

**Edge cases:**
- Selected media without speech, or no transcript yet: **Clean up** is disabled, with a tooltip that offers **Transcribe…** (AI-009). Every AI-009 failure state applies.
- Nothing found: each empty group says so, and the sheet suggests trying a lower pause threshold or other media.
- Caption tools (bilingual captions, find and replace, caption shortcuts) stay available while the sheet is open. The reference editor disables them in its removal mode.
- Languages without a filler list: only the pause and repeat groups are available.
- A filler at the edge of a clip: the in/out point is trimmed instead of split.
- A filler that carries meaning (e.g. "like" as a verb): unchecked by default, so the user decides.
- Low-confidence matches are listed but unchecked.
- If recognized captions already carry filler tags (TX-038's Mark filler words, or AI-010), the tags are reused.

**Undo:** One compound op, "Clean up: N fillers, M pauses, K repeats". Removing a single row is its own op. Marking, unchecking and stepping through items are review state (n/a) until something is removed.

**Model notes:** `freecut-analysis::cleanup::{find_fillers, find_pauses, find_repeats}` are pure functions over `Transcript`. The filler lists are FreeCut's own data files, one per language, versioned in the repo.
- *Lexicon format (captions build, 2026-10-13):* one JSON file per language in `freecut-analysis/src/fillers/data/<bcp47>.json` with a version, the language, a FreeCut-written **verbatim prompt** (a short sentence full of hesitations that nudges the recognizer to write them down), the entries (`text`, `kind`: hesitation or discourse, `checked`), and a short list of phrases the recognizer tends to invent over silence. Matching is on normalized words (NFKC, case-folded, punctuation stripped), longest phrase first; scripts written without spaces match on the token sequence. A test validates every shipped file. English ships with the captions build (slice CS3); other languages arrive as reviewed data files. Auto captions (AI-010) and this sheet share the same lexicons and the same `find_fillers`.

### AI-013 Caption translation & bilingual captions — later

**What the user sees:** **Translate captions…** (TX-041) and the translate option in the auto-captions dialog (AI-010) run this job. Where the translation lands (a new caption track, the secondary line of the same track per TX-036, or replacing the text) and how it is styled are defined in TX-041; this feature covers the engine side, including the **context window** and the **split-back rule**. The dialog lists only target languages that have an installed translation model and offers to install a missing pair through the model manager (AI-002). Progress shows as an AI-001 "Translate" row in PM-073's task list.

**Inputs / controls (FreeCut defaults):**
- Source language: **taken from the caption track or the transcript**, or chosen by hand.
- Target language: from the installed models.
- Engine: **local translation model** (default) / Whisper's built-in translate-to-English. The second option is offered only when the target is English and the source audio is available, because it re-runs recognition (AI-009).
- Line rules: TX-037's rules of the receiving track.
- Context window: up to **5 segments or 400 characters** per window, whichever comes first (FreeCut default). Fixed in this version.

**Behavior / output:**
- **Context window** (owned here): consecutive segments are translated together so the model sees whole sentences. A window never crosses a gap of more than 2 s between segments or a change of speaker (TX-042), and a segment is never split across two windows. Timing never changes.
- **Split-back rule** (owned here): inside a window, each source segment is wrapped in a numbered boundary token that translation models pass through unchanged. If every token comes back, in order, the text is split at the tokens. Otherwise FreeCut falls back to splitting the translated window in proportion to the source segments' character counts, moving each cut to the nearest sentence end, then the nearest word boundary (or grapheme boundary for scripts without spaces). A window that falls back is translated again segment by segment only if a part would come out empty.
- Each translated segment keeps the range of its source segment, so the result can be handed to TX-041 one segment at a time.
- When no direct model exists for the language pair, translation goes through English with two models, and the dialog notes that quality may drop.
- Engines: Argos Translate (MIT) with OPUS-MT-family models (CC-BY-4.0, attribution shown in About), or Bergamot / Firefox Translations models. Each model's license is checked before it enters the manifest.
- *Engine order (captions build, 2026-10-13; architecture C51):* the second line (TX-036) does not wait for a local translation model. Available engines, in the order they ship: typing by hand; attaching a subtitle file; **Whisper's Translate to English** (no extra model, English targets only, re-recognizes the audio and attaches by TX-036's overlap rule); the user's **assistant provider** (AS-002), opt-in and only offered when one is configured: only caption text is sent, in this feature's context windows with the boundary tokens, after AS-004's disclosure for that run, and the reply goes through the same split-back rule; and the **local translation models** above, which become the default engine once AI-002's ONNX runtime exists. How the local models run (ONNX exports through `ort`, or the CTranslate2 format that Argos Translate packages) is decided when this feature is built; either way no Python runs inside FreeCut.
- The translated text is handed to TX-041's op.

**Edge cases:**
- Language pair not installed: AI-002 prompt.
- Pivot through English with one of the two models missing: both missing models are listed in a single install prompt.
- A translation longer than the line rules allow: TX-037's re-flow warnings; text is never cut.
- Re-translating after the user edited translations by hand: TX-041 asks before overwriting.
- Names and terms: corrected by hand afterwards; no glossary support in this version.
- Right-to-left target languages: layout is TX's.
- A single segment longer than the 400-character window: translated alone, in one window.
- A window whose boundary tokens are dropped or reordered by the model: the proportional fallback applies, and the affected segments are flagged for review in the Captions panel.

**Undo:** One op per run, as TX-041 defines: `ReplaceCaptions` (TX-038) for a new track, or `TranslateCaptions { track, field, before, after }` for bilingual and replace output.

**Model notes:** `freecut-analysis::translate` (cargo feature `ml-mt`) takes text in and returns text out; apart from loading the model it does no I/O. `plan_windows(segments, limits) -> Vec<Window>` and `split_back(window, translated) -> Vec<String>` are pure functions with unit tests (tokens preserved, tokens lost, CJK text). Caption types belong to text-and-captions.md.

### AI-014 Background removal (automatic cutout) — later

**What the user sees:** Inspector → Video → a **Remove background** switch on video and image clips. The same command is in the clip's right-click menu.
- While the analysis runs, the switch and the clip's strip show AI-001's states and percent. On failure the switch offers **Retry**.
- After the first analysis pass (progress shown on the clip), the preview shows the subject on a transparent background, with the tracks below or the canvas background showing through.
- Mode: **Person** (default; fast, built for video) / Any subject (best for products, pets and objects; slower).
- Edge controls are listed under Inputs.
- **Invert** keeps the background and removes the subject.
- A link opens **Refine** (AI-015).

**Inputs / controls (FreeCut defaults):**
- Edge feather: 0–20 px at 1080p, scaled with resolution. Default: **1 px**.
- Edge shift (choke / spread): −10 to +10 px. Default: **0**.
- Edge cleanup: 0–100. Default: **0**. Removes small specks and fills pin-holes in the matte with a morphological open and close, using a radius from 0 to 6 px at 1080p.
- Temporal smoothing (reduces edge flicker): 0–100. Default: **30**.
- Remove color spill on edges: on/off. Default: **on**.
- Analysis resolution: **Auto** (long side up to 1920 px, matte upscaled with an edge-aware guided filter) / Full.
- Invert: off.

**Behavior / output:**
- A local matting or segmentation model makes an alpha matte for each source frame.
  - Person mode uses a recurrent video-matting model such as Robust Video Matting (GPL-3.0, compatible with FreeCut's GPL-3.0-or-later) or a portrait model such as MODNet (Apache-2.0).
  - Any-subject mode uses a salient-object model such as BiRefNet (MIT code).
- Mattes are stored in the **media cache (PM-074)** as 8-bit grayscale frames, losslessly compressed, at analysis resolution, under their AI-001 key. They are regenerable cache entries, not derived media items, so they never appear in the media panel's Generated bin (PM-041).
- The matte is applied at the keying stage (L4) of the frame pipeline in effects-transitions-animations.md (FX-001): after feather, shift and spill, it multiplies the clip's alpha together with any chroma or luma key (clip-transform-and-speed.md). User masks are applied later, at L7. Mattes are computed on the raw source frame and warped with the stabilization correction when stabilization is on (AI-007).
- Image clips get a single matte.
- Export uses the same cached mattes, at full analysis quality. If they were evicted, export waits while they are regenerated (AI-001), and the export progress names the analysis it is waiting for (for example the cutout, or the cutout and stabilization together). The reference editor can also postpone heavy cutout work to export time; in FreeCut this is simply AI-001's export wait.

**Edge cases:**
- Subjects other than people: Any subject mode handles them. The reference editor's automatic mode is tuned for people only.
- Outlines around the cut-out subject belong to OV-006: outline and shadow at FX-001's L8b, traced from the matte's alpha. The reference editor offers about a dozen outline looks (single, triple and other stacked lines, dashed, glow, hand-drawn, torn paper, zigzag, offset shadow with separate X and Y offsets, texture fill), with color, thickness, spacing and opacity. Which of these OV-006 adopts is OV's decision. This feature only guarantees a clean alpha edge.
- Replacing the removed background: put any image or video on a lower track, or use TR-011–TR-013's canvas fill. Generated backgrounds are out of scope.
- No subject found: an empty matte plus a "No subject detected" warning. The clip is not hidden silently; the switch shows a warning state.
- Several people in Person mode: all are kept. Picking one person is done in AI-015.
- Hair, motion blur, and transparent or reflective objects: soft or flickering edges. Temporal smoothing and Refine help.
- A source that already has alpha (PNG, ProRes 4444): the alphas multiply.
- Trimming past the analyzed range: incremental analysis. Recurrent models start 1 s earlier to warm up.
- Speed and reverse changes need no re-analysis, because mattes are indexed by source frame.
- Original media offline but a proxy is available: analysis runs on the proxy, and the matte is flagged "low-res". It is re-run automatically when the original comes back.
- Very high resolution (6K/8K): inference runs downscaled, and the matte is upscaled edge-aware.
- Cache cleared or the matte evicted (PM-074): the clip shows unmatted with a "re-analyzing" badge until the matte is rebuilt.

**Undo:** The switch and every parameter are undoable property edits. Analysis is not on the stack.

**Model notes:**
- `CutoutParams { enabled, mode, feather, shift, cleanup, temporal, despill, invert, analysis_res }` lives on the clip's video properties.
- `MatteSequenceRef { key: AnalysisKey }` points to the media-cache entry (PM-074). There is no `GeneratedKind::Matte`.
- Inference runs in `freecut-analysis` (`ml-onnx`). Matte encoding and decoding is in `freecut-media`. The matte, feather, choke and despill shader is in `freecut-render` (WGSL).
- The Person-mode matte sequence is also what FX-020's person-tracking effects read as `PersonMaskTrack`. Cut-out stickers in overlays-and-stickers.md (OV-023) use Any-subject mode on a single frame.

### AI-015 Cutout refinement & object selection — later

**What the user sees:** Inside Remove background (AI-014) or Remove object (AI-022), a **Refine** mode offers:
- **Keep brush**: paint areas that must stay.
- **Remove brush**: paint areas that must go.
- **Click select**: click to add the object under the cursor; Alt/Option-click removes.
- **Box select**.
- **Plain eraser**: a manual mask stroke with no AI, applied to the current frame or to a frame range.

The current selection is tinted in the preview. **Propagate** carries the corrections through the clip (forward / backward / both), with percent in the Refine bar, AI-001's states in the task list, and a notice when it finishes. Frames that have a user correction show a small diamond on a mini-timeline.

**Paint subject** starts a cutout from strokes alone, without the automatic pass. It switches Remove background on in **Selected object only** mode and opens Refine with the Keep brush. Otherwise Refine becomes available once AI-014's first pass has run.

**Inputs / controls:**
- Brush size: 5–200 px. Default: **40 px** (FreeCut default).
- Brush hardness: 0–100. Default: 80.
- Propagation direction: **Both** (default) / Forward / Backward.
- Mode: **Add to automatic result** (default; keep/remove strokes adjust the AI-014 matte) / **Selected object only** (the clicked object alone becomes the matte, e.g. to keep one person out of a group).

**Behavior / output:**
- User strokes, clicks and boxes become prompts for a promptable video segmentation model, SAM 2 (Apache-2.0 code and checkpoints; tiny, small, base+ and large sizes). The model spreads the masks through the clip.
- The final matte is (base matte ∪ keep) − remove in Add mode, or the propagated object mask in Selected-object mode.
- Strokes are stored in the project as resolution-independent vector paths per corrected frame, in raw source coordinates (AI-007). Propagated masks are regenerable and live in the media cache (PM-074) under their AI-001 key; the key's parameter hash includes the prompts.

**Edge cases:**
- The object leaves the frame and comes back: the model may re-acquire it. If not, the user adds a correction on the frame where it reappears.
- A correction on frame N overrides propagation from N up to the next correction frame.
- Strokes on frames that are later trimmed away: kept but inactive.
- A stroke is undone while propagation is running: the job is cancelled and restarted with the remaining prompts.
- Combined with a chroma or luma key (TR-022, TR-025): the alphas multiply, so neither is removed. The reference editor cancels a custom cutout, after a confirmation, when another matte feature is applied.
- Very large brushes on 4K sources: the stroke is rasterized at analysis resolution.

**Undo:** Each stroke, click, box or erase is one undoable edit of the refinement data. Propagation is analysis and is not on the stack.

**Model notes:**
- `RefinePrompts { frames: BTreeMap<SourceFrame, Vec<Prompt>> }` is stored in the project.
- SAM 2 inference runs in `freecut-analysis`. Small/tiny model sizes are preferred for interactive use.

### AI-016 Auto reframe — later

**What the user sees:** There are two entry points:
1. **Per clip:** Inspector → **Auto reframe**, with the target aspect set to the project canvas.
2. **Project-wide:** when the user changes the canvas aspect ratio (project settings, owned by project-and-media.md), a dialog offers **Auto reframe all clips**.
3. **Standalone:** right-click a video in the media library → **Reframe into a new project…**. The dialog asks for the target ratio (9:16, 1:1, 4:5 or 16:9; FreeCut list), creates a new project at that ratio (PM-002), places the one video, runs Auto reframe and opens the project, ready for export. It takes one video per run. Files the importer rejects (PM-037) never reach it.

Progress follows AI-001's states on the clip. Afterwards each clip has position/scale keyframes that keep the subject in frame. The keyframes are visible and editable like any other keyframes. **Re-run** recomputes them.

**Adjust framing:** with playback paused, the preview shows the planned crop as a rectangle. Dragging it adds a **pin** at that frame. Pins are hard constraints: the path is re-planned through them, and **Re-run** keeps them. A pin can be dragged again or deleted.

**Inputs / controls (FreeCut defaults):**
- Subject: **Auto** (most prominent person or face) / Faces / People / Custom box (drawn on a frame).
- Camera motion: Calm / **Normal** / Fast. These set how quickly the virtual camera follows: a smoothing cutoff of 0.3 / 0.8 / 2 Hz and a dead zone of 8 / 5 / 2 % of the crop width.
- Prefer a still frame: **on**. If the subject stays inside a fixed crop for the whole shot, a single static crop is used. Off gives the freer, always-following framing.
- Stabilize first: **off** / on. A shortcut that switches on TR-026 at Balanced. The two compose (AI-007).
- Headroom / padding: 0–30 %. Default: **10 %**.
- Fit: Fill (crop). Fixed in this version.

**Behavior / output:**
- Frames are sampled (every 2nd frame, at most 640 px wide). Faces and people are detected, and detections are linked into tracks.
- The subject track is chosen: the largest and longest-lived, or the one nearest the custom box.
- A target crop center is computed per frame, with headroom above faces. The path is smoothed with a dead zone and a critically damped follower, and resets at shot cuts (AI-004) so the camera never pans across a cut.
- The path is simplified to as few keyframes as possible: about 1 % position tolerance, ease interpolation.
- Google's AutoFlip (part of MediaPipe, Apache-2.0) is a public reference design. Detectors: BlazeFace / MediaPipe face detection (Apache-2.0) and the MediaPipe object detector with an EfficientDet-Lite model for people (Apache-2.0; model card to verify). YOLO builds under AGPL are avoided.

**Edge cases:**
- No subject found: a centered static crop and a notice.
- Two subjects far apart: both are framed if they fit inside the crop; otherwise the dominant one.
- Target aspect equal to the source aspect: disabled.
- Source smaller than the target crop at canvas resolution: a quality warning about upscaling.
- The clip already has user keyframes on position or scale: confirmation, then they are replaced. Keyframes on other properties are untouched.
- Image clips: a static, face-aware crop.
- Letterboxed sources: black bars are detected and ignored.
- Clips with a transition between them: each clip is reframed separately, and the transition blends the results.
- Speed changes: keyframes are in clip-local time (clip-transform-and-speed.md), so they stay correct.
- Other edits on the clip: stabilization, crop, cutouts and tracking all stay, because they compose (AI-007). Only AI-001's motion-writer rule applies, towards AI-017 and AI-030. The reference editor removes up to three conflicting edits when reframing (it lists them in its confirmation), and removes the reframe when stabilization or tracking is applied afterwards.
- Export: the keyframes are part of the clip, so export uses them as they are. If detection is still running, export waits (AI-001). The reference editor blocks export until reframing finishes, and lets the user export at either the reframed or the original ratio. In FreeCut, export always renders the canvas, and other output ratios are EX-042's presets plus a **Re-run** at the new ratio.
- A pin outside the clip's current trim is kept but inactive, like TL-037's trimmed markers.

**Undo:** Per clip: one op that writes the keyframes. Project-wide: one compound op. Adding, moving or deleting a pin: one op each, which also rewrites the keyframes. Standalone mode: the new project starts with its own history, and its first entry is the reframe.

**Model notes:**
- `freecut-analysis::reframe::{detect_subjects, choose_track, plan_path}` returns a list of keyframes in clip-local rational time. `plan_path` takes the pins as hard constraints and is unit-tested for "passes through every pin".
- `ReframeParams { subject, motion, prefer_still, headroom, pins: Vec<(SourceFrame, Rect)> }` is stored with the clip's `SmartMotion` record (AI-001), in source coordinates (AI-007).
- Writing keyframes uses the keyframe ops owned by clip-transform-and-speed.md.
- Face and person detection run through `ml-onnx`.

### AI-017 Subject follow (automatic pan & zoom) — later

**What the user sees:** Inspector → **Follow subject** keeps a chosen face, body or custom region near the center of the existing canvas by slowly panning and zooming the clip, like a camera operator. It is the same engine as auto reframe, without changing the aspect ratio. The user clicks a person in the preview to choose the subject. Clicking another person later switches the subject from the playhead on. **Clear** removes the follow.

**Inputs / controls:**
- Framing: **Face** (default) / Upper body / Full body / Face and body together / Custom box. Hands-only framing is not offered, because no hand model is in the manifest yet.
- Zoom: 100–300 %. Default: **150 %**.
- Keep subject size: **off** (fixed zoom) / on. When on, the zoom changes over time so the subject keeps the size it has at the reference frame, within the zoom range.
- Camera motion: Calm / **Normal** / Fast.
- Keep subject at: center (default) / rule-of-thirds left / right.

**Behavior / output:** Uses the AI-016 pipeline with the target aspect equal to the canvas and a fixed zoom, and writes position/scale keyframes.

**Edge cases:**
- Subject lost for more than 1 s: the camera eases back to the full frame at 100 %.
- Zoom pushes past the source edges: the zoom is limited so no empty border shows.
- Multiple faces: the one with the longest track (or the custom box) is followed.
- With stabilization on: detections are mapped through TR-026's correction before the path is planned (AI-007), so the camera follows the subject as the user sees it.
- No person in the clip: the section says no person was found and offers Custom box.
- A subject switch at a frame that already has one replaces it.
- Stabilization is not switched on automatically. A hint suggests it for shaky handheld footage. The reference editor turns stabilization on by itself when its subject lock is applied.
- Follow is never applied automatically to newly imported clips; FreeCut doesn't edit on import. The reference editor offers this as an option.
- Other motion writers (AI-016, AI-030): AI-001's motion-writer rule applies.

**Undo:** One op that writes the keyframes ("Follow subject"). Each subject switch is one op ("Switch subject"). Clear removes them in one op ("Clear follow").

**Model notes:** It shares `freecut-analysis::reframe`. Only the planning parameters differ. Subject switches are stored as `Vec<(SourceFrame, SubjectRef)>` in the clip's `SmartMotion` record.

### AI-018 AI frame interpolation for smooth slow motion — later

**What the user sees:** The **Frame sampling** dropdown in the Speed tab (clip-transform-and-speed.md: Nearest frame and Frame blend in TR-041, classic Optical flow in TR-042) gains one more choice, **AI interpolation**. It only matters when the clip's effective frame rate (source fps × speed) is below the project frame rate; otherwise the control says "Not needed". A background job renders the in-between frames. Until it finishes, the preview uses frame blend, as TR-042 does, and the clip shows a progress strip.

**Inputs / controls (FreeCut defaults):**
- Mode: AI interpolation, as one value of TR's frame-sampling setting.
- Quality: **Standard** / High. In Standard, sources above 1440p use a half-resolution flow internally.
- Don't interpolate across shot cuts: **on**. Uses AI-004 detection inside the clip.
- **Smooth all slowed clips…** (Smart tools menu): sets AI interpolation (or Frame blend, chosen in the dialog) on every clip whose effective frame rate is below the project frame rate.
- Availability: the observed rule in the reference editor is "speed below 1×". FreeCut uses "effective frame rate below the project frame rate", which also covers a 24 fps clip at 1× in a 60 fps project.

**Behavior / output:**
- For each project frame, the source time comes from the clip's time map (TR), including speed curves.
- If that time falls between source frames i and i+1 at fraction *a*, the model synthesizes the frame at *a*: a RIFE-family model (MIT code and models) or FILM (Apache-2.0).
- If *a* is within 0.02 of a real frame, that real frame is used instead (FreeCut default).
- Synthesized frames are deterministic and regenerable, so they are stored per clip at the project frame rate in the **media cache (PM-074)** under their AI-001 key. Export uses the cache and waits for regeneration when entries are missing or were evicted.
- Frames are synthesized at the project frame rate. When the export frame rate (EX-026) is higher, a hint suggests raising the project frame rate (PM-005). It never changes on its own, and the user is asked first because the change affects every clip.
- Progress, queueing and ETA follow AI-001's states. Long clips keep their progress visible in the task list.

**Edge cases:**
- Speed at or above 1× with matching frame rates: nothing to do.
- Very slow speeds (e.g. 0.1×) on low-fps sources: many synthesized frames per real frame, which shows as warping. The UI suggests high-fps sources.
- Occlusions, fast or complex motion, and thin objects can smear. Frame blend (TR-041) is the safer fallback.
- Changing speed, the speed curve, trim or project frame rate invalidates only the affected part of the cache.
- Variable-frame-rate sources: fractions come from real timestamps.
- Reversed clips: same pipeline, with the frame order mirrored.
- Shot cuts inside the clip: no interpolation across a cut; the nearest real frame is used.
- Sources below 10 fps: a warning that motion between frames is too large for good results. The reference editor refuses sources whose frame rate is too low.
- Re-choosing the quality, or turning the mode off: the earlier frames stay in the cache under their own key, so switching back is instant and no confirmation is needed.
- GPU unavailable or failing: inference runs on the CPU with AI-001's notice, and Frame blend is suggested as the fast alternative.
- Model not installed: the choice stays visible and opens the AI-002 install prompt. A clip that already uses AI interpolation (e.g. a project from another machine) previews and exports with frame blend until the model is installed, with a badge.
- A project saved with AI interpolation opened in a FreeCut build that doesn't know the variant: TR-041's rule loads it as Frame blend.

**Undo:** Choosing the mode is TR's `SetFrameSampling { clip, old, new }`. **Smooth all slowed clips** is one compound op of those. Synthesis is not on the undo stack.

**Model notes:**
- TR-041's `FrameSampling` enum reserves the variant `Ai { quality }`; this feature defines its behaviour. There is no separate type. An unknown variant loads as Frame blend (TR-041).
- Inference runs in `freecut-analysis` (`ml-onnx`); `freecut-media` encodes and decodes the cached frames. Some RIFE versions only support fixed 2×/4× steps, so arbitrary-fraction support must be checked for the chosen model version.

### AI-019 Upscale & enhance quality — later

**What the user sees:** Inspector → Video → **Enhance** on video and image clips offers:
- Upscale: ×2 / ×4, or a target (HD 1080p / 2K 1440p / UHD 2160p).
- Detail: Natural / Strong.
- Content: Live action / Animation.
- Denoise strength.

**Create enhanced version** renders in the background, with AI-001's states and percent, and can be cancelled. When it finishes, the clip switches to the enhanced version, and a **Use enhanced version** switch lets the user compare. In the media library, the derived file appears under the original.

**Enhance low-resolution clips…** (Smart tools menu) lists every clip whose source is scaled above 150 % at the export resolution (FreeCut threshold), with a checkbox each and **Select all**. Clips that already have a large enough enhanced version are listed as done. **Enhance** queues one job per media item, at the smallest target that covers all of that item's uses. Export preflight can offer the same list as an action on its low-resolution warning (EX-036). The reference editor offers this as a switch at export time, with queue waits; FreeCut runs it before export, so the result can be previewed and compared.

**Inputs / controls (FreeCut defaults):**
- Target: **UHD** if the source is ≤ 1080p, otherwise ×2. A target that isn't larger than the source is disabled, so a clip that is already UHD offers only ×2.
- Detail: **Natural** (light restoration that keeps the original texture) / Strong (recovers more fine detail and can look processed). Each maps to a model variant in the manifest.
- Content: **Live action**.
- Denoise strength (AI): 0–100. Default: **50**.
- Range: **clip's used range + 1 s handles** (default) / whole media.
- Before starting, the dialog shows estimated time and disk use.

**Behavior / output:**
- Frames are decoded and run through a super-resolution model: Real-ESRGAN-family (BSD-3-Clause code; general and anime-video models; the general model has a denoise-strength control). Large frames are processed in 256 px tiles with 16 px overlap.
- The result is encoded with `freecut-media` as a visually lossless intermediate, written to the **generated-media store (PM-041)** and registered as a derived media item (`GeneratedKind::Upscaled`). It is not a cache entry, so cache cleanup never removes it while a clip points at it. The clip's media reference is swapped, keeping the original in/out points mapped onto the derived range.
- Audio is not touched.
- Image clips run the model on one frame. The result is a PNG in the generated-media store and can be at most 16,384 px on its long side (FreeCut default).
- A classic, live video noise-reduction filter is not part of this feature. It is CL-033 (Denoise), and flicker removal is CL-034 (Deflicker).

**Edge cases:**
- Clip extended past the enhanced range: that part falls back to normal scaling, and a "Partially enhanced — extend?" badge appears.
- Long jobs, or jobs with a 4K or larger target: the time and disk estimate is shown before start. The job can be cancelled, and the partial file is deleted.
- Length and size limits: there is no length cap. For media over 30 min, the estimate must be confirmed before the job starts. If the estimated disk use exceeds the free space minus 1 GB, the job is refused before it starts. The reference editor refuses clips over 30 min, video that is already 4K and stills above about 6K, and needs a network connection.
- Effects added later never remove the enhancement. It is a source substitute at L1, and effects run on top of it. The reference editor removes the enhancement when certain effects are applied. When object removal or colorize is also used, AI-001's substitute chain applies.
- GPU memory pressure: smaller tiles, then the CPU.
- Per-frame enhancement can flicker over time. This is a known limitation, and a light temporal blend option is planned.
- Interlaced sources: deinterlaced first.
- HDR sources: disabled in this version, because the models expect SDR.
- Sources with alpha: alpha is upscaled separately with bicubic.
- The derived file is deleted from disk: the clip falls back to the original, with a badge, and can be rebuilt.

**Undo:** Switching the clip to the enhanced media (or back) is an undoable op. In the batch, each media item's switch lands as one op when its job finishes, covering every clip that uses that item. Deleting derived files is a media-library action (project-and-media.md).

**Model notes:**
- `EnhanceParams` live on the clip. The derived item is PM-041's `MediaOrigin::Generated { kind: Upscaled, parent, source_range, recipe }`, with `EnhanceParams` and the model id and version in `recipe` so the relink dialog can offer **Regenerate**. ("Derived media" in this spec always means that PM-041 type.)
- Inference runs in `freecut-analysis` (`ml-onnx`). Encoding is in `freecut-media`.

### AI-020 Face retouch — later

**What the user sees:** Inspector → **Retouch** on video and image clips. Faces are detected automatically (after AI-002's one-time face-analysis notice). With several faces, **Apply to** offers All faces (default) or Selected face (click a face in the preview; it stays selected through the clip). When a skin setting applies to every face, the section says how many faces it affects. All sliders start at 0, meaning no change. **Reset** asks first, then clears the sliders and the brush strokes. Nothing is ever switched on automatically. Presets (AI-032) and color tints (AI-033) sit in the same section.

**Where:** **Faces** (default) or **Brush**. In Brush mode the user paints the areas to smooth with a brush and an eraser. This covers skin outside faces, such as the neck or arms, or a single spot. Strokes inside a face are stored relative to its landmarks so they follow the face. Strokes elsewhere stay fixed in source coordinates. The brush paints only inside the video frame.

**Inputs / controls (FreeCut defaults; all 0 = off):**
- **Skin** (0–100): Smooth, Even out tone, Clear blemishes. **Skin brightness** (−100 to +100): negative values darken or tan, positive values brighten.
- **Features** (0–100): Whiten teeth, Brighten eyes, Reduce under-eye shadows.
- **Shape** (−100 to +100), grouped by region:
  - Face: Width, Slim cheeks, Jaw, Chin length, Forehead height, Head size.
  - Eyes: Size, Height, Spacing, Corner tilt.
  - Brows: Height, Spacing, Arch, Tilt.
  - Nose: Width, Length, Bridge.
  - Mouth: Width, Height, Smile (corner lift).
- **Brush** (Brush mode): size 5–200 px (default **40**), intensity 0–100 (default **50**); eraser of the same size.
- Faces processed per frame: up to **10**, largest first (FreeCut default).

**Behavior / output:**
- A face-landmark model finds the face points: MediaPipe Face Landmarker, 478 3-D points per face. Its model-card license must be checked; the API code is Apache-2.0.
- Landmarks are cached per source frame and smoothed over time with an adaptive low-pass filter to remove jitter. Faces are linked across frames by overlap so a selected face stays selected.
- Masks for skin, teeth and eyes come from landmark polygons, with soft edges.
- Smoothing is an edge-preserving filter blended by strength. Whitening lowers yellow saturation and raises lightness inside the teeth mask.
- Clear blemishes is classic, with no model. Small dark or red spots (2–12 px at 1080p) are found inside the skin mask by a difference-of-Gaussians detector and healed from the surrounding skin.
- Brush strokes are vector paths, like AI-015's. Their smoothing uses the same filter as Smooth, at the brush intensity.
- Shape changes are a smooth mesh warp driven by the landmarks, limited to a radius around the face so the background bends as little as possible.
- All of it is rendered in `freecut-render` (WGSL).

**Edge cases:**
- Profile views, occlusion (hands, hair), or faces smaller than 64 px: landmarks are dropped and the effect fades out over 3 frames instead of popping.
- A face entering or leaving the frame: same fade.
- Faces crossing paths: identities may swap. The user re-selects on the frame where it happens; the selection is stored per frame range.
- Strong shape values near the frame edge: the warp is clamped to keep edge pixels in place.
- More than 10 faces in a frame: the 10 largest are retouched, and a notice gives the count. The reference editor refuses frames above its face limit.
- No face detected: the face controls are disabled with a notice, and Brush mode still works.
- Skin-tone protection is CL-030's, which is hue-based. Retouch's skin mask already keeps eyes, brows and lips out of smoothing.
- Face shape changes only affect the area inside the detected face outline plus the warp radius. The reference editor also limits face adjustments to the detected face.
- Place in the frame pipeline: retouch is FX-001's **L2b Retouch** stage. It runs on raw source pixels, after L2 Normalize and before stabilization and crop (L3), so keying (L4) and everything later see the retouched face. Analyses (including this feature's own landmarks) read the L2 output, before L2b (AI-007).
- Strong Face width or Jaw values with background removal on: the matte was computed on the unretouched frame, so a face edge moved by more than a few pixels can show a thin seam. The shape warp is therefore limited to the inside of the face outline by default, and a hint appears when both are on.
- Image clips: one landmark pass.

**Undo:** Every slider change and face selection is an undoable property edit. Each brush or eraser stroke is one op. Reset is one compound op.

**Model notes:**
- `RetouchParams { target: AllFaces | FaceTrack(id), skin, features, shape, brush: Vec<BrushStroke>, colors: MakeupParams /* AI-033 */ }` lives on the clip. A `BrushStroke` is anchored either to a face track's landmark mesh or to source coordinates.
- Landmark tracks extend AI-021's face tracks and are stored as an `AnalysisRecord` in the media cache (PM-074). The selected face and its per-range re-selections are stored in `RetouchParams`, in the project.
- Inference runs in `freecut-analysis` (`ml-onnx`).

### AI-021 Automatic face blur (privacy) — later

**What the user sees:** Right-click a clip → **Blur faces…**. FreeCut finds the faces in the clip (percent shown) and lists each tracked face as a thumbnail with its screen time. While the dialog is open, the preview also draws a box around each detected face, and clicking a box toggles that face, the same as its checkbox. All are checked by default; the user unchecks the faces that should stay visible. **Apply** adds the face privacy effect from effects-transitions-animations.md (FX-021, blur or pixelate) to the clip, limited to the checked faces. Faces the detector missed can be covered by hand with a tracked region (AI-008) or a manual mask (clip-transform-and-speed.md).

**Inputs / controls (FreeCut defaults):**
- One checkbox per face track: all **checked**.
- **Same person**: select two thumbnails to join them into one face track, e.g. after the face left the frame and came back.
- Cover: **Blur or pixelate** (default) / Shape or sticker.
  - Blur or pixelate: the look, intensity, padding and feather are FX-021's controls and defaults. They are not redefined here.
  - Shape or sticker: a solid circle, a heart or any other OV-024 shape, or an OV-018 sticker, placed over each checked face and bound to its face track with AI-008's binding (`Face(id)` source). Size: 50–300 % of the face box, default **130 %**. Opacity: 0–100 %, default **100 %**.

**Behavior / output:**
- Faces are detected on raw source frames (AI-007) with a local detector (MediaPipe face detector, BlazeFace-class, Apache-2.0; or OpenCV Zoo YuNet, MIT) and linked into face tracks by box overlap plus a simple motion model.
- Short detection gaps are bridged by interpolation, up to FX-021's 10-frame limit, so a face is not exposed for a frame or two. The box grows with the face's speed so fast movement stays covered.
- The result is a set of face tracks, stored as an analysis record in the media cache (PM-074) under its AI-001 key. Track ids are deterministic for a given key, so a rebuilt record has the same ids. FX-021 reads the tracks, together with the checked track ids and the **Same person** joins, which are stored on the effect in the project.
- The same face tracks also serve face retouch (AI-020) and auto reframe / subject follow (AI-016, AI-017).

**Edge cases:**
- Missed faces (profiles, very small, heavily occluded): covered by hand. The dialog says that automatic detection is not a guarantee.
- False positives: uncheck them.
- Faces at the frame edge: the region is clamped to the frame.
- Two faces cross and their tracks swap: the user fixes it with Same person or by unchecking the wrong part.
- Trimming past the analyzed range: incremental detection. New face tracks found there are added **checked** (the privacy-safe choice), and a notice says so.
- No faces found: a notice, and nothing is added.
- No caps: FreeCut covers every checked face in every frame, whatever the count or the media length. Long media is chunked, and a notice appears when there are more than 30 face tracks, because the list gets long. The reference editor covers at most 5 faces per frame and refuses videos with more than 10 faces or longer than 60 min, which would leave faces exposed.
- Leaving the dialog during detection keeps the finished chunks (AI-001), and reopening resumes.
- **Apply** needs at least one checked face. Unchecking every face on an existing effect offers **Remove blur**.
- Export must match the preview, so the cached tracks are always used, and export waits for missing or evicted analysis.

**Undo:** One op, "Blur N faces", that adds the FX-021 effect with its checked track ids. Changing the selection later is one parameter edit on that effect. With shape or sticker covers, one compound op adds the bound element clips, one per checked face.

**Model notes:** `FaceTrack { id, samples: Vec<FaceSample { src_time: RationalTime, bbox: Rect, landmarks: Option<LandmarkSetRef>, confidence: f32 }> }` is produced in `freecut-analysis` and stored as an `AnalysisRecord`. Spatial values are floats; time is rational. The effect type, its face-selection field (FX-021's "which faces" control, extended with a list of track ids) and its rendering belong to effects-transitions-animations.md.

### AI-022 Object removal (video inpainting) — later

**What the user sees:** Inspector → **Remove object**. The user paints over an unwanted object on the current frame with a brush and an eraser, or clicks it with a **Smart brush** that selects the object under the cursor (AI-015's click select). **Track object** spreads the mask through the clip. **Remove** fills the area with plausible background. The result renders in the background, with percent and time left in the task list, where the user can leave it while editing on. It then shows in the preview, and a switch compares before and after.

**Inputs / controls (FreeCut defaults):**
- Brush size: 5–200 px. Default: **40**.
- Mask expansion: 0–30 px. Default: **8**.
- Mode:
  - **Moving object** (default): the mask is tracked, and the fill borrows pixels from other frames.
  - **Fixed area**: one mask for the whole clip, e.g. a burned-in timestamp on the user's own footage.

**Behavior / output:**
- The mask is spread by SAM 2 (Apache-2.0).
- **Fill:**
  1. Motion-guided copying of background pixels from nearby frames where the area is visible (classic optical flow).
  2. Holes that remain are filled per frame by LaMa (Apache-2.0).
  3. A temporal blend reduces flicker.
- The filled frames for the clip range are encoded as a visually lossless intermediate and written, like AI-019's output, to the **generated-media store (PM-041)** as a derived media item (a `GeneratedKind` for inpainted video, proposed `Inpainted`). The fill is not bit-for-bit reproducible across GPUs and model versions, so it is never an evictable cache entry. The clip uses it as a non-destructive source substitution: the switch flips back to the original at any time.

**Edge cases:**
- Large objects (over 25 % of the frame) or objects in front of complex moving backgrounds: a quality warning.
- Shadows and reflections of the object stay unless they are painted too.
- Frames where the mask is empty: passed through unchanged.
- Trim, speed or reverse changes: handled in source frames. Extending the clip triggers incremental processing.
- The derived file is deleted from disk: the clip falls back to the original with a badge, and PM-041's relink dialog offers **Regenerate**.
- Clip length: at least 3 frames. Above 10 min, the time and disk estimate must be confirmed first (FreeCut defaults). The reference editor enforces both a minimum and a maximum length.
- The clip stays editable while the fill renders. A mask edit cancels the job and restarts it with the new mask. The reference editor locks the clip while it processes.
- Export during rendering waits for the fill (AI-001) instead of being blocked.
- Another GPU job is running: the fill is queued (AI-001), not refused.
- Stop: the partial derived file is deleted, after a confirmation when more than 1 min of rendering would be lost (AI-001).
- Re-running with a new mask or mode replaces the derived item, after a confirmation. Later steps of AI-001's substitute chain (colorize, enhance) are marked out of date.
- Describing the object in words, finding it automatically, and generating new content in its place (for example other clothing) are not offered (see Out of scope).
- Video inpainting models with non-commercial licenses (e.g. ProPainter, E2FGVI) are **excluded**.

**Undo:** Mask edits, mode changes and turning it on or off are undoable property edits. Rendering is not on the stack.

**Model notes:** `ObjectRemovalParams { prompts: RefinePrompts, expand, mode }` lives on the clip; masks are in raw source coordinates (AI-007). SAM 2 and LaMa run in `freecut-analysis` (`ml-onnx`), and the flow-guided copying and temporal blend are pure Rust in the same crate. The propagated masks are media-cache entries (PM-074); the filled video is PM-041 derived media, which `freecut-media` encodes and decodes.

### AI-023 Voice & stem separation engine — later

**What the user sees:** Both user commands belong to audio.md (AU-021): the in-place **Voice isolation** dropdown, and **Split into stems…**, which creates linked clips on new audio tracks (2 or 4 stems), mutes but never deletes the source audio, and is one undo step. Lyric captions (TX-044, AI-010's song and Align known text modes) and the silence-removal hint (AI-003) use the same engine to isolate vocals first. This feature owns only the **engine**: the model, the separation job, the quality choice, and the stem files. Around the job it adds the model install prompt (AI-002), an AI-001 "Separate voice" or "Split stems" row in PM-073's task list with progress and Cancel, and an estimate of time and disk use before long runs.

**Inputs / controls (FreeCut defaults):**
- Quality: **Standard** / High. Standard uses a two-stem (voice / rest) model. High, and every four-stem split, use the larger hybrid model.
- Stems requested: set by the caller (AU-021: voice and background, or vocals / drums / bass / other; TX-044 and AI-003: voice only).
- Range: the whole media file, once per audio stream, so every clip that uses the media (or a copy with the same PM-038 fingerprint) shares the result.

**Behavior / output:**
- `freecut-media` decodes the audio at 44.1 kHz stereo. A local source-separation model processes it in overlapping chunks joined by overlap-add: Demucs-family hybrid transformer models (MIT code; weight license checked per model) or Open-Unmix (MIT code).
- Inference runs in **`freecut-analysis`** (`ml-onnx`), on AI-001's analysis pool, orchestrated by `freecut-app`.
- Stems are written as lossless **FLAC** files to the **generated-media store (PM-041)** as derived media items (`GeneratedKind::Stem`), one file per stem per stream. They are not evictable cache entries, because AU-021's stem clips point at them and the model output is not bit-for-bit reproducible across hardware.
- AU's signal chain decodes a stem at **step 1** (in place of the source stream) whenever AU-021's isolation mode or a stem clip asks for it.

**Edge cases:**
- Mono sources: duplicated to stereo for the model and written back as mono.
- 5.1 and other multichannel sources: downmixed to stereo, with a warning.
- Very long media: chunked and cancellable; the time and disk estimate is shown first for media over 1 h (FreeCut default).
- Re-running with another quality: confirmation first; the new stems replace the old ones and every clip that uses them switches together.
- Stem file deleted: PM-041's relink dialog offers **Regenerate**, and the job re-runs on demand. Meanwhile AU-021 plays the unprocessed source with a warning.
- Model not installed: AU-021 decides whether its commands are hidden or shown; installing always goes through AI-002.
- Job cancelled: partial stem files are deleted; AU-021 decides what its command does next.

**Undo:** n/a for the engine. The edits are AU-021's (`SetIsolation` and its "Split into stems" compound op). Undo never deletes stem files; PM-074's **Clean up project…** offers to trash unused generated files.

**Model notes:** The stem item is PM-041's `MediaOrigin::Generated { kind: Stem, parent, source_range, recipe }`, with the stem name (voice, background, vocals, drums, bass, other), the quality and the model id and version in `recipe`. Inference is in `freecut-analysis`; `freecut-media` writes and decodes the FLAC files; `freecut-audio` only plays them.

### AI-024 Speech-enhancement model engine — later

**What the user sees:** The control belongs to audio.md: AU-020's **Voice clarity** toggle and strength slider (0–100, default 70), stored in `ClipAudio.clarity` and applied at step 3 of AU's signal chain from the processed-audio cache. Classic noise reduction (AU-018) stays model-free and does not use this engine. This feature owns only the **engine**: the DeepFilterNet model, the mapping from AU-020's strength to the model, the model install prompt (AI-002), and the cache keys. While a clip is processed it shows AI-001's progress strip, and a small "processed" badge once the preview plays the processed audio.

**Inputs / controls (FreeCut defaults):**
- Strength comes from AU-020. The engine maps it to the model's maximum attenuation: 0 = bypass, 1 → 6 dB, 99 → 60 dB (linear in dB in between), 100 → unlimited (FreeCut default mapping).
- Engine: **DeepFilterNet** (MIT/Apache-2.0 code, Rust core library, works at 48 kHz). Fixed in this version.

**Behavior / output:**
- `freecut-media` decodes the clip's source range (plus 1 s of handles). The audio is resampled to 48 kHz, enhanced in `freecut-analysis`, and resampled back to the source rate.
- The output is deterministic and regenerable, so it goes to AU's processed-audio cache in the **media cache (PM-074)**. **Cache key:** AI-001's key, that is the PM-038 fingerprint + stream + source range + kind `SpeechEnhance` + DeepFilterNet model id and version + a hash of the mapped attenuation. Changing the strength or the model version therefore makes a new entry instead of overwriting the old one.
- AU-020 plays the cached result at chain step 3, before AU-018's noise reduction at step 4. Until the entry exists, AU-020's own fallback applies (unprocessed audio with a badge; export waits).

**Edge cases:**
- Music or singing: it will be damaged; AU-020's hint warns about this.
- Strength 0: bypassed, no job.
- Very short clips (< 0.5 s): processed with padding.
- Sources at 44.1 kHz or other rates: resampled in and out.
- Cache entry evicted (PM-074): rebuilt on demand under the same key.
- Model not installed: AU-020 decides how its control looks; installing always goes through AI-002.

**Undo:** n/a for the engine. The toggle and strength are AU-020's undoable edit of `ClipAudio.clarity`.

**Model notes:** No clip fields are defined here; `ClipAudio.clarity` and its type belong to AU-020. `freecut-analysis::enhance::{map_strength(u8) -> Attenuation, process(pcm48k, attenuation) -> Vec<f32>}`; `map_strength` is pure and unit-tested. The cache files are written by `freecut-media`, and `freecut-audio` plays them.

### AI-025 Text-to-speech engine & voices — later

**What the user sees:** **Speak text** and **Read all captions** (TX-043 in text-and-captions.md) open this feature's voice dialog. It lists the installed voices (name, language, short description, license) with a **Preview** button that speaks the first sentence of the selected text. Speed, pitch, volume, fit-to-caption and keep-in-sync are TX-043's controls. A voice that is not installed yet can be installed from the dialog (AI-002).

**Inputs / controls (FreeCut defaults):**
- Voice: the first installed voice matching the text's language (the caption track's language tag, otherwise the UI language).
- Voice list filter: **the text's language** / all languages.
- Speed and pitch ranges and defaults: as in TX-043. Pitch is applied after synthesis by audio.md's pitch effect.

**Behavior / output:**
- A local TTS engine reads the text sentence by sentence. Candidates: Kokoro-82M (Apache-2.0 weights; several languages, dozens of voices) and Piper (the original repository is MIT and archived; development continues under GPL-3.0; **voice licenses vary per voice**).
- Only voices with permissive, CC0 or CC-BY licenses enter the manifest. Non-commercial voices are excluded.
- Numbers, dates and symbols are normalized for the chosen language before synthesis. Emoji are skipped; URLs are read as their domain names.
- Long text is chunked by sentence, with progress (TX-043 sets the size limit).
- Output is a WAV file in the **generated-media store (PM-041)**, registered as a derived media item (`GeneratedKind::Tts`) with the voice, its version and the normalized text in `recipe`. Placing the clip on a track, linking it to its text item, marking it out of date and fitting it to the caption are TX-043's.

**Edge cases:**
- Empty text: Generate is disabled.
- No installed voice for the text's language: the voice list is empty and points to the model manager.
- A voice removed after generation: existing audio keeps playing; regenerating asks for another voice.
- Text that mixes two languages: read with the selected voice, and a hint suggests splitting the text.
- Preview while a long generation runs: allowed, at higher priority.

**Undo:** TX-043's `GenerateSpeech` op, one per run. Previews are not edits.

**Model notes:** Inference runs in `freecut-analysis` (`ml-onnx`), and the WAV is written by `freecut-media`. FreeCut does **not** offer voice cloning (see Out of scope).

### AI-026 Generative media extension point — later

**What the user sees:** Preferences → Smart tools → **External generators**. This is an advanced setting and is **off** by default. The user can register a local generator they installed themselves: a command-line tool, or a server on `localhost`, for images, video or music. The media library then gets **Generate…**, which takes a prompt and options. The outputs are imported as normal media and tagged **AI-generated** along with the prompt and the tool's name.

**Inputs / controls:**
- Generator entries: name, kind (image / video / audio), command or `http://localhost:<port>` endpoint, and a JSON argument template.
- Prompt, output size or duration, and number of results.

**Behavior / output:**
- FreeCut sends a documented JSON request and expects files in a temporary folder. It imports them using project-and-media.md's import rules and stores the generation info as media metadata.
- FreeCut ships **no** generative models and connects to **no** remote host. Endpoints other than localhost are refused.

**Edge cases:**
- Tool not found or crashes: the job fails and its stderr is shown.
- The output format isn't supported: import error.
- Long runs: they show up as AI-001 rows in PM-073's task list and can be cancelled.
- The tags keep a record of where the media came from. The user is responsible for the license of anything generated.

**Undo:** Follows project-and-media.md's media-import semantics. Registering a generator is a preference, so n/a.

**Model notes:** `GeneratorSpec` (preferences). The outputs are stored in the generated-media store as PM-041's `MediaOrigin::Generated { kind: External, produced_by, recipe }`, with the generator's name in `produced_by` and the prompt and options in `recipe`; there is no separate provenance type. Process and HTTP I/O are in `freecut-app`.

### AI-027 Search media by spoken words — later

**What the user sees:** The Media panel's search field (PM-054) gains a **Spoken words** chip next to its type chips. When the chip is on, the query also looks inside the transcripts of the media items (AI-009). An item that matches only by its speech shows a speech-bubble badge with the number of hits. Expanding the item lists its hits as short snippets: a few words of context with the matched words in bold, and the timecode. Then:
- Clicking a snippet opens the item in the player's media preview (PM-060 / EX-015) at that moment, with in/out marks set around the phrase.
- Dragging a snippet to the timeline inserts just that phrase, with a little padding.
- A footer says how many items in scope have no transcript yet and offers **Transcribe…** for them (AI-009, as a batch).

Reference: the reference editor's desktop product page advertises finding footage by the dialogue spoken in it, alongside object and people search. FreeCut covers the spoken-word part locally; visual search is out of scope (see below).

**Inputs / controls (FreeCut defaults):**
- Spoken words chip: **off**. Its state is remembered per project as view state.
- Query: the same text field as PM-054. Words without quotes must all appear within one transcript segment, in any order. Text in double quotes is an **exact phrase**: consecutive words, which may cross a segment boundary.
- Minimum query length for spoken-word search: 2 characters.
- Snippet padding for drag and preview marks: **0.3 s** on each side.
- Snippets per item: the first **20**, then **Show all**.

**Behavior / output:**
- Matching uses PM-054's folding (case- and accent-insensitive) on the transcript's current word text, including the user's corrections (AI-011). Word starts must match: "cat" finds "cats" but not "concat". Languages written without spaces (e.g. Chinese, Japanese) use substring matching on the folded text.
- Items are ranked by the number of hits, then by PM-054's normal order. Name and note matches from PM-054 still appear; an item that matches both ways shows both.
- A hit's source range runs from its first word's start minus the padding to its last word's end plus the padding, clamped to the media and snapped outward to frames. Dragging inserts that range with TL's normal insert (TL placement rules).
- The search runs over an in-memory index of folded word tokens, built in the background when the project opens and updated after each transcript change. Searching 10 hours of transcripts (about 100,000 words) takes under 50 ms (FreeCut default target) and never blocks the UI thread.
- Nothing leaves the machine. No new data is saved.

**Edge cases:**
- No transcripts in scope: the chip shows "No transcripts yet" with **Transcribe…**.
- Stale transcript (PM-093 changed the file, AI-001): still searched, but its hits carry a "stale" badge, and inserting a hit asks for confirmation because the timing may be off.
- Partial transcript (only a range of the file was transcribed): only that range is searched, and the item notes it.
- Offline media: hits are listed; preview is unavailable, and an inserted clip is offline like any other (PM-090).
- Media with several audio streams: each transcribed stream is searched, and the snippet names the stream.
- Low-confidence words: they still match; the snippet shows them with AI-011's dotted underline.
- Generated speech (TTS, AI-025): searched like any audio that has a transcript.
- Very common query words (thousands of hits): results are listed lazily, and the badge shows "999+".

**Undo:** n/a (view state). Inserting a hit is TL's undoable insert op.

**Model notes:**
- `freecut-analysis::search::{fold_tokens(transcript) -> TokenIndex, find(index, query) -> Vec<Hit { words: Range<usize>, src_range: TimeRange }>}` are pure functions with unit tests (phrase across segments, accents, CJK, edited words).
- The per-project index lives in `freecut-app` next to PM-054's matcher, which merges the two result lists. No new persisted data; the chip state is view state.

### AI-028 Colorize black-and-white footage — later

**What the user sees:** In Inspector → Video → **Enhance** (AI-019) there is a **Colorize** option for video and image clips. A preview pane shows the current frame before and after, and the single-frame preview is ready in a few seconds. **Create colorized version** renders the clip in the background. When it finishes, the clip switches to the colorized version, and a **Use colorized version** switch lets the user compare, as in AI-019. In the media panel, the colorized file appears under the original in the Generated bin (PM-041).

Reference: the reference editor's public enhancer page shows a restored, colorized vintage clip, and a separate online tool colorizes photos. FreeCut does it locally with an open model.

**Inputs / controls (FreeCut defaults):**
- Engine: **Video (stable)** for video clips, **Image (detailed)** for still images. The user can switch engines.
- Color strength: 0–200 %. Default: **100 %**. It scales the predicted color; 0 % gives the original gray picture.
- Color detail: Low / **Medium** / High. These set the resolution the model works at (256 / 384 / 512 px on the short side).
- Temporal smoothing: 0–100. Default: **50**. It reduces color flicker between frames.
- Range: **the clip's used range + 1 s handles** (default) / the whole media.
- Before starting, the dialog shows the estimated time and disk use.

**Behavior / output:**
- Frames are decoded by `freecut-media`. The model sees the frame's lightness and predicts only the color (the two chroma channels of a Lab-style color space) at the detail resolution.
- The output keeps the **original full-resolution lightness**. Only the predicted color is added, upscaled with an edge-aware guided filter that follows the original's edges. Fine detail and grain stay as shot.
- Temporal smoothing blends each frame's color with the previous frame's color, moved along classic optical flow. The blend resets at shot cuts (AI-004), so colors never smear across a cut.
- Color strength multiplies the chroma before the result is encoded.
- The result is encoded with `freecut-media` as a visually lossless intermediate and written to the **generated-media store (PM-041)** as a derived media item (proposed `GeneratedKind::Colorized`). The clip's media reference is swapped as in AI-019, keeping the in/out points.
- Color grading (CL) and every later stage of FX-001 apply on top of the colorized source as normal, because the substitution happens at L1.

**Edge cases:**
- The source already has color: a warning says that colorizing replaces the existing color. Sepia or tinted footage is treated as gray, because only its lightness is used.
- Colors are the model's guesses: clothes, cars and walls may come out in plausible but historically wrong colors. There are no per-object color hints in this version.
- Large flat areas (sky, walls) can drift in color over a long shot. Higher temporal smoothing helps.
- Clip extended past the colorized range: that part plays in gray, with a "Partially colorized — extend?" badge (as AI-019).
- Interlaced sources: deinterlaced first. HDR sources: disabled in this version, because the models expect SDR.
- Sources with alpha: the alpha is copied through unchanged.
- The derived file is deleted: the clip falls back to the original with a badge, and PM-041's relink dialog offers **Regenerate**.
- Model not installed: AI-002 prompt.
- Long jobs: cancellable; the partial file is deleted.

**Undo:** Switching the clip to the colorized media, or back, is the same undoable media-switch op as AI-019. Parameter changes before rendering are undoable property edits. Rendering itself is not on the stack.

**Model notes:**
- `ColorizeParams { engine, strength: u16 /* percent */, detail, temporal: u8 }` lives with `EnhanceParams` on the clip, and is copied into the derived item's `recipe` with the model id and version.
- Inference runs in `freecut-analysis` (`ml-onnx`). Candidates: the DeOldify video and stable models (MIT code and MIT weights; the repository is archived, so FreeCut would maintain its own ONNX export) for video, and DDColor (Apache-2.0 code with an ONNX export script; weight license to verify) for stills. The flow-guided smoothing and the guided filter are pure Rust in the same crate.
- Artistic style transfer is not part of this feature (see Out of scope).

### AI-029 Auto montage (beat-synced auto edit) — later

**What the user sees:** Select several video or image clips (in the media panel or on the timeline) and one music clip or music file, then choose **Auto montage…** from the Smart tools menu. A dialog shows a strip previewing the planned edit: one block per slot, with a thumbnail and its length in beats. **Create** builds the edit in one step: the clips are cut to the music's beats, optionally trimmed down to their shots, optionally joined with transitions, and laid on a new track. The result is ordinary clips that the user can keep editing.

Reference: the reference editor's help centre describes an automatic cut feature that turns clips and music into a rhythm-synced edit, with sensitivity, minimum clip length and transition style settings. FreeCut's version is deterministic and built only from its own tools: AI-004 shot detection, AU-025 beat markers, AI-006 beat fitting and FX-006 transitions. Prompt-driven or "best moment" editing stays out of scope.

**Inputs / controls (FreeCut defaults):**
- Pace: cut every 1 / **2** / 4 / 8 beats (AI-006's N).
- Shot trimming: **off** / on. When on, each video clip is first divided into shots with AI-004 (its default detector and sensitivity), and each shot is a candidate.
  - Max shots per source clip: 1–10. Default: **3**. The longest shots are kept.
  - Minimum shot length: AI-004's minimum, 0.6 s by default.
- Order: **selection order** (timeline order or media-panel order) / by name / shuffle. Shuffle uses a seed that is stored in the op, so redo rebuilds the same edit; **Reshuffle** picks a new seed.
- Transitions: **None** / the project's default transition on every cut (FX-006), at its default duration clamped to half a slot.
- Length: **fit to the music** (stop at the music's end) / use every clip once.
- Clip audio: **muted** on the new clips (AU-007 clip mute) / kept.
- Start: **the playhead** / the start of the timeline.

**Behavior / output:**
1. If the music has no beat markers, AU-025's detection runs first (the dialog shows its progress). If the music came from the media panel, it is placed on a new audio track at the start time.
2. With shot trimming on, the AI-004 shot ranges of each source (cached per media, AI-001) become candidates. Otherwise each selected clip or media item is one candidate.
3. Candidates are ordered, then assigned slots of N beats by AI-006's `fit_to_grid`. A candidate longer than its slot is trimmed to the **middle** of its range (FreeCut default), so the cut avoids the shot's first and last frames. Shorter candidates follow AI-006's "skip to the next beat" rule. Images take exactly N beats.
4. The new clips go on a new video track above the topmost selected track. Originals on the timeline are left untouched.
5. With transitions on, FX-006's batch op adds the default transition to every cut of the new clips.
6. Everything is committed as one compound op, "Auto montage (N clips)".

**Edge cases:**
- Music with a variable tempo or no beats found: the dialog offers a fixed grid at a tempo the user types (AI-005's manual BPM), or Cancel. Nothing is built without beats.
- Fewer candidates than slots with "fit to the music": the montage stops when the candidates run out, and a hint offers **Repeat clips** (off by default).
- More candidates than slots: the extra candidates are left out and listed in the dialog.
- Candidates shorter than one beat: skipped and listed.
- Locked tracks: the destination is always a new track, so it is never locked. A locked source track is fine, because the sources are only read.
- Source clips with speed changes, effects or keyframes: the new clips copy the source clips' settings; trimming happens in timeline time through TR's time map.
- Shot or beat detection fails (e.g. media offline): the montage is not built, and the report names the failing item.
- Very many candidates (hundreds): the plan is computed off the UI thread, and the commit is one batched op with a single layout pass.
- The timeline is edited while the dialog's analysis runs: the plan is rebuilt before **Create** is enabled again.

**Undo:** One compound op, "Auto montage (N clips)": create tracks, insert the music clip when needed, insert and trim the new clips (AI-006's op builder), set clip mute, and FX-006's `ApplyTransitionToCuts` when transitions are on. One undo removes all of it.

**Model notes:**
- `freecut-analysis::montage::plan(candidates: &[Candidate], beats: &[Tick], params) -> MontagePlan` is a pure, deterministic function (seeded shuffle, middle trimming), unit-tested without media.
- `freecut-timeline::ops::auto_montage(plan) -> CompoundOp` composes `fit_to_grid` (AI-006) and FX-006's batch op. It uses no ML: the only analyses are AI-004's shot detection and AU-025's beat detection.
- No new persisted types beyond the seed and settings recorded in the op for redo.

### AI-030 Smart camera moves (subject- and beat-aware) — later

**What the user sees:** Inspector → Video → **Camera moves** on video and image clips, also available as right-click → **Add camera moves…**. The user picks a style, sees the planned moves on the preview as a dotted path with numbered framing boxes, and presses **Apply**. The result is ordinary position, scale and rotation keyframes (TR-027) that can be edited afterwards. **Reshuffle** plans a different variation, **Re-plan** refits the moves after the clip changed, and **Remove** deletes them. This differs from FX-019's continuous presets and FX-015's motion effects, which loop one fixed pattern. Camera moves are planned per clip from its length, its subject and, optionally, the music's beats.

**Inputs / controls (FreeCut names and defaults):**
- Style: **Gentle** (slow push-ins and drifts) / Handheld (a small, smoothed wobble around a steady framing) / Punch-in (quick zoom-ins that hold, then reset) / Energetic (larger, faster moves with a little rotation).
- Travel: 0–100 %, default **30 %**. How far the frame moves, as a share of the room the zoom leaves.
- Zoom range: 100–200 %, default **115 %** (Punch-in: 130 %).
- Rotation: 0–10°, default **0°** (Handheld: 1°, Energetic: 3°).
- Aim: **Center** / Subject. Subject keeps the detected face or person in frame, using AI-016's detections.
- Timing:
  - **Even**: a new move every 0.5–10 s, default 2 s.
  - On beats: every N beats of a chosen music clip, N = 1 / **2** / 4 / 8, read from its TL-037 beat markers.
  - On shot cuts: at AI-004's cuts inside the clip.
- Seed: stored with the moves. **Reshuffle** picks a new one.

**Behavior / output:**
1. Timing points are collected in clip-local time: even steps, the music clip's beat markers mapped through both clips' time maps, or AI-004 shot cuts.
2. For each segment between timing points, the planner draws a target framing from the seeded generator: a zoom within the range, an offset within the travel and a rotation within the limit. Every framing is constrained so the scaled picture always covers the canvas, so no empty border ever shows. With Aim = Subject, offsets are biased so the subject's box stays inside the central 60 % of the frame.
3. Moves are eased with TR-031's presets: ease-in-out for Gentle and Energetic, and a fast ease-out followed by a hold for Punch-in. Handheld adds seeded smooth noise at 0.5–2 Hz on top of a near-static framing, sampled at 6 keyframes per second.
4. The keyframes are simplified to about 1 % position tolerance, as in AI-016, and written in one compound op. The clip records `SmartMotion { origin: CameraMoves, params, seed }` (AI-001), so the inspector can show the settings and re-plan.

**Edge cases:**
- The clip already has motion keyframes from the user or from AI-016 or AI-017: AI-001's motion-writer rule asks before replacing them. The reference editor instead removes auto reframe automatically when camera moves are added, and the other way round.
- On beats with no beat markers on the chosen music: the dialog offers AU-025's detection first. Beats outside the clip are ignored. Fewer than 2 timing points inside the clip give one move across the whole clip.
- Clip shorter than 1 s: one move.
- Speed-changed or reversed clips: keyframes are in clip-local time and timing points are mapped, so moves still land on the beats.
- Replacing the clip's media (TL-035): the keyframes stay as they are, and **Re-plan** refits them. The reference editor re-applies its moves automatically.
- Aim = Subject and no subject found: falls back to Center, with a notice.
- Image clips work the same. This is the common slow push on a still photo.
- Locked track: disabled.
- The user cancels subject detection: nothing is written.

**Undo:** Apply, Re-plan, Reshuffle and Remove are one op each: `SetKeyframes` on position, scale and rotation, plus the `SmartMotion` record.

**Model notes:** `freecut-analysis::camera_moves::plan(duration, timing_points: &[Tick], style, params, seed, subject: Option<&SubjectPath>) -> Vec<MotionKey>` is pure and deterministic, because the generator is seeded. It is unit-tested for "never shows a border" and "keys land on timing points". `SmartMotion` lives on the clip in `freecut-timeline`; the keyframes are TR-027's. Subject detection reuses AI-016's `detect_subjects`. No new model.

### AI-031 Planar tracking (corner pin) — later

**What the user sees:** Inspector → **Tracking** gains a mode switch: **Point** (AI-008) / **Plane**.
- In Plane mode, a four-corner outline appears on the preview. The user drags its corners onto a flat surface, such as a screen, a sign, a wall or a book cover, and presses **Track**.
- The outline then sticks to the surface through the shot, with a confidence strip under the clip as in AI-008.
- Overlays attached to the plane (image, video, text, sticker) are pinned into it, so they look as if they were on the surface.
- A clip can hold several planes ("Plane 1", "Plane 2"…, which can be renamed). They are listed in the Tracking section with their tracked ranges. Each can be shown, edited or removed. With no planes, the section shows an empty state with **Add plane**.

**Inputs / controls (FreeCut defaults):**
- Direction: Forward / Backward / **Both** from the playhead.
- Motion: **Perspective** (full corner pin) / Position, scale and rotation only (keeps the shape).
- Allowed deformation: 0–100, default **50**. Limits how much the plane's shape may change from one frame to the next. Lower values resist jitter on small or noisy surfaces.
- Correction mode:
  - **Whole track**: moving a corner shifts it on every tracked frame.
  - This frame: moves it on the current frame only, then re-tracks from there.
- Corner positions: X and Y per corner in source pixels, and an angle offset for the attached content (0–359°, default 0°).
- Lock: none (default) / lock position (only the shape follows) / lock shape (only the position follows).

**Behavior / output:**
- Inside the plane, about 200 strong corner features are found at analysis size (long side ≤ 960 px) and followed from frame to frame with pyramidal Lucas–Kanade. A homography is fitted to them with RANSAC (inlier threshold 2 px at analysis size) and then refined on the inliers. The four corners move with that homography.
- Confidence is the share of inliers. Below 0.3 for 5 frames in a row, the plane is **lost** and tracking stops, as in AI-008.
- Per-frame corrections made in This frame mode become anchors. Re-tracking starts from them and never overwrites them.
- The result is a `PlaneTrack`, sampled per source frame of the tracked clip, in raw source coordinates (AI-007). Stabilization, trims and speed changes therefore need no re-track.
- An attached overlay is drawn into the plane's quadrilateral at L8. In Perspective mode this needs a corner-pin (projective) placement, which TR currently lists as out of scope. Until TR adopts it, Perspective planes can be tracked and edited, but attached overlays use the plane's best position, scale and rotation fit instead, and a notice says so.
- Tracking runs as an AI-001 job with Stop. Its result lands as one op.

**Edge cases:**
- Too little texture (fewer than 20 features, as on a blank wall or a screen with glare): a warning before tracking suggests a larger or more detailed area.
- Partial occlusion, such as a hand passing over the surface: RANSAC ignores the covered features while at least 8 inliers remain.
- The plane partly leaves the frame: tracking continues on the visible features, and corners may lie outside the picture.
- Switching Motion, re-tracking or stopping keeps the user's per-frame anchors. The reference editor discards hand-adjusted frames in these cases, after a warning.
- Moving, trimming or retiming the tracked clip keeps the plane, because it is stored in source time and coordinates. The reference editor removes tracking when the tracked clip is moved.
- Tracking range shorter than 2 frames: Track is disabled.
- Locked track: read-only.

**Undo:** Add plane, Track or re-track, each corner correction, rename, attaching or detaching an overlay, and Remove plane are one op each.

**Model notes:** `PlaneTrack { id, name, source_clip: ClipId, motion: PlaneMotion, deform_limit: u8, lock: PlaneLock, anchors: BTreeMap<SourceFrame, [Vec2; 4]>, samples: Vec<PlaneSample { src_time: RationalTime, corners: [Vec2; 4], confidence: f32 }> }` is stored in the project, because it is bound data like AI-008's `TrackData`. The tracker is pure Rust in `freecut-analysis::track::planar`, with no model. Overlays bind through AI-008's binding with a `Plane(id)` source. If TR adopts the projective placement, it belongs to TR-001's transform and to `freecut-render`.

### AI-032 Retouch presets — later

**What the user sees:** The Retouch section (AI-020) has a **Presets** row at its top. It holds a few bundled FreeCut presets, described by their effect (for example a light, natural clean-up, softer skin, or more defined features), followed by the user's own presets. Clicking one applies it to the current **Apply to** target. An **Amount** slider (0–100 %) then scales the whole preset. **Save as preset…** stores the current retouch values under a name. Each user preset has Rename, Duplicate and Delete in its menu. The bundled list is FreeCut's own; no preset of the reference editor is reproduced.

**Inputs / controls (FreeCut defaults):**
- Amount: 0–100 %, default **100 %**. It multiplies every non-zero value of the applied preset.
- Preset content: any of AI-020's skin, feature and shape values, AI-033's color tints and, optionally, a color look (a CL-005 bundled look or a CL-026 user look) with its intensity.
- Names: 1–40 characters, unique (case-insensitive). The default name is "Retouch 1", "Retouch 2"…, using the next free number.
- At most **50** user presets.

**Behavior / output:**
- Applying a preset writes the scaled values into the clip's `RetouchParams`, and applies the look through CL-004's op. Afterwards they are ordinary slider values. Changing a slider changes only that value, and the preset is then shown as "modified". Nothing is cleared. The reference editor's one-click look and its manual adjustments clear each other, with a notice; FreeCut keeps one set of values instead.
- Presets are app-level data in a JSON file in the user's config directory, like CL-026's user looks, so they are available in every project. A preset holds values only, never media.
- Save is disabled while every value is 0, with a tooltip.

**Edge cases:**
- 50 presets already saved: Save is disabled, with a hint to delete one first.
- Duplicate name: the dialog proposes the next free variant, such as "(2)".
- A preset whose look is missing (for example a deleted user look): it is applied without the look, with a notice.
- A preset saved by a newer FreeCut with sliders this build doesn't know: the unknown values are ignored.
- Writing the preset file fails (disk full, permissions): an error toast with **Retry**, and the list is unchanged.
- **Selected face** in a multi-face clip: the preset applies to that face only.

**Undo:** Applying a preset or changing Amount is one compound op: `SetRetouchParams`, plus CL's look op when the preset has a look. Saving, renaming, duplicating and deleting presets are app preferences, not edits (n/a).

**Model notes:** `RetouchPreset { id, name, values: RetouchValues, look: Option<(LookRef, u8)>, version }` is stored by `freecut-app`'s preferences storage (UI-018). `apply_preset(values, amount) -> RetouchParams` is a pure function in `freecut-timeline`, unit-tested so that amount 0 is the identity and 100 gives the exact values.

### AI-033 Makeup & hair color tints — later

**What the user sees:** The Retouch section gains a **Color** group with Lip tint, Eye shadow, Blush, Brow tint and Hair color. Each row has a color swatch and an Amount slider, and all start at 0 (off). Hair color works on video and still images. It recolors only the hair and keeps its shine and strands.

**Inputs / controls (FreeCut defaults):**
- Each tint has a color and an Amount from 0 to 100, default **0**. Default colors: lips a muted rose, eye shadow a warm brown, blush a soft pink, brows a dark brown, hair a chestnut.
- Hair color also has **Keep highlights**, default on. The tint fades out in the brightest 10 % of hair pixels, so shine stays natural.
- Targeting follows AI-020's **Apply to** (all faces or the selected face).

**Behavior / output:**
- The lips, eyelids, cheeks and brows come from AI-020's landmark polygons, feathered and smoothed over time. The teeth mask is subtracted from the lip mask, so an open mouth stays natural.
- Tints are applied in linear light at FX-001's **L2b Retouch** stage as a luminance-preserving color blend, so texture and shading stay.
- Hair uses a hair segmentation model (MediaPipe hair segmentation; Apache-2.0 code, model card to verify). It runs per source frame at analysis size, is smoothed over time, and is upscaled with the edge-aware guided filter that AI-014 uses. Hair masks are AI-001 records in the media cache (PM-074).
- Changing a color or an Amount never re-runs any analysis.

**Edge cases:**
- Very dark hair takes little color in a luminance-preserving blend, and a hint says so. Lightening hair is not offered.
- Beards and body hair are outside the hair mask.
- Hats, hands or hair crossing the face: the masks follow the landmarks and the segmentation. Small errors show as a faint tint on the edges, which a lower Amount reduces.
- Faces that AI-020's landmark rules drop (profile, occlusion, too small) fade their tints out over 3 frames, like the other retouch values.
- Hair model not installed: the Hair color row shows AI-002's install prompt, and the face tints still work.
- Changing hair style or length is not offered (see Out of scope).

**Undo:** Every color and Amount change is an undoable property edit (`SetRetouchParams`).

**Model notes:** `MakeupParams { lips, eyes, blush, brows, hair: Tint { color: Rgb, amount: u8 }, keep_highlights: bool }` lives inside `RetouchParams`. Masks and blends are WGSL in `freecut-render`. Hair inference runs in `freecut-analysis` (`ml-onnx`).

### AI-034 Virtual relighting — later

**What the user sees:** Inspector → Video → **Relight** on video and image clips. The user adds up to 4 lights and drags each light's handle on the preview. The picture brightens and takes on the light's color around it, as if a lamp stood there. A **Target** choice limits the light to the subject, the background, faces or the whole frame. **Animate** makes a light pulse, cycle its color or move along an orbit. User presets can be saved and reused.

**Inputs / controls (FreeCut defaults):**
- Lights: 0–4, with Add and Remove. Each light has:
  - Type: **Point** / Directional.
  - Color (default warm white) and Brightness 0–200 % (default **60 %**).
  - For a point light: position (the handle), Radius 1–100 % of the frame diagonal (default **35 %**), and Falloff soft / **medium** / hard.
  - For a directional light: Angle 0–359° (default 45°) and Elevation 0–90° (default 30°).
  - Softness 0–100 (default **50**): how far the light wraps around shapes.
  - Animate: **None** / Pulse / Color cycle / Orbit, at 0.1–5 cycles per second (default 0.5).
- Target: **Whole frame** / Subject (inside AI-014's matte) / Background (outside it) / Faces (AI-021's face tracks).
- Ambient: −100 to +100, default **0**. Darkens or brightens everything the lights don't reach. An ambient tint color goes with it.
- Highlight protection: **on**. A soft shoulder keeps lit highlights from clipping.
- Depth: **Auto** (uses the depth model when it is installed, otherwise Flat) / Flat (2D falloff only, no model).
- Presets: Save as preset, Rename, Duplicate and Delete, with at most **50** user presets. **Reset all** returns every value to its default, after a confirmation.
- Light position, color and brightness are keyframable (TR-027).

**Behavior / output:**
- With depth, a local monocular depth model estimates a depth map per source frame at analysis size (short side ≤ 518 px). Normals come from the depth gradients. Each light adds diffuse, Lambert-style shading in linear light: out = in × (ambient + Σ color × brightness × max(0, n·l) × falloff). Flat mode treats every pixel as facing the camera, so lights become soft radial gradients (point) or linear gradients (directional).
- Depth maps are AI-001 records in the media cache (PM-074). They are smoothed over time to prevent flicker and upscaled with the edge-aware guided filter.
- Light positions are stored in normalized source-frame coordinates and mapped like any analysis result (AI-007), so lights stay put in the scene when the clip is stabilized or cropped.
- Relight renders at FX-001's **L6** as a clip effect item (FX-010). Its parameters, analysis and shader belong to this feature; FX owns the effect-item mechanics and stacking order. It therefore sees the color-corrected picture (L5) and the keyed alpha (L4).
- Export uses the cached depth. If depth is missing, export waits for it (AI-001). The reference editor blocks export until relighting finishes.

**Edge cases:**
- Depth model not installed: Auto works as Flat, with an install link (AI-002).
- Subject or Background target without a matte: AI-014's analysis is queued without switching background removal on. Until the matte is ready, the light covers the whole frame and shows a badge.
- Animated images (OV-012) are handled like video. The reference editor refuses GIF sources.
- GPU unavailable or its driver too old: depth runs on the CPU with AI-001's notice. Flat mode needs no analysis at all.
- Very bright lights on areas that are already bright: highlight protection compresses them. Turning it off allows clipping.
- 50 presets saved: Save is disabled, with a hint. The reference editor has the same limit.
- Cancelling the depth analysis: the effect renders Flat until the analysis runs again.
- Depth models are weak on mirrors, glass and sky, where the light can look painted on. Documented.

**Undo:** Adding or removing a light, every parameter change and applying a preset are undoable effect edits (FX-012's parameter op). Reset all is one compound op. Saving and managing presets are app preferences (n/a).

**Model notes:** `RelightParams { lights: Vec<Light { kind, color: Animatable<Rgb>, brightness: Animatable<f32>, pos: Animatable<Vec2>, radius, falloff, angle, elevation, softness, animate }>, target, ambient, ambient_tint, protect_highlights, depth_mode }` are the Relight effect's parameters. Depth inference runs in `freecut-analysis` (`ml-onnx`) with Depth Anything V2 **Small** (Apache-2.0 code and weights; its larger variants are CC-BY-NC and excluded) or MiDaS (MIT). The shading is WGSL in `freecut-render`. Presets are app-level JSON, like AI-032's.

### AI-035 Project health check & one-click fixes — later

**What the user sees:** Smart tools → **Check project…** opens a **Health** panel (a UI-056 auxiliary panel).
- **Analyze** scans every clip in scope in the background, with percent and **Stop**. The panel shows when the project was last checked.
- Findings are grouped into **Picture**, **Sound** and **Speech** (FreeCut's grouping). Each finding names the problem (for example flicker, shake or uneven loudness) and lists the affected clips with thumbnails. A "why" line gives the measured value for each clip.
- Each finding has one fix button. It applies a tool from its owning spec to all listed clips, or only to the ones the user keeps checked.
- Applied fixes are listed with **Remove**.
- When nothing is found, the panel says the project looks fine.
- The panel can be collapsed, and each finding has a more/less detail toggle.

**Inputs / controls (FreeCut defaults):**
- Scope: **whole timeline** / selected clips / in–out range.
- Each check can be turned off. Thresholds are FreeCut defaults, shown in each finding's "why" line, and can be changed under **Advanced**:

| Finding | Detector (classic, no model) | Flagged when | Fix (owner) |
|---|---|---|---|
| Flicker | per-frame band levels (CL-034's figures) | frame-to-frame jitter above 0.15 EV RMS | Deflicker (CL-034) |
| Blur | variance of the Laplacian on 8 sampled frames | below a resolution-scaled threshold on most samples | Enhance (AI-019) |
| Shake | global motion over short bursts of frames | high-frequency motion above 1 % of the frame width | Stabilize (TR-026) |
| Dropped or repeated frames | identical consecutive frames, timestamp gaps | more than 1 % of frames | Frame blend or AI interpolation (TR-041, AI-018) |
| Low resolution | source size against on-canvas size at the export resolution | scaled above 150 % | Enhance low-resolution clips (AI-019) |
| Video noise | variance in flat areas | above a level-dependent threshold | Denoise (CL-033) |
| Color mismatch | mean Lab difference between adjacent clips on a track | ΔE above 8 | Color match (CL-020) |
| Exposure | luma percentiles | more than 5 % of pixels clipped or crushed | Auto color (CL-019) |
| Uneven loudness | integrated loudness per clip (AU-017's meter) | spread above 6 LU | Loudness normalization (AU-017) |
| Background noise | noise floor in pauses (AI-003's measurement) | above −50 dBFS | Noise reduction (AU-018) |
| Clipping | sample peaks | more than 0.1 % of samples at full scale | flagged only, no automatic fix |
| Unclear speech | voice activity plus speech-to-noise ratio | below 15 dB | Voice clarity (AU-020) |
| Speech without captions | voice activity against caption tracks | speech with no caption over it | Generate captions (AI-010) |

**Behavior / output:**
- The scan runs on AI-001's analysis pool. It reuses existing records (CL-034's figures, AU-017's loudness, AI-003's levels, TR-026's motion) and stores its own per-clip measurements as AI-001 records, so re-checking after small edits is fast.
- A fix calls the owning feature's own op on every checked clip, with that feature's defaults, as **one compound op** named after the fix. It also records an **applied fix** in the project: the fix kind and, for each clip, the values it set.
- **Remove** on an applied fix restores the previous values on every clip where the values are still the ones the fix set. Clips the user has since changed by hand keep the user's values, and the panel says how many. Remove is one compound op.
- Nothing is applied automatically; every fix takes a click.
- Cosmetic tools (face retouch, eye-contact changes, text-to-speech, caption styling) are not offered as fixes, because they are a matter of taste rather than faults. The reference editor's version includes them.

**Edge cases:**
- The project is edited after a scan: findings are marked as possibly out of date, and **Re-check** offers a new scan. Findings whose clip was deleted disappear without an error.
- Clips added after the scan: the panel shows how many clips are not checked yet, with **Check them**. If an applied fix fits them (for example the same flicker), the panel offers to extend the fix to them.
- A fix fails on some clips (media offline, model missing): it is applied to the others, and the failures are listed with **Retry**. The op covers only the clips that succeeded.
- Stop during a scan keeps the finished measurements.
- Offline clips are skipped and listed.
- A fix whose model isn't installed shows AI-002's prompt before it runs.
- The detectors are heuristics and can flag intentional looks, such as a strobe or a deliberate blur. Unchecking a clip in a finding is remembered for this project.
- No consent dialog is needed, because the analysis is local. The reference editor asks for consent before its analysis.

**Undo:** Each fix and each Remove is one compound op. Scanning is analysis and is not on the undo stack.

**Model notes:** `freecut-analysis::health::{measure_clip(...) -> ClipHealth, findings(&[ClipHealth], thresholds) -> Vec<Finding>}` are pure and unit-tested with synthetic signals (flicker, noise, shake, loudness steps). `AppliedFix { id, kind, entries: Vec<(ItemId, FixValues)> }` is stored in the project by `freecut-project`. Each owning spec supplies the op that sets and restores its values. The Health panel is in `freecut-app`. No model is used, apart from those of the fixes themselves.

### AI-036 Clips from long recordings — later

**What the user sees:** Smart tools → **Find clips in long recordings…** opens a dialog with three steps.
1. **Setup:** the sources (all videos in the media panel, only items with speech, or the selected items, with a count), a clip length, a total length target, the order, optional keywords, and whether to reframe the clips to the canvas.
2. **Analyze:** transcription (AI-009), shot detection (AI-004) and loudness run in the background, with percent and ETA. The dialog can be minimized to the task list and reopened. A notice appears when the results are ready.
3. **Results:** a two-level list. Chapters (topic blocks of each recording) hold candidate clips, each with a thumbnail, its length, a score bar and a "why" line (for example "3 keyword hits · speech 92 % · lively"). The user can select or unselect clips, drag to reorder them, drag a clip's edges to trim it within its source, rename, delete from the list, add a chapter and re-run. Then the user chooses an output.

**Inputs / controls (FreeCut defaults):**
- Clip length: **Auto** (follows the content, 10–90 s) / under 15 s / 15–30 s / 30–60 s / over 60 s.
- Total length: **Auto** (keep every clip scoring at least 50) / 15 s / 30 s / 60 s / 3 min / 5 min / 15 min / no limit.
- Order: **Chronological** / Highest score first / Grouped by recording.
- Keywords: optional free text up to 500 characters, or **Load from text file…** (TXT or Markdown, up to 64 KB, where headings and list items become keywords).
- Reframe to the canvas: **off** / on (AI-016 on each placed clip).
- Output:
  - **Place on the timeline**: on a new track at the playhead (default), or **Replace the timeline**, which asks first.
  - **Mark ranges in the media panel**: sub-ranges on each media item, like AI-004's shots.

**Behavior / output:**
- **Candidates.** For recordings with speech: runs of transcript sentences bounded by pauses of at least 0.6 s, merged or split to fit the length setting. An edge moves to a shot cut (AI-004) when one lies within 0.5 s. For recordings without speech: AI-004 shots that fit the length setting.
- **Chapters.** A new chapter starts at a pause of 2 s or more that coincides with a shot cut, and at the start of each recording.
- **Score** (0–100, deterministic, explained in a "How scores work" popover): 40 % keyword hits (matched with AI-027's folding on word starts), 30 % share of the candidate covered by speech, 20 % loudness energy relative to the recording's median, and 10 % visual activity (the spread of AI-004's frame scores). Without keywords, the other weights are scaled up in proportion.
- **Fill.** Candidates are picked by score until the total length target is reached, then put in the chosen order.
- **Place on the timeline** inserts the selected clips end to end as ordinary clips cut from the media items, on a new video track with their linked audio on a new audio track, as one compound op. Because they come from media items, they carry no speed changes or keyframes. **Replace the timeline** removes the existing content in the same op.
- Nothing leaves the machine, and no language model is used. Ranking against a written brief with a language model stays out of scope.
- Observed alternative: the reference editor runs this as an online service, with content-type choices (interview, podcast, tutorial and so on), mood and story arrangements, and an uploaded outline document. FreeCut keeps length settings, keywords from TXT or Markdown, and a transparent score.

**Edge cases:**
- No sources selected: Next is disabled, with a hint.
- Sources removed from the project during analysis: if all of them are gone, the job stops. Otherwise the dialog offers **Continue with the rest** or **Start over**.
- No clips found (no speech and no fitting shots): an empty state suggests another clip length.
- Image items are skipped, because they have no duration to cut. The reference editor keeps them but cannot change their duration.
- A recording that is only partly transcribed: only the transcribed part gives speech candidates, and the rest falls back to shots.
- Re-run after edits to the results: asks before replacing the list, because hand edits are lost.
- Closing the results with unplaced edits: asks first.
- Placing with nothing selected: disabled, because at least one clip must remain selected.
- Locked tracks: untouched, because output goes on new tracks. **Replace the timeline** is disabled while any track is locked.
- Very long sources (hours): analysis is chunked (AI-009), and the results list is virtualized.

**Undo:** Place on the timeline, including Replace, is one compound op. Mark ranges is one op on the media items. Edits in the results list are dialog state (n/a) until output.

**Model notes:** `freecut-analysis::segments::{candidates(transcript, shots, loudness, bucket) -> Vec<Candidate>, chapters(...) -> Vec<Chapter>, score(candidate, keywords, weights) -> Score, fill(candidates, target, order) -> Plan}` are pure and unit-tested on fixture transcripts. The plan is app state in `freecut-app`. Placement uses TL's insert ops. The only model is AI-009's.

### AI-037 Organize media into bins — later

**What the user sees:** In the media panel, right-click → **Organize into bins…** works on the selection. With nothing selected it works on all media, and the dialog says so. The user picks a grouping, and a preview tree shows the proposed bins with item counts and thumbnails. Bins can be renamed, and items dragged between proposed bins, before **Create bins**. A grouping setup can be saved as a preset and reused in other projects.

**Inputs / controls (FreeCut defaults):**
- Group by: **Capture date** (by day; or by month or year) / Place / Camera or device / Look / Orientation and size.
- Place: cluster radius 0.1–50 km, default **1 km**. Bins are named by date range and coordinates, because there is no online map lookup. The user can rename them.
- Look: grouping strength 0–100, default **50**. Higher values give fewer, broader groups.
- Use file dates when the capture date is missing: **on**.
- Second level: optional, for example capture month, then camera.
- Existing bins: **organize only items that are not in a bin** / reorganize everything.

**Behavior / output:**
- Capture date, place and device come from each file's own tags, read by `freecut-media`'s probe: the container's creation time and location tags, and Exif date, GPS and camera model for photos. These would be optional fields of PM-035's probe data.
- Place clusters items by great-circle distance (single linkage within the radius).
- Look uses classic image statistics on PM-070's thumbnail frames: a color histogram plus a tiny grayscale layout per item, clustered agglomeratively with a threshold set by grouping strength. It uses no model and no faces.
- Items without the needed data go into an "Unknown date" or "No location" bin (FreeCut names).
- **Create bins** creates the bins (PM-052) and moves the items, as one compound op. Bins are virtual, so files on disk never move.
- Presets store only the grouping settings, in app preferences.
- Observed alternative: the reference editor uploads media for grouping and can also group by recognized people. FreeCut works locally and does not group by faces (see Out of scope).

**Edge cases:**
- Time zones: a tag with an offset is shown in that local time. A tag without one is treated as UTC and shown in the computer's time zone, with a note.
- Dates that exist only in the file name: not parsed (FreeCut default). File dates are the fallback.
- No item has date or location data: the preview shows a single "Unknown" bin and suggests another grouping.
- Very large libraries (thousands of items): grouping runs off the UI thread, and the preview tree is virtualized.
- Closing the dialog after renaming or moving items in the preview: asks first.
- A second level that would exceed PM-052's nesting limit: it is flattened, with a notice.

**Undo:** One compound op, "Organize N items into M bins", made of PM-052's create-bin and move ops. Presets are preferences (n/a).

**Model notes:** `freecut-analysis::organize::{group_by_date, group_by_place, group_by_device, group_by_look}` are pure functions over plain item metadata and thumbnail statistics, and are unit-tested. PM would add the probe fields (`captured_at`, `location`, `device`) to PM-035's probe data. The bin ops belong to `freecut-timeline` (PM-052). No model is used.

## Interactions with other areas

- **timeline-editing.md (TL):**
  - Defines rational time, clips, linked clips, splits, ripple deletion, markers, link groups and the undo stack. Every apply step here is built from TL ops as one compound, atomic op.
  - **Beat markers are TL-037 clip markers** with `origin = Beat`, stored in source time with an auto/manual flag. AU-025 creates them; AI-005 rewrites only the auto ones; AI-006 and AI-029 read both kinds. There is no separate beat-marker type. Scene markers (AI-004) also use TL-037.
  - This area needs from TL: split and range delete/insert/join ops (AI-003, AI-004, AI-011, AI-012), TL-037's marker ops (AI-004, AI-005), insert (AI-027, AI-029, AI-036) and a `fit_to_grid` op (AI-006, AI-029).
  - TL-023's split copies AI-008's overlay binding to the new piece, so a tracked clip that is split keeps its tracking on both pieces.
  - TL-003 allows one timeline per project, so AI-036 places its clips on new tracks of that timeline (or replaces its content). AI-016's standalone mode creates a new project instead.
  - AI-030 reads beat markers from TL-037 in the same way as AI-006.
- **clip-transform-and-speed.md (TR):**
  - Owns keyframes, transforms, crop, masks, chroma and luma key, and the clip time map (speed, curves, reverse). All source↔timeline time mapping here uses TR's time map.
  - Owns **video stabilization** (TR-026). This spec only makes its own analyses stabilization-aware (AI-007).
  - Owns the frame-sampling setting: nearest and frame blend (TR-041) and classic optical flow (TR-042). TR-041's `FrameSampling` enum reserves `Ai { quality }`, whose behaviour is AI-018's; an unknown variant loads as Frame blend.
  - Auto reframe, subject follow and camera moves (AI-016, AI-017, AI-030) write TR keyframes. AI-001's motion-writer rule decides when one of them replaces another's keyframes. Motion tracking (AI-008) binds overlays and can bake to TR keyframes. TR-017 masks can follow AI-008 tracks.
  - AI-031's Perspective mode needs a corner-pin (projective) placement, which TR's Out of scope currently excludes. Until TR adopts it, plane-attached overlays use TR-001's position, scale and rotation as an approximation.
- **text-and-captions.md (TX):**
  - Owns caption segments and tracks, caption line rules (TX-037), word timings and the active-word highlight (TX-039), keyword emphasis (TX-040), speaker labels (TX-042), styles, subtitle formats, RTL/CJK layout, and how recognition, translation, speech and lyric results land on caption tracks (TX-038, TX-041, TX-043, TX-044).
  - This spec owns the jobs, models and dialogs behind those commands (AI-009, AI-010, AI-013, AI-025, and AI-023 for vocal isolation in TX-044). Transcript editing, filler cleanup and spoken-word search (AI-011, AI-012, AI-027) are this spec's.
  - Recognized captions use **per-media transcription**: AI-009 recognizes each media file once and AI-010 maps the words through the clips, so the same transcripts serve captions, transcript editing and search. Results land through TX-038's `ReplaceCaptions`. The runtime is `freecut-analysis` in both specs.
  - AI-013 owns the translation context window and the split-back rule; TX-041 owns where the translated text lands.
  - TX-044's "align pasted lyrics" path is AI-010's **Align known text** mode (captions slice CS3), which outputs through `ReplaceCaptions` with `origin: Lyrics` or `origin: Script`.
  - TX-042's automatic speaker detection needs a diarization model that this spec has not chosen yet (see Out of scope).
- **audio.md (AU):**
  - Owns the clip audio attributes (`ClipAudio`) and the signal chain, and these features:
    - **Beat detection** (AU-025): the onset, tempo and tracker job (run on AI-001's analysis pool), the density and sensitivity controls, the tempo readout and the tap key, with AU-026's latency compensation. Beats are TL-037 markers only; `ClipAudio` holds no beat data.
    - Model-free noise reduction (AU-018) and voice polish (AU-019).
    - The **Voice clarity** control (AU-020: toggle, strength 0–100, default 70, in `ClipAudio.clarity`), applied at chain step 3 from the processed-audio cache.
    - Both separation commands (AU-021): the in-place Voice isolation dropdown, and **Split into stems…** (linked clips on new audio tracks, 2 or 4 stems, source audio muted but never deleted, one undo step).
    - Voice detection for ducking (AU-028).
  - This spec adds beat-grid refinement only (AI-005, with `BeatGridParams` on AU-025's beat-analysis record), beat sync (AI-006), auto montage (AI-029), silence removal (AI-003), and the engines behind AU-020 and AU-021: AI-024 (DeepFilterNet, its strength mapping, install prompt and cache keys) and AI-023 (separation model, job, quality, and FLAC stems as PM-041 derived media, decoded at chain step 1).
  - All model inference runs in `freecut-analysis`, orchestrated by `freecut-app`; `freecut-audio` only plays processed audio and stem files.
  - AI-035's health check reads AU-017's loudness measurement, and its sound fixes call AU-017 (normalization), AU-018 (noise reduction) and AU-020 (voice clarity) with their own ops and defaults.
- **color-and-filters.md (CL):**
  - Owns automatic color correction (CL-019) and color matching (CL-020), which are histogram-based and not in this spec.
  - Owns classic, live video noise reduction (CL-033, Denoise) and flicker removal (CL-034, Deflicker). The reference editor's two deflicker strengths (one for fast-changing content, one for slow) correspond to CL-034's Strength and Window; any change there is CL's call.
  - AI-019 adds ML upscaling and denoising, and AI-028 adds ML colorization. Both produce derived media that CL then grades like any source.
  - AI-035's picture fixes call CL-019, CL-020, CL-033 and CL-034, and its flicker check reuses CL-034's per-frame figures.
  - AI-020's skin smoothing leaves skin-tone protection to CL-030. AI-032's presets can carry a CL-005 or CL-026 look, applied through CL-004's op.
  - AI-034's Relight sits at L6, after CL's color stage (L5).
  - Style transfer is out of scope (below), so CL's filters and LUTs remain the way to give footage a stylized look.
- **effects-transitions-animations.md (FX):** Owns the per-frame pipeline (FX-001), the person- and face-tracking effects (FX-020, FX-021) and the transition batch op (FX-006). Where this spec's output enters FX-001's stages:
  - L1 source sample: derived substitutes (AI-018 synthesized frames from the media cache; AI-019 enhanced, AI-022 inpainted and AI-028 colorized media from the generated-media store).
  - **L2b Retouch:** face retouch (AI-020, including brush strokes) and makeup and hair tints (AI-033), on raw source pixels after L2 Normalize. This spec's analyses read the L2 output, before L2b (AI-007).
  - L3 stabilization and crop (TR-026): this spec's analyses are mapped through it (AI-007).
  - L4 keying: the cutout matte (AI-014/AI-015), multiplied with TR's keys.
  - L6 clip effects: FX-020 reads AI-014's person matte, and FX-021 reads AI-021's face tracks. AI-034's Relight is an FX-010 effect item at L6: FX owns the item mechanics and stacking order, and this spec owns its parameters, depth analysis and shader.
  - L7–L8: user masks (with corner-radius coverage folded into L7), then the transform, including AI-016/AI-017/AI-030 keyframes, AI-008 tracked placement and AI-031 plane placement.
  - AI-030's camera moves are planned per clip from analysis and written as keyframes. They are distinct from FX-019's continuous presets and FX-015's motion effects, which loop a fixed pattern.
  - AI-021's shape or sticker covers are an alternative to FX-021's blur and pixelate looks. Whether FX-021 adds solid-shape looks of its own is FX's call.
  - **L8b Element styling** (shadow, outline, content, border, clip frame), after the transform and before L9/L10: where OV's outline and shadow land on a cut-out subject.
  - AI-029 uses FX-006's `ApplyTransitionToCuts` for its optional transitions. It is not a template-driven edit; FX's out-of-scope item about templates is unaffected.
- **overlays-and-stickers.md (OV):**
  - Stickers, shapes and images are the usual targets of motion tracking (AI-008) and plane tracking (AI-031). OV-022 follows an object with AI-008's tracker.
  - OV-023's cut-out stickers use AI-014. Outline and drop shadow (OV-006), border and corner radius (OV-005) and clip frames (OV-029) on a cut-out subject are applied by FX-001 at L8b Element styling (corner radius at L7), after AI-014's matte at L4. The outline looks observed in the reference editor (AI-014's edge cases) are input for OV-006, which decides which to adopt.
  - AI-021's shape or sticker covers are OV-024 shapes or OV-018 stickers bound to face tracks through AI-008's binding.
- **project-and-media.md (PM):**
  - **PM-038's `Fingerprint`** is the only media content key. AI-001's result key builds on it (with no media id or modification time), so copies and relinks reuse results. **PM-093** decides when a file has changed, which is where AI-001's staleness comes from.
  - **Media cache (PM-074):** holds this spec's deterministic, regenerable outputs (analysis records, mattes, propagated masks, AI-interpolated frames, processed audio). AI-001 owns their keys and the **Clear analysis cache** button; export waits for regeneration.
  - **Generated-media store (PM-041):** holds derived files that clips point at (AI-019 upscaled, AI-022 inpainted, AI-023 stems, AI-025 TTS, AI-028 colorized, AI-026 external). They use `MediaOrigin::Generated` with a `GeneratedKind`. Mattes are not among them (no `GeneratedKind::Matte`). This spec proposes `Inpainted` and `Colorized` kinds.
  - **Task list (PM-073):** AI-001's analysis pool reports its rows into PM-073's single list; the pool keeps its own concurrency settings.
  - **Search (PM-054):** AI-027 adds the Spoken words chip and merges its transcript hits into PM-054's results.
  - **Bins (PM-052):** AI-037 proposes bins and commits them with PM-052's create-bin and move ops. It needs the capture date, location and device tags as optional fields of PM-035's probe data, which PM would add.
  - **New project (PM-002):** AI-016's standalone mode creates a project at the target ratio. AI-036's Mark ranges output adds sub-ranges to media items, like AI-004's scene markers.
  - AI-001's substitute chain (AI-022 → AI-028 → AI-019) is expressed through PM-041's `parent` links.
  - Also owns proxies, relinking, scene markers on media items, canvas aspect changes (which trigger AI-016), and media metadata.
- **preview-and-export.md (EX):**
  - Export waits for or runs missing analyses (AI-001) and always uses cached results, so the output matches the preview. FreeCut never blocks export while a smart tool is processing, as the reference editor does for reframe, relighting and object removal; it waits instead.
  - Preview quality settings may use half-resolution mattes and flow. AI-027's snippet preview uses EX-015's media preview mode.
  - EX-036's preflight can list low-resolution clips and offer AI-019's **Enhance low-resolution clips…** as an action.
  - AI-018's hint about raising the project frame rate compares it with EX-026's output frame rate.
- **workspace-and-shortcuts.md (UI):** UI-040 owns the background-task indicator and all toasts; PM-073 owns the task list it opens. UI also owns the dock placement of the Transcript panel and AI-035's Health panel (UI-056 auxiliary panels), the Smart tools menu (which gains Check project, Find clips in long recordings, Smooth all slowed clips and Enhance low-resolution clips), and every key binding used here (e.g. Delete in the Transcript panel). AI-001 contributes only job-kind labels, the shared job states, per-clip progress strips, result badges and the compare eye. AI-032's and AI-034's presets and AI-037's grouping presets are stored through UI-018's preferences storage.

## Out of scope (and why)

- **Any cloud processing, account, credits or online model service.** FreeCut runs locally only. (Proposed exception, pending the owners' decision: [cloud-generation.md](cloud-generation.md) lets a user connect their own Higgsfield account, opt-in, with a cost confirmation per run; its clip actions point back to the local tools here first.) Model downloads come from upstream open-source release pages and only after the user explicitly confirms (AI-002).
- **Usage limits, paywalls and server plumbing around smart tools.** The reference editor puts free-use counters, paid tiers, upload-permission prompts, server queues with wait estimates, "service busy" and network-error states on tools such as background removal, filler-word removal, quality enhancement, super resolution, the highlight clipper and media search. FreeCut has none of these: every tool is free, nothing is uploaded, and the only queue is AI-001's local one with ETAs measured on the user's machine.
- **Bundling models in the installer.** They are too large (hundreds of MB to GB). They stay optional downloads, so the base app remains small.
- **Voice cloning, voice swapping, dubbing or video translation in the speaker's own voice, transcript-driven re-speaking (editing words so the voice and lips are regenerated), lip-sync, talking photos and generated presenter avatars.** These carry a high risk of misuse (impersonation), most capable models have non-commercial or unclear licenses, and they need heavy compute. Local substitutes: caption translation (AI-013) and plain narration from open TTS voices (AI-025).
- **Script-to-video, one-click "packaging" in a chosen style, and ranking "best moments" with a language model against a written brief.** These need a capable language model to understand content; they stay out of this spec. The editing assistant that takes typed instructions, listed here before, is now specified in [assistant.md](assistant.md) (AS) as an opt-in, proposal-only tool whose macros (AS-012) reuse AI-003, AI-006, AI-010 and AI-011; ranking against a brief may be revisited there. Today's classic building blocks (AI-003/004/006/012) cover the mechanical parts. AI-029 is the deterministic, beat-synced montage built from them, and AI-036 cuts long recordings into candidate clips with a transparent, deterministic score. For packaging, auto captions (AI-010), keyword emphasis (TX-040) and FreeCut's own text styles (TX-019) cover the caption part locally. Generated intros and stock sound effects are not offered.
- **Visual content search (finding footage by the objects or people in it), and B-roll suggestions that match the speech.** Searching by what is seen needs an image-text embedding model and a per-frame index of every file. The open candidates differ widely in weight and training-data licenses, and none has been checked for the manifest. Searching by person means recognizing and grouping faces across the user's whole library, which is biometric processing that FreeCut does not want to do by default. Matching B-roll to speech needs the same embedding model, applied to the user's own library, since FreeCut has no online stock or web search. Spoken-word search (AI-027) covers the dialogue part locally. We may revisit with a clearly licensed embedding model and an explicit opt-in.
- **Grouping media by recognized people.** For the same biometric reason, AI-037 groups by date, place, device and look, never by faces.
- **Artistic style transfer (repainting footage as a painting, cartoon or anime look).** Per-frame style models flicker on video, temporally stable video stylization needs heavy research models, and popular anime-style models carry non-commercial licenses (e.g. AnimeGANv2). CL's filters and LUTs and FX's effects cover stylized looks. Colorizing black-and-white footage is in scope (AI-028).
- **Bundled generative image, video and music models.** Common options carry non-commercial (e.g. CC-BY-NC) or use-restricted (RAIL-style) licenses, which conflict with our free-software, no-strings goal. AI-026 lets users plug in their own local tools.
- **Generative edits of an existing picture**, and their local substitutes:
  - Generated animated treatments around a cut-out subject: AI-014's matte plus FX-020's person effects (outline glow, trails, particles), all procedural.
  - Generated replacement backgrounds: AI-014's cutout over the user's own image or video, or TR-011–TR-013's canvas fill.
  - Prompt-based region regeneration (for example new clothing), several generated candidates per region, and describing in words what to remove: AI-022 erases only. Text-grounded selection would need an open-vocabulary segmentation model whose license has not been checked.
  - Hair restyling (length, curls, style): AI-033 offers color only.
  - Generated transitions and generated sound effects for a clip: FX's transitions and AU-031's sound effects library.
- **Forced visible "AI-generated" labels or watermarks on export.** FreeCut adds no watermark of any kind (EX-040). Where media came from a generator, AI-026 records the provenance in the media item's metadata instead.
- **Eye-contact correction, generating new picture area beyond the frame edges, body reshaping, and one-click beauty filter looks of the reference editor.** Eye-contact correction synthesizes new eye regions and changes what a person appears to do, and no permissively licensed model has been checked. Outpainting is generative. Body reshaping (waist, legs, weight) raises body-image concerns. The reference editor's beauty filter looks are its assets; AI-032 offers FreeCut's own retouch presets instead. Relighting, which an earlier draft listed here, is now in scope as AI-034, with a model-free Flat mode and an optional permissively licensed depth model.
- **A speaker-diarization engine (for now).** TX-042 (later) can assign speakers by hand and from imported WebVTT voice tags without it. No local diarization model with clearly permissive code and weights has been chosen yet; when one is, it becomes an AI-002 model pack that TX-042's **Detect speakers** uses.
- **Removing other people's watermarks or logos.** FreeCut does not advertise or tune any tool for this. AI-022's fixed-area mode is meant for the user's own footage, e.g. a camera date stamp.

## Clean-room notes

- **No CapCut models, presets or assets.** Every model is an open-source project downloaded from its own upstream release location. Every algorithm is either a public, documented technique implemented fresh in Rust or a permissively licensed library. Behavior was learned from public help pages, tutorials and reviews, and from the English UI labels of a locally installed copy of the reference editor (desktop 9.5.0), taken from its bundled English localization file (see Provenance in the [README](README.md)). Those labels were paraphrased into a behavior inventory of options, ranges, limits and error states. Nothing was decompiled, disassembled or traced. No assets, models, presets or strings were copied, and none of the reference editor's online services were used.
- **Our own names and data.** Camera-motion levels (Calm / Normal / Fast), camera-move styles (AI-030), quality tiers, retouch and relight presets (AI-032, AI-034), default tint colors (AI-033), health-check groups and thresholds (AI-035), segment scoring weights (AI-036), bin names (AI-037), filler-word lists, keyword heuristics, default values and all UI wording are FreeCut's own. No preset names, labels or icons are copied, and observed option lists are recorded only as generic categories and numbers.
- **Icons and UI artwork** (task badges, beat-grid controls, transcript markers, face thumbnails' frames) are drawn by FreeCut or taken from an open icon set with a compatible license (e.g. Lucide, ISC; Phosphor, MIT).
- **TTS voices:** only voices whose license allows redistribution and commercial use (Apache-2.0 / MIT / CC0 / CC-BY with attribution). Non-commercial voices are excluded from the manifest.
- **License table** (all must be shown in Help → About → Third-party licenses when installed; weight licenses marked "verify" must be confirmed before a model enters the manifest):

  | Task | Candidate | Code license | Weights | GPL-3.0+ compatible |
  |---|---|---|---|---|
  | Speech recognition | whisper.cpp (via the `whisper-rs` binding, Unlicense) + Whisper models in ggml form | MIT | MIT (model card checked 2026-10-13) | yes |
  | Voice activity | Silero VAD, ggml form run by whisper.cpp (`ggml-org/whisper-vad`) | MIT | MIT (model card checked 2026-10-13) | yes |
  | Person matting | Robust Video Matting | GPL-3.0 | verify | yes |
  | Portrait matting | MODNet | Apache-2.0 | Apache-2.0 | yes |
  | Any-subject cutout | BiRefNet | MIT | verify | yes |
  | Promptable segmentation / video tracking | SAM 2 | Apache-2.0 | Apache-2.0 | yes |
  | Image inpainting | LaMa | Apache-2.0 | verify | yes |
  | Frame interpolation | Practical-RIFE / FILM | MIT / Apache-2.0 | MIT / Apache-2.0 | yes |
  | Super-resolution | Real-ESRGAN | BSD-3-Clause | verify | yes |
  | Stem separation | Demucs / Open-Unmix | MIT / MIT | verify | yes |
  | Speech enhancement | DeepFilterNet | MIT or Apache-2.0 | verify | yes |
  | Colorization (video) | DeOldify (archived; own ONNX export) | MIT | MIT | yes |
  | Colorization (stills) | DDColor | Apache-2.0 | verify | yes |
  | Face landmarks & detection | MediaPipe Face Landmarker / face detector | Apache-2.0 | verify model cards | yes |
  | Face detection (alternative) | OpenCV Zoo YuNet | MIT (per model README) | MIT (verify) | yes |
  | Person detection (reframe) | MediaPipe object detector, EfficientDet-Lite | Apache-2.0 | verify model card | yes |
  | Depth estimation (relight) | Depth Anything V2 Small / MiDaS | Apache-2.0 / MIT | Apache-2.0 (Small only) / MIT | yes |
  | Hair segmentation (hair tint) | MediaPipe hair segmentation | Apache-2.0 | verify model card | yes |
  | Text-to-speech | Kokoro-82M / Piper | Apache-2.0 / MIT (archived) + GPL-3.0 (current) | Apache-2.0 / per-voice | yes (per voice) |
  | Translation | Argos Translate + OPUS-MT | MIT | CC-BY-4.0 (attribution) | yes |
  | Scene detection reference | PySceneDetect (algorithm reference) | BSD-3-Clause | n/a | yes |

  Stabilization (vid.stab, classic optical flow) is listed in clip-transform-and-speed.md, which owns it. Beat tracking (AU-025) and noise reduction (AU-018) are model-free DSP owned by audio.md. Transcript search (AI-027), auto montage (AI-029), camera moves (AI-030), planar tracking (AI-031), the health check's detectors (AI-035), segment scoring (AI-036) and media grouping (AI-037) use no model.
- **Excluded because of license:** non-commercial models (e.g. ProPainter, E2FGVI, CoTracker, madmom's trained models, AnimeGANv2 and similar style models, CC-BY-NC alignment checkpoints, the Base and Large Depth Anything V2 checkpoints, many music generators), RAIL-style use-restricted generative models, and AGPL detector builds (they would put network-use obligations on users and contributors).
- **Auto montage (AI-029)** ships no templates, music or transition presets of the reference editor. It uses the user's own clips and music, FreeCut's own transitions (FX), and FreeCut's own default values.
- **Inference runtimes:** ONNX Runtime via the `ort` crate (MIT/Apache-2.0), or pure-Rust runtimes (`tract`, `candle`, `burn`; MIT/Apache-2.0). whisper.cpp goes through Rust bindings. No `unsafe` in FreeCut's own analysis code.

## Sources

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- How to Remove Background in CapCut (CapCut resource) — https://www.capcut.com/resource/how-to-remove-background-in-capcut
- How to remove background in CapCut: the ultimate guide (Miracamp) — https://www.miracamp.com/learn/capcut/how-to-remove-background
- How to Remove Background in CapCut (Kevin Stratvert) — https://kevinstratvert.com/2024/10/25/how-to-remove-background-in-capcut/
- Auto Reframe Video (CapCut tools page) — https://capcut.com/tools/auto-reframe
- Top 9 Tools to Automatically Reframe Videos (CapCut resource) — https://www.capcut.com/resource/best-video-reframers
- How to Effortlessly Edit Videos with Text (CapCut resource) — https://capcut.com/resource/edit-video-with-text
- 7 CapCut Video Editing AI Features (Primal Video) — https://primalvideo.com/video-creation/editing/capcut-video-editing-ai-features
- CapCut free AI tool walkthrough, transcript (GoTranscript) — https://gotranscript.com/public/effortless-video-editing-with-capcuts-free-ai-tool-a-step-by-step-guide
- Does CapCut Remove Silence Automatically? (BlitzCut, competitor) — https://blitzcutai.com/blog/capcut-remove-silence
- Auto-Remove Silences and Filler Words (FreeAcademy lesson) — https://freeacademy.ai/lessons/ai-video-editing-remove-silences
- AI-Powered Filler Word Remover (CapCut tools page) — https://www.capcut.com/tools/filler-words
- Quickly Split Scenes (CapCut tools page) — https://www.capcut.com/tools/split-scene/
- Scene detection guide (CapCut ideas page) — https://www.capcut.com/ideas/seedance-2-0-for-scene-detection
- How to Use Auto Cut? (CapCut help) — https://www.capcut.com/help/how-to-use-auto-cut
- Motion Tracking in CapCut: Complete Guide (Miracamp) — https://www.miracamp.com/learn/capcut/motion-tracking
- Camera Tracking guide (CapCut resource) — https://capcut.com/resource/motion-camera-tracking
- How to Add Music to CapCut and Beat Matching (TourBox) — https://www.tourboxtech.com/en/news/how-to-add-music-to-capcut.html
- How to Add Music and Sync It with Cuts in CapCut Desktop (NotJustOk) — https://notjustok.com/article/how-to-add-music-and-sync-it-with-cuts-in-capcut-desktop/
- How to Do Optical Flow in CapCut (CapCut resource) — https://www.capcut.com/resource/optical-flow-in-capcut
- Upscale Videos to 4K on Desktop and Mobile (CapCut resource) — https://www.capcut.com/resource/upscale-video-to-4k
- Remove Objects from Video guide (CapCut resource) — https://www.capcut.com/resource/how-to-remove-objects-from-video
- CapCut Stabilizer guide (CapCut resource) — https://capcut.com/resource/capcut-stabilizer
- Free AI Voice Isolator (CapCut resource) — https://www.capcut.com/resource/voice-isolator
- How to Recognize Bilingual Subtitles? (CapCut help) — https://www.capcut.com/help/bilingual-subtitles
- How Do I Fix Inaccurate Auto-Captions in CapCut? (CapCut help) — https://www.capcut.com/help/auto-captions-in-capcut
- Video Caption Generator guide (CapCut resource) — https://www.capcut.com/resource/video-caption-generator
- CapCut Auto Captions: How to Add & Style Subtitles (Typito) — https://typito.com/blog/capcut-auto-captions-add-subtitles-in-capcut/
- CapCut Auto Captions: Setup, Edit, And Export (SocialRevver) — https://socialrevver.com/blog/capcut-auto-captions
- CapCut captions alternatives comparison (VexaScribe, competitor) — https://vexascribe.com/alternatives/capcut-captions
- How to Retouch Faces in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-retouch-faces-in-capcut-the-complete-guide--cms-108852
- Retouch your face in CapCut (MiniTool) — https://moviemaker.minitool.com/news/retouch-your-face-in-capcut.html
- Add Auto Reframe effect to a sequence (Adobe Premiere help) — https://helpx.adobe.com/dk/premiere/desktop/add-video-effects/commonly-used-effects/add-auto-reframe-effect-to-a-sequence.html
- DaVinci Resolve 18.6 manual, Smart Reframe / Scene Cut Detection (third-party mirror) — https://www.steakunderwater.com/VFXPedia/__man/Resolve18-6/DaVinciResolve18_Manual_files/part1253.htm
- Exploring Scene Cut Detection in Resolve and Premiere (Elements.tv) — https://elements.tv/blog/exploring-the-new-scene-cut-detection-features-of-davinci-resolve-and-adobe-premiere-pro/
- Comparing Auto Reframe in DaVinci Resolve and Premiere Pro (Elements.tv) — https://elements.tv/blog/comparing-the-auto-reframe-in-davinci-resolve-and-premiere-pro/
- PySceneDetect CLI documentation — https://www.scenedetect.com/docs/cli.html
- PySceneDetect repository — https://github.com/Breakthrough/PySceneDetect
- whisper.cpp repository — https://github.com/ggml-org/whisper.cpp
- Robust Video Matting repository — https://github.com/PeterL1n/RobustVideoMatting
- MODNet repository — https://github.com/ZHKKKe/MODNet
- BiRefNet repository — https://github.com/ZhengPeng7/BiRefNet
- SAM 2 repository — https://github.com/facebookresearch/sam2
- LaMa license file — https://raw.githubusercontent.com/advimman/lama/main/LICENSE
- Practical-RIFE repository — https://github.com/hzwer/Practical-RIFE
- Real-ESRGAN repository — https://github.com/xinntao/Real-ESRGAN
- Demucs repository — https://github.com/adefossez/demucs
- DeepFilterNet repository — https://github.com/Rikorose/DeepFilterNet
- Piper (archived) repository — https://github.com/rhasspy/piper
- Piper (current, GPL) repository — https://github.com/OHF-Voice/piper1-gpl
- Kokoro-82M model card — https://huggingface.co/hexgrad/Kokoro-82M
- MediaPipe Face Landmarker guide — https://developers.google.com/edge/mediapipe/solutions/vision/face_landmarker
- vid.stab repository — https://github.com/georgmartius/vid.stab
- Desktop video editor product page (CapCut tools page) — https://www.capcut.com/tools/desktop-video-editor
- AI video enhancer (CapCut tools page) — https://www.capcut.com/tools/ai-video-enhancer
- Photo colorizer (CapCut tools page) — https://www.capcut.com/tools/photo-colorizer
- DeOldify repository — https://github.com/jantic/DeOldify
- DDColor repository — https://github.com/piddnad/DDColor
- AnimeGANv2 repository (license reference) — https://github.com/TachibanaYoshino/AnimeGANv2
- Depth Anything V2 repository (per-checkpoint licenses) — https://github.com/DepthAnything/Depth-Anything-V2
- MiDaS repository — https://github.com/isl-org/MiDaS
- MediaPipe image segmenter guide (hair segmentation model) — https://ai.google.dev/edge/mediapipe/solutions/vision/image_segmenter
