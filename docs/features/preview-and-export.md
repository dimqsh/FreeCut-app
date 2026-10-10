# Preview player & export

> Status: draft · Owns: playback transport and clock policy (play/pause, J/K/L shuttle, loop, frame stepping buttons), scrub/seek frame delivery, offline/error slates, preview resolution, playback priority and render cache, viewer zoom/fullscreen/rulers/guides/safe areas, frame snapshot, the export dialog, the export settings model and export presets, the export job pipeline, its failure catalogue and queue, output size/frame-rate/codec/container/rate-control/color tagging, export range resolution, per-clip export, export preflight, the export cover and the cover designer (cover composition) · Crates: freecut-app, freecut-render, freecut-media, freecut-audio, freecut-timeline, freecut-project

## Summary

This area covers watching the edit and turning it into a file. The player shows
the composited frame at the playhead. It plays in sync with sound, steps frame
by frame, shuttles with J/K/L, loops a range and goes fullscreen. It can also
show rulers, safe-area and framing guides and save the current frame as an
image.
Export renders the timeline, or an in/out range, through the same compositor
the preview uses. It writes MP4 (H.264/AAC) in the MVP; later releases add
HEVC, AV1, VP9, ProRes, MOV/MKV/WebM, GIF, audio-only files, captions and
cover art. Every export setting is visible, with no tiers, no watermark, no
account and no uploading. FreeCut writes a local file and shows you where it
is.

### Conventions used by every feature below

- **Time.** All playback and export times are rational `Tick` values on the
  sequence frame grid, as defined in `timeline-editing.md` (TL-001). Output
  frame `n` of an export with output rate `R` and range start `S` is evaluated
  at `t = S + n × (1/R)`. This is computed exactly in ticks. Floats appear only
  for display (seconds, percentages, ETA) and for non-time quantities such as
  bitrates computed from a formula and then stored as integer kbps.
- **One pipeline.** Preview and export both render through the per-frame
  pipeline FX-001 (`effects-transitions-animations.md`). Preview may render at
  a lower resolution, may drop frames and may use proxies. It never uses a
  different look. Export never drops frames and uses original media at output
  resolution. The exceptions are EX-051 draft mode, and a proxy standing in
  for a missing original after the EX-036 warning (PM-081).
- **Threads.** The UI thread only sends requests and shows finished textures.
  Decoding (`freecut-media`), rendering (`freecut-render`), mixing
  (`freecut-audio`) and encoding (`freecut-media`) run on worker threads.
  Export runs as a job (EX-022).
- **Player state is view state.** Transport, zoom, guides, preview resolution
  and fullscreen are not edits. They have no undo entry. The ones worth keeping
  are saved in the project's UI section, so they come back on reopen, and they
  never mark the project as needing a save on their own (FreeCut default).
- **Export settings** are saved with the project when an export starts (see
  EX-021). They are not undoable edits, because they do not change the
  timeline.
- **Keys.** The default keymap is UI-031 (`workspace-and-shortcuts.md`), and
  it is canonical. This spec names commands and writes "see UI-031" for their
  keys; a key quoted here only repeats UI-031's binding. A command that
  UI-031 lists as unbound by default, or does not list, has no key until the
  user assigns one (UI-032).
  The behaviour of arrow keys, Home/End and cut-point jumps is defined in
  TL-013.
- **Color space.** SDR Rec.709 (CL-002) is the single working and output
  space. HDR output is out of scope (see Out of scope).
- **Reference notes.** "Reference" lines record behaviour of the reference
  desktop editor seen in public pages. "Reference (observed)" lines come from
  a paraphrased inventory of its English UI labels (desktop 9.5.0, a local
  install); labels show which options, limits and refusals exist, not always
  their defaults. "Alternative seen in the reference app" marks, in one line,
  a behaviour FreeCut deliberately does differently. "Reference (unverified)"
  marks details we could not confirm (see Sources).
- **New commands without keys.** Commands this spec adds that UI-031 does not
  list yet (for example Go to clip start, Show rulers) have no default key
  until UI-031 lists one or the user assigns one (UI-032).

## Features

### EX-001 Player panel and viewer — mvp

**What the user sees:** A preview panel shows the project canvas. Its aspect
comes from project settings (PM). The canvas is centered and letterboxed in a
neutral, theme-dependent surround. Below it is a toolbar with:
- current time and total duration (`HH:MM:SS:FF`, format from TL-002);
- transport buttons (EX-003);
- a play/pause button;
- on the right, a menu holding preview resolution (EX-006), playback
  priority (EX-056, v1), guides and rulers (EX-011, EX-055), zoom (EX-009),
  snapshot (EX-014) and fullscreen (EX-010).

A small chip above the canvas names what the player is showing: "Timeline",
the compound clip's name while one is open (TL-040), or "Media: <name>" in
media preview mode (EX-015). On-canvas handles from TR-007 are drawn on top
of the viewer.

**Inputs / controls:** None beyond the toolbar. Clicking the viewer selects the
topmost clip under the pointer (TR-007). The panel can be resized and docked
(UI). Right-clicking the time readout offers the time display style
(Timecode, Seconds or Frames); the setting itself is UI-019's and applies
app-wide, and drop-frame labels are TL-002's (v1).
- Reference (observed): the reference player shows the time as hours,
  minutes and seconds plus a frame number with a selectable display style,
  separate "loading" and "failed to load" states, and refuses to preview
  content whose duration is 0.

**Behavior / output:**
- The viewer always shows the frame at the playhead (TL-013), or the hover-skim
  frame (TL-014) while skimming.
- The canvas is drawn to fit the panel (EX-009 default "Fit"). Rendering targets
  physical pixels on HiDPI displays.
- Empty timeline: the canvas shows the background color with a quiet hint to
  add media. Duration is 00:00:00:00 and transport controls are disabled.
- A frame that is still rendering keeps showing the previous frame. FreeCut
  never shows a blank flash. If a request takes more than 250 ms, a small
  spinner appears in the corner (FreeCut default).
- **Loading state:** right after a project opens, until the first frame is
  ready, the canvas shows the background color with "Preparing preview…"
  (after the same 250 ms delay). Editing is possible meanwhile.
- **Unavailable state:** if the preview renderer cannot start (no compatible
  GPU adapter, or device creation fails), the panel shows a plain error
  panel, "Preview unavailable", with the reason, **Retry** and **Restart in
  safe mode** (UI-024). It never shows a frozen last frame. The timeline,
  inspector and saving keep working; export is disabled with the same reason.

**Edge cases:**
- Panel smaller than 64×36 px: the viewer stops rendering and shows a "too
  small" hint, to save GPU time.
- Canvas aspect changed (PM/EX-013): the letterbox recomputes instantly. Guides
  and handles follow.
- Very tall or very wide canvases (for example 1:4 or 4:1) still fit. The
  minimum drawn canvas dimension is 16 px.
- The GPU device is lost (driver reset or sleep): the viewer shows the error
  slate (EX-005), recreates the device and re-requests the frame. If the
  device is lost 3 times within 60 s (FreeCut default), automatic recovery
  stops and the unavailable state above appears, naming the graphics driver
  as the likely cause, instead of a black or frozen preview.
- Zero-length content (an empty compound clip, or a media item whose probe
  reports no frames): "Nothing to preview (duration is 0)" instead of a blank
  canvas; the transport is disabled.

**Undo:** n/a (view state).

**Model notes:** `freecut-app::player::PlayerPanel` (egui) shows a texture
produced by `freecut-render::preview::PreviewRenderer` on a shared wgpu
device (egui-wgpu). The panel never decodes or renders on the UI thread. It
holds only a `TextureId` and the frame's `(seq_version, Tick)` tag, plus a
`PlayerHealth { Loading, Ready, Unavailable(Reason) }` state; the
device-loss counter that moves it to `Unavailable` is a pure, tested helper.

### EX-002 Play / pause and the playback clock — mvp

**What the user sees:** Space or the play button starts playback from the
playhead, with sound in sync. Pressing either again pauses on the frame
showing at that moment. At the end of the timeline playback stops on the last
frame. Pressing Space there restarts from 0.

**Inputs / controls:**
- Play/pause: Space (see UI-031).
- Preference "When playback stops, return the playhead to where it started":
  default off (FreeCut default).
- Preference "Auto-scroll timeline during playback": page or continuous
  (owned by TL-013).

**Behavior / output:**
- The **audio output clock is the master** (AU-010). The player turns played
  audio samples into ticks exactly:
  `t = start + samples_played × timebase / sample_rate`, computed with integer
  math. Video presents the newest rendered frame whose time is ≤ `t`, minus
  the reported output latency.
- With no audio device, a monotonic system clock drives playback (AU-010
  banner).
- **Drop policy:** if rendering falls behind, video frames are skipped and
  audio is never stretched or paused. The renderer always works on the frame
  for "now + pipeline latency", never on stale ones.
- **Buffering:** if no new frame has arrived for more than 1 s (for example
  slow network storage), the player pauses the clock and audio and shows
  "Buffering…". It resumes when 0.5 s of frames are ready ahead (FreeCut
  defaults).
- Decode read-ahead starts at the playhead when playback is requested.
  Start-up latency target: under 150 ms for cached or proxy media.
- Hidden tracks render nothing and muted tracks are silent. These are TL-008's
  two track flags (mvp; persisted and undoable through `SetTrackFlags`), and
  playback reads them from the document like any other edit. Toggling one
  during playback takes effect from the next rendered frame and audio buffer.
  Track lock (TL-009, v1) has no effect on playback. Solo follows AU-012.
- Playback presents at the display refresh rate. A 24 fps timeline on a 60 Hz
  display uses an uneven 3:2 cadence, which is accepted.

**Edge cases:**
- Playhead exactly at `duration`: Play jumps to 0 first.
- Edits during playback: the document version changes. Read-ahead after the
  playhead is discarded, and playback continues showing the new content from
  the next rendered frame. Audio is re-planned within one buffer.
- Playing inside an open compound clip (TL-040) plays that nested sequence
  only.
- Timeline mixes frame rates or has VFR sources: the frame mapping follows
  FX-001 L1. Playback is still clocked by the sequence grid.
- NTSC timelines (29.97, 59.94): sync never depends on the timecode display.
  The audio clock and exact rational frame times keep picture and sound
  together whether drop-frame labels (TL-002) are on or off. Alternative
  seen in the reference app: drop-frame mode is presented as the way to stop
  preview audio drifting at those rates.
- The system sleeps or the lid closes during playback: playback stops. On wake,
  the device is re-opened and the transport stays paused.
- Pressing Space while a modal dialog has focus does nothing. Space in a text
  field types a space (UI focus rules).

**Undo:** n/a (view state). The playhead itself is TL view state.

**Model notes:**
- `freecut-app::player::Transport { state: Stopped | Playing { rate: ShuttleRate, started_at: Tick } | Scrubbing }`.
- `ShuttleRate { num: i32, den: u32 }` is rational. 1× is `1/1`.
- The clock trait `PlaybackClock { fn now(&self) -> Tick }` has an audio-backed
  implementation from `freecut-audio` and a monotonic fallback.
- `freecut-render` keeps a small queue of rendered frames tagged
  `(seq_version, Tick)`. `freecut-media` owns the decoder pool and its
  read-ahead.
- Unit tests in `freecut-app` can drive the transport with a fake clock.

### EX-003 Transport controls, frame stepping and go-to-time — mvp

**What the user sees:** Next to play/pause are buttons for: go to start,
previous frame, next frame, and go to end. From v1, clicking the
current-time readout turns it into a field where you can type a time and
press Enter to jump.

**Inputs / controls:**
- Buttons: |◀ start, ◀| previous frame, |▶ next frame, ▶| end. Their keys
  are in UI-031; the moves they make (single frames, 10-frame steps, cut
  points, start/end) are defined in TL-013.
- The typed go-to-time field is v1. The MVP ships the buttons and a
  read-only readout. The "Go to timecode…" command focuses the field (key:
  see UI-031). The field accepts:
  - `HH:MM:SS:FF`;
  - `MM:SS:FF`;
  - `SS:FF`;
  - a plain number of frames, prefixed with `#` (for example `#240`);
  - a relative offset `+N` or `-N` in frames, or `+MM:SS:FF`.
  The field uses the same display format as TL-002.
- Jumps to the current clip's first and last frame, and revealing the clip in
  the timeline, are EX-054 (v1).
- Reference (observed): the reference player's labels cover play/pause,
  single-frame steps, jumps to the current clip's first and last frame, and a
  "locate" action that scrolls the timeline to the previewed clip.

**Behavior / output:**
- Frame step moves exactly one sequence frame. Holding the button repeats at
  the key-repeat rate. Each repeat requests an exact frame (EX-004).
- Stepping while playing pauses first. It then steps from the frame on screen.
- Typed times are quantized to the frame grid (TL-001) and clamped to
  `[0, duration]`.

**Edge cases:**
- Invalid typed time (for example `00:61:00:00` or letters): the field shakes
  and reverts. Nothing moves.
- `FF` value at or above the frame rate (for example `:30` at 30 fps): invalid.
- Drop-frame display (TL-002) on 29.97/59.94: typed drop-frame timecodes are
  parsed with the drop-frame rule. Non-existent labels (for example
  `00:01:00;00`) snap to the next valid frame.
- Stepping at 0 or at `duration` does nothing (no wrap).
- Empty timeline: all transport controls are disabled.

**Undo:** n/a (view state).

**Model notes:** Timecode parsing and formatting are pure functions in
`freecut-timeline::time` (`parse_timecode(&str, FrameRate, DropFrame) ->
Result<Tick, TimecodeError>`) and unit-tested there. The buttons live in
`freecut-app`.

### EX-004 Scrub and seek frame delivery — mvp

**What the user sees:** Dragging the playhead, clicking the ruler, stepping
frames or skimming (TL-014) updates the viewer smoothly. Fast drags show
recent frames without lag. When you stop, the exact frame appears quickly.

**Inputs / controls:** None (behaviour of TL-013/TL-014 interactions).

**Behavior / output:**
- **Latest-wins requests.** Each pointer move replaces the pending frame
  request. Requests that have not started are dropped. A request in progress
  finishes only if it is still the newest.
- **Two-step delivery.** During a fast drag the renderer may decode only the
  nearest keyframe (I-frame) at or before the target, and shows that
  approximate frame at once. When the pointer has been still for 80 ms, or the
  drag ends, the exact frame is decoded and shown. Target: exact frame within
  150 ms for 1080p long-GOP sources on a mid-range machine (FreeCut goal).
- Requests are throttled to the display refresh rate.
- Decoded frames go into the frame cache (EX-016). Scrubbing back over a
  region is served from it.
- Audio scrubbing is defined in AU-016. It uses the same playhead positions.

**Edge cases:**
- Sources with very long GOPs (screen recordings with a keyframe every 10 s
  and more): the approximate frame may be visibly off. The exact frame
  arrives after the pause. FreeCut suggests proxies (PM-080) once per session
  for such files (FreeCut default).
- All-intra sources (ProRes, MJPEG, images): always exact.
- Scrubbing over offline media shows the slate (EX-005).
- Scrubbing past the end shows the last frame.
- A document change while dragging (rare, from background jobs): the pending
  request is re-tagged with the new version.

**Undo:** n/a.

**FreeCut mvp implementation (2026-10-12):** the app's preview thread (`freecut-app/src/preview.rs`) renders with `freecut_render::Compositor`, the same CPU renderer export uses (architecture C44), fed by the shared `freecut_render::media::MediaImages` (one decoder per asset, `SeekMode::Exact`). Requests stay latest-wins with a generation number; a new one goes out when the document revision, playhead, preview size, offline set, font registry generation or the inspector's uncommitted text draft (TX-002) changes. Each frame comes back with the box of every visible clip (`LayerBox`: matrix to canvas pixels and size; text boxes measured by the compositor's own text renderer), which the player uses for hit-testing and outlines (TR-007). Two-step delivery, throttling and the frame cache are not built yet.

**Model notes:**
- `freecut-render::preview::FrameRequest { seq_version: u64, time: Tick, scale: PreviewScale, mode: Exact | Approximate }`.
- `freecut-media::decode::DecoderPool::seek(source, Tick, SeekMode)` returns
  frames. Keyframe indexes are built at probe time (PM/media).

### EX-005 Offline, missing and error slates — mvp

**What the user sees:** Where a clip's media file is missing, the viewer shows
a dark slate. It says the media is offline and gives the file name. Where a
frame cannot be decoded, a similar slate says the frame could not be read.
The rest of the composite (other tracks, text) renders normally. Exports use
the same slates, after a warning (EX-036).

**Inputs / controls:** None. Relinking is PM's job (PM-091), and the slate's
context menu offers "Relink…".

**Behavior / output:**
- Offline detection and media status belong to PM-090. EX owns the slate and
  the export rules for offline media.
- The slate is drawn procedurally by `freecut-render`: solid dark gray, a
  diagonal hatch, and two lines of text in the UI font. It is not a bitmap
  asset. It fills the clip's layer rectangle, so transform, crop and mask
  still apply. That makes it obvious which clip is missing.
- Audio from offline media is silent (AU-010).
- Decode errors are logged once per source and time range, not per frame.

**Edge cases:**
- Media goes offline during playback (drive unplugged): the slate appears from
  the next frame. Playback continues.
- Media returns (re-plugged): the slate clears without restarting, because PM
  notifies a media-state change (PM-090) and the frame cache is invalidated
  for that source.
- Proxy available but original missing: the preview uses the proxy, with
  PM-081's "Original offline" badge. Export warns that the proxy will stand
  in, offers Relink…, and marks the export "contains proxy media" (EX-036,
  PM-081).
- Text in the slate is the file name only. Full paths are never burned into
  exports.

**Undo:** n/a.

**Model notes:** `freecut-render::slate::{offline_slate, error_slate}` generate
the layer image for FX-001 L1. Media status comes from PM-090's runtime
`MediaStatus`.

### EX-006 Preview resolution and paused full-quality refresh — mvp

**What the user sees:** A "Preview resolution" choice in the player menu:
Auto, Full, 1/2, 1/4. Lower settings make heavy timelines play smoothly. When
playback stops, the frame on screen is re-rendered at full quality, so still
frames always look sharp (FreeCut default).

**Inputs / controls:**
- Playback resolution: **Auto** (default, labelled "recommended"), Full,
  1/2, 1/4.
  - Auto renders at the smaller of the canvas size and the viewer's
    physical-pixel size.
  - Full renders at canvas size even when the viewer is smaller.
  - 1/2 and 1/4 scale the canvas size by that factor.
- A one-line note under the choice: "Changes only the preview. Exports always
  render at full quality."
- "Sharpen when paused": default on.
- Whether Auto may step down by itself during playback is EX-056 (v1).
- PM-081's "Use proxies" toggle is also listed in this menu once proxies ship
  (v1). Its default and behaviour are defined there.
- Reference: the reference editor's help pages advise turning down preview
  playback quality when a heavy project stutters, and point out that a
  laggy preview does not mean the exported file will lag. Kdenlive and
  Shotcut both offer preview scaling.
- Reference (observed): the reference player offers Full, 1/2 and 1/4 with
  one level marked as recommended, plus help text saying export is
  unaffected. One reading of its labels shows four named levels instead of
  three; FreeCut keeps Auto plus three fixed factors, which covers both. Its
  slow-playback prompt, which can be silenced per project, is EX-017's hint.

**Behavior / output:**
- The render target size is `canvas × factor`, rounded to even pixels, with a
  minimum of 64 px on the short side. Every spatial parameter is
  resolution-independent (FX-001), so the image looks the same, only softer.
- Text, shapes and vector stickers are rasterized at the render-target size.
- A setting change applies from the next frame. It does not invalidate the
  RAM frame cache, because cache entries are keyed by scale.
- Never affects export or snapshots (EX-014).

**Edge cases:**
- Auto on a 5K display with a 1080p canvas: renders at 1080p (canvas cap).
- 1/4 on a 480p canvas: clamped to the 64 px minimum.
- Viewer zoomed above Fit (EX-009) while paused: the paused refresh renders at
  the zoomed size, capped at Full.
- Preview-only switches from other areas (for example "Disable all color" in
  CL-023) are listed in this menu with a warning badge while they are active.

**Undo:** n/a (view state, saved in the project UI section).

**Model notes:** The saved choice is a plain serde enum
`PreviewScaleSetting { Auto, Full, Half, Quarter }` in
`freecut-project::ui::PlayerUiState`. `freecut-app` maps it to
`freecut-render`'s `PreviewScale`, so `freecut-project` never depends on the
render crate. The render-target size is a pure function
`preview_target_size(canvas, viewer_px, scale) -> (u32, u32)` and is
unit-tested.

### EX-007 J/K/L shuttle and variable-speed playback — v1

**What the user sees:** L plays forward and J plays backward. Pressing the
same key again speeds playback up. K pauses. Holding K and tapping J or L
steps one frame. A small badge in the viewer shows the current speed (for
example "◀◀ 4×").

**Inputs / controls:** Keys: J / K / L as bound by UI-031 ("Shuttle
backward / stop / forward"). EX-007 owns what the presses and chords do:
- L: forward. Repeated presses cycle 1× → 2× → 4× → 8× (FreeCut default; the
  maximum is 8×).
- J: reverse at the same steps (−1×, −2×, −4×, −8×).
- J while moving forward, or L while moving backward: switches direction at
  1×.
- K: pause.
- K+L or K+J: step one frame forward or back.
- Holding K, then holding L or J: slow play at 1/2× while held.
- "Shuttle forward / backward at 1/2×" commands: start at 1/2×. Keys: see
  UI-031.
- Play/pause (see UI-031) always means play at 1× or pause.
- Comparison: Kdenlive steps 1×, 1.5×, 2×, 3×, 5.5×, 10×. Shotcut maps
  J/K/L the same way, with K+J and K+L for frame steps.
- Reference (unverified): whether the reference desktop editor binds J/K/L is
  not confirmed by the public pages we read, and its observed UI labels list
  play/pause and single-frame steps but no shuttle commands. FreeCut follows
  the common NLE convention.

**Behavior / output:**
- The playback clock advances by `rate × elapsed`, in rational math.
- Forward rates above 1× render only the frames due at display cadence.
  Intermediate frames are skipped.
- Reverse playback decodes GOPs backwards. Each GOP is decoded forward into a
  bounded RAM cache (default 512 MB, FreeCut default) and presented in reverse.
- Audio: forward 2×–8× plays pitch-preserved time-stretched sound (AU-016,
  using AU-023's engine). 1/2× plays slowed, pitch-preserved sound.
  **Reverse shuttle is silent** (FreeCut default; reversed scrub audio may be
  revisited after user testing).
- Playback stops at 0 when going backward, and at `duration` when going
  forward (or loops, EX-008).

**Edge cases:**
- 8× on 4K long-GOP media: frames will drop heavily. The dropped-frame
  indicator (EX-017) shows it. Correctness is unaffected.
- Reverse over very long GOPs (300+ frames): the cache budget limits
  read-behind, and stutter is expected. FreeCut suggests proxies.
- Reverse into an offline clip shows the slate.
- Shuttle inside media preview mode (EX-015) uses the source's frame rate.
- Pressing L six times stays at 8×.

**Undo:** n/a (view state).

**Model notes:** `ShuttleRate` (EX-002) holds signed rational rates. The
rate-step table is a `const` in `freecut-app::player`. Reverse decoding is
`freecut-media::decode::ReverseReader` (GOP-wise). The time-stretch voice is in
`freecut-audio` (AU-016).

### EX-008 Loop playback and play range — v1

**What the user sees:** A loop toggle on the player toolbar. With loop on,
playback that reaches the end of the in/out range (TL-038), or the end of the
timeline when no range is set, jumps back to the start and keeps playing. A
separate "Play range" command plays the in/out range once and then stops.

**Inputs / controls:**
- Loop toggle: default off. Key: see UI-031 ("Toggle loop playback").
- Play range: key: see UI-031 ("Play range"). It is disabled without a
  range.
- Preference "Loop range follows selection": when on and no in/out marks
  exist, loop covers the span of the selected clips. Default off (FreeCut
  default).

**Behavior / output:**
- Loop region = the in/out range if set (with TL-038 rules for a missing In or
  Out), otherwise `[0, duration)`.
- If loop is on and playback starts outside the loop region, it starts at the
  region start (FreeCut default, as in Kdenlive's loop zone).
- At the loop end, audio and video both wrap. The loop start is pre-fetched,
  so the wrap gap is no more than one audio buffer (about 21 ms at the
  defaults).
- The loop end is exclusive: the last frame shown is the frame before Out.
  This matches export, which also excludes the Out frame.
- Shuttle at any rate loops in its own direction. In reverse, it wraps from
  the start to the end.

**Edge cases:**
- A range of 1–5 frames loops. Rapid repeats may stutter, which is accepted.
- The range changes while looping: the new bounds apply at the next wrap. If
  the playhead is now outside the range, it jumps to the new start.
- The range lies wholly beyond `duration`: loop falls back to the whole
  timeline, with a toast.
- Edits during looping follow EX-002.

**Undo:** n/a (view state; the loop toggle is saved in the project UI section).

**Model notes:** `LoopMode { Off, Range, Timeline }` in `freecut-app::player`.
The pure helper `loop_region(seq, RangeMarks, selection) -> Option<TimeRange>`
lives in `freecut-timeline` (shared with EX-035). The audio side gets the loop
region so its prefetch wraps (`freecut-audio`).

### EX-009 Viewer zoom and pan — v1

**What the user sees:** The canvas can be shown at Fit (default) or at a fixed
magnification. Above Fit, it can be panned to inspect detail or reach
off-canvas handles.

**Inputs / controls:**
- Zoom menu: Fit, 10 %, 25 %, 50 %, 75 %, 100 %, 150 %, 200 %, 400 %.
- Ctrl/Cmd + mouse wheel zooms around the pointer in steps of about 1.25×,
  within 5 %–800 %.
- Pinch zoom on trackpads.
- Pan: middle-drag, or Space+drag while paused (Space+drag only starts after
  a 150 ms hold, so a tap still plays).
- **Hand tool:** a hand button in the player toolbar, shown while zoomed above
  Fit, toggles pan mode. While it is on, a left-drag pans and clicks do not
  select clips or move handles (TR-007). It turns itself off when the zoom
  returns to Fit. Command "Hand tool (player)": no default key (see
  Conventions).
- "Fit" button or double-click on the zoom readout returns to Fit.
- Reference (observed): the reference player has a zoom control with a hover
  tooltip and a separate move (hand) tool for panning a zoomed canvas.
- Comparison: Shotcut's player offers Fit, 10/25/50/100/200 %. Kdenlive offers
  monitor zoom buttons.

**Behavior / output:**
- 100 % means one canvas pixel per physical display pixel.
- The area outside the canvas (the pasteboard) is drawn dimmed, so off-canvas
  parts of clips and their handles stay visible (TR-007).
- Zoom and pan are view-only. Guides (EX-011) and handles stay crisp.
- At zoom levels above the render resolution, the paused refresh (EX-006)
  renders a sharper frame.

**Edge cases:**
- Panel resize at Fit: stays at Fit. At a fixed zoom: keeps the same canvas
  point centered.
- Zoom above 100 % on a 1/4 preview during playback: the image is blocky until
  playback pauses.
- Pan is limited so that at least 10 % of the canvas stays visible.

**Undo:** n/a (view state; not persisted, so each session opens at Fit).

**Model notes:** `ViewerTransform { zoom: f32, pan: Vec2 }` and a
`hand_tool: bool` flag in `freecut-app::player`. Float is fine here because
this is display, not time.

### EX-010 Fullscreen preview — v1

**What the user sees:** The fullscreen button (or the shortcut, or a
double-click on the area around the canvas) shows only the canvas on a whole
monitor, on black. Moving the mouse briefly shows a minimal transport bar. Esc
returns to the editor.

**Inputs / controls:**
- Toggle: the "Full-screen preview" command (key: see UI-031), or a
  double-click on the letterbox or pasteboard around the canvas (FreeCut default; Kdenlive uses a viewer
  double-click). Double-clicks on clips keep their own meaning, such as text
  editing (TX-002). Esc exits.
- "Fullscreen on:" lets you pick a monitor when more than one is connected.
  Default: the monitor showing the main window.
- "Show guides in fullscreen": default off. It covers EX-011, EX-012 and
  EX-055's guides; rulers never show in fullscreen.
- The fullscreen button's tooltip names its current key (UI-039).
- Reference (observed): the reference player enters and leaves full screen
  from a player button or a shortcut, and the tooltip names the shortcut.

**Behavior / output:**
- The canvas fits the monitor, letterboxed in black. Preview resolution Auto
  follows the larger target.
- The transport overlay fades in on mouse move and hides after 2 s of
  stillness. The cursor hides with it.
- All playback commands (play/pause, shuttle, frame stepping, loop, play
  range; keys per UI-031) keep working. Editing shortcuts are inactive.
- Audio continues unchanged.

**Edge cases:**
- The fullscreen monitor is unplugged: fullscreen exits and the player returns
  to the panel.
- macOS separate Spaces: FreeCut uses a borderless window covering the
  display rather than the native fullscreen Space when another display is
  chosen (FreeCut default), so the editor stays usable on the other display.
- A modal dialog opening (for example an autosave error) exits fullscreen
  first.
- Pressing fullscreen with an empty timeline shows the background color.

**Undo:** n/a (view state).

**Model notes:** egui/eframe viewport API (`ViewportBuilder` with
fullscreen/borderless). The fullscreen viewport shares the preview texture,
so no second render pass is needed when sizes match.

### EX-011 Safe areas, grid and aspect guides — v1

**What the user sees:** A "Guides" submenu in the player turns on overlays:
title-safe and action-safe rectangles, a rule-of-thirds grid, a custom grid, a
center cross, and an aspect-ratio mask. The aspect mask outlines (and can
dim outside) a crop of another shape, such as 1:1 or 9:16, to check how the
framing would survive a re-crop. The overlays are thin lines drawn over the
canvas. They never appear in exports or snapshots.

**Inputs / controls:**
- Action safe: on/off. Inset default 93 % of the canvas (FreeCut default,
  following the commonly used modern broadcast values). Adjustable 50–100 %.
- Title safe: on/off. Default 90 %. Adjustable 50–100 %.
- Rule of thirds: on/off.
- Custom grid: columns × rows, each 2–20. Default 4×4.
- Center cross: on/off.
- Aspect mask: off / 1:1 / 4:5 / 16:9 / 9:16 / 4:3 / 2.39:1 / custom W:H.
  Outside dim opacity 0–100 % (default 50 %).
- Guide color: auto (contrasts with the theme), or a custom color.
- "All guides off" command: turns every guide off at once, including EX-055's
  user guides (it hides them; it does not delete them). Key: see UI-031.
- Rulers and freely placed guide lines are EX-055.
- Reference (observed): the reference player lists a grid option among its
  overlays; its exact layout is not shown by the labels.

**Behavior / output:**
- Guides are drawn in screen space at one physical pixel width (two on HiDPI),
  positioned in canvas space. They scale with zoom (EX-009).
- The aspect mask is the largest rectangle of that ratio centered in the
  canvas.
- Visible safe-area edges and grid lines act as snap targets for on-canvas
  dragging (TR-008).
- Text/caption areas use title safe (TX notes).

**Edge cases:**
- Aspect mask equal to the canvas ratio: draws nothing, and a hint says so.
- Very small viewer (< 200 px): grid lines are hidden to avoid clutter. Safe
  areas stay.
- Fullscreen: hidden unless "Show guides in fullscreen" is on (EX-010).
- Guides are never rendered by the export path, by design.

**Undo:** n/a (view state, saved per project in the UI section).

**Model notes:** `GuideSet` in `freecut-project::ui::PlayerUiState`. Pure
geometry (`safe_rect(canvas, pct)`, `aspect_mask_rect(canvas, ratio)`) lives
in `freecut-app::player::guides` with unit tests. Drawing uses the egui
painter over the viewer texture.

### EX-012 Vertical-feed UI zone overlay — v1

**What the user sees:** An optional overlay for portrait canvases. Shaded
zones mark where phone short-video apps usually place their own interface:
- a header band at the top;
- a caption and user-name band at the bottom;
- a column of buttons on the right.

A clear central box marks the area that stays unobstructed. This helps place
text and faces so app UI won't cover them.

**Inputs / controls:**
- Toggle in Guides (default off).
- **Zone layout** (FreeCut's own generic layouts, no app names):
  - **Standard** (default): the sizes listed below.
  - **Tall caption area:** top band 6 %, bottom band 30 %, right column 10 %
    wide spanning 35–80 %.
  - **Wide side buttons:** top band 8 %, bottom band 18 %, right column 16 %
    wide spanning 30–85 %.
  - **Custom:** editing any size switches to Custom and keeps the values.
- Zone sizes for Standard, as a fraction of canvas height or width (FreeCut
  defaults; band and column thickness adjustable 0–50 %):
  - top band 8 %;
  - bottom band 20 %;
  - right column 12 % wide, spanning 40–85 % of height;
  - central safe box at a 4:5 ratio.
- Overlay opacity 0–100 % (default 35 %).
- Reference: an article on the reference editor's site advises placing
  important captions and branding in a middle box of about 4:5 proportions on
  a 1080×1920 portrait frame.
- Reference (observed): the reference player offers "none" plus 3 overlays
  styled after particular short-video apps, with a note that real layouts
  differ by device. Alternative seen in the reference app: those overlays
  imitate each app's interface; FreeCut's three layouts are plain shaded
  zones with generic labels.

**Behavior / output:**
- Drawn as plain translucent rectangles with short generic labels (for example
  "app controls"). FreeCut ships no imitation of any app's interface, icons or
  layout artwork.
- Available for any canvas. It is suggested automatically (one-time hint)
  when the canvas is portrait and a platform preset for vertical video is
  chosen (EX-042).

**Edge cases:**
- Landscape canvas: the overlay still works, but a hint notes that it is
  designed for portrait.
- Apps change their layouts, so the zone sizes are editable and documented as
  approximate. The Guides menu shows a one-line note under the toggle: real
  layouts vary by phone, app version and language.
- A layout whose zones overlap (for example a bottom band of 50 % and a right
  column reaching 85 %): drawn as is; the overlap is simply darker.

**Undo:** n/a (view state).

**Model notes:** Part of `GuideSet` (EX-011), as `FeedZones { layout:
FeedLayout { Standard, TallCaption, WideSide, Custom }, sizes, opacity }`.
The built-in layouts are a `const` table. It is pure geometry and has no
assets.

### EX-013 Canvas ratio menu in the player — v1

**What the user sees:** A ratio menu on the player toolbar, so the canvas
shape can be changed without opening Project Settings. EX owns the menu; the
list of shapes, their hints and pixel sizes, and the operation are PM-003's
(including the wide cinema shapes, Custom… from PM-004, Auto from PM-006, and
"Original", a one-time match to the first main-track clip). Choosing one
reshapes the canvas immediately. Clips re-fit according to TR-002.

**Inputs / controls:** The menu, with exactly PM-003's entries.
- Reference: the reference editor's resizing page offers 16:9, 9:16, 1:1,
  4:3, 3:4 and 2:1 presets, and suggests 4:5 for one photo-sharing app's
  feed.

**Behavior / output:**
- This is a shortcut to PM's canvas operation (PM-003, PM-011). The resolution
  tier (short side) is kept, as PM-003 defines.
- Empty letterbox areas are filled by the canvas background (TR-011…TR-014).
- Guides, safe areas and the viewer letterbox update on the next frame.

**Edge cases:**
- Choosing the ratio the canvas already has: nothing happens and no undo
  entry is created.
- "Original" with an empty main track: disabled.
- "Original" with non-square pixels or rotation metadata: uses the display
  aspect after rotation and pixel-aspect correction (PM-035 probe).
- The player is in media preview mode (EX-015): the drop-down is disabled,
  because it changes the project canvas, not the source.
- Changing the ratio while a background export runs does not affect the job,
  which uses its snapshot (EX-038). While a blocking MVP export runs, the
  settings are read-only (PM-011).

**Undo:** PM's `SetProjectSettings { before, after }` ("Change aspect ratio",
PM-003). Undo restores the previous size and the fit results.

**Model notes:** The UI lives in `freecut-app`. The operation and its reverse
are defined in `project-and-media.md` (PM-003, PM-011). TR owns re-fit
behaviour.

### EX-014 Snapshot: save or copy the current frame — v1

**What the user sees:** A camera button on the player and a menu entry, "Save
frame as image…". It writes the frame at the playhead as a PNG or JPEG at full
quality. Sub-actions copy the frame to the clipboard or add it to the media
library as an image clip source.

**Inputs / controls:**
- Key: see UI-031 ("Export current frame…"). Comparison: Shotcut also gives
  its frame export a shortcut.
- Format: PNG 8-bit (default), PNG 16-bit, or JPEG (quality 1–100, default
  92).
- Size: canvas size (default), current export resolution (EX-024), or a custom
  long side of 16–8192 px.
- Destination: save as file (default name `<project>_<HH-MM-SS-FF>.png` in
  the last-used snapshot folder, else the OS Pictures folder), copy to
  clipboard, or add to media library (the image is written to PM-041's
  generated-media store and imported as a still, PM-034).
- Reference (observed): the reference player has a still-frame export with a
  success notice. One label speaks of exporting frames in the plural, so a
  batch variant may exist (unclear). FreeCut covers many stills with image
  sequence export over a range (EX-047) and per-marker batches with EX-050.

**Behavior / output:**
- Renders through the export path at the chosen size with full quality. It
  ignores preview resolution, proxies, guides, handles and preview-only
  switches.
- Color: working-space values (Rec.709 primaries, gamma-encoded; CL-002) are
  written as-is. PNG files get an sRGB chunk, since sRGB shares those
  primaries.
- Runs as a short background job. The UI shows a toast with "Show in folder".

**Edge cases:**
- Playback running: it pauses first, then captures the shown frame.
- Playhead at `duration`: captures the last frame.
- Offline media in frame: captures the slate (EX-005), as export would, with a
  warning toast.
- Pending analysis (stabilization, AI mattes): waits for it, with progress, so
  the snapshot matches export (AI-001).
- File exists: auto-suffix ` (2)`, ` (3)`… (FreeCut default for snapshots, so
  a quick capture never prompts).
- Clipboard too large (8K, 16-bit): falls back to 8-bit and notes it.

**Undo:** File and clipboard output are n/a. "Add to media library" is an
undoable PM import; undo removes the item but keeps the file (PM-041).

**Model notes:**
- `freecut-render::offscreen::render_still(seq_snapshot, Tick, size) -> RgbaImage`.
- PNG/JPEG encoding goes through `freecut-media` (FFmpeg image encoders), so
  only one crate does codec work.
- The clipboard is handled in `freecut-app`.

### EX-015 Media preview mode (source viewing in the player) — v1

**What the user sees:** A single click on a library tile (PM-060) shows that
item in the player instead of the timeline. A chip above the viewer reads
"Media: <name>", and the transport controls play that file alone. You can
mark in and out on it, and the next add to the timeline uses only that part.
Clicking the timeline, the chip, or pressing Esc returns the player to the
timeline.

**Inputs / controls:**
- Single-click a media tile to preview it (PM-060). Adding to the timeline
  uses TL-012's controls ("+", Enter, or dragging from the tile or the
  viewer).
- "Set range in / out" mark the media item while in this mode, and "Clear
  range" clears both (as TL-038 on the timeline; keys: see UI-031).
  Play/pause, shuttle, frame stepping and snapshot all work.
- The "Media / Timeline" chip switches modes.

**Behavior / output:**
- Video plays at the source's own frame rate and display aspect, fitted to the
  viewer, with its own audio. No project effects or canvas apply. Proxies are
  used when PM-081's toggle is on.
- Images show as stills, with no range. Audio-only media shows a waveform
  strip with a playhead (waveform data from PM-072).
- The marked range is remembered per item as PM-060's view state and is used
  by the next add to the timeline (TL-012). Source-side insert and overwrite
  edits are TL-047 (later).
- The timeline playhead is not moved by media preview.

**Edge cases:**
- Offline media item: the slate is shown (EX-005) and transport is disabled.
- VFR source: the timecode shows the source's nominal rate. Stepping goes by
  decoded frame timestamps.
- In set after Out: the marks swap (PM-060). A range shorter than one source
  frame is refused.
- A media item is removed from the library while previewed: the player returns
  to timeline mode.
- Starting timeline playback (Space with the timeline focused) switches back
  to timeline mode first.

**Undo:** n/a (view state). Marking a range never dirties the project and is
not on the undo stack (PM-060). Placing media is TL's op (TL-012, later
TL-047).

**Model notes:**
- `PlayerMode { Timeline, Media(MediaId) }` in `freecut-app::player`.
- `freecut-app` wires a simple source player: frames decoded by
  `freecut-media`, audio through a `freecut-audio` voice that bypasses the
  timeline mixer. `freecut-media` never depends on `freecut-audio`.
- The marks live in PM-060's `PreviewRanges: HashMap<MediaId, TimeRange>`
  view-state map in `freecut-app` (media ticks), not in the document.

### EX-016 Render cache and pre-render — v1

**What the user sees:**
- Heavy sections (stacked effects, 4K multicam-style layering) play smoothly
  after their first viewing, or after you choose "Pre-render range".
- A thin status bar above the ruler shows each section's state: cached
  (green), not cached (none), or too slow to play live (amber).
- Compound clips offer "Pre-render" in their context menu (TL-040).

**Inputs / controls:**
- RAM frame cache budget: default 25 % of system RAM, up to 8 GB (FreeCut
  default). Configurable in preferences.
- "Pre-render in–out range" command. "Pre-render compound clip" (TL-040).
- "Smart pre-render while idle": default off. When on, sections that played
  slower than real time are rendered in the background after 5 s of
  inactivity.
- Disk storage: pre-rendered segments are a "Render cache" category inside
  PM-074's media cache. They share its location, size limit and LRU eviction;
  EX defines no separate limit.
- "Clear render cache" command (clears only this category), and "Clear this
  project's render cache", which removes only the segments the open
  project's timeline currently uses.
- Reference (observed): the reference editor has an auto-render toggle for
  heavy sections, a render-selection command, pre-render for compound clips,
  rendered/unrendered marks on the timeline, a "view" action once rendering
  completes, and a command to delete a project's render files. Its default
  for auto-render is not shown; FreeCut keeps idle pre-render off by default
  so the disk is not written unasked.

**Behavior / output:**
- Every rendered frame has a **render key**: a 128-bit hash of every input
  that affects that frame at that time and preview scale. Inputs include clip
  ids and parameters, keyframes evaluated at t, media fingerprints (PM),
  effect versions and the preview scale.
- Cached frames are reused only when the key matches. Edits change the keys
  of the frames they affect, so stale frames are never shown. Keys are cheap
  to compute per frame.
- Pre-rendered sections are written to PM-074's cache (regenerable data, so
  never PM-041's generated-media store) as an intra-frame intermediate at the
  current preview scale. Each segment covers 2 s, so an edit invalidates
  little. The format is ProRes 422 LT through FFmpeg's own encoder, chosen
  for fast decode (FreeCut default, pending performance tests).
- Export ignores the disk cache by default and renders from the originals at
  output resolution (EX-022).
- Pre-render runs as a row in PM-073's task list with progress. When it
  finishes, a toast says the range is ready, with **Play range** (EX-008).
- A segment that fails to render is marked red in the status bar, with the
  reason in its tooltip and **Retry pre-render** in its context menu.
  Playback renders that section live; the other segments are kept.
- Pre-render pauses while an export job runs (EX-038) and resumes after it,
  so the two never compete for the GPU.

**Edge cases:**
- The chosen range contains no clips (only gaps or empty tracks): the command
  is refused with "Nothing to pre-render in this range".
- Not enough free space for the estimated segments when pre-render starts:
  refused before any work, naming the volume and how much to free, with
  **Storage settings…** (UI-026). PM-113's pause threshold also pauses a
  running pre-render.
- A clip in the range still waits for an analysis (stabilization, AI mattes,
  AI-001): its sections show "Waiting for analysis" in the status bar and are
  rendered once the analysis finishes. Sections without such clips start at
  once.
- Cache folder is on a full disk: pre-render stops, with a toast. Playback
  falls back to live rendering. PM-074 may shrink the limit temporarily.
- Cache cleared from PM-074's Preferences while playing: segments regenerate
  on demand, and the preview may stutter briefly.
- Media file replaced on disk with the same name: the fingerprint changes,
  and cached segments for it become invalid.
- Effects with random seeds are deterministic (FX-001), so cached frames stay
  valid.
- Preview scale change: a different key; the old segments remain until
  evicted.
- A project opened on another machine: the disk cache is not portable and is
  rebuilt on demand.

**Undo:** n/a (cache, not document state).

**Model notes:**
- `freecut-timeline` exposes a pure `frame_inputs_hash(seq, Tick) -> u128`
  over the resolved per-frame model inputs (clips, parameters, keyframes at t,
  media fingerprints). It is unit-tested for "edit outside range leaves hash
  unchanged". It knows nothing about preview scale or shaders.
- `freecut-render` combines that hash with the preview scale and its shader
  versions into the render key.
- `freecut-render::cache::{RamFrameCache, DiskSegmentCache}`. Segment files
  are stored through `freecut-media::cache` (PM-074) and encoded and decoded
  by `freecut-media`.
- Segment states for the status bar: `SegmentState { Cached, NotCached,
  TooSlow, Waiting, Failed(Reason) }`. "This project's render cache" is the
  set of render keys the current timeline resolves to, so clearing it needs
  no per-project bookkeeping in the cache.

### EX-017 Playback performance indicator — v1

**What the user sees:** An optional readout in the viewer corner, such as
"29.4 / 30 fps". It is green at 95 % of the target rate or more, amber from
75 % and red below that. Hovering lists the reasons (decode, render, disk) and
suggests fixes: lower preview resolution, use proxies, pre-render the range.

**Inputs / controls:**
- Toggle "Show playback stats". Default: auto, which shows the readout only
  while frames are being dropped.
- An extended mode adds decode ms, render ms, cache hit rate and the GPU
  adapter name.
- "Can't keep up" hint (FreeCut defaults): when fewer than 75 % of due frames
  are shown for 5 s in a row, a one-line banner appears in the viewer once
  per session and project. It offers **Lower preview resolution** (one step,
  EX-006), **Use proxies** (PM-081, when proxies exist or can be made),
  **Pre-render this range** (EX-016) and **Smooth playback** (EX-056, when
  Sharp is set), plus **Don't suggest again for this project** (stored in the
  project's UI section).
- Reference (observed): the reference player detects dropped frames, flags
  severe drops, and then suggests a performance-first preview mode or proxy
  media; its lower-resolution prompt can be silenced per project.

**Behavior / output:** Counts presented frames against the frames due, over a
1 s sliding window. Counters reset at each play start. Nothing is logged to
disk unless debug logging is on.

**Edge cases:**
- Shuttle at 2× and faster: the target is the display cadence, not `rate ×
  fps`, so dropping intermediate frames is not counted as a failure.
- Paused: the readout hides.
- The hint never appears while an export job runs (EX-038 lowers preview
  priority on purpose) or in the first second after a seek, while the
  decoders warm up.
- The hint's actions are already in effect (lowest resolution, proxies on):
  those buttons are hidden; with nothing left to offer, the hint does not
  appear.

**Undo:** n/a.

**Model notes:** `PlaybackStats` is collected in `freecut-render`'s frame
queue and shown by `freecut-app`. The hint trigger (thresholds, once per
session, per-project dismissal) is a pure function over a stats history,
tested with synthetic counters; the dismissal flag lives in
`freecut-project::ui::PlayerUiState`.

### EX-018 Hardware-accelerated decoding for preview — v1

**What the user sees:** Smoother playback of H.264/HEVC/AV1 sources on
machines with video decode hardware. A preference "Hardware video decoding"
offers Auto (default) or Off.

**Inputs / controls:** Preference: Auto / Off.

**Behavior / output:**
- Backends through FFmpeg hwaccels:
  - VideoToolbox (macOS);
  - D3D11VA (Windows);
  - NVDEC/CUDA, VAAPI (Linux);
  - Intel QSV where available.
- If a hardware decoder cannot open a stream (unsupported profile, for example
  4:2:2 10-bit H.264, or too many sessions), that stream silently uses
  software decoding.
- Standard video decoding is bit-exact by specification. Color conversion
  happens in FreeCut's own shaders (CL-002), so preview and export look the
  same whether hardware decode is used or not.
- Export uses the same setting, falling back to software on errors.
- A change applies to the next decoder that opens (the next seek or clip
  boundary), without restarting FreeCut or reopening the project.
- Reference (observed): the reference editor has compatibility toggles meant
  to fix export problems, and offers a retry with hardware acceleration off
  when an export stalls. Alternative seen in the reference app: those toggles
  take effect only after the project is reopened.

**Edge cases:**
- Driver crash or device loss mid-playback: the decoder for that source is
  recreated in software, and a one-time toast appears.
- Too many concurrent 4K streams for the hardware: excess streams use software
  decode.
- Remote desktop or VM without GPU: Auto detects nothing and uses software.

**Undo:** n/a (app preference).

**Model notes:** `freecut-media::decode::HwDecodePolicy`. `unsafe` FFI to
FFmpeg hwaccel contexts stays inside `freecut-media`, with `// SAFETY:`
comments. Zero-copy upload into wgpu textures is a later optimization; the v1
path copies through CPU memory.

### EX-019 Audio monitoring controls on the player — v1

**What the user sees:** Next to the player are a speaker button that mutes
preview audio, the master volume slider and meter (AU-010, AU-015), and a
toggle for audio scrubbing (AU-016).

**Inputs / controls:**
- Preview mute: default off. Key: see UI-031 ("Preview mute").
- Master volume and limiter (AU-010).
- Scrub audio toggle (AU-016; key: see UI-031, "Toggle audio scrubbing").

**Behavior / output:**
- Preview mute only silences the output device. It does not change the
  project or the export, and it is not saved.
- Master volume is a project setting with undo, as AU-010 defines.
- The playback clock still runs from the audio device while muted (it outputs
  silence), so sync is unaffected.

**Edge cases:**
- Preview mute on while exporting: export audio is unaffected, and the dialog
  does not warn.
- No output device: the controls are disabled and the AU-010 banner explains
  why.

**Undo:** Preview mute and scrub toggle are n/a. Master volume follows AU-010.

**Model notes:** UI in `freecut-app`. Gain and mute flags go to
`freecut-audio`'s output stage. Preview mute is applied after the master bus
and after metering, so the meters still move while muted (FreeCut default).

### EX-020 External display / secondary preview window — later

**What the user sees:** "Show preview on second display" puts a borderless,
always-fullscreen copy of the viewer on another monitor. It stays there while
you edit on the main screen.

**Inputs / controls:** Display picker. Options: show guides, and mirror
pause/play overlays.

**Behavior / output:** Shares the rendered texture with the main viewer.
Renders at the larger of the two targets when sizes differ.

**Edge cases:**
- Display disconnected: the window closes, with a toast.
- Display at a different refresh rate: frames are presented at that
  display's vsync.
- The window cannot be created or shown on the chosen display (the OS
  refuses, or the GPU cannot drive it): a warning toast names the display;
  editing continues on the main window and the main viewer is unaffected.
- Reference (observed): the reference editor can send its preview to an
  external display and reports connection problems, after which the user
  keeps editing without it.

**Undo:** n/a.

**Model notes:** A second egui viewport. Hardware video-output cards are out
of scope.

### EX-021 Export dialog — mvp

**What the user sees:** An **Export** button at the top right, plus File →
Export… (key: see UI-031), opens one dialog with these parts:
1. **File:** name and destination folder (EX-023).
2. **Output type:** Video (MVP). Audio only (EX-044, v1), GIF (EX-043, v1)
   and Image sequence (EX-047, later) are added by those features. From v1,
   "Also export" checkboxes add an audio file or a GIF to the same run
   (EX-057).
3. **Preset** drop-down (v1, EX-041/042). It shows "Custom" when settings are
   edited.
4. **Video:**
   - resolution (EX-024);
   - frame rate (EX-026);
   - codec and container (MVP: H.264 in MP4, shown read-only; choices arrive
     with EX-030 in v1);
   - quality (EX-027; custom rate control is EX-028, v1);
   - color: "Rec.709 (SDR)", shown read-only, with bit depth from v1
     (EX-030, EX-032);
   - hardware encoding (EX-034, v1).
5. **Audio:** EX-031.
6. **Range** (v1, EX-035).
7. **Captions** (v1, hosted for TX-033/TX-032).
8. **Extras** (v1):
   - cover image and metadata (EX-046); a **Design…** button opens the cover
     designer once EX-053 ships (later);
   - "save credits next to the video" (OV-041);
   - later: your own logo (EX-059) and "Add motion blur to moving clips…"
     (EX-060).
9. **Footer:**
   - output duration, estimated size and free disk space (v1, EX-037), with a
     one-line summary of resolution, aspect ratio, frame rate, codec and
     container, bitrate and color;
   - a preflight summary line (EX-036);
   - Cancel, and **Export** (v1 adds "Add to queue", EX-038).

**Inputs / controls:**
- Defaults the first time a project is exported:
  - name = project name;
  - folder = the last export folder used on this machine, else the OS
    Movies/Videos folder;
  - resolution = canvas size (EX-024's "Canvas size" entry, matching PM-004),
    so a 1080×1920 canvas exports at 1080×1920;
  - frame rate = project frame rate (PM-005);
  - quality = Recommended;
  - audio = AAC 192 kbps at the project sample rate (PM-007).
- Later exports reuse the last settings used for this project.
- Reference: the reference editor's public pages say its export step lets you
  choose resolution, frame rate and file format (bitrate too on mobile), with
  output up to 4K and 60 fps, and that audio can be saved as MP3, WAV, AAC or
  FLAC.
- Reference (observed): the desktop dialog has a required title field, a
  folder picker with "use the last folder", separate checkboxes for video,
  audio only, a captions file and GIF (several in one run), a codec section
  with a one-line description per codec, bitrate modes, color space and bit
  depth, a faster-versus-higher-quality switch, an export-time motion blur
  option, an optional sound when finished, and a summary panel. FreeCut
  places each of these in the sections above or in the features they link
  to. Alternative seen in the reference app: video output is itself a
  checkbox that can be turned off; FreeCut picks one main output type and
  adds companions (EX-057), which gives the same results.

**Behavior / output:**
- The dialog is non-blocking for the UI thread. The available encoders are
  probed in the background at app start and cached. Options that need a
  missing encoder are shown disabled, with the reason.
- Every change re-runs the preflight checks (EX-036), and from v1 the size
  estimate (EX-037), within 100 ms, debounced.
- Pressing **Export** validates the settings, stores them in the project's
  export section (marks the project modified, no undo entry), and starts the
  job (EX-022).
- In the MVP the dialog becomes the progress view and editing is blocked
  until the job ends. v1 can export in the background (EX-038).

**Edge cases:**
- Empty timeline: the Export button is disabled, with a tooltip (TL-003).
- Empty or whitespace-only file name: Export is disabled, and the name field
  says "Enter a file name" (EX-023).
- The preview renderer is unavailable (EX-001): Export is disabled with the
  same reason, since export renders on the same GPU device.
- Opened while inside a compound clip (TL-040): it exports the main timeline,
  and a note says so. A compound clip alone can be exported with EX-058
  (later).
- Saved settings refer to an encoder this machine lacks (project moved from
  macOS to Linux): the closest available equivalent is chosen (same codec in
  software, else H.264), with a notice.
- A project file from an older FreeCut without export settings: defaults
  apply. A newer version adds fields through the project migration chain.
- The dialog is opened during a running background export (v1): allowed. A
  new job queues.

**Undo:** n/a. Export settings are saved with the project but are not
timeline edits.

**Model notes:**
- `freecut-project::export::ExportSettings` (serde, versioned) contains:
  - `output: OutputKind`;
  - `file: FileTarget`;
  - `video: VideoSettings { size, frame_rate: FrameRate, codec, container, quality: QualitySetting, color: OutputColor, hw: HwEncodePref }`;
  - `audio: AudioExportSettings`;
  - `range: ExportRangeKind`;
  - `captions: Vec<CaptionExportChoice>`;
  - `cover: Option<CoverSource>`;
  - `metadata: ExportMetadata`;
  - `companions: Vec<CompanionOutput>` (v1, EX-057);
  - `preset: Option<PresetId>`.
- Validation is a pure function, `validate(&ExportSettings, &EncoderCaps,
  &SequenceInfo) -> Vec<Issue>`, shared with preflight.
- The dialog UI lives in `freecut-app`.

**Implementation note (2026-10-12):** `freecut_project::export` holds the
MVP subset: `ExportSettings { file: FileTarget { name, folder }, video:
VideoSettings { size: OutputSize, frame_rate: FrameRateChoice, codec:
VideoCodec::H264, container: Container::Mp4, quality: QualityTier }, audio:
AudioExportSettings { include, kbps, sample_rate } }`, plus
`ExportSettings::defaults(name, folder, &Document)` for the first export
(audio off when nothing in the root sequence is audible). The last settings
are stored in the project file's top-level `"export"` key (format 3; `null`
before the first export): the app passes them in `SaveRequest::export` and
gets them back in `Opened::export`. Unknown keys inside survive like any
other. A folder path that isn't Unicode is written as `null`, so the next
export asks for a folder. Output kind, range, captions, cover, metadata,
companions and presets are not modelled yet.

**App implementation note (2026-10-12):** the dialog is
`freecut-app/src/ui/export.rs` (`ExportDialog`), opened by the accent
**Export** button at the right of the top bar, File › Export… and `⌘E` /
`Ctrl+E`. The button is disabled on an empty timeline, with the reason as
its tooltip. Built: name with the extension shown after it, folder with
Browse… (the OS folder picker), resolution (Canvas size plus the five
presets, each showing its pixel size), frame rate shown read-only as
"Project (<rate> fps)", quality tiers with their bitrate, read-only
"H.264 video in MP4" and "Rec.709 (SDR)" rows, Include audio with AAC
bitrate and 44.1/48 kHz, a footer with duration, size, frame rate, "up to"
size estimate and free space, and the preflight line (EX-036), which
expands to the issues. Clicking an issue about clips selects them.
Defaults: the project's last settings, else `ExportSettings::defaults` with
the project file name ("Untitled" before the first save). The folder is the
last export folder of this session, else `~/Movies` (macOS) or `~/Videos`;
the per-machine "last export folder" is not persisted yet. Folder checks,
free space and the encoder probe run on the worker pool; the dialog shows
"Checking…" and keeps Export disabled until they answer. Export stores the
settings in `Project::export` (modified, no undo entry) and pauses
playback. **Deviation (FreeCut, this round):** editing is not blocked while
the job runs. The progress view has **Keep editing**, which hides the
dialog; the job works on its snapshot, and the status bar shows "Exporting
42 % · 0:12 left — Show" (then "Export finished — Show"). Only one export
runs at a time: the Export button shows "Exporting…" and reopens the
progress view.

### EX-022 Export job: render, encode, progress and cancel — mvp

**What the user sees:** After Export, a progress view shows:
- the phase ("Preparing", "Analyzing", "Rendering", "Finishing");
- a progress bar with percent;
- frames done out of the total;
- elapsed time, ETA and speed (for example "1.8× real time");
- a small live thumbnail of the frame being encoded, updated every 1 s.

**Cancel** stops the export and leaves no partial file. On success, the
result view appears (EX-039). On failure, it shows a plain-language reason,
a short failure code, the suggested fix as buttons, and "Copy details" (see
the failure catalogue below).

**Inputs / controls:** Cancel. A confirmation is asked only if the job has
run for more than 30 s (FreeCut default). Alternative seen in the reference
app: every stop asks for confirmation.
- During the Analyze phase each pending analysis has its own status label
  (for example "Stabilizing clip 3 of 5"), so a long wait is explained.
- Reference (observed): the reference editor gives each export failure its
  own message, an error code and a suggested fix, with a generic "try again"
  fallback and a support path.

**Behavior / output:** Pipeline, all off the UI thread:
1. **Snapshot.** An immutable `Arc` snapshot of the document is taken. The
   job never reads the live document again.
2. **Resolve.** The range (EX-035) and the output frame list are resolved:
   `N = ceil(range.duration × R)` frames at times `S + n/R`.
3. **Analyze.** Missing analyses are run or waited for (AI-001,
   stabilization, mattes), with their own progress.
4. **Open outputs.** The encoder and muxer are opened in `freecut-media` at a
   temporary path `<final name>.part` in the destination folder. The container
   format is passed explicitly, not guessed from the extension.
5. **Render loop.** For each `n`:
   - `freecut-render` composites the frame at output size (FX-001).
   - A shader converts it to the output pixel format (EX-032).
   - The GPU-to-CPU readback is double buffered (up to 3 frames in flight).
   - Frames go into a bounded channel (capacity 8) to the encoder thread.
6. **Audio.** `freecut-audio` mixes the same range offline in 1024-sample
   blocks (AU-010) on its own thread. The muxer interleaves audio and video by
   timestamp.
7. **Finish.** Encoders are flushed and the trailer written. MP4/MOV get
   "fast start" (the index moves to the front of the file; this briefly needs
   free space about equal to the file size). The file is synced to disk and
   atomically renamed to the final name.

Other rules:
- **Progress:** 0–99 % is frames muxed over `N`, and the last 1 % is
  finishing. ETA uses the average frame rate of the last 10 s and is shown
  after 3 s.
- **Never drops frames.** A slow decode makes the job slower, never different.
- **Cancel** takes effect within 1 s. It stops threads, closes the encoder
  and deletes the `.part` file.
- The system is kept awake while a job runs: macOS power assertion, Windows
  execution state, Linux idle inhibit over D-Bus.
- **Source check:** at job start the size and modification time of every
  source file in the snapshot are recorded. If a source disappears, or its
  size or time differs when the job opens or reopens it, the job stops with
  the `source-changed` failure below instead of encoding mixed content.

**Failure catalogue** (messages are FreeCut's own; every failure deletes the
`.part` file and keeps the dialog's settings for a retry):

| Code | Cause | What FreeCut says and offers |
|---|---|---|
| `disk-full` | destination out of space | "Not enough space on <volume>. Free at least <amount>, or choose another folder." **Choose folder…** |
| `file-too-large` | file system's per-file limit: FAT32 4 GB, FAT16 2 GB | "<volume> can't hold files larger than <limit>." **Choose folder…**; the hint suggests a smaller quality tier or a range export |
| `dest-unavailable` | folder deleted, invalid path, or drive disconnected | "The export folder is no longer available." **Choose folder…** |
| `no-permission` | the OS refused to write | "FreeCut isn't allowed to save in this folder." **Choose folder…**; on macOS the hint names the system privacy setting for folder access |
| `out-of-memory` | CPU or GPU memory allocation failed | "Not enough memory to export at this size." **Retry**; the hint suggests closing other apps or a lower resolution |
| `source-changed` | a source file was deleted, moved or overwritten during the job | "'<file>' changed while exporting." **Retry** (takes a new snapshot) |
| `decode-error` | a damaged frame or stream in a source | names the clip and timeline time; **Retry using slates for unreadable frames** (below) |
| `gpu-lost` | the GPU device was lost | **Retry** |
| `encoder-failed` | the encoder or muxer reported an error | **Retry**; with hardware encoding, Auto has already retried in software (EX-034) |
| `stalled` | no frame finished for 60 s (v1, EX-034) | **Keep waiting**, **Retry without hardware acceleration**, Cancel |
| `internal` | a panic inside the job (contained, never a crash) | "Something went wrong inside FreeCut." **Copy details** and the log location |

- Codes are stable identifiers, shown small under the message, so users can
  search the user guide's troubleshooting page (UI) for them. FreeCut has no
  support service and sends nothing anywhere.
- "Copy details" copies the code, the message, a summary of the settings,
  the FreeCut, FFmpeg and GPU driver versions, and the last log lines. File
  paths appear only in that copied text.

**Edge cases:**
- **Disk full mid-export:** the job stops with `disk-full` and the partial
  file is deleted.
- **Destination becomes unavailable** (USB removed): `dest-unavailable`.
  Deleting the partial file is attempted, and the job reports the path.
- **The file system cannot hold files over 4 GB** (FAT32, or 2 GB on FAT16):
  FreeCut detects the `EFBIG`-type error and reports `file-too-large`. From
  v1, preflight warns before the export when the estimate already exceeds the
  limit (EX-036).
- **A source file is deleted or replaced while the job runs:**
  `source-changed`. (A file that went missing between queueing and job start
  is handled by preflight and EX-038 instead.)
- **Decode error in a source frame:** the job stops and names the clip and
  timeline time. "Retry using slates for unreadable frames" re-runs it with
  error slates (EX-005) instead of failing, and the result view lists them.
- **GPU device lost:** the job fails with "Retry". It does not try to resume
  mid-file.
- **The app is quit during an export:** a prompt offers to cancel the export
  and quit, or keep exporting.
- **Crash during export:** the `.part` file remains. On next launch FreeCut
  offers to delete leftover `.part` files it recorded as in progress.
- **Range of exactly one frame:** a valid one-frame file.
- **Offline media accepted at preflight** (EX-036, PM-090): those layers
  render the offline slate and their audio is silent (EX-005).
- **Hidden tracks, muted tracks, solo:** hidden tracks render nothing and
  muted tracks are silent, read from TL-008's two track flags (mvp) in the
  job's snapshot. Solo follows AU-012 ("Export as heard"). Locked tracks
  (TL-009, v1) export normally.

**Undo:** n/a (file output).

**Model notes:**
- `freecut-app::jobs::ExportJob` orchestrates and reports `ExportProgress {
  phase, frames_done, frames_total, elapsed, eta }` over a channel.
- Library errors use `thiserror`, for example `freecut-media::EncodeError`
  and `freecut-render::RenderError`. `anyhow` is used only in the app.
- `ExportFailure { code: FailureCode, detail, fixes: Vec<FixAction> }` in
  `freecut-app::jobs`. The mapping from library errors (I/O kinds, FFmpeg
  error numbers, wgpu errors) to `FailureCode` is a pure, table-tested
  function, so every row of the catalogue has a test.
- The frame-time list `export_frame_times(range, FrameRate) -> impl
  Iterator<Item = Tick>` is pure in `freecut-timeline` and unit-tested,
  including NTSC rates and very long ranges.

**App implementation note (2026-10-12):** `freecut-app::jobs::export`.
`ExportJob::start(ExportRequest)` takes an `Arc<Document>` snapshot, the
media paths, offline items, the settings, the conflict choice and the font
registry, and runs on a `freecut-export` thread:
- `ExportPlan::resolve` gives the `ExportClock` (whole root sequence), the
  `EncodeSpec` (`output_dimensions`, `output_rate`, `VideoQuality::TargetKbps(
  video_kbps)`, AAC stereo when audio is included, title = sanitized name).
- The source files used by clips are stamped (size and modification time).
- `<final>.part` is claimed with `create_new`; a taken name moves on to
  `<final>.<n>.part`. `EncodeSession::create` then opens it.
- A render thread (`ExportRenderer::with_media`, the shared CPU compositor,
  frames in a bounded queue of 4 with buffer reuse) and an audio thread
  (`OfflineMix::for_document`, blocks in a queue of 16) feed the mux loop,
  which pulls whichever stream is behind (`video_due_before_audio`).
- After the last frame the stamps are checked again (`source-changed`),
  then `finish`, `verify_output(..).check(..)`, fsync, rename onto the final
  name (Keep both is re-resolved if a file appeared meanwhile) and a best
  effort fsync of the folder.
Progress is a lock-free `ProgressCell` (frames rendered, video packets
written, phase); the bar is 99 % frames and 1 % finishing, and the ETA and
speed use `EtaWindow` (last 10 s, shown after 3 s). Cancel sets the
`CancelToken`; the mux loop checks it at least every 100 ms, producers once
per frame or block, and the `.part` file is deleted (measured: a few ms at
320×180). Cancel asks first after 30 s. Dropping a running job (quitting)
cancels it and waits; quitting asks first. `FailureCode` covers
`disk-full`, `file-too-large`, `dest-unavailable`, `no-permission`,
`out-of-memory`, `source-changed`, `encoder-failed` and `internal` (a panic
in any job thread), mapped from `io::ErrorKind` and `EncodeError` by
table-tested pure functions; the failure view shows the message, the code,
**Choose folder…** or **Retry**, and **Copy details**. Not built yet:
keep-awake, the live thumbnail, the stall watchdog, the crash-recovery
registry of `.part` files, and `gpu-lost` (no GPU path yet). `decode-error`
(2026-10-12): `ExportRenderer::render_frame`'s `ExportRenderError::Decode { clip,
at, reason }` maps to `decode-error`, whose message names the clip and the
timeline timecode ("The picture of “interview.mp4” at 00:00:12:04 can't be
read"); its only fix is **Retry using slates for unreadable frames**, which
starts the job again with `with_slates_for_unreadable()`. The result view then
says how many frames show the error slate and names the first five (clip and
time). If another project was opened while an export ran, its failure view
offers no fix (only Copy details and Close), a cancel closes the dialog, and the
next Export takes the open project's settings: the old project's name, folder and
conflict choice are never used for the new one (FreeCut default). A cancel during the first seconds after
launch can wait for the system font scan (at most 30 s), because the
renderer waits for it before the first frame.

### EX-023 Output file name, folder and overwrite rules — mvp

**What the user sees:** A name field (without extension; the extension shown
next to it follows the container) and a folder field with a Browse button. If
a file with that name exists, a notice appears before export starts, with
**Replace** and **Keep both** choices.

**Inputs / controls:**
- Name: 1–200 characters, required. Default: the project name.
- Folder: picked with the OS dialog. It is remembered per machine and per
  project. A "Use last export folder" entry at the top of the folder menu
  fills in the folder of the most recent export on this machine.
- On conflict: Replace / Keep both (adds ` (2)`, ` (3)`…). The default button
  is Keep both.
- Reference (observed): the reference dialog requires a title, rejects an
  empty one, asks before overwriting an existing file, and asks for another
  folder when the chosen one is unwritable or on an unavailable external
  drive.

**Behavior / output:**
- Names are sanitized on every OS:
  - characters `<>:"/\|?*` and control characters become `_`;
  - leading and trailing spaces and trailing dots are trimmed;
  - Windows reserved names (CON, PRN, AUX, NUL, COM1–9, LPT1–9) get a `_`
    suffix.
- The extension comes from the container (`.mp4`, `.mov`, `.mkv`, `.webm`,
  `.gif`, or the audio format extensions in AU-032).
- The folder is checked for write access when the dialog opens and again at
  Export.

**Edge cases:**
- Empty or whitespace-only name: Export is disabled and the field says "Enter
  a file name". FreeCut does not fill it in silently.
- Name made only of invalid characters: falls back to "Untitled".
- Folder on an external drive that is not connected: the field turns red,
  naming the drive ("'Card A' isn't connected"), and Export is disabled until
  the drive returns or another folder is chosen.
- Folder that exists but cannot be written (permissions, read-only volume,
  OS folder privacy): the field turns red with the reason; **Browse…** is
  suggested. It is checked again at Export, because permissions can change
  while the dialog is open.
- The target path is a media file used by the project: export is refused, with
  "This file is used in your project".
- The folder no longer exists: the field turns red and Browse is suggested.
  FreeCut never creates folders silently.
- Very long paths on Windows: `\\?\` long-path handling is used. Names over 200
  characters are refused.
- Two queued jobs (v1) with the same target: the second gets the Keep-both
  suffix at queue time.
- Case-insensitive file systems: a conflict check that ignores case.

**Undo:** n/a.

**Model notes:** Pure helpers `sanitize_file_stem(&str) -> String` and
`resolve_conflict(stem, existing: &dyn Fn(&Path) -> bool)` live in
`freecut-project::export`. They take a callback, so they need no I/O and are
easy to test. The file system is accessed only in `freecut-app` jobs.

**Implementation note (2026-10-12):** `resolve_conflict(folder, stem,
container, exists)` returns `name.mp4`, then `name (2).mp4`, `name (3).mp4`…
(up to 9999, else `None`); `target_path(.., ConflictChoice, ..)` covers
Replace too. `sanitize_file_stem` returns `""` for an empty or blank name
(preflight reports `EmptyFileName`), `Untitled` when no valid character is
left, and puts the `_` of a device name right after the device part, so
`nul.backup` becomes `nul_.backup` (Windows also reserves device names with
an extension). Preflight refuses a stem over 200 characters and a file name
over 255 UTF-8 bytes.

**App implementation note (2026-10-12):** the dialog asks the worker pool
(`jobs::platform::probe_destination`) about the folder and the file
whenever either changes: missing folder, a drive under `/Volumes/<name>`,
`/media/<user>/<name>` or `/run/media/<user>/<name>` (or a missing Windows
drive) that isn't mounted, writability (a uniquely named probe file is
created and deleted), free space (`fs4::available_space`), and whether
`<name>.mp4` exists. An existing file shows the notice with **Keep both
(save as "<name> (2).mp4")** (default) and **Replace**; changing the name
or folder goes back to Keep both. The job checks again when it starts and
re-resolves Keep both before the final rename; Replace keeps the old file
until the finished `.part` file is renamed over it.

### EX-024 Output resolution presets — mvp

**What the user sees:** A resolution drop-down with 480p, 720p, 1080p, 1440p
(2K), 2160p (4K) and "Canvas size (W×H)". Each entry shows the actual pixel
size for this canvas, for example "1080p — 1080×1920" for a portrait project.

**Inputs / controls:**
- The presets name the short side: 480, 720, 1080, 1440 and 2160 px.
- Default: see EX-021.
- Reference: the reference editor's pages list 720p, 1080p, 2K and 4K, plus
  480p in its desktop export.
- Reference (observed): its dialog has an "Original" entry meaning the
  project's resolution, which is changed in project settings rather than in
  the dialog (FreeCut's "Canvas size" behaves the same), named steps that
  include 720p and 1080p, and an 8K entry. Older public claims stop at 4K at
  60 fps, so 8K was probably added later; FreeCut offers 8K in v1 (EX-025).

**Behavior / output:**
- For a canvas of width `W` and height `H` and a preset short side `P`:
  - landscape (W > H): height = P, width = `round_even(P × W / H)`;
  - portrait: width = P, height = `round_even(P × H / W)`;
  - square: P × P.
- `round_even` rounds to the nearest even integer, as 4:2:0 chroma requires.
- The compositor renders directly at the output size. There is no
  render-then-rescale step, so text and vectors stay crisp, and each source
  is resampled once (FX-001 L8).
- Positions are stored as fractions of the canvas (TR conventions), so the
  layout is identical at every output size.

**Edge cases:**
- Output larger than every source: allowed. An info note in preflight
  (EX-036) says the result may look soft.
- Canvas aspect with odd results (for example a 1081×1921 custom canvas): the
  even rounding changes the aspect by less than 0.2 %. This is accepted and
  noted in the tooltip.
- Output size above the GPU's maximum texture size
  (`max_texture_dimension_2d`): the option is disabled, with the reason.
- Hardware encoder size limits (many cap H.264 near 4096 px wide): these are
  handled by EX-034's fallback.
- H.264 level: chosen automatically from size and frame rate (EX-029).

**Undo:** n/a (export setting).

**Model notes:** `OutputSize { Preset(ShortSide), Canvas, Custom { w, h } }`.
`output_dimensions(canvas, OutputSize) -> (u32, u32)` is pure in
`freecut-project::export`, with tests for landscape, portrait, square and odd
canvases.

### EX-025 Custom output size and 8K — v1

**What the user sees:** The resolution list adds **4320p (8K)** and
**Custom…**. Custom takes width and height, locked to the canvas aspect by
default (the lock can be released).

**Inputs / controls:**
- Custom W and H: 16–8192 px each, even values (odd input rounds up, and the
  field shows the rounded value at once). Alternative seen in the reference
  app: odd values are rejected, and one label asks for multiples of 4.
  FreeCut requires only even values, which every listed software encoder
  accepts; a hardware encoder that needs coarser alignment is handled by
  EX-034's fallback to software.
- Aspect lock: on (default). With it off, the content is fitted inside the
  output, and the bars use the canvas background (the same rule as EX-042
  "Fit").
- 8K: offered when the GPU texture limit is at least 8192 and an encoder that
  supports it is present (software x265, SVT-AV1, x264 at level 6.x).

**Behavior / output:** Same rendering rule as EX-024. 8K shows a warning about
render time and file size in the estimate (EX-037).

**Edge cases:**
- 8K with H.264: allowed through software x264 (level 6.0–6.2). Many players
  cannot decode it, and a note recommends HEVC or AV1.
- Custom size with a different aspect and the lock off: preflight explains
  the letterbox.
- Sizes over the GPU limit are refused, with the limit shown.

**Undo:** n/a.

**Model notes:** Extends `OutputSize::Custom`. Fit-into-different-aspect
reuses the output-stage reframing from EX-042.

### EX-026 Output frame rate — mvp

**What the user sees:** A frame-rate drop-down: "Project (30 fps)" (default),
23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60. Choosing a rate other than the
project's shows a small note about how frames will be converted.

**Inputs / controls:**
- Options as above, plus the project rate if it is not in the list (for
  example 120 or 15).
- The MVP shows only "Project (<rate>)", so MVP exports never convert frame
  rates. The other entries, the conversion note and the related preflight info
  arrive in v1.
- Labels: NTSC rates are shown as 23.976, 29.97 and 59.94; whole rates as
  integers. The project entry uses the same label ("Project (29.97 fps)").
- Reference: the reference editor's public pages describe frame-rate choice at
  export with up to 60 fps.
- Reference (observed): its list is 24, 25, 29.97, 30, 50, 59.94 and 60 fps,
  with fractional rates shown to two decimals. FreeCut's v1 list is a
  superset (it adds 23.976 and 48, and the project rate).

**Behavior / output:**
- Output is always constant frame rate.
- Output frame `n` is evaluated at `S + n/R` (exact rationals). Each clip then
  picks its source frame by FX-001 L1/TR-041. A lower output rate skips
  frames. A higher one repeats frames (no interpolation; optical flow is
  TR-042, later).
- Frame count: `ceil(duration × R)`. The audio length is exactly the range
  duration. A container may report up to one frame of difference between
  streams, which is harmless.

**Edge cases:**
- Non-integer ratio, such as a 25 fps project exported at 30 fps: uneven
  cadence (judder). The note warns about it.
- NTSC rates (24000/1001 and so on): kept exact in the container timebase.
  They are never rounded to 23.98.
- A project rate above 60 fps exported at 120: allowed if the encoder and
  level allow it.
- Very low project rates (for example 12 fps stop-motion): offered as
  "Project (12 fps)".
- Export rate higher than every source: an info note says frames will repeat.

**Undo:** n/a.

**Model notes:** `FrameRate` comes from TL-001. `export_frame_times` (EX-022)
handles any rational rate. Tests cover 30→29.97 and 25→30 cadence, with no
lost or duplicated time across 1 h.

### EX-027 Quality presets (bitrate tiers) — mvp

**What the user sees:** A "Quality" control with three choices: **Smaller
file**, **Recommended** (default) and **Higher quality**. Each shows its video
bitrate, for example "Recommended — 10 Mbps".

**Inputs / controls:**
- The three tiers. Custom rate control is EX-028.
- The control's tooltip explains the trade-off in one line: a higher bitrate
  keeps more detail and makes a bigger file; a lower one makes a smaller file
  with more visible compression.
- Reference (observed): the reference dialog's bitrate choice has a
  recommended (automatic) setting plus constant, variable and average
  bitrate modes, with help text on the same trade-off. FreeCut's
  Recommended tier is the automatic setting; the modes are EX-028.

**Behavior / output:**
- The bitrate formula is our own (FreeCut default):
  `recommended_mbps = 10 × (W×H / 2 073 600)^0.85 × fps_factor × codec_factor`,
  where:
  - `fps_factor` is 1.0 at ≤ 30 fps, 1.5 at > 30–60 fps and 2.25 above 60;
  - `codec_factor` is 1.0 for H.264, 0.6 for HEVC, 0.6 for VP9 and 0.5 for AV1.
- Tiers: Smaller = ×0.5, Recommended = ×1, Higher = ×2. The result is rounded
  to 0.1 Mbps, with a minimum of 0.5 Mbps.
- Resulting H.264 values at ≤ 30 fps:

  | Size | Recommended |
  |---|---|
  | 480p | ≈ 2.5 Mbps |
  | 720p | ≈ 5 Mbps |
  | 1080p | 10 Mbps |
  | 1440p | ≈ 16 Mbps |
  | 2160p | ≈ 32 Mbps |
  | 4320p | ≈ 106 Mbps |

- Encoding is single-pass VBR at that target. The peak is capped at 1.5×
  target, with a buffer of 2× target. Keyframes come every 2 s with a closed
  GOP (FreeCut defaults), which suits streaming platforms and seeking.
- ProRes ignores tiers. Its rate is set by the profile (EX-030), and the
  control shows "set by profile".

**Edge cases:**
- Very static content (slides): the VBR encoder uses less than the target. The
  estimate is an upper bound and is labelled "up to".
- Portrait 1080×1920 has the same pixel count as 1920×1080, so it gets the
  same bitrate.
- Hardware encoders (EX-034) use the same targets.

**Undo:** n/a.

**Model notes:** `QualitySetting { Tier(QualityTier), Custom(RateControl) }`.
`recommended_kbps(w, h, fps: FrameRate, codec) -> u32` is pure, in
`freecut-project::export`, with a table test. Float math there is not time
math, so it is allowed. The result is stored as integer kbps.

**Implementation note (2026-10-12):** because `freecut-media` must not
depend on `freecut-project`, the H.264 formula currently lives in
`freecut_media::encode::{recommended_kbps, tier_kbps}` (pure, table-tested
against the table above; 1440p, 2160p and 4320p come out at 16.3, 32.5 and
105.6 Mbps). `EncodeSpec::video` is `VideoQuality::Tier(QualityTier)` or
`VideoQuality::TargetKbps(u32)`. The x264 mapping is: `b` = target,
`maxrate` = 1.5 × target, `bufsize` = 2 × target, `g` = round(2 s × fps),
preset `medium`, profile `high`, up to 16 threads; x264's GOPs are closed by
default. `freecut-project::export` can take the formula over (and add the
codec factor) once the export settings model exists.

### EX-028 Custom rate control — v1

**What the user sees:** "Custom…" in Quality opens rate-control options:
target bitrate, constant bitrate, constant quality or target file size. It
also has a two-pass checkbox and advanced encoder settings.

**Inputs / controls:**
- Mode:
  - **Target bitrate (VBR)**: 0.5–300 Mbps; default the Recommended value.
    A **Cap peaks** checkbox (default on) limits the peak to 1.5× the target
    with a 2× buffer, as EX-027 does. Turned off, the encoder only keeps the
    average over the whole file (plain average bitrate), which suits
    file-size goals but not streaming.
  - **Constant bitrate (CBR)**: same range.
  - **Constant quality**: an encoder-specific scale. Defaults: x264 CRF 20,
    x265 CRF 24, SVT-AV1 CRF 32 (0–63), VP9 CRF 31 (0–63). Hardware encoders
    use their CQ/ICQ equivalents.
  - **Target file size**: in MB. Bitrate = `size × 8 / duration − audio
    bitrate`, clamped to at least 0.3 Mbps. Two-pass is forced on for
    software encoders.
- Two-pass: default off. Software encoders only.
- Keyframe interval: 0.5–10 s (default 2 s).
- Encoder speed: faster / balanced (default) / slower, mapped to each
  encoder's presets (x264: veryfast / medium / slow).
- Comparison: Kdenlive offers a quality slider, encoder speed and two-pass.
  OpenShot's advanced tab offers a bitrate or CRF.
- Alternative seen in the reference app: variable (VBR) and average (ABR)
  bitrate are separate modes next to constant bitrate. FreeCut folds both
  into Target bitrate with the Cap peaks checkbox, and adds constant quality
  and target size.

**Behavior / output:**
- Two-pass renders the timeline twice. Progress shows "Pass 1/2" and "Pass
  2/2", and the ETA doubles.
- A target size that cannot be reached at the minimum bitrate gives a warning,
  and the export proceeds at the minimum.

**Edge cases:**
- Target size smaller than the audio alone: refused, with the reason.
- CBR with very low values at 4K: allowed. Preflight warns about visible
  artifacts below 25 % of Recommended.
- A mode the selected hardware encoder doesn't support: hardware is disabled
  for this export, with a note.

**Undo:** n/a.

**Model notes:** `RateControl { Vbr { kbps, cap_peaks: bool }, Cbr { kbps }, ConstQuality { value: u8 }, TargetSize { bytes }, two_pass: bool, gop_ms: u32, speed: EncoderSpeed }`.
`freecut-media::encode` maps these to FFmpeg options for each encoder. Unit
tests cover the size→bitrate math.

### EX-029 Baseline format: H.264 + AAC in MP4 — mvp

**What the user sees:** In the MVP every video export is an `.mp4` with H.264
video and AAC audio that plays on effectively every device and site. The
codec and format rows show these values read-only.

**Inputs / controls:** None in the MVP beyond quality, size and frame rate.

**Behavior / output:**
- **Video:**
  - H.264 High profile, 8-bit 4:2:0, level chosen automatically from size and
    frame rate;
  - progressive, closed GOP every 2 s, B-frames at the encoder default;
  - color tags BT.709 (EX-032).
- **Encoder:** libx264 through FFmpeg. Its GPL-2.0-or-later license is
  compatible with FreeCut's GPL-3.0-or-later.
  - If the linked FFmpeg lacks libx264 (some distribution builds), FreeCut
    falls back to OpenH264 (BSD-2-Clause) or a hardware encoder (EX-034).
  - If no H.264 encoder exists at all, Export explains the problem and offers
    other codecs once v1 ships.
- **Audio:** AAC-LC through FFmpeg's native AAC encoder (LGPL). The
  non-free FDK-AAC library is never used.
- **Container:** MP4 with fast start, and the `avc1` sample entry.

**Edge cases:**
- Sizes or frame rates beyond the highest H.264 level: the closest valid level
  is used. If the size itself is invalid (wider than 8192), it is refused
  (EX-024).
- A timeline with no audible clip: see EX-031.
- Files over 4 GB: 64-bit chunk offsets are used automatically by the muxer.

**Undo:** n/a.

**Model notes:** `freecut-media::encode::{VideoEncoder, AudioEncoder, Muxer}`
wrap `ffmpeg-next`. `EncoderCaps::probe()` runs once in the background and
lists the available encoders. All `unsafe` stays inside `freecut-media`, with
`// SAFETY:` comments.

**Implementation note (2026-10-12):** built as one type,
`freecut_media::encode::EncodeSession` (`create(path, &EncodeSpec)`,
`push_video(rgba, frame_index)`, `push_audio(interleaved_f32)`,
`finish(&CancelToken)`, `abandon()`), plus `verify_output(path) ->
OutputInfo` and `OutputInfo::check(&spec, frames, samples)` for the job's
final check (P6). Behaviour:
- the muxer is named (`mp4`), the path is handed to FFmpeg as `file:<path>`
  and must be UTF-8 (FFmpeg reopens it by name for the fast-start pass);
- `GLOBAL_HEADER` is set before the encoders open, and any encoder or muxer
  option FFmpeg leaves unused is an error (`UnknownOption`);
- frames must arrive in order from 0; frame `n` has pts `n` in `1/fps`;
- packets go through `av_interleaved_write_frame`; producers push whichever
  stream is behind (`video_due_before_audio`, exact integer comparison);
- `finish` flushes video and audio, writes the trailer (fast start) and
  checks the final close; it deletes the file on any error or cancel;
  `abandon`, and dropping an unfinished session, delete it too;
- `ENOSPC`/`EFBIG` from FFmpeg map to `DiskFull`/`FileTooLarge`.
Not built yet: `EncoderCaps::probe()` and the OpenH264/VideoToolbox fallback
when libx264 is missing (today: `EncoderUnavailable("libx264")`).

### EX-030 More codecs and containers, and 10-bit output — v1

**What the user sees:** Codec and Format become drop-downs. Choosing one
filters the other, so only valid pairs can be picked:

| Codec | Containers | Bit depth | Audio default |
|---|---|---|---|
| H.264 | MP4, MOV, MKV | 8-bit | AAC (PCM in MOV optional) |
| HEVC (H.265) | MP4, MOV, MKV | 8 or 10-bit | AAC |
| AV1 | MP4, MKV, WebM | 8 or 10-bit | AAC (MP4/MKV), Opus (WebM) |
| VP9 | WebM, MKV | 8 or 10-bit | Opus |
| ProRes (Proxy, LT, 422, HQ) | MOV | 10-bit 4:2:2 | PCM 24-bit |

**Inputs / controls:**
- Codec, Container and Bit depth (where offered; default 8-bit, except
  10-bit for ProRes).
- ProRes profile (default 422).
- Each codec entry has a one-line description in FreeCut's words:
  - H.264: "Plays almost everywhere";
  - HEVC: "Similar quality in roughly half the space; newer devices";
  - AV1: "Smallest files; slow to encode, newer players";
  - VP9: "Open web format";
  - ProRes: "Editing master; very large files".
- Choosing 10-bit adds a note that encoding takes longer and that it reduces
  banding in gradients. 10-bit here is still SDR Rec.709 (EX-032).
- Reference: public pages for the reference editor describe MP4 and MOV
  output.
- Reference (observed): its dialog has a codec section in which each codec
  has a one-line description: a general-purpose default and a
  high-efficiency codec described as saving space, plus alpha-capable codecs
  (EX-048). The labels do not list the containers. Its bit-depth choice is 8
  or 10-bit, with 10-bit recommended for HDR and noted as slower.
  Alternative seen in the reference app: choosing a bit depth that differs
  from the project's warns that quality may shift. FreeCut has no project bit
  depth (the working space is always 16-bit float, CL-002), so it shows no
  such warning.

**Behavior / output:**
- Encoders, all through FFmpeg:
  - HEVC: libx265 (GPL-2.0+);
  - AV1: SVT-AV1 (BSD-3-Clause-Clear plus the AOMedia patent license), with
    libaom (BSD-2-Clause) as fallback;
  - VP9: libvpx (BSD-3-Clause);
  - ProRes: FFmpeg's own independent encoder (`prores_ks`, LGPL);
  - Opus: libopus (BSD-3-Clause).
- HEVC in MP4/MOV is tagged `hvc1` for Apple-player compatibility.
- 10-bit output renders from the 16-bit-float working texture and converts
  straight to a 10-bit format (`yuv420p10` or `yuv422p10`), with no 8-bit
  step in between.
- MKV accepts any listed codec and also FLAC or Opus audio.
- Preflight warns about compatibility risks: ProRes and AV1 on older phones,
  HEVC on some older browsers, VP9/WebM in Apple apps (EX-036).

**Edge cases:**
- Switching codec to one the current container can't hold: the container
  switches to the first valid one, and a toast says so.
- A preset (EX-041) names a codec missing on this machine: that preset is
  disabled, with the reason.
- 10-bit H.264: not offered, because player support is poor.
- ProRes at 8K: allowed. The very large size estimate is shown.

**Undo:** n/a.

**Model notes:** `VideoCodec`, `Container` and `BitDepth` enums, with a pure
compatibility table `allowed(codec, container) -> bool` in
`freecut-project::export`, tested exhaustively. Encoder mapping lives in
`freecut-media`.

### EX-031 Audio settings for video exports — mvp

**What the user sees:** An Audio section with a checkbox, **Include audio**
(on), a bitrate drop-down and the sample rate.

**Inputs / controls:**
- Include audio: default on. If the timeline has no audible clip, it defaults
  off with the note "Timeline is silent". An "Add silent track" option is
  offered, because some sites reject video without audio.
- Codec: AAC (MVP). v1 adds:
  - PCM 16 or 24-bit (MOV/MKV);
  - Opus (WebM/MKV; 64–320 kbps, default 160);
  - FLAC (MKV).
- AAC bitrate: 96, 128, 192 (default), 256 or 320 kbps.
- Sample rate: the project rate from PM-007 (48 kHz default, 44.1 kHz
  allowed). The other rate is offered with resampling.
- Channels: stereo. Mono is v1.
- Reference (observed): the reference dialog's audio settings offer AAC at
  192, 256 or 320 kbps, or PCM, with selectable sample rate and bit depth.
  It also carries Dolby-family codec entries with blank labels, which look
  hidden; FreeCut leaves those out (see Out of scope). FreeCut's AAC list adds
  96 and 128 kbps for small files.

**Behavior / output:**
- The master bus is rendered offline through `freecut-audio` (AU-010), over
  exactly the export range, then encoded and interleaved by the muxer.
- Solo is ignored unless "Export as heard" is ticked (AU-012).
- The master limiter applies as in preview.

**Edge cases:**
- Clips with different sample rates: already resampled by the mixer (AU-010).
- A range that starts or ends in the middle of a fade: the audio is cut at the
  range edge. No fade is added (FreeCut default).
- Long silence at the head: kept as silence. FreeCut never trims.

**Undo:** n/a.

**Model notes:** `AudioExportSettings { include: bool, codec: AudioCodec, kbps: u32, sample_rate: u32, channels: Channels, silent_if_empty: bool }`
in `freecut-project::export`. It is shared with the audio-only export
(AU-032, EX-044).

**Implementation note (2026-10-12):** `freecut_media::encode::AudioSpec
{ sample_rate: SampleRate, channels: 2, kbps }`. Audio is pushed as
interleaved stereo `f32` at the output rate in any block size; a FIFO
re-chunks it into 1024-sample AAC-LC frames and sends one short last frame,
so the stream ends at the pushed length (the MP4 edit list hides the
encoder's priming). Non-finite samples are encoded as silence. A 2 s test at
48 kHz reads back as exactly 2.000 s.

### EX-032 Output color baseline: BT.709 tags, 8-bit, dithering — mvp

**What the user sees:** Nothing to set. Exported colors match the preview in
standard players and on websites. Gradients don't band harshly.

**Inputs / controls:** None in the MVP; the dialog shows "Color: Rec.709
(SDR)" read-only (EX-021). v1 adds an Advanced toggle, "Full range output"
(default off; only for codecs and containers that signal it), and bit depth
(EX-030).
- Alternative seen in the reference app: a color-space choice, with HDR
  options, and a warning that a non-HDR monitor may preview colors
  differently from the exported file. FreeCut has one output space (SDR
  Rec.709) and tone-maps HDR sources at decode for preview and export alike,
  so the preview already matches the file and no such warning is needed.

**Behavior / output:**
- A WGSL shader converts the working texture (CL-002, FX-001) to Y′CbCr with
  the BT.709 matrix and limited range (16–235 / 16–240 at 8-bit).
- Chroma is downsampled to 4:2:0 with left-sited samples (MPEG-2 style),
  using a box filter.
- **Dithering:** a deterministic blue-noise or ordered dither (seeded by the
  frame index) is applied when quantizing the 16-bit float working values to
  8 bits. The same input gives the same output.
- Streams are tagged: primaries BT.709, transfer BT.709, matrix BT.709, range
  limited (CL-002). Containers get matching color atoms (`colr` in MP4/MOV).

**Edge cases:**
- HDR sources: already tone-mapped to SDR at decode (CL-002). The export
  stays SDR Rec.709; HDR output is out of scope (see Out of scope).
- Wide-gamut images: treated as Rec.709 (CL-002 limitation).
- GIF and image outputs bypass Y′CbCr and use RGB (EX-043, EX-047).
- Players that ignore tags see BT.709 anyway, because it is the HD default.

**Undo:** n/a.

**Model notes:** `OutputColor { matrix: Bt709, range: Limited | Full, depth: Eight | Ten }`.
The shader lives in `freecut-render::output`. The tags are applied by
`freecut-media`. A test renders a known RGB ramp and checks the Y′CbCr code
values against the BT.709 formulas.

**Implementation note (2026-10-12):** until the GPU converter exists, the
CPU path converts in `freecut-media`: `EncodeSession::push_video` takes
straight-alpha RGBA8 (non-opaque pixels are composited over black) and
converts it with swscale to BT.709 limited-range 4:2:0 with left chroma
siting, area (box) chroma filter, accurate rounding and one thread, so the
output is deterministic. No dither is added at this step: the input is
already 8-bit, so EX-032's dither belongs to the renderer's float → 8-bit
quantization. Codec context, stream and `colr` box all carry
709/709/709/limited; a unit test checks black, white, primaries, grey and an
orange against the BT.709 formulas (±1 code value).

### EX-034 Hardware-accelerated encoding — v1

**What the user sees:** A **Hardware encoding** row: Auto (default), On, or
Off. With Auto or On, exports use the GPU's video encoder when one exists for
the chosen codec and size. They are often several times faster. The row
names the encoder in use, for example "Using: Apple VideoToolbox" or "Using:
NVIDIA NVENC".

**Inputs / controls:**
- Auto / On / Off. "On" shows an error, rather than a fallback, if no
  hardware encoder exists.
- The preference lives in the app (global) and can be overridden per export.
- Comparison: Kdenlive and OpenShot both ship hardware presets (NVENC, VAAPI,
  QSV, VideoToolbox) and fall back to CPU presets on failure.
- The app-wide preference sits in Preferences ▸ Performance next to EX-018's
  hardware decoding. Changes apply to the next export; nothing needs a
  restart or a project reopen.
- Alternative seen in the reference app: a "faster export" switch warns that
  colors may differ slightly. In FreeCut the RGB→Y′CbCr conversion is always
  FreeCut's own shader (EX-032), so hardware encoding changes compression
  efficiency only, never color.

**Behavior / output:**
- Backends through FFmpeg:
  - VideoToolbox (H.264, HEVC, and ProRes on Apple silicon);
  - NVENC (H.264, HEVC, and AV1 on newer GPUs);
  - Intel QSV (H.264, HEVC, AV1);
  - AMD AMF on Windows (H.264, HEVC, AV1);
  - VAAPI on Linux (driver-dependent).
- **Probe:** at app start, in the background, FreeCut opens each candidate
  encoder with a tiny test frame (256×256) and records success. The result is
  cached per app version and driver version.
- **Fallback:** if a hardware encoder fails to open, or fails mid-export,
  Auto restarts the job from the beginning with the software encoder and says
  so in the result view.
- Quality tiers map to the same bitrates (EX-027). Constant-quality mode maps
  to each encoder's CQ/ICQ.
- **Stall watchdog:** if no frame has been muxed for 60 s (FreeCut default)
  while the job is not waiting on an analysis, the progress view shows
  "Export seems stuck" with **Keep waiting**, **Retry without hardware
  acceleration** (restarts the job with hardware decoding and encoding off
  for this job only) and Cancel. With software paths only, the middle choice
  is replaced by **Retry**. This is the `stalled` failure of EX-022.

**Edge cases:**
- Size beyond the encoder's limit (for example H.264 wider than 4096 on some
  GPUs): the software encoder is used for this job, and the dialog says so.
- Too many concurrent encode sessions (consumer GPUs): fallback.
- Two-pass and target-size modes: software only. Hardware is disabled, with a
  note.
- Output is not bit-identical across machines, which is expected and
  documented.
- Slow but steady jobs (for example 8K AV1 in software) do not trigger the
  watchdog as long as each frame finishes within 60 s; two-pass jobs reset it
  at the start of each pass.

**Undo:** n/a.

**Model notes:** `HwEncodePref { Auto, On, Off }` and
`freecut-media::encode::HwEncoderCaps`. The watchdog is a pure timer over
"last frame muxed" and "waiting on analysis" events in `freecut-app::jobs`,
tested with a fake clock. Device contexts are FFI inside
`freecut-media`, with SAFETY comments. The probe runs on a worker thread
(never at UI start-up on the main thread).

### EX-035 Export range: whole timeline, in–out, selected clips — v1

**What the user sees:** A Range choice:
- **Whole timeline** (default);
- **In–out range** (preselected whenever in/out marks exist);
- **Selected clips span**.

The duration of the chosen range is shown.

**Inputs / controls:**
- The three choices.
- The in/out marks themselves are set with I/O on the timeline (TL-038).
- Reference: the reference editor's help center describes setting start and
  end marks (with the I and O keys, or Shift+X) so that an export covers just
  the span between them. Kdenlive
  also offers "selected zone" and marker-zone ranges, and OpenShot offers
  start and end frames.

**Behavior / output:**
- Whole timeline = `[0, duration)` (TL-003).
- In–out = `[in, out)` with TL-038 rules (In only → to the end; Out only → from
  0).
- Selected clips span = `[min start, max end)` over the selected clips. Every
  visible track inside that span is rendered, not only the selected clips
  (FreeCut default; an "only selected clips" variant may be added later).
- The output always starts at 00:00:00:00, rebased. Captions (TX-033), chapters
  (EX-049) and cover times are rebased too.

**Edge cases:**
- A range partly beyond `duration`: clipped to `duration`. A range entirely
  beyond it: Export is disabled.
- A range edge inside a transition: frames render exactly as on the timeline
  (half-transition). No handles are added.
- Selection empty or containing only markers: that choice is disabled.
- Range shorter than one output frame at a lower output rate: rounded up to
  one frame.
- In–out chosen while content exists outside it: the range row says so in
  one line ("Only 00:00:12:00–00:00:47:00 will be exported"), with a link
  that switches to Whole timeline. Clearing the marks themselves is TL-038's
  Clear range command (key: see UI-031). Reference (observed): the reference
  dialog warns that content outside the export range is left out and
  explains how to clear the range.

**Undo:** n/a.

**Model notes:** `ExportRangeKind { Whole, InOut, SelectionSpan }`, plus a pure
`resolve_export_range(seq, &RangeMarks, &Selection, kind) ->
Result<TimeRange, RangeError>` in `freecut-timeline`. It is shared with loop
(EX-008).

### EX-036 Export preflight checks — mvp

**What the user sees:** A line in the export dialog footer: "Ready", or "2
warnings · 1 problem". Expanding it lists each issue with a severity icon.
Clicking an issue jumps the playhead or selects the clip concerned.
**Problems** block Export. **Warnings** don't, and Export proceeds after one
confirmation listing them.

**Inputs / controls:** Expand/collapse. "Relink…" for offline media (PM-091).
"Don't warn again for this project" for informational items only.

**Behavior / output:** Checks (MVP set first, extended as features ship):
- **Problems (block):**
  - empty timeline or empty range;
  - empty file name (EX-023);
  - destination not writable, missing, or on a disconnected drive;
  - target file equals a project source;
  - output size exceeds the GPU or encoder limit with no fallback;
  - no encoder available for the chosen codec;
  - free disk space below 100 MB; the message names the volume and how much
    to free ("Free at least 1.2 GB on 'Data', or choose another folder");
  - GIF longer than 60 s (EX-043);
  - logo file missing (EX-059, later).
- **Warnings:**
  - offline media (lists clips and times, with **Relink…**; slates will
    render with silence, EX-005, PM-090). Missing video renders the offline
    slate and missing audio is silent; the list says which applies to each
    clip;
  - media in online-only cloud placeholders (PM-114, v1): reading them makes
    the sync client download them, which can be slow or fail offline. The
    warning lists them with their sizes and suggests making them available
    offline first;
  - estimated size above the destination file system's per-file limit
    (FAT32 4 GB, FAT16 2 GB; v1, with EX-037);
  - proxies standing in for missing originals (PM-081); the export is marked
    "contains proxy media";
  - estimated size above free space (v1, with EX-037);
  - fonts substituted (TX-007);
  - missing LUTs or looks (CL-016/017);
  - analyses still pending (AI-001; export will run them first, with an
    estimated time);
  - solo active (AU-012);
  - every visual track hidden (TL-008): the video will show only the canvas
    background;
  - very long GIF (EX-043);
  - bitrate far below Recommended (EX-028).
- **Info:**
  - frame-rate conversion (EX-026);
  - upscaling beyond the largest source (EX-024);
  - hidden or muted tracks (TL-008) contain clips;
  - output not widely playable (EX-030).

- Reference (observed): the reference editor refuses to export an empty
  timeline; asks to relink missing media, letting missing video export as
  black or relinking missing audio; makes export wait for media that is
  still downloading and for unfinished background processing, with a link to
  its task list; computes some heavy effects during export with their own
  status labels; and checks disk space up front, stating how much to free.
  Alternative seen in the reference app: missing video exports as black
  frames; FreeCut renders the offline slate so the gap is visible. Its
  cloud-download wait has no FreeCut equivalent beyond the placeholder
  warning above.

**Edge cases:**
- Checks finish in under 100 ms for typical projects. Expensive checks (for
  example scanning thousands of clips) run on a worker thread, and the line
  shows "Checking…".
- The OS does not report placeholder state for a volume: no placeholder
  warning; the export simply reads the files.
- The destination's file system type is unknown (some network shares): no
  per-file-limit warning; a write failure is still reported as
  `file-too-large` (EX-022).
- The project changes while the dialog is open (only possible in v1 with
  background jobs): checks re-run.
- Background queue (EX-038): checks run at queue time and again at job start.
  New problems fail the job with the list.

**Undo:** n/a.

**Model notes:** `Issue { severity, code: IssueCode, message, target: Option<IssueTarget> }`.
The pure checks are in `freecut-timeline` (structure) and
`freecut-project::export::validate` (settings). Checks needing I/O (disk,
encoders, fonts) are gathered in `freecut-app` and passed in as data, so
every rule is unit-testable.

**Implementation note (2026-10-12):** `validate(&ExportSettings, &Document,
SeqId, &PreflightEnv) -> Vec<Issue>`. `PreflightEnv` carries the folder
state, free bytes and volume name, the GPU texture limit, whether an H.264
encoder exists, the offline media ids and the substituted font families.
`Issue { severity, code, targets }` has no message: `IssueCode::id()` is a
stable Fluent id (`export-low-disk-space`, …) and the variant carries the
data. Built: every MVP problem in the list above except "GIF longer than
60 s" and "logo file missing" (both later), plus unsupported frame rate,
AAC bitrate and sample rate (hand-edited files); the warnings for offline
media, substituted fonts, every visual track hidden and size above free
space; and the three info items for frame-rate conversion, upscaling and
hidden or muted tracks. Problems come first, then warnings, then info. The
target-is-a-source check compares paths without regard to case.

### EX-037 Size, duration and disk-space readout — v1

**What the user sees:** In the footer: output duration, for example "Duration
02:13.40"; an estimate, for example "Size ≈ 168 MB" (or "up to 168 MB" for VBR
and "varies" for constant quality); and "Free: 112 GB on Macintosh HD".

A summary line under it lists resolution (with aspect ratio, for example
"1080×1920 · 9:16"), frame rate, codec and container, bitrate or rate mode,
color ("Rec.709 SDR, 8-bit") and audio. Hovering the size explains that it is
an estimate.

**Inputs / controls:** None. The MVP has no readout; it keeps only EX-036's
low-disk-space problem check.
- Reference (observed): the reference dialog's summary panel shows duration,
  an estimated size that is always labelled as an estimate, resolution,
  bitrate, codec, format, color space, frame rate and aspect ratio.

**Behavior / output:**
- The size is never shown without "≈", "up to" or "varies", because it is
  always an estimate.
- Estimate = `(video kbps + audio kbps) × seconds / 8 × 1.01` (container
  overhead).
- With companion outputs (EX-057), the size is the sum of all files, and the
  tooltip lists each one.
- ProRes uses a per-profile table of approximate rates scaled by pixels and
  fps (our own measurements).
- GIF uses a heuristic based on pixels, fps, duration and colors, shown with
  "≈".
- Free space is read from the destination volume when the dialog opens and
  after a folder change, off the UI thread.

**Edge cases:**
- Constant-quality and target-size modes: the target-size mode shows the
  target, constant quality shows "varies".
- Network folders where free space is unknown: "Free: unknown".
- Durations over 24 h: shown as `HH:MM:SS`.

**Undo:** n/a.

**Model notes:** `estimate_size(&ExportSettings, duration: TimeRange) ->
SizeEstimate` is pure, in `freecut-project::export`.

### EX-038 Background export and export queue — v1

**What the user sees:** Next to Export, an **Add to queue** button (and
"Export in background" as the default action once enabled in preferences).
The dialog closes, and a jobs indicator in the status bar shows progress. You
can keep editing, switch projects, or queue more exports, each with its own
settings. A Jobs panel lists every job with its status, progress and actions.

**Inputs / controls:**
- Add to queue.
- Jobs panel actions: Cancel, Remove (finished jobs), Move up/down (queued),
  Show in folder, Open, Retry (failed).
- Preference "Run jobs: one at a time" (default) or two at once.
- Preference "Notify when finished" (default on; uses OS notifications when
  FreeCut isn't focused).
- Preference "Play a sound when an export finishes or fails" (default off;
  FreeCut default). It uses the OS's standard notification sound, so FreeCut
  ships no sound asset for it; where the OS offers none (some Linux
  desktops), the option is hidden.
- Row action **Open project** for a job whose project is not the open one.
- Comparison: Kdenlive renders through a job queue and lets you keep editing.
  Changes made after submission don't affect a queued job.
- Reference (observed): in the reference editor an export keeps running when
  the user returns to the home screen or opens another project; one export
  runs at a time and the rest wait; the home screen lists each job as
  exporting, waiting, done or failed, and a failed job can be opened to see
  its error; editing may lag during an export and the app says so; and a
  second export-type action during an export reports that one is already
  running. FreeCut's "one at a time" default matches; "two at once" is a
  FreeCut option.

**Behavior / output:**
- Each job holds the immutable document snapshot taken at queue time
  (EX-022), plus resolved media paths. Later edits never affect it.
- Jobs run in order. Preview playback continues during a job but may drop
  frames, because export work uses below-normal thread priority while you are
  playing back (FreeCut default).
- The queue is saved in app state. After a restart, unfinished queued jobs are
  listed as "Interrupted", with Retry.
- Keep-awake (EX-022) applies while any job runs.
- The start screen (PM-001) shows a compact export strip while any job is
  queued, running, failed or finished this session: one row per job with
  its state (Exporting 42 %, Waiting, Done, Failed). Clicking a failed row
  opens its failure details (code, message, fixes, Copy details; EX-022).
- The first time in a session that a job runs while the editor is open, an
  info toast says that playback may be less smooth until the export
  finishes.
- Other GPU work while a job runs: pre-render (EX-016) pauses and resumes
  afterwards; snapshots (EX-014) and cover renders (EX-046, EX-053) run at
  once, ahead of export frames, because they are single frames.
- A second export started while one runs is queued, never refused; the
  toast says "Added to the queue (2nd)". (The MVP's export is blocking, so
  the question does not arise there.)

**Edge cases:**
- A source file deleted or moved after queueing: that clip renders the offline
  slate (EX-005). The job completes "with warnings" and lists it (FreeCut
  default), unless preflight at job start finds it first.
- Closing the project while its job runs: allowed, because the job owns its
  snapshot. If the job still waits for analyses (AI-001), they keep running
  in the analysis pool for the job; nothing blocks closing. Alternative seen
  in the reference app: some processing must finish before the user may
  leave the editor.
- Quitting the app with jobs running or queued: a prompt to wait, or to cancel
  and quit.
- Memory use: a snapshot shares unchanged model data with the live document
  (structural sharing through `Arc`). Very large projects show the snapshot
  size in the Jobs panel tooltip.
- Two jobs targeting the same file: resolved at queue time (EX-023).

**Undo:** n/a.

**Model notes:**
- `freecut-app::jobs::JobQueue` with `JobState { Queued, Running(progress), Done(result), Failed(error), Cancelled, Interrupted }`.
- Snapshots rely on the timeline model being cheaply clonable (`Arc`-shared
  nodes), which is a TL design requirement.
- Queue persistence is a small JSON file in the app data dir, written through
  `freecut-project` serde types.

### EX-039 Export finished: result view and actions — mvp

**What the user sees:** When an export completes, the progress view becomes a
result view. It shows the file name, size, duration, resolution and frame
rate, codec, and how long the export took. Buttons: **Show in folder**,
**Open**, **Copy path** and **Done**. Notes appear if anything changed during
the job (for example "Hardware encoder failed; finished with software
encoder").

**Inputs / controls:** The buttons. v1 adds "Export again with other settings"
and **Change cover…** (EX-046). With companion outputs or sidecar files
(EX-057, EX-045, EX-046), the view lists every file written, each with its
own Show in folder.
- Reference (observed): after success the reference editor offers to open
  the file or its folder and notes that the file is saved on this computer
  and ready to share.

**Behavior / output:**
- "Show in folder" reveals the file in Finder, Explorer or the Linux file
  manager.
- "Open" uses the OS default player.
- FreeCut offers **no** "share to" buttons and never uploads anything.
- In v1 background mode, the same information appears in the Jobs panel and
  in the OS notification.

**Edge cases:**
- File moved or deleted before clicking Open: an error toast, "File no longer
  exists".
- No default player registered (some Linux setups): the error is shown, and
  "Show in folder" still works.

**Undo:** n/a.

**Model notes:** UI in `freecut-app`. The "reveal" and "open" calls use the
OS opener on a worker thread.

**App implementation note (2026-10-12):** the result view shows the file
name, size, duration, resolution and frame rate, "H.264 video, AAC audio,
MP4" (or "no audio") and the export time, with **Show in folder**,
**Open**, **Copy path** and **Done**. Open and Show in folder run on the
worker pool: `open` / `open -R` on macOS, `explorer` / `explorer
/select,<path>` on Windows, `xdg-open` on Linux (Show in folder opens the
containing folder there). The path is always one argument, never passed
through a shell. A file that no longer exists reports "File no longer
exists" in the status line.

### EX-040 No watermark, no paywall, no account, no upload — mvp

**What the user sees:** Exports contain exactly the timeline:
- no logo, watermark, intro or end card is ever added;
- no resolution, frame rate, codec or length is reserved for a paid tier;
- no sign-in is asked for;
- export works fully offline.

**Inputs / controls:** None. This is a product guarantee recorded so it can be
tested.

**Behavior / output:**
- The render path has no code that adds content beyond FX-001's layers.
  Bundled templates (OV) contain no FreeCut branding clips.
- **Metadata privacy:** by default the file contains only technical metadata
  (FFmpeg's encoder tag, creation time = export time, and title = the name
  field). It never contains user names, machine names, file paths, location
  or project internals.
- Optional fields (title, artist, comment, copyright) come in EX-046.
- Reference: per the reference editor's help center, the mark seen in some
  template-based videos is a closing segment or template layer that sits on
  the timeline; the export step does not stamp it on, and deleting that
  segment gets rid of it. FreeCut adds no such segment or layer.
- Reference (observed): the reference dialog also has a brand-watermark
  placement (off or one of four corners) that a paid plan removes, and an
  "AI-generated" mark on generated segments that cannot be hidden. FreeCut
  has neither (see Out of scope). The only mark FreeCut can put on an export
  is the user's own logo, opt-in and off by default (EX-059, later).

**Edge cases:**
- Imported media that already contains a watermark is the user's content and
  is exported as-is.
- An "encoder" tag is required by some muxers. It names the library only.

**Undo:** n/a.

**Model notes:** A regression test renders a known timeline and asserts that
pixel output equals FX-001's reference output frame for frame, so there are
no extra frames and no overlay. It also checks that the container metadata
key set is within the allowed list.

**Implementation note (2026-10-12):** `EncodeSession` writes only
`creation_time` (UTC export time), `title` when `EncodeSpec::title` is set,
and FFmpeg's own `encoder` tag (the MP4 demuxer also reports the technical
`major_brand`, `minor_version` and `compatible_brands`). The integration test
asserts the container key set is within that list and that the packet count
equals the frames pushed.

The app's end-to-end export test (`freecut-app/src/e2e.rs`) checks the
same key set on a file written by the export job, and that the frame at 1 s
shows the timeline's text, centred, over the video.

### EX-041 Export presets: built-in and user — v1

**What the user sees:** A Preset drop-down at the top of the export dialog.
Built-in presets are listed first, then "Saved presets". "Save as preset…"
stores the current settings under a name. Presets can be renamed, deleted,
duplicated, set as the default, and exported or imported as files.

**Inputs / controls:**
- Built-in presets (our own names and values):
  - **Recommended** (match canvas, H.264/AAC MP4, Recommended tier);
  - **Small file** (720p short side, Smaller tier);
  - **High-quality master** (ProRes 422 HQ in MOV, PCM 24-bit);
  - **Efficient** (HEVC 10-bit MP4, Recommended);
  - **Open web** (VP9 or AV1 with Opus in WebM);
  - **Archive** (AV1 10-bit MKV, FLAC);
  - plus the platform presets in EX-042.
- A user preset stores everything except the file name, folder and range
  (FreeCut default), plus an optional name pattern using the tokens
  `{project}`, `{date}`, `{time}`, `{res}`, `{fps}` and `{preset}`.
- Import and export files use the extension `.freecut-preset.json`.
- Comparison: Kdenlive groups presets by category and supports custom,
  downloaded, editable and deletable presets (built-ins cannot be edited).
  OpenShot organises them by profile and target.

**Behavior / output:**
- Choosing a preset fills the controls. Editing any control switches the label
  to "Custom (from <preset>)".
- Presets are stored in the app config dir, not in projects. The project
  remembers only its last settings (EX-021).

**Edge cases:**
- A preset needs an encoder missing on this machine: listed but disabled, with
  the reason (EX-030/034).
- Importing a preset from a newer FreeCut: unknown fields are ignored and
  missing fields get defaults. A notice appears if something was dropped.
- Name collision on save or import: "Name (2)".
- Built-ins cannot be deleted or overwritten. "Duplicate" makes an editable
  copy.

**Undo:** n/a (app configuration). Deleting a user preset asks for
confirmation.

**Model notes:** `ExportPreset { id, name, schema_version, settings: PresetSettings }`
in `freecut-project::export`, with serde migrations, the same as project
files. Name-pattern expansion is a pure function.

### EX-042 Platform presets by aspect ratio — v1

**What the user sees:** A "For…" group in the Preset list with generic
destination presets. Each states its shape and size:
- **Vertical short video** — 9:16, 1080×1920, 30 fps;
- **Widescreen HD** — 16:9, 1920×1080, 30 fps;
- **Widescreen 4K** — 16:9, 3840×2160, 30 fps;
- **Square post** — 1:1, 1080×1080;
- **Portrait post** — 4:5, 1080×1350;
- **Classic 4:3** — 1440×1080;
- **Cinema wide** — 21:9, 2560×1080;
- **Messaging / small** — 720p short side, Smaller tier.

Tooltips mention example services each suits (descriptive only, no logos).
If the preset's shape differs from the canvas, the dialog shows a framing
choice:
- **Fit** (bars filled with the canvas background), the default;
- **Fill** (center crop);
- **Change canvas…**.

**Inputs / controls:**
- Preset choice.
- Framing choice: Fit / Fill. "Change canvas…" runs PM's canvas operation
  (EX-013).
- Frame rate: 30 fps, or "match project" when the project rate is 24, 25, 50
  or 60 (FreeCut default, which avoids conversion judder).
- Comparison: OpenShot ships targets for major video sites at 1080p 30 fps,
  with vertical variants. The reference editor's resize tool also lists common
  ratios per social platform.

**Behavior / output:**
- Fit and Fill are an **output-stage reframe**. The composited canvas is
  scaled into the output frame (Fit, bars filled with the canvas background
  color) or scaled to cover and center-cropped (Fill). The timeline is never
  edited, so one project can produce several shapes.
- Smart (subject-aware) reframing belongs to the AI area and is not part of
  these presets.
- When a vertical preset is chosen, the vertical-feed overlay (EX-012) is
  suggested once.
- FreeCut hard-codes no platform rules about length, size or upload limits.
  Such limits change often, so presets only set technical parameters.

**Edge cases:**
- Fill would crop text near the edges: preflight lists text and caption clips
  whose bounds fall outside the cropped area.
- Canvas already matches: no framing choice is shown.
- Blurred-background canvas fill (TR-012): with Fit, the bars use the canvas
  background *color* only. A note suggests changing the canvas instead to get
  blurred bars.

**Undo:** Fit/Fill: n/a (export setting). "Change canvas…" is PM's undoable
operation.

**Model notes:** `OutputFraming { Fit, Fill }` in `VideoSettings`. The reframe
is a final pass in `freecut-render::output` (one resample from canvas to
output, folded into the EX-024 rendering by adjusting the canvas-to-output
transform rather than resampling twice). Built-in platform presets are data
in `freecut-project::export::builtin_presets()`.

### EX-043 Animated GIF export — v1

**What the user sees:** Output type **GIF** shows its own options: size, frame
rate, loop, colors and dithering. The estimate warns when the file will be
large. GIFs have no audio.

**Inputs / controls:**
- Size (short side): 240, 320, 480 (default), 640 or 720 px. It never exceeds
  the canvas.
- Frame rate: 10, 12, 15 (default), 20, 24 or 25 fps.
- Loop: forever (default), play once, or N times (2–100).
- Colors: 256 (default), 128, 64 or 32.
- Dithering: none, ordered (Bayer), or error-diffusion (default).
- Palette: one global palette for the whole GIF (default), or per-frame
  palettes for footage whose colors change a lot.
- Range: as EX-035. GIFs are usually short, so In–out is preselected when
  marks exist.
- Reference (observed): the reference dialog has a GIF checkbox with two
  sizes, 320p and 640p, shown as two quality steps, and refuses content that
  is too long (the limit is not in the labels). FreeCut's size list includes
  both.
- Comparison: Kdenlive lists GIF among its image presets.

**Behavior / output:**
- Frames come from the same render path at GIF size.
- Encoding uses FFmpeg's two-pass palette generation and palette application,
  then the GIF encoder (LGPL), in `freecut-media`.
- Captions are burned in if their track is visible. Embedding is disabled
  (TX-033).

**Edge cases:**
- Duration over 15 s, or estimate over 50 MB: warning (EX-036).
- Duration over 60 s: a problem that blocks the GIF (FreeCut default), with
  the hint to choose an In–out range or a video format. GIF files grow very
  fast, and many viewers struggle with long ones.
- A single-frame range: a static GIF.
- Transparency: not supported, because the canvas is opaque. Alpha output is
  EX-048.
- Frame rates GIF cannot express exactly (frame delays are in 1/100 s):
  timing is spread so the total duration stays correct. 15 fps alternates 7
  and 6 hundredths of a second.

**Undo:** n/a.

**Model notes:** `OutputKind::Gif(GifSettings { short_side, fps, loop_count, colors, dither, palette_mode })`.
The delay-spreading math is a pure, tested function.

### EX-044 Audio-only export — v1

**What the user sees:** Output type **Audio only** hides the video settings and
shows the audio format controls defined in AU-032: MP3, WAV, AAC (.m4a),
FLAC; bitrate or bit depth; sample rate; channels. Range, naming, progress,
queue and result view work as for video.

**Inputs / controls:** As AU-032. The naming, range and job controls come
from EX-023, EX-035 and EX-038.
- Reference: an article on the reference editor's site shows its export
  dialog letting you save the sound as MP3, WAV, AAC or FLAC.
- Reference (observed): its labels name MP3 or WAV for audio-only files and
  AAC or PCM in its audio settings panel. The likely reading is file formats
  for audio-only output and codecs for the video's audio track; FreeCut
  follows that split, with AU-032's formats here and EX-031's codecs for
  video.

**Behavior / output:** The job runs the AU-032 offline render and encode. No
GPU work happens. Preflight runs the audio-relevant checks only.

**Edge cases:** See AU-032 (silent timeline, solo, WAV over 4 GB). A
video-only timeline gives a "Timeline is silent" warning.

**Undo:** n/a.

**Model notes:** `OutputKind::AudioOnly(AudioExportSettings)`, shared with
EX-031. The behaviour is owned by AU-032. This entry only hosts it in the
dialog and job system.

### EX-045 Captions and subtitle files in export — v1

**What the user sees:** A **Captions** section in the export dialog. For each
caption track you choose one of:
- burn into video;
- embed as a subtitle stream;
- don't include.

An "Also save caption file" option writes SRT, WebVTT, ASS, TXT or LRC next to
the video.

**Inputs / controls:** Defined by TX-033 (per-track mode) and TX-032 (sidecar
formats and options).
- Reference: the reference editor's public pages describe exporting a
  standalone SRT file on desktop and burning captions into the video.
- Reference (observed): its dialog exports a captions file alone or together
  with an audio-only file, with a completion notice.
- "Also save caption file" stays available when the output type is Audio
  only (EX-044), so a soundtrack and its captions come out of one job. The
  sidecar then takes the audio file's stem.

**Behavior / output:**
- Burned captions render through FX-001 like any text.
- Embedded streams are muxed by `freecut-media` using the container rules in
  TX-033.
- **Sidecar names (export dialog):** sidecars written from this section
  take the exported video's file stem, as finally resolved by EX-023 (after
  sanitizing and any Keep-both suffix), not the project name:
  `<video stem>.<track-language-or-name>.<ext>`. For example, a video saved
  as `Trip (2).mp4` gets `Trip (2).en.srt`. The Captions panel's standalone
  **Export subtitles…** has no video file, so it names files after the
  project instead (`<project>.<track-language-or-name>.<ext>`). TX-032 states
  both cases.
- Range exports rebase all times (EX-035).

**Edge cases:**
- The container cannot carry subtitles (GIF, audio-only): Embed is disabled.
- Sidecar name collision: the sidecar follows the video's EX-023 choice. With
  Keep both, the suffix is already in the video stem, so video and sidecar
  stay paired; with Replace, an existing sidecar of that name is replaced
  too.
- Two caption tracks with the same language and name: the second gets a
  ` (2)` after the language part, as in EX-023.
- The completion toast and the result view (EX-039) list the video or audio
  file and every sidecar written.

**Undo:** n/a.

**Model notes:** The behaviour is owned by TX-032/TX-033. EX provides the
dialog section, the job step that writes sidecars after the video finalizes,
and the muxing hook.

### EX-046 Cover image and file metadata — v1

**What the user sees:**
- **Cover:** a cover thumbnail in the export dialog's Extras. Click it to pick
  a frame (a scrubber over the timeline) or an image file.
- **Metadata:** optional fields for title, artist/author, comment, copyright
  and year.
- **Embedding:** the cover is embedded in the file, which players and file
  browsers show as the thumbnail.
- **"Save cover as image"** also writes a JPEG next to the video.
- **"Show cover as first frame"** also puts the cover at the very start of
  the video, for players and sites that take the opening frame as the
  thumbnail.

**Inputs / controls:**
- Cover source: frame at a time, or an image file (PNG/JPEG/WebP). Default:
  the project cover frame pinned with PM-019 when it lies inside the range,
  else the first frame of the range. The export cover is separate from PM-019's
  project-card cover; choosing one here does not change the other. The
  thumbnail is labelled "Default" while no explicit choice was made, so the
  dialog never shows an empty cover.
- **Suggested frames:** the frame picker shows up to 6 suggestions above the
  scrubber (FreeCut default). Candidates are 24 evenly spaced frames of the
  range, scored locally by sharpness (variance of a Laplacian filter on a
  160 px copy) with a penalty for near-black or near-white frames; the 6
  best that are at least 1 s apart are shown. No ML model is involved.
- This feature only picks a frame or an image file. Composing a cover with
  text, stickers and layouts is EX-053 (later), which adds a third source,
  **Designed cover**, and becomes the default source when a design exists.
- Embed cover: default on for MP4, MOV and MKV.
- Save cover as image: default off.
- Show cover as first frame: default off. Length 1 frame (default) up to
  3 s, in frames.
- **Change cover…** on the result view (EX-039) and on finished rows in the
  Jobs panel (EX-038) picks a new cover for a file already written.
- Reference (observed): the reference editor's cover comes from a video
  frame, with suggested frames, or an imported image; a toggle exports the
  cover as the video's first frame or leaves it out; the cover is also
  written as a separate JPG named after the video; and adding a cover after
  an export re-exports to the same path. Alternative seen in the reference
  app: the cover can only be previewed once the export has started; FreeCut
  shows it in the dialog before exporting.
- Metadata fields:
  - title (default: the name field);
  - artist;
  - comment (up to 1 000 characters);
  - copyright;
  - year.

**Behavior / output:**
- The cover is rendered through the snapshot path (EX-014) at output size,
  limited to 1920 px on the long side.
- It is embedded as an attached-picture stream (JPEG), where the container
  supports one, and written as `<name>.jpg` when "Save cover as image" is on.
  `<name>` is the video's final stem after EX-023 (sanitizing and any
  Keep-both suffix), so the image stays paired with its video, as EX-045's
  sidecars do.
- **Show cover as first frame:** the job renders the cover, fitted to the
  output size like an image cover, for the chosen length before frame 0 of
  the range. Audio gets matching leading silence, and captions, chapters
  (EX-049) and cover-related times shift by the same amount. The timeline is
  not edited; to put a cover on the timeline itself, use EX-053's **Insert
  at start of video…**.
- **Change cover…:** for MP4, MOV and MKV files written without "Show cover
  as first frame", FreeCut remuxes the file with stream copy (no
  re-encoding) into a temporary file next to it, replacing only the
  attached picture, then swaps it in atomically. It takes seconds. If the
  file was written with the cover as its first frame, the cover is part of
  the picture, so FreeCut offers **Export again** with the same settings and
  path (Replace) instead. The JPEG sidecar is rewritten too when it exists.
- Metadata goes into the container's standard tags only.

**Edge cases:**
- Cover frame outside the export range: allowed, with a note. The cover is
  just an image.
- Image cover with a different aspect: fitted into the output aspect with
  bars in the canvas color.
- WebM/GIF/audio formats: the cover embed is hidden for WebM and GIF. For
  MP3, FLAC and M4A the cover becomes album art.
- Metadata containing characters a container can't store: transliterated or
  dropped, with a note.
- Cover image file missing at export time: preflight warns and the first
  frame of the range is used instead.
- An image that cannot be used (it fails to decode, is a format FreeCut does
  not read, or is larger than 16 384 px on a side): an error under the
  thumbnail, "This image can't be used as a cover", with **Choose another
  image**. CMYK JPEGs are converted to RGB, not refused.
- Cover frame later falls past the timeline end after edits: reset to the
  default, with a note.
- Show cover as first frame with GIF or audio-only output: hidden; it only
  applies to video.
- Change cover… on a file that was moved, renamed or is now read-only:
  "File no longer exists" or the `no-permission` message (EX-022); the
  original file is never left half-written.
- Change cover… on WebM: not offered, because WebM has no cover stream.

**Undo:** n/a. The cover choice and the metadata fields are export settings,
saved with the project like the rest of `ExportSettings` (EX-021). The
project-card cover is PM-019's undoable `SetCoverFrame`.

**Model notes:** `CoverSource { Frame(Tick), Image(StoredPath) }` is the
`cover` field of `freecut-project::export::ExportSettings` (EX-021); the
image path is stored the portable way PM-100 defines. EX-053 later adds a
`Designed` variant that refers to the project's cover design. Rendering the frame uses
`freecut-render` (EX-014). Image decoding, JPEG encoding and muxing of
attached pictures are in `freecut-media`. The first-frame option is
`cover_lead: Option<FrameCount>` in `ExportSettings`; the time shift it
causes is applied by the pure frame-time and caption-rebase helpers
(EX-022, EX-035), with tests. The remux for **Change cover…** is
`freecut-media::mux::replace_attached_picture(src, image, dst)`. The
suggestion scorer is a pure function over downscaled frames in
`freecut-render`, unit-tested on synthetic images.

### EX-047 Image sequence export — later

**What the user sees:** Output type **Image sequence** writes one image per
frame into a new folder.

**Inputs / controls:**
- Format: PNG 8 or 16-bit (default 8), TIFF 16-bit, or JPEG (quality 1–100,
  default 92).
- Start number: 0 or 1 (default 0).
- Digits: 4–8 (default 6).
- Name pattern: `<name>_%06d.png`.
- Range: as EX-035.
- Comparison: Kdenlive offers many image-sequence formats. OpenShot uses a
  printf-style pattern.

**Behavior / output:**
- Images are rendered in RGB (no Y′CbCr) and tagged sRGB.
- Files go into a subfolder named after the export, which is created if
  missing.
- Encoding runs in parallel worker threads (FreeCut default: CPU count − 1).

**Edge cases:**
- Folder not empty: a prompt to replace or to choose another.
- Very long timelines produce hundreds of thousands of files. A warning
  appears with the count.

**Undo:** n/a.

**Model notes:** `OutputKind::ImageSequence(..)`. Image encoders are in
`freecut-media`.

### EX-048 Export with transparency (alpha) — later

**What the user sees:** A "Transparent background" checkbox for codecs that
can store alpha:
- ProRes 4444 (MOV);
- QuickTime Animation, a lossless run-length codec (MOV), through FFmpeg's
  own encoder;
- HEVC with alpha (MOV), only where the macOS hardware encoder supports it
  (EX-034); hidden elsewhere, because no open software encoder writes it;
- VP9 with alpha (WebM);
- PNG image sequence.

The viewer then shows a checkerboard where nothing covers the canvas, and
the size estimate (EX-037) carries a note that alpha files are much larger
than ordinary video.

**Inputs / controls:** Checkbox. Codec choices are filtered to alpha-capable
codecs.
- Reference (observed): the reference dialog's alpha-capable codecs are
  ProRes 4444, HEVC with alpha and a run-length (RLE) codec, with a warning
  that alpha exports produce much larger files.

**Behavior / output:** This requires FX-001 step 1 to clear to transparent
instead of the canvas background when the option is on (an FX change). The
output stays premultiplied internally and is converted to straight alpha
where the codec expects it.

**Edge cases:**
- Blurred or image canvas backgrounds (TR-012/013) are dropped. Preflight
  warns.
- Effect layers on the lowest track see transparency instead of the
  background.
- Blend modes over transparent areas follow the W3C rules from FX-001.

**Undo:** n/a.

**Model notes:** A flag passed into the FX-001 pipeline. The output
conversion keeps alpha. Overlays (OV) already expect alpha to be preserved.
`VideoCodec` gains `QtAnimation` and `HevcAlpha`; the latter is offered only
when `HwEncoderCaps` (EX-034) reports it.

### EX-049 Chapters from markers — later

**What the user sees:** "Write chapters from markers": named timeline markers
(TL-037) become chapters in MP4, MOV and MKV files. A copyable chapter list
(`00:00 Intro`, `01:23 Part two`…) is offered for video descriptions.

**Inputs / controls:**
- Which markers: all named markers (default), or markers of selected colors.
- "Add a 00:00 chapter if missing": default on.

**Behavior / output:** Chapter times are rebased to the export range and
quantized to milliseconds. Markers outside the range are skipped.

**Edge cases:**
- Two markers at the same time: the first is kept and the rest are listed in
  a note.
- Unnamed markers are skipped, or named "Chapter N" (option).

**Undo:** n/a.

**Model notes:** A pure conversion from `Marker` to `Chapter` in
`freecut-timeline`. The chapters are muxed by `freecut-media`.

### EX-050 Batch and multi-range export — later

**What the user sees:**
- "Export each range marker as a file" creates one job per range marker
  (TL-037), named `{project}-{marker}`.
- "Export with several presets" creates one job per chosen preset, for
  example 16:9 and 9:16 with Fit/Fill.

All jobs go into the queue (EX-038).

**Inputs / controls:** Marker selection by color; preset multi-select; a name
pattern.
- Comparison: Kdenlive's marker multi-export writes one file per marker
  section and names them after the markers.

**Behavior / output:** Each job is independent, with its own snapshot,
preflight and result.

**Edge cases:**
- Duplicate marker names: suffixes are added, as in EX-023.
- Overlapping ranges are allowed.
- Range markers that extend past the timeline end are clipped to it. Markers
  that start after the end are skipped.

**Undo:** n/a.

**Model notes:** A generator over `ExportSettings` in `freecut-app::jobs`.

### EX-051 Draft export (fast test render) — later

**What the user sees:** A "Draft" toggle that exports quickly for review:
- it renders at preview resolution;
- it uses proxies where present;
- it uses the fastest encoder speed;
- it adds `_draft` to the file name.

**Inputs / controls:** Toggle. Its scale follows EX-006's playback
resolution.
- **Review extras** (shown when Draft is on), FreeCut's local stand-in for
  hosted review links:
  - **Burn in timecode:** default on. The timeline time (TL-002 format,
    including drop-frame labels when on) is drawn bottom-center on a dark
    box, at 3 % of the output's short side (adjustable 2–8 %).
  - **Save marker list:** default off. Writes `<stem>.markers.csv` next to
    the file with time, name, color and note for each marker in the range
    (TL-037), rebased like captions (EX-035).
- Comparison: Kdenlive offers proxy and preview-resolution rendering for test
  renders.

**Behavior / output:** Same pipeline, with substituted inputs. The result view
labels the file as a draft. The timecode is drawn in the output stage, after
the canvas, so it never touches the timeline. The reviewer sends notes back
by any means they like; FreeCut hosts nothing.

**Edge cases:**
- Drafts never overwrite a non-draft file of the same base name.
- Clips without proxies render from the originals, scaled down.
- Burned-in timecode on a range export shows timeline time, not file time,
  so notes point at the timeline directly.
- No markers in the range: the marker list is written with its header only,
  and the result view says "0 markers".

**Undo:** n/a.

**Model notes:** A flag in `ExportSettings` that `freecut-app::jobs` reads to
choose the preview scale, proxy media and encoder speed, plus
`ReviewExtras { burn_timecode: Option<TimecodeBurn>, marker_list: bool }`.
The timecode text uses `freecut-timeline`'s formatting helpers (TL-002) and
the UI font, drawn by `freecut-render::output`. The CSV writer is a pure
function over `Marker`s.

### EX-052 Command-line export — later

**What the user sees:** Running `freecut export <project> --preset <name>
[--range in-out] [--out <file>]` exports without opening the editor window.
It prints progress and returns an exit code (0 success, 2 settings error,
3 render or encode error, 4 cancelled).

**Inputs / controls:** CLI flags mirroring ExportSettings, plus `--list-presets`
and `--probe-encoders`.

**Behavior / output:** Uses the same job code. It needs a GPU adapter, and
falls back to a software wgpu adapter where available (slow), with a
warning.

**Edge cases:**
- Project with offline media: fails unless `--allow-offline` is given.
- Missing fonts: warns.
- SIGINT or Ctrl+C: cancels cleanly and removes the `.part` file.

**Undo:** n/a.

**Model notes:** A thin binary entry point in `freecut-app` (or a small
`freecut-cli` binary, if the crate split warrants it) reusing
`jobs::ExportJob`. `anyhow` is used at the edge.

### EX-053 Cover designer — later

**What the user sees:** A small cover tile sits at the start of the main
track, next to its header (TL-004). It shows the current cover, and a pencil
badge appears on hover. Clicking it opens the **Cover designer** window. The
same window opens from **Design…** next to the cover thumbnail in the export
dialog (EX-046) and from **Design cover…** in the player context menu.
- The middle of the window shows the cover at its own shape, with center
  lines and a title-safe margin.
- On the left are tabs: **Base** (what the picture is made from),
  **Layouts**, **Text**, **Stickers** and **Shapes**.
- On the right is an inspector for the selected layer, and a layer list
  that can be reordered.
- In a corner, a small inset shows the cover at about 160 px wide, which is
  roughly how big it appears in a list of videos. This makes it easy to check
  that the title is still readable.
- At the bottom are the **Use for** checkboxes, then **Reset**, **Cancel**
  and **Save**.

The saved cover becomes the export cover (EX-046) and the project-card cover
(PM-019), so users no longer need another tool to make a cover.

**Inputs / controls:**
- **Base:**
  - **Frame** (default): a scrubber over the timeline, with frame steps. It
    starts at PM-019's pinned frame if there is one, else at the playhead.
    Option "Hide timeline text and stickers in the base": default off. When
    on, the base frame is rendered without text, caption and sticker
    tracks, so they don't clash with the cover's own title.
  - **Image**: a PNG, JPEG or WebP file. Its path is stored the portable way
    (PM-100).
  - **Color**: a solid color, or a two-stop linear gradient.
  - Fit: **Fill** (default; crops to cover the shape, and the crop can be
    dragged and zoomed from 100 % to 400 %) or **Fit** (bars use the base
    color).
  - Treatment: dim 0–80 % (default 0), blur 0–100 % of a maximum radius
    equal to 5 % of the short side (default 0), and one optional look from
    CL's filter library with its intensity (CL-004).
  - **Freeze base frame**: turns the frame into a stored still image, so later
    timeline edits no longer change the cover.
- **Shape:**
  - **Match export** (default): uses the aspect of the current export output
    size (EX-024/025/042).
  - 16:9 at 1280×720. Public guidance on video sites gives this as the usual
    thumbnail size.
  - 9:16 at 1080×1920, 1:1 at 1080×1080, 4:5 at 1080×1350.
  - Custom: 16–1920 px on each side.
  - The long side is never larger than 1920 px, the limit EX-046 embeds
    (FreeCut defaults).
- **Layouts:** about 12 FreeCut-made layout presets, each with a variant per
  shape. Examples:
  - a title band across the bottom;
  - a large centered title over a dimmed frame;
  - a split with a text panel on one side;
  - a title with a corner badge (for example an episode number);
  - bars at the top and bottom;
  - a frame drawn around the picture.

  Each layout has a palette control with three color slots: main, accent and
  text. The choices are 6 FreeCut palettes, or **From frame**, which uses
  the base's two most common colors plus a contrasting text color.
- **Layers** (at most 64; FreeCut default):
  - Text uses TX's controls: content, font (TX-003, TX-018), size, fill,
    outline, shadow, glow, background box, gradient, spacing and curved text
    (TX-004 to TX-016). TX-019 style presets apply too.
  - Stickers come from OV's library: bundled packs (OV-019), the user's own
    stickers (OV-020) and emoji (OV-021).
  - Shapes, lines and decorative frames come from OV-024, OV-025, OV-027 and
    OV-029.
  - Images are the user's own files (as OV-011).
  - Every layer has position, scale, rotation and opacity. Text size is
    relative to the cover's short side.
  - Dragging snaps to center lines, the title-safe margin (EX-011's
    percentage) and the edges of other layers (as TR-008).
- **Use for:** "Export cover" (default on) and "Project card" (default on).
- **Actions:**
  - **Save as image…**: PNG or JPEG. An optional "Keep under … KB" field
    lowers the JPEG quality step by step, down to 50. If the file is still
    too big, a note says so.
  - **Insert at start of video…**: places the rendered cover on the main
    track at time 0 as a still clip (default 1 s). This is for sites that
    pick the opening frame as the thumbnail.
  - **Reset**: removes the design after a confirmation. EX-046 and PM-019
    then go back to their own defaults.
  - **Start over** keeps the base and removes every layer, as one step on
    the designer's own undo history (nothing is committed until Save).
- Opening the designer: no default key (see UI-031).
- Reference: a guide on the reference editor's site describes a Cover
  button, marked with a pencil, by the timeline. It opens a cover editor
  where you either pick a video frame or replace it with an image from your
  computer, then add filters, stickers and text, including ready-made text
  and cover designs, before saving. A second guide on that site recommends
  1280×720 at 16:9 for video-site thumbnails.
- Reference (observed): the reference cover editor adds text, text effects
  and layout templates; it can start a new cover or edit the current one,
  and replace, delete or confirm it; it shows a "generating cover" state
  while rendering; it asks before deleting a cover with unsaved edits; and
  it says so when a change of the project's aspect ratio also changes the
  cover's. Its export settings can put the cover in the file as the first
  frame and save it as a separate JPG (FreeCut: EX-046).

**Behavior / output:**
- The design is a still composition stored in the project. It is not a
  clip. It has no time, never appears in the video's frames (unless you use
  **Insert at start of video…**) and does not change the timeline duration.
- Rendering:
  - The base frame is rendered by the export path at that time (EX-014's
    still render, with full quality and no proxies). It shows the timeline's
    current content, so later edits at that time show up in the cover until
    the base is frozen.
  - The base treatment is applied next.
  - Then the layers are drawn, in list order, by the same text, sticker and
    shape renderers the timeline uses. The cover therefore matches what those
    items look like in a video.
  - Text animations, keyframes and sticker animation are ignored. Animated
    stickers show their first frame. A different frame can be picked in the
    inspector.
- Outputs:
  - EX-046 embeds the cover as a JPEG at the cover size when "Export cover"
    is on. In that case the export dialog's cover source becomes **Designed
    cover**. It can still be switched back to a frame or an image for one
    export.
  - When "Project card" is on, PM-019 writes the design at 320 px on its
    long side, at save time, in place of the pinned frame.
  - Save as image writes the full-size render.
- Applying a layout:
  - The base is kept. The layout's layers are added, its Title is filled
    from the project name, and its other text slots get placeholder text.
  - If the design already has layers, a prompt offers **Replace my layers**
    or **Add on top**.
  - The design stores the resolved layers. Like TX-019 presets, it never
    depends on the preset library afterwards. The layout ID is kept only so
    the matching variant can be re-applied after a shape change.
- From frame:
  - The palette comes from the base, scaled down to 64×64. The two most
    common colors are found with a small k-means (k = 4, seeded the same way
    every time).
  - The text color is black or white, whichever has the higher WCAG
    contrast against the main color.
- The designer's preview renders on a worker thread at the viewer size,
  debounced by 50 ms. The UI thread only shows finished textures.
- **Save** renders the cover at full size first. Meanwhile the button reads
  "Rendering cover…" and is disabled; the window closes when the render is
  stored. Closing the window with unsaved changes asks **Discard changes?**
  (Discard / Keep editing).

**Edge cases:**
- **Empty timeline:** Frame is disabled, and Color becomes the default base.
  A text-only cover is still valid and shows on the project card.
- **Base frame past the timeline end after edits:** the base falls back to
  the first frame. The designer shows a note, and preflight (EX-036) shows
  one too. The layers are not changed.
- **Base frame on offline media:** the slate (EX-005) is shown in the base,
  and preflight warns, as for any offline media.
- **Base image or a layer's image file missing:** the designer shows a
  hatched placeholder. Export warns at preflight. The missing base is drawn
  with the base color, and a missing layer is skipped.
- **Missing font in a text layer:** TX-007 substitution. It is listed under
  preflight's "fonts substituted" warning.
- **A layer's bounds leave the title-safe area:** a dashed outline marks
  it. This is allowed; anything outside the cover is clipped.
- **Shape changed after designing:** positions are fractions of the cover,
  and text scales with the short side, so the layout moves in proportion.
  If the layout has a variant for the new shape, **Re-apply layout** is
  offered.
- **Design shape differs from the export output aspect:** EX-046's rule for
  image covers applies (fitted, with bars in the canvas color). The designer
  says so; Match export avoids it.
- **Canvas ratio changed (PM-003) while the shape is Match export:** the
  cover's shape follows. A one-time note appears on the cover tile and in
  the export dialog's cover thumbnail ("Cover shape now 9:16 to match the
  video"), and **Re-apply layout** is offered as above.
- **Reset with unsaved designer changes:** the confirmation also mentions
  that the unsaved changes are discarded.
- **Cover frame outside the export range:** allowed. The cover is just an
  image (as in EX-046).
- **Background export running or queued:** each job uses the design in its
  snapshot (EX-038). Later changes affect only later exports.
- **Playback running when the designer opens:** playback pauses.
- **Open-licensed stickers that need attribution (OV-040):** they are listed
  in the credits sheet (OV-041) when they are used in the cover.
- **A project file from an older FreeCut:** it has no design, so EX-046 and
  PM-019 behave as before. The field is added through the PM-021 migration
  chain.
- **Insert at start of video with the main track locked (TL-009):** refused,
  with the same toast as other main-track edits.

**Undo:**
- Inside the designer, each change is a step on the designer's own undo
  history. Undo and Redo (keys per UI-031) work while it has focus.
- **Save** commits one project edit, `SetCoverDesign { before:
  Option<CoverDesign>, after: Option<CoverDesign> }`. Undo restores the
  previous design, or no design. **Cancel** commits nothing.
- **Reset** is the same op with `after: None`.
- **Freeze base frame** is one transaction. It adds a generated still to
  PM-041's store and switches the base to that image. Undo removes the item
  and keeps the file, as PM-041 does.
- **Insert at start of video…** is one transaction: the PM-041 import plus
  TL's insert (TL-012) on the main track at 0, which ripples the main track.
- All four have tests: apply the op, then undo, and the document is equal to
  before.

**Model notes:**
- `CoverDesign` sits next to PM-019's `cover` in the project document. It is
  made of:
  - `shape: CoverShape`;
  - `base: CoverBase`;
  - `fit: BaseFit`;
  - `crop: BaseCrop`;
  - `look: Option<LookRef>`;
  - `dim_pct: u8`;
  - `blur_pct: u8`;
  - `layers: Vec<CoverLayer>`;
  - `layout: Option<LayoutId>`;
  - `use_for_export: bool`;
  - `use_for_card: bool`.
- `CoverBase` is `Frame { at: Tick, hide_text_and_stickers: bool }`,
  `Image(StoredPath)` or `Color(ColorFill)`.
- `CoverLayer` holds an id, a kind and a static transform (TR's transform
  without keyframes). The kind is `Text(TextContent, TextStyle)`,
  `Sticker(AssetRef)`, `Shape(ShapeParams)` or `Image(StoredPath)`.
  Positions are fractions of the cover, as in the TR conventions. They are
  not time, so floats are fine here.
- The types and `SetCoverDesign` live in `freecut-timeline` (pure, no I/O).
  Serialization and the migration live in `freecut-project`.
- These pure helpers are unit-tested:
  - `apply_layout(&CoverDesign, &CoverLayout, project_name) -> CoverDesign`,
    in `freecut-timeline`;
  - `cover_pixel_size(CoverShape, OutputSize, canvas) -> (u32, u32)`, in
    `freecut-project::export` next to `output_dimensions` (EX-024), because
    `OutputSize` is an export type and `freecut-timeline` depends on no
    other FreeCut crate;
  - `palette_from_image(&RgbaImage) -> Palette`, in `freecut-render` next to
    the cover render. The model crate never handles pixels.
- Layout presets are JSON in `resources/cover-layouts/`. Each has a schema
  version and a manifest that states its CC0-1.0 license. `freecut-app`
  loads them on a worker thread.
- `freecut-render::offscreen::render_cover(&CoverDesign, seq_snapshot,
  size) -> RgbaImage` reuses `render_still` (EX-014) for the base and the
  TX and OV renderers for the layers.
- JPEG, WebP and PNG encoding and the attached-picture muxing (EX-046) are
  in `freecut-media`.
- The designer window is `freecut-app::cover_designer`. It reuses the TX
  and OV inspector widgets.

### EX-054 Clip start/end jumps and locate in timeline — v1

**What the user sees:** Two transport commands, **Go to clip start** and
**Go to clip end**, in the player's overflow menu and the command palette
(UI-034), and **Locate in timeline** in the player's context menu. Locate
scrolls the timeline so that the playhead, and the clip being shown, come
into view, and briefly highlights that clip.

**Inputs / controls:**
- Go to clip start, Go to clip end, Locate in timeline: no default keys (see
  Conventions).
- Reference (observed): see EX-003. The reference player has jumps to the
  current clip's first and last frame, and a locate action for the clip
  being previewed or selected.

**Behavior / output:**
- **Current clip:** the selected clip when exactly one clip is selected;
  otherwise the topmost visible clip under the playhead in layer order
  (TL-004); otherwise none.
- Go to clip start moves the playhead to the clip's first frame. Go to clip
  end moves it to the clip's last frame (`end − 1 frame`), so the viewer shows
  that clip's final picture rather than the next clip's first.
- Both pause playback first, as frame stepping does (EX-003).
- Locate in timeline: if the playhead is outside the visible timeline area,
  the view scrolls to center it (TL-041's scroll; the zoom is unchanged). If
  there is a current clip, the view also scrolls vertically to its track,
  and the clip's outline flashes twice (about 0.6 s). Selection and playhead
  do not change.
- In media preview mode (EX-015): clip start and end jump to the marked In
  and Out, or to the media's first and last frame. Locate runs PM-055's
  **Find on timeline** for the previewed item, which selects the clips that
  use it, as PM-055 defines.

**Edge cases:**
- No current clip (a gap, or an empty timeline): the commands are disabled,
  with the reason in the tooltip.
- Several clips selected: the selection is ignored, and the clip under the
  playhead is used.
- One-frame clip: start and end are the same frame.
- Playhead already on the target frame: nothing happens.
- Inside an open compound clip (TL-040): nested time and nested clips are
  used.
- Clip on a collapsed track (TL-010): Locate does not expand it; it scrolls
  to the track and flashes the collapsed row.
- Locate in media mode for an item that is not used on the timeline (no
  PM-055 usage badge): disabled, with "Not used on the timeline" in the
  tooltip.

**Undo:** n/a (view state).

**Model notes:** Pure `current_clip(seq, playhead, &Selection) ->
Option<ClipId>` and `clip_frame_bounds(seq, ClipId) -> (Tick, Tick)` in
`freecut-timeline`, tested with stacked tracks and one-frame clips. The
scroll and flash are `freecut-app` timeline-view code. TL-013's cut-point
jumps stay TL's; these commands belong to EX's transport family.

### EX-055 Player rulers and guide lines — v1

**What the user sees:** Optional rulers along the top and left edges of the
viewer, in canvas pixels or percent, with 0 at the canvas's top-left corner.
Dragging out of a ruler creates a guide line: horizontal from the top ruler,
vertical from the left one. Guides are thin colored lines across the canvas
that help line up text and images. Like EX-011's overlays, they never
appear in exports or snapshots.

**Inputs / controls:**
- New entries in the Guides submenu (EX-011): **Show rulers** (default off),
  **Show guides** (default on), **Lock guides** (default off), **Add
  horizontal guide** and **Add vertical guide** (both placed at the canvas
  center), **Clear all guides**, and **Units:** Pixels (default) or
  Percent.
- On a guide: drag to move it. Double-click, or **Edit position…** in its
  context menu, opens a field in the current unit: pixels from 0 to the
  canvas width or height (whole numbers), or percent 0–100 (two decimals).
  The context menu also has **Delete**, **Lock guides** and **Hide guides**.
- Dragging a guide back onto its ruler, or off the canvas edge, deletes it.
- At most 50 guides per project (FreeCut default).
- Color: EX-011's guide color.
- Commands Show rulers, Show guides and Lock guides: no default keys (see
  Conventions).
- Reference (observed): the reference player has rulers; horizontal and
  vertical guides whose position can be edited in percent or pixels; delete
  from a guide's context menu and clear all; hide and show all; lock and
  unlock; shortcuts for guides and rulers; a maximum guide count (the value
  is not shown); and a toast when a locked guide is dragged.

**Behavior / output:**
- Positions are stored as fractions of the canvas width or height, so guides
  keep their relative place when the canvas size changes. Pixel readouts are
  `round(fraction × size)`.
- While being dragged, a guide snaps to the canvas center lines and edges,
  EX-011's visible safe areas and other guides (6 screen px, as TR-008). A
  tooltip shows its position in the current unit.
- Visible guides are snap targets when dragging clips on the canvas (TR-008).
- Ruler labels adapt to the viewer zoom (EX-009), and a small marker on each
  ruler follows the pointer. The rulers take 18 px of the viewer on two
  sides.
- Saved per project in the UI section. Like other player state, they never
  mark the project modified on their own (Conventions).

**Edge cases:**
- Adding a 51st guide: refused, with the toast "Guide limit reached (50)".
- Dragging a guide while guides are locked: nothing moves; a toast, "Guides
  are locked", with **Unlock**, at most once per drag. Locked guides can
  still be hidden or cleared.
- **Clear all guides:** removes them at once and shows a warning toast with
  **Restore** (UI-040's lifetime), since guides are not on the undo stack.
- Canvas size or ratio change (PM-003, PM-004): the fractions are kept, so a
  guide at 540 px on a 1080 px-wide canvas sits at 960 px after the width
  becomes 1920 px.
- A typed position outside the canvas: clamped to the edge.
- Two guides at the same position: allowed.
- Viewer narrower or shorter than 200 px: the rulers hide; guides stay.
- Fullscreen (EX-010): rulers never show; guides follow "Show guides in
  fullscreen".
- Media preview mode (EX-015): rulers and guides hide, because they belong
  to the project canvas.
- Viewer zoomed above Fit (EX-009): guides extend across the pasteboard, so
  off-canvas items can be aligned too.

**Undo:** n/a (view state). Clear all guides has the toast's Restore
instead.

**Model notes:** `UserGuides { show_rulers: bool, show: bool, locked: bool,
units: GuideUnits, lines: Vec<GuideLine> }`, with `GuideLine { axis: Axis,
frac: f64 }`, in `freecut-project::ui::PlayerUiState` next to `GuideSet`
(EX-011). Floats are fine: these are canvas fractions, not time.
Pixel↔fraction conversion, clamping, hit-testing and the limit are pure
helpers in `freecut-app::player::guides`, unit-tested. Drawing uses the egui
painter, as EX-011 does.

### EX-056 Playback priority: smooth or sharp — v1

**What the user sees:** A choice under Preview resolution in the player
menu: **During playback: Keep it smooth** (default) or **Keep it sharp**.
With Smooth, a heavy section plays at a lower resolution for a while
instead of stuttering, and a small badge in the viewer corner shows the step
in use (for example "½"). With Sharp, the picture keeps its resolution and
frames are dropped instead.

**Inputs / controls:**
- Playback priority: Smooth (default; FreeCut default) or Sharp. It is an
  app preference (Preferences ▸ Performance, UI-018), mirrored in the player
  menu.
- Reference (observed): the reference editor offers a preference between
  favouring smooth playback, which may lower the displayed resolution, and
  favouring full display resolution, which may stutter. It also shows an HD
  badge, and a separate quality-priority mode whose scope (preview, export
  or both) is unclear. FreeCut's choice affects only the preview; export
  quality is set by EX-027/EX-028 and export speed by EX-034.

**Behavior / output:**
- It acts only when Preview resolution is **Auto** (EX-006). A fixed Full,
  ½ or ¼ is always kept exactly, in both modes.
- **Smooth:** when EX-017's counter shows fewer than 90 % of due frames for
  2 s, the render size steps down to the next smaller of the ½ and ¼ canvas
  sizes (never below ¼). After 10 s at 99 % or more, with render time under
  half the frame budget, it steps back up one level. At most one step
  happens per 2 s, so it does not oscillate.
- Each playback starts at the level the previous playback in this session
  ended on, so a known heavy section does not stutter at its start. A seek
  of more than 10 s, or any edit, resets to Auto's size.
- **Sharp:** Auto never steps down; EX-002's drop policy applies.
- Pausing always renders the full-quality still (EX-006's paused refresh),
  in both modes.
- Never affects export, snapshots or covers.

**Edge cases:**
- Shuttle at 2× and faster: frames skipped by design (EX-017) do not count
  as drops, so they do not cause a step.
- Already at ¼ and still dropping: no further step; EX-017's hint may
  suggest proxies or pre-render.
- Pre-rendered sections (EX-016) exist only at another scale: the cache key
  includes the scale, so a step may switch between cached and live frames;
  both look the same apart from sharpness.
- Fullscreen and media preview mode: the same rule, applied to their own
  render targets.

**Undo:** n/a (app preference).

**Model notes:** `PlaybackPriority { Smooth, Sharp }` in the app
preferences. The stepping is a pure state machine,
`AdaptiveScale::update(&PlaybackStats, now) -> PreviewScale`, in
`freecut-app::player`, tested with synthetic stats and a fake clock. It
sets the `PreviewScale` of EX-004's frame requests.

### EX-057 Several outputs from one export — v1

**What the user sees:** When the output type is Video, an **Also export**
row under it has two checkboxes: **Audio file** and **GIF**. Each checked
companion shows a short summary ("MP3 · 320 kbps", "GIF · 480p · 15 fps")
and an **Options…** button with its full settings (AU-032 for audio, EX-043
for GIF). One Export writes every file under the same name stem, for example
`Trip.mp4`, `Trip.mp3` and `Trip.gif`. Caption files stay in EX-045's
section for every output type.

**Inputs / controls:**
- Also export ▸ Audio file: default off. Settings and defaults as AU-032.
- Also export ▸ GIF: default off. Settings and defaults as EX-043.
- Offered only when the main output type is Video.
- Reference (observed): the reference dialog has independent checkboxes for
  video, audio only, a captions file and GIF, and produces several of them
  in one run.

**Behavior / output:**
- One job, one snapshot, one range and one preflight. The companions are
  extra steps of the same job, in this order: video, audio file, GIF, then
  sidecars (EX-045, EX-046).
- The master mix is rendered once. The same samples feed the video's audio
  encoder and the audio file's encoder.
- The GIF is a second, cheaper render pass at its own size and frame rate.
- Progress is weighted by estimated work (frames × pixels for picture
  passes, samples for audio) and names the current step ("2 of 3: Audio
  file").
- EX-023's naming and conflict rules apply to the whole set. If any target
  exists, one conflict prompt lists them all, and Keep both picks a single
  suffix that is free for every file, so the names stay paired.
- EX-036 checks every output. A problem that concerns only a companion (for
  example a GIF over 60 s) unticks that companion with the reason instead of
  blocking the video.
- The result view (EX-039) lists each file with its own Show in folder. In
  the queue (EX-038) the set is one row, with its steps in the tooltip.

**Edge cases:**
- A companion step fails after the video has finished: the video is kept.
  The result view shows the companion as failed, with its code (EX-022) and
  **Retry** for that file only.
- Cancel during a companion step: the unfinished file is deleted. Files that
  were already finished are kept and listed, because each file is written
  atomically (EX-022).
- Silent timeline: Audio file unticks itself with "Timeline is silent"
  (EX-044).
- A preset (EX-041) saved with companions restores them; one whose audio
  encoder is missing on this machine disables only that companion, with the
  reason.

**Undo:** n/a.

**Model notes:** `CompanionOutput { AudioFile(AudioExportSettings),
Gif(GifSettings) }` in `ExportSettings.companions` (EX-021). The job runs a
`Vec<OutputStep>` built by a pure planner, `plan_outputs(&ExportSettings) ->
Vec<OutputStep>`, tested for order and naming. Feeding one mix to two
encoders is done in `freecut-media`; mixing stays in `freecut-audio`.

### EX-058 Export clips as separate files — later

**What the user sees:** With one or more clips selected on the timeline, or
items selected in the media panel, the context menu offers **Export as
separate files…**. A compact dialog shows how many files will be written
("4 clips"), a base name, the folder, a preset (EX-041) or "Same as last
export", and **Include clip edits**. When the run ends, a summary ("4 files
exported") offers **Open folder**.

**Inputs / controls:**
- Base name: default the project name. Files are named `<base>_<n>`, with `n`
  zero-padded to the number of digits of the count, at least two (`_01`,
  `_02`…).
- Order: timeline clips by start time, then by layer order from the top
  (TL-004); media items in panel order.
- Include clip edits: default on (timeline clips only).
- Folder, preset, **Export** and **Add to queue** (EX-038).
- Reference (observed): the reference editor exports clips selected in the
  media panel or on the timeline as separate files, named with a user-set
  title plus a sequence number. It has an "include all edits" switch (on:
  effects, filters and the like are baked in; off: only position, size and
  speed changes are kept), shows the number of clips, and offers to open the
  folder afterwards.

**Behavior / output:**
- **Timeline clip, edits on:** the clip is rendered alone over its own span
  `[start, end)`, on the canvas background, with everything that belongs to
  it: transform, crop, masks, keying, color, filters, effects, animations and
  speed (FX-001). Other tracks are left out. Its audio is its own clip chain
  (volume, fades, effects), through the master bus as if it were alone.
  Transitions touching the clip are left out, so the clip shows in full.
- **Timeline clip, edits off:** only trim, speed (reverse included) and the
  static transform and crop are kept (TR). Effects, filters, color, masks,
  keying, animations and keyframes are dropped; a keyframed transform uses
  its value at the clip's first frame.
- **Media panel item:** exported whole, or over its marked preview range
  (PM-060), at its own display size and frame rate, with no edits. The
  preset supplies only codec, quality and audio settings. Include clip edits
  is disabled for these.
- **Audio-only clips and items:** written as audio files with the preset's
  audio settings, or AU-032's defaults.
- **Text, sticker, shape and effect clips** have no media of their own. They
  are rendered only with edits on, over the canvas background; with edits
  off they are skipped and listed.
- **Compound clips** (TL-040) are exported with their nested content.
  Alternative seen in the reference app: a compound clip cannot be exported
  on its own.
- Every file is its own job in the queue (EX-038). All of them share one
  snapshot, taken when the batch starts.

**Edge cases:**
- More than 100 files: a confirmation states the count first.
- Disabled clips (TL-036): skipped and listed. Clips on hidden tracks:
  exported, because they were selected explicitly, with an info note.
- Offline media: preflight runs once for the batch and lists the affected
  clips; accepted clips render the slate (EX-005).
- Name collisions: EX-023's prompt appears once for the batch, as Replace
  all or Keep both for all.
- One file fails: the others continue. The summary lists the failed ones
  with their codes (EX-022) and offers **Retry failed**.
- Two selected clips with the same media and range: two files.
- A clip shorter than one output frame: rounded up to one frame (EX-035).

**Undo:** n/a (file output; the timeline is not changed).

**Model notes:** A pure `isolate_clip(seq, ClipId, keep: KeepEdits { All,
BasicOnly }) -> Sequence` in `freecut-timeline` builds the one-clip sequence
each job renders. It is tested for both modes, including keyframed
transforms and compound clips. The batch generator lives in
`freecut-app::jobs`, next to EX-050's. Naming reuses EX-041's pure
name-pattern expansion.

### EX-059 Your own logo on exports — later

**What the user sees:** In the export dialog's Extras, **Add my logo** (off
by default). When it is on, the user picks an image of their own and a
corner, and a small preview of the first output frame shows the logo in
place. The logo then appears on every frame of the video or GIF. FreeCut
never adds a logo by itself (EX-040).

**Inputs / controls:**
- Image: PNG, WebP, SVG or JPEG; transparency is kept. The path is stored
  the portable way (PM-100).
- Corner: top-left, top-right, bottom-left or bottom-right (default
  bottom-right).
- Size: the logo's longer side as 5–40 % of the output's short side
  (default 12 %).
- Margin: 0–20 % of the short side (default 3 %).
- Opacity: 0–100 % (default 80 %).
- "Also on the cover": default off (EX-046).
- Saved with the export settings and in user presets (EX-041), so a preset
  can carry a personal logo.
- Reference (observed): the reference dialog places its own brand
  watermark on free-tier exports, off or in one of four corners. FreeCut
  keeps only the placement idea, for the user's own image.

**Behavior / output:**
- Drawn in the output stage (`freecut-render::output`), after EX-042's
  reframe and before the color conversion (EX-032), in output pixels. It
  therefore sits in the same corner for every output shape and never changes
  the timeline.
- The player does not show it, because it is not part of the timeline; the
  dialog's preview does. A timed or animated logo is an ordinary image
  overlay on the timeline instead (OV-011).
- SVG logos are rasterized at the final pixel size, so they stay sharp.

**Edge cases:**
- Logo file missing at export: a preflight problem (EX-036) with **Remove
  logo** and **Choose file…**, because the user expects the logo to be
  there.
- Very small outputs (for example a 240p GIF): the logo is at least 16 px on
  its longer side.
- Alpha export (EX-048): the logo keeps its own transparency.
- Draft exports (EX-051): the logo is drawn too, below the burned-in
  timecode.
- A logo image larger than 8192 px on a side: scaled down when loaded.

**Undo:** n/a (export setting).

**Model notes:** `LogoOverlay { image: StoredPath, corner: Corner, size_pct:
u8, margin_pct: u8, opacity_pct: u8, on_cover: bool }`, as
`Option<LogoOverlay>` in `VideoSettings`. The placement is a pure function,
`logo_rect(output_size, &LogoOverlay, logo_aspect) -> Rect`, tested for each
corner and the limits. Raster decoding is in `freecut-media`; SVG goes
through the same rasterizer OV uses for vector stickers.

### EX-060 Motion blur for all moving clips — later

**What the user sees:** In the export dialog's Extras, **Add motion blur to
moving clips…**. It opens a small confirmation with FX-022's shutter angle
and quality, and the number of clips it will change. Confirming turns on
FX-022's motion blur for every clip that moves and has no motion-blur
setting of its own. The preview then shows the blur as well, because it is a
clip setting.

**Inputs / controls:**
- Shutter angle 0–360° (default 180°) and quality 4, 8 or 16 samples
  (default 8), as in FX-022.
- The same command, and its reverse, **Remove motion blur from all clips**,
  are in the command palette (UI-034), with no default keys.
- Reference (observed): the reference dialog has an optional motion blur
  applied at export. Alternative seen in the reference app: it is an
  export-only switch. In FreeCut it is a clip look, so preview and export
  stay the same (one pipeline, FX-001).

**Behavior / output:**
- "Moves" means movement FreeCut creates within the clip: keyframed
  position, scale or rotation (TR), or an entrance, exit or continuous
  animation (FX). Motion recorded in the footage is FX-028's and is not
  touched.
- Clips that already have their own FX-022 setting, on or off, are left
  alone.
- Applied as one transaction. The export dialog stays open, and the size
  estimate and preflight re-run (EX-021).

**Edge cases:**
- No moving clips: the button is disabled, with "No clips move" in its
  tooltip.
- Clips on locked tracks (TL-009): skipped; the confirmation says how many.
- Clips inside compound clips: only top-level clips change; the
  confirmation mentions nested ones it did not touch.
- A background export already running: allowed; the job keeps its snapshot
  (EX-038).

**Undo:** One transaction of FX-022's `SetClipMotionBlur { old, new }` ops,
named "Add motion blur to N clips". Undo restores every clip. Tested: apply,
then undo, and the document equals the one before.

**Model notes:** A pure `clips_with_movement(seq) -> Vec<ClipId>` in
`freecut-timeline` (keyframes and animation slots within each clip's
interval), unit-tested. The op and the accumulation are FX-022's; EX only
hosts the command.

## Interactions with other areas

- **Timeline & core editing ([timeline-editing.md](timeline-editing.md),
  TL):**
  - TL owns time (TL-001), the playhead and keyboard navigation (TL-013),
    hover skim (TL-014), markers (TL-037), in/out range marks (TL-038) and
    compound clips (TL-040).
  - TL-008 (mvp) owns the hide and mute track flags: two persisted, undoable
    flags set with `SetTrackFlags`. Preview (EX-002) and export (EX-022)
    apply them, and preflight reports them (EX-036). Track lock (TL-009, v1)
    does not affect playback or export. It only blocks edits, such as
    EX-053's insert at the start of the video.
  - EX-053's cover tile sits at the start of the main track, and its "insert
    at start" action uses TL's insert op (TL-012).
  - EX-054's clip jumps and Locate use TL's layer order (TL-004), timeline
    scrolling (TL-041) and collapsed tracks (TL-010). TL-013's cut-point
    jumps remain TL's.
  - EX-058 isolates one clip per file with a pure TL helper, skips disabled
    clips (TL-036) and exports compound clips (TL-040) with their content.
  - EX-051's marker list and EX-049's chapters read TL-037 markers.
  - EX owns transport (play/pause, J/K/L, loop) and frame delivery for scrub
    and skim. It turns TL-038 marks into loop and export ranges, and runs
    compound pre-render through the render cache (EX-016).
  - EX needs the document to be cheaply snapshot-able (`Arc`-shared), so that
    jobs can run in the background (EX-038).
- **Transform, canvas, masks, keyframes & speed
  ([clip-transform-and-speed.md](clip-transform-and-speed.md), TR):**
  - The player hosts TR's on-canvas handles (TR-007), snapping (TR-008) and
    matte view (TR-024).
  - EX's safe areas, guides and user guide lines (EX-011, EX-055) act as
    snap targets.
  - EX-058's "edits off" mode keeps only TR's trim, speed and static
    transform and crop; EX-060 counts TR keyframed movement as "moving".
  - Export waits for stabilization and optical-flow analysis, and decodes
    reversed media without relying on preview caches.
  - Canvas background fill (TR-011…014) defines what Fit bars contain
    (EX-042).
- **Effects, transitions & animations
  ([effects-transitions-animations.md](effects-transitions-animations.md),
  FX):**
  - FX-001 is the single compositing pipeline. EX only chooses the size,
    applies the output-stage reframe (EX-042), draws output-stage additions
    (EX-051's burned-in timecode, EX-059's logo) and converts color (EX-032).
  - Alpha export (EX-048) needs an FX-001 flag to clear to transparent.
  - Motion blur is FX's clip look (FX-022, FX-028). EX-060 only hosts a
    command that applies FX-022 to every moving clip; there is no
    export-only blur.
- **Color & filters ([color-and-filters.md](color-and-filters.md), CL):**
  - CL-002 defines decode color handling and the BT.709 output tags that
    EX-032 writes.
  - SDR Rec.709 is the single working and output space. HDR output (formerly
    EX-033) is out of scope, to be revisited only together with an HDR
    working space in CL.
  - Preview-only CL switches are surfaced in EX-006.
  - Missing LUTs feed EX-036.
  - The cover designer's base can take one look from CL's filter library
    (CL-004).
- **Audio ([audio.md](audio.md), AU):**
  - AU supplies the playback clock (AU-010), scrubbing and shuttle audio
    (AU-016), offline mixing for export, the audio-only export options
    (AU-032; hosted by EX-044, by EX-057's audio companion and by EX-058 for
    audio clips), stem export (AU-033) and the solo-during-export rule
    (AU-012).
  - EX owns the dialog, jobs and transport that drive them.
- **Text & captions ([text-and-captions.md](text-and-captions.md), TX):**
  - EX hosts the Captions export section (TX-033) and sidecar subtitle files
    (TX-032), and repeats the font-substitution warning (TX-007).
  - Sidecar naming: files written from the export dialog use the video file
    stem (EX-045). The Captions panel's own export uses the project name.
    TX-032 states both cases.
  - EX provides title-safe overlays and the playback clock used by
    tap-to-sync (TX-030).
  - The cover designer's text layers (EX-053) use TX's text model, fonts and
    style presets (TX-003 to TX-019) without animation.
- **Overlays & stickers ([overlays-and-stickers.md](overlays-and-stickers.md),
  OV):**
  - The export dialog offers "save credits next to the video" (OV-041).
    That credits sheet is also FreeCut's local answer to the reference
    app's online commercial-use checks (see Out of scope).
  - Exports must match preview compositing exactly. Alpha export keeps
    overlay transparency.
  - EX-059's export logo is an output-stage addition, not an overlay clip; a
    timed or animated logo is an OV-011 image overlay. EX-059 rasterizes SVG
    logos with OV's vector-sticker rasterizer.
  - The cover designer (EX-053) takes stickers, emoji, shapes, lines and
    frames from OV's library (OV-018 to OV-029) and draws them with OV's
    renderers. Stickers used only on the cover still appear in the credits
    sheet (OV-040, OV-041).
- **Smart / AI tools ([ai-tools.md](ai-tools.md), AI):**
  - Export and snapshots run or wait for missing analyses and always use
    cached results (AI-001), so output matches preview.
  - Preview resolution may use half-resolution mattes.
  - Subject-aware reframing for other aspect ratios lives in AI; EX-042
    offers only Fit and Fill.
- **Projects & media ([project-and-media.md](project-and-media.md), PM):**
  - PM owns canvas ratio and size (PM-003, PM-004; EX-013 hosts the Ratio
    drop-down and EX-024 defaults to the canvas size), the project frame rate
    (PM-005) and sample rate (PM-007).
  - PM also owns media status and offline detection (PM-090; EX-005 draws the
    slate and sets the export rules), relinking (PM-091), proxies and the
    proxy toggle (PM-080, PM-081; used by EX-006 and EX-051), the library
    click that opens media preview mode and the stored preview ranges
    (PM-060; EX-015 owns the player side), the media cache that holds
    pre-rendered sections (PM-074; EX-016), the generated-media store
    (PM-041; EX-014 "add to library", EX-053 frozen bases), the project-card
    cover (PM-019, separate from EX-046's export cover; EX-053's design can
    feed both), and the project-file migration chain (PM-021, which EX-021
    export settings and EX-053's cover design join).
  - PM-073's single background-task list shows export rows (EX-038) and
    pre-render rows (EX-016); it is FreeCut's local equivalent of the
    reference app's server task manager. PM-113's disk thresholds pause
    pre-render, and PM-114's volume classification feeds EX-036's
    online-only placeholder warning. EX-054's Locate uses PM-055's Find on
    timeline in media preview mode. The start screen (PM-001) hosts EX-038's
    export strip.
- **Workspace & shortcuts ([workspace-and-shortcuts.md](workspace-and-shortcuts.md),
  UI):**
  - UI-031 is the canonical default keymap. This spec writes "see UI-031"
    instead of proposing keys. That includes Play/pause, shuttle, Play range,
    Toggle loop playback, Go to timecode, Full-screen preview, Export, Export
    current frame (EX-014) and Preview mute (EX-019), which UI-031 lists
    either bound or unbound by default. Its collision test covers them.
  - Commands added here that UI-031 does not list yet have no default key
    until UI-031 lists them: Go to clip start / end and Locate in timeline
    (EX-054), Show rulers, Show guides and Lock guides (EX-055), Hand tool
    (EX-009), and Add / Remove motion blur for all moving clips (EX-060).
  - UI also owns panel docking of the player and Jobs panel, the status-bar
    job indicator, preferences storage (UI-018, including EX-056's playback
    priority and the hardware toggles of EX-018/EX-034), the time display
    style offered from the player readout (UI-019), safe mode (UI-024,
    offered by EX-001's unavailable state), tooltips that name keys
    (UI-039) and toasts (UI-040).

## Out of scope (and why)

- **Uploading or "share to" any platform, and cloud rendering or cloud
  drafts.** FreeCut is local-only, with no accounts and no third-party
  services. Platform presets only set technical parameters (EX-042). This
  includes the reference app's post fields (title, description, tags,
  visibility), posting queues and daily limits, and its share targets' rules
  (file-size caps, length caps, "only a video frame may be the cover").
  Local substitutes: presets (EX-041, EX-042), target-size rate control
  (EX-028), the cover image and its JPEG sidecar (EX-046). The user uploads
  the file with whatever tool they like.
- **Share-for-review links** (hosted copies with passwords, comments and
  download permissions). Needs hosting. Local substitute: a draft review
  copy with burned-in timecode and a marker list (EX-051), sent by any
  means.
- **Automatic syncing of exports to a cloud space, and cloud backup of
  exported files.** Needs an account and storage service. Users can choose
  an export folder inside any sync folder they already use (EX-023).
- **Server-based copyright or music checks before export,** and online
  commercial-use scans. These require an online service and a rights
  database. FreeCut does not police the user's content. Local substitute:
  the asset credits sheet (OV-041), which lists every bundled or imported
  asset used with its licence note.
- **Watermarks, paid tiers and export-feature gating.** Against the project's
  free-forever principle (EX-040). The reference app's free-tier brand
  watermark (with its corner placement) has no FreeCut counterpart; the
  user's own opt-in logo (EX-059) is the only mark FreeCut can add.
- **Forced "AI-generated" labels on exports.** The reference app stamps a
  label that cannot be hidden on segments made with its cloud generative
  tools and shows a compliance notice while exporting. FreeCut ships no
  generative models; media from a generator the user registers locally
  (AI-026, later) is already tagged as AI-generated in its media metadata.
  Export stamps no visible mark. If provenance in exported files is ever
  wanted, it would be open metadata (for example an open content-credentials
  manifest), with any visible label left to the user (for example as text or
  EX-059's logo).
- **Server-side task queues with remote wait estimates** (queued AI jobs,
  automatic server retries, credit checks). FreeCut's long jobs run locally
  in PM-073's task list with local progress and ETA; export waits for them
  (EX-022, EX-036).
- **Waiting for media held in the reference app's cloud space** ("not stored
  locally" or "still downloading"). Every FreeCut media item is a local
  file. The nearest local case, files left online-only by a third-party sync
  client, gets a preflight warning (EX-036, PM-114).
- **Contact-support paths for export errors.** FreeCut has no support
  service. Failures carry a stable code, a fix and "Copy details" (EX-022),
  matched to the user guide's troubleshooting page.
- **Tablet-specific export rules** (keep the screen on, keep the app in the
  foreground, save to the photo library). FreeCut is a desktop app; exports
  already keep the system awake (EX-022).
- **Dolby-family audio codecs** (AC-3, E-AC-3, AC-4). AU mixes to stereo,
  for which AAC, Opus, FLAC and PCM are enough (EX-031, AU-032). AC-4 has no
  open encoder, and E-AC-3 needs a licensing review. AC-3 may be revisited
  only together with surround export.
- **Interlaced output, field order and broadcast deliverables** (MXF, XDCAM,
  DCP, closed-caption CEA-608/708). These are niche for this audience. They
  may be revisited much later as presets if contributors want them.
- **Surround (5.1/7.1) export.** AU mixes to stereo (see `audio.md`).
- **HDR export (formerly EX-033):** HDR10 (PQ) or HLG output with BT.2020
  tags and mastering or content-light metadata.
  - SDR Rec.709 is the single working and output space (CL-002). PM and CL
    also keep HDR working spaces and HDR output out of scope, and no spec
    plans one.
  - HDR output only makes sense on top of an HDR working space. That needs
    HDR-aware compositing, a tone-mapped preview on SDR displays and a
    defined reference white for SDR graphics.
  - It will be revisited only together with such a working space. The ID
    EX-033 stays reserved, so old references still resolve.
  - HDR sources are tone-mapped to SDR at decode (CL-002, EX-032). 10-bit SDR
    output (EX-030) is not affected.
- **Dolby Vision and other proprietary HDR metadata formats.** These need
  licenses and SDKs that are incompatible with the GPL. They would stay out
  even if open HDR10/HLG output were revisited.
- **Generated cover art.** Cover images made by a text-to-image model or a
  cloud service are out of scope. EX-053 composes covers only from the
  user's frames and files and FreeCut's own or open-licensed assets.
  Subject cut-outs for covers come from local tools in other areas, for
  example OV-023, if at all.
- **Live streaming output.** A different product category.
- **AI upscaling, denoise or frame interpolation as export options.** Owned by
  the AI area or TR-042, if at all, never hidden inside export.
- **Hardware video I/O cards** (SDI/HDMI capture-playback boards). Vendor
  SDKs are not GPL-compatible. EX-020 covers a second desktop display.
- **Project interchange export** (EDL/XML/OTIO). Not an export-dialog output
  type: timeline interchange is PM-103 (OpenTimelineIO, later).

## Clean-room notes

- **Presets and numbers are ours.** Quality tiers, the bitrate formula
  (EX-027), GIF defaults, platform preset names and sizes, the vertical-feed
  zones (EX-012) and safe-area percentages are FreeCut's own choices. They are
  based on general video-engineering practice and public standards, not on
  the reference editor's tables. No preset names from the reference editor are
  reused.
- **No borrowed artwork.**
  - The offline/error slates, guides, safe areas and vertical-feed zones are
    drawn procedurally: lines, rectangles and text in the UI font. That font is
    the open-licensed font the app bundles for its UI (for example an
    OFL-1.1 font; the choice belongs to UI), never a font taken from the
    reference editor.
  - The blue-noise dither mask (EX-032) is generated by our own build-time tool
    (void-and-cluster method) or taken from a CC0-licensed texture. The
    ordered (Bayer) matrix is a formula, not an asset.
  - No app-interface imitation and no platform logos appear anywhere. Platform
    names appear only in tooltips, descriptively.
  - Player and transport icons come from FreeCut's own icon set (UI).
- **Cover designer (EX-053):**
  - The cover layout presets and the 6 palettes are made by FreeCut
    contributors and released under CC0-1.0, as JSON data. They are not
    modelled on the reference editor's cover or text designs, and none of
    its design names are reused.
  - The layouts use only:
    - the fonts FreeCut bundles under the SIL OFL 1.1 (TX-003);
    - FreeCut's own or CC0 sticker packs (OV-019), whose licences are
      recorded in the pack manifests (OV-040);
    - procedural shapes and frames (OV-024 to OV-029).
  - Users may add their own images, fonts and stickers.
  - No reference-editor cover designs, text designs, stickers, fonts or
    filters are copied or bundled.
  - The From frame palette is our own algorithm (k-means on a downscaled
    frame, plus a WCAG contrast check). It is not a model or an asset.
- **Encoders and licenses** (all through FFmpeg, which is LGPL-2.1+, or
  GPL-2.0+ when built with GPL components; both are compatible with FreeCut's
  GPL-3.0-or-later):
  - x264 and x265: GPL-2.0-or-later;
  - OpenH264: BSD-2-Clause, as an optional fallback;
  - SVT-AV1: BSD-3-Clause-Clear plus the AOMedia Patent License 1.0;
  - libaom: BSD-2-Clause plus the AOMedia Patent License 1.0;
  - libvpx and libopus: BSD-3-Clause;
  - LAME: LGPL-2.0+;
  - FFmpeg's native AAC, ProRes (`prores_ks`), QuickTime Animation
    (`qtrle`, EX-048), GIF and PNG encoders: LGPL;
  - FDK-AAC is **excluded** because its license is not GPL-compatible.
- **ProRes** is produced by FFmpeg's independent implementation. The UI says
  "ProRes 422 HQ (FFmpeg encoder)" and makes no claim of Apple certification.
- **Patents:** H.264, HEVC and AAC are patent-encumbered in some
  jurisdictions. Distribution builds may be compiled without some encoders,
  so FreeCut detects encoders at runtime (EX-029/030/034) and never assumes
  one exists. Which encoders official builds include is a packaging decision
  outside this spec.
- **Hardware encoders and decoders** are reached only through FFmpeg's public
  hwaccel interfaces. No vendor SDK code is copied into FreeCut.
- **New player and export extras are ours too.** The vertical-feed zone
  layouts (EX-012), the guide limit (EX-055), the playback-priority
  thresholds (EX-056), the cover suggestion scorer (EX-046), the failure
  codes and their wording (EX-022) and the logo placement defaults (EX-059)
  are FreeCut's own choices. The vertical-feed layouts imitate no app's
  interface, icons or artwork. The completion sound (EX-038) is the OS's
  standard notification sound, so FreeCut ships no sound asset for it.
- **Research method:** behaviour notes come from public web pages (the
  reference editor's help center, resource pages and tool pages, plus the
  documentation of Kdenlive, OpenShot and Shotcut) and from a paraphrased
  inventory of the reference app's English UI labels, taken from the bundled
  English localisation file of a locally installed copy (CapCut desktop
  9.5.0; see Provenance in the [README](README.md)). Nothing was decompiled,
  disassembled or traced, and no assets, presets, project files or network
  traffic were inspected or copied. The
  labels were used only to learn which options, ranges, limits and refusal
  messages exist. Every label in this spec is FreeCut's own wording, and
  where FreeCut chooses differently the entry says so in one line
  ("Alternative seen in the reference app").

## Sources

Research note: the web-search tool's quota was exhausted for the first pass,
so sources were limited to pages fetched directly. Details marked "Reference
(unverified)" in the text rely on general knowledge and should be re-checked.
In review, the in/out export, watermark, slow-preview, audio-format, vertical
format, YouTube-editor and resizer pages, plus the Kdenlive and Shotcut
shortcut pages, were re-read to check wording and facts. For the cover
designer pass (EX-053) the web-search quota was again exhausted. Public pages
were fetched directly, and only the reference editor's thumbnail guide
described the desktop cover editor. Several guessed help-page addresses did
not exist. A later pass merged a paraphrased inventory of the reference
app's English UI labels into this spec ("Reference (observed)" lines, and
EX-054 to EX-060). It resolved several earlier "unverified" notes (still-frame
export, codec choice, GIF sizes, the export frame-rate list, the cover's
first-frame option); where its readings disagreed with each other or with
public pages, the entry names the conflict and FreeCut's choice.

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- CapCut Help Center (index) — https://www.capcut.com/help
- How to export a part or clip of a video (in/out marks) — https://www.capcut.com/help/export-clip-of-video
- Removing the watermark when exporting a template video — https://www.capcut.com/help/remove-watermark-when-exporting-video
- Why a template plays in slow motion (preview performance and quality) — https://www.capcut.com/help/template-play-in-slow-motion
- Exporting a third-party project to CapCut (project formats) — https://www.capcut.com/help/how-to-export-pro-project
- CapCut desktop video editor feature page — https://www.capcut.com/tools/desktop-video-editor
- Best free video editor for YouTube (export up to 4K/60 fps, no watermark) — https://www.capcut.com/resource/best-free-video-editor-for-youtube
- Splitting sound from video (audio format list in the export dialog) — https://www.capcut.com/resource/split-sound-from-video
- Vertical short video format (9:16, 1080×1920, central safe zone) — https://www.capcut.com/resource/youtube-shorts-format
- Add subtitles to video (SRT export, burned-in captions) — https://www.capcut.com/tools/add-subtitles-to-video
- Video resizer (aspect presets, resolutions) — https://www.capcut.com/tools/video-resizer
- Online video editor (export flow, no watermark) — https://www.capcut.com/tools/online-video-editor
- Online video converter (MP4/MOV output) — https://www.capcut.com/tools/online-video-converter
- Free video compressor (resolution, frame rate, quality trade-offs) — https://www.capcut.com/tools/free-video-compressor
- CapCut llms.txt site index (used to locate pages) — https://www.capcut.com/llms.txt
- How to make a thumbnail for YouTube videos (desktop Cover button, frame or own image, text, stickers and designs on the cover) — https://www.capcut.com/resource/how-to-make-a-thumbnail
- Resizing YouTube thumbnails (1280×720, 16:9, minimum width, file-size limit, PNG/JPG) — https://www.capcut.com/resource/youtube-thumbnail-size
- Online graphic design tool (still-image designer: templates, text styling, background tools) — https://www.capcut.com/tools/online-graphic-design
- Kdenlive manual: Rendering (presets, ranges, job queue, hardware presets, audio-only, image sequences) — https://docs.kdenlive.org/en/exporting/render.html
- Kdenlive manual: Monitors (preview resolution, fullscreen, overlays, zoom) — https://docs.kdenlive.org/en/user_interface/monitors.html
- Kdenlive manual: Keyboard shortcuts (J/K/L speeds, loop zone, frame steps) — https://docs.kdenlive.org/en/user_interface/shortcuts.html
- OpenShot user guide: Export (simple/advanced, targets, frame range, hardware targets) — https://www.openshot.org/static/files/user-guide/export.html
- Shotcut features (preview scaling, zoom levels, safe areas, export frame, hardware encoding) — https://shotcut.org/features/
- Shotcut keyboard shortcuts (J/K/L, loop, export frame, fullscreen) — https://shotcut.org/howtos/keyboard-shortcuts/
