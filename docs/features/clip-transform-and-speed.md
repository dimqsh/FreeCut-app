# Transform, canvas, masks, keying, keyframes & speed

> Status: draft · Owns: keyframes & easing (keyframe model, interpolation, easing curves, the keyframe split rule), the project-wide rule for storing non-time values, speed/retiming model (time map, constant speed, speed curves, reverse, the frame-sampling enum, and which source frame a freeze-frame still shows), clip transform & framing model, on-canvas align & distribute, crop, opacity, masks, chroma/luma key, stabilization, canvas background fill · Crates: freecut-timeline, freecut-render, freecut-media, freecut-audio, freecut-project, freecut-app

## Summary

This area covers how a clip is placed and shaped on the project canvas and how
its playback time is bent. A FreeCut user can move, scale, rotate, flip and
crop any visual clip, either from the inspector or by dragging handles in the
player; fill the empty letterbox area with a color, a blurred copy of the clip
or an image; fade it with opacity and combine it with lower layers through
blend modes; cut it with shape masks; remove a green or blue screen; steady
shaky footage; animate almost any numeric property with keyframes and easing;
and change speed with a constant factor, a speed curve or reverse playback.
Several selected items can be aligned and spaced evenly on the canvas, and
later tiers add text-shaped masks, a perspective tilt and speed ramps timed
to music beats. The same time map decides which frame a freeze frame (a
timeline edit, TL-031) shows. Everything is non-destructive, undoable and
stored in the project file; nothing here needs a network connection.

### Shared conventions used by every feature below

- **Canvas space.** The project canvas (width × height in pixels and frame rate
  come from project settings, see `project-and-media.md`) has its origin at the
  canvas center, +X to the right, +Y downward, rotation in degrees, positive =
  clockwise. Positions are *stored* as fractions of canvas width/height so a
  project keeps its layout when the export resolution changes, and *shown* in
  project pixels.
- **Clip space.** Masks, crop and chroma key work in the clip's own frame
  (after crop, before transform) with normalized coordinates: (0,0) = clip
  center, ±0.5 = clip edges. They therefore move, scale and rotate with the
  clip.
- **Time.** All times (keyframe times, speed-curve positions, freeze times,
  durations) are rationals (`i64` ticks + timebase) as defined in
  `timeline-editing.md` (TL-001). Speed factors are also rationals because they
  convert one time into another. Floats for *time* appear only at the UI edge
  (drawing graphs, showing seconds).
- **Non-time values (project-wide rule, recorded here once).** Only *time*
  must be rational (project `CLAUDE.md`). Every other scalar is stored in
  whatever type its owning spec picks, and that spec says so in its own model
  notes: this area and OV use `f64` (positions, scale, angles, opacity,
  feather); CL and TX use fixed-point integers; AU stores gain as `GainDb(i32)`.
  All of them animate through the same generic keyframe engine,
  `Animatable<T: Interpolate>` (TR-027), where each value type supplies its own
  interpolation. There is no single project-wide scalar type, and no spec
  needs to convert another area's values to its own.
  *FreeCut decision (2026-10-12, mvp):* this area's static values are
  stored as fixed-point integers too, so clips stay exactly comparable
  (`Eq`) in the edit model and its tests; `f64` appears only when the
  render plan builds the matrix. The units are listed under TR-001.
- **Clip-local time.** `t` = output time since the clip's timeline start
  (0 ≤ t < clip duration). `s` = source time inside the media file. The
  retiming model (TR-036) is the single function that maps `t → s`.
- **Where this area sits in the render pipeline.** The per-frame order is
  owned by FX-001 in `effects-transitions-animations.md`; this spec only says
  which stage each of its features feeds: source time from the time map
  (TR-036) → L1; stabilization correction then crop (TR-026, TR-009) → L3;
  chroma/luma key (TR-022..TR-025) → L4; shape mask (TR-017..TR-021) → L7;
  transform (TR-001..TR-006) → L8; opacity (TR-015) → L9. The blend mode
  stored next to opacity (dropdown placed by TR-016, modes owned by OV-004)
  is executed by FX-001 at L10. Inside L8 this area fixes the order: flip,
  then scale and rotate about the anchor, then translate. The canvas
  background (TR-011..TR-014) is the accumulator's first fill (FX-001 step 1).
- **Derived data on disk.** Everything this area computes in the background is
  deterministic and can be rebuilt from the media and the clip settings:
  stabilization analysis (TR-026), reverse-playback preview chunks (TR-043),
  optical-flow in-between frames (TR-042), and AI-interpolated frames (AI-018,
  selected through TR-041). All of it lives in the **media cache (PM-074)**,
  never in the project file and never in the generated-media store (PM-041),
  which is reserved for derived files a clip points at that are user-approved
  or non-deterministic. Clearing or evicting the cache only means the data is
  rebuilt; preview falls back meanwhile, and export waits for the rebuild
  where it needs the data (reverse is the exception: export never needs its
  preview chunks). No feature in this area writes to the PM-041 store.
- **Undo.** Every edit below is an operation on the undo stack. A continuous
  gesture (slider drag, handle drag, curve-point drag) is coalesced into one
  undo entry when the pointer is released; the coalescing rules themselves are
  defined in `timeline-editing.md`.
- **Locked tracks.** Clips on a locked track cannot be selected for editing in
  the player and every control in this area is read-only for them (lock
  semantics are defined in `timeline-editing.md`, TL-009).
- **Main track.** "Main track" means TL's single magnetic main track, the
  lowest visual layer (TL-004); canvas background settings apply to its clips.
- **Settings combine; they do not cancel each other (FreeCut rule).** Because
  every result here is either a pure function of source time (TR-036) or data
  stored in source coordinates (tracking, beat markers, mattes, stabilization
  analysis), this area's settings stack instead of excluding one another:
  speed or a speed curve keeps beat markers (AU-025) and motion tracking
  (AI-008); rotating keeps tracking; crop and stabilization keep auto-reframe
  keyframes (AI-016); chroma key and automatic cutout (AI-014) multiply;
  reverse keeps every audio tool (AU-021, AU-022). A combination that cannot
  work is *disabled with a reason*, never silently removed. Alternative seen
  in the reference app: turning on one of these removes the other after a
  confirmation.
- **Disabled clips and hidden tracks.** A clip disabled with TL-036, or on a
  hidden track (TL-008), is not drawn in the player. Tools that need the
  user to point at the clip's own picture (chroma-key eyedropper, on-canvas
  handles and mask handles) are unavailable for it, with a hint; numeric
  fields stay editable.

## Features

### TR-001 Clip transform: position, scale, rotation — mvp

**What the user sees:** With a visual clip (video, image, text, sticker,
shape) selected, the inspector's *Transform* section shows Position X/Y, Scale
and Rotation, each with a numeric field, a scrub-able label and a reset arrow.
The preview updates live while the value changes.

**Inputs / controls:**
- Position X, Y: project pixels, origin at canvas center. Range ±4 × canvas
  dimension (FreeCut default). Default 0, 0. Field accepts decimals; shows
  whole pixels unless zoomed in.
- Scale: percent of the clip's fitted size (see TR-002). Range 1 %–1000 %
  (FreeCut default). Default 100 %.
- Rotation: degrees, range −3600° to +3600° (ten full turns, so keyframed spins
  are possible). Default 0°. The value is not wrapped: 370° stays 370°.
- Modifier while scrubbing labels: Shift = 10× step, Alt/Option = 0.1× step.
- Section reset button: returns position/scale/rotation (and anchor, flips,
  non-uniform scale) to defaults.
- Not here: one-click *center horizontally / vertically* buttons and
  multi-item alignment are TR-045 (v1); the width × height size readout is
  TR-004 (v1); quick ±90° buttons are TR-005 (v1).

**Behavior / output:** The clip is drawn on the canvas at the given position,
scaled about its anchor (TR-006, default center), then rotated about the same
anchor. Parts outside the canvas are clipped by the canvas at render time but
keep existing in the model (moving the clip back reveals them).

**Edge cases:**
- Scale entered as 0 or negative: clamped to 1 %. Negative scale is never used
  for flipping; flips are separate booleans (TR-003).
- Clip moved entirely off-canvas: allowed; the timeline shows the clip as
  normal, the player shows nothing for it, and on-canvas handles are drawn at
  the canvas border pointing toward the clip (so it can be grabbed back).
- Audio-only clips: section hidden.
- Property is animated (has keyframes, TR-027): editing creates/updates a
  keyframe at the playhead instead of changing the static value.
- Project resolution changed later: positions stay proportional because they
  are stored as fractions of the canvas.
- Project aspect ratio changed later (e.g. 16:9 → 9:16): fractional positions
  are kept; clips may land in different visual places, but nothing is lost.
- Rotation of exactly ±90°/270° on a non-square clip: bounding box for
  hit-testing is recomputed from the rotated rectangle, not the unrotated one.
- Clip that follows a motion track, or is the tracked clip (AI-008, v1):
  changing its rotation, position or scale keeps the tracking, because the
  samples are mapped through the clip's current transform (shared rule
  "Settings combine"). Alternative seen in the reference app: rotating a
  clip removes its tracking.

**Undo:** `SetTransformProperty { clip, property, old, new }`; reverses by
restoring `old`. Section reset is one `SetTransform { clip, old: Transform,
new: Transform::default() }` that also removes keyframes of those properties
(restored on undo).

**Model notes:** `freecut-timeline`: `Transform { position: Vec2Norm,
scale: ScaleXY, rotation_deg: f64, anchor: Vec2Norm, flip_h: bool, flip_v:
bool, fit: FitMode }` on every visual clip; each numeric field is an
`Animatable<T>` (static value + optional keyframe track, TR-027).
`freecut-render`: builds the clip's 2D affine matrix from the evaluated values
each frame. `freecut-project`: fields serialize with defaults so older files
load.

**FreeCut mvp model (2026-10-12):** `Clip.transform: Transform { position:
CanvasPos { x, y }, scale: Scale, rotation: Angle, flip_h, flip_v, opacity:
PerMille }` on every clip; audio clips keep the default. Units: position in
millionths of the canvas width / height from the canvas centre (±4,000,000);
scale in basis points of the fitted size (10,000 = 100 %, range 100–100,000);
rotation in 1/100° (±360,000); opacity per-mille (TR-015, kept here until the
Compositing section exists). Anchor, non-uniform scale and fit mode come
later; every clip uses the Fit baseline for now. `EditRequest::SetTransform {
seq, clips, edit: TransformEdit::{Position, Scale, Rotation, FlipH, FlipV,
Opacity, Reset} }` sets one value on every listed visual clip, clamped to the
ranges above; audio clips are skipped, an audio-only selection is refused,
a clip on a locked track refuses the edit, and an unchanged value records
nothing. `Reset` is the section reset: everything but opacity. The timeline
builds the matrix (`freecut_timeline::plan::clip_placement`): flip, scale,
rotate clockwise about the clip centre, then translate to canvas centre +
position. A picture layer's matrix is that placement after the Fit baseline;
a text layer's origin is its text box centre, in canvas px at 100 % scale.

**FreeCut mvp UI (2026-10-12):** the inspector's Transform section shows for any visual clip: Position X/Y fields in project pixels from the canvas centre (±4 × canvas size), Scale (1–1000 %), Rotation (−3600° to 3600°, not wrapped), Flip Horizontal/Vertical toggles (TR-003), an Opacity slider (0–100 %, TR-015), and a **Reset transform** button (`TransformEdit::Reset`, opacity kept). Fields commit when a typed value is confirmed; drags on a field or slider merge into one undo entry (TL-043). With several clips selected the values shown are the first visual clip's, and an edit sets that value on all of them. Scrub-label modifiers (Shift ×10, Alt ×0.1), per-row reset arrows and keyframes are not built yet.

### TR-002 Fit mode and the 100 % baseline — mvp

**What the user sees:** A *Fit* dropdown in the Transform section with *Fit*,
*Fill*, *Stretch* and *Original size*. New clips arrive fitted inside the
canvas, so a landscape clip in a vertical project shows bars above and below
(which TR-011..TR-014 can fill).

**Inputs / controls:** Fit (default for new clips): whole clip visible,
letterboxed. Fill: clip covers the canvas, overflow cropped by the canvas.
Stretch: clip is stretched to exactly the canvas (non-uniform). Original size:
one source pixel = one canvas pixel. From v1, a *Fit to canvas ▸ Fit / Fill /
Stretch* item in the clip context menu sets that mode on every selected clip
and also resets Position, Scale and Rotation to their defaults, so the clips
sit exactly in the frame (the dropdown alone never touches those values). The
default mode for newly added media is a project setting (PM-008).

**Behavior / output:** The fit mode defines what *Scale = 100 %* means. It is
computed from the clip's **cropped** size (TR-009), so cropping a 16:9 clip to
9:16 in a 9:16 project and staying in *Fit* fills the frame without any manual
scaling. Changing the fit mode does not touch Position/Scale/Rotation values;
it only changes the baseline they apply to.

**Edge cases:**
- Media with non-square pixels (anamorphic, some DV): the baseline uses the
  display aspect ratio reported by probing (`freecut-media`).
- Rotated-metadata phone footage (e.g. 90° rotation tag): the baseline uses the
  display orientation; the user's Rotation is applied on top.
- Missing media: baseline uses the last known dimensions stored in the media
  reference; if none are known, 1920×1080 is assumed (FreeCut default) and the
  clip shows an offline placeholder.
- Images larger than 16k on a side: still fitted correctly; render uses a
  downscaled texture (render-side limit, not a model limit).
- *Fit to canvas* on a clip whose Position/Scale/Rotation are keyframed: the
  reset follows TR-001's section reset (those keyframes are removed; undo
  restores them). Clips on locked tracks in the selection are skipped with a
  status message.

**Undo:** `SetFitMode { clip, old, new }`. *Fit to canvas* is one compound op
per command (a `SetFitMode` plus a `SetTransform` per selected clip).

**Model notes:** `FitMode` enum in `freecut-timeline`; the baseline is a pure
function of (clip display size after crop, canvas size, fit mode) so it can be
unit-tested without media.

### TR-003 Flip horizontal / vertical — mvp

**What the user sees:** Two toggle buttons (mirror left-right, mirror
top-bottom) in the Transform section; flip-horizontal is also a button on the
timeline toolbar.

**Inputs / controls:** `flip_h`, `flip_v` booleans, default off.

**Behavior / output:** The clip image is mirrored about its own center before
scale/rotation, so a flipped clip still rotates clockwise for positive angles.
Masks and crop are defined in clip space before the flip, so they flip with the
image (a mask on the left side ends up on the right). Text clips flip like any
other visual (readable text mirrored) — this matches what users ask for when
they mirror selfie footage.

**Edge cases:**
- Both flips on = 180° rotation visually, but stored as two flips (no
  rotation change).
- Flipping does not change the anchor position on the canvas.
- Not keyframable (discrete); toggling while the property is animated is
  allowed because flips are never animated.

**Undo:** `SetFlip { clip, axis, old, new }`.

**Model notes:** booleans in `Transform`; `freecut-render` applies a −1 scale on
the matching axis in clip space.

### TR-004 Non-uniform scale (independent width/height) — v1

**What the user sees:** A link toggle next to Scale. When unlinked, Scale
splits into Scale X and Scale Y.

**Inputs / controls:** Linked by default. Scale X / Y each 1 %–1000 %. When
re-linking, both values become Scale X (FreeCut default; Y snaps to X).
- *Size* row next to Position: the clip's displayed width × height in project
  pixels (cropped, fitted and scaled, before rotation). Editing W or H sets
  the matching scale so the clip has that size; while linked, the other side
  follows.
- *Restore proportions* button, shown only while Scale X ≠ Scale Y: sets Y to
  X (the clip's own aspect ratio comes back) and leaves the link state as it
  is.

**Behavior / output:** The clip is stretched along its own (unrotated) axes,
then rotated, so a rotated, non-uniformly scaled clip stays a rotated
rectangle (no shear).

**Edge cases:**
- Unlinking while uniform scale has keyframes: the uniform track is copied into
  both X and Y tracks (same times, values, easing).
- Re-linking while X and Y have different keyframes: the X track is kept, the Y
  track discarded; confirmation dialog; undo restores Y.
- Text clips: allowed (it stretches the rendered text); text layout itself is
  owned by `text-and-captions.md`.
- *Restore proportions* when Scale Y has keyframes that differ from X's: the
  same confirmation as re-linking, because Y's keyframes are replaced. With
  no keyframes it applies at once (undo is enough). Alternative seen in the
  reference app: it always asks before restoring the aspect ratio.
- Size typed as 0 or below the 1 % scale floor: clamped to 1 % with the
  field showing the resulting size.

**Undo:** `SetScaleLink { clip, old: ScaleXY, new: ScaleXY }` carrying both
tracks; *Restore proportions* and size edits use the same op (or
`SetTransformProperty` for a single axis).

**Model notes:** `enum ScaleXY { Uniform(Animatable<f64>), Separate {
x: Animatable<f64>, y: Animatable<f64> } }`.

### TR-005 Quick rotate 90° — v1

**What the user sees:** "Rotate 90° clockwise" and "Rotate 90°
counter-clockwise" buttons in the Transform section and in the clip context
menu.

**Inputs / controls:** Each click adds +90° (clockwise) or −90°
(counter-clockwise) to Rotation. Keyboard shortcuts assigned in
`workspace-and-shortcuts.md` (unbound by default, UI-031).

**Behavior / output:** Adds ±90° to the current value (or to the keyframe at
the playhead if Rotation is animated). After four clockwise clicks the value
is 360°, not 0° (four counter-clockwise clicks give −360°) — the user can
reset it. Results past the ±3600° range are clamped (TR-001).

**Edge cases:** On a multi-selection, each selected clip gets +90° in a single
undo step. Clips on locked tracks in the selection are skipped with a status
message.

**Undo:** Same `SetTransformProperty` op (batched for multi-select).

**Model notes:** UI convenience only; no new model.

### TR-006 Anchor point — v1

**What the user sees:** Anchor X/Y fields in the Transform section and a
cross-hair marker in the player that can be dragged with a modifier (Alt/Option
+ drag on the marker).

**Inputs / controls:** Anchor in clip-normalized coordinates, (0,0) = center,
±0.5 = edges; range ±2 (so the pivot can sit outside the clip). Default (0,0).
Checkbox "Keep image in place when moving anchor" (default on).

**Behavior / output:** Scale and rotation pivot around the anchor. With "keep
in place" on, moving the anchor also adjusts Position so the picture does not
jump; with it off, the picture moves.

**Edge cases:**
- Anchor animated while Position is also animated and "keep in place" is on:
  the compensation writes a Position keyframe at the playhead too (both in one
  undo step).
- Flip toggled: anchor stays at the same canvas point (anchor is mirrored in
  clip space).

**Undo:** `SetAnchor { clip, old_anchor, new_anchor, old_position,
new_position }`.

**Model notes:** `anchor: Animatable<Vec2>` in `Transform`.

### TR-007 On-canvas transform handles in the player — mvp

**What the user sees:** Clicking a visual clip in the player selects it (and
the matching timeline clip) and draws a bounding box: four corner handles,
four edge handles (only when scale is unlinked), a rotation handle above the
top edge, and the anchor marker (v1). Dragging inside the box moves the clip.

**Inputs / controls:**
- Drag inside: move. Shift: constrain to horizontal/vertical.
- Corner drag: uniform scale about the anchor; Alt/Option: scale about the
  opposite corner instead.
- Edge drag (unlinked scale only): scale one axis.
- Rotation handle: rotate; Shift snaps to 15° steps.
- Arrow keys: nudge 1 project pixel; Shift+arrow: 10 px.
- Click on empty canvas: deselect. Ctrl/Cmd+click: cycle selection to the next
  clip underneath at that point.
- Double-click on a text clip: enters text editing (owned by
  `text-and-captions.md`).

**Behavior / output:** Only clips visible at the playhead can be picked.
Hit-testing goes top track → bottom track, using each clip's transformed
bounding box, skipping clips at 0 % opacity, disabled clips (TL-036) and
clips on hidden or locked tracks. During a drag a small tooltip shows the
new value (e.g. "X 312 Y −40", "118 %", "37.5°"). The player draws handles at screen resolution regardless of
preview zoom (the player's zoom/fit is owned by `preview-and-export.md`).

**Edge cases:**
- Playhead on a transition between two clips: either clip can be picked; the
  upper/incoming one wins on overlap.
- Clip with mask (TR-017): the transform box follows the full clip rectangle,
  not the mask shape, so a clip whose visible part is tiny can still be
  grabbed.
- Clip mostly off-canvas: handles are drawn even outside the canvas area when
  the preview has room; otherwise an edge indicator is drawn (TR-001).
- Animated property: dragging writes a keyframe at the playhead (auto-key,
  TR-028).
- Drag cancelled with Esc: value returns to the pre-drag value and no undo
  entry is created.
- Playback running: handles hidden; dragging is disabled until playback stops
  (FreeCut default, avoids keyframe spam).

**Undo:** One coalesced `SetTransformProperty` (or `SetTransform` for combined
move+scale) per drag.

**FreeCut mvp subset (2026-10-12):** built: click selects the topmost visual clip under the pointer (top track first, through the inverse of each layer's matrix; text uses the laid-out box the preview thread reports with each frame), click on empty canvas clears the selection, dragging inside a clip moves it (`SetTransform` Position on that clip, one gesture, so one undo entry), Shift keeps the larger axis, Esc during the drag reverts it with no undo entry (`Editor::cancel_gesture`), and selected clips get a thin outline of their transformed box. Clips on locked tracks and at 0 % opacity can't be picked. While playing, the canvas ignores the pointer and draws no outline. Double-click on a text clip focuses the inspector's content field (TX-002's interim edit mode). Not built yet: corner, edge and rotation handles, arrow-key nudges, the drag tooltip, Ctrl/Cmd+click cycling, and edge indicators for off-canvas clips. The outline follows the latest rendered frame, so it can lag the document by one preview frame.

**Model notes:** Interaction lives in `freecut-app` (egui overlay over the
player texture); the app needs read access to the evaluated transform at the
playhead, computed by `freecut-timeline` (pure) — not read back from the GPU.

### TR-008 Snapping and alignment guides on the canvas — v1

**What the user sees:** While moving or scaling in the player, thin guide lines
appear and the clip snaps when its center or edges come near the canvas
center lines, canvas edges, or the center/edges of other visible clips.

**Inputs / controls:** Snapping on by default; hold Ctrl/Cmd during the drag
to suspend; toggle in the player's overflow menu and as a *Canvas snapping*
command in the command registry (UI-030; unbound by default, separate from
timeline snapping `N`, TL-028). The on/off state is a per-user preference
(UI-018), not stored in the project. Snap distance 6 screen pixels (FreeCut
default). Rotation snaps to 0°/90°/180°/270° within 2°.

**Behavior / output:** Snapping only alters the value being dragged; it never
moves other clips. Safe-area overlays (title/action safe) are drawn by the
player (`preview-and-export.md`); when visible they also act as snap targets.
Explicit alignment of one or several items (align edges or centers,
distribute evenly) is TR-045.

**Edge cases:** Rotated clips snap their rotated bounding box. Snapping is
computed on screen pixels, so it behaves the same at any preview zoom. Clips
that are disabled or on hidden tracks are not snap targets (they are not
drawn).

**Undo:** n/a (modifies the same drag op as TR-007).

**Model notes:** `freecut-app` only.

### TR-009 Crop (free and fixed ratios) — v1

**What the user sees:** A *Crop* button on the timeline toolbar and in the
inspector opens a crop view in the player showing the whole source frame,
darkened outside a crop rectangle with corner/edge handles and a ratio
dropdown. *Apply* / *Cancel* / *Reset* buttons.

**Inputs / controls:**
- Ratio presets: Free, Original (source ratio), Project canvas ratio, 16:9,
  9:16, 1:1, 4:3, 3:4, 4:5, 2:1, 2.35:1 (FreeCut list).
- Crop rectangle as left/top/right/bottom insets (0–100 % of the source,
  shown also as pixels). Drag inside to move the rectangle.
- Minimum crop size: 16×16 source pixels (FreeCut default).

**Behavior / output:** Crop removes the outside of the source frame. The fit
baseline (TR-002) is recomputed from the cropped size, so the cropped area is
re-fitted to the canvas. Position/Scale/Rotation keep their values. Crop is
applied before masks and chroma key, so mask coordinates refer to the cropped
frame.

**Edge cases:**
- Switching ratio with an existing free crop: the new rectangle is the largest
  rectangle of that ratio centered on the old crop's center that fits in the
  source.
- Source with rotation metadata: crop works in the displayed (rotated)
  orientation.
- Clip with stabilization: the crop view shows the stabilized frame.
- Masks already set: their normalized coordinates stay as they are, which
  means they move relative to the picture; a one-line hint appears (FreeCut
  choice: we do not silently rewrite mask geometry).
- Cancel discards all changes made in the crop view without an undo entry.
- Crop is static in v1 (not keyframable); panning crops are done with
  Position/Scale keyframes. Keyframable crop is a later extension.
- Image clips and text clips: crop allowed for images; disabled for text
  (text has its own box, see `text-and-captions.md`).
- Clip with auto-reframe or subject-follow keyframes (AI-016, AI-017, later):
  the crop is applied and the keyframes are kept, but they were planned for
  the old framing, so a hint offers *Re-run* (shared rule "Settings
  combine"). Alternative seen in the reference app: cropping removes the
  automatic reframing after a confirmation.
- Observed reference ratio set (free, 16:9, 1:1, 3:4, 4:3, 9:16) is a subset
  of FreeCut's list above.

**Undo:** `SetCrop { clip, old: CropRect, new: CropRect }` on *Apply*.

**Model notes:** `CropRect { left, top, right, bottom }` as fractions of the
displayed source size in `freecut-timeline`; render samples the sub-rectangle.

### TR-010 Straighten inside the crop view — v1

**What the user sees:** A straighten slider under the crop rectangle.

**Inputs / controls:** −45° to +45°, 0.1° steps, default 0°. Checkbox "Auto
zoom to hide corners" (default on).

**Behavior / output:** Rotates the source inside the crop rectangle. With auto
zoom on, the crop rectangle is shrunk to the largest rectangle (same ratio)
that contains no empty corners.

**Edge cases:** Straighten is independent of the clip's Rotation (it is part of
the crop step). Resetting crop also resets straighten.

**Undo:** Part of `SetCrop` (CropRect carries `straighten_deg`).

**Model notes:** `CropRect.straighten_deg: f64`.

### TR-011 Canvas background: solid color — v1

**What the user sees:** In the inspector, a *Background* section for clips on
the main track (TL-004): a mode dropdown (*None*, *Color*, *Blur*, *Image*)
and, for *Color*, a palette of 12 swatches, a color picker and an eyedropper
that samples the preview. (Until this ships, the MVP shows the project
background color from project settings behind every clip.)

**Inputs / controls:** Mode default *None* (= project background color, black
unless changed in project settings, `project-and-media.md`). Color: any opaque
sRGB color.

**Behavior / output:** For the time span of that clip, the canvas area not
covered by the clip is filled with the color. The fill sits under every track,
so overlays above still draw on top.

**Edge cases:**
- Clip fully covers the canvas: background has no visible effect; the
  compositor skips drawing it.
- Clip with opacity < 100 %, a mask or a chroma key: the background shows
  through the transparent parts (intended — it lets users put a backdrop
  behind keyed footage).
- Gaps on the main track: project background color.
- Clip moved from the main track to an upper track: the background setting is
  kept but inactive (section shown disabled with a hint); moving it back
  re-activates it.
- Transition between two main-track clips with different backgrounds: the two
  backgrounds cross-fade with the transition (FreeCut default).
- Background chosen while the clip covers the whole canvas (nothing to fill):
  the setting is stored and a hint says the fill becomes visible once the
  clip is scaled down or letterboxed. FreeCut shows no separate "background
  added" confirmation; the preview is the confirmation.
- Eyedropper: samples the composited preview at the playhead (whatever is on
  screen), so it also works while the clip is disabled or its track hidden;
  it never needs an OS screen-capture permission because it only reads
  FreeCut's own player. Colors from outside FreeCut are typed as hex.
- Not keyframable.

**Undo:** `SetCanvasBackground { clip, old, new }`.

**Model notes:** `enum CanvasBackground { None, Color(Rgb8), Blur { strength:
u8 }, Image { media: MediaRef, fit: BgFit } }` on visual clips in
`freecut-timeline`; drawn by `freecut-render` as the first layer.

### TR-012 Canvas background: blurred fill — v1

**What the user sees:** Choosing *Blur* fills the bars with a soft, blurred,
enlarged copy of the same clip that moves in sync with it. A strength slider
plus four quick-pick buttons (light → heavy).

**Inputs / controls:** Strength 0–100, default 50; quick picks 25 / 50 / 75 /
100 (FreeCut defaults).

**Behavior / output:** The background copy uses the clip's frame after
stabilization and crop but before mask, key and transform, scaled to *cover*
the canvas and blurred with a Gaussian whose radius maps linearly from 0 to 8 %
of the canvas diagonal (FreeCut default). It follows the clip's speed and
reverse (and shows the frozen frame on a freeze-frame still), because it is
the same frame.

**Edge cases:**
- Strength 0 = sharp cover-scaled copy (useful as a "fill" look).
- Very small source (e.g. 320×240): upscaled before blur; blur hides the
  softness.
- Image clips: works (static blurred copy).
- Performance: the blur is computed at reduced resolution (¼ canvas) and
  upscaled; it never runs on the UI thread.

**Undo:** `SetCanvasBackground`.

**Model notes:** WGSL two-pass separable blur in `freecut-render`; nothing new
in the model beyond the enum.

### TR-013 Canvas background: image fill — v1

**What the user sees:** Choosing *Image* lets the user pick an image already in
the project's media library or import one from disk; a small row of bundled
FreeCut patterns (our own CC0 artwork) is offered as well.

**Inputs / controls:** Image source; fit within canvas: Fill (default), Fit,
Stretch.

**Behavior / output:** The image is drawn under the clip for the clip's time
span.

**Edge cases:**
- Image file later missing: background falls back to the project background
  color, and the media library flags it as offline (`project-and-media.md`).
  The clip also gets a small "background missing" badge on the timeline and
  in the Background section, with *Relink…* (PM-091); export preflight
  (EX-036) lists it as a warning, not an error.
- Animated images (GIF/APNG/WebP): first frame only in v1.
- Video files chosen as background: not allowed in v1 (put the video on a
  lower track instead).
- Importing a new file from this control adds it to the media library first
  (one undo step for the whole action).

**Undo:** `SetCanvasBackground` (plus the media-library import op, if any, in
the same compound step).

**Model notes:** The background holds a `MediaRef` like any clip; media
references are owned by `project-and-media.md`.

### TR-014 Apply background to all clips — v1

**What the user sees:** An *Apply to all* button under the Background section.

**Inputs / controls:** One click; confirmation not required.

**Behavior / output:** Copies the current clip's background setting to every
clip on the main track. Clips on other tracks are untouched.

**Edge cases:**
- Main track locked: none changed, button disabled.
- Image clips and freeze-frame stills on the main track receive the same
  setting.
- A status message reports how many clips changed ("Background applied to
  12 clips"); clips that already had the same setting count as unchanged.
- Background settings are not keyframable, so the reference app's refusal
  when target clips hold keyframes on the setting never arises here; the
  rule for batch-copying animated values is TR-034's.
- The pure batch op cannot fail half-way: either every eligible clip is
  changed or, if the command is refused (locked track), none is.

**Undo:** One compound op containing a `SetCanvasBackground` per changed clip.

**Model notes:** Batch op in `freecut-timeline`.

### TR-015 Opacity — v1

**What the user sees:** An Opacity slider (0–100 %) in a *Compositing* section
of the inspector.

**Inputs / controls:** 0–100 %, integer steps in the UI, default 100 %.
Keyframable (TR-027). The control is always labelled *Opacity* and always
reads 100 % = fully visible in every panel (FreeCut default; no inverted
"transparency" scale anywhere).

**Behavior / output:** Multiplies the clip's alpha (after mask and key) before
blending onto lower layers (FX-001 stage L9). Until this ships, every clip is
drawn at 100 %.

**Edge cases:**
- 0 %: clip invisible but still on the timeline; still selectable from the
  timeline, skipped by on-canvas hit-testing.
- Opacity on a main-track clip reveals the canvas background (TR-011).
- Interaction with fade-in/out clip animations from
  `effects-transitions-animations.md`: they multiply with this value.

**Undo:** `SetTransformProperty { property: Opacity, .. }` (or keyframe ops).

**Model notes:** `opacity: Animatable<f64>` in `[0,1]` in a `Compositing`
struct next to `Transform`.

### TR-016 Blend mode placement in the Compositing section — v1

The `BlendMode` type is owned by OV-004 in `overlays-and-stickers.md`: the
mode list, the formulas, serialization and the hover preview. FX-001 stage
L10 (`effects-transitions-animations.md`) executes the blend. This entry only
places the dropdown in this area's Compositing section and states which alpha
the blend weights.

**What the user sees:** The *Blend* dropdown defined in OV-004 sits directly
under the Opacity slider (TR-015) in the *Compositing* section, for every
visual clip including main-track clips.

**Inputs / controls:** The mode list and default (*Normal*) from OV-004. No
extra controls here.

**Behavior / output:** The clip's alpha after key (TR-022), mask (TR-017) and
opacity (TR-015) is what the blend weights, so keyed-out or masked-out pixels
contribute nothing whatever mode is chosen. FX-001 stage L10 performs the
blend with OV-004's formulas; this area adds no blend math of its own.

**Edge cases:**
- Main-track clip: blends with the canvas background fill (TR-011..TR-014) or,
  with background *None*, the project background color.
- Clip at 0 % opacity: no visible effect in any mode.
- Not keyframable (discrete; see Out of scope).
- Reset of the Compositing section returns opacity to 100 % and the mode to
  *Normal* in one undo step.

**Undo:** `SetBlendMode` as defined in OV-004; the Compositing-section reset is
one compound op (`SetTransformProperty { property: Opacity, .. }` plus
`SetBlendMode`).

**Model notes:** The `BlendMode` enum is defined by OV-004 (in
`freecut-timeline`); this area stores it as `blend: BlendMode` in the
`Compositing` struct next to `opacity`.

### TR-017 Shape masks — v1

**What the user sees:** A *Mask* section with a row of shape buttons: None,
Linear (one straight edge), Band (a strip between two parallel edges),
Ellipse/Circle, Rectangle, Heart, Star. Picking a shape shows it immediately in
the preview.

**Inputs / controls (per shape, all in clip space):**
- Common: center X/Y (default 0,0), rotation (−360°…+360°, default 0°),
  feather and invert (TR-018).
- Linear: one edge through the center; keeps one side (default: the upper half
  stays visible). Rotation turns it into a vertical/diagonal split.
- Band: keeps the strip between two parallel edges; band width 0–200 % of the
  clip height, default 50 %.
- Ellipse: width/height 0–400 % of clip's shorter side, default 50 %; a link
  toggle (on by default) keeps it a circle.
- Rectangle: width/height (as ellipse, default 60 %/40 %), corner roundness
  0–100 %, default 0.
- Heart / Star: width/height with link toggle (default on), default 50 %; star
  has 5 points in v1 (point count and inner radius are *later*).
- Later shapes join the same row: Pen (TR-021) and Text (TR-046).

**Behavior / output:** The mask produces a coverage value per pixel (0 =
hidden, 1 = visible) that multiplies the clip's alpha. Shapes are evaluated as
signed-distance functions in a shader so edges stay clean at any scale. The
mask lives in clip space, so moving/scaling/rotating the clip carries the mask
along; keyframing the mask's own parameters moves it relative to the picture.

**Edge cases:**
- Size 0: nothing visible (or everything visible if inverted).
- Shape partly outside the clip frame: coverage outside the frame is
  irrelevant; the mask never extends the clip.
- Changing shape keeps center, rotation, width/height and feather; shape-only
  parameters (roundness, band width) are dropped with their keyframes (undo
  restores them).
- One mask per clip in v1; multiple masks are TR-020.
- Text clips: masks allowed (applied to the rendered text layer).
- Masked clip on the main track shows the canvas background in the hidden
  area.

**Undo:** `SetMask { clip, old: Option<Mask>, new: Option<Mask> }`; parameter
edits use `SetMaskParam` or keyframe ops.

**Model notes:** `Mask { shape: MaskShape, center: Animatable<Vec2>, size:
Animatable<Vec2>, rotation_deg: Animatable<f64>, feather: Animatable<f64>,
invert: bool }`, `MaskShape { Linear, Band { width: Animatable<f64> },
Ellipse, Rectangle { roundness: Animatable<f64> }, Heart, Star }` in
`freecut-timeline`; SDF evaluation in WGSL (`freecut-render`) with a CPU
reference implementation for tests. *Build note (2026-10-13):* color masks
(CL-031) need this type first, so slice K6 of color-and-filters.md's Adjustments
round 1 builds the core in `freecut_timeline::model::mask` and
`freecut_render::shape::mask` (Linear, Band, Ellipse, Rectangle, feather, invert,
combine modes, on-canvas handles), with fixed-point fields keyed through
`ClipKeys` like every other property; clip masks extend it, Heart and Star included.

### TR-018 Mask feather, invert and shape parameters — v1

**What the user sees:** Under the shape buttons: Feather slider, Invert toggle
and the shape's extra sliders (roundness, band width), each with a keyframe
diamond where animatable.

**Inputs / controls:** Feather 0–100, default 0; maps to an edge softness
distance of 0–25 % of the clip's shorter side, centered on the edge (half
inside, half outside) (FreeCut default). Invert: off by default.

**Behavior / output:** Feather smooths the coverage transition with a
smoothstep across the softness distance. Invert flips coverage (1 − c) after
feathering.

**Edge cases:**
- Feather larger than the shape: the shape fades to a soft blob whose peak
  coverage is below 1 — allowed, matches what users expect from a "spotlight"
  look.
- Animatable: center, size, rotation, feather, roundness, band width.
  Not animatable: shape, invert.
- Shape *None*: feather, invert and the shape sliders are disabled (there is
  no edge to soften or flip). Picking *None* removes the mask
  (`SetMask { new: None }`); undo brings it back with its invert flag.
- No outline, stroke or shadow along the mask edge in this area (see Out of
  scope).

**Undo:** `SetMaskParam { clip, param, old, new }` or keyframe ops.

**Model notes:** see TR-017.

### TR-019 On-canvas mask editing — v1

**What the user sees:** While the Mask section is focused, the player hides
the clip's transform box and shows the mask outline instead, with handles:
center (drag), size (corner/edge), rotation (round handle), feather (a small
arrow handle pulled away from the edge), roundness (corner dot on rectangles),
and band-width handles on the band shape.

**Inputs / controls:** Same modifiers as TR-007 (Shift constrain / 15° steps;
arrow nudge).

**Behavior / output:** Handles are drawn in the clip's transformed space, so
they line up with the picture even when the clip is scaled, rotated or
flipped. Clicking outside the clip leaves mask editing and returns to
transform handles.

**Edge cases:** Mask outline drawn even where the clip is off-canvas (if the
preview shows that area). Animated mask parameters get keyframes at the
playhead (auto-key).

**Undo:** Coalesced `SetMaskParam` per drag.

**Model notes:** `freecut-app` overlay; inverse of the clip transform is
computed by the pure transform evaluator in `freecut-timeline`.

### TR-020 Multiple masks per clip with combine modes — later

**What the user sees:** "Add mask" creates a mask list; each mask has a mode
(Add, Subtract, Intersect) and its own parameters.

**Inputs / controls:** Up to 8 masks per clip (FreeCut default); reorder by
drag.

**Behavior / output:** Masks combine in list order: Add = max, Subtract =
a·(1−b), Intersect = min.

**Edge cases:** Project files from v1 (single mask) migrate to a list of one
(`freecut-project` migration).

**Undo:** `AddMask`, `RemoveMask`, `ReorderMask`, `SetMaskMode`.

**Model notes:** `Vec<Mask>` replaces `Option<Mask>` in a versioned migration.

### TR-021 Freehand pen mask (Bezier path) — later

**What the user sees:** A pen tool that draws a closed path of points with
Bezier handles on the clip; points and handles animatable.

**Inputs / controls:** Click to add points, drag to create handles, close
path; feather and invert as in TR-018.

**Behavior / output:** Path rasterized to coverage on the GPU.

**Edge cases:**
- Keyframing a path requires the same point count at every keyframe; adding a
  point adds it to all keyframes, deleting one removes it from all (one undo
  step).
- Open or degenerate path (fewer than 3 points, or all points on a line):
  encloses no area, so nothing is visible (everything if inverted).
- Self-intersecting path: filled with the non-zero winding rule (FreeCut
  default), so loops do not punch holes.

**Undo:** Path edit ops.

**Model notes:** `MaskShape::Path { points: Animatable<Vec<PathPoint>> }`.
Subject/AI masks are owned by `ai-tools.md`.

### TR-022 Chroma key with color picker — v1

**What the user sees:** A *Chroma key* section inside the inspector's *Keying
& cutout* group (the group also hosts the automatic cutout from
`ai-tools.md`). The user arms an eyedropper, clicks a backdrop color in the
player (typically green or blue, but any color works), and pixels close to
that color turn transparent as governed by the sliders below.

**Inputs / controls:**
- Key color: eyedropper or color field (hex). Unset by default; the key does
  nothing until a color is chosen. Once armed, the eyedropper shows a small
  magnifier ring that can be dragged around the frame; the key previews live
  under the ring while dragging and the color is committed on release.
- Tolerance (how much of the color range is removed): 0–100; set to 30 when a
  color is first picked (FreeCut default), 0 before.
- Softness (edge feather): 0–100, default 10.
- Until a key color exists, every slider in the section is disabled and the
  section shows the hint "Pick a backdrop color first".

**Behavior / output:** For each pixel the shader measures the distance between
the pixel's chroma and the key color's chroma (BT.709 Y′CbCr derived from the
clip's RGB, so brightness differences in the screen matter less than hue
differences). Distances below the tolerance become transparent, distances
above tolerance + softness stay opaque, with a smooth ramp in between. Keyed
areas reveal lower tracks and, on the main track, the canvas background.

**Edge cases:**
- Eyedropper samples the clip's own un-keyed frame (after crop, before mask)
  at the clicked point, averaged over a 5×5 pixel area — not the final
  composite. Clicking outside the clip is ignored with a hint.
- The sample requires the decoded frame at the playhead; the pick completes
  asynchronously when the frame is available (no decode on the UI thread).
- Picked color with very low saturation (gray/white/black screens): warning
  that a chroma key will also remove neutral parts of the subject; suggest
  luma key (TR-025).
- Unevenly lit screen: tolerance/softness tuning; no automatic per-region
  handling in v1.
- Clip already transparent (PNG with alpha): key multiplies the existing
  alpha.
- Clip disabled (TL-036) or on a hidden track: the eyedropper is unavailable,
  with the hint "Show the clip to pick from it"; the hex field still works.
- The eyedropper reads only FreeCut's own decoded frame, so it never needs an
  OS screen-capture permission (macOS asks for one when an app samples
  outside its window); colors from elsewhere are typed as hex.
- Automatic cutout (AI-014, later) on the same clip: both mattes multiply at
  FX-001 stage L4, so turning one on never cancels the other. Alternative
  seen in the reference app: enabling a keying-type removal cancels a custom
  cutout after a confirmation.
- Painting areas in or out by hand: AI-015's keep/remove brushes (later)
  apply to keyed clips as well; until they ship, a shape mask (TR-017) or pen
  mask (TR-021, later) serves as a garbage matte.
- Not keyframable in v1 (later).

**Undo:** `SetChromaKey { clip, old: Option<ChromaKey>, new }`; slider drags
coalesced; an eyedropper drag is one entry on release.

**Model notes:** `ChromaKey { color: Rgb8, tolerance: u8, softness: u8,
spill: u8, shrink: u8, keep_shadows: u8 }` in `freecut-timeline`
(`keep_shadows` serializes with default 0, so older files load unchanged);
WGSL shader in `freecut-render`; CPU reference function for tests.

### TR-023 Spill removal and edge shrink — v1

**What the user sees:** Three more sliders in the Chroma key section: *Spill
removal*, *Edge shrink* and *Keep shadows*.

**Inputs / controls:** Spill removal 0–100, default 50. Edge shrink 0–100,
default 0 (maps to 0–4 px at canvas resolution). Keep shadows 0–100, default 0
(FreeCut default).

**Behavior / output:** Spill removal desaturates the key color's hue out of
the remaining (opaque and semi-transparent) pixels, removing green fringes on
hair and reflections. Edge shrink erodes the matte inward to cut off a
remaining halo. Keep shadows turns darkening of the backdrop (a shadow the
subject casts on the screen) into a semi-transparent black layer instead of
removing it: inside the keyed area, alpha = keep × clamp(1 − Y / Y_key, 0, 1)
with Y the pixel's luma and Y_key the key color's luma, so the shadow lands
on whatever shows through.

**Edge cases:** A subject wearing the key color loses color in those areas at
high spill values — expected; the slider can be lowered. Edge shrink on very
thin details (hair) may remove them; default 0. Keep shadows on an unevenly
lit screen also keeps the dark corners of the screen; lower it or mask them
out (TR-017). All three sliders stay disabled until a key color is picked
(TR-022).

**Undo:** Part of `SetChromaKey`.

**Model notes:** Fields of `ChromaKey`.

### TR-024 Matte preview for keys and masks — v1

**What the user sees:** A "Show matte" toggle in the Chroma key and Mask
sections. While on, the preview shows the selected clip's alpha as grayscale
(white = visible, black = removed) so holes and fringes are easy to see.

**Inputs / controls:** Toggle; turns itself off when the selection changes.

**Behavior / output:** Affects the player only; never exported and not stored
in the project.

**Edge cases:** With several keyed clips stacked, only the selected clip's
matte is shown.

**Undo:** n/a (view state).

**Model notes:** A preview-only render flag passed from `freecut-app` to
`freecut-render`.

### TR-025 Luma key — later

**What the user sees:** A key that removes pixels by brightness (dark or
bright), for black/white backgrounds and overlays like smoke or light leaks.

**Inputs / controls:** Mode (key out darks / key out brights), threshold 0–100,
softness 0–100.

**Behavior / output:** Coverage from luma (BT.709) with a smooth ramp.

**Edge cases:**
- Combined with chroma key: both multiply alpha.
- Threshold 0 (key out darks) or 100 (key out brights): nothing is removed.
- Sources with alpha: the key multiplies the existing alpha.
- Limited-range video: luma is measured after FX-001's range expansion (L2),
  so pure black really is 0.

**Undo:** `SetLumaKey`.

**Model notes:** `LumaKey` struct next to `ChromaKey`.

### TR-026 Video stabilization — v1

**What the user sees:** A *Stabilize* toggle with a strength dropdown:
*Light*, *Balanced* (default), *Strong*. After enabling, a progress bar on the
clip and in the inspector shows analysis; when done, the preview plays steadier
and slightly zoomed in.

**Inputs / controls:** Strength (FreeCut names and defaults; stronger =
steadier but more zoom):
- Light: smoothing window 0.3 s, maximum auto-zoom 5 %.
- Balanced: smoothing window 1.0 s, maximum auto-zoom 12 %.
- Strong: smoothing window 2.0 s, maximum auto-zoom 25 %.

**Behavior / output:**
1. Enabling queues a background job (`freecut-media`) that decodes the clip's
   source range plus 0.5 s on each side at reduced resolution and estimates
   per-frame camera motion (translation, rotation, scale).
2. The motion track is smoothed over the level's window; the correction for
   each frame is "smoothed − measured".
3. A single static zoom for the whole clip, no larger than the level's cap,
   hides the moving borders. Where a correction would expose borders beyond
   the cap, the correction is clamped (slightly less stable, never black
   edges).
4. The correction is applied by the compositor on the source frame, before
   crop and everything else, so speed, reverse, freeze-frame stills, masks
   and keys all see the stabilized picture.

The analysis result is deterministic, regenerable data keyed by media content
id + source range (the measured motion does not depend on the strength, so
changing strength never re-analyzes), stored in the media cache (PM-074,
`project-and-media.md`), not in the project file and not in the
generated-media store (PM-041); clearing or evicting it only means
re-analysis.

**Job states.** The analysis is one row in the single background-task list
(PM-073, shown by UI-040) and a badge on the timeline clip: *Queued* →
*Analyzing n %* → done (badge disappears), or *Paused* / *Failed*. The
inspector shows the same state under the toggle.
- *Cancel* (task list or inspector) stops the job; the clip keeps its
  *Stabilize* setting but plays unstabilized with a *Paused* badge and a
  *Resume* button. The cancel toast also offers *Turn stabilization off*
  (an ordinary undoable `SetStabilize`). Alternative seen in the reference
  app: a cancelled job leaves the clip unstabilized with the setting gone.
- *Failed* (decode error): error badge with the reason and *Retry*.
- Changing the strength while or after analyzing applies at once from the
  same motion data (no re-queue); a status line names the new level.
  Alternative seen in the reference app: a level change queues the job again.
- Editing the clip is never blocked while analysis runs; trims and splits
  during analysis are handled by the incremental rule below.

**Edge cases:**
- Preview before analysis finishes, or after the cache entry was evicted:
  unstabilized, with a badge, while the analysis (re)runs. Export started
  before the analysis is available waits for it to be regenerated (with
  progress) rather than exporting unstable footage.
- Trimming outward beyond the analyzed range: incremental analysis of the new
  part; inward trims reuse the cache.
- Split: both halves reuse the same analysis; their corrections are identical
  to the unsplit clip's at every frame (split never changes output).
- Still images, text, freeze-frame stills (TL-031): control disabled. A still
  frozen from a stabilized clip keeps that frame's correction (TR-044).
- Clips shorter than 0.5 s: allowed, little effect.
- Variable-frame-rate media: motion keyed by source PTS, not frame index.
- Missing media or decode failure: the setting stays, an error badge explains
  why there is no effect; export warns.
- Clip media replaced (TL-035): the new content id has no analysis, so a new
  job is queued automatically; the old entry stays in the cache until
  evicted, so undoing the replace needs no re-analysis.
- Export while a job is *Paused* or *Failed*: export preflight (EX-036) lists
  the clip and offers *Analyze and wait* (default) or *Export this clip
  unstabilized*.
- Nothing turns stabilization on by itself: tracking, auto reframe and
  subject follow (AI-008, AI-016, AI-017) work with it on or off, because
  their analyses map through the correction (AI-007). Alternative seen in the
  reference app: some tracking features switch stabilization on for the
  user, and stabilization and automatic reframing exclude each other.
- Heavy rolling-shutter wobble: not corrected (out of scope).

**Undo:** `SetStabilize { clip, old: Option<StabLevel>, new }`. The cached
analysis (PM-074) is not part of undo.

**Model notes:** `stabilize: Option<StabLevel>` in `freecut-timeline`;
analysis job and motion data in `freecut-media`; correction matrix applied in
`freecut-render`. The smoothing and zoom computation is a pure function over
the motion track (no FFmpeg calls), kept in its own module inside
`freecut-media` and unit-tested on synthetic motion.

### TR-027 Keyframe model and animatable properties — v1

**What the user sees:** Every animatable control in the inspector has a small
diamond button; properties with keyframes show the value at the playhead and
the diamond highlights when the playhead sits on a keyframe.

**Inputs / controls:** Animatable properties owned by this area: Position (as
one 2D property), Scale (uniform, or X and Y when unlinked), Rotation, Anchor,
Opacity, and mask center/size/rotation/feather/roundness/band width. Other
areas register their own animatable properties on the same engine: color and
adjustment values (`color-and-filters.md`), volume and pan
(`audio.md`), effect parameters (`effects-transitions-animations.md`), text
properties (`text-and-captions.md`).

**Behavior / output:**
- A property with no keyframes has one static value.
- With keyframes, the value at a time is: the first keyframe's value before
  it, the last keyframe's value after it, and between two keyframes the
  interpolation chosen on the earlier keyframe (TR-031/TR-032).
- Adding the first keyframe captures the current static value; removing the
  last keyframe turns its value into the static value (no visual jump at that
  moment).
- **Anchoring (FreeCut decision):** keyframe times are stored in the clip's
  *source time*, so the animation sticks to the footage. Consequences: trimming
  the head does not shift animations relative to the picture; splitting a clip
  leaves the output unchanged because both halves keep *all* keyframes (no
  boundary keyframe is inserted at the cut — an eased, elastic or Bezier
  segment cannot be cut into two segments exactly, so inserting one would
  change the motion); slipping moves animations with the content; changing
  speed stretches/compresses animations together with the footage; reversing
  mirrors keyframes inside the current source range so the animation keeps
  its place on the timeline (TR-043). For clips without media time (images,
  text, shapes, freeze-frame stills) the "source time" is the clip's content
  time: clip-local time plus the clip's source in-point, which is 0 for a new
  clip and which a split advances on the right half (TL-023 gives both halves
  adjacent source ranges), so the same rule holds for them. The split and
  trim ops in `timeline-editing.md` (TL-020, TL-023) call this
  area's keyframe helpers to apply this rule.
- **One split rule for every keyframe track (canonical).** The split rule
  above is the only one in FreeCut and covers every `Animatable` on the clip,
  whichever area registered it: this area's transform, opacity and mask
  values, effect parameters (FX-010), volume and pan (AU-008), color and
  adjustment values (CL) and text properties (TX). TL-023 (split), FX-010 and
  AU-008 defer to it: on a split, both halves keep all keys of every track,
  no interpolated or boundary keyframe is inserted on either side, and the
  rendered picture and sound are unchanged.
- Keyframes outside the trimmed range are kept and still shape the curve at
  the clip edge; the timeline shows a small edge marker when hidden keyframes
  exist.

**Edge cases:**
- Two keyframes on the same property at the same time: impossible; inserting
  one there replaces the value.
- Value types: each owning area chooses its own non-time value type (see the
  non-time values rule in the shared conventions) and implements
  `Interpolate` for it. This area's `f64` scalars and 2D vectors interpolate
  numerically; rotation interpolates numerically (0° → 720° spins twice).
  Other areas' types bring their own interpolation, defined in their own
  specs (CL's and TX's fixed-point scalars, AU's `GainDb`, CL's colors, which
  interpolate in linear-light RGB). The engine only requires that evaluation
  is pure and that any rounding to a type's step happens when a value is
  evaluated, never when it is stored. Booleans and enums are not animatable
  in v1.
- A split with keys of eased, elastic or Bezier segments that straddle the
  cut: both halves keep the whole segment and evaluate it exactly as before;
  each half simply shows only its part of it.
- Extremely dense keyframes (every frame) are allowed; no hard limit, UI
  thins drawing of markers.
- Keyframes on a clip whose media is missing remain and still evaluate.
- **Edits that delete keyframes as a side effect** (section reset and *Fit to
  canvas*, TR-001/TR-002; mask shape change, TR-017; auto reframe replacing
  position/scale keys, AI-016) never do it silently: a toast names the
  property and how many keys were removed and carries an *Undo* button
  (UI-040). Where the loss replaces work the user cannot easily see
  (re-linking scale or *Restore proportions* with differing keys, TR-004;
  AI-016 over user keys), a confirmation comes first. Alternative seen in the
  reference app: every such operation asks for confirmation before deleting
  keyframes.
- Effects and clip animations never conflict with keyframed properties:
  animations compose on top of the keyframed transform (FX-018/FX-019) and
  effect parameters have their own tracks (FX-014). Alternative seen in the
  reference app: adding a conflicting effect drops the clip's keyframe
  animation with a warning.

**Undo:** All keyframe changes are explicit ops (TR-028..TR-033).

**Model notes:** `freecut-timeline`: `Animatable<T> { static_value: T,
keys: Option<KeyframeTrack<T>> }`, `KeyframeTrack<T> { keys: Vec<Keyframe<T>> }`
sorted by time, `Keyframe<T> { time: RationalTime /* source time */, value: T,
interp: Interpolation }`, generic over any `T: Interpolate` (this area's `f64`
and `Vec2`, CL's and TX's fixed-point scalars, AU's `GainDb`, CL's colors;
each type is defined by its owning spec). `Animatable<T>` implements TL-023's
`SplitAt` trait once, for every `T`: it clones the whole keyframe track into
both halves and never adds keys, so other areas get the rule without writing
their own split code; a property test checks that a split never changes any
evaluated value on either side of the cut.
`fn evaluate(&self, s: RationalTime) -> T` is pure and
unit-tested; the progress fraction between two keyframes is computed exactly
as a rational and converted to `f64` only to feed the easing function and the
value interpolation. `freecut-project` serializes keys as a sorted array.

*FreeCut decision (v1 subset, `freecut-timeline/src/model/animate.rs`):* the first
`Animatable<T>` serves AU-008's volume. It holds the static value and a shared, sorted key
list (`Arc<[Keyframe<T>]>`, so clip clones and undo snapshots stay cheap) behind accessors;
`Animatable::from_parts` refuses keys whose times don't strictly increase. `Keyframe<T> { at:
Tick /* source time */, value, interp: Interp { Linear, Hold } }`; easing and Bézier come with
TR-031/TR-032. Progress between keys is an exact rational (`Progress { num, den }`), and
`Interpolate::interpolate(a, b, p)` rounds only when a value is evaluated; `lerp_i64` rounds to
nearest with halves to even. Helpers return new values: `with_key` (add or set, keeping the
interpolation of a key already there), `with_interp`, `with_key_moved` (clamped strictly between
the neighbours), `without_keys` (the earliest removed key's value becomes the static value when
none is left), `cleared` (the first key's value) and `with_static`. `value_at(s)` and
`segment_at(s)` evaluate. Keys are unique by exact tick for now; the architecture's ±1-tick snap
and 3-tick spacing arrive with TR-028's UI. Because both pieces of a split clone the clip, both
keep every key, and trims never touch keys.

*FreeCut decision (v1 app, volume only):* the first inspector diamond is AU-008's, next to
Volume: hollow off a key, filled on one, disabled with the reason while the playhead is off the
clip; ⏴ / ⏵ move the playhead to the previous / next key inside the clip's trimmed range. An
animated Volume shows the value at the playhead and editing it sets or creates the key there.
The transform properties keep their static controls until TR-028.

*FreeCut decision (S2 model, 2026-10-13):* transform position, scale, rotation and opacity and
TX-021's text properties take keys. A property's `Animatable` is its static value, kept where it
always lived (`Clip.transform`, the text style and layout), plus its keys in the clip's
`ClipKeys`, under a `PropPath` with a stable string (`transform.position`, `text.outline.width`,
`text.fill.stops.N.color`, `text.reveal`); `Clip::animatable(prop)` joins them without copying,
so clips without keys carry nothing extra and every reader of static values is unchanged. Key
values are one `PropValue` enum whose variant matches the property's `ValueKind`. Images, text
and generated items get the content time origin of this section in `ClipKeys::origin`: 0 for a
new clip, advanced on the right piece of a split and moved with the in-point by a head trim. Key
edits are one request, `EditKeys { seq, clips, prop, edit: KeyEdit }`, whose times are sequence
times (the playhead) converted per clip, so a multi-selection works as TR-028 says; volume keys
keep AU-008's `EditGainKeys` in source time and its history label. Evaluated transforms are
clamped into TR-001's ranges at evaluation (TR-031), never in stored keys.

### TR-028 Add, update and remove keyframes from the inspector — v1

**What the user sees:** Clicking a property's diamond adds a keyframe at the
playhead with the current value; clicking a highlighted diamond removes the
keyframe there. Changing a value of an already-animated property at a time
without a keyframe creates one automatically.

**Inputs / controls:**
- Diamond states: empty outline (no keyframes), outline with dot (animated,
  none at playhead), filled (keyframe at playhead).
- "Add keyframe for all transform properties" shortcut (binding owned by
  `workspace-and-shortcuts.md`).
- Preference "Auto-key animated properties" (default on).
- Clip context menu: "Remove all keyframes" (per section or whole clip).

**Behavior / output:** Keyframe time = the playhead's timeline frame converted
to source time through the clip's time map.

**Edge cases:**
- Playhead outside the selected clip: diamonds disabled; editing an animated
  property is blocked with the hint "Move the playhead over the clip";
  non-animated properties stay editable.
- Strong slow motion (e.g. 0.1×, where ten timeline frames show the same
  source frame): each timeline frame still maps to a distinct exact source
  time, so each can hold its own keyframe; if a keyframe already exists at
  the mapped time, its value is updated.
- Freeze-frame stills (TL-031) have no time map; keyframes use the clip's
  content time (TR-027), as on images.
- Locked track: diamonds read-only.
- Multi-selection: diamond adds keyframes to the same property on all
  selected clips at the playhead (clips not under the playhead are skipped).
- *Add keyframe for all transform properties* when every one of them already
  has a key at the playhead: nothing changes and the status bar says
  "Keyframes already at playhead" (no undo entry). Where only some exist, the
  missing ones are added. The per-property diamond and its toggle shortcut
  remove an existing key instead.
- Keyframe shortcuts follow UI-032's binding rules (bare keys allowed under
  the field-first rule). Alternative seen in the reference app: the
  add-keyframe binding must be non-empty and include a modifier.

**Undo:** `AddKeyframe { clip, prop, key }`, `RemoveKeyframe { clip, prop,
key }`, `SetKeyframeValue { clip, prop, time, old, new }`; first-keyframe and
last-keyframe cases also store the static value they replace.

**Model notes:** ops in `freecut-timeline`, each with a reversal test.

*FreeCut decision (S2 app, 2026-10-13):* every inspector row whose edit sets a keyable
property shows the navigator; the row finds its property from the edit it makes, so no row
lists it by hand. The navigator shows with one clip selected; with several the gutters are
empty (ui-concept §6.4), and the card commands below act on every selected clip. Auto-key is
always on in this build (the preference arrives with Preferences ▸ Editing): changing an
animated property sets the key at the playhead, adding one there when needed, and the static
value stays; on a multi-selection, clips with keys on the property get the key and the others
their static value, in one entry. With the playhead off the clip the diamond is disabled with
the hint "Move the playhead onto this clip to add a keyframe" (the arrows still work), and an
edit of an animated property is refused with "None of the selected clips is under the
playhead". Every text card and the Placement card have, in their `...` menu, **Key this
card**, **Remove this card's keys here**, **Previous key in card** and **Next key in card**,
and a collapsed card with keys shows a diamond in its summary chip. The Motion lens's
**Keyframes** card lists every keyed property with its navigator and, under it, each key with
its time, value, easing (TR-031) and a remove button. Keys outside the clip's trimmed range
are kept but not reachable from the navigator.

### TR-029 Keyframes on the timeline: view, move, select, delete — v1

**What the user sees:** A selected clip shows small diamonds along its bottom
edge, one per distinct keyframe time across its properties. A disclosure
arrow expands per-property keyframe lanes under the clip (v1).

**Inputs / controls:** Click selects; Shift-click adds to selection;
drag-rectangle selects several; drag moves selected keyframes (snaps to
timeline frames and to the playhead); Delete removes; right-click: Delete,
Copy, Paste, Interpolation ▶ (TR-031), Custom curve… (TR-032), Edit values…,
Select all on this property.
- *Edit values…* opens a small popover with the numeric fields of every
  property keyed at that time (one row per property for an aggregated
  diamond), so a keyframe's value can be changed without moving the
  playhead. Works for keys outside the trimmed range too.
- View toggles (per-user view state, not undoable): *Show keyframes on
  clips* (default on) in the timeline's view menu hides or shows the
  diamonds on every clip; the disclosure arrow expands or collapses one
  clip's lanes.

**Behavior / output:** Moving an aggregated diamond moves every property's
keyframe at that time. Moving keeps the order constraint per property: a
keyframe cannot jump over its neighbour on the same property; dropping onto an
existing keyframe time replaces that keyframe (single undo).

**Edge cases:**
- Dragging beyond the clip edges: allowed up to the clip's source range
  limits; keyframes outside the visible trim remain (TR-027).
- Dragging on a reversed or speed-curved clip: the visual drag is in timeline
  time and converted to source time through the time map (TR-036).
- Clip shorter than 2 frames: markers overlap; lane view is the way to edit.
- *Edit values…* is disabled on a locked track (with the reason as a
  tooltip). On a clip bound to a motion track (AI-008) the popover edits the
  user's own keyframes, which add on top of the tracked path; it never edits
  the tracking samples.

**Undo:** `MoveKeyframes { clip, moves: Vec<(prop, old_time, new_time)>,
replaced: Vec<...> }`, `RemoveKeyframes { .. }`; *Edit values…* uses
`SetKeyframeValue` (TR-028), one entry per popover field commit.

**Model notes:** Timeline drawing in `freecut-app`; ops in `freecut-timeline`.

### TR-030 Keyframe navigation — v1

**What the user sees:** Small ◀ ▶ arrows on both sides of each property's
diamond, and global "previous/next keyframe" commands.

**Inputs / controls:** Per-property arrows jump to that property's previous
/ next keyframe; global commands jump across all properties of the selected
clip (shortcuts owned by `workspace-and-shortcuts.md`).

**Behavior / output:** Moves the playhead to the timeline frame where the
keyframe's source time is shown. Arrows are disabled when there is nothing in
that direction.

**Edge cases:** Keyframes outside the trimmed range are skipped. On a reversed
clip "next" still means later on the timeline. In slow motion, where several
timeline frames show the same source frame, it jumps to the timeline frame
whose interval contains the keyframe's exact source time (found by binary
search over frames with the forward map, so no inverse of a speed curve is
needed).

**Undo:** n/a (view state — playhead moves are not undoable, per
`timeline-editing.md`).

**Model notes:** Pure helper in `freecut-timeline`: `next_key_time(clip, t)`
in timeline time via the time map.

### TR-031 Interpolation and easing presets — v1

**What the user sees:** Right-clicking a keyframe (or using the *Easing* menu
in the inspector when the playhead is on a keyframe) offers a list of easing
presets with tiny curve thumbnails. The menu's first level shows the five
everyday choices — Linear, Ease in, Ease out, Ease in-out (medium strength)
and Hold — followed by *More easing ▸* with every strength and the
overshoot, elastic and bounce families, and *Custom curve…* (TR-032). A
one-line hint under the thumbnails explains that easing changes how fast
the value moves between two keyframes, not where it ends.

**Inputs / controls (applied to the segment that starts at the selected
keyframe):**
- Linear (default for new keyframes).
- Hold (value stays until the next keyframe, then jumps).
- Ease in (slow start), Ease out (slow end), Ease in-out, each in four
  strengths: gentle (sine), medium (quadratic), strong (cubic), very strong
  (exponential).
- Overshoot out (passes the target slightly and settles back), Overshoot in.
- Elastic out (springy wobble into place).
- Bounce out (bounces against the target value).
- Custom (opens TR-032).

**Behavior / output:** The easing function maps segment progress p ∈ [0,1]
to eased progress e(p); value = a + (b − a)·e(p). Overshoot, elastic and
bounce may produce values outside [a, b] by design.

**Edge cases:**
- Overshooting values on clamped properties (opacity below 0 or above 1,
  feather below 0, scale below 1 %) are clamped at evaluation, not in the
  stored easing.
- Easing on the last keyframe has no effect (no outgoing segment) but is
  stored, so adding a later keyframe uses it; the menu says "No following
  keyframe yet" next to the choices. Alternative seen in the reference app:
  a curve can only be applied once a segment or at least two keyframes are
  selected.
- Multi-selection: the chosen preset is applied to all selected keyframes in
  one step.
- Position uses straight-line paths in v1; easing changes speed along the
  line, not its shape (curved paths are TR-035).

**Undo:** `SetInterpolation { clip, keys: Vec<(prop, time, old, new)> }`.

**Model notes:** `enum Interpolation { Linear, Hold, Ease { kind: EaseKind,
dir: EaseDir }, Bezier { x1, y1, x2, y2 } }` in `freecut-timeline`; the easing
functions are written from their mathematical definitions and covered by
tests (endpoints e(0)=0, e(1)=1, monotonic where expected).

*FreeCut decision (S2 model, 2026-10-13):* `Interp` gains `Ease { curve: Sine | Quad | Cubic |
Expo, dir: In | Out | InOut }` (gentle, medium, strong, very strong), `OvershootOut`,
`OvershootIn`, `ElasticOut` and `BounceOut`; `Interp::PRESETS` lists the menu. Progress between
keys stays an exact rational and becomes an `f64` only to feed the easing; the eased fraction
goes back to a fixed 2^24 denominator (`Progress::from_eased`), may leave [0, 1], and values
extrapolate and saturate at their type's width before the evaluation-time clamps. Bezier
(TR-032) comes later.

### TR-032 Custom easing curve editor — v1

**What the user sees:** A small graph panel (docked in the inspector, or a
floating window) showing the selected segment as a curve from bottom-left
(start value) to top-right (end value), with two draggable handles — the
usual cubic-Bezier timing-curve editor. Preset thumbnails sit beside it.

**Inputs / controls:** Handle 1 (x1, y1), handle 2 (x2, y2). x clamped to
[0, 1]; y allowed in [−1, 2] for overshoot. Numeric fields for all four.
Buttons: Reset to linear, Copy curve, Paste curve.

**Behavior / output:** The segment uses `Interpolation::Bezier`; the graph
shows time on X and progress on Y; the steeper the curve, the faster the
change. Solving x(t) → t uses a deterministic Newton/bisection method with a
fixed iteration count so preview and export agree.

**Edge cases:**
- Choosing any preset after a custom curve replaces it.
- Opening the editor with several keyframes selected edits all of them
  together if they share a curve; otherwise it starts from linear and applies
  to all on change.
- Segments shorter than one frame: editor still opens, but the effect is
  invisible; a hint says so.
- Only a last keyframe selected (no outgoing segment): *Custom curve…* is
  disabled with the reason, because there is no segment to draw.
- The curve always has exactly two handles; there are no extra control
  points to add, so no point limit applies here (speed curves have their own
  limit, TR-040).

**Undo:** Coalesced `SetInterpolation` per drag.

**Model notes:** Graph widget in `freecut-app`; Bezier evaluation in
`freecut-timeline` (pure, float math allowed because it maps unitless
progress, not time).

### TR-033 Copy and paste keyframes — v1

**What the user sees:** Copy selected keyframes, move the playhead (same clip
or another clip), paste.

**Inputs / controls:** The standard copy/paste shortcuts while keyframes (not
clips) are selected; context menu items. Clip copy/paste and the clipboard
itself are TL's (TL-032); this feature only defines the keyframe payload.

**Behavior / output:** Pasted keyframes keep their relative spacing; the
earliest lands at the playhead. They go onto the same-named properties of the
target clip, replacing keyframes at identical times.

**Edge cases:**
- Target clip lacks a property (e.g. pasting mask keys on a clip without a
  mask): those keys are skipped and a status message lists them.
- Pasting across clips with different speeds: spacing is preserved in
  timeline time and converted through the target's time map (FreeCut
  default), so the motion looks the same on the timeline.
- Pasted keys landing past the clip end: kept (inactive until the clip is
  extended).
- Target on a locked track: paste refused with a status message.

**Undo:** `PasteKeyframes { clip, added, replaced }`.

**Model notes:** Clipboard payload is a serde struct (no OS clipboard
dependency required inside the model).

### TR-034 This area's groups in Paste attributes — v1

The *Paste attributes…* command (Ctrl+Alt+V, ⌥⌘V on macOS; menu and binding
in UI-012 and UI-031), its checklist dialog, the attribute clipboard and the
`PropBag::category()` registry are owned by TL-034 in `timeline-editing.md`;
each owning spec only registers its own categories. TL-034 and this entry are
v1 because FX-017 and CL-025 (both v1) paste through the same command, and
copying a look, an effect stack or a framing from one clip to others is
everyday editing. This entry defines this area's categories and ships
together with TL-034.

**What the user sees:** In TL-034's checklist, these groups: Transform (incl.
anchor, flips, fit mode and their keyframes), Crop (incl. straighten),
Background, Opacity & blend, Mask, Chroma/luma key, Stabilization, Speed
(incl. reverse, curve and frame sampling).

**Inputs / controls:** One checkbox per group. Defaults: all on except Speed
and Stabilization (FreeCut defaults, because Speed changes durations and
Stabilization starts a background analysis).

**Behavior / output:** Each checked group replaces the target's value for that
group. Keyframes in the Transform, Opacity and Mask groups are placed using
TL-034's timing option (rescale or absolute offsets), measured in clip-local
timeline time and converted to source time through the target's time map
(TR-036), so the motion looks the same on the timeline.

**Edge cases:**
- Pasting Speed changes durations: later clips move by TL's ripple rules; the
  whole paste stays one undo step.
- A group the target cannot take (Speed onto an image or a freeze-frame still,
  Background onto a clip not on the main track, Crop onto text) is skipped and
  listed in TL-034's summary.
- Pasting Stabilization onto a clip whose analysis is missing queues the
  analysis job (TR-026) after the paste commits.
- Target already has keyframes on a property in a checked group: they are
  replaced by the source's value or keyframes, and TL-034's summary lists
  each such clip and property ("3 keyframes on Position replaced"); undo
  restores them. Alternative seen in the reference app: copying a setting
  to all clips is refused when other clips hold keyframes on it.
- Locked-track targets are skipped (TL-009).
- Settings from features that ship later (luma key TR-025, extra masks
  TR-020, pen masks TR-021, text masks TR-046, curved paths TR-035,
  perspective tilt TR-047, optical-flow or AI frame sampling) join their
  existing group when they ship; no new group is added and v1 behaviour does
  not depend on them. A beat ramp (TR-048) is pasted as the plain speed
  curve it produced (Speed group); its accent effect travels with FX-017.

**Undo:** Each group contributes its own set-op (`SetTransform`, `SetCrop`,
`SetCanvasBackground`, `SetMask`, `SetChromaKey`, `SetStabilize`,
`SetRetimeMode`, …) to TL-034's single compound op.

**Model notes:** This area registers its categories through TL-034's
`PropBag::category()` registry in `freecut-timeline`, the same registry FX-017,
CL-025 and OV-044 use. There is no `ClipAttributes` struct anywhere; the
copied values sit in TL-034's attribute clipboard as the source clip's
property bags.

### TR-035 Curved motion paths — later

**What the user sees:** Position keyframes drawn as a path in the player;
handles on each point bend the path into curves.

**Inputs / controls:** Per-keyframe spatial in/out tangents.

**Behavior / output:** Position follows a cubic spline; temporal easing (TR-031)
still controls speed along the path (arc-length parameterized).

**Edge cases:**
- Old projects (straight paths) load with zero tangents and look unchanged.
- Two consecutive keyframes at the same position with non-zero tangents
  produce a small loop; allowed.
- A Hold segment ignores tangents (the value jumps).

**Undo:** `SetSpatialTangent`.

**Model notes:** Optional tangents on `Keyframe<Vec2>`.

### TR-036 Retiming model (time map) — v1

**What the user sees:** Nothing directly; it is the shared rule behind speed,
speed curves and reverse, behind which frame a freeze-frame still shows, and
behind how keyframes, audio, transitions and the blurred background follow
the footage.

**Inputs / controls:** Per media-backed clip (video, audio, video with
audio): `retime` mode — Normal (constant speed) or Curve — plus `reverse` and
`frame_sampling`. The *Keep pitch* toggle (default on) that decides how audio
follows the map is owned by AU-023 (`audio.md`) and stored in
`ClipAudio.preserve_pitch`, not in `Retime`.

**Behavior / output:**
- `D` = source range length (out − in). `T` = clip duration on the timeline.
- **Normal:** `s(t) = in + t·r` (forward) or `s(t) = out − t·r` (reverse), and
  `T = D / r` rounded to the nearest whole timeline frame (minimum 1 frame);
  the exact rational `r` is kept, so the time map is exact even when `T` was
  rounded.
- **Curve:** speed `v(u)` is defined over normalized output progress
  `u = t/T ∈ [0,1]` as cubic Hermite segments between points `(u_i, v_i,
  slope_i)`. Source progress `P(u) = ∫₀ᵘ v / ∫₀¹ v`; `s(t) = in + D·P(t/T)`
  (mirrored when reversed); `T = D / mean(v)` rounded to frames. Because `v`
  is polynomial with rational coefficients, `P` and `T` are computable
  exactly (rational arithmetic, `i128` intermediate) — floats are used only to
  draw the graph.
- **Freeze frames** are not a retime mode: TL-031 inserts a still clip with a
  fixed source time `s₀ = source_time_at(clip, playhead)` (TR-044).
- **Video frame choice:** the source frame whose display interval contains
  `s(t)` for the timeline frame's start time (Nearest), or a blend (TR-041).
- **Audio:** follows the same map at the local rate `ds/dt` (TR-038); whether
  that is varispeed or pitch-preserving stretch is AU-023's setting.

**Edge cases:**
- Source frame rate ≠ timeline frame rate (24 fps in a 30 fps timeline at
  1×): handled by the same frame-choice rule; no special case.
- Variable-frame-rate sources: frame choice by PTS, not index.
- Trim on a retimed clip: trim handles move in timeline time; the new source
  in/out are computed through the map. For curves, trimming cuts the curve
  exactly (Hermite subdivision at the cut, remaining points re-normalized), so
  the kept part plays exactly as before. Extending past the authored range
  continues at the edge speed.
- Split on a retimed clip: both halves get the matching piece of the map;
  output is unchanged (property test: split never changes any rendered frame
  or audio sample).
- Source range would exceed the media length (e.g. after a speed decrease
  combined with a long clip on a magnetic track): impossible, because speed
  changes alter `T`, never `D`.
- Rounding `T` to whole frames in Normal mode: the start of every output frame
  still maps inside `[in, out)`, but the last frame's interval (and its audio
  samples) can reach up to half a frame × `r` past `out`. Source reads are
  clamped to `out`; audio past `out` is silence. Tested at 100× and 0.1×.
- Transitions need handle media beyond in/out; handle lengths in timeline time
  are source handles divided by the local speed; reversed clips take handles
  from the other side (transition rules are owned by
  `effects-transitions-animations.md`).
- Still images, text, freeze-frame stills and other generated clips have no
  time map; speed controls are hidden for them.

**Undo:** Retime edits are ops listed in TR-037..TR-043.

**Model notes:** `freecut-timeline`: `Retime { mode: RetimeMode, reverse:
bool, frame_sampling: FrameSampling }`, `enum RetimeMode { Normal(Rational),
Curve(SpeedCurve) }`, plus pure
functions `source_time_at(&Clip, t) -> RationalTime`,
`timeline_duration(&Clip) -> RationalTime`, `local_rate(&Clip, t) ->
Rational`. `freecut-render`, `freecut-audio` and `freecut-media` call these and
never re-implement the mapping.

### TR-037 Constant speed — v1

**What the user sees:** A *Speed* tab in the inspector with a *Normal* mode: a
speed slider, a numeric speed field, quick buttons, and a duration field that
shows (and accepts) the clip's resulting length. The timeline clip shows a
small "2.0×" badge when speed ≠ 1×.

**Inputs / controls:**
- Speed 0.1× – 100×, default 1.0×. Slider is logarithmic with ticks at 0.1,
  0.25, 0.5, 1, 2, 5, 10, 100. The field accepts up to two decimals; the value
  is stored as an exact rational (e.g. 1.37 → 137/100).
- Quick buttons: 0.1×, 0.25×, 0.5×, 1×, 1.5×, 2×, 4×, 10× (FreeCut
  defaults; the two outer buttons reach the extremes users ask for — very
  slow motion and time-lapse — without dragging the slider).
- Reset arrow (UI-009) next to the field: back to 1× (the original rate);
  shown only when speed ≠ 1×.
- Duration field (timecode): entering a target length sets `r = D / T_target`
  exactly, clamped to the speed range.
- *Speed…* command (UI-031, unbound by default; also in the clip context
  menu's *Speed ▸*): selects the clip if needed and opens the inspector on
  the Speed tab with the field focused.
- Multi-selection: the field, slider and quick buttons set the same factor
  on every selected media-backed clip; each clip's own duration is
  recomputed from its own `D`. Mixed values show "—" until one is chosen.

**Behavior / output:** The clip's timeline length changes to `D / r`, rounded
to whole frames (TR-036). The picture and its audio play faster or slower;
whether the audio keeps its pitch is AU-023's *Keep pitch* setting.
Downstream clips move according to the ripple rules of
`timeline-editing.md`; the FreeCut default is to ripple the clip's own track
(and audio detached from it, TL-030), leaving other tracks in place.

**Edge cases:**
- Speeding up a clip that would leave a gap: gap closes on a magnetic/ripple
  track, otherwise stays (per TL rules).
- Slowing down a clip on a non-ripple track where the next clip would be
  overlapped: the next clips on that track are pushed right (FreeCut default)
  rather than overwritten; the whole change is one undo step.
- Duration typed shorter than 1 frame or implying > 100×: clamped to 100× with
  a message; longer than 10 × D: clamped to 0.1×.
- Video with embedded audio: speed applies to both (one clip, TL-011).
  Detaching (TL-030) gives the audio clip an identical copy of the retime
  settings, which can then be changed separately; AU-023's *Keep pitch* flag
  moves with the rest of `ClipAudio` (AU-003).
- Keyframes stretch with the footage (TR-027 anchoring).
- Clip with a transition whose handles become too short at the new speed: the
  transition shortens to fit, never fails (rule owned by FX; this area only
  recomputes handle lengths).
- Locked track: disabled.
- Missing media: allowed (model-only change); preview shows offline frames.
- 100× on long-GOP 4K media: preview may drop frames (proxies recommended,
  `project-and-media.md`); export never drops frames.
- Speeds below 0.5×: a one-line hint under the field suggests *Frame blend*
  (TR-041) or, once it ships, *Optical flow* (TR-042) for smoother slow
  motion; nothing changes automatically.
- *Keep pitch* (AU-023) stays available at every speed; AU-023 only shows a
  quality hint at extreme ratios. Alternative seen in the reference app: at
  some multipliers pitch preservation is forced on and the toggle is locked.
- Clip with motion tracking (AI-008) or beat markers (AU-025): both stay,
  because they are stored in source time and drawn through the new map
  (shared rule "Settings combine"). Alternative seen in the reference app:
  speed cannot be changed while tracking is on, and adding beat markers
  removes an existing speed change.
- Multi-selection containing stills, text or clips on locked tracks: those
  are skipped and the status bar names how many; the rest change in one undo
  step.

**Undo:** `SetRetimeMode { clip, old, new, ripple: RippleChanges }` — restores
speed, duration and every moved clip. A multi-selection change is one
compound op with one `SetRetimeMode` per clip and the combined ripple.

**Model notes:** Speed as `Rational` (`i64` numerator/denominator, reduced);
ripple computed by the timeline's existing ripple machinery.

### TR-038 Audio follows the time map — v1

The *Keep pitch* toggle (default on), its `SetPreservePitch` op, its hints and
the time-stretch engine are owned by AU-023 in `audio.md`; the flag is stored
in `ClipAudio.preserve_pitch` and carried along when audio is detached
(AU-003). This entry defines only the time-map contract this area hands to
the audio engine.

**What the user sees:** Audio of a retimed clip stays in sync with its
picture: it speeds up, slows down, follows a speed curve and plays backwards
together with the video.

**Inputs / controls:** None here; the retime settings come from TR-037,
TR-039, TR-040 and TR-043, and *Keep pitch* from AU-023.

**Behavior / output:** For any span of output time, `freecut-audio` asks this
area's pure functions for the source time at the span start
(`source_time_at`), the local rate over the span (`local_rate`) and the
direction (`reverse`), and reads exactly the source samples the picture shows
over that span. It never re-derives the mapping from the UI speed value.

**Edge cases:**
- Speed curves: the local rate changes continuously; the audio engine
  receives it per processing block (block size chosen by AU), so audio and
  picture stay within one block of each other and never drift over the clip.
- Reverse: the same source range is read backwards; how *Keep pitch* behaves
  for reversed clips is AU-023's rule.
- Freeze-frame stills (TL-031) have no time map and no audio.
- End of a rounded clip: samples past `out` are silence (TR-036).
- Source sample rate different from the project's: the map is in rational
  time, not samples, so it is rate-independent; resampling is AU's.

**Undo:** n/a here — retime changes use the ops of TR-037..TR-043; the
*Keep pitch* toggle uses AU-023's `SetPreservePitch`.

**Model notes:** `source_time_at`, `local_rate` and `timeline_duration` in
`freecut-timeline` (TR-036) are the only interface; the *Keep pitch* flag
lives in `ClipAudio.preserve_pitch` (AU-023, default on, moved by AU-003 on
detach), not in `Retime`, and this area neither reads nor writes it. A
property test checks that the audio sample read at output time `t` equals the
source position the video uses at `t` (within one sample).

### TR-039 Speed curve presets — v1

**What the user sees:** In the Speed tab, a *Curve* mode with a grid of preset
tiles, each showing the shape of its speed curve, plus *None* and *Custom*.
Clicking a tile applies it and the clip length updates.

**Inputs / controls:** FreeCut's own presets (our names and shapes):
- *Accelerate* — starts at 0.5×, ends at 4×.
- *Decelerate* — starts at 4×, ends at 0.5×.
- *Slow center* — fast (3×), dips to 0.3× in the middle, fast again.
- *Fast center* — slow (0.5×), rises to 5× in the middle, slow again.
- *Pulse* — alternates between ~3× and ~0.5× four times.
- *Double dip* — fast with two short slow-motion dips at ⅓ and ⅔.
- *None* — back to Normal mode at 1×.
- *Beat ramp…* — a curve built from music beats (TR-048, later); shown only
  once that feature ships.

**Behavior / output:** Applying a preset replaces any constant speed or
earlier curve; the speed values are absolute. The resulting duration is shown
under the tiles. Presets are stored as plain point lists, so the custom editor
(TR-040) opens with the preset's points. With several media-backed clips
selected, clicking a tile gives each of them the same preset (each clip's
duration follows from its own `D`) in one undo step.

**Edge cases:**
- Clips shorter than 0.5 s: presets with many points (Pulse) get points closer
  than one frame; points are still kept, but the visible effect is minimal; a
  hint suggests using a longer clip.
- Resulting duration below 1 frame: impossible within the presets' 0.3×–5×
  range unless the clip is a single frame, in which case the clip stays at
  1 frame. Alternative seen in the reference app: a ramp whose result would
  be shorter than 0.1 s is refused.
- Frame sampling (TR-041) and AU-023's *Keep pitch* setting are left as they
  are.
- Reverse stays as is; the curve is applied in timeline order.
- Beat markers (AU-025) and motion tracking (AI-008) on the clip are kept
  (shared rule "Settings combine"). Alternative seen in the reference app:
  applying a speed curve deletes the clip's beat markers.
- Multi-selection with stills, text or locked-track clips: those are skipped
  and counted in a status message. Alternative seen in the reference app:
  curves apply to one clip at a time (only constant speed works on several).

**Undo:** `SetRetimeMode { old, new: Curve(..), ripple }`; one compound op for
a multi-selection.

**Model notes:** Preset point lists live as data in `freecut-timeline` (no
assets needed); thumbnails are drawn from the same data.

### TR-040 Custom speed curve editor — v1

**What the user sees:** A graph under the presets (or in a larger popup):
horizontal axis = the clip from start to end, vertical axis = speed on a log
scale from 0.1× (bottom) through 1× (middle line) to 10× (top). Points can be
dragged; a vertical line tracks the timeline playhead. The clip's new duration
is shown live ("6.0 s → 4.3 s").

**Inputs / controls:**
- Drag a point vertically = speed (0.1×–10×); horizontally = position
  (constrained between neighbours, min gap 1 % of the clip).
- "Add point" at the playhead; double-click on the curve also adds one.
- "Delete point" (Del key) for a selected point; endpoints (start, end) can
  only move vertically and cannot be deleted.
- Up to 32 points (FreeCut default).
- "Reset" = flat 1×.
- Smooth (default) or Linear segment shape toggle.
- *Show speed curve on clip* (clip context menu and the graph's menu; per-user
  view state, default off): draws the curve as a thin line over the timeline
  clip, on the same log scale, so ramps can be read while editing other
  things.

**Behavior / output:** Slopes at each point are computed automatically
(monotone cubic, so the curve never overshoots between points) and stored with
the point. While dragging, the timeline clip length and preview update live;
the change is committed on release.

**Edge cases:**
- Dragging a point so the clip would become shorter than 1 frame: not
  possible within 10× on clips ≥ 10 frames; for shorter clips the minimum
  duration of 1 frame clamps the drag. (The reference app refuses results
  shorter than 0.1 s; FreeCut keeps the 1-frame floor used everywhere in
  TR-036.)
- Adding a point closer than the 1 % minimum gap to an existing point:
  refused with the status message "Too close to the next point"; no undo
  entry.
- Adding a 33rd point: refused with "Speed curves hold up to 32 points".
- Several clips selected: the editor is read-only and says "Select one clip
  to edit its curve" (presets still apply to all, TR-039).
- Clip with embedded audio: the audio follows the same curve (TR-038).
- Keyframes follow the content (source-anchored) and visibly speed up or slow
  down with the curve.
- Locked track: graph read-only.

**Undo:** One coalesced `SetRetimeMode` (with ripple changes) per drag or
point add/delete.

**Model notes:** `SpeedCurve { points: Vec<SpeedPoint { u: Rational, v:
Rational, slope: Rational }> , shape: Smooth | Linear }` in
`freecut-timeline`; graph widget in `freecut-app`.

### TR-041 Frame sampling for retimed clips (nearest / frame blend) — v1

**What the user sees:** A "Frame sampling" dropdown in the Speed tab: *Nearest
frame* (default) and *Frame blend*. (*Optical flow* appears when TR-042
ships, *AI interpolation* when AI-018 ships; both are later.)

**Inputs / controls:** Dropdown; applies in Normal and Curve modes, hidden
for stills (images, freeze-frame stills) that have no time map. Under it, a
read-only frame readout for the clip at the playhead's local speed, e.g.
"Source 30 fps · 0.25× → 7.5 source frames per second of output (each shown
4 times)", so the user can see when slow motion will look steppy.

**Behavior / output:** Nearest repeats or skips source frames. Frame blend
averages the source frames that fall inside each output frame's interval
(speed-ups gain motion-blur-like smoothness) and cross-fades between the two
nearest frames for slow motion.

**Edge cases:**
- At exactly 1× with matching frame rates the setting has no effect.
- Frame blend is offered at every speed, because it also smooths speed-ups.
  Alternative seen in the reference app: frame smoothing is offered only
  when the clip plays below 1×. (*Optical flow*, TR-042, keeps that below-1×
  condition.)
- Readout on a speed curve: shows the value at the playhead's position in
  the clip, plus the curve's minimum.
- At 100× frame blend would average up to ~100 frames; it is capped at 16
  samples per output frame (FreeCut default) to bound decode cost.
- Project file with a frame-sampling value this build does not know (for
  example written by a newer FreeCut): the clip loads as *Frame blend*, the
  closest choice that needs no generated frames, and a one-time warning
  names the affected clips.
- A reserved value whose feature is not built into this version (*Optical
  flow* before TR-042 ships, *AI interpolation* before AI-018 ships): the
  value is kept in the model so saving does not lose it; preview renders
  frame blend, and export warns before starting and renders those clips with
  frame blend. What happens when AI-018 is built in but its model is not
  installed is AI-018's rule.

**Undo:** `SetFrameSampling { clip, old, new }` — the one op for every
variant, including the later ones.

**Model notes:** This entry owns the enum, stored in `Retime`:
`enum FrameSampling { Nearest, FrameBlend, OpticalFlow, Ai { quality:
AiInterpQuality } }`. `OpticalFlow` is TR-042's (later). `Ai { quality }` is
reserved here so the type and the file format stay stable; its behaviour and
the meaning of its quality levels belong to AI-018 (later), and no other area
adds variants of its own. `freecut-project` loads any unknown variant as
`FrameBlend`. Blending in `freecut-render`, frame requests via
`freecut-media`.

### TR-042 Optical-flow smooth slow motion — later

**What the user sees:** *Optical flow* in the Frame sampling dropdown, plus a
progress badge while in-between frames are generated.

**Inputs / controls:** Enabled only when some part of the clip plays below
1×.

**Behavior / output:** Synthesizes intermediate frames from estimated motion
between neighbouring source frames. The generated frames are deterministic
and regenerable, so they are stored in the media cache (PM-074), not in the
generated-media store (PM-041). Preview uses frame blend until the cached
frames are ready.

**Edge cases:**
- Poor results on scene cuts, fast motion with blur, overlays and
  semi-transparent text inside the footage; a tooltip says so.
- Export waits for generation, and for regeneration if the cache was cleared
  or the frames were evicted.
- Changing speed, the speed curve, trim or the project frame rate invalidates
  only the affected frames.
- Generation job: a row in the background-task list (PM-073) with
  percentage, *Cancel* and, on failure, *Retry*; cancelling keeps the
  setting and leaves preview on frame blend until the job is resumed.
- The two-tier choice users expect (fast blend vs. slower, higher-quality
  motion estimation) is TR-041's *Frame blend* plus this entry; AI-018 is a
  third, optional tier.

**Undo:** Same `SetFrameSampling` (value `OpticalFlow`). Generation is not on
the undo stack.

**Model notes:** `FrameSampling::OpticalFlow` (TR-041). Generation job in
`freecut-media` (non-ML first, e.g. FFmpeg's motion-interpolation filter).
ML-based interpolation is AI-018 in `ai-tools.md`, selected through the
reserved `FrameSampling::Ai { quality }` variant (TR-041).

### TR-043 Reverse — v1

**What the user sees:** A *Reverse* button on the timeline toolbar and in the
Speed tab. The clip plays backwards (picture and its audio). A small "◀◀"
badge marks reversed clips. Clicking again restores forward playback.

**Inputs / controls:** Toggle.

**Behavior / output:** Instant model change (`reverse = true`); no file is
written. For smooth preview, `freecut-media` builds in the background
reverse-friendly data for the clip's source range (decoded GOP chunks or an
all-intra proxy) in the media cache (PM-074); until it is ready, or after it
was evicted, preview may stutter and shows a progress badge. Export decodes
backwards GOP by GOP and never depends on the cache.
Keyframes are mirrored inside the current source range as part of the same op,
so animations keep their position on the timeline.

**Preview-cache job.** The reverse-friendly data is a row in the
background-task list (PM-073) and a badge on the clip: *Queued* → *Preparing
reverse n %* → ready, or *Failed* with *Retry*. *Cancel* stops only the cache
build: the clip stays reversed, preview keeps stuttering, and the badge
offers *Resume*. Because cancelling is a local thread stop, it cannot fail.
None of these states affects export.

**Edge cases:**
- Reverse with constant speed or a curve: speed shape stays in timeline order
  (the curve is not mirrored).
- Source media missing: the toggle still works as a model change; preview
  shows the offline slate (EX-005) and the cache job starts once the media
  is relinked. Alternative seen in the reference app: reverse is refused when
  the source file is missing.
- Audio tools on a reversed clip (voice isolation and *Split into stems*,
  AU-021; voice effects, AU-022; pitch shift, AU-024) stay available: AU's
  signal chain places each of them before, inside or after its reverse
  stage, so none depends on forward playback (rules are AU's). Alternative
  seen in the reference app: reversed clips cannot use vocal isolation, stem
  separation or some voice effects.
- Embedded audio is reversed with the video; to keep forward audio, detach the
  audio first (TL-030).
- Stabilization, crop, mask, key: unaffected (they operate on the source
  frame).
- Image, text and freeze-frame still clips: disabled.
- Very long clips (hours): allowed; preview cache builds progressively around
  the playhead first.
- Toggling twice returns to the identical original state (including keyframe
  times) — tested.

**Undo:** `SetReverse { clip, old, new, mirrored_keyframes }`.

**Model notes:** `reverse: bool` in `Retime`; keyframe mirroring is a pure
function in `freecut-timeline`; backward decode support in `freecut-media`.

### TR-044 Freeze-frame still: frozen frame and carried-over settings — v1

The *Freeze* command itself (button, default length, split, insertion,
ripple, undo step, the still clip kind) is owned by TL-031 in
`timeline-editing.md`. This entry defines the two things TL-031 takes from
this area: which source frame is frozen, and how this area's clip settings
carry over to the still.

**What the user sees:** The still shows exactly the picture that was on screen
at the playhead — same framing, crop, mask, key, background and stabilized
frame — and it stays put while animations on the original clip would have
moved on.

**Inputs / controls:** None of its own; the frozen source time is not editable
in v1 (to pick a different frame, undo and freeze again).

**Behavior / output:**
1. Frozen source time `s₀ = source_time_at(clip, playhead)` (TR-036), so
   speed, curve and reverse are honored and the still matches the frame the
   user saw. TL-031 stores it in the still's `StillSource::MediaFrame`.
2. This area's settings are copied to the still: transform (incl. anchor,
   flips, fit mode), crop and straighten, canvas background, opacity and
   blend mode, mask, chroma/luma key.
3. Every animated value among them is baked to its value at the playhead (a
   static value, no keyframes), because the still has no source time to
   anchor keyframes to; the user can add new keyframes on the still, which
   then use the still's content time (TR-027).
4. If the clip is stabilized, the still bakes the stabilization correction for
   `s₀` into its source sampling (the still itself has no *Stabilize* control,
   TR-026).
5. Retime settings are not copied (stills have no time map, TR-036).

**Edge cases:**
- Stabilization analysis not finished at freeze time: the still is created
  unstabilized and picks up the correction for `s₀` when the analysis
  completes (no extra undo entry; it is derived data).
- Playhead on a slow-motion frame between two source frames: `s₀` is the exact
  rational time; Nearest sampling shows the same source frame the user saw.
  With frame blend (TR-041) on the original clip, the still shows the
  nearest source frame instead (a still has no sampling interval to blend
  over); the difference is at most one source frame's worth of motion.
- Missing media: TL-031 still creates the still; it shows the offline
  placeholder and keeps `s₀` for when the media is relinked.
- Clip with a mask or key whose parameters were mid-animation: baked values
  reproduce the on-screen result exactly at that frame.

**Undo:** Part of TL-031's single `FreezeFrame` compound op; undo removes the
still and its copied settings together.

**Model notes:** Pure helper in `freecut-timeline`:
`freeze_settings_at(&Clip, playhead) -> (RationalTime, FrozenSettings)`
returning `s₀` and this area's settings with keyframes baked; TL-031 calls it
when building the still. No retime mode for freezes exists in this area.

### TR-045 Align and distribute on the canvas — v1

**What the user sees:** An *Arrange* row at the top of the Transform section,
repeated in the player's right-click menu: six align buttons (left edges,
horizontal centers, right edges, top edges, vertical centers, bottom edges)
and two distribute buttons (horizontally, vertically). With one item
selected, the buttons line it up with the canvas, so the two center buttons
are the everyday "center horizontally / center vertically". With two or more
items selected, an *Align to* dropdown appears.

**Inputs / controls:**
- Items: the selected visual clips that are visible at the playhead — video
  and image overlays, main-track clips, text, stickers, shapes.
- Align to: *Canvas* or *Selection*. Default: Canvas with one item, Selection
  with two or more (FreeCut default).
- Distribute: needs at least 3 items. It spaces them so the gaps between
  neighbouring bounding boxes are equal (gap spacing, FreeCut default) and
  keeps the two outermost items where they are.
- Commands for all eight actions are in the command registry (UI-030),
  unbound by default (UI-031).

**Behavior / output:**
- An item's bounds are the axis-aligned box of its transformed, cropped
  rectangle at the playhead — the same bounds OV-003 uses. Masks are
  ignored, as with TR-007's transform box.
- *Selection* aligns to the union of the selected bounds; *Canvas* to the
  canvas rectangle.
- Only Position changes. Scale, rotation, anchor and every other value stay.
  The offset between the item's current edge or center and the target line
  is added to Position (converted to the stored canvas fractions).
- Distribute orders items by their bounds centers along the axis (ties:
  upper track first), takes the span between the outer edges minus the sum
  of the item sizes, and splits it into equal gaps. If the items are wider
  than the span, the gaps become equal overlaps.
- Results are computed exactly in project pixels; canvas snapping (TR-008)
  is not applied on top.

**Edge cases:**
- Distribute with fewer than 3 items: both distribute buttons are disabled
  with the tooltip "Select 3 or more items".
- Selected items not visible at the playhead (outside their time range,
  disabled with TL-036, on a hidden track): skipped; the status bar says how
  many.
- Items on locked tracks are not selectable (TL-009); any left in the
  selection are skipped and counted.
- Position keyframed: a keyframe is written at the playhead (auto-key,
  TR-028); other keyframes stay, so the item may leave the alignment at
  other times — expected.
- Item following a motion track (AI-008): the move changes its offset from
  the track, as dragging does.
- A group (TL-039) selected as a whole counts as one item with its members'
  union bounds; every member moves by the same offset, so the group's inner
  layout is kept.
- Rotated items: the box of the rotated rectangle is used, so a square
  rotated 45° is centered by its diamond's extents.
- Text clips: the bounds are the laid-out text box (TX-005), not the ink of
  the glyphs.
- Item larger than the canvas aligned to the canvas's left edge: its left
  edge lands on the canvas edge and the rest overflows to the right.
- Nothing would move (already aligned): no undo entry.

**Undo:** One compound op per click: a `SetTransformProperty { property:
Position, .. }` per moved item, or `AddKeyframe` / `SetKeyframeValue` when
Position is animated.

**Model notes:** Pure functions in `freecut-timeline`:
`align(items: &[ItemBounds], edge: AlignEdge, target: AlignTarget, canvas)
-> Vec<(ClipId, Vec2)>` and `distribute(items: &[ItemBounds], axis) ->
Vec<(ClipId, Vec2)>`, computed from the evaluated transform at the playhead
(the same evaluator TR-007 uses). Unit tests cover every edge, both targets,
rotated items, overlap and ties. Buttons and commands in `freecut-app`.

### TR-046 Text-shaped mask — later

**What the user sees:** A *Text* button in the Mask section's shape row
(TR-017). The picture shows only inside the letters of a word the user types,
or everywhere except the letters when inverted. The letters can be moved,
sized, rotated and feathered on the canvas like any other mask (TR-019).

**Inputs / controls:**
- Text: up to 200 characters, line breaks allowed (FreeCut default). Default
  "TEXT".
- Font: any font in TX's font registry — bundled, system or imported (TX-003,
  TX-018). Default: TX's factory default, the bundled sans in Bold.
- Line alignment: left / center (default) / right.
- Letter spacing: same control, unit and range as TX-009.
- Common mask controls: center, size (the text block's width/height, linked
  by default so letters keep their proportions), rotation, feather, invert
  (TR-017, TR-018).

**Behavior / output:** The text is shaped and laid out by the engine that
draws text clips (`freecut-render::text`, with TX-006's script support and
font fallback), scaled so its layout box fills the mask size, and rasterized
to a coverage texture in clip space at the clip's output resolution. Feather
and invert then apply as for the other shapes. Coverage comes from glyph
alpha, so edges stay smooth at any size.

**Edge cases:**
- Empty text or only spaces: nothing visible (everything if inverted), like
  a size-0 shape.
- Font missing when the project opens on another machine: TX-007's fallback
  and missing-font report apply; the mask uses the fallback until the font is
  available.
- Color-emoji glyphs: their alpha is the coverage; their colors are ignored.
- Right-to-left and complex scripts: shaped correctly (TX-006).
- Large feather on a thin font: the letters fade to a soft glow; allowed.
- Switching to Text from another shape keeps center, size, rotation and
  feather (TR-017's rule).
- Unlinking the size stretches the letters; the text is not re-laid out.
- Text content, font, alignment and spacing are not keyframable; center,
  size, rotation and feather are.

**Undo:** `SetMask` when the shape is picked; `SetMaskParam { param: Text |
Font | Align | LetterSpacing, old, new }` for the text options (typing in the
field coalesces until the field loses focus).

**Model notes:** New variant `MaskShape::Text { content: String, font:
FontRef, align: HAlign, letter_spacing }`, using TX's `FontRef`, `HAlign`
and spacing type. Coverage rendering in `freecut-render` reuses TX's shaping
and glyph cache and is cached per (content, font, size bucket). Older
FreeCut builds do not know the variant; `freecut-project` handles it under
PM-021's versioning rules. Tests compare the GPU coverage with a CPU raster
at a few sizes.

### TR-047 Perspective tilt (plane rotation) — later

**What the user sees:** Under Rotation in the Transform section, *Tilt X*
(the top edge leans back or forward), *Tilt Y* (a side edge turns away) and a
*Perspective* slider. The clip turns like a card in space, its far edge
drawn smaller.

**Inputs / controls:**
- Tilt X, Tilt Y: −80° to +80°, default 0° (FreeCut default; the limit keeps
  the picture from going edge-on or showing its back).
- Perspective: 0–100, default 50 (FreeCut default), mapped to a camera field
  of view from 10° (0, almost flat) to 90° (100, strong).
- All three are keyframable (TR-027) and reset with the Transform section.
- No dedicated on-canvas handles in this tier; the transform box (TR-007)
  follows the projected outline.

**Behavior / output:** Inside FX-001 stage L8 the order becomes: flip,
scale, tilt (about X, then Y, through the anchor), in-plane rotation,
projection with a perspective camera centered on the anchor, translation.
The clip is drawn as a projected quad with perspective-correct sampling.
With both tilts at 0 the result is exactly TR-001's 2D transform, whatever
the perspective value.

**Edge cases:**
- Masks, crop and keys are in clip space, so they tilt with the picture.
- Hit-testing and handles use the projected outline; align and distribute
  (TR-045) use the projected quad's axis-aligned box.
- Blurred canvas fill (TR-012) uses the untilted frame (it is taken before
  the transform).
- Steep tilt on high-resolution footage: sampled with mipmaps so the far
  edge does not shimmer.
- Clip animations with their own 3D-style rotation (FX-018) compose on top,
  like other transform animations.
- Projects saved before this feature load with tilt 0 and look unchanged.
- Paste attributes carries tilt in the Transform group (TR-034); a
  freeze-frame still bakes it like the other transform values (TR-044).

**Undo:** `SetTransformProperty { property: TiltX | TiltY | Perspective, ..
}` or keyframe ops.

**Model notes:** `tilt_x_deg: Animatable<f64>`, `tilt_y_deg:
Animatable<f64>` and `perspective: Animatable<f64>` in `Transform` (serde
defaults 0, 0 and 50). The pure evaluator in `freecut-timeline` returns a
3×3 homography plus the projected corners (used by TR-007's hit-testing);
`freecut-render` draws the quad with it. Full 3D layers, depth sorting and
camera layers stay out of scope.

### TR-048 Beat-synced speed ramp — later

**What the user sees:** In the Speed tab's Curve mode, a *Beat ramp…* tile
(TR-039) opens a small panel. The user picks a music source, how often to
ramp and how hard, and optionally an accent, then presses *Apply*. The clip
gets a speed curve that slows down on the beats and rushes between them,
with every slow moment landing on a beat.

**Inputs / controls:**
- Music source: any clip on the timeline that has beat markers (AU-025),
  including this clip's own audio. Default: the topmost music clip that
  overlaps this clip in time. Clips with sound but no markers are listed
  with a *Detect beats* button (AU-025).
- Beat step: every beat / every 2nd beat (default) / every 4th beat.
- Intensity: quick picks *Gentle*, *Strong* (default) and *Extreme* plus a
  fine slider 0–100. The slider sets the fast-to-slow speed ratio to
  10^(slider/100), rounded to hundredths; the picks set it to 30 (≈2:1), 60
  (≈4:1) and 90 (≈8:1) (FreeCut names and values).
- Accent: *None* (default) or one effect from FreeCut's built-in effect
  library (FX-015), such as a zoom pulse, a flash, a shake or a short blur,
  whose strength peaks on each ramp beat.

**Behavior / output:**
1. Beat times come from the source clip's beat markers (auto and manual,
   after any AI-005 refinement), mapped to timeline time through the source
   clip's time map, limited to this clip's span and thinned by the beat
   step.
2. A pure planner builds a `SpeedCurve` in TR-040's format: a slow point on
   each beat and a fast point halfway between beats, at the chosen ratio,
   with Smooth segments.
3. The curve is scaled so its average speed equals the clip's current
   average speed (`D / T`). The clip therefore keeps its timeline length
   (within TR-036's frame rounding), and the beats from step 1 stay where
   they are (FreeCut default).
4. If a point would leave TR-040's 0.1×–10× range, the ratio is lowered until
   all points fit, and the panel says so.
5. If the beats need more than TR-040's 32 points, the beat step is
   coarsened (2nd → 4th → 8th) and the panel says so.
6. When the source is the clip's own audio, its beats sit in its own source
   time and move with the curve; the planner iterates (at most 8 passes,
   FreeCut default) until every slow point lands within half a frame of its
   beat.
7. With an accent, an FX-010 effect is added to the clip with strength
   keyframes (TR-027) peaking on each ramp beat.

The result is an ordinary Curve retime. It can be edited in TR-040
afterwards and keeps no link to the music; applying again replaces it.

**Edge cases:**
- Fewer than 2 beats under the clip after thinning: *Apply* is disabled with
  "Not enough beats under this clip".
- No clip with sound on the timeline: the panel says a music or audio clip
  is needed first.
- The music clip is later moved, trimmed, retimed or deleted: the ramp stays
  as it is; reopening the panel shows "Beats changed since this ramp was
  made — apply again". Alternative seen in the reference app: deleting the
  reference audio forces a new source to be picked, and replacing the source
  re-applies the ramp automatically.
- Applying replaces constant speed or an earlier curve, like any curve
  preset (TR-039); nothing else on the clip is removed (shared rule
  "Settings combine"). Alternative seen in the reference app: manual speed
  and the beat ramp exclude each other.
- Multi-selection: each media-backed clip gets its own ramp from the same
  source, in one undo step.
- Reverse: the curve is applied in timeline order (TR-039).
- Locked track, stills and text: disabled.
- Everything is local: beat detection is AU-025's DSP and the planner is a
  pure function, so the only refusal is "not enough beats"; there is no
  network failure or retry.

**Undo:** One compound op: `SetRetimeMode { new: Curve(..), ripple }` (no
ripple, as `T` is unchanged) plus, with an accent, FX's add-effect op and the
keyframe ops for its strength.

**Model notes:** `plan_beat_ramp(clip: &Clip, beats: &[RationalTime],
params: BeatRampParams) -> Result<SpeedCurve, BeatRampError>` in
`freecut-timeline`, with exact rational arithmetic (the mean of a Hermite
curve is exact, TR-036). Tests check that slow points land on beats within
half a frame and that `T` is unchanged. Beats come from TL-037 clip markers
with origin Beat (AU-025); the accent's look is FX-015's. `BeatRampParams`
is not stored on the clip; only the resulting curve and effect are.

## Interactions with other areas

- **Timeline & core editing (`timeline-editing.md`, TL):** owns tracks, clips,
  selection, split/trim/slip/ripple, the main-track role, lock/hide, and undo
  semantics (gesture coalescing, compound ops), the clipboard (TL-032), the
  freeze-frame command and still clip kind (TL-031), embedded/detached audio
  (TL-030), and *Paste attributes* (TL-034, v1): the command, the attribute
  clipboard, the checklist dialog and the `PropBag::category()` registry that
  this area (TR-034) and FX-017, CL-025 and OV-044 register into. This area
  adds clip fields (`Transform`, `Compositing`, `CropRect`,
  `CanvasBackground`, `Mask`, `ChromaKey`, `stabilize`, `Retime`) and ops.
  Speed changes reuse TL's ripple logic. Split and trim call the retime and
  keyframe helpers defined here so output is unchanged by a split; TL-023
  defers to TR-027 for keyframes: a split keeps all keyframes on both halves
  and inserts no interpolated or boundary keyframes. Disabled clips (TL-036)
  and hidden tracks (TL-008) are skipped by on-canvas picking, snapping and
  alignment; replacing a clip's media (TL-035) queues a new stabilization
  analysis (TR-026).
- **Projects & media (`project-and-media.md`, PM):** canvas size, aspect ratio,
  frame rate and project background color come from project settings; image
  backgrounds and freeze-frame stills hold `MediaRef`s; the stabilization
  analysis, reverse-preview chunks and optical-flow frames are deterministic,
  regenerable data in the media cache (PM-074), and export waits for
  regeneration where it needs them. This area puts nothing in the
  generated-media store (PM-041). Proxies help high-speed and reversed
  preview. The canvas aspect-ratio list, including "match the first clip"
  and custom ratios, is PM-003 and PM-006 (with EX-013's menu in the
  player); the default fit for new media is PM-008. Stabilization, reverse
  and optical-flow jobs appear as rows in PM-073's single background-task
  list.
- **Effects, transitions & animations (`effects-transitions-animations.md`,
  FX):** owns the authoritative per-frame render order (FX-001) and the
  compositor's working color space; this area's features feed stages L1, L3,
  L4, L7, L8 and L9 and the canvas fill, and FX-001 stage L10 executes the
  blend using OV-004's modes and formulas. Clip in/out/loop animations compose
  *on top of* the keyframed transform (additive position/rotation,
  multiplicative scale and opacity). Transitions consume handle media computed
  through the time map. Effect parameters use this area's keyframe engine,
  and on a split they follow TR-027 (FX-010 inserts no boundary keys). FX-017
  pastes effects through TL-034's `PropBag::category()` registry. Animations
  never cancel keyframes (TR-027). FX-018's 3D-style rotation presets compose
  on top of TR-047's tilt. The beat-ramp accents (TR-048) are ordinary FX-010
  effects taken from FX-015's built-in library.
- **Color, adjustments & filters (`color-and-filters.md`, CL):** adjustment
  values are animatable through this keyframe engine with CL's own
  fixed-point value types (shared-conventions rule); color interpolation is
  linear-light. CL-025 pastes color through TL-034's `PropBag::category()`
  registry. Limiting a grade to a region (CL-031, later) reuses this area's
  mask shapes, feather, invert and keyframes (TR-017..TR-019) instead of
  defining its own.
- **Audio (`audio.md`, AU):** volume/pan keyframes use this engine with AU's
  own `GainDb(i32)` value type, and on a split they follow TR-027 (AU-008
  adds no boundary volume keyframes). AU-023 owns the *Keep pitch* toggle
  (default on), its `SetPreservePitch` op and the time-stretch engine; the
  flag is stored in `ClipAudio.preserve_pitch` and travels with the clip's
  audio on detach (AU-003). The engine follows the time map defined here
  (TR-038), which is the only thing this area defines for audio; detached
  audio clips carry their own `Retime`. AU-025's beat markers are stored in
  source time and drawn through the time map, so speed changes, curves and
  reverse keep them; TR-048 reads them to plan beat ramps. Reversed clips
  keep isolation, stems, voice effects and pitch shift (AU-021, AU-022,
  AU-024).
- **Overlays & stickers (`overlays-and-stickers.md`, OV):** OV-004 owns
  `BlendMode`: the mode list, formulas, serialization and hover preview. This
  area only places the Blend dropdown in its Compositing section (TR-016).
  Stickers and shapes use this area's Transform, Compositing, masks and
  keyframes (with OV's `f64` values); they have no time map. OV's style
  properties (OV-044) reach *Paste attributes* through TL-034's
  `PropBag::category()` registry. OV also owns three neighbours of this
  area's canvas tools: rounded corners and borders on clips (OV-005; the
  rectangle mask's roundness, TR-017, is separate), bring forward / send
  backward (OV-007), and picture-in-picture position presets (OV-003), which
  use the same item bounds as TR-045's align and distribute. Moving a
  selected group as one box is OV-046.
- **Text & captions (`text-and-captions.md`, TX):** text clips use the same
  Transform, Compositing, masks and keyframes and have no time map; TX stores
  its own animatable values as fixed-point integers on the generic keyframe
  engine (TR-027), as the shared-conventions rule allows. The text-shaped
  mask (TR-046) uses TX's font registry, shaping, fallback and missing-font
  rules (TX-003, TX-006, TX-007, TX-009); align and distribute (TR-045) use
  TX-005's layout box as a text clip's bounds.
- **AI tools (`ai-tools.md`, AI):** owns background removal without a green
  screen, subject masks, motion tracking (a binding evaluated on top of this
  area's transform, which can be baked into its keyframes), auto-reframe,
  and ML frame interpolation (AI-018, later), which defines the behaviour of
  the `FrameSampling::Ai { quality }` variant reserved in TR-041 and keeps
  its frames in the media cache (PM-074). Under the shared rule "Settings
  combine": tracking (AI-008) survives rotation and speed changes; the
  automatic cutout matte (AI-014) and the chroma key multiply at L4;
  AI-015's keep/remove brushes are the hand-painting tool for keyed clips;
  auto reframe and subject follow (AI-016, AI-017) work with crop and
  stabilization (mapped through AI-007).
- **Preview & export (`preview-and-export.md`, EX):** the player hosts the
  on-canvas handles, mask overlay, matte view and safe-area guides; export
  waits for stabilization analysis and optical-flow or AI frames to be
  (re)generated in the media cache, and decodes reversed media without
  relying on preview caches. Export preflight (EX-036) lists paused or failed
  stabilization and missing background images; offline clips use EX-005's
  slate.
- **Workspace & shortcuts (`workspace-and-shortcuts.md`, UI):** inspector
  layout (Transform / Compositing / Background / Mask / Key / Stabilize / Speed
  sections) and all keyboard shortcuts referenced here (add keyframe,
  prev/next keyframe, rotate 90° both ways, reverse, crop, *Speed…*, the
  eight align/distribute commands and *Canvas snapping*), including *Paste
  attributes…* (Ctrl+Alt+V), which UI-012 and UI-031 list as v1. Binding
  rules (bare keys allowed) are UI-032's. The keyframe context menu's items
  are defined here (TR-029, TR-031, TR-032); toasts with *Undo* are UI-040's.

## Out of scope (and why)

- **Cloud-processed effects** (server-side stabilization, server-side
  interpolation, online preset libraries): FreeCut is local-only; every
  feature here runs on the user's machine.
- **Paywalls, free-use counters and premium gating** on any tool in this
  area (masks, keying, stabilization, speed curves, frame smoothing,
  background patterns, preset curves), and an export check that lists
  "premium" features used: FreeCut is free and open source; every feature is
  always available with no counters, quotas or export gate.
- **Account-bound or downloadable preset and pattern libraries** (speed-curve
  shapes, easing presets, canvas background patterns fetched from a server):
  FreeCut bundles its own presets and CC0 patterns and lets users import
  their own images (TR-013); nothing is fetched or synced.
- **Cloud AI "style" looks that ramp speed automatically**, and their
  conflicts with manual speed and reverse: dropped (cloud generation, credit
  economics). Local substitutes: speed-curve presets (TR-039), the custom
  editor (TR-040), beat-synced ramps planned from local beat detection
  (TR-048) and ordinary effects (FX).
- **Server-assisted beat-ramp generation:** the reference app's wording
  suggests a server may help; FreeCut plans ramps with local DSP (AU-025)
  and a pure planner (TR-048), so there is no "try again later" failure.
- **Generative expand / outpaint and generated backgrounds** to fill the
  canvas around a reframed clip: dropped (cloud models). Local substitutes:
  color, blurred or image fill (TR-011..TR-013), Fill mode (TR-002), auto
  reframe (AI-016) and, later, a local cutout (AI-014) in front of a
  user-chosen image or video.
- **AI subject masks, background removal without a key, motion tracking,
  auto-reframe and ML frame interpolation:** belong to `ai-tools.md` (ML
  frame interpolation is AI-018; this area only reserves its frame-sampling
  value, TR-041).
- **Full 3D: depth-sorted 3D layers, corner-pin and camera layers:** beyond
  the everyday CapCut-class workflow; may be revisited after v1. A simple
  per-clip perspective tilt is TR-047 (later).
- **Outline, stroke offset and shadow along a mask edge:** present only as
  unlabelled, apparently unreleased settings in the reference app; not
  planned here. OV-005 lists a border that follows the mask as a later
  improvement, and OV-006 outlines visible shapes.
- **Automatic "responsive" layout of several elements:** its scope in the
  reference app is unclear (it may belong to templates). Single clips use
  fit modes (TR-002) and align/distribute (TR-045); multi-item layouts are
  OV's (OV-003, OV-045).
- **Picking colors from outside FreeCut's window:** would need an OS
  screen-recording permission; eyedroppers here read only FreeCut's own
  player, and other colors are typed as hex.
- **Suggestion lists that recommend a beat ramp or other speed effects for a
  clip:** no recommendation engine in this area; the ramp is one tile in the
  Speed tab (TR-039, TR-048).
- **Rolling-shutter correction and lens-distortion correction:** niche and
  analysis-heavy; not part of the stabilizer.
- **Per-property graph editor for all properties at once (full value-graph
  editing like compositing apps):** the segment curve editor (TR-032) covers
  the everyday need.
- **Keyframing discrete settings (blend mode, flips, mask shape):** not
  needed for the everyday workflow; may be added later as hold keyframes.
- **Mobile-style gestures (pinch to scale):** FreeCut is a desktop app; the
  same results come from handles, scroll-wheel over the player (scale) and
  numeric fields.

## Clean-room notes

- **Behavior only.** This spec was written from public help pages, tutorials
  and reviews, from general knowledge of video editing, and from a
  paraphrased inventory of the reference app's English UI labels, taken from
  the bundled English localization file of the user's local install (CapCut
  desktop 9.5.0; see Provenance in the [README](README.md)). Nothing was
  decompiled, disassembled, traced or translated, and no project files, network traffic, assets, presets, models or UI text
  were copied. The labels were used only to learn which options, ranges,
  limits and refusal messages exist; every label in this file is FreeCut's
  own wording, and where the observed behavior differs from FreeCut's choice
  the entry says so in one line ("Alternative seen in the reference app").
- **Speed-curve presets:** shapes and names (Accelerate, Decelerate, Slow
  center, Fast center, Pulse, Double dip) are FreeCut's own designs, stored as
  plain point lists; we do not reproduce any other product's preset names or
  exact curves.
- **Easing presets:** implemented from the mathematical definitions of common
  easing families (sine, polynomial, exponential, overshoot, elastic, bounce)
  and the cubic-Bezier timing function used in CSS; FreeCut names them by
  behavior (ease in/out, overshoot, elastic, bounce), and the code is written
  from scratch.
- **Blend modes:** defined in OV-004 from the W3C *Compositing and Blending
  Level 1* specification (open standard); MDN's description was consulted as a
  readable reference.
- **Stabilization strength names** (Light, Balanced, Strong) and their
  smoothing/zoom numbers are FreeCut's own choices.
- **Mask shapes (line, band, ellipse, rectangle, heart, star):** geometric
  signed-distance functions written by us; no artwork is shipped. The text
  mask (TR-046) draws letters with FreeCut's own text engine from bundled
  open-licensed fonts (TX) or the user's fonts; no font is taken from the
  reference app.
- **Align and distribute (TR-045):** generic layout operations found in
  every drawing and layout tool; the gap-spacing rule and the defaults are
  FreeCut's.
- **Perspective tilt (TR-047):** a standard projective transform
  (homography) from public computer-graphics literature; ranges and the
  perspective mapping are FreeCut's own.
- **Beat ramp (TR-048):** the planner, the intensity names (Gentle, Strong,
  Extreme) and their ratios are FreeCut's own; beats come from AU-025's
  textbook DSP. Accents are effects from FreeCut's own library (FX-015); no
  reference-app effect, preset or name is reproduced.
- **Keep shadows (TR-023):** a luma-ratio formula of our own design.
- **Canvas background patterns:** FreeCut ships its own small set of pattern
  images released as CC0; users can import any image. No third-party
  background library is bundled.
- **Swatch palette:** FreeCut's own 12 colors.
- **Stabilization:** our own global-motion estimation (phase correlation on
  downscaled frames, from public literature) as the default; optional
  integration of *libvidstab* through FFmpeg (GPL-2.0-or-later, compatible
  with FreeCut's GPL-3.0-or-later) where the FFmpeg build includes it.
- **Pitch-preserving time-stretch:** engine choice and its license are
  AU-023's (`audio.md`); this area ships no audio code.
- **Optical flow (later):** FFmpeg's motion-interpolation filter
  (LGPL/GPL depending on build); any ML model goes through `ai-tools.md`
  (AI-018) with its license checked (code and weights separately).
- **No CapCut screenshots** are referenced or committed; UI descriptions in
  this file are FreeCut's own design.

## Sources

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- A Comprehensive Guide to Keyframes in CapCut PC — https://www.capcut.com/resource/how-to-add-keyframes-in-capcut
- Keyframes in CapCut (VideoWizardTools) — https://videowizardtools.com/keyframes-in-capcut/
- CapCut mask keyframes guide (VideoWizardTools) — https://videowizardtools.com/capcut-mask-keyframes-guide/
- CapCut masking tutorial (CapCut Guide) — https://capcutguide.com/capcut-masking-tutorial/
- How to Remove Green Screen in CapCut: A Desktop Chroma Key Guide — https://www.capcut.com/resource/how-to-remove-green-screen-in-capcut
- How Can I Fix the Chroma Key Feature Not Working in CapCut? — https://www.capcut.com/help/chroma-key-feature
- Your Guide on How to Do Velocity on CapCut — https://www.capcut.com/resource/how-to-do-velocity-on-capcut
- CapCut Speed Curve tool page — https://www.capcut.com/tools/speed-ramp
- How to Speed Up a Video (CapCut resource) — https://www.capcut.com/resource/how-to-speed-up-a-video
- CapCut desktop listing on the Microsoft Store — https://apps.microsoft.com/detail/xp9kn75rrb9nhs
- Velocity in CapCut (Shotkit) — https://shotkit.com/velocity-capcut/
- How to Do Optical Flow in CapCut — https://www.capcut.com/resource/optical-flow-in-capcut
- CapCut Stabilizer: How to Stabilize Shaky Videos — https://www.capcut.com/resource/capcut-stabilizer
- How to Quickly Stabilize Videos in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-quickly-stabilize-videos-in-capcut--cms-108875
- How to Create Custom Backgrounds in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-create-custom-backgrounds-in-capcut--cms-108866
- How to Blur Videos in CapCut — https://capcut.com/resource/how-to-blur-videos-in-capcut
- How to Crop Video in CapCut PC — https://www.capcut.com/resource/how-to-crop-video-in-capcut-pc
- How to Crop Videos on CapCut (TourBox) — https://www.tourboxtech.com/en/news/how-to-crop-on-capcut-pc.html
- How to Freeze a Frame in CapCut PC (TourBox) — https://www.tourboxtech.com/en/news/how-to-freeze-a-frame-in-capcut-pc.html
- Why Isn't the Reverse Feature Working in CapCut? — https://www.capcut.com/help/reverse-feature
- How to Reverse a Video on CapCut (Movavi) — https://www.movavi.com/learning-portal/how-to-reverse-a-video-on-capcut
- How to Mirror a Video on CapCut — https://www.capcut.com/resource/how-to-mirror-videos-on-capcut
- How to Flip a Video Across Devices — https://capcut.com/resource/how-to-flip-videos
- Uniform scale absent (community forum thread) — https://www.capeditcut.com/community/capcut-pro/uniform-scale-absent/
- How to edit blend modes in CapCut PC (third-party guide) — https://resources.oreateai.com/resources/how-to-edit-blend-modes-in-capcut-pc-for-cinematic-video-overlays
- blend-mode (MDN Web Docs) — https://developer.mozilla.org/en-US/docs/Web/CSS/blend-mode
- Shotcut QML API: KeyframesModel — https://shotcut.org/notes/qml-api/qml-org-shotcut-qml-keyframesmodel.html
- Keyframe Types and Easing (Shotcut forum) — https://forum.shotcut.org/t/keyframe-types-and-easing/42271
- Time Remap filter (Shotcut forum) — https://forum.shotcut.org/t/time-remap-filter/27289
- KDE Bug 353066: keyframeable speed effect with intact audio — https://bugs.kde.org/show_bug.cgi?id=353066
- Kdenlive issue 1379: frame blending in time remap — https://invent.kde.org/multimedia/kdenlive/-/issues/1379
- Kdenlive merge request 928: time remap keyframe types — https://invent.kde.org/multimedia/kdenlive/-/merge_requests/928
- Change Speed: pitch compensation unavailable (KDE Discuss) — https://discuss.kde.org/t/change-speed-pitch-compensation-unavailable/39403
