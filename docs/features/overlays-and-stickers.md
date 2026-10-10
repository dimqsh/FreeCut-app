# Overlays, stickers, shapes & templates

> Status: draft · Owns: blend modes (`BlendMode`: mode list, formulas, serialization and hover preview — TR-016 only places the dropdown, FX-001 L10 executes the blend); the element-styling assembly that FX-001 runs at L8b; the clip-time → animation-time mapping of animated images and stickers (OV-013); element clip content (stickers, emoji, shapes, lines, frames, custom paths, charts) and the chart data model (`ChartSpec`); clip border / corner radius, element outline & shadow, clip frames; layer-order commands; element-pack manifest & licence metadata; the project-wide asset-credits sheet; user project templates and replaceable placeholder slots · Crates: freecut-timeline, freecut-render, freecut-media, freecut-project, freecut-app

## Summary

This area covers everything a FreeCut user stacks *on top of* their main footage:
picture-in-picture video, logos and still images with transparency, animated
GIF/APNG/WebP stickers, emoji, vector shapes, pen-drawn paths and arrows,
decorative frames and borders, and data charts built from a table. It also
covers the element library those items come from (our own and open-licensed
packs plus anything the user imports), how the user finds and favorites things
in it, and user-made project templates with replaceable placeholder slots.
Everything is local: no online sticker store, no template marketplace, no
account.

Shared vocabulary used below:

- **Visual stack** — all visual tracks, bottom to top. A clip on a higher track
  draws over clips on lower tracks at the same time. The track model (one
  *main track*, typed overlay tracks such as `Video` and `Sticker`, strict vs
  flexible stacking), selection and undo semantics are defined in
  [timeline-editing.md](timeline-editing.md) (TL-004, TL-007, TL-015, TL-043);
  per-frame render order is defined in
  [effects-transitions-animations.md](effects-transitions-animations.md)
  (FX-001).
- **Overlay** — any visual clip that is not on the main track. Being an overlay
  is a matter of position, not a separate clip kind.
- **Element clip** — a clip whose content is a library item rather than an
  imported media file: a sticker, emoji, shape, custom path, line/arrow, frame
  or chart. In TL's
  clip model (TL-011) it is a *generated* clip; it lives on a `Sticker` track
  (TL-004).
- **Reference short side** — sizes stored for borders, outlines and shadows are
  stored as a fraction of the canvas short side so they survive a resolution
  change. The UI shows them as pixels at a 1080-px short side ("px@1080").
- **Numeric storage** — only *time* must be rational (TL-001; project
  `CLAUDE.md`). For every other value the project-wide rule, recorded once in
  [clip-transform-and-speed.md](clip-transform-and-speed.md) (*Shared
  conventions*, non-time values), lets each owning spec pick its own type.
  OV's choice: `f64` for sizes, reference-short-side fractions, positions,
  angles, radii and percentages; `Rgba8` for colours; rationals for anything
  that converts one time into another (the element playback rate, OV-013).
  OV values that can be keyframed (shape parameters, line endpoints, path
  progress) use TR-027's generic `Animatable<T: Interpolate>`; OV implements
  `Interpolate` for its own value types (e.g. `ShortSideFrac`). OV never
  converts another area's stored values to its own types.

## Features

### OV-001 Overlay clips on upper tracks (picture-in-picture) — mvp

**What the user sees:** Dragging a video or image from the media bin onto any
`Video` track above the main track makes it appear on top of the footage below
in the preview. The overlay can be moved and resized directly in the preview
(handles from TR-007 in
[clip-transform-and-speed.md](clip-transform-and-speed.md)), and trimmed on
the timeline to control when it appears and disappears.

**Inputs / controls:**
- Drag from bin to an overlay track, or to the empty area above the top track
  (creates a new track there). Drop, auto-create and *Add as overlay at
  playhead* rules are TL-006 / TL-012 in
  [timeline-editing.md](timeline-editing.md).
- Transform, fit mode, crop and opacity of the overlay: owned by
  [clip-transform-and-speed.md](clip-transform-and-speed.md) (TR-001, TR-002,
  TR-009, TR-015).
- Default framing of a newly placed overlay: TR-002's default *Fit* (the whole
  source fitted inside the canvas, like a main-track clip), so the user
  immediately sees it cover the footage and shrinks it from there. OV-003
  offers one-click corner placement.

**Behavior / output:**
- Compositing follows FX-001: premultiplied "source over" — the overlay's
  pixels replace what is below, weighted by the overlay's alpha and opacity —
  with OV-004's blend formula executed at FX-001's blend stage (L10) when the
  mode is not *Normal*.
- Video overlays keep their embedded audio (TL-011); the audio is mixed with
  every other track (see [audio.md](audio.md)). Nothing is auto-muted.
- Preview and export use the same compositor, so the export matches the preview
  frame for frame ([preview-and-export.md](preview-and-export.md)).

**Edge cases:**
- Dropping onto the main track inserts per the magnetic main-track rules
  (TL-005); it does not create an overlay. The drop indicator must make this
  visible before release.
- An overlay that extends past the end of all main-track clips still renders,
  over the canvas background (PM-009 colour; TR-011 fills from v1).
- An overlay whose source frame rate differs from the project (e.g. 60 fps over
  a 30 fps project, or 23.976 over 25): frames are picked with the clip's frame
  sampling setting (TR-041, default *Nearest frame*); no separate rule here.
- Overlay with a different aspect ratio: fitted (letterboxed inside its own
  bounds); the bars are transparent, not black, so the footage below shows
  through them.
- Zero-length result after trimming is not allowed; trims clamp at one frame
  (TL-011).
- Overlay attached to a main-track clip (TL-029): it shifts with that clip when
  the main track ripples.
- Missing media: drawn as the offline slate (TL-011, EX) at the overlay's last
  known size and position so the layout stays readable; relink per
  [project-and-media.md](project-and-media.md).
- Locked track (TL-009): the overlay cannot be picked or transformed in the
  preview (TR-007) and its timeline clip cannot be edited.
- Hidden track (TL-008) or disabled clip (TL-036): skipped in preview and
  export.
- Track limit: overlay tracks count toward TL-004's sequence-wide limit (100
  tracks, FreeCut default); there is no separate overlay cap. A drop or
  *Add as overlay* that would need one more track past the limit is refused
  before any change, and the toast names the limit (TL-006, TL-043
  validation-first). Alternative seen in the reference app: a hard cap of 7
  overlay tracks; FreeCut does not adopt it because the 100-track limit
  already bounds compositor cost.

**Undo:** Placing is TL's `InsertClips` (TL-012; undo removes the clip and any
track it created); moving between tracks is TL's `MoveClips` (TL-018).
Transform edits are TR ops.

**Model notes:** No new clip kind: media overlays are ordinary media clips on
`Video` overlay tracks (TL-004). Z-order is TL-004's vertical track order,
under strict layering or flexible stacking (TL-007). `freecut-render`
composites the visual stack at a given time in the order FX-001 defines; OV
only needs a pure
`freecut-timeline` query that lists the visible visual clips at time `t`
bottom to top, which the compositor, OV-007 and OV-008 share.

### OV-002 Stack several items as layers at the playhead — v1

**What the user sees:** Adding a single item as an overlay at the playhead is
TL-012's *Add as overlay at playhead*. OV adds a multi-item variant for
collages and layered looks: with several media-bin or library items selected,
a right-click *Stack as layers at playhead* puts every item at the playhead at
once, each on its own track, instead of TL-012's back-to-back placement.

**Inputs / controls:** Two or more selected media-bin items and/or element
library items; playhead position. Shortcut: proposal for UI's keymap
([workspace-and-shortcuts.md](workspace-and-shortcuts.md)).

**Behavior / output:**
- Items are stacked upward in selection order, all starting at the playhead.
- Each item goes to a track of the kind it needs (TL-004): video and images to
  `Video` overlay tracks, element items to `Sticker` tracks. For each item the
  lowest unlocked track of that kind with no clip in
  `[playhead, playhead + duration)` that is not already used by this command is
  taken; otherwise a new track is created in the right band (TL-006).
- Duration: media's natural length (video/animated), or the default still /
  element duration (PM-010 for stills; elements per OV-018).
- All new clips are selected and the inspector shows the multi-selection.

**Edge cases:**
- Playhead past the end of the timeline: clips are placed at the playhead
  anyway, leaving a gap.
- All candidate tracks locked: new unlocked tracks are created.
- Audio-only bin items in the selection: skipped, with a note saying how many.
- Item whose media is offline: still placed, shown as missing (OV-001).
- Track limit (TL-004) would be exceeded: the whole command is refused before
  any change (TL-043 validation-first), with a toast.

**Undo:** One `InsertClips` transaction covering all items and any tracks it
created (TL-012, TL-043); undo removes them together.

**Model notes:** A pure planner in `freecut-timeline` built on TL's
`plan_insert` (TL-012) that picks one free compatible track per item; it
returns one change set so the command is one undo step.

### OV-003 Picture-in-picture position presets — v1

**What the user sees:** With an overlay selected, a small 3×3 grid of buttons in
the inspector ("PiP position") plus a size dropdown. One click snaps the overlay
into a corner, an edge centre, or the centre, at a chosen size.

**Inputs / controls:**
- Anchor: 9 positions (top-left … bottom-right). Default: bottom-right.
- Size: 25 %, 33 %, 50 % of canvas width, or "Keep current". Default 33 %.
- Margin: 0–15 % of the canvas short side, default 4 % (FreeCut default).

**Behavior / output:** Sets the overlay's scale and position (TR properties) so
its visible, cropped bounds sit at the anchor with the margin. Rotation is left
as is; bounds are computed from the rotated rectangle's axis-aligned box.

**Edge cases:**
- Overlay with keyframed position/scale: the preset writes a keyframe at the
  playhead if the property is already keyframed; otherwise it sets the static
  value (keyframe rules in TR).
- Overlay larger than the target cell because "Keep current" size exceeds the
  canvas: it is still anchored; it overflows on the far side.
- Vertical (9:16) and horizontal canvases both use canvas width for the size
  percentage; in 9:16 a 50 % PiP is therefore smaller in absolute pixels.

**Undo:** One `SetTransform` step (TR op) per click.

**Model notes:** Pure function in `freecut-timeline`
(`pip_transform(anchor, size, margin, clip_bounds, canvas) -> Transform`);
unit-tested for all 9 anchors and both orientations.

### OV-004 Layer blend modes — v1

**What the user sees:** A *Blend* dropdown next to opacity for any visual clip
(video, image, text, element). This feature owns `BlendMode`: the mode list,
the formulas, the serialization and the hover preview. TR-016 only places the
dropdown in the inspector's *Compositing* section, and FX-001's L10 stage
executes the blend. Default *Normal*. Hovering an entry previews the result
live in the player; clicking commits it.

**Inputs / controls:** Modes, grouped in the dropdown:
- Normal
- Darken group: Darken, Multiply, Color Burn, Linear Burn
- Lighten group: Lighten, Screen, Color Dodge, Add (linear dodge)
- Contrast group: Overlay, Soft Light, Hard Light
- Difference group: Difference, Exclusion, Subtract
- Component group: Hue, Saturation, Color, Luminosity

**Behavior / output:**
- The clip is blended against the composite of everything below it in the
  visual stack at that frame (not just the next track down).
- Blend math follows the open W3C *Compositing and Blending Level 1* definitions
  for the modes it defines (on un-premultiplied colour, then source-over);
  Linear Burn, Add and Subtract use the standard arithmetic formulas clamped to
  [0, 1].
- Opacity then mixes the blended result with the backdrop, so a Screen layer at
  30 % is 30 % of the screened result. The alpha that weights the blend is the
  one left after key (FX-001 L4), mask and corner coverage (L7), element
  styling (L8b) and opacity (L9); TR-016 states this for the Compositing
  section.
- An element's own styling is assembled into the clip's layer first, at
  FX-001's L8b stage, bottom to top: drop shadow (OV-006), outline (OV-006),
  the clip content, border (OV-005), clip frame (OV-029). That assembled layer
  is blended exactly once at L10, so a sticker set to Screen has its outline
  screened too rather than drawn normally on top.
- What this is good for, in short: the lightening modes make dark areas of a
  layer disappear (bright effect footage recorded on black), the darkening
  modes make light areas disappear (line art or scans on white), and the
  contrast modes lay a texture or grain over the footage while keeping its
  midtones.

**Edge cases:**
- Clip on the main track: blends against the canvas background fill (TR-016,
  TR-011).
- Transitions between two clips on the same track: FX-001 renders both sides
  through L1–L9, mixes them with the transition, and blends that result once
  at L10 using the incoming clip's mode; OV adds no separate rule.
- Adjustment/effect layers (FX): no blend dropdown unless FX defines one.
- Partially transparent sources (PNG with soft edges): blend is computed on
  un-premultiplied colour, then weighted by alpha, so edges do not darken.
- Colour space: blending runs in the working space that FX-001 fixes at its
  normalize stage (L2); this spec does not choose its own.
- Not keyframable: the mode is a discrete choice (OV rule; TR-016 repeats it
  for the Compositing section).
- Unknown mode string in a project from a newer version: load as Normal and
  show a one-time warning.

**Undo:** `SetBlendMode { clip, old, new }` (defined here; TR-016 reuses it in
its Compositing reset); reversing restores `old`. Hover-preview is view state
and creates no undo entry.

**Model notes:** OV-004 owns the `BlendMode` enum, defined in
`freecut-timeline` (no GPU types), its formulas, its serialization and the
hover preview. TR stores the value as `blend: BlendMode` in its `Compositing`
struct next to opacity (TR-016); `freecut-project` serializes it as kebab-case
strings. The hover preview is a transient mode override that `freecut-app`
passes to the preview render request; it never touches the model.
`freecut-render` implements the formulas as one WGSL blend function switched
on a mode index, which FX-001's L10 stage calls; reading the backdrop requires
rendering the stack bottom-up into an intermediate target (render order owned
by FX-001). Each mode gets a CPU reference implementation in tests to check
shader output on known colour pairs.

### OV-005 Overlay border and rounded corners — v1

**What the user sees:** A *Border* section for rectangular visual clips (video,
images, PiP): toggle, width, colour, and *Corner radius*. Common for webcam PiP
and "framed photo" looks.

**Inputs / controls:**
- Border: off by default. Width 0–200 px@1080, default 8 when enabled. Colour
  picker with alpha, default white. Position: inside / centre / outside, default
  outside.
- Corner radius: 0–50 % of the clip's visible short side (50 % turns a square
  into a circle, a rectangle into a pill). Default 0. Applies with or without a
  border.

**Behavior / output:** The two parts run at different FX-001 stages:
- **Corner radius** is a coverage term on the clip's visible rectangle (after
  TR crop): at L7, in layer space, the rounded-rectangle coverage multiplies
  the clip's alpha together with any TR mask. Because it is applied before the
  transform, the rounding scales and rotates with the clip.
- **Border** is element styling: at L8b, in canvas space after the transform,
  it is drawn along the transformed rounded rectangle, above the content and
  below a clip frame (OV-029). Its width therefore does not scale with the
  clip's scale (it stays the set width on canvas), so shrinking a PiP keeps a
  crisp, even border.
- Both are part of the clip's assembled layer, so the clip's opacity (L9) and
  blend mode (L10, OV-004) apply to them together with the content.

**Edge cases:**
- Clip with a TR mask: border follows the cropped rectangle, not the mask shape
  (border-follows-mask is a later improvement).
- Source with transparent areas (logo PNG): the rectangle is the image bounds,
  so the border is a box; use OV-006 to outline the visible shape instead.
- Width larger than half the visible short side with "inside": clamps so the
  content is never fully covered.
- Very small clips (< 4 px): corner radius clamps to half the short side.
- Non-uniform scale: the rounded corners become elliptical on the canvas; the
  border follows that shape at a constant canvas width.
- Border colour alpha 0: same as off, but the setting is kept.

**Undo:** `SetBorder { clip, old, new }` and `SetCornerRadius { clip, old, new }`.

**Model notes:** `Border { width: ShortSideFrac, color: Rgba8, align:
BorderAlign }` and `corner_radius: f64` (fraction of the visible short side) on
the clip's visual style in `freecut-timeline`; `ShortSideFrac` is a small
`f64` newtype for the reference-short-side unit (OV's own non-time type, see
*Numeric storage* above). `freecut-render` evaluates one rounded-rect signed
distance field for both parts: as alpha coverage in the L7 pass and as the
border stroke in the L8b pass.

### OV-006 Element outline and drop shadow — v1

**What the user sees:** For any visual clip with transparency (stickers, cut-out
PNGs, emoji, shapes; text has its own outline and shadow in TX), an *Outline*
toggle that draws a
coloured stroke around the visible shape (the "die-cut sticker" look), and a
*Shadow* toggle for a soft drop shadow or glow.

**Inputs / controls:**
- Outline: width 0–60 px@1080 (default 6 when enabled), colour (default white),
  opacity 0–100 % (default 100).
- Shadow: colour (default black), opacity 0–100 % (default 50), angle 0–359°
  (default 135°, light from top-left), distance 0–100 px@1080 (default 8),
  blur 0–100 px@1080 (default 12), spread 0–100 % (default 0). Distance 0 with
  a light colour gives a glow.

**Behavior / output:** Outline and shadow are element styling, built at
FX-001's L8b stage in canvas space, after the clip's transform. They are
derived from the clip's alpha as it leaves L8 (so after key, mask and corner
coverage), which keeps them correct when the element is rotated or scaled and
keeps their widths and the shadow angle fixed on the canvas. Animated stickers
recompute them per frame. L8b assembles the clip's layer bottom to top as
shadow, outline, content, border (OV-005), clip frame (OV-029). The clip's
opacity (L9) then applies to the whole assembled layer, and the layer is
blended once with the clip's mode (L10, OV-004).

**Edge cases:**
- Fully opaque source (a normal video): outline traces the rectangle; shadow is
  a rectangular drop shadow (works for PiP too).
- Alpha with noise (semi-transparent fringes from a cut-out): an alpha
  threshold of 50 % defines the outline edge (FreeCut default).
- Large outline on thin features: features merge; expected.
- Outline/shadow extending past the clip's own bounds is not cut off by the
  layer-space bounds of its effects (L6), because L8b draws in canvas space;
  it is clipped only by the canvas, like the element itself.
- Clip with a TR mask: outline and shadow trace the masked shape, since they
  read the alpha after L7.
- Performance: static elements cache their outline/shadow per (size, rotation)
  so playback does not recompute every frame.

**Undo:** `SetOutline { clip, old, new }`, `SetShadow { clip, old, new }`.

**Model notes:** Parameters stored in the clip's visual style in
`freecut-timeline` (widths, distance and blur as `ShortSideFrac`, angle in
degrees as `f64`, OV's own non-time types). `freecut-render` runs them in the
L8b pass: the outline with a GPU distance transform (jump flooding) on the
alpha and the shadow with a separable Gaussian blur on an offset alpha copy,
both in WGSL.

### OV-007 Layer order commands — v1

**What the user sees:** Right-click on a visual clip (timeline or preview):
*Bring forward*, *Bring to front*, *Send backward*, *Send to back*, with
shortcuts. The clip moves up or down the visual stack; the timeline shows it on
a different track.

**Inputs / controls:** One or more selected visual clips (not on the main
track).

**Behavior / output:** Order is defined only against clips that overlap the
selected clip in time. Targets are always tracks of the clip's own kind
(TL-004: media overlays on `Video` tracks, elements on `Sticker` tracks), and
under strict layering (TL-004 default) a clip never leaves its band.
- *Bring forward*: find the nearest higher track within reach holding a clip
  that overlaps in time. Move the clip to a track of its kind directly above
  that one: reuse an existing unlocked track there if it is free over the
  clip's range, otherwise create a new track there (TL-006).
- *Bring to front*: same, relative to the highest overlapping clip within
  reach.
- *Send backward* / *Send to back*: mirror image, but never onto or below the
  main track; the lowest allowed position is the first track above it.
- "Within reach": under strict layering, only tracks in the clip's own band;
  with flexible stacking on (TL-007), any visual track above the main track.
- If nothing overlaps in that direction within reach, the command is disabled.
  When the only overlapping clips are in the other band (e.g. sending a
  sticker under a video overlay under strict layering), the disabled item's
  hint points to *Flexible stacking* (TL-007).

**Edge cases:**
- Locked tracks are never used as targets and never moved; a clip on a locked
  track cannot be reordered (TL-009).
- Multi-selection: clips keep their relative order and move as a block; mixed
  kinds are each kept within their own kind's tracks.
- Linked audio (TL linking) stays linked; only the visual part changes track.
- Moving a clip that is part of a transition (FX) on its current track: the
  command is disabled with a hint, because the transition needs both clips on
  one track.
- Group members (TL-039): only the selected member moves; the group keeps it.
- Empty tracks left behind are removed by TL's auto-remove policy (TL-006).
- Track limit (TL-004) reached and a new track is needed: refused with a toast.

**Undo:** One TL `MoveClips` transaction (TL-018, TL-043), including any track
it created or auto-removed. Undo restores the original track and removes any
created track.

**Model notes:** Pure `freecut-timeline` functions computing the target track
from the stack and the layering mode; tests cover overlap/no-overlap, locked
tracks, main-track floor, strict vs flexible stacking and track kinds.

### OV-008 Click-through selection in the preview — v1

**What the user sees:** This refines the player picking defined in TR-007.
Clicking in the preview selects the topmost clip whose *visible pixels* are
under the cursor, not just its bounding box: clicking the transparent corner of
a round sticker selects what is behind it.

**Inputs / controls:** Click; Ctrl/Cmd+click cycles through every clip stacked
under the cursor (TR-007's cycle gesture); adding to the selection follows
TL-015.

**Behavior / output:** Hit test walks the visual stack top-down at the current
frame; a clip is hit if the point is inside its transformed bounds (TR-007)
*and* its alpha there is above 10 % (FreeCut default). The main track is picked
only when nothing above it is hit. Once a clip is selected, dragging anywhere
inside its bounding box still moves it (TR-007), so masked or mostly
transparent clips stay easy to grab.

**Edge cases:**
- Alpha data not yet available (still decoding): fall back to bounds.
- Clips on locked or hidden tracks and clips at 0 % opacity are skipped, as in
  TR-007; they stay selectable from the timeline.
- Very small elements (< 12 screen px): bounds are padded to 12 px for hit
  testing.
- Text clips use their glyph coverage the same way (rasterised by TX).

**Undo:** n/a (view state — selection).

**Model notes:** Hit testing must not decode on the UI thread: `freecut-render`
publishes a small alpha map (longest side ≤ 256 px) per visible clip after each
rendered frame; `freecut-app` reads the latest maps. Bounds come from the
evaluated transform computed in `freecut-timeline` (pure), as in TR-007.

### OV-009 Span an overlay across the project — v1

**What the user sees:** For logos and watermarks, a right-click *Extend to whole
project* on any visual clip, and *Extend to project end* (keeps its start).

**Inputs / controls:** Selected visual clip(s).

**Behavior / output:** Sets the clip's start to 0 (first command only) and its
end to the sequence end (the sequence duration as defined by TL-003, computed
without the selected clip). Stills and elements simply get longer. Media whose
OV-013 playback mode loops (TL-011's `SourceLimits` reports it *unbounded*)
is extended the same way; plain video cannot be extended past its media
length, so the command reports how far it got.

**Edge cases:**
- Empty project apart from this clip: nothing to span; disabled.
- Something else occupies the same track in the target range: the clip is moved
  to the first free track of its kind (search as in OV-002) rather than
  overwriting.
- Clip on the main track: command disabled (main-track length is governed by
  TL-005).
- Locked track: disabled (TL-009).
- Later changes to project length do not follow automatically (one-time edit).

**Undo:** One TL transaction (TL-043): the range change (TL-020 trim / TL-018
move ops) plus `MoveClips` and any created track if the clip was relocated.

**Model notes:** Pure planner in `freecut-timeline` using TL-003's sequence
duration; no new op type.

### OV-010 Replace an element's source, keeping its settings — v1

**What the user sees:** Replacing the media of a video/image overlay is TL-035
(*Replace…*, or drag with `Mod+Alt` held). OV extends the same gesture to
element clips: a sticker, emoji or canvas frame can be swapped for another
library item, and the new item takes the old one's place, size, timing,
animation and styling.

**Inputs / controls:** Target element clip; replacement library item, or an
image file from disk (imported into *My stickers* first, OV-020). Same entry
points as TL-035: context menu *Replace…* and `Mod+Alt` drag.

**Behavior / output:**
- Media overlays: exactly TL-035. The OV styling (border, corner radius,
  outline, shadow, clip frame, blend, placeholder mark) is part of the property
  bags TL-035 keeps.
- Element clips: keeps timing, transform, keyframes, opacity, blend,
  outline/shadow, CL colour stack (raster elements) and clip animations. The
  new item is fitted inside the old one's visible bounds (preserving its own
  aspect), so a wide logo replacing a square sticker does not distort.

**Edge cases:**
- Kind compatibility: sticker ↔ sticker ↔ emoji ↔ SVG/image item from the
  library; canvas frame ↔ canvas frame. Shapes, paths and lines change through
  their own parameters (OV-024, OV-027, OV-049) and charts through the chart
  editor (OV-051), not by replacement. Replacing an element
  with bin video is not offered, because the clip would have to change track
  kind (TL-004); the user deletes and adds instead.
- Animated ↔ static: an animated replacement starts with the clip's existing
  playback settings (OV-013); a static one keeps them stored but inactive.
- Tint (OV-015) on a replacement that is not tintable: kept stored, inactive.
- Replacing on a locked track: impossible (the clip is not selectable,
  TL-035).

**Undo:** TL's `ReplaceSource { clip, before, after, duration_change }`
(TL-035); element swaps have no duration change.

**Model notes:** TL's `plan_replace` (TL-035) accepts element content as well
as media; the fit-to-old-bounds adjustment is a pure function in
`freecut-timeline` folded into the same change set. Any probe of an imported
file runs in `freecut-media` off the UI thread before the op is planned.
Template slot filling (OV-032) uses TL-035 for media slots.

### OV-011 Still-image overlays with transparency — mvp

**What the user sees:** PNG logos, cut-outs and graphics placed on an overlay
track show the footage through their transparent parts, with clean, smooth
edges at any scale.

**Inputs / controls:** Imported still images. Import, supported formats, EXIF
orientation, large-image downscaling, colour-profile handling and bit depth
are PM-034 / PM-036 in [project-and-media.md](project-and-media.md); default
duration is PM-010. The alpha interpretation override (straight /
premultiplied / ignore) is PM-040. OV adds no controls.

**Behavior / output:** What OV requires of the compositor for still overlays:
- The decoded image's alpha is carried unchanged into compositing (FX-001 L2)
  and converted to premultiplied exactly once, on texture upload.
- Textures get mipmaps, so a large logo scaled down to a corner bug stays
  smooth instead of shimmering.
- Edges are filtered in premultiplied space, so soft-edged cut-outs show no
  dark or light fringe against the footage.

**Edge cases:**
- Palette PNG with a transparent index, grey+alpha PNG: expanded to RGBA with
  the transparency kept.
- Image with an alpha channel that is fully opaque: renders like an opaque
  image; no special casing.
- Image whose straight/premultiplied flag is wrong (fringes visible): fixed via
  PM-040, which updates every clip using the item.
- Very small overlays (scaled below 1/8 of source size): mip level selection
  avoids aliasing; no extra user control.
- Corrupt file: import fails (PM-037); a clip whose file later becomes
  unreadable renders as missing (OV-001, PM-090).

**Undo:** Same as OV-001 (TL ops); interpretation changes are PM-040's op.

**Model notes:** Decoding lives in `freecut-media` (PM-034). `freecut-render`
premultiplies on upload, builds mipmaps and treats every texture as
premultiplied afterwards; a golden-image test checks a soft-edged PNG over a
mid-grey and a saturated background for fringes.

### OV-012 Animated image overlays (GIF, APNG, animated WebP) — v1

**What the user sees:** Animated GIF, APNG and animated WebP files import as
moving clips (with transparency) instead of a frozen first frame. Their timeline
bar shows a filmstrip and a small "loop" marker at each repeat.

**Inputs / controls:** Import as media (PM-034) or as a sticker (OV-020).
Playback options are in OV-013.

**Behavior / output:**
- Each frame keeps its own display delay from the file (variable timing is
  preserved; frames are not resampled to a fixed rate).
- Frame for output time *t*: the frame whose start ≤ *t* in the animation's own
  timeline, after loop mapping.
- Natural length = sum of frame delays. Default clip length = natural length,
  but at least the default element duration (3 s) when looping.
- In a looping playback mode (OV-013; *Loop* is the default) the clip can be
  extended beyond the natural length and the animation repeats: for a media
  clip placed from the bin, TL-011's `SourceLimits` reports the media as
  *unbounded*, so TL-020 trims the clip like a still. In *Play once, then
  hide*, the one mode that does not loop, a media clip is capped at the
  natural length like a video, since nothing would show past the end. An
  animated image used as a sticker is an element clip, which TL-020 never caps
  (OV-013). Which frame shows at each clip time is OV-013's mapping.

**Edge cases:**
- Frame delay of 0 or ≤ 10 ms: treated as 100 ms, matching common browser
  behaviour (FreeCut default).
- A single-frame animated file: treated as a still.
- GIF disposal modes and APNG blend/dispose ops must be honoured by the
  decoder, otherwise ghosting appears; covered by decoder tests on fixtures we
  author ourselves.
- Very long or large animations (e.g. 2,000 frames at 1080p): stream-decode
  with an LRU frame cache (default budget 256 MB per asset) instead of holding
  all frames.
- Truncated/corrupt trailing frames: play up to the last good frame and warn.
- Splitting an animated clip (TL-023): the right half continues the
  animation phase where the left half ended, not from frame 1.
- Project frame rate lower than the animation's effective rate: frames are
  dropped, never blended.

**Undo:** TL ops for placing/trimming/splitting; OV-013 ops for options.

**Model notes:** `freecut-media` exposes `AnimatedImage { frames: Vec<FrameInfo
{ start: Rational, duration: Rational }> }` and frame decode by index; times are
converted to rational ticks at probe time. The timeline stores only the source
reference and in-point; it never stores decoded data. Source limits: TL-011's
`SourceLimits` provider returns *unbounded* for media whose OV-013 playback
mode loops (it asks OV-013's pure `ElementPlayback::loops()`), and TL-020 then
trims such clips like stills; for *Play once, then hide* it returns the
natural length. OV-013 owns the mapping from clip time to animation time.

### OV-013 Animated element loop & playback options — v1

**What the user sees:** For animated stickers and animated images, a *Playback*
section: loop mode, start offset and (for element clips) playback rate.

**Inputs / controls:**
- Loop mode: *Loop* (default), *Play once, hold last frame*, *Play once, then
  hide*, *Ping-pong* (forward then backward).
- Start offset: 0 to natural length, default 0 (lets two copies of the same
  sticker animate out of step).
- Rate:
  - Animated images placed from the media bin are media clips, so their speed,
    speed curves and reverse come from TR's time map (TR-036, TR-037); no
    separate control here.
  - Element clips have no TR time map (TR), so animated stickers get a simple
    constant *Rate* here: 0.25×–4×, default 1× (FreeCut default), stored as a
    rational because it converts one time into another (like TR's speed
    factors).
- *Apply to all animated elements*: a button under the section copies the
  selected clip's loop mode and rate (not its start offset, which exists to
  keep copies out of step) to every other animated element clip in the
  sequence. Animated media clips from the bin are not touched, since their
  rate is TR's. The same choice is offered once per batch when importing
  animated stickers (OV-020, OV-047), where it sets the library default that
  each newly placed copy starts with.

**Behavior / output:** This feature owns the mapping from clip time to
animation time. It maps clip-local time `t` to animation time, where `u` is
`t` after the TR time map (media clips) or `t·rate` (element clips):
loop → `(u + offset) mod L`; hold → `min(u + offset, start of last frame)`;
hide → transparent once `u + offset ≥ L`; ping-pong → triangle wave over `2L`.
The modulo is Euclidean, so source times below 0 (after a left-edge trim)
map into the animation too.

*Loop*, *Ping-pong* and *Play once, hold last frame* are the **looping
modes**: each gives a picture for any clip time (hold repeats its last frame
indefinitely). For media clips (animated images from the bin, and image
sequences once OV-017 lands), TL-011's `SourceLimits` reports a looping mode
as *unbounded* and TL-020 trims the clip like a still, at either edge.
*Play once, then hide* does not loop; such a clip is capped at the natural
length like a video. Element clips are generated content, which TL-020 never
caps, whatever the mode.

**Edge cases:**
- Media clips: speed curves and reverse (TR) apply before loop mapping;
  reversed ping-pong is still a valid triangle wave.
- Left-edge trim of a looping media clip: the source in-point moves with the
  edge as for any media clip (TL-020) and may go below 0 or past `L`; the
  frames that stay on screen do not change. In *Hold* mode, times before the
  first frame show the first frame.
- Natural length shorter than one project frame: shows a single frame, no
  flicker.
- Changing loop mode or rate does not change clip length. Switching a media
  clip that is already longer than its natural length to *Play once, then
  hide*: the clip keeps its length (no silent trim), the part past the end
  renders nothing and is shaded on the timeline, and TL-020 allows shortening
  it but not extending it further.
- This content loop is separate from FX-019's *Continuous* motion slot: an
  animated sticker can loop its own frames and also sway or pulse as a whole.
- *Apply to all animated elements* with clips on locked tracks: those clips
  are skipped and the toast says how many (TL-009); with no other animated
  element in the sequence the button is disabled.
- The reference app offers just two choices, loop or play once; FreeCut's
  four modes include both (*Loop*, *Play once, hold last frame*).

**Undo:** `SetElementPlayback { clip, old, new }`. *Apply to all animated
elements* is one compound step of `SetElementPlayback` ops (TL-043).

**Model notes:** `ElementPlayback { loop_mode: LoopMode, offset: Rational,
rate: Rational }` in `freecut-timeline` (`rate` used only by element clips),
with a pure `loops() -> bool` (true for Loop, Ping-pong and Hold) that TL-011's
`SourceLimits` provider consults. The mapping function is pure and
table-tested with rational times (no floats), including negative source times
after left trims.

### OV-014 Video overlays with alpha — v1

**What the user sees:** Video files that carry transparency (exported motion
graphics, lower thirds, animated overlays from other tools) show the footage
behind them automatically.

**Inputs / controls:** Supported on import when the decoded pixel format has
alpha: ProRes 4444 / 4444 XQ, QuickTime Animation (RLE), PNG-in-MOV,
VP8/VP9 WebM with alpha (format list owned by PM-036). Whether alpha is read
as straight or premultiplied, or ignored, is the per-media interpretation
override in PM-040 (default *Auto (from file)*); OV adds no separate toggle.

**Behavior / output:** Alpha is premultiplied once on upload and composites
like OV-001 / OV-011.

**Edge cases:**
- WebM VP8/VP9 alpha is stored as a side stream; it must be decoded with a
  decoder that reads it (the libvpx decoders in FFmpeg), otherwise the clip
  silently looks opaque. Probe must detect the alpha side data and pick the
  right decoder.
- HEVC with alpha: depends on whether the bundled FFmpeg build can decode the
  alpha layer; when it cannot, the probe reports "alpha not supported for this
  file" and the clip imports as opaque instead of failing silently.
- Alpha stream present but fully opaque: renders correctly; no special casing.
- Proxies (PM-080) for alpha sources must use an alpha-capable proxy codec,
  or transparency disappears while editing.
- Export with alpha, if EX offers it, must keep overlay transparency
  ([preview-and-export.md](preview-and-export.md)).

**Undo:** Placement as OV-001; alpha interpretation changes are PM-040's
`SetInterpretation`.

**Model notes:** Alpha detection and decoder selection in `freecut-media`
(PM-035 probe); `has_alpha` is part of the probed media info persisted by PM.

### OV-015 SVG vector overlays — v1

**What the user sees:** SVG files (logos, icons) can be added through *My
stickers* and placed like images, and they stay crisp at any size because they
are re-rasterised at the size they are shown. The same renderer draws the
bundled icon stickers (OV-019) and emoji (OV-021).

**Inputs / controls:** `.svg` / `.svgz` via sticker import (OV-020) in v1.
Importing SVG into the media bin is PM-034's call (currently *later*); when it
lands it uses this renderer. Optional *Tint* colour (replace all fill/stroke
colours with one colour, keep alpha) — useful for single-colour icons.
Default: no tint.

**Behavior / output:** Rendered at the clip's on-canvas size (after scale) into
a texture; re-rendered when the size changes by more than 25 % (size buckets),
so zooming in keeps edges sharp without re-rasterising every frame.

**Edge cases:**
- Static only: SVG scripts and SMIL animation are ignored.
- External references: linked images are resolved once at import, relative
  to the original SVG file, and embedded into the library copy; no network
  fetches ever; unresolved links render empty.
- Text inside SVG uses installed system fonts, so it can look different across
  machines; warn at import if the SVG contains text.
- Degenerate or missing `viewBox`/size: fall back to 512 × 512 user units.
- Raster cap: 8192 px on the long side.

**Undo:** Tint: `SetTint { clip, old, new }`; placement as OV-001.

**Model notes:** `freecut-media` reads the file and parses it into an SVG tree
(validation, size, text detection) off the UI thread; `freecut-render`
rasterises that tree at the requested size on a worker thread and keeps the
result in its texture cache. Library: the `resvg`/`usvg` crates (recent
releases are dual Apache-2.0 / MIT; confirm at adoption). The tint is a stored
clip property in `freecut-timeline`.

### OV-016 Lottie-style vector animations — later

**What the user sees:** Animated vector stickers from Lottie JSON / `.lottie`
files play smoothly at any size, with transparency.

**Inputs / controls:** Import `.json` (Lottie) and `.lottie` (zipped) files.
Same playback options as OV-013.

**Behavior / output:** Rendered per frame on the GPU at the on-canvas size.
Natural length and frame rate come from the file; timing converted to rational
ticks.

**Edge cases:**
- Unsupported features (expressions, embedded text layers, some image layers):
  import succeeds with a warning listing what will not render.
- Files referencing external images: resolved relative to the file only.
- Very complex files that cannot render in real time: preview drops frames;
  export renders every frame.

**Undo:** As OV-012/OV-013.

**Model notes:** Candidate renderer: `velato` on `vello` (wgpu-based;
Apache-2.0 OR MIT, verify at adoption) inside `freecut-render`; alternative `rlottie` (mostly MIT,
mixed component licences — needs review). Parsing/validation in
`freecut-media`.

### OV-017 Looping image-sequence overlays — later

**What the user sees:** A folder of numbered PNG frames (e.g. `spark_0001.png …`)
already imports as one media item with transparency (PM-033, v1) and works as
an ordinary overlay from then on (OV-001). This feature adds the animated-image
playback options to such sequences, so a short particle or sparkle loop can
repeat, ping-pong or hold its last frame across a longer clip.

**Inputs / controls:** Sequence detection, frame rate prompt and the gap rule
are PM-033 in [project-and-media.md](project-and-media.md); loop mode and start
offset from OV-013.

**Behavior / output:** Behaves like OV-012 with a constant frame duration
(1 / sequence frame rate, rational). In a looping mode (OV-013) TL-011's
`SourceLimits` reports the sequence as *unbounded* and TL-020 trims it like a
still. Without this feature, a sequence clip cannot be longer than its frame
count allows, like any video.

**Edge cases:** Gaps in numbering hold the previous frame (PM-033's rule);
mixed frame sizes are fitted to the first frame's size; thousands of frames
stream through the same LRU cache as OV-012; a proxied sequence (PM-080) loops
the proxy in preview and the original frames in export.

**Undo:** TL ops for placing/trimming; `SetElementPlayback` (OV-013) for loop
options.

**Model notes:** Reuses the `AnimatedImage` interface in `freecut-media`, and
the same `SourceLimits` rule as OV-012 (unbounded when OV-013's `loops()` is
true); the time mapping is OV-013's.

### OV-018 Sticker clips — v1

**What the user sees:** Clicking *+* on a sticker in the library (or dragging it
to the timeline/preview) adds it at the playhead, centred on the canvas, on a
`Sticker` track (TL-004) with free space at that time, or a new one (TL-012
overlay rule). It shows on the timeline as a clip with a distinct colour and
the sticker's thumbnail, and can be moved, trimmed, split, duplicated,
transformed and animated like any visual clip.

**Inputs / controls:**
- Default duration 3 s (TL-011's sticker default).
- Default size: longest side = 30 % of the canvas short side (FreeCut default).
- Placement: dragging into the preview drops it centred at the cursor;
  *+* places it centred.
- Inspector: transform/opacity (TR), blend (OV-004), outline/shadow (OV-006),
  playback (OV-013, animated only), tint (single-colour/icon stickers only),
  flip H/V (TR), filters and adjustments (CL; raster stickers only — vector
  shapes have no colour stack), and FX's clip-animation slots — Entrance,
  Exit and Continuous (FX-018, FX-019), whose applicability covers element
  clips. Clip effects are not offered on stickers in v1; an effect layer above
  them is used instead (FX).
- Animation slot policy: element clips (stickers, emoji, shapes, paths,
  lines, frames, charts) use the **exclusive** policy that FX-018/FX-019 define for video,
  image and element clips: choosing a Continuous preset clears Entrance and
  Exit, and choosing Entrance or Exit clears Continuous. The coexisting
  policy, where a Loop runs between In and Out, is for text clips only
  (TX-022). OV adds no slot rule of its own.

**Behavior / output:** A sticker clip references a library asset, not a bin
media item; it renders exactly like an image/animated-image clip with alpha.
Using a sticker does not add it to the media bin.

**Edge cases:**
- Asset missing on open (pack removed, user library cleared): placeholder
  drawn at the clip's bounds with a "missing sticker" badge; the clip keeps all
  settings and relinks if the asset reappears (matched by content hash) or the
  user picks a replacement (OV-010).
- Every candidate `Sticker` track locked or occupied at the playhead: a new
  track is created (TL-012 rule).
- Adding at the very end of the timeline: allowed; extends the project.
- Duplicating many stickers: each duplicate shares the decoded texture (cache
  keyed by asset hash), so memory does not grow per copy.

**Undo:** TL's `InsertClips` with element content (TL-012) — undo removes it.
Animation choices are FX's `SetClipAnimation` (FX-018), which carries any slot
cleared by the exclusive policy in the same step.

**Model notes:** Element clips store FX's `ClipAnimations` (FX-018), and FX's
slot logic reads the exclusive policy from the clip kind; element animations
are evaluated by FX's `animation_offsets` and enter FX-001 at L8. Element clips
use TL-011's generated content,
`ClipContent::Generated { kind }`, with an element kind carrying
`ElementSource::{Sticker { asset: AssetRef }, Emoji {..}, Shape(..), Line(..),
Frame {..}, Chart(..)}` defined in `freecut-timeline` (custom paths, OV-049,
are a `ShapeKind` inside `Shape`; charts are OV-051).
`AssetRef { pack: PackId, id: AssetId, hash: ContentHash }` is a plain
identifier — no file I/O in the timeline. `freecut-project` serializes it;
`freecut-app` resolves it to a file through the library index.

### OV-019 Bundled sticker packs — v1

**What the user sees:** A Stickers tab with ready-to-use packs grouped by
category, available offline from first launch.

**Inputs / controls:** Categories (FreeCut's own taxonomy): Arrows & pointers,
Badges & labels, Speech & thought bubbles, Hand-drawn doodles, Celebration
(confetti, sparkles, balloons), Nature & weather, Reactions & symbols (hearts,
stars, check marks, question marks), Generic UI-style buttons (like / follow /
subscribe styles with no platform branding), Icons (tintable), Seasons &
holidays (generic seasonal motifs, no religious or commercial mascots),
Countdowns & progress bars (animated 3-2-1 counters, loading and progress
bars), Emphasis marks (hand-drawn circles, underlines, highlighter strokes,
"look here" bursts), Pets & animals, Vlog & travel decorations (tape, stamps,
tickets, location pins), and Lettering (short generic words in decorative
type, drawn from open fonts). Each category mixes static and animated items;
a filter chip shows only *Animated* or only *Static* items (OV-036). The
taxonomy aims at roughly 15–25 categories (FreeCut default); the reference
app ships about 22.

**Behavior / output:** Packs install with the app (read-only), each with a
manifest carrying licence and attribution data (OV-040). Animated items ship as
APNG or animated WebP; icons as SVG. Community packs the user adds (OV-040)
appear as further categories, labelled with the pack's author, so there is no
separate "creator stickers" area. Bundled packs never expire, are never
withdrawn from existing projects and need no download on first use.

**Edge cases:**
- App update changes a pack: released assets are immutable; changes ship as new
  asset IDs, so old projects never change appearance. Retired assets stay
  available in a hidden "legacy" pack.
- A pack fails validation at startup (corrupt file): that pack is skipped,
  others load, and the issue is logged.
- First launch on a slow disk: library index loads asynchronously; the tab
  shows a spinner, the rest of the app is usable.
- Countdown and progress-bar stickers are fixed-length animations; stretching
  the clip loops or holds them per OV-013 rather than slowing them, so a
  10-second countdown needs its own asset or a rate change (OV-013 *Rate*).

**Undo:** n/a (shipping content).

**Model notes:** Packs live in the app's resource directory; the index is built
by `freecut-app` on a worker thread. Asset decoding goes through
`freecut-media`. Asset sourcing rules are in *Clean-room notes*.

### OV-020 Import your own stickers ("My stickers") — v1

**What the user sees:** A *My stickers* category with an *Import…* button and
drop zone. Imported files appear in the library from then on, in every project.
A right-click on any image/animated clip in the timeline offers *Save as
sticker*.

**Inputs / controls:**
- Formats: PNG, JPEG, WebP, GIF, APNG, SVG; Lottie JSON when OV-016 lands. A
  folder of numbered frames can become one animated sticker (OV-047).
- Optional name and tags at import (default name = file name without
  extension). Name length 1–80 characters (FreeCut default, as OV-030);
  names need not be unique.
- *Trim transparent edges* (default on): fully transparent rows and columns
  around the picture are cut off in the library copy, so the sticker's bounds
  match what is visible (better placement, OV-005 borders and OV-008 picking).
  For animated files the union of all frames' visible areas is kept; SVG
  files keep their own view box (raster images only).
- *Padding* 0–25 % of the longer side (default 0 %): transparent margin added
  around the trimmed picture, for stickers that get a thick OV-006 outline.
- Multi-file and folder import; a counter shows progress ("12 of 15") for
  batches.
- Managing: rename (`F2` or right-click), multi-select (`Ctrl`/`Shift`+click)
  and *Delete* for several stickers at once, edit tags.

**Behavior / output:** Imported files are copied into the user library folder
(`<user data dir>/library/stickers/`), named by content hash (SHA-256, the
same hash CL uses for LUTs; PM-042 reads it), with a small sidecar entry in
the library index (name, tags, source file name, import date, licence
"user-provided"). Copying means moving or deleting the original file later
does not break anything. The source keeps its own aspect ratio and pixel size
(up to the cap below); FreeCut does not force stickers onto a square canvas.
Alternative seen in the reference app: every sticker is normalised onto a
square canvas of about 800 × 800 px, with transparent padding for stills and
none for GIFs; FreeCut's *Trim* and *Padding* options cover the useful part
of that.

**Edge cases:**
- Duplicate import (same content hash): not copied again; the existing entry is
  shown and optionally renamed.
- Files over 50 MB or larger than 8192 px on a side: rejected with the reason
  (FreeCut default limits). The reference app is stricter (about 20 MB per
  file and roughly 800 px per side, minimum about 360 px); FreeCut keeps its
  larger caps because stickers are also used as full-screen logos.
- Smaller than 32 px on the short side (after trimming): rejected as too
  small to be useful; below 256 px the import succeeds with a "may look soft
  when enlarged" note (FreeCut defaults).
- A fully transparent image: nothing is left after trimming, so it is
  rejected with that reason.
- JPEG (no alpha channel): imported, with a note that it will show as a
  rectangle; *Trim transparent edges* does nothing for it.
- Unsupported or corrupt file: skipped; a summary lists which files failed and
  why, with *Retry failed* (useful after fixing a file or freeing disk space).
- Empty *My stickers*: the category shows an empty state with the *Import…*
  button and the drop zone.
- Rename to an empty name: refused, the old name stays; over 80 characters:
  the field stops accepting input.
- Deleting user stickers (one or several) that are used in open projects:
  allowed after one confirmation naming the number of stickers and the count
  of uses in open projects; those clips show as missing (OV-018). Deletion
  moves the files to the OS trash, not a hard delete, so it can be recovered
  outside FreeCut (the reference app deletes permanently).
- Opening a project on another machine without that sticker: missing-asset
  placeholder; user sticker files are tracked as referenced assets (PM-042),
  so *Collect & package project* (PM-101) copies the used ones into the
  package to avoid this.
- Import runs off the UI thread; progress is shown for large batches.

**Undo:** Library changes are not project edits and are not on the project undo
stack. *Save as sticker* from a clip does not change the project.

**Model notes:** Library storage and index in `freecut-app` (async file I/O);
format probing, edge trimming and padding (re-encoding the library copy, for
animated files frame by frame) in `freecut-media` on a worker thread; the
visible-bounds computation is a pure function with table tests. Referenced
from projects via `AssetRef` with pack = `user`.

### OV-021 Emoji elements — v1

**What the user sees:** An Emoji tab with the full emoji set, grouped by the
standard Unicode categories (smileys, people, animals & nature, food, travel,
activities, objects, symbols, flags), a search box, and a skin-tone selector.
Clicking an emoji adds it as an element clip, crisp at any size.

**Inputs / controls:**
- Skin tone: default (yellow) + 5 tones; remembered per user.
- Default size: 20 % of the canvas short side; default duration 3 s.
- Same inspector as stickers (OV-018).

**Behavior / output:** Emoji are drawn from a bundled open-licensed vector emoji
set (see *Clean-room notes*) and re-rasterised at display size like SVG
(OV-015). Search uses the open Unicode CLDR emoji names and keywords in the UI
language, with English fallback.

**Edge cases:**
- Multi-codepoint sequences (ZWJ families, flags, keycaps, tone modifiers) are
  stored as the full sequence string and must render as one glyph.
- Sequence not present in the bundled set version (e.g. a newer Unicode emoji
  pasted in): render the components that exist, warn once.
- Changing the emoji set in preferences does not change existing clips: each
  clip records the set it was made with; only new clips use the new set.
- Emoji inside text titles are a text feature ([text-and-captions.md](text-and-captions.md));
  this feature is the standalone emoji element.

**Undo:** `InsertClips` (TL-012) with element content; changing the emoji of
an existing clip is TL's `ReplaceSource` (OV-010).

**Model notes:** `ElementSource::Emoji { sequence: String, set: EmojiSetId }`
in `freecut-timeline`. Glyph lookup and rasterisation in `freecut-render`
(SVG path, OV-015). CLDR annotation data bundled with the app.

### OV-022 Arrow tips that follow a tracked object — later

**What the user sees:** Making a sticker, emoji, shape or frame follow a moving
subject is AI-008 (*Tracking* in the inspector, owned by
[ai-tools.md](ai-tools.md)); element clips get it like any other overlay. OV
adds one element-specific option: a line or arrow (OV-027) can follow the
subject with only one endpoint, so the arrow tip stays on the moving subject
while its tail stays where the user put it.

**Inputs / controls:** The tracking run, its options and defaults: AI-008. For
line/arrow elements only, an *Attach* choice: whole element (default, plain
AI-008 behaviour) / tip only / tail only.

**Behavior / output:**
- Whole element: exactly AI-008 (binding kept in source time, offset taken at
  the reference frame).
- Tip only / tail only: the chosen endpoint = tracked position + the offset
  captured at the reference frame; the other endpoint keeps its own (possibly
  keyframed) value; an optional curve control point keeps its position
  relative to the two endpoints.
- Outline and shadow (OV-006) are recomputed from the moved element each frame,
  so the shadow keeps its canvas-space angle.

**Edge cases:**
- Target lost, tracked clip moved or deleted, element longer than the tracked
  range: AI-008's rules apply to the bound endpoint.
- Bound endpoint dragged onto the free one: zero-length rule of OV-027.
- *Bake to keyframes* (AI-008) writes keyframes for the bound endpoint only.
- Changing *Attach* after tracking keeps the track data; only the binding
  target changes.

**Undo:** AI-008's ops for tracking, correction, baking and removal; switching
*Attach* is `SetTrackAttach { clip, old, new }`.

**Model notes:** The binding and track data are AI-008's types in
`freecut-timeline`; OV adds only `attach: TrackAttach::{Whole, EndpointA,
EndpointB}` on line elements. Evaluation is a pure function in
`freecut-timeline`; keyframes are TR's model.

### OV-023 Make a sticker from a cut-out — later

**What the user sees:** On an image or video frame, *Make sticker*: the subject
is cut out (automatic background removal, AI-014, or the clip's current mask),
previewed with an optional outline, and saved to *My stickers*.

**Inputs / controls:** Source clip + frame (playhead), cut-out method (automatic
/ use current mask), outline (OV-006 values baked in or kept live).

**Behavior / output:** Writes a PNG with alpha into the user library (OV-020).
Does not modify the project unless the user also ticks *Add on top*, which
inserts the new sticker as an element clip at the playhead, aligned with the
subject, on a `Sticker` track.

**Edge cases:** No subject detected (fall back to manual mask); very small
result (< 32 px): refuse; segmentation model not installed (AI area): offer the
mask path only; source frame not yet decoded: wait off the UI thread with
progress.

**Undo:** The library change is not undoable (OV-020); *Add on top* is one
`InsertClips` step (TL-012).

**Model notes:** Segmentation via the local open-source model managed by
[ai-tools.md](ai-tools.md) (AI-014); the mask path uses TR masks; frame decode
and PNG encoding in `freecut-media`; library update in `freecut-app` on a
worker thread.

### OV-024 Shape elements — v1

**What the user sees:** A Shapes tab with: rectangle, rounded rectangle, ellipse
(circle), triangle, regular polygon, star, heart, and a ring (donut). Adding
one places it centred; it is edited like any element and stays sharp at any
size. Each shape is its own clip on a `Sticker` track. Free-form shapes drawn
with a pen are OV-049.

**Inputs / controls:**
- Width / height in canvas units (via transform handles; aspect unlocked by
  default for shapes). Default 30 % of the canvas short side, square.
- Rounded rectangle corner radius 0–50 % of short side (default 15 %).
  *Independent corners* toggle (default off; rectangles and rounded
  rectangles only) splits it into four radii — top-left, top-right,
  bottom-right, bottom-left — each with the same range, each keyframable.
  Opacity is TR-015's, as for every clip.
- Polygon sides 3–12 (default 6). Star points 3–24 (default 5), inner radius
  10–90 % (default 40 %). Ring thickness 5–95 % of radius (default 25 %).
- Default duration 3 s.

**Behavior / output:** Shapes are drawn analytically in the GPU (signed distance
fields) with anti-aliased edges; no bitmap involved. Shape parameters can be
keyframed (TR keyframe system) — e.g. animating star inner radius.

**Edge cases:**
- Width or height 0: allowed in the data (keyframe passing through 0) and
  renders nothing; handles prevent dragging below 1 px.
- Polygon/star with extreme ratios (very thin): edges stay anti-aliased; stroke
  joins may spike — miter limit 4 (FreeCut default) falls back to bevel.
- Non-uniform scale on circles yields ellipses; corner radius stays circular in
  canvas space.
- Turning *Independent corners* on while the single radius has keyframes:
  its keyframes are copied to all four corners, so nothing changes visually.
  Turning it off keeps the top-left corner's value and keyframes for the
  single radius and drops the other three (one undo step restores them).
  Alternative seen in the reference app: per-corner radii are simply
  unavailable while the radius has keyframes; FreeCut converts instead of
  blocking.
- Two adjacent corner radii that add up to more than the side between them:
  both are scaled down in proportion so the corners meet without overlapping
  (the CSS border-radius rule).
- The shape itself has no separate blur control; a soft look comes from the
  OV-006 shadow's blur or a blur effect (FX).

**Undo:** `InsertClips` (shape content, TL-012); `SetShapeSpec { clip, old, new }` for
parameter edits, including the *Independent corners* switch.

**Model notes:** `ShapeSpec { kind: ShapeKind, size: (f64, f64), params,
fill: Fill, stroke: Option<Stroke> }` in `freecut-timeline` (`ShapeKind` enum
with per-kind params; size as fractions of the canvas short side, stored as
`f64`, OV's own non-time type). Keyframable parameters are
`Animatable<f64>` on TR-027's generic engine. SDF evaluation in a WGSL shader
in `freecut-render`; the same SDF code used for OV-005 corners. CPU SDF
reference in tests.

### OV-025 Shape fill and stroke — v1

**What the user sees:** For a selected shape: *Fill* (on/off, colour with
alpha) and *Stroke* (on/off, colour, width, position, dash style). Outline-only
shapes are made by turning fill off.

**Inputs / controls:**
- Fill: on by default, white. Fill type *Solid* in v1; *Gradient* (OV-026)
  and *Image* (OV-050) join the same dropdown later. *Fill opacity* 0–100 %
  (default 100 %) applies to every fill type.
- Stroke: off by default; width 1–100 px@1080 (default 6); colour; *Stroke
  opacity* 0–100 % (default 100 %); position inside / centre / outside
  (default centre); dash: solid / dashed / dotted (dash length = 3× width,
  gap = 2× width, FreeCut default); joins round. End caps and arrowheads
  apply only to open paths (OV-027 lines, OV-049 open paths).
- Drop shadow and outline come from OV-006 (colour, opacity, angle, distance,
  blur), shown in the same inspector.
- Eyedropper sampling a colour from the preview.

**Behavior / output:** Stroke follows the shape geometry exactly, including star
points and rounded corners. Fill colour, stroke colour, both opacities and
stroke width are keyframable through TR-027; colours interpolate in FX-001's
working space.

**Edge cases:**
- Fill off and stroke off: shape is invisible; inspector shows a hint, the clip
  stays selectable via bounds.
- Stroke wider than the shape with "inside": clamps to fill the shape.
- Colour alpha in fill combined with fill opacity and clip opacity multiplies.
- Colour keyframes need a colour to animate: they exist for solid fills and
  gradient stops (OV-026); an image fill (OV-050) has none, so switching a
  fill with colour keyframes to *Image* keeps the keyframes stored but
  inactive, and switching back restores them. Its fill opacity stays
  keyframable.
- Several shapes selected with different values: the field shows UI-010's
  mixed state ("—"); typing sets every shape, scrubbing adds the same delta.

**Undo:** Part of `SetShapeSpec`.

**Model notes:** `Fill::{None, Solid(Rgba8)}` (gradients added by OV-026,
images by OV-050) plus `fill_opacity: f64` next to it,
`Stroke { width: ShortSideFrac, color: Rgba8, opacity: f64, align, dash:
DashStyle }`. The keyframable ones are `Animatable<…>` (TR-027); OV implements
`Interpolate` for `Rgba8` in the working space.

### OV-026 Shape gradient fills — later

**What the user sees:** Fill type *Linear gradient* or *Radial gradient* with
two to five colour stops and an angle.

**Inputs / controls:** Stops (colour + position 0–100 %), angle 0–359° for
linear (default 90°, top to bottom), centre and radius for radial.

**Behavior / output:** Gradient is evaluated in the shape's local space, so it
rotates and scales with the shape. Interpolation runs in the same working
space as blending (fixed by FX-001, L2).

**Edge cases:** Single stop → solid; stops at identical positions → hard edge;
banding on 8-bit export mitigated with ordered dithering. Stop colours and the
angle are keyframable (TR-027); adding or removing a stop applies to every
keyframe of that fill (one undo step), so all keyframes keep the same stop
count. *Fill opacity* (OV-025) applies on top.

**Undo:** Part of `SetShapeSpec`.

**Model notes:** `Fill::Linear { stops, angle } | Fill::Radial { stops, center,
radius }`.

### OV-027 Lines and arrows — v1

**What the user sees:** A *Line* and an *Arrow* in the Shapes tab. Endpoints are
dragged directly in the preview; useful for tutorials and callouts.

**Inputs / controls:**
- Thickness 1–100 px@1080 (default 8); colour (default white).
- Arrowheads: none / end / start / both (Arrow default: end); head style:
  filled triangle / open chevron / dot; head size 1–10× thickness (default 3×).
- Dash: solid / dashed / dotted; caps: butt / round (default round).
- Optional curve: one control point for a quadratic bend (default straight).

**Behavior / output:** Drawn as an SDF stroke; endpoint positions are clip
properties (keyframeable through TR's keyframe model) in TR's canvas space, so
the arrow can be animated pointing at moving things (or bound to a tracked
object, OV-022). A "draw-on" reveal is provided by FX clip animations
using a 0–100 % path-progress property exposed by this element.

**Edge cases:** Endpoints equal (zero length) renders a dot with round caps or
nothing with butt caps; endpoints off-canvas are allowed; head size larger than
the line length is scaled down to fit.

**Undo:** `InsertClips` (TL-012); `SetShapeSpec` for edits, including endpoint drags (one
step per drag).

**Model notes:** `ElementSource::Line(LineSpec { a, b, control: Option<Point>,
thickness, heads, dash, cap, color })` with points stored as `f64` fractions
of canvas width/height (TR's canvas-space convention). Endpoints, control
point and path progress are `Animatable<…>` (TR-027) over OV's own value
types.

### OV-028 Speech-bubble and callout shapes — later

**What the user sees:** Bubble shapes (rounded, oval, cloud-like, rectangular)
with a movable tail, which can contain a text element that wraps inside it.

**Inputs / controls:** Bubble kind, tail tip position (drag in preview), tail
width, padding for contained text.

**Behavior / output:** The bubble resizes to fit its linked text box
(text layout owned by [text-and-captions.md](text-and-captions.md)).

**Edge cases:** Text removed (bubble keeps last size); tail tip inside the body
(tail hidden).

**Undo:** `SetShapeSpec`; linking the text clip uses TL's clip link (TL-011
`link`), as its own undoable op.

**Model notes:** Another `ShapeKind` with an optional link to a text clip ID;
the fit-to-text size is a pure function in `freecut-timeline` fed by the text
bounds TX computes. The bubble kinds, tail parameters and outline geometry are
TX-055's (`BubbleSpec`, `bubble_path` in `freecut-render::shape::bubble`), so a
stand-alone bubble and a bubble drawn as part of a text clip's style look the
same. For text that sits in a bubble, TX-055 is the everyday path (the bubble
is part of the text clip and needs no link); this feature covers bubbles
without text, or one bubble shared by several text clips.

### OV-029 Decorative frames and borders — v1

**What the user sees:** A Frames tab with decorative frames in two kinds:
**Canvas frames** cover the whole picture (film-strip edge, rounded vignette,
hand-drawn border, instant-photo border) and **Clip frames** wrap a single
clip (a PiP gets an instant-photo-style or taped-photo border that moves with it).

**Inputs / controls:**
- Canvas frame: added like a sticker; default duration = from playhead to
  project end (OV-009 rule). Opacity, tint (if the frame is marked tintable).
- Clip frame: drag onto a clip in the timeline or preview, or pick in that
  clip's inspector *Frame* dropdown; *Remove frame*. Thickness scale 50–200 %
  (default 100 %).

**Behavior / output:** Frames are 9-slice images: corners keep their aspect,
edges stretch or tile, the centre is transparent. So one asset fits 16:9, 9:16,
1:1 and any clip shape without distortion. A clip frame is element styling: it
is drawn at FX-001's L8b stage, in canvas space after the clip's transform,
outside its visible bounds, and last in L8b's order (shadow, outline,
content, border, clip frame). It is therefore part of the clip's layer, faded
by the clip's opacity (L9) and blended once with it (L10, OV-004).

**Edge cases:**
- Canvas aspect changes after placing a canvas frame: frame re-fits
  automatically (it is defined against the canvas, not fixed pixels).
- Clip smaller than the frame's corners: corners scale down uniformly to fit.
- Clip with rotation: frame rotates with it.
- Clip frame plus border (OV-005): the frame is laid out around the border's
  outer edge and drawn after it (L8b order), so both stay visible.
- A user-imported plain PNG used as a canvas frame (no 9-slice data): it is
  stretched to the canvas; the import dialog offers to set slice insets.

**Undo:** Canvas frame: `InsertClips` (TL-012). Clip frame: `SetClipFrame { clip, old,
new }`.

**Model notes:** `ElementSource::Frame { asset, slices: NineSlice }` for canvas
frames; `clip_frame: Option<FrameRef>` on the clip's visual style for clip
frames. `NineSlice { left, right, top, bottom: u32 (source px), edge_mode:
Stretch | Tile }` comes from the pack manifest. Canvas frames render like any
element clip; clip frames render in `freecut-render`'s L8b pass.

### OV-030 Save project as template — v1

**What the user sees:** *File → Save as template…* turns the current project
into a reusable starting point: a dialog asks for a name, description, which
clips are replaceable (pre-filled from OV-031 marks), and a cover frame.

**Inputs / controls:** Name (required, ≤ 80 chars), description (optional),
cover frame (default: frame at one third of the duration), include non-slot
media (always on: needed for a self-contained template).

**Behavior / output:** Writes a template bundle into the user templates folder:
the project document (current format version), copies of every non-placeholder
media file and user sticker it references, the cover image, and a small
manifest (name, description, canvas size, fps, duration, slot list, app
version). The original project is unchanged. Bundled pack assets are referenced,
not copied.

**Edge cases:**
- Missing media in the project: the save dialog lists them; the user can
  continue (those clips stay missing in the template) or cancel.
- Bundle larger than 500 MB: warning with the size before writing.
- Name clash: offer replace or keep both ("Name (2)").
- Fonts used by text clips are not copied (font licences); the manifest lists
  them so a machine without the font shows a substitution warning (TX).
- The user's own media (including music) is copied as-is; FreeCut does not
  judge the licence of files the user supplies.
- Write failure halfway (disk full): the partially written bundle is removed;
  no half-template appears in the list.
- Saving happens on a worker thread with progress; the user can keep editing.

**Undo:** n/a (writes a file; does not edit the project).

**Model notes:** Template manifest and bundle layout defined in
`freecut-project` (versioned like projects, with migrations). Copy/write in
`freecut-app` worker tasks.

### OV-031 Replaceable placeholder slots — v1

**What the user sees:** Right-click any video, image or audio clip → *Mark as
replaceable*.
The clip gets a slot badge with a number (order of filling) and an optional
label ("Your selfie", "Product shot"). A *Slots* list shows all slots in order.

**Inputs / controls:**
- Label: free text, optional.
- Accepts: video and image (default) / image only / video only.
- Fit: *Fill* (crop to cover the slot's visible bounds, default) or *Fit*
  (letterbox inside).
- Duration rule: *Keep slot length* (default) or *Use media length* (ripples
  later clips per TL's ripple rules, TL-005 / TL-021).
- Slot order: follows timeline order (start time, then track bottom to top);
  can be reordered by drag in the Slots list.
- Text clips can be marked *Editable text* with a label (the text content is the
  slot); other text stays fixed.
- Audio clips (a music bed, a voice line) can be marked *Replaceable audio*
  with a label; such a slot accepts audio files and the audio of video files.
- *Sound of the filled media* (video slots only): *Muted* / *Kept* / *Filler
  decides* (default: *Filler decides*, starting muted). It controls whether
  the embedded audio of the video someone drops into the slot is heard, by
  setting that clip's mute (AU-007).
- *Count* (FreeCut default *Fixed*): *Fixed* / *Optional* (the person filling
  the template may remove the slot) / *Repeatable up to N* (N 2–20, default
  5; the slot can be duplicated in place to take more clips). This is how a
  template allows a different number of clips.

**Behavior / output:** Marks are stored in the project, so a project can be
prepared as a template over several sessions. Non-slot clips are locked against
replacement in the fill flow, and non-slot text shows a lock badge there, but
everything stays editable in the normal editor. The timeline shows a slot
badge on every marked clip so the author can see at a glance which parts are
replaceable and which are fixed.

**Edge cases:**
- Marking an element/sticker/shape clip: not allowed (only media, text and
  audio); menu item disabled.
- Two slots sharing the same media file: still separate slots.
- Slot clip with keyframes, mask, effects, animations: all kept and re-applied
  to the new media relative to clip start.
- Deleting a slot clip renumbers the remaining slots.
- *Optional* or *Repeatable* on a slot that is not on the main track: removing
  or repeating it does not ripple anything (overlays keep their own times);
  on the main track it ripples per TL-005.
- *Sound of the filled media* on an image-only slot: hidden (images have no
  sound).
- Audio slot with *Use media length*: later audio clips on the same track
  ripple, other tracks do not (TL-021 rules).

**Undo:** `SetPlaceholder { clip, old: Option<PlaceholderSpec>, new }`;
reordering is `ReorderSlots { old, new }`.

**Model notes:** `PlaceholderSpec { label: String, accepts: MediaKindMask, fit:
SlotFit, duration_rule: SlotDuration, sound: SlotSound, count: SlotCount,
order: u32 }` as an optional property bag on the clip (TL-011) in
`freecut-timeline`; `SlotCount::{Fixed, Optional, Repeatable { max: u8 }}`,
`SlotSound::{Muted, Kept, FillerDecides}`. `MediaKindMask` gains an audio bit
for audio slots.

### OV-032 Start a project from a template and fill its slots — v1

**What the user sees:** On the start screen and in *File → New from template*,
a grid of the user's templates with covers (hover plays a short preview if one
was saved). Each card states how many clips the template needs ("6 clips,
2 texts"). *Preview* plays the template with its own sample media in the
player before anything is created. Choosing one creates a new, untitled
project and opens a *Fill slots* strip: one tile per slot, in order, each
showing its label and the sample media. The user clicks any tile, in any
order, and picks a file, or drops files onto tiles, or uses *Fill all…* to
choose several files at once. Below the tiles come the editable texts and the
replaceable audio, if the template has any.

**Inputs / controls:** Template; media per slot; per slot: re-frame (pan/zoom
inside the slot when Fill crops), choose in-point for videos longer than the
slot, *Use clip sound* toggle (slots whose sound rule is *Filler decides*,
OV-031; default off), *Remove* (optional slots) and *Add another* (repeatable
slots); editable text fields; a file per replaceable-audio slot.

**Behavior / output:**
- New project inherits canvas size, fps, all tracks, effects, music and fixed
  media from the template; the template itself is never modified.
- Each fill is a TL-035 replace, followed by the slot's fit and duration rules
  in the same transaction.
- *Fill all*: assigns selected files to unfilled slots in slot order, skipping
  slots whose *Accepts* rule the file does not meet.
- *Remove* on an optional slot deletes that slot clip (ripple delete on the
  main track, TL-025); *Add another* on a repeatable slot duplicates the slot
  clip right after itself with all its styling and slot settings (TL-033's
  duplicate, rippling the main track) until the slot's maximum is reached;
  the copy becomes an unfilled tile and can itself be removed again.
- Replaceable audio is a TL-035 replace on the audio clip; *Keep slot
  length* trims longer audio to the slot and leaves silence after shorter
  audio (no looping), keeping the clip's fades (AU-006).
- The full editor remains available at any time; the strip is just a shortcut.
  Nothing is locked: the project can be edited freely before every slot is
  filled, and every other feature (captions, export, …) works on it as on any
  project. Alternative seen in the reference app: some features stay blocked
  until every sample clip is replaced, and keyframed templates must be
  unlocked before free editing; FreeCut does not gate anything.

**Edge cases:**
- More files than slots: extras are imported into the bin but not placed; the
  user is told how many.
- Fewer files: remaining slots keep sample media and stay marked *unfilled*;
  export warns once if any slot is unfilled.
- Video shorter than a *Keep slot length* slot: instead of TL-035's shortening,
  the remainder is filled by holding the last frame (a TL-031 freeze in the same
  transaction; FreeCut default); the tile shows a "held frame" badge.
- Image into a slot: fills the slot length.
- Template made with an older format version: migrated on open (PM-021).
  Made with a newer FreeCut: refused with a message naming the required version.
- Template's fixed media missing from the bundle: missing-media placeholders
  and relink, as for any project.
- Fonts missing: substitution warning from TX.
- A slot clip deleted in the full editor: its tile shows "removed from the
  timeline" and is skipped by *Fill all* (undo restores the clip and the
  tile); a text slot whose clip was deleted is hidden from the text list the
  same way.
- Template with no editable text or no replaceable audio: those sections are
  replaced by a one-line note ("This template has no editable text").
- *Add another* at the slot's maximum: disabled with a tooltip naming the
  maximum; *Remove* on the last remaining copy of a repeatable (not optional)
  slot: disabled.
- *Use clip sound* on a slot filled with an image, or with a video that has no
  audio: hidden.

**Undo:** Creating the project is not an undoable edit (new document). Each slot
fill is TL's `ReplaceSource` (TL-035); *Fill all* is one compound step.
*Remove* is TL-025's delete, *Add another* TL-033's duplicate, *Use clip
sound* AU-007's `SetClipMuted`; each is one step.

**Model notes:** Instantiation (copying the bundle's project into a new project
document, re-rooting media paths) in `freecut-project`; fill ops in
`freecut-timeline`; file copies and probes off the UI thread in `freecut-app` /
`freecut-media`.

### OV-033 Template library management — v1

**What the user sees:** A Templates page listing the user's templates with
cover, name, canvas shape (16:9 / 9:16 / 1:1 …), duration, slot count and
how many projects were started from it on this computer ("used 4 times");
actions: rename, edit description, duplicate, delete, *Open template for
editing*, *Show in file manager*, favorite (OV-038); filters by aspect and a
name search.

**Inputs / controls:** As listed; sort by name / last used / most used / date
created. Use counts are local and are never sent anywhere.

**Behavior / output:** *Open template for editing* opens the template's project
as a normal project; saving writes back into the bundle (with a confirmation).

**Edge cases:**
- Deleting moves the bundle to the OS trash (no hard delete).
- A template folder edited by hand / corrupted: shown with an error badge and
  only *Delete* / *Show in file manager* enabled.
- Templates folder on a slow or network drive: listing loads asynchronously.

**Undo:** n/a (file management outside a project).

**Model notes:** `freecut-app` manages the folder and an index cache;
manifests parsed by `freecut-project`.

### OV-034 Template package export and import — later

**What the user sees:** *Export template…* writes a single `.fctemplate` file
(zip) that can be emailed or put on a USB stick; double-clicking or *Import
template…* on another machine adds it to that user's Templates page.

**Inputs / controls:** Template; export path. Import: file.

**Behavior / output:** The package contains the bundle plus a licence/credits
file listing all attributed assets used (OV-041).

**Edge cases:** Package referencing a bundled pack asset the receiving FreeCut
version does not have (older app): missing-asset placeholders and a message to
update; package from a newer format version: refused with message; zip with
path traversal entries (`../`) or absolute paths: rejected; size over 2 GB:
warning.

**Undo:** n/a.

**Model notes:** Packaging in `freecut-project` (format) + `freecut-app`
(I/O). No network sharing — local files only.

### OV-035 Bundled starter templates — later

**What the user sees:** A handful of FreeCut-made templates on the start screen
(e.g. vertical reaction video with PiP, two-up split screen, ten-photo
slideshow with crossfades, title + lower-third interview layout).

**Inputs / controls:** As OV-032.

**Behavior / output:** Behave exactly like user templates but are read-only;
*Duplicate* creates an editable user copy.

**Edge cases:** Same as OV-032; they use only bundled packs, open fonts,
generated sample media and (if any) music from AU-030's own or open-licensed
tracks, so they always resolve and carry their credits (OV-041).

**Undo:** As OV-032.

**Model notes:** Shipped as template bundles in app resources, authored in
FreeCut itself.

### OV-036 Elements library panel — v1

**What the user sees:** The element tabs *Stickers*, *Emoji*, *Shapes* and
*Frames* (plus *Charts* once OV-051 lands) in the shared library panel. The
panel itself — placement, search field, category chips with *Favorites* and
*Recent* pinned first, tile grid — is owned by
[workspace-and-shortcuts.md](workspace-and-shortcuts.md) (UI) and also hosts
the FX and CL libraries (FX-016); this feature defines only what the element
tabs add. Animated items animate on hover; hovering any item over
a selected timeline position shows a ghost preview in the player. Each tile
has *+* (add at playhead) and a star (favorite).

**Inputs / controls:** Thumbnail size slider (3 steps); click *+*, double-click,
drag to timeline, drag to preview, drag onto a clip with `Mod+Alt` (replace —
OV-010 / TL-035), drag a clip-frame item onto a clip (apply clip frame —
OV-029).
- *Overview* chip (first after *Favorites* and *Recent*): every category as
  one row of its first items, each row ending in a *More (N)* link that opens
  that category.
- Filter chips: *Animated* / *Static* (both shown by default) and *No credit
  needed* (OV-040's licence filter).
- Sort: *Pack order* (default) / *Starred first* / *Used most often* (from
  the per-user add counter kept with OV-039's recents).
- *Manage categories* (from the chip row's context menu): drag chips to
  reorder, hide a category, bring hidden ones back from a *Hidden* list, and
  *Reset order*. *Favorites*, *Recent*, *Overview* and *My stickers* cannot be
  hidden. The order is per user and per tab, stored with OV-038's favorites.

**Behavior / output:** Adding uses OV-018 defaults. The panel is virtualised so
thousands of items scroll smoothly; thumbnails are generated/decoded off the UI
thread and cached on disk. While a page of tiles loads, grey placeholder tiles
hold its place; the end of a category shows its item count ("128 stickers").
Everything is on disk, so there is no download step, no offline state and no
endless-scroll marker. Hover previews and try-before-apply use UI-007's own
sample footage, so they work on an empty timeline too (the reference app
needs media on the timeline first).

**Edge cases:**
- Hover preview while playing: disabled during playback to avoid stutter.
- Very large categories: lazy loading in pages of 100.
- Item whose file vanished (user library): tile shows a broken badge; *Remove
  from library* offered.
- A hidden category that later gains items (pack update): stays hidden; its
  entry in the *Hidden* list shows the new count.
- A pack is removed while its category is hidden: the hidden entry is dropped.
- Filters leave a category empty: the category shows "No items match the
  filters" with *Clear filters*.
- Keyboard (proposals for UI's keymap): arrows move focus, Enter adds at the
  playhead (element items always go to a `Sticker` track, never the main
  track), F toggles favorite.

**Undo:** Adding is `InsertClips` (TL-012); browsing, sorting, filters and
category order are view state (n/a).

**Model notes:** `LibraryItem { id: ItemId, kind: ItemKind, name, tags,
categories, license: LicenseInfo, animated: bool, thumb }` (the element
libraries' item record) and the element tabs live in `freecut-app`, on top of
UI's panel framework; thumbnail decoding goes through `freecut-media`. Sorting
and filtering are pure functions over the index, unit-tested.

### OV-037 Library search and tags — v1

**What the user sees:** Typing in the panel's search field (UI) filters the
current element tab instantly; a toggle searches all element tabs. Results show
matching items from bundled packs and *My stickers* together.

**Inputs / controls:** Query text; scope (this tab / all tabs). With the field
focused and empty, a row of suggestion chips: the user's recent searches (up
to 8, per user, with *Clear*) and the most common tags of the installed packs
(top 12 by item count). Suggestions come only from the local index.

**Behavior / output:** Case- and accent-insensitive matching on name, tags,
category names and (for emoji) CLDR keywords in the UI language, so the user
can search by an item's name or by what it shows ("arrow", "heart",
"snow"). Ranking: name starts with query > word in name starts with query >
tag match > substring. Empty query restores the category view. Filter chips
(OV-036) apply to results too.

**Edge cases:** No results: show "nothing found", a *Did you mean …* chip with
the closest tag (edit distance ≤ 2, FreeCut default) when one exists, and a
shortcut to *Import…*; non-Latin input matched against localized tags where
packs provide them; queries longer than 100 characters are truncated. Search
never goes online: there is no web GIF search (see *Out of scope*).

**Undo:** n/a (view state).

**Model notes:** In-memory index built at startup on a worker thread in
`freecut-app`; pure matching/ranking function unit-tested. The ranking rule is
offered to UI as the shared default; where UI's shared search defines it
differently, UI wins.

### OV-038 Favorites — v1

**What the user sees:** A star on every element tile (stickers, emoji, shapes,
frames) and on template cards (OV-033). Starred items appear in the tab's
*Favorites* category, in the order starred (newest first), and can be
re-ordered by drag. The star control itself is part of UI's shared panel; other
libraries keep their own favorites (FX-016, CL, AU-030).

**Inputs / controls:** Click star / press F; right-click *Remove from
favorites*; drag to reorder within Favorites. With several tiles selected,
right-click *Add to favorites* / *Remove from favorites* acts on all of them.

**Behavior / output:** Favorites are per user (not per project), persist across
sessions and app updates, and are keyed by stable item IDs. Every element item
and every template card can be favorited; there are no exceptions. Each change
shows a short toast (UI-040) naming the item, or the count for a
multi-selection ("5 stickers added to favorites"). Each element tab keeps its
own list, as do templates; no sign-in and no network are involved.

**Edge cases:**
- A favorited item disappears (pack removed, user sticker deleted): hidden from
  the grid; the Favorites header shows "N unavailable" with *Clean up*.
- No limit on count.
- Empty Favorites: an empty state explains that starred items collect here.
- Multi-select that mixes favorited and unfavorited items: *Add* stars the
  rest, *Remove* unstars the starred ones; the toast counts only changed items.
- Favorites file corrupt: start with an empty set, keep a backup copy of the bad
  file, log it.
- Writing the favorites file fails (disk full, read-only config folder): the
  star reverts, and an error toast offers *Retry*.
- Two FreeCut windows starring at once: last write wins per item (merge by
  item ID, not whole-file overwrite).

**Undo:** n/a (user preference; re-star to restore).

**Model notes:** Stored in the user config dir (app preferences, not the
project — same rule as FX-016) as a small JSON list of `ItemId`s with
timestamps; read/write off the UI thread in `freecut-app`.

### OV-039 Recently used — v1

**What the user sees:** A *Recent* category per element tab listing the last
items the user added to a timeline, most recent first.

**Inputs / controls:** *Clear recent* in the category's context menu.

**Behavior / output:** Up to 30 items per tab (FreeCut default); adding an item
again moves it to the top; per user, across projects. Alongside the list,
FreeCut keeps a per-item add counter that OV-036's *Used most often* sort
reads; it is local, never shared, and *Clear recent* resets both.

**Edge cases:** Items that become unavailable are dropped silently; undoing an
add does not remove it from Recent or lower its count.

**Undo:** n/a.

**Model notes:** Same storage mechanism as OV-038 (`recents.json`, UI-006),
with `{ item, last_used, count }` entries.

### OV-040 Asset packs with licence metadata — v1

**What the user sees:** Every element library item has *Info* (right-click or
`i`): name, pack, author, licence (with a short plain-language summary such as
"free to use, credit required"), source link, and whether credit is required.
A *Packs* page in preferences lists installed element packs and lets the user
add a pack folder or `.zip` (e.g. a community pack of CC0 stickers) or disable
one. Music and sound-effect packs use AU-030's manifest
([audio.md](audio.md)), not this one.

**Inputs / controls:** Add pack (folder or zip), enable/disable, remove (user
packs only). In the library panel, the *No credit needed* filter chip (OV-036)
shows only items whose licence needs no attribution (CC0, public domain, or
a manifest flag saying so).

**Behavior / output:** A pack is a folder with a manifest (pack ID, name,
version, default licence as an SPDX identifier or CC licence name, authors,
source URL, categories, per-item tags/names/licence overrides, optional 9-slice
data for frames, tintable flag, default playback mode for animated items
(OV-013), localized names). Packs
are validated on install: manifest schema, file types, size limits (OV-020),
duplicate IDs. A pack may mix static and animated items and has no fixed item
count (the reference app caps a set at 99 and refuses mixed sets; FreeCut
needs neither rule). Enabled packs appear as categories in the matching
element tab; disabling a pack removes its category from the panel (projects
that use its items are covered in the edge cases).
FreeCut shows licence facts only; it never labels an item as "cleared for
commercial use", because that judgement depends on the user's use.

**Edge cases:**
- Manifest without licence: the pack installs but every item shows "licence
  unknown" and an orange badge; it is excluded from bundled-asset CI checks.
- Pack ID collision with an installed pack: refuse unless the version is newer
  (then update in place, keeping old asset files referenced by projects).
- Zip with path traversal or symlinks: rejected.
- Disabled pack used by a project: clips render as missing until re-enabled.
- *No credit needed* filter and user-provided items (OV-020): those are left
  out, because FreeCut does not know their licence; inside the *My stickers*
  category the chip is hidden.

**Undo:** n/a (app-level configuration).

**Model notes:** Manifest schema and parsing in `freecut-project` (it is a
versioned serde format); install/index in `freecut-app`. Licence fields use the
same SPDX / CC identifiers as AU-030's manifest so OV-041 can read both. CI
check in the repo fails the build if any bundled item lacks an allowed
licence.

### OV-041 Credits sheet for used assets — v1

**What the user sees:** *File → Asset credits…* shows every library item used in
the project that requires attribution (and optionally all items), with author,
licence and link, ready to copy into a video description or save as a text
file. Element items come from OV-040 metadata; music and sound-effect lines
come from AU-030's credits data, so one sheet covers the whole project. The
export dialog ([preview-and-export.md](preview-and-export.md)) offers "save
credits next to the video".

**Inputs / controls:** Scope: attribution-required only (default) / all;
format: plain text / Markdown; copy / save.

**Behavior / output:** Walks the clips that actually render (clips on hidden
tracks and disabled clips are skipped) and groups the items by licence.

**Edge cases:** Project uses nothing requiring credit: sheet says so; user
stickers are listed as "user-provided" and FreeCut never guesses a licence for
them; assets missing right now are listed from the metadata saved with their
`AssetRef` when available, otherwise as "unknown asset".

**Undo:** n/a.

**Model notes:** Pure function over the timeline + element and audio library
metadata in `freecut-app`; nothing is burned into the video.

### OV-042 User collections for imported stickers — later

**What the user sees:** Folders inside *My stickers* ("Channel branding",
"Arrows I like"), drag items between folders, import a whole folder as a
collection.

**Inputs / controls:** New / rename / delete collection; drag to move; an item
can be in several collections.

**Behavior / output:** Collections are labels over the same stored files (no
duplication).

**Edge cases:** Deleting a collection does not delete its stickers; duplicate
names allowed with a disambiguating suffix in the list. No limit on the number
of collections or on stickers per collection (the reference app caps a set at
99 stickers; a local label needs no cap). An empty collection shows the same
empty state as *My stickers* (OV-020). Collections are private labels; sharing
a set with someone else is OV-048's pack export.

**Undo:** n/a.

**Model notes:** Extends the user library index in `freecut-app`.

### OV-043 Local brand kit — later

**What the user sees:** A *Brand* category holding the user's logos (light/dark
variants), a colour palette, and default watermark placement. Colour pickers
across the app show the brand palette as swatches; *Add logo watermark* places
the logo using OV-003 + OV-009 in one click.

**Inputs / controls:** Logos (images/SVG), named colours (hex), watermark
defaults (anchor, size, margin, opacity default 80 %).

**Behavior / output:** Stored locally per user; several kits can exist and one
is active. The active kit's logos also show as a *Brand* category in the
Stickers tab, so they are placed like any sticker. Brand fonts are a text
concern ([text-and-captions.md](text-and-captions.md)). No quotas: the
reference app's shared team kit limits colours, images and stickers per
space, which a local folder does not need.

**Edge cases:** Logo file replaced in the kit: existing clips keep the old
version (content-hash referenced); palette with duplicate colours allowed;
no active kit: the *Brand* category is hidden.

**Undo:** Kit edits n/a (user data, not the project); *Add logo watermark* is
one transaction: `InsertClips` (TL-012) plus the TR transform/opacity values
from OV-003 and the range from OV-009.

**Model notes:** Stored like the user library in `freecut-app`; no cloud sync.
Palette swatches are offered to UI's shared colour picker.

### OV-044 Element style presets — later

**What the user sees:** *Save style…* on a styled sticker/shape/PiP captures its
look (fill, stroke, outline, shadow, border, corner radius, blend, opacity) as a
named preset; *Apply style* copies it onto other selected elements.

**Inputs / controls:** Preset name; which property groups to include.

**Behavior / output:**
- Presets appear in a *Styles* chip row in the inspector; applying sets only
  the included groups. A preset may also capture opacity and blend, which it
  writes through TR-015's and OV-004's own ops.
- One-off copying from clip to clip is *Paste attributes…* (v1; `Ctrl+Alt+V`,
  `⌥⌘V` on macOS, binding owned by UI). TL-034 owns the command, its
  checklist dialog, the attribute clipboard and the `PropBag::category()`
  registry. With this feature OV registers one
  category there, **Element style**: border and corner radius (OV-005),
  outline and shadow (OV-006), clip frame (OV-029), tint (OV-015) and, for
  shapes and lines, fill, stroke and line styling (OV-025–OV-027). Opacity and
  blend are not in this category: they travel in TR-034's *Opacity & blend*
  group, so no property belongs to two categories.
- Until this later feature ships, TL-034 (v1) works without an OV category.
  OV's values that other owners already register (opacity and blend via
  TR-034, colour via CL-025, animations via FX-017) paste in v1, and a whole
  clip can still be duplicated (TL-032). No v1 feature depends on this one.
- On top of the registry, this feature adds saved, named presets that outlive
  the clipboard.

**Edge cases:**
- Applying shape-only properties (fill) to a sticker: skipped silently;
  multi-select apply is one undo step.
- Pasting *Element style* onto a target that cannot take a part of it
  (OV-005's border and corner radius onto an element clip, shape fill onto a
  sticker): that part is skipped and listed in TL-034's summary.
- Pasting onto a clip on a locked track: skipped, as for every TL-034
  category (TL-009).

**Undo:** `ApplyStyle` compound of the individual `Set*` ops. A paste of the
*Element style* category adds the same `Set*` ops (`SetBorder`,
`SetCornerRadius`, `SetOutline`, `SetShadow`, `SetClipFrame`, `SetTint`,
`SetShapeSpec`) to TL-034's single compound op.

**Model notes:** Preset storage in `freecut-app` user data; the style struct is
the same visual-style type defined in `freecut-timeline`. The *Element style*
category is registered through TL-034's `PropBag::category()` registry in
`freecut-timeline`, the same registry TR-034, FX-017 and CL-025 use; there is
no `ClipAttributes` struct. The copied values sit in TL-034's attribute
clipboard as the source clip's visual-style property bag.

### OV-045 Multi-overlay layout presets (split screen) — later

**What the user sees:** Select 2–4 clips that overlap in time and choose a
layout: side by side, stacked, three columns, 2×2 grid, one large + two small.
The clips are scaled and cropped to fill the cells.

**Inputs / controls:** Layout; gutter 0–5 % of canvas short side (default 1 %);
optional border (OV-005). The gutters show the canvas background (TR-011); the
dialog has a shortcut to set it.

**Behavior / output:** Writes transform + crop (TR) for each clip; cell
assignment follows track order (lowest track = first cell), changeable by
dragging clips between cells in the preview.

**Edge cases:** More clips than cells: extra clips untouched with a warning;
clips that only partially overlap in time are laid out anyway; layout applied
in a 9:16 canvas uses the vertical variants (stacked instead of side by side);
a main-track clip may be one of the cells; clips on locked tracks: refused;
clips with keyframed transform: values written as keyframes at the playhead
(TR-028 auto-key).

**Undo:** One transaction (TL-043) of TR transform and crop ops for all cells.

**Model notes:** Pure layout function in `freecut-timeline`; tests per layout
and orientation.

### OV-046 Transform a group as one in the preview — later

**What the user sees:** Grouping is TL-039 (clips select and move together in
time, `Mod+G`). OV adds the matching canvas behaviour: when a group is selected
in the preview, one bounding box surrounds all its members that are visible at
the playhead, and dragging, scaling or rotating that box moves, scales or
rotates the members together, like a single picture.

**Inputs / controls:** Selecting and member selection follow TL-039 (clicking
any member selects the group; its member-select gesture picks one). The box
uses TR-007's handles and modifiers; the pivot is the box centre.

**Behavior / output:** No group transform is stored. The gesture is applied to
each visible member's own TR transform about the shared pivot (positions move
and rotate around it, scales multiply, rotations add), so ungrouping later
leaves every member exactly where it appears.

**Edge cases:**
- Members not visible at the playhead are not changed; a toast says how many.
- Member with keyframed transform: a keyframe is written at the playhead
  (TR-028 auto-key).
- Member on a locked track: TL-039 / TL-009 rules — the gesture is refused as a
  whole.
- Non-uniform scaling of a rotated member cannot be expressed exactly by its
  own transform; the box allows uniform scale only for groups (FreeCut
  default).
- Group containing audio-only members: they are ignored by the box.

**Undo:** One transaction (TL-043) of TR `SetTransform` ops, one per changed
member; coalesced per drag like TR-007.

**Model notes:** `GroupId` and membership are TL-039's. The pivot math is a
pure function in `freecut-timeline` producing per-member transforms; tests
cover rotation about the pivot and keyframed members.

### OV-047 Animated stickers from numbered frames — later

**What the user sees:** In *My stickers*, *Import frames as animated
sticker…* takes one or more folders, and each folder of numbered still images
becomes one animated sticker. Before anything is written, a dialog lists every
folder with its frame count, a looping preview and its playback settings.

**Inputs / controls:**
- Source: one or more folders, or a multi-selection of image files treated as
  one folder. No fixed limit on folders per batch (the reference app allows
  20); subfolders that contain frames each become their own sticker.
- Frame order: the number at the end of each file name (before the
  extension), compared as a number, so `a9.png` comes before `a10.png`. Zero
  padding is optional. The digit-run rule is PM-033's.
- Frame rate: 1–50 fps (default 12 fps, FreeCut default; the same range the
  reference app offers).
- Playback default: *Loop* (default) or *Play once, hold last frame* — the
  OV-013 mode each placed copy starts with.
- *Same settings for every folder* (default on); off shows one settings row
  per folder.
- *Trim transparent edges* and *Padding*, as OV-020; name defaults to the
  folder name.

**Behavior / output:**
- The frames are encoded into one lossless animated PNG (APNG) in the user
  library, with a constant frame delay of exactly 1/fps (APNG stores delays as
  fractions, so 1/12 s stays exact) and a play count matching the playback
  default. From then on it is an ordinary animated sticker (OV-012, OV-013,
  OV-018) and goes through OV-020's index, duplicate check and deletion rules.
- Import runs on a worker thread with per-folder progress; a summary lists the
  imported and the refused folders with reasons, with *Retry failed*.

**Edge cases:**
- Frame count: 2–1,000 frames (FreeCut default). A folder with only one usable
  frame offers to import it as a still sticker; more than 1,000 is refused
  with the count. The reference app takes a minimum of 2 or 3 frames (its
  labels disagree) and a maximum of 49; FreeCut uses 2 because the user picked
  the folder explicitly (PM-033's 3-frame threshold only governs automatic
  detection) and a higher maximum because frames are encoded locally.
- File names without a trailing number: skipped and listed, since they cannot
  be ordered; the folder still imports if at least 2 numbered frames remain.
- Two files with the same number (`a01.png` and `a1.png`): the folder is
  refused, naming both, because the order would be ambiguous.
- Gaps in the numbering: ignored; the numbers only set the order. (PM-033
  holds the previous frame for a missing number because a media sequence
  maps numbers to time; a sticker sequence does not.)
- Mixed formats (PNG and JPEG): as in PM-033, each extension forms its own
  run; the longest run is used and the rest are listed as skipped. The
  reference app refuses such folders.
- Animated files (GIF, APNG, animated WebP) among the frames: skipped and
  listed; only stills can be frames.
- Mixed frame sizes: each frame is fitted, centred, into the first frame's
  size (PM-033's rule). The reference app refuses them.
- A frame over 8192 px, or an encoded result over 50 MB (OV-020's caps):
  refused with the reason and a hint to use fewer or smaller frames.
- Folder without any images: skipped with a note.

**Undo:** n/a (library change, not a project edit; as OV-020).

**Model notes:** Ordering and validation are a pure function
(`plan_frame_sticker(&[FileName]) -> Result<FramePlan, FrameError>`) in
`freecut-media`, table-tested; decoding and APNG encoding are also in
`freecut-media` (the `png` crate writes APNG; MIT OR Apache-2.0, verify at
adoption); library writes in `freecut-app` on a worker thread. The frame
delay is stored as a rational, so OV-012's `AnimatedImage` sees exact times.

### OV-048 Export stickers as a shareable pack — later

**What the user sees:** In *My stickers*, select stickers and choose *Export
as pack…*. A dialog asks for a pack name, description, tags, author and
licence, and writes one `.zip` that any FreeCut user can install with
OV-040's *Add pack*. This is FreeCut's local stand-in for publishing to an
online sticker store: the user shares the file however they like.

**Inputs / controls:**
- Name: required, 1–80 characters. Description: optional, up to 500
  characters. Tags: up to 20, each up to 30 characters (FreeCut defaults).
- Author: optional free text.
- Licence: a picker of SPDX / Creative Commons identifiers plus *All rights
  reserved* and *Unknown* (default *Unknown*: FreeCut never guesses a licence
  for user files).
- Cover: one of the selected stickers (default: the first).
- Per animated sticker: the default loop mode (OV-013) the pack carries.
- *Update an existing pack…*: pick a pack exported earlier to reuse its pack ID.

**Behavior / output:** Writes an OV-040 manifest plus copies of the sticker
files into the zip. A new export gets a fresh pack ID and version 1; an
update keeps the pack ID and raises the version, so a receiver who installed
the old one is updated in place (OV-040's newer-version rule). Static and
animated stickers can be mixed, and there is no item-count limit (the
reference app's store limits do not apply to a local file).

**Edge cases:**
- Nothing selected: the command is disabled. One sticker: allowed.
- A selected sticker whose file is missing: skipped and listed.
- Target file exists: overwrite confirmation.
- Licence left at *Unknown*: the receiving FreeCut shows OV-040's
  "licence unknown" badge on every item.
- Pack over 2 GB: warning with the size before writing (as OV-034).
- Write failure halfway (disk full): the partial zip is removed.
- Names or tags with leading or trailing spaces: trimmed silently.

**Undo:** n/a (writes a file; the library and projects are unchanged).

**Model notes:** The manifest uses OV-040's schema from `freecut-project`;
zip writing runs on a worker thread in `freecut-app` (e.g. the `zip` crate,
MIT; verify at adoption).

### OV-049 Custom path shapes drawn with a pen tool — later

**What the user sees:** In the Shapes tab, *Draw shape* turns the pointer into
a pen over the preview. Each click adds a point (dragging pulls out curve
handles); clicking the first point closes the shape, and double-clicking or
`Enter` ends an open path (a line or curve). The result is a shape element
clip with fill and stroke like any other shape. *Edit path* (double-click the
clip in the preview) reopens point editing at any time: move points, click a
segment to add one, delete points, and switch a point between sharp and
smooth.

**Inputs / controls:**
- Points with optional Bezier handles; closed or open.
- Fill and stroke from OV-025 (fill only on closed paths).
- Open paths: start and end caps *None* / *Round* / *Arrowhead*, with OV-027's
  head styles and size.
- `Esc` cancels drawing; `Enter` finishes it (keys are proposals for UI-031).

**Behavior / output:**
- The path is stored in the shape's own space, so the clip's TR transform
  (position, scale, rotation) applies on top, and the clip's bounds are the
  path's bounding box.
- Closed paths fill with the non-zero winding rule (TR-021's rule), so loops do
  not punch holes; joins and the miter limit follow OV-024.
- Point positions are keyframable with TR-021's rule: every keyframe has the
  same point count, and adding or removing a point applies to all keyframes in
  one step. A 0–100 % path-progress property gives the "draw-on" reveal, as
  for OV-027 lines.

**Edge cases:**
- Fewer than 2 points when finishing: nothing is created. Deleting a point
  that would leave fewer than 2 is refused with a tooltip; a closed path
  reduced to 2 points becomes open.
- Editing the path while the clip has position or scale keyframes: allowed,
  because points live in the shape's own space and the keyframes keep
  working. Alternative seen in the reference app: path editing is blocked
  while such keyframes exist, and the user is pointed to the stroke settings
  instead.
- Points dragged off the canvas: allowed.
- More than 1,000 points: drawing continues, with a performance note in the
  inspector; FreeCut does not simplify paths automatically.
- While drawing, clicks on other clips add points and never change the
  selection.
- No free `Sticker` track at the playhead: a new one is created (TL-012).

**Undo:** Finishing a drawing is one `InsertClips` (TL-012); points placed
while drawing are a draft, and `Esc` discards it without an undo entry. Later
edits are `SetShapeSpec`, one step per drag, add or delete.

**Model notes:** `ShapeKind::Path { points: Animatable<Vec<PathPoint>>,
closed: bool, caps }` in `freecut-timeline`, reusing TR-021's `PathPoint` type
and its on-canvas path editor (whichever of TR-021 and OV-049 ships first
builds them). `freecut-render` fills and strokes the path with the same GPU
path rasteriser as TR-021's pen mask.

### OV-050 Image fill for shapes — later

**What the user sees:** A third fill type, *Image*: a picture fills the shape
(a photo inside a heart, a paper texture inside a banner). The picture comes
from the media bin, from *My stickers*, or from FreeCut's own CC0 textures.

**Inputs / controls:**
- Source image.
- Mode: *Cover* (default; fills the shape and crops the rest) or *Tile*.
- Scale 10–400 % (default 100 %), angle 0–359° (default 0°), flip horizontal /
  vertical, offset (drag inside the shape in *Edit fill* mode).
- *Fill opacity* from OV-025 applies.

**Behavior / output:** The image is mapped in the shape's own space, so it
moves, scales and rotates with the shape. Scale, angle, offset and fill
opacity are keyframable (TR-027). Stills only: an animated image or a video
used as a source contributes its first frame (FreeCut default).

**Edge cases:**
- Source missing (media offline or sticker deleted): a neutral checker
  pattern with a missing badge; relinking follows PM-091 / OV-018.
- Colour keyframes from a solid fill are kept but inactive (OV-025).
- *Tile* at a small scale: many repeats, sampled with mipmaps (OV-011), so
  no shimmer.
- A bin item used as a fill counts as used (PM-055), so removing it from the
  project goes through PM-058's confirmation for used items.

**Undo:** Part of `SetShapeSpec`.

**Model notes:** `Fill::Image { source: FillSource::{Media(MediaId),
Asset(AssetRef)}, mode, scale, angle, flip, offset }` in `freecut-timeline`;
the same `Fill` type is used by chart backgrounds and sector fills (OV-053).
Sampling in `freecut-render`'s shape shader.

### OV-051 Chart elements and the chart editor — later

**What the user sees:** A *Charts* sub-tab in the Elements panel with *New
chart* and the user's saved charts (*My charts*). *New chart* opens the chart
editor: a window with a type gallery on the left, a live preview in the
middle, and *Data* (OV-052), *Style* (OV-053) and *Animation* (OV-054) tabs on
the right. *Add to timeline* places the chart at the playhead as an element
clip. Double-clicking a chart clip reopens it in the editor, where *Update
clip* writes the changes back.

**Inputs / controls:** Chart types (FreeCut's own set):
- Bar: simple, grouped or stacked; vertical or horizontal.
- Line: one or several series; smooth or straight; optional area fill.
- Pie and donut: pie, donut, progress ring (one value against a total) and
  grouped progress rings (concentric).
- Gauge: arc gauge with a needle, and ring gauge with the value in the centre.
- Radar: one or several series.
- Rose (polar area).
- Word cloud.
- Table.

Decorative looks are style themes (OV-053), not extra types, and every type
can animate (OV-054). Other controls: *Save to My charts* with a name (1–80
characters); *Add to timeline*; *Update clip*; *Rename* and *Delete* in *My
charts*. A new chart clip lasts 5 s and its box spans 60 % of the canvas
width, centred (FreeCut defaults).

**Behavior / output:**
- A chart clip carries its whole definition (`ChartSpec`: type, data table,
  field mapping, style, animation), so a project never depends on *My
  charts*. Saving to *My charts* stores a copy in the user library
  (`<user data dir>/library/charts/`, one file per chart).
- The chart is vector content drawn at the clip's on-canvas size, crisp at
  any scale. It lives on a `Sticker` track like other elements; transform,
  opacity, blend (OV-004), outline and shadow (OV-006) and FX's clip
  animations (OV-018's exclusive policy) apply as for any element clip.
- Placement on the canvas is TR's transform; the editor preview shows only
  the chart box.
- Everything happens on this computer; no data leaves it.

**Edge cases:**
- Name already used in *My charts*: FreeCut asks *Replace* or *Keep both*
  ("Sales (2)"). The reference app either refuses or silently replaces,
  depending on where the save happens; FreeCut always asks.
- Switching chart type: the data and the field mapping are kept where the new
  type can use them; style options the new type lacks stay stored but hidden,
  so switching back restores them, and no confirmation is needed.
  Alternative seen in the reference app: switching type throws away the
  current edits after a confirmation.
- Opening another saved chart, or closing the editor, with unsaved edits:
  *Save* / *Discard* / *Cancel*.
- The clip being edited is deleted or undone away while the editor is open:
  *Update clip* turns into *Add to timeline*.
- Deleting a saved chart: confirmation; clips made from it are unaffected.
- Chart clip on a locked track (TL-009): the editor opens read-only.
- A font used by the chart is missing on another computer: TX-007's warning
  and fallback.

**Undo:** *Add to timeline* is `InsertClips` (TL-012); *Update clip* is one
`SetChartSpec { clip, old, new }`. Edits inside the editor work on a draft
with the editor's own undo while it has focus; they reach the project's undo
stack only through *Update clip*. *My charts* changes are library changes
(n/a).

**Model notes:** `ElementSource::Chart(ChartSpec)` in `freecut-timeline`, with
`ChartSpec { kind: ChartKind, table: DataTable, mapping: FieldMapping, style:
ChartStyle, anim: ChartAnimation }` as plain data that `freecut-project`
serializes. Layout (axes, bars, sectors, label positions, word placement) is
a pure function in `freecut-render`, `layout_chart(spec, box, text_metrics)
-> ChartScene`, that takes TX's text measurement as input and is unit-tested
on fixed tables; word clouds use a seeded spiral placement so preview and
export always match. Drawing reuses the GPU path rasteriser shared by TR-021
and OV-049.

### OV-052 Chart data: table editing and spreadsheet import — later

**What the user sees:** The chart editor's *Data* tab is a spreadsheet-like
grid. The user types values, adds rows and columns next to the current
cell, deletes the selected rows or columns, renames fields and assigns them
roles.
*Import file…* (or dropping a file on the grid) loads a CSV or spreadsheet
file; *Save example file…* writes a small example CSV showing the expected
layout. Files are read on this computer only.

**Inputs / controls:**
- Grid: up to 10,000 cells (for example 500 rows × 20 columns); cell text up
  to 200 characters (FreeCut defaults).
- *First row is header* (default on).
- Field roles: *Category* (text or dates), *Value* (one or more numeric fields,
  each with a display name), *Series* (for grouped, stacked and multi-line
  charts), *Order* (optional sort field). Word clouds use *Word* + *Weight*;
  progress rings and gauges use *Value* + *Maximum* (default 100).
- *Clear all*, with a confirmation.
- Import: CSV (separator comma, semicolon or tab, detected automatically with
  a manual override; UTF-8 with or without BOM, or UTF-16 with BOM) and XLSX
  or ODS workbooks (sheet picker, default the first sheet).
- Number parsing: the UI locale's decimal and thousands separators (a `.`
  decimal is always understood), `%`, a leading or trailing currency sign
  (offered as the label prefix or suffix, OV-053), K / M / B suffixes (×10³,
  ×10⁶, ×10⁹), and negatives written as `-12` or `(12)`.

**Behavior / output:** An import fills the grid, replacing current data after
a confirmation if the grid is not empty. Roles are guessed (first text column
→ Category, numeric columns → Values) and can be changed. Formula cells use
the result stored in the file (no recalculation); date cells become category
text in ISO form. The preview updates as the user types.

**Edge cases:**
- A file larger than 1 MB, or more than 10,000 cells: refused, naming the
  limit (the reference app uses the same 1 MB guard).
- A cell in a *Value* column that does not parse: kept as text, shown in red
  and listed with its address (for example "C7: 12,5x"); it draws as a gap.
  The reference app silently drops K/M/B suffixes and rejects thousands
  separators; FreeCut parses both and names any cell it cannot read.
- Ambiguous separators ("1,234" in a locale where the comma is the decimal
  mark): the locale wins; the import summary states the reading used and
  offers *Swap separators*.
- Empty header cells get names like "Column 3"; duplicate names get a suffix.
- Header row turned off: fields are named Column 1 … N.
- Encrypted or corrupt workbook: refused with the reason.
- Formula cell without a stored result: empty, with a warning.
- Merged cells: the value goes to the top-left cell; the rest are empty.
- The field assigned to a role is deleted: the role becomes unassigned and the
  preview says which role is missing.
- Negative values in pie, donut, rose or word-cloud charts: drawn as 0 with a
  warning; an all-zero pie shows an empty ring with a note.
- Empty cells in line charts: a gap in the line (default) or *Connect across
  gaps*.

**Undo:** Within the editor's draft (OV-051); reaches the project as
`SetChartSpec` on *Update clip*.

**Model notes:** `DataTable { columns: Vec<Column { name, cells:
Vec<Cell::{Empty, Number(f64), Text(String)}> }> }` in `freecut-timeline`
(`f64` per OV's numeric rule). The number parser is a pure function taking the
locale, table-tested in `freecut-timeline`. Reading and parsing files runs on
a worker thread in `freecut-app` with the `csv` crate (MIT OR Unlicense) and
`calamine` (MIT); verify both at adoption.

### OV-053 Chart styling — later

**What the user sees:** The chart editor's *Style* tab, with sections Theme,
Text, Labels, Axes & legend, Colours, Background and Shape.

**Inputs / controls:**
- Theme: FreeCut's own themes (for example Light, Dark, High contrast, Pastel,
  Mono), each setting colours, fonts and background at once; the user's
  changes stay on top of the theme.
- Title and subtitle: text, font (TX's font picker), size, colour, alignment;
  each can be turned off.
- Data labels: on/off, size, colour; prefix and suffix up to 8 characters each
  (for example a currency sign or `%`); display unit none / K / M / B / auto;
  decimals 0–4 or auto; for pie-like charts: value, percentage or both.
- Axes: X and Y on/off, axis titles, gridlines on/off, value range auto or a
  manual minimum and maximum.
- Legend: on/off; top, bottom (default), left or right.
- Colours: theme palette (default), custom palette (2–12 colours), a
  two-colour gradient across items, or one colour for everything.
- Background: none (transparent, default), solid colour or image (OV-050's
  image fill); background opacity 0–100 %.
- Stroke around bars and sectors: on/off, colour, width.
- Bars: width 10–100 % of the slot (default 70 %), gap between groups 0–100 %
  (default 20 %), corner rounding 0–50 % (default 0 %), vertical or
  horizontal.
- Arcs (pie, donut, progress rings, gauges, rose): inner radius 0–90 % (0 =
  pie), outer radius 50–100 % of the box, start angle 0–359°, sweep 1–360°
  (default 360°; gauges 180°), gap between sectors 0–10°, sector corner
  rounding 0–50 %. Pie and donut sectors can also show one image across all
  sectors, each sector tinted with its palette colour at 0–100 % strength.
- Lines: smooth or straight, width 1–20 px@1080, point markers on/off, area
  fill on/off.

**Behavior / output:** Sizes are stored relative to the chart box, so the
chart rescales cleanly with its clip. Number formatting uses a locale stored
in the chart (the UI locale when the chart was created), so a project renders
the same on every computer.

**Edge cases:**
- Controls that do not apply to the current type are hidden: arc controls
  only for pie-like charts and gauges, smoothing only for line and radar
  charts, axes not for pies or word clouds.
- Overlapping labels: the lower-priority ones (smaller values) are hidden;
  labels are never drawn on top of each other.
- Category names longer than 24 characters are cut with an ellipsis (FreeCut
  default).
- A manual value range that excludes some data: those bars are clipped at the
  axis, with a warning in the editor.
- Right-to-left text in titles or labels: TX's bidirectional layout applies.
- A background or sector image that goes missing: theme colours with a missing
  badge in the editor.

**Undo:** Within the editor's draft; reaches the project as `SetChartSpec`.

**Model notes:** `ChartStyle` is plain data in `freecut-timeline` (colours as
`Rgba8`, fills as OV-050's `Fill`); themes are bundled data files designed by
us and released CC0.

### OV-054 Chart build animation — later

**What the user sees:** The chart editor's *Animation* tab lets the chart
build up when it appears: bars grow from the axis, lines draw from left to
right, sectors sweep round, words pop in and value labels count up.

**Inputs / controls:**
- On/off (default on).
- Order: *Together* (default) or *In sequence* (item after item; series by
  series for grouped charts).
- Style: *Build up* (default; bars and sectors grow from zero, lines draw
  on), *Fade in*, or *Fall into place* (items land with a small bounce);
  *Count up* for value labels on/off (default on).
- Duration 0.2–5 s (default 1 s), easing from TR-031's presets (default ease
  out), delay after the clip starts 0–5 s (default 0 s). FreeCut defaults.

**Behavior / output:** Build progress is a pure function of clip-local time
since the chart's content start, the delay, the duration and the easing.
*In sequence* gives each item an equal window, overlapping its neighbour by
half (FreeCut default), so the whole build still takes the set duration.
After the build the chart stays complete. The build is content, like OV-013's
content loop, so it combines freely with FX's Entrance and Exit animations,
which move or fade the clip as a whole.

**Edge cases:**
- Clip shorter than delay + duration: the build is cut off at the clip end,
  never squeezed; the timeline bar marks where the build would finish.
- Splitting a chart clip (TL-023): the right half continues where the left
  half ended (time counts from the content start, like OV-012), so it usually
  shows the finished chart. A left-edge trim into the build skips its
  beginning.
- *Count up* with prefixes, suffixes and K/M/B units: the in-between values
  use the same formatting, rounded to the label's decimals.
- Word clouds with *Fall into place*: words land in their final positions;
  the layout never reflows during the animation.
- The system's reduced-motion setting changes only the editor's UI, never
  the rendered video.

**Undo:** Part of `SetChartSpec`.

**Model notes:** `ChartAnimation { enabled, order, style, count_up, duration:
Rational, delay: Rational, easing }` in `freecut-timeline`; durations are
times and therefore rational (TL-001). The progress function is pure and
table-tested with rational times; its eased output is an `f64` that the
layout in `freecut-render` consumes.

## Interactions with other areas

- **[timeline-editing.md](timeline-editing.md)** — owns time, tracks (main
  track, typed overlay tracks, strict/flexible stacking — TL-004, TL-007), the
  add/insert rules including single-item *Add as overlay* (TL-012), selection,
  linking, attachments, groups (TL-039), replace (TL-035), split/trim/move and
  undo semantics (TL-043). OV adds element clip content on `Sticker` tracks,
  multi-item stacking (OV-002), layer-order commands (OV-007), element
  replacement (OV-010) and placeholder marks on clips. Source limits: TL-011's
  `SourceLimits` returns *unbounded* for media whose OV-013 playback mode
  loops (animated images, OV-012; image sequences, OV-017), and TL-020 trims
  such clips like stills; OV-013 owns the clip-time → animation-time mapping.
  *Paste attributes…* (TL-034, v1) owns the command, attribute clipboard and
  `PropBag::category()` registry; OV registers its *Element style* category
  there (OV-044) and has no `ClipAttributes` struct.
- **[clip-transform-and-speed.md](clip-transform-and-speed.md)** — owns
  position/scale/rotation/flip, fit mode, opacity, crop, masks, chroma key,
  canvas background, on-canvas handles and picking (TR-007), keyframes, speed
  and frame sampling. Every overlay and element uses these unchanged; PiP
  presets, layout presets and group transforms only *write* TR properties.
  OV-004 owns `BlendMode` (list, formulas, serialization, hover preview);
  TR-016 only places the dropdown in its Compositing section and stores the
  value there. TR's shared conventions record the project-wide non-time
  values rule once: only time is rational, each owning spec picks its own
  scalar type (OV: `f64`), and everything keyframable animates through
  TR-027's generic `Animatable<T: Interpolate>`. TR-034's *Opacity & blend*
  paste group carries opacity and blend for element clips too. OV-049's
  custom paths reuse TR-021's `PathPoint` type, same-point-count keyframe rule
  and on-canvas path editor; chart build animations (OV-054) use TR-031's
  easing presets.
- **[effects-transitions-animations.md](effects-transitions-animations.md)** —
  owns render order (FX-001), the working colour space, transitions and the
  clip-animation slots (FX-018 Entrance/Exit, FX-019 Continuous), whose
  applicability covers video, image, text and element clips. Element clips
  use FX's exclusive slot policy (OV-018); only text uses the coexisting one
  (TX-022). FX-001 places OV's work in fixed stages: OV-005's corner coverage
  at L7 (with masks), element styling at L8b (shadow, outline, content,
  border, clip frame — OV-005, OV-006, OV-029), and the OV-004 blend formula
  executed once per layer at L10. The shared library panel is used by FX-016
  too.
- **[color-and-filters.md](color-and-filters.md)** — filters and adjustments
  apply to media overlays and raster element clips like any visual clip
  (vector shapes have no colour stack); gradient and blend
  colour handling follow FX-001's working space, which CL also uses.
- **[text-and-captions.md](text-and-captions.md)** — text clips are visual
  clips in the same stack and use OV-004's blend modes; text draws its own
  outline and shadow while rasterizing, so FX-001's L8b element styling does
  not apply to text. Text's Loop animation coexists with In/Out (TX-022),
  unlike element clips. Also: emoji inside text; fonts for templates and brand
  kit; text slots in templates. Chart titles and labels (OV-051–OV-054) use
  TX's font picker, font registry, text measurement, bidirectional layout and
  missing-font fallback (TX-007).
- **[ai-tools.md](ai-tools.md)** — background removal for cut-out stickers
  (AI-014 → OV-023); motion tracking of any overlay, elements included
  (AI-008), with OV-022 adding endpoint-only binding for arrows.
- **[project-and-media.md](project-and-media.md)** — media import and image
  decoding rules (PM-034, PM-036), still-image default duration (PM-010),
  image-sequence detection (PM-033), alpha interpretation (PM-040), proxies
  (alpha-capable, PM-080), offline media and relink (PM-090, PM-091),
  referenced user sticker files and packaging (PM-042, PM-101), format
  migrations (PM-021); the project format stores element sources, visual
  styles, chart specs and placeholder specs defined here. PM-033's digit-run
  ordering rule is reused for frame-sequence stickers (OV-047); a bin item used
  as a shape's image fill (OV-050) counts as used (PM-055, PM-058).
- **[audio.md](audio.md)** — audio of video overlays is mixed normally;
  template slots set the filled clip's mute through AU-007 (OV-031, OV-032);
  music and sound-effect packs use AU-030's manifest and credits data, which
  OV-041 folds into the project credits sheet; bundled starter templates use
  only AU-030's own or open-licensed music.
- **[preview-and-export.md](preview-and-export.md)** — export must match the
  preview compositing exactly; credits sheet hook in the export dialog; export
  with alpha (if offered) must keep overlay transparency.
- **[workspace-and-shortcuts.md](workspace-and-shortcuts.md)** — owns the
  shared library panel framework (search field, favorites/recent sections, tile
  grid) that OV-036–OV-039 build on, the shared colour picker, the
  start-screen template grid placement and the final keymap (all OV shortcuts
  are proposals), including the v1 *Paste attributes…* binding that OV-044's
  category is reached through (UI-012, UI-031). UI-004's Elements row lists
  the element sub-tabs; *Charts* joins it when OV-051 lands. Mixed values in
  multi-selection follow UI-010, and favorites/import toasts use UI-040.

## Out of scope (and why)

- **Online sticker, GIF or template stores / trending feeds / third-party GIF
  search services** — need network accounts or API keys and conflict with
  "no account, no cloud". The reference app's sticker panel queries an
  external GIF service and suggests trending search words; FreeCut searches
  only its local index (OV-037), and users import files they downloaded
  themselves (OV-020). An opt-in plugin for an openly licensed source could be
  considered later; nothing goes over the network by default.
- **Download-on-first-use asset libraries** — the reference app fetches each
  sticker the first time it is used, shows download progress and retry, can
  withdraw or expire items so they vanish from projects, and cannot list the
  library offline. FreeCut ships its packs inside the app, never expires or
  withdraws them (OV-019), and keeps the missing-asset placeholder only for
  files the user deleted (OV-018).
- **Publishing templates to a community gallery, creator programs, template
  links, use and play counters, purchased-template lists** — no servers.
  Local file exchange only (OV-034); use counts are local (OV-033).
- **Publishing sticker packs to a marketplace** (review queues, approval,
  covers to a store's spec, per-store size and frame-rate rules, usage
  statistics) — no servers. Local substitute: *Export as pack* (OV-048) and
  *Add pack* (OV-040).
- **Cloud AI sticker generation from a text prompt, and cloud asset libraries
  of generated "elements"** — cloud dependency and credit economics; a local
  generative model could be considered later under
  [ai-tools.md](ai-tools.md), not here. Cut-out stickers from the user's own
  footage are local (OV-023).
- **Team brand-kit stickers shared through an online space** — needs accounts
  and a server, with per-space quotas. Local substitute: the brand kit
  (OV-043), whose logos appear as a *Brand* sticker category.
- **Sign-in-bound favorites synced across devices** — favorites are a local
  file (OV-038); there is no sign-in and nothing to sync.
- **Commercial-use certification, scans and labels** — FreeCut cannot judge
  how the user will use an asset. It shows licence facts (OV-040, with the
  *No credit needed* filter) and the credits sheet (OV-041) instead.
- **Region locks, expiring unlocks, mobile-only templates and feature gates in
  template-based projects** (for example "replace every sample clip before
  other tools work") — none apply to local templates; projects made from
  templates are ordinary projects (OV-032).
- **Recording (shooting) templates** that guide a camera capture shot by shot
  — they depend on in-app camera recording, which is a later PM feature
  (PM-043); revisit only after that ships.
- **Importing another editor's proprietary template or draft files** — not
  planned; any future importer would follow the clean-room rule of documenting
  only files we created ourselves.
- **Face-anchored AR stickers (glasses, masks that follow facial landmarks)** —
  ML-heavy; may come later via the AI area's landmark tracking.
- **3D stickers/objects** — large feature with little everyday use in this
  class of editor.
- **Paywalled or locked content** — everything FreeCut ships is free; there is
  no "pro" tier to gate stickers, frames, packs or templates, no paid-pack
  badges, and nothing is counted or blocked at export.

## Clean-room notes

- **Stickers (OV-019)**: we ship only (a) art made for FreeCut and released as
  CC0, (b) CC0 / public-domain art from open sources (e.g. Kenney game assets,
  CC0; Openclipart, public domain) with each item's source recorded, and
  (c) open icon sets as tintable icon stickers (e.g. Lucide, ISC; Tabler Icons,
  MIT). No stickers, GIFs, category names or artwork from any proprietary
  editor's library, and no platform logos or trademarks (generic "subscribe /
  like / follow" button styles only).
- **Emoji (OV-021)**: proprietary vendor emoji designs are not shipped. Default
  candidate: Noto Emoji SVG artwork (Apache-2.0; its flag images are public
  domain; the font builds are OFL-1.1). Alternatives: Microsoft Fluent Emoji
  (MIT), OpenMoji (CC BY-SA 4.0), Twemoji as maintained in the community fork
  (graphics CC BY 4.0, code MIT). The Noto default matches the Noto Color
  Emoji font (OFL-1.1) that TX bundles for emoji inside text, so both look
  alike. Names/keywords from Unicode CLDR (Unicode licence). Attribution data
  goes in the pack manifest so OV-041 can credit it.
- **Frames (OV-029)**: our own vector/9-slice designs or procedurally generated
  borders; CC0 textures only.
- **Shapes, lines, arrows, custom paths (OV-024–028, OV-049)**: pure
  geometry, no assets needed. Image-fill textures shipped with FreeCut
  (OV-050) are our own or CC0.
- **Charts (OV-051–OV-054)**: chart types are generic data-visualisation
  forms; FreeCut's chart themes, palettes and example data are designed by us
  and released CC0, and no chart template, theme or preset of another editor
  is recreated. Spreadsheet parsing uses open-source crates (`csv`,
  `calamine`); verify their licences at adoption.
- **Sticker taxonomy (OV-019)**: the category list is FreeCut's own; seeing
  that the reference app covers seasons, countdowns, pets and similar themes
  only told us which generic subjects people expect. No category names,
  artwork or sticker designs were taken from it.
- **Blend modes (OV-004)**: formulas from the open W3C Compositing and Blending
  Level 1 specification plus standard arithmetic modes; mode names are generic
  industry terms.
- **Templates (OV-030–035)**: user-made only; starter templates are authored in
  FreeCut by us with generated sample media and released CC0; any music in them
  comes from AU-030's own or open-licensed tracks and any fonts from TX's
  bundled open fonts (e.g. OFL-1.1). No template, layout or preset from another
  editor's catalogue is recreated.
- **Element pack sources (OV-040)**: user-added packs are the user's
  responsibility; bundled packs must pass the licence CI check.
- **Renderers/decoders**: `resvg`/`usvg` (SVG; Apache-2.0 OR MIT),
  `velato`/`vello` (Lottie; Apache-2.0 OR MIT) or `rlottie` (mostly MIT, mixed
  component licences), FFmpeg/`image` decoders — all open source; verify each
  licence is compatible with GPL-3.0-or-later when adopted.
- **Research method**: behaviour was observed through public help pages,
  vendor marketing pages, third-party tutorials and forums, and through the
  English UI labels of a locally installed copy of the reference app
  (desktop 9.5.0), taken from its bundled English localisation file (see
  Provenance in the [README](README.md)) and paraphrased into a feature
  inventory. Nothing was decompiled, disassembled or traced, and no assets,
  draft or template files or network traffic were inspected or copied. Numbers taken from that inventory (ranges, limits, option lists) are
  stated as observations, and wherever FreeCut chooses differently the spec
  says so in one line.

## Sources

- How to Effectively Use Video Overlay in Your Edits (CapCut resource) — https://www.capcut.com/resource/free-video-overlay-effects
- How to Put a Picture on a Picture (CapCut resource) — https://www.capcut.com/resource/how-to-put-a-picture-on-a-picture
- How to Overlay a Video on a Video (CapCut resource) — https://capcut.com/resource/overlay-video-on-video
- Blend modes for creative video and photo effects (CapCut) — https://www.capcut.com/create/blend-modes-creative-video-photo-effects
- CapCut stickers overview (CapCut resource) — https://www.capcut.com/resource/capcut-stickers
- Video editing with stickers (CapCut resource) — https://capcut.com/resource/video-editing-with-stickers
- How to make a template (CapCut resource) — https://capcut.com/resource/how-to-make-a-capcut-template
- Help center index (CapCut) — https://www.capcut.com/help
- How to set replaceable material clips (CapCut help) — https://www.capcut.com/help/how-to-set-replaceable-material-clips
- How to use and export templates (CapCut help) — https://www.capcut.com/help/use-and-export-templates-in-capcut
- Downloaded template shows one frame on PC (CapCut help) — https://www.capcut.com/help/download-tempalte-problem
- Text hidden by other layers (CapCut help) — https://www.capcut.com/help/can-not-display-the-text
- How to add stickers to videos in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-add-stickers-to-videos-in-capcut--cms-108820
- How to create picture-in-picture videos in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-create-picture-in-picture-videos-in-capcut--cms-108806
- How to create custom backgrounds in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-create-custom-backgrounds-in-capcut--cms-108866
- CapCut for PC & Mac desktop tutorial (Primal Video) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- How to add overlay in CapCut PC (GeekChamp) — https://geekchamp.com/how-to-add-overlay-in-capcut-pc-full-guide/
- How to add overlay in CapCut PC (MacMyths) — https://macmyths.com/how-to-add-overlay-in-capcut-pc/
- How to add a frame in CapCut (MiniTool, Japanese) — https://moviemaker.minitool.com/jp/news/how-to-add-frame-on-capcut.html
- Logo animation in CapCut PC (antwort.net, German) — https://www.antwort.net/a/tutorial/logo-animation-in-capcut-pc-erstellen-das-ultimative-tutorial.html
- Motion tracking in CapCut PC (antwort.net, German) — https://www.antwort.net/a/tutorial/motion-tracking-in-capcut-pc-die-ultimative-anleitung.html
- Motion tracking in CapCut (Miracamp) — https://www.miracamp.com/learn/capcut/motion-tracking
- CapCut brand kit guide (capcutguide.com) — https://capcutguide.com/capcut-brand-kit/
- CapCut Desktop review (Passionfruit) — https://passionfru.it/capcut-desktop-review-104982/
- Shotcut forum: track blend modes — https://forum.shotcut.org/t/videos-on-top-of-each-other-using-modes-linear-burn-pin-light-etc/22123
- Shotcut forum: default image duration — https://forum.shotcut.org/t/how-to-change-the-default-duration-of-images-or-other-clips/24178
- Kdenlive docs: Transitions and compositions — https://docs.kdenlive.org/en/compositing.html
- Kdenlive docs: Misc configuration (default durations) — https://docs.kdenlive.org/en/getting_started/configure_kdenlive/configuration_misc.html
- Kdenlive issue: animated GIF imported as still — https://invent.kde.org/kde/kdenlive/issues/335
- OpenShot user guide: preferences (image length) — https://en.openshot.org/files/user-guide/preferences.html
- OpenShot Q&A: animated GIFs show first frame — https://answers.launchpad.net/openshot/+question/224609
- Guide to composite modes in DaVinci Resolve (Mixing Light) — https://mixinglight.com/color-grading-tutorials/guide-to-composite-modes-davinci-resolve
- Blackmagic forum: still duration setting — https://forum.blackmagicdesign.com/viewtopic.php?p=439373
- W3C Compositing and Blending Level 1 — https://www.w3.org/TR/compositing-1/
- Glaxnimate emoji set metadata (licences) — https://lxr.kde.org/source/graphics/glaxnimate/data/emoji/sets.json
- Emoji SVG collections and licences (allsvgicons) — https://www.allsvgicons.com/collections/emoji/
- velato crate docs — https://docs.rs/crate/velato/0.11.0
- rlottie COPYING (downstream mirror) — https://repo.retrodeck.net/Xargon/ES-DE/raw/tag/retrodeck-main-200125.42/external/rlottie/COPYING
- resvg crate page (licence) — https://docs.rs/crate/resvg/latest
- Twemoji community fork README (licences) — https://github.com/jdecked/twemoji
- Noto Emoji repository README (licences) — https://github.com/googlefonts/noto-emoji
- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
