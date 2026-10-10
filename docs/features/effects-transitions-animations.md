# Effects, transitions & clip animations

> Status: draft · Owns: per-frame compositing/render order (layer stages, including where other areas' stages slot in); transition model (cuts, transition region, incoming-clip in-point shift and ripple, duration cap, visible range); effect items (clip effects and effect layers), effect stacking order and effect regions (FX-027); clip entrance/exit/continuous animation model and its per-clip-kind slot policy; the animation preset manifest format, Intensity and the motion definitions of clip presets (FX-030) · Crates: freecut-timeline, freecut-render, freecut-audio, freecut-project, freecut-app (ML-backed body/face effects and photo-parallax depth consume analysis from the AI area)

## Summary

This area lets a FreeCut user soften or stylise the change from one clip to the
next with a transition, change the look of a clip (or of everything beneath a
point in the track stack, or only one region of the frame) with video effects,
and make a clip enter, leave, or move continuously with one-click animations. It also defines, once for the
whole project, the exact order in which every per-frame operation (decode,
retouch, crop, keying, color, effects, masks, transform, element styling,
animation, opacity, blend, transitions, effect layers, captions) is applied, so
preview and export always produce the same picture.

Terminology used below:
- **Cut**: the boundary between two clips that touch on the same track (the
  out-point of clip A equals the in-point of clip B on the timeline, frame-exact).
  Clips on one track never overlap (TL-004); a transition never changes that.
- **Transition region**: the stretch of timeline time, of length `D`, in which a
  transition mixes A and B (FX-002, FX-008). With the default placement it is
  `[cut − D, cut)`, i.e. the last `D` of A's stored range.
- **Pre-roll / post-roll**: frames a clip shows inside a transition region but
  outside its own stored range: B's source before its in-point (pre-roll), or
  A's source after its out-point (post-roll, handle mode only). They are read
  through the clip's time map (TR-036), which continues past in/out at the edge
  speed.
- **Visible range**: a clip's stored range extended by any pre-roll and
  post-roll its transitions use. A clip with no transitions has visible range =
  stored range. Clip-local time, clip effects and clip animations are measured
  over the visible range.
- **Clip effect**: an effect attached to one clip; it lives and dies with the clip.
- **Effect layer**: an effect placed as its own timeline item on its own track;
  it changes the composite of the tracks beneath it for its time range.
- **Effect region**: an optional mask on one effect (clip effect or effect
  layer) that limits where in the frame it applies (FX-027). Without one, the
  effect covers the whole frame.
- **Entrance / Exit / Continuous animation**: FreeCut's names for one-click
  motion presets that play at the start of a clip, at its end, or across the
  whole clip.
- Time values are rational timeline time as defined in `timeline-editing.md`
  (TL). Where this spec says "0.5 s" it means the rational value snapped to the
  project frame grid.

## Features

### FX-001 Per-frame compositing pipeline — mvp

**What the user sees:** A predictable layer stack. Higher tracks cover lower
ones; an effect layer changes only what lies beneath it; captions always sit on
top; a clip's color, effects, mask, position, outline and fade always combine
the same way. What the viewer shows is what the exported file contains (at the
export resolution).

**Inputs / controls:** No direct controls. Inputs are the project canvas size,
frame rate and background (PM/TR), track order, bands and visibility (TL), and
every per-item setting owned by other areas.

**Behavior / output:** For each output frame `n`, FreeCut evaluates the timeline
at time `t = n × frame_duration` (rational). The frame is built as follows.

1. **Canvas.** Clear an opaque canvas-sized accumulator to the project
   background (solid color, blurred copy of the main-track clip, or an image —
   defined in TR). Default: black.
2. **Collect layers.** The render order is TL's validated bottom-to-top track
   order (TL-004, TL-007); FX does not reorder tracks or decide which kinds may
   sit where. By default strict layering applies, with three visual bands,
   bottom to top: Video, then Text/Sticker/Effect/Adjustment, then Caption.
   With Flexible stacking (TL-007) the first two bands mix freely above the
   main track (which is always the lowest), but the Caption band still stays
   on top. The walk takes every visible non-caption
   track bottom to top, then composites the **Caption band after all other
   visual tracks**, its tracks in their timeline order (TX-025). For each track,
   find the item active at `t`. On a track whose transition region contains
   `t`, the track yields a *transition pair* (outgoing A, incoming B) instead
   of a single item.
3. **Per visual clip (video or image) — layer stages L1–L10**, in this order.
   In MVP only L1, L2, L8 (base transform), L9 and L10 (Normal) do work; every
   other stage is a pass-through until the feature that feeds it lands, so
   the order never changes later.
   - **L1 Source sample.** Map `t` to source time through speed, speed curve,
     reverse and freeze (TR-036), and fetch the decoded frame (media crate, off
     the UI thread). Inside a transition region a side may need pre-roll or
     post-roll (FX-002), which the same time map supplies. Frame sampling
     (nearest or blended) follows TR-041. Missing media yields the offline
     placeholder from PM.
   - **L2 Normalize.** Convert to the working format: RGBA 16-bit float,
     premultiplied alpha, Rec.709 primaries, display-referred (gamma-encoded)
     values. Apply container rotation, pixel aspect ratio, and the YUV→RGB
     matrix, range and transfer handling that CL-002 defines (including its
     HDR tone mapping). Sources with alpha keep it.
   - **L2b Retouch** (AI-020, later) — face retouch on raw source pixels, before
     stabilization and crop, so keying and everything later see the retouched
     face.
   - **L2c Footage motion blur** (FX-028, later) — blur along the clip's
     cached optical flow in source pixel space, after retouch and before
     stabilization, so the blur follows the motion recorded in the footage.
   - **L3 Stabilize and crop** (TR-026, TR-009): the stabilization
     correction, then crop, in source pixel space.
   - **L4 Keying** (TR; background removal from AI) — writes alpha.
   - **L5 Color** — adjustments then filter/LUT at its intensity (CL owns the
     internal order of its own stages).
   - **L6 Clip effects** — the clip's effect stack, first to last (FX-010,
     FX-013), in *layer space*: the cropped image at the size it would occupy on
     the canvas at 100 % scale. Effects cannot grow the layer bounds in v1
     (glow beyond the clip edge is clipped). An effect with a region
     (FX-027) mixes its output with its input by the region's coverage.
   - **L7 Mask and corner coverage** — TR's shape masks with feather/invert
     multiply alpha; OV-005's rounded-corner coverage of the visible (cropped)
     rectangle multiplies alpha in the same stage.
   - **L8 Transform** — the keyframed transform from TR, applied in TR's
     order (flip, then scale and rotate about the anchor, then translate). The
     animation offsets of FX-018/FX-019 are folded into those values before
     the transform is built: position offsets add, scale factors multiply,
     rotation offsets add. The layer is resampled into a canvas-sized texture
     (bilinear with mipmaps for downscaling; FreeCut default).
   - **L8b Element styling** (OV) — in canvas space, after the transform, so
     widths stay constant on the canvas and follow rotation. The clip's layer
     is assembled bottom to top in this order: drop shadow (OV-006), outline
     (OV-006), the content from L8, border (OV-005), clip frame (OV-029). With
     none of them set this stage is a pass-through.
   - **L9 Opacity** — alpha × keyframed opacity (TR-015) × animation opacity
     factor, applied to the whole assembled layer (styling included).
   - **L10 Blend** — composite the layer onto the accumulator once, with the
     clip's blend mode. OV-004 owns `BlendMode`: the list, the formulas, the
     serialization and the hover preview; TR-016 only places the dropdown.
     This stage executes OV-004's formula for the mode (for Normal that is
     premultiplied source-over, `out = src + dst × (1 − src.a)`). FX fixes only
     the working color space the blend runs in (L2) and that each layer is
     blended exactly once, after L9.
4. **Text, stickers, shapes and captions** (TX/OV) produce their own
   premultiplied layer image (rasterization plus their own animations) and
   enter at L8, then L8b (elements only; text and captions draw their own
   outline and shadow while rasterizing, TX), L9 and L10 with the same rules.
   Raster stickers that carry a CL color stack run L5 on their image first
   (OV-018). Caption segments are rasterized from TX-025's payload and enter
   in the Caption band (step 2).
5. **Transition pair.** A and B are each rendered through L1–L9 into separate
   transparent canvas-sized textures, each side sampled at its own source time
   for `t` (B's pre-roll, and in handle mode A's post-roll, FX-002/FX-008). The
   transition shader combines them into one texture using progress `p`
   (FX-002/FX-005), and that result goes through L10 using **B's blend mode**
   (FreeCut default; in the common case both clips use Normal and nothing
   changes).
6. **Effect layers and filter/adjustment layers.** When the walk reaches such
   an item, its input is the accumulator (everything beneath it, including the
   background); its output replaces the accumulator, mixed by the item's Mix
   amount times its region coverage (FX-027; 1 without a region). Scope
   "Main track only" (FX-011) instead appends the effect to the
   end of the main-track clip's L6 stack for that time range. Because the
   Caption band is composited after every effect and adjustment track,
   these layers never alter captions.
7. **Output.** The opaque accumulator is handed to preview (scaled to the
   viewer) or to export, which converts it to the output pixel format and color
   range (EX).

Determinism rules (preview = export):
- One set of WGSL shaders serves both paths; no preview-only shortcuts that
  change the look, only lower resolution.
- Every spatial parameter is resolution-independent (expressed as a fraction
  of layer or canvas height), so proxy or half-resolution preview looks the same.
- Anything random uses a stored per-instance seed hashed with the frame index —
  never wall-clock time or frame-delivery order.
- Shader time inputs (clip-local time, effect-local time, progress) are computed
  as rationals and converted to `f32` only when uniforms are uploaded.

**Edge cases:**
- No item active at `t` on any track → background only.
- Hidden track → skipped entirely, including effect layers on it. Muted-only
  (audio) state has no visual effect.
- Opacity 0 or a layer fully off-canvas → may be skipped as an optimization; the
  output must be identical to rendering it.
- Effect layer with nothing beneath it at `t` (e.g., over a main-track gap) →
  affects only the background.
- Sources at a different frame rate → L1 picks the source frame at or before the
  mapped source time in MVP (no frame blending); later sampling modes such as
  frame blend and optical flow are defined in TR-041/TR-042.
- Sources at a different resolution or aspect → handled at L2 and by the fit
  rule in TR; effects see the layer-space image, not the raw source.
- HDR or wide-gamut sources → handled at L2 exactly as CL-002 defines (PQ/HLG
  BT.2020 is tone-mapped to SDR Rec.709 with its filmic curve in the same
  conversion shader); FX adds no rule of its own.
- Caption tracks → always composited last (step 2), even with Flexible
  stacking on; a hidden caption track is skipped like any hidden track
  (TX-025). Users who want captions under other layers convert them to text
  clips (TX-035).
- Element styling larger than the layer (a wide shadow or outline) → drawn in
  canvas space at L8b, so it is not clipped by the L6 layer-space bounds; it is
  clipped only by the canvas.
- PNG/WebP stills and alpha video (e.g., ProRes 4444, VP9 with alpha) keep alpha
  through every stage; transitions and blends see correct transparency.
- A transition pair where one side is offline → that side uses the placeholder.
- Heavy stacks → correctness never degrades; preview may drop frames (EX owns
  the playback policy and render cache).

**Undo:** n/a (a rendering rule; every input it reads is changed by an undoable
edit owned by its area).

**Model notes:** `freecut-timeline` exposes a pure query, e.g.
`Timeline::frame_plan(t) -> FramePlan`, listing, bottom to top in TL's
validated track order with the Caption band last, each active layer with its
local time (measured over its visible range) and source time,
transform/animation state, and transition pairs with exact rational progress
and each side's source time. It has no GPU types and does not re-check
layering (TL-004's `Sequence::validate()` does). `freecut-render` turns a
`FramePlan` into a wgpu pass graph with a fixed stage enum (L1, L2, L2b, L2c,
L3–L8, L8b, L9, L10, Transition, EffectLayer) so the order is enforced in one
place. Blend formulas come from OV-004's `BlendMode` and its CPU reference
implementation. Tests: plan construction (including caption-band ordering
under Flexible stacking) is unit-tested in the timeline crate; stage order and
blend math get golden-image tests at tiny resolutions (e.g., 16×16) in a
headless wgpu device in the render crate.

### FX-002 Transitions between adjacent clips — mvp

**What the user sees:** Each cut on a track shows a small marker. Dropping a
transition from the library onto the marker (or using the library tile's "+"
button) places a transition badge over the transition region, which ends at
the cut. The badge's width is its duration; dragging either of its edges
changes it, and the region stays anchored to the cut. Selecting it opens its
settings in the inspector. The track becomes shorter by the transition's
duration: the incoming clip hands its first stretch of footage to the
transition, which plays it over the last stretch of the outgoing clip, and the
incoming clip continues from there after the cut. No footage is skipped or
shown twice.

**Inputs / controls:**
- **Add:** drag from the Transitions library onto a cut (snaps to the nearest
  cut within ~12 px; FreeCut default), or click "+" on a tile. With "+", the
  target cut is picked from where the playhead sits inside the clip under it:
  closer to that clip's start → its leading cut; closer to its end → its
  trailing cut. When the clip has no neighbour on that side, FreeCut uses the
  opposite cut. Track choice: the track of the selected clip; otherwise the
  main track; otherwise the lowest track with a clip under the playhead.
- **Duration:** default 0.5 s (FreeCut default; public guides call 0.5–1 s
  typical). Minimum 0.1 s, rounded up to whole frames, and never less than
  1 frame. Maximum is the smallest of: 10 s (FreeCut default); **50 % of the
  shorter neighbouring clip's visible length** (adding the transition does not
  change either clip's visible length, so the cap is the same before and after);
  and the source B has before its current in-point, which feeds the pre-roll.
  That last limit is always met right after adding, because the pre-roll
  frames came from B itself; it only matters once B's head is extended later.
  Public tutorials describe the same half-of-the-shorter-clip ratio
  for the reference editor; it guarantees that a clip's pre-roll and its own
  outgoing transition region never overlap. Values snap to whole project
  frames. The inspector shows seconds to 0.1 s and frames in a tooltip.
- **Replace:** drop a different transition on an existing one. It keeps the
  current duration (clamped to the new kind's limits) and resets kind-specific
  parameters to their defaults.
- **Remove:** select the badge and press Delete, or right-click → Remove
  transition.

**Behavior / output:**
- **Default placement — "Shorten track" (FreeCut default).** Clip ranges on a
  track never overlap (TL-004), and adding a transition keeps it that way.
  `AddTransition` of duration `D` at cut A|B does exactly this:
  1. B's source in-point advances by the source span B plays in its first `D`
     (`D × speed` at constant speed; through TR-036's time map for speed curves;
     from the other source side for reversed clips, TR-043). Stills have no
     time map and simply get shorter.
  2. B's start stays at the cut, so B becomes `D` shorter.
  3. Every later item on that track ripples by −`D`. Items on other tracks
     follow TL's ripple rules for a ripple edit on this track. Items attached
     to B (TL-029) are content-anchored, so they move with B's footage exactly
     as for a ripple head-trim of B by `D`.
- **Placement (observed conflict).** The reference app's UI keeps the
  timeline length: the clips overlap into footage beyond the cut, and when
  that footage is missing it offers to repeat edge frames. Public tutorials
  describe the shortening behaviour above. FreeCut keeps *Shorten track* as
  the default because it always works without spare footage; the
  length-keeping behaviour is FX-008.
- **What renders.** The transition region is `[cut − D, cut)`. In it, A shows
  its own last `D` frames (no handle needed) and B shows its pre-roll: its
  source `[in − D, in)`, meaning the source just before its new in-point that
  plays in `D` through its time map. Those are exactly the frames B showed in
  its first `D` before the transition was added. The track renders
  the transition pair (FX-001 step 5) with progress
  `p = (t − region_start + ½·frame) / D`. Mid-frame sampling makes an N-frame
  transition symmetric and means no frame inside it is purely A or purely B.
  After the cut, B plays from its new in-point.
- **Remove.** `RemoveTransition` is the exact reverse: B's in-point moves back
  by the same source span, B's start stays at the cut, B grows by `D`, and
  every later item on the track ripples by +`D`.
- **Change duration.** Going from `D` to `D′` (edge drag, inspector, or a
  clamp) moves B's in-point by the span for `D′ − D` and ripples later items by
  −(`D′ − D`), all in one op.
- **Visible range.** B's visible range starts at the region start (`cut − D`);
  A's visible range is unchanged (its region lies inside its stored range).
  Clip-local time, clip effects and clip animations of B are measured from its
  visible start (FX-010, FX-018), so they line up with B's footage exactly as
  before the transition was added.
- FX-008 adds an optional placement that keeps the timeline length.
- Allowed only when: both items are video or image clips; they are on the same
  track; A's end equals B's start (no gap); the track is unlocked; each clip's
  visible length is at least 2 × minimum duration. Transitions are not offered
  between text, stickers, captions or effect items (they use their own
  animations), across tracks, or at the start or end of the timeline. A fade
  from or to the background uses an Entrance or Exit animation (FX-018)
  instead.
- Track choice (observed conflict): the reference app's current UI allows
  transitions only on its main track, while its older release notes also
  allowed them between overlay clips. FreeCut allows them on any video track
  as long as both clips sit on that same track; "main track only" is the
  rejected alternative.
- One transition per cut. A clip can have one at each end: its incoming
  transition's region lies in the previous clip's tail, and its outgoing
  transition's region lies in its own tail.

**Edge cases:**
- Single clip, or clips separated by a gap → the "+" button and drop target are
  disabled, with a hint saying why (e.g., "Needs two touching clips on the
  same track"). Quick add with no touching pair near the playhead → the same
  hint; nothing changes and no undo entry is made.
- Clips too short for the minimum duration → disabled, hint "Not enough clip length
  here for a transition".
- A transition dragged onto a text, sticker, effect, caption or audio track,
  or onto empty timeline space → the drop is refused (not-allowed cursor) with
  the same hint.
- Transitions removed as a side effect of another edit (the adjacency rule
  below, a clip moved to another track, a cap that falls below the minimum) →
  an info toast (UI-040) says how many were removed, naming at most two by
  their clip pair and then "+N more". FreeCut does not ask first, because the
  edit is one undo step that brings them back.
- An edit makes a neighbour's visible length shorter than `2 × D` (trim, speed
  change, split elsewhere) → the same undoable operation clamps `D` to the new
  cap, with B's in-point and the ripple adjusted to match. If the cap falls
  below the minimum, that operation removes the transition (as
  `RemoveTransition` does) and records it, so undo restores it.
- B's head extended at the cut (a ripple head-trim on the magnetic main track,
  TL-020) → B's in-point moves earlier, leaving less source before it for the
  pre-roll. If less than `D` remains, the same op clamps `D` (or removes the
  transition below the minimum). Stills have unlimited pre-roll.
- An edit that ends the adjacency of A and B (moving either away, deleting
  either, a trim that opens a gap at the cut, reordering on the magnetic main
  track) → the transition is removed in the same op exactly as
  `RemoveTransition` does (B's head restored, later items +`D`) before the edit
  itself is applied. The result equals that edit applied to a timeline on which
  the transition was never added. A moved B therefore arrives at its
  destination with all its footage. Transitions whose two clips are still
  adjacent are kept.
- Ripple-deleting A or B → the rule above removes transitions at both of its
  cuts.
- Splitting at a point inside a transition region (inside A's last `D`) → split
  is disabled with the hint "Shorten or remove the transition first" (FreeCut
  default). Splitting B elsewhere is fine: the left part keeps B's incoming
  transition and the right part its outgoing one (TL-023).
- Locked track → no add, remove or edit. Hidden track → transitions are kept and
  not rendered.
- Clips with different resolutions, aspect ratios or frame rates → allowed; each
  side is normalized by FX-001 before the transition shader runs.
- Clips with alpha over lower tracks → the transition works on premultiplied
  RGBA, so lower tracks show through correctly during the transition.
- Overlay (non-magnetic) tracks → the same rule; only items later on that track
  ripple. On the magnetic main track the ripple keeps the track contiguous from
  0 (TL-005).
- Speed change on B (TR-037) → `D` stays the same. The same op re-derives B's
  in-point for the new speed so that the region still starts on the same
  footage frame, ripples by the resulting length change as TR does, and
  re-checks the cap.
- B with a speed curve → step 1 cuts the curve exactly (TR-036), and the
  pre-roll follows TR-036's rule that the map continues at the edge speed past
  the authored range. So the frames inside the region can differ slightly from
  what the removed piece of the curve showed. Undo restores B's exact previous
  range and curve.
- Project frame rate changed later (PM) → durations are re-snapped to the new
  frame grid, rounding to nearest and clamping to the cap, with B's in-point
  and the ripple adjusted in the same migration op.

**Undo:** `AddTransition { track, left_clip, kind, params, duration }` ↔
`RemoveTransition`. Both store the full transition, B's previous source range
and retime, and the ripple shift they caused, so each is exactly reversible.
`SetTransitionDuration { old, new }` (one undo step per drag gesture, including
B's in-point change and the ripple). `ReplaceTransition { old_kind, old_params,
new_kind, new_params }` (no ripple when the duration is unchanged).

**Model notes:** In `freecut-timeline`, a track keeps
`transitions: BTreeMap<ClipId /* left clip */, Transition>` with
`Transition { id, kind: TransitionKindId, duration: RationalTime, params: ParamSet, easing: Easing, audio_crossfade: bool, placement: Placement }`.
`easing` reuses the easing type owned by TR (`clip-transform-and-speed.md`,
TR-031) rather than defining a second one. `TransitionKindId` is a namespaced
string (e.g., `"freecut.dissolve"`) so the timeline crate never depends on the
render crate.
- Stored clip ranges are the only positions; a transition adds no range of its
  own and never makes clips overlap. TL-004's `Sequence::validate()`, magnet
  contiguity (TL-005), ripple (TL-021), snapping (TL-028) and split (TL-023) see
  two ordinary butted clips. The only transition-specific rule they need is
  refusing a split inside a region.
- Pure functions derive the rest: `transition_region(track, left) -> TimeRange`
  and `visible_range(track, clip) -> TimeRange`. Each side's source time comes
  from TR-036's `source_time_at`, which already continues past in/out at the
  edge speed.
- `AddTransition`, `RemoveTransition` and `SetTransitionDuration` are built
  from TL primitives: a head trim of B through its time map plus TL's
  `RipplePlan` for the later items.
- Validity (adjacency, cap, pre-roll available) is a pure function with unit
  tests. Property tests: Add then Remove restores every stored range exactly;
  after Add, that track's layer is unchanged before the region and, from the
  cut on, equals its old layer `D` later; an adjacency-ending edit equals
  Remove followed by that edit.
- `freecut-project` serializes it. On load, an unknown kind is kept verbatim
  (forward compatibility), rendered as a cross-dissolve, and flagged in the UI.

### FX-003 Cross-dissolve — mvp

**What the user sees:** The one MVP transition. The outgoing clip gradually
becomes the incoming clip.

**Inputs / controls:** Duration (FX-002). From v1, easing (FX-005); default
linear.

**Behavior / output:** `out = A × (1 − e(p)) + B × e(p)` on premultiplied RGBA,
where `e` is the easing function (identity in MVP). Opaque clips give a classic
dissolve. Clips with alpha dissolve both color and coverage consistently.

**Edge cases:**
- 1-frame transition → a single 50/50 frame (mid-frame sampling).
- A and B identical (e.g., the same shot split) → visually a no-op; allowed.
- Very long dissolve on stills → allowed up to the cap.

**Undo:** As FX-002.

**Model notes:** The render crate registers `"freecut.dissolve"` with a WGSL
fragment shader that takes two textures and a progress uniform. It is the
fallback kind for unknown transitions.

### FX-004 Transition library — v1

**What the user sees:** A Transitions tab in the library panel with categories
of animated tiles. All entries are built in, offline and free: a tile never
shows a download, retry or locked state. The reference app lists
dozens of transitions; FreeCut's initial set is in that range, and more can be
added from local packs (FX-025).

**Inputs / controls:** Category tabs, search, favorites and hover preview are
shared with the other libraries (FX-016). Initial set (about 40 entries,
FreeCut-made, described by look):
- **Dissolves and dips:** cross-dissolve; dip through a color (black, white,
  custom); additive (brightening) dissolve; blur dissolve; brightness-ordered
  fade (dark areas switch first).
- **Wipes:** straight wipe at any angle; clock and radial wipe; iris (circle,
  diamond, rectangle); barn-door split; blinds; checkerboard; pattern wipes
  driven by our own grayscale maps.
- **Slides and pushes:** push (both clips move); slide over; uncover; split
  doors; slice bands.
- **Zoom and spin:** zoom-through; zoom-blur punch; 2D spin; spin with motion
  blur.
- **Camera-style:** whip pan (directional motion blur in the travel direction);
  shake cut.
- **3D:** cube rotate; card flip; swinging door.
- **Light:** flash to white; procedural light-leak sweep; glow burst; exposure
  bloom.
- **Glitch and distortion:** RGB channel split; block displacement; pixelate
  through; ripple/wave; swirl.
- **Shape reveal:** growing shape (circle, heart, star, from our own vector
  shapes); brush-stroke reveal from our own grayscale maps.

**Behavior / output:** Each entry is a manifest (id, display name, category,
tags, parameter schema, default duration) plus a WGSL shader taking A, B,
progress and parameters. Tiles show a short loop generated locally by our
renderer from bundled CC0 sample stills, made on first use and cached.

**Edge cases:**
- A transition whose shader fails to compile on the user's GPU → the tile is
  hidden. Existing uses render as a dissolve and get a warning badge.
- Library search with no results → empty state with "clear search".

**Undo:** n/a (browsing). Applying one uses FX-002.

**Model notes:** Registry in `freecut-render` (`TransitionRegistry`); the
manifests are data compiled into the binary. `freecut-app` reads the registry
for the panel. Tile loops are rendered by `freecut-render` in a background job
started by `freecut-app` (never on the UI thread) and written to the app's
cache directory by `freecut-app`; nothing about them enters the project or the
timeline model.

Acceptance tests for every bundled transition (golden images at 16:9, 1:1 and
9:16 canvases, at the default and at the minimum and maximum of every
parameter):
- The look does not change with the canvas aspect ratio, apart from the
  geometry it is meant to follow (e.g., a wipe angle stays an angle).
- No jump at either end: the first frame of the region is within one frame's
  progress of pure A, the last within one frame's progress of pure B.
- Neither side flickers, blinks to the background or skips a frame inside the
  region.
- Parameters change the look in the expected direction and stay stable (no
  NaN, no black frames) at their extremes.

### FX-005 Transition parameters — v1

**What the user sees:** Selecting a transition shows its duration and the
controls its kind defines, with a reset button per control.

**Inputs / controls** (all FreeCut defaults):
- **Common:** Duration (FX-002); Easing — any preset from TR-031 except Hold
  (linear, ease in/out/in-out at its strengths, overshoot, elastic, bounce),
  applied to transition progress. Dissolves default to linear, motion-based
  kinds to ease-in-out (FreeCut defaults). Reverse direction toggle (default
  off).
- **Kind-specific**, shown only when relevant:
  - Direction: left/right/up/down, or an angle 0–359° (default 0° = left to right).
  - Softness/feather of the edge: 0–100 % (default 10 %). Not shown for dissolves.
  - Color for dips and flashes (default black for dip, white for flash).
  - Center point for iris and zoom kinds: normalized 0–1 (default canvas
    center); draggable on the viewer.
  - Count for blinds and checkerboard: 2–64 (default 8).
  - Intensity/amount: 0–100 (default 50).
  - Motion blur amount for push and whip kinds: 0–100 (default 50).
- Transition parameters cannot be keyframed (progress is the animation).

**Behavior / output:** Parameters are uniforms of the transition shader. Easing
maps `p` before the shader sees it; eased values outside 0–1 (overshoot,
elastic) are clamped to 0–1 for dissolve-type kinds and passed through for
motion kinds (FreeCut default).

**Edge cases:**
- Switching kind → parameters that don't exist in the new kind are dropped;
  duration and easing are kept.
- Center point outside 0–1 → allowed (iris starts off-canvas) and clamped to
  −1…2.
- Loaded value out of range → clamped with a load warning.

**Undo:** `SetTransitionParams { old, new }`, one step per slider drag.

**Model notes:** `ParamSet` is a map from parameter key to typed value (scalar
`f64`, angle, color, point, int, bool, enum). Time-valued parameters use
`RationalTime`, never floats. Unknown keys are preserved.

### FX-006 Apply to every cut — v1

**What the user sees:** With a transition selected, an "Apply to every cut"
button. One click puts the same transition (kind, parameters, duration) on every
cut of that track. A short toast reports the result, e.g. "Applied to 12 cuts,
skipped 2 (too short)".

**Inputs / controls:** Scope: this track (default), or the cuts between the
currently selected clips when two or more adjacent clips are selected. Existing
transitions in scope are replaced.

**Behavior / output:** For each eligible cut in scope, add or replace the
transition, clamping the duration to that cut's cap. Ineligible cuts (gaps,
non-media items, clips too short, locked track) are skipped. Each add or
replace shifts the incoming clip's in-point and ripples as FX-002 defines. The
shifts accumulate, so the track shortens by the sum of the added durations
(minus the durations of any transitions that were replaced).

**Edge cases:**
- Every cut ineligible → nothing changes, no undo entry, toast explains why.
- Mixed existing transitions → all replaced. There is no "only empty cuts"
  option in v1.
- A large track (hundreds of cuts) → applied as one batched operation with a
  single layout pass.

**Undo:** One compound `ApplyTransitionToCuts { per_cut: Vec<(cut, previous: Option<Transition>, new)> }`
that restores every previous transition (or absence) and position.

**Model notes:** Pure batch op in `freecut-timeline`, unit-tested with mixed
eligible and ineligible cuts.

### FX-007 Audio during transitions — mvp

**What the user sees:** When two clips with sound are joined by a transition,
the sound crossfades smoothly across the transition instead of the two clips
playing over each other at full volume.

**Inputs / controls:** Per transition, "Crossfade audio" toggle, default on.

**Behavior / output:** Within the transition region, the audio attached to A
(embedded/linked audio) fades out and B's fades in using the equal-power gain
law defined in `audio.md` (AU-029); FX only supplies the region and progress
`p`. Each side's audio is read through the same time map as its picture
(TR-038), so B's audio in the region is its pre-roll (and in handle mode A's
is its post-roll, FX-008). This multiplies with each clip's own volume,
keyframes and fades (AU). With the toggle off, both play over each other
unchanged. Audio clips on separate audio tracks are never affected. Gains are
evaluated per sample from the rational region start and duration, so the
crossfade is sample-accurate at any output sample rate.

**Edge cases:**
- Only one side has audio, or one side is muted → that side alone gets its half
  of the curve (a fade-out of A or a fade-in of B).
- Audio detached from its video (TL/AU) → no longer part of this transition;
  `audio.md` (AU-003) decides whether it becomes an audio crossfade or a short
  fade on the detached clip.
- Mismatched source sample rates → resampled by the audio crate before mixing;
  the curve is applied at the mix rate.
- Very short transitions (1–2 frames) → still crossfaded, which avoids clicks.

**Undo:** `SetTransitionAudioCrossfade { old, new }`. The crossfade is otherwise
derived, not stored.

**Model notes:** `freecut-timeline` exposes transition regions and each side's
source time for audio (rational).
The mixer in `freecut-audio` applies AU's equal-power law. Because this feature
is MVP while AU-029 is v1, that one gain law has to ship with the MVP mixer.

### FX-008 Transitions that keep timeline length (handle mode) — later

**What the user sees:** A per-transition "Placement" choice: *Shorten track*
(default, FX-002) or *Use extra footage — keeps length*. In the second mode
nothing on the timeline moves and no clip changes length; the transition uses
footage beyond the cut instead. Alignment: centered on the cut (default),
starting at the cut, or ending at the cut. This matches what the reference
app's UI describes as its normal behaviour (length kept, edge frames repeated
when spare footage runs out); FreeCut offers it as an option (FX-002).

**Inputs / controls:** Placement, alignment, and duration (same limits as
FX-002, further limited by the available handles).

**Behavior / output:**
- No stored range changes, no in-point moves and nothing ripples. Clip ranges
  stay butted and never overlap (TL-004).
- The transition region depends on the alignment:
  - Centered: `[cut − D/2, cut + D/2)`.
  - Starting at the cut: `[cut, cut + D)`.
  - Ending at the cut: `[cut − D, cut)`.
- Inside the region each side is sampled through its own time map (TR-036).
  Before the cut, A shows its stored frames; from the cut on, A shows
  post-roll, the source after its out-point. Before the cut, B shows pre-roll,
  the source before its in-point; from the cut on, B shows its stored frames.
- So a centered transition needs `D/2` of unused source after A's out-point and
  `D/2` before B's in-point. Starting at the cut needs `D` after A. Ending at the
  cut needs `D` before B.
- Handles in timeline time are source handles divided by the local speed at
  that edge; reversed clips take handles from the other source side (TR-036,
  TR-043). Stills have unlimited handles.
- A's visible range grows by its post-roll and B's by its pre-roll (FX-002),
  so clip effects and animations stay aligned with the footage.
- The default *Shorten track* placement is the "ending at the cut" region whose
  B handle is created by shifting B's in-point and rippling (FX-002). Handle
  mode is the same region rule using handles that already exist.

**Edge cases:**
- Not enough handle → a dialog offers: shorten to the available duration;
  hold the last/first available frame for the missing part (edge-frame
  padding, so both clips keep their length); or cancel. "Apply to every cut"
  skips such cuts and reports them.
- Held (padded) part → shows one steady frame, never a flicker or a blink to
  the background; the transition badge marks the padded stretch with hatching
  so the freeze is not a surprise. The hold choice is stored on the
  transition (`pad_with_held_frames`) and re-evaluated when handles change.
- Switching *Shorten track* → *Use extra footage* → first undoes FX-002's
  shift (B's in-point moves back, later items ripple +`D`), then needs real
  handles as above. Switching back applies FX-002's shift and ripples −`D`.
  Both directions are one undoable step.
- Reversed or speed-ramped clips → handle availability is computed through the
  retime mapping (TR-036).
- A split inside the region is refused as in FX-002; in this mode the region
  can also reach into B's head.
- Trims that consume a handle (extending A's tail or B's head into the footage
  the transition reads) → the same op clamps `D` to what remains, or removes the
  transition below the minimum.
- An edit that ends the adjacency of A and B → the transition is removed in the
  same op and nothing ripples, because handle mode never shifted anything.

**Undo:** `SetTransitionPlacement { old, new, b_range_change, ripple }`.

**Model notes:** `Placement::{ShortenTrack, Handles { alignment: Alignment, pad_with_held_frames: bool }}`
with `Alignment::{Centered, StartAtCut, EndAtCut}` already sits on
`Transition` in FX-002, so this adds no migration. `transition_region` and `visible_range`
(FX-002) take the placement into account; no other model change is needed,
because handle mode never touches stored ranges.

### FX-009 Default transition and quick add — v1

**What the user sees:** A command (and right-click item on a cut) "Add default
transition" puts the user's default transition at the cut nearest the playhead.
With several adjacent clips selected, it adds one at every cut between them.

**Inputs / controls:** Preferences: default transition (cross-dissolve) and
default duration (0.5 s). The key binding is owned by UI (suggested Ctrl/Cmd+T).

**Behavior / output:** Same rules as FX-002, using the preference values.

**Edge cases:**
- No eligible cut near the playhead → status-bar hint, no undo entry.
- Default transition missing (e.g., a removed custom luma) → falls back to
  cross-dissolve.

**Undo:** As FX-002 (or one compound step for multiple cuts).

**Model notes:** Preferences live in `freecut-app`'s user config, not in the
project. The add itself calls the same pure `freecut-timeline` op as FX-002.

### FX-010 Clip effects — v1

**What the user sees:** Dragging an effect from the Effects library onto a clip
attaches it to that clip: the clip gets an "fx" badge, and the inspector's
Effects section lists it. Clicking the badge, or *Edit effects* in the clip's
context menu, opens that section. The effect covers the whole clip (or only
its region, FX-027) and follows it when the clip is moved, trimmed, split or
copied.

**Inputs / controls:**
- Drop on a clip; or select clips and click "+" on a tile (adds to every
  selected visual clip). With no clip selected, "+" creates an effect layer at
  the playhead (FX-011), except for clip-only effects (FX-015 manifest), which
  show the hint "This effect goes on a clip — pick one on the timeline".
- *Apply to all clips…* (FX-026) copies one effect to many clips at once.
- Parameters are in FX-012.

**Behavior / output:** The effect runs at L6 for the clip, using clip-local time
for animated effects: time since the clip's visible start (FX-002), which is
its timeline start, or the start of its incoming transition region. That keeps
an effect aligned with the same footage whether or not a transition is added.
Video and image clips only in v1. Text and stickers are affected through effect
layers placed above them.

Feedback: a single add shows no toast, because the badge and the inspector
already show the result (FreeCut default; the reference app confirms every
add and every edit with a toast). Batch adds and refusals report through a
toast (UI-040).

**Edge cases:**
- The same effect added twice → allowed, stacked twice.
- More than 16 effects on one clip → allowed, with a performance hint.
- Split → both halves get independent copies (new ids). Keyframes follow
  TR-027: they are source-anchored, both halves keep all of them unchanged,
  and no boundary keyframe is inserted, so the output does not change. Seeds
  are copied, so the random pattern continues seamlessly.
- Clip on a locked track → no add or remove.
- Dropped on a text or sticker item → refused, with the hint "Use an effect
  layer above it".
- Media-type mismatch: an effect whose manifest accepts only video or only
  stills (FX-015; e.g., photo parallax, FX-029, is stills-only) dropped on the
  other kind → refused with a hint naming the kind it needs. In a multi-clip
  "+", such clips are skipped and counted in the toast.
- Conflicts: effects stack freely and adding one never asks about conflicts,
  except for kinds whose manifest declares an exclusive group (at most one
  member per clip, e.g., FX-029's photo parallax, or the background treatments
  of FX-020). Adding a second member of the group asks "Replace <name>?" and
  warns that the old effect's keyframes go with it. Replace is one undo step.
- "+" while the timeline has no visual items → hint "Put a clip on the
  timeline to use effects"; nothing is added (FreeCut default; an effect layer over the
  bare background is the alternative).
- Deleting a clip deletes its effects without a confirmation; undo restores
  both (FreeCut default; the reference app asks first and names up to two of
  the effects that will go).
- An effect never locks its clip's position or length. Effects bound to an
  analysis (FX-020, FX-021, FX-028, FX-029) analyse any newly exposed source
  range instead.
- Compound clips (TL-040) take ordinary clip effects on their composite
  output. Effects that need per-source analysis are refused there (FX-020).
- Offline media → the effect is kept and applied to the placeholder.
- Effect needing frame history ("temporal" effects such as echo or trails) →
  history comes only from this clip's own earlier frames (never across a cut).
  It is capped at 8 frames (FreeCut default) and recomputed deterministically
  after seeks.

**Undo:** `AddClipEffect { clip, index, effect }` ↔ `RemoveClipEffect`
(stores the effect and its index). An exclusive-group replace is one
`ReplaceClipEffect { clip, index, old, new }` that keeps the old effect, its
keyframes included, for undo.

**Model notes:** `Clip.effects: Vec<EffectInstance>` with
`EffectInstance { id, kind: EffectKindId, enabled: bool, params: ParamSet, seed: u64, region: Option<EffectRegion> }`
(`region` is FX-027's; `None` means the whole frame). The kind id is a
namespaced string, so the timeline crate does not depend on render. Media-type
and exclusive-group checks read a small `EffectTraits` record that
`freecut-app` passes in from the render crate's manifest, so the pure add op
can refuse or replace without depending on `freecut-render`.

### FX-011 Effect layers — v1

**What the user sees:** Dragging an effect onto an empty part of the timeline (or
onto an effect track) creates an effect item on its own track, which can be
moved, trimmed and split like a clip. For its time range it changes everything
beneath it in the track stack — clips, text, stickers and background — and
nothing above it. Captions are never affected, because the Caption band is
always above every effect track (TL-004).

**Inputs / controls:**
- Placement: dropped at the drop position, or at the playhead when added with
  "+" while no visual clip is selected (FX-010). Default duration 3 s
  (FreeCut default). A new effect track is created
  directly above the topmost non-caption track that has content at that time
  (so always below the Caption band), unless one is dropped on an existing
  effect track. Under strict layering it lands in the upper band (TL-004/TL-006
  placement rules).
- **Applies to:** "Everything below" (default) or "Main track only".
- **Mix:** 0–100 % (default 100 %), which blends the effect's output with its
  input. It can be keyframed (FX-014), e.g. for fade-in.
- **Region:** whole frame (default) or a mask in canvas space, optionally
  tracked to a moving object (FX-027).
- Effect parameters as in FX-012.

**Behavior / output:** Rendered at FX-001 step 6 with effect-local time (time
since the item's start). "Main track only" makes the effect behave like a clip
effect on whatever main-track clip(s) are under it, for the overlapping range
only. Across a main-track transition, it applies to both sides before they are
mixed.

**Edge cases:**
- Two effect items overlapping in time on the same track → not allowed (TL item
  overlap rules); the drop goes to a new track instead.
- Effect layer over a gap or past the last clip → it affects the background;
  the timeline length extends to include it (TL rule for any item).
- Effect layer spanning a transition (scope "Everything below") → applies to the
  transition's output.
- Hidden or locked effect track → not rendered / not editable.
- Clip-only effects (FX-015 manifest; e.g., the People effects of FX-020)
  dropped on empty timeline space → refused with the hint "Drop this effect on
  a clip".
- Scope "Main track only" while the main track is empty at `t` → no-op.
- Track reorder moves an effect layer below a layer → that layer is now
  unaffected. This is intended and is the way to exclude a layer. Under strict
  layering this works within the upper band (e.g., below a sticker track);
  moving it below a video overlay needs Flexible stacking (TL-007). An effect
  layer can never be moved into or above the Caption band.
- Effect wanted on captions → convert them to text clips first (TX-035), which
  places them on a text track where effect layers can sit above them.

**Undo:** Creation, move, trim, split and delete use TL's generic item
operations. `SetEffectLayerScope { old, new }`; `SetEffectMix` is coalesced per
drag.

**Model notes:** `EffectLayer { id, range: TimeRange, effect: EffectInstance, scope: EffectScope, mix: Param }`
is a timeline item kind (TL defines item and track kinds). Filter and adjustment
layers (CL) use the same position in the render walk.

### FX-012 Effect parameters — v1

**What the user sees:** Selecting a clip effect (in the inspector list) or an
effect layer shows that effect's controls under a header with the effect's
display name and category icon. Each control has a reset; there is also
"reset all", and an eye toggle to bypass the effect without deleting it.
Which controls appear depends on the effect. The viewer updates live while
dragging.

**Inputs / controls:** Defined by each effect's manifest. Control types:
- Slider with min, max, default and unit. FreeCut's common conventions:
  Intensity 0–100 (default set per effect, typically 50–80); Speed 0–100 (maps
  to a per-effect cycle rate, default 50); Size/Scale 0–100; Range/Spread 0–100.
- Angle 0–359°, color (with alpha), on-canvas point (normalized 0–1), toggle,
  choice list.
- Seed (integer) with a "shuffle" button.
- All spatial values are resolution-independent (FX-001).
- Shared parameter vocabulary. Manifests reuse these keys, labels and ranges
  so the same word always means the same thing (all FreeCut defaults; the
  list matches the kinds of controls the reference app's effects expose):

  | Key | Label | Range | Default |
  |---|---|---|---|
  | `intensity` | Intensity | 0–100 | per effect (50–80) |
  | `amount` | Amount | 0–100 | 50 |
  | `speed` | Speed | 0–100 | 50 |
  | `range` | Range | 0–100 | 50 |
  | `rotation` | Rotation | −180…180° | 0° |
  | `shift_x`, `shift_y` | Horizontal / Vertical shift | −100…100 | 0 |
  | `aberration_x`, `aberration_y` | Horizontal / Vertical color fringe | −100…100 | 0 |
  | `distortion` | Distortion | 0–100 | 50 |
  | `glow` | Glow | 0–100 | 50 |
  | `noise` | Noise | 0–100 | 0 |
  | `texture` | Texture | 0–100 | 50 |
  | `sharpen` | Sharpen | 0–100 | 0 |
  | `soft_light` | Soft light | 0–100 | 0 |
  | `frequency` | Frequency | 0–100 | 50 |
  | `tint` | Tint amount (the effect's own built-in color cast) | 0–100 | per effect |
  | `backdrop` | Backdrop amount (for effects that add a generated background layer) | 0–100 | per effect |

  Choice lists use the same pattern; for example glow-type effects have
  *Direction*: Inner, Outer (default) or Both.

**Behavior / output:** Values become shader uniforms. A disabled effect is
skipped at render.

**Edge cases:**
- Loaded value out of range → clamped, with a non-blocking load warning.
- Missing key (file from an older FreeCut) → default value.
- Unknown key (file from a newer FreeCut) → preserved on save.
- Unknown effect kind → kept, bypassed at render, shown as "Unavailable effect"
  so it is never silently dropped. Its panel offers *Replace with…* (pick a
  bundled effect; parameters with matching keys carry over) and *Remove*. It
  renders again as soon as a newer build or an installed pack (FX-025)
  provides the kind, so nothing has to be re-applied. Export is not blocked;
  export preflight (EX-036) should list unavailable effects, transitions and
  animations as a warning, the way it lists missing LUTs.
- Effects never point at a LUT file. A look that needs one is a CL filter,
  and its missing-LUT state is CL-016's.
- Multi-selection of clips with the same effect → editing a shared control
  changes all of them (one undo step); differing values show as "mixed".
- Several effects selected at once (rows in one clip's list, the same effect
  on several clips, or several effect layers) → edited together when they are
  all the same kind. With different kinds the panel shows "These effects
  don't share controls — pick effects of one kind to edit them together" and
  no controls (FreeCut default; showing only the parameter keys
  they share is the alternative).

**Undo:** `SetEffectParams { target, effect_id, old, new }` (one step per
gesture); `SetEffectEnabled { old, new }`; `ResetEffectParams` stores the old
values. A batch edit is one `SetEffectParams` per target inside one compound
step. *Replace with…* is FX-010's `ReplaceClipEffect` (for an effect layer,
the same op on its `EffectInstance`).

**Model notes:** The manifest schema lives in `freecut-render`. The timeline
stores only `ParamSet` values plus the kind id. The app builds controls from the
manifest.

### FX-013 Effect stacking order — v1

**What the user sees:** A clip's effects appear as an ordered list in the
inspector. A tooltip says they run top to bottom. Items can be dragged to
reorder, or moved with "Move up" / "Move down". Effect layers stack by track
order: the lower track runs first.

**Inputs / controls:** Drag handle per row; context menu Move up, Move down,
Duplicate, Delete.

**Behavior / output:** The clip stack runs at L6 in list order (first row first).
For the whole frame, FX-001 fixes the order: color before clip effects, then
mask and corner coverage, then transform/animation, element styling, opacity
and blend; effect layers run in TL's track order.

**Edge cases:**
- Reordering a disabled effect → allowed; order is kept when it is re-enabled.
- Order-sensitive pairs (e.g., blur then edge-detect vs. the reverse) → no
  automatic reordering. FreeCut never reorders silently.
- Duplicate → inserted directly below the original, with a new id and the same
  seed.

**Undo:** `MoveClipEffect { clip, from, to }`; `DuplicateClipEffect` ↔ remove.

**Model notes:** The order is the `Vec` order in `Clip.effects`. The pure op is
tested for reversibility.

### FX-014 Effect parameter keyframes — v1

**What the user sees:** A keyframe diamond beside each animatable effect control
(numbers, colors, points, and the Mix of effect layers). Keyframes appear on the
clip or effect item in the timeline and use the same editor as transform
keyframes.

**Inputs / controls:** As defined by TR's keyframe system (add/remove keyframe
at the playhead, interpolation, easing).

**Behavior / output:** Parameter values are interpolated before upload as
uniforms, at the time TR-027 anchors keyframes to: the clip's source time for
clip effects on media clips (so pre-roll inside a transition evaluates
naturally), and item-local time for effect layers and other items without a
time map. Enum and toggle parameters are not animatable in v1 (TR-027).

**Edge cases:**
- Trims, splits and speed changes → TR-027 applies: keyframes are
  source-anchored, a split keeps all keys on both halves and inserts no
  boundary key, and trims and speed changes move keys with the footage.
- A keyframe outside the item's current range → kept and used for
  interpolation, but not shown.
- A keyframed control is reset → asks whether to clear its keyframes (default:
  yes).

**Undo:** TR's keyframe operations, targeted at `(item, effect_id, param_key)`.

**Model notes:** `ParamSet` values may be `Static(v)` or `Animated(KeyframeTrack<v>)`,
reusing the keyframe types from TR's model in `freecut-timeline`.

### FX-015 Built-in video effect library — v1

**What the user sees:** An Effects tab with categories of animated tiles. All
entries are FreeCut-made procedural shaders: no downloads, no image assets
beyond our own CC0 grayscale maps. The tab has two families, each with its own
sub-tabs and its own favorites group (FX-016): **Whole frame** (this feature)
and **People** (effects that follow a person or face, FX-020/FX-021). The
reference app lists hundreds of downloadable presets; FreeCut instead ships
about 60 parameterised looks whose controls cover many preset variations, and
more can be installed from local packs (FX-025).

**Inputs / controls:** Initial set (about 60 entries; described by look):
- **Basic:** gaussian blur; directional blur; zoom/radial blur; sharpen;
  vignette; film grain; noise; mosaic/pixelate; letterbox bars.
- **Lens and light:** glow/bloom (Direction: inner, outer or both); soft
  dreamy focus; chromatic fringe;
  procedural light-leak sweep; streak flares; bokeh-dot overlay.
- **Retro:** analog-tape look (scanlines, tracking wobble, color bleed); old-film
  look (procedural dust, scratches, flicker, warm tint); old-TV look (curved
  screen, scanlines, RGB mask); camcorder overlay (recording dot and timestamp
  drawn with our bundled OFL font).
- **Glitch:** RGB split; block displacement; line jitter; pixel sorting; digital
  noise bursts; strobe.
- **Distortion and mirror:** wave; ripple; swirl; bulge/fisheye; pinch;
  kaleidoscope; mirror (horizontal, vertical, quad); split-screen tiles (2, 3, 4,
  6, 9 copies).
- **3D-style:** perspective tilt and swing; mirrored tunnel (an endless
  corridor of reflected copies with a camera move through it); rotating
  panel; photo parallax for stills (FX-029).
- **Collage:** a grid of copies with gaps and slight seeded rotations; a
  photo-strip of offset copies; scattered snapshot frames.
- **Motion:** camera shake; beat-style zoom pulse; wobble; slow push-in; rotation
  sway; flash-freeze.
- **Atmosphere and particles:** rain; snow; sparkles; floating dust; confetti
  (all procedural, seeded).
- **Stylize:** edge detect; posterize; halftone dots; ink-outline comic look;
  painterly smoothing (Kuwahara-style filter); cartoon look (edge-preserving
  smoothing plus outlines); comic-panel look (halftone, speed lines and a
  panel border); false-color heat look; invert; duotone.
- **Openings and endings:** fade from/to black or white; shutter/curtain open
  and close; iris open and close; focus-pull blur in and out.
- **Temporal:** echo/trails; afterimage (frame history, FX-010 rules).

**Behavior / output:** Each entry has a manifest (id, name, category, tags,
parameters with ranges and defaults, whether it needs frame history, whether
clip-only, which media it accepts — any, video only or stills only — an
optional exclusive group (FX-010), and whether it needs an analysis job) and
one or more WGSL passes. Multi-pass effects (e.g., bloom) declare their
intermediate textures.

**Edge cases:**
- An effect that fails to compile on the user's GPU → hidden from the library;
  existing uses are bypassed with a warning badge.
- Effects at extreme settings (blur radius 100 % on 8K) → the render crate
  downsamples internally (e.g., a mip-chain blur) to bound cost, while keeping
  the look resolution-independent.
- Openings/endings whose look depends on position (e.g., fade-in) → their time
  is relative to the effect's own start, so they work as layers or on clips.

**Undo:** n/a (catalog).

**Model notes:** `EffectRegistry` in `freecut-render`. Shaders are validated with
naga in CI. Every effect gets a golden-image test at 32×32 for a fixed seed and
time.

Acceptance tests for every bundled effect (they mirror the quality checks the
reference app's UI applies to its own effect content):
- Same look on 1:1, 16:9 and 9:16 canvases: golden images at all three show no
  breakage and no change of character.
- Any length works: an effect has no fixed base length, periodic motion loops
  without a seam, and no part of the effect drops out partway along a long
  clip or layer (rendered at 1 s, 60 s and 10 min of effect-local time).
- Every slider drives the property its label names, in the expected
  direction, and is stable at its minimum and maximum (no NaN, no black or
  blank frames).
- The effect never changes, trims or hides the clip it sits on unless that is
  its stated purpose (e.g., an opening fade): clip ranges are untouched by
  construction, and a test checks that the layer's alpha coverage is unchanged
  at default settings for every effect not tagged as a reveal or fade.
- Smooth on retimed clips: effect time is clip-local timeline time (FX-010),
  so a 2× or 0.5× clip shows the same effect motion per second.
- No jump at the start or end of the clip beyond what the effect intends.
- The library tile loop and the timeline render match, because both use the
  same pipeline (FX-016).

### FX-016 Library browsing: search, favorites, preview — v1

**What the user sees:** The Transitions, Effects and Animations libraries (and,
later, the person- and face-tracking effects) use the shared library panel
defined in `workspace-and-shortcuts.md` (UI): category tabs, a search field,
Favorites and Recently used sections, and animated tiles. This feature defines
only what is FX-specific: what a tile preview shows and how it is applied.
Hovering (or clicking)
a tile previews it on the selected clip in the viewer without changing the
project. Only drop, "+", or picking it in the inspector applies it.

**Inputs / controls:** Search over display names and tags (with localized tags).
Each library tab has its own search field (UI-006), and so does each preset
grid in the inspector's Motion lens (FX-018). A star toggle on tiles.
Right-click → "Add to favorites" / "Apply" ("Apply" follows the "+" rules of
FX-010/FX-011).

**Behavior / output:** Preview renders through the normal pipeline with a
temporary, non-undoable overlay of the candidate item, removed when the pointer
leaves. Everything works offline, and favorites need no account: they are
per-user, local (UI-006). In the Effects tab, Favorites are shown in two
groups, People and Whole frame (FX-015).

**Edge cases:**
- No clip selected → preview uses a built-in CC0 sample clip in the tile only.
- Preview while playing → playback continues with the candidate applied.
- Favorites referring to a removed item → silently dropped.
- Search with no results → UI-006's empty state with *Clear search*, plus a
  "Try these" row: the user's recent items from this tab, or the tab's Basic
  category when there are none (FreeCut default; the reference app suggests
  popular items, which needs usage data FreeCut does not collect).
- End of a result list → a small "No more items" line, so a short list is
  not mistaken for one that is still loading.
- Effects that need an analysis (FX-020, FX-021, FX-028, FX-029 with automatic
  depth) cannot preview on the selected clip before its analysis exists →
  hover plays the tile loop only, with the note "Preview after analysis".

**Undo:** n/a (view state). Favorites and recents are saved in the user's app
config, not in the project.

**Model notes:** UI only (`freecut-app`, on top of UI's panel framework). The
candidate preview is a render-time overlay passed to `freecut-render`, never a
timeline edit.

### FX-017 Copy and paste effects, animations and transitions — v1

**What the user sees:** Effects and animations appear as two groups, "Effects"
and "Animations", in the *Paste attributes…* checklist (`Mod+Alt+V`). TL-034
(`timeline-editing.md`) owns that command, its dialog and the attribute
clipboard; UI owns the key binding. The Effects group has one extra option:
Replace (default, TL-034's normal per-group behaviour) or Append. Separately, a
copied transition can be pasted onto selected cuts or onto the cuts between
selected clips.

**Inputs / controls:** The two group checkboxes (on by default; FreeCut
default), the Effects option, and TL-034's keyframe timing option (rescale or
absolute offsets). For transitions: Copy on a selected transition, Paste on
selected cuts or clips.

**Behavior / output:**
- **Effects** copies `EffectInstance`s with new ids and the same seeds.
  Replace swaps the target's whole stack; Append adds the copies to the end of
  it. Keyframes are placed using TL-034's timing option, measured in
  clip-local timeline time and converted to the target's source time through
  its time map, as TR-034 does for its own groups. The motion therefore looks
  the same on the timeline.
- **Animations** copies `ClipAnimations` (all three slots). The target's slot
  policy (FX-019) is then applied. When the target uses the exclusive policy
  and the copied set has both a Continuous and an Entrance/Exit (copied from a
  text clip), the Continuous slot is dropped and reported (FreeCut default).
- **Transitions** are pasted with the duration clamped per cut, following
  FX-002's eligibility rules, in-point shift and ripple.

**Edge cases:**
- Pasting onto text or stickers → the Effects group is skipped (clip effects
  are video and image only in v1) and listed among TL-034's skipped groups.
  Animations apply, because text and element clips use the same slots
  (FX-018). Presets the target kind does not offer (e.g., a per-glyph text
  preset onto a video clip) are skipped and listed.
- Animation durations longer than the target clip → clamped at render (FX-018
  rule).
- Effects with a region (FX-027) → the region is copied with its keyframes. A
  tracking binding stays only when the target is the clip whose footage was
  tracked; otherwise it is dropped, the region keeps its shape at the
  reference frame, and TL-034's summary lists it.
- Pasting onto a locked track → skipped, and reported.

**Undo:** One compound step per paste. The attribute paste is part of TL-034's
single `SetClipProps` batch. The transition paste is one
`ApplyTransitionToCuts`-style compound op as in FX-006, including the in-point
shifts and ripples of FX-002.

**Model notes:** FX registers two property bags, `effects` (`Clip.effects`) and
`animations` (`Clip.animations`), through TL-034's `PropBag::category()`
registry in `freecut-timeline`. There is no separate attributes struct. A
copied transition is a serde snapshot held by `freecut-app` (in-process;
optionally on the system clipboard as FreeCut JSON).

### FX-018 Entrance and exit animations — v1

**What the user sees:** The inspector's **Motion** lens (UI-008; called the Animation section in earlier drafts) for a selected
video, image, text or element (sticker, shape, emoji) clip, with three
sub-tabs: Entrance, Exit and Continuous. Each shows a search field (FX-016), a
grid of presets starting with "None", and a duration slider. The clip in the
timeline shows a shaded ramp over the animated part at its start or end. Text
clips (and caption tracks and segments) label the sub-tabs In, Out and Loop
and offer their own presets (TX-022, TX-028; catalogue in TX-056). Element clips use the presets below (OV-018).

**Inputs / controls:**
- **Entrance presets** (generic, FreeCut-made): fade in; grow from small; shrink
  from large; slide in; rise with a small bounce; drop in; 2D spin in; 3D flip
  in; swing in; blur in; wipe reveal; shake in; pop with overshoot; stretch in;
  white flash reveal (a brief bright flash as the clip appears); squash-and-stretch pop
  (a cartoon-style bounce).
- **Exit presets:** the mirror of each entrance preset.
- **Direction:** directional presets (slide, wipe, swing, 3D flip) are one tile
  each with a Direction choice: from the left, right, top or bottom (default
  per preset, e.g., slide from the left). FreeCut default; one tile per
  direction is the alternative. Stored in `AnimationSpec.params`.
- **Duration:** default 0.5 s (FreeCut default; public tutorials use roughly
  0.3–1.5 s); minimum 0.1 s (at least 1 frame); maximum = the clip's visible
  duration (FX-002) minus the other animation's duration. Values snap to
  frames. Text and element clips never have transitions, so for them the
  visible duration is simply the clip duration.
- **Easing:** each preset has a built-in curve (ease-out for entrances, ease-in
  for exits; FreeCut defaults). Optional override: any easing preset from
  TR-031 except Hold.
- **Intensity** (0–100, default 50) and, for slides, **Travel** (From edge or
  Short): FX-030, which also defines each preset's motion and default
  duration.

**Behavior / output:** The animation produces per-frame offsets: translation (in
canvas-normalized units), scale factor, 2D rotation, 3D rotation with
perspective, opacity factor, blur amount and a wipe reveal. They are composed
with the keyframed transform at L8/L9 (FX-001). The entrance runs over
`[visible_start, visible_start + in_duration)` and the exit over
`[visible_end − out_duration, visible_end)`, in timeline time, so changing clip
speed does not change animation length. Works on any track, including the main
track, where a sliding clip reveals the background or lower tracks. Entrance
and Exit can be used together. Whether Continuous can be combined with them
depends on the clip kind (slot policy, FX-019).

**Edge cases:**
- Clip too short: an Entrance or Exit grid is disabled when the room left
  (visible duration minus the other slot's duration) is below the 0.1 s
  minimum, with the hint "Not enough room on this clip for the animation". A
  clip shorter than
  2 frames renders no animation even if one is stored (the same rule as
  TX-022).
- An animation preset this build does not know (a file from a newer FreeCut,
  or a pack that was removed, FX-025) → kept in the file, rendered as None, and
  shown as "Unavailable animation" with *Replace…* and *Remove*. It plays again
  once the preset is available.
- The clip is trimmed or sped up so that `in + out > visible duration` → effective
  durations are scaled down proportionally at render time. Stored values are
  kept, so lengthening the clip again restores them, and trimming never needs to
  rewrite animation data.
- Split → the entrance stays on the left piece and the exit on the right piece.
  The left piece gets no exit and the right piece no entrance.
- Clip that also has a transition at the same end → both apply. B's entrance
  is measured from its visible start, which is the start of the incoming
  transition region (FX-002), so the entrance plays during the transition over
  the same footage it was set on. A's exit ends at its visible end (the cut,
  with the default placement), so it also plays during the transition. The
  inspector shows a hint "Plays during a transition".
- Reverse playback of the clip (TR reverse) → animations are unaffected; they
  are in timeline time.
- Clip already keyframed (position, scale, opacity) → combined, never replaced.
- Image (still) clips → identical behavior.
- Locked track → no edits.
- Choosing an Entrance or Exit while a Continuous animation is set → handled by
  the slot policy (FX-019): on video, image and element clips the Continuous
  animation is cleared, with the change shown in the inspector; on text clips
  nothing is cleared.

**Undo:** `SetClipAnimation { clip, slot: Entrance|Exit|Continuous, old: Option<AnimationSpec>, new }`
(changes to the other slots made by the slot policy are recorded in the same
step). Duration slider drags are coalesced into one step.

**Model notes:** `Clip.animations: ClipAnimations { entrance: Option<AnimationSpec>, exit: Option<AnimationSpec>, continuous: Option<ContinuousSpec> }`,
with `AnimationSpec { preset: AnimationPresetId, duration: RationalTime, easing: Option<Easing>, params: ParamSet }`.
`Easing` is TR's easing type (TR-031). The evaluation function
`animation_offsets(clip, t) -> AnimOffsets` takes rational `t` and the clip's
visible range, and is pure in `freecut-timeline` (no GPU), so it is
unit-testable. TX (TX-022) and OV (OV-018) reuse the slot model, the slot
policy and this evaluator. Text adds per-glyph evaluation in
`freecut-render`, because that needs the layout (TX-022).

### FX-019 Continuous animations — v1

**What the user sees:** The Continuous sub-tab offers presets that run for the
whole clip. On text clips it is labeled Loop and runs between In and Out
(TX-022).

**Inputs / controls:**
- **Presets** (generic), in three kinds:
  - *Loops:* pendulum sway; spin loop; bounce loop; heartbeat pulse; jitter
    shake; float (bob up and down); rotate-and-zoom cycle.
  - *Drifts* (one-shot, spanning the clip): slow zoom-in drift; slow zoom-out
    drift; pan drift (Direction: left, right, up, down).
  - *Combined in-and-out* (one-shot, spanning the clip): one choice that plays
    an entrance at the start and the matching exit at the end, with the
    motion between them continuous, e.g., zoom through, spin in and out,
    slide through from one side to the other, pop in then fade out. Not
    offered on text clips, whose In and Out slots already coexist with Loop
    (TX-022).
- **Cycle length** for loops: 0.1–10 s (default per preset, e.g. 1 s), shown
  with a speed readout in cycles per second. The reference app exposes the
  same thing as a speed control (shorter cycle = faster).
- **Repeat** for loops: *Fill clip* (default) or a count of 1–50 cycles; after
  the last cycle the clip rests in the preset's rest pose (zero offset).
- **In/out portion** for combined presets: 10–50 % of the visible duration
  for each end (default 30 %; FreeCut default).
- **Strength:** 0–100 (default 50; FreeCut addition). Shown as **Intensity**;
  it is FX-030's control, and FX-030 defines what each preset does with it.

**Behavior / output:** Offsets are evaluated from clip-local time (over the
visible range, FX-002) plus a stored phase offset, and composed at L8/L9 like
FX-018. Drifts interpolate from the clip's visible start to its visible end.
Combined presets map the first and last *portion* of the visible range to
their entrance and exit curves and the middle to their through-motion; on a
video, image or element clip they occupy the Continuous slot, so the
exclusive policy below applies to them like to any Continuous preset.

**Slot policy (per clip kind):** FX owns the rule for combining the three
slots. It applies to every clip kind that has animation slots:
- **Video, image and element clips — exclusive.** Choosing a Continuous preset
  clears Entrance and Exit; choosing an Entrance or Exit clears Continuous
  (FreeCut default, for unambiguous composition).
- **Text clips, including caption tracks and segments — Loop between In and
  Out.** Loop coexists with In and Out. It is active only in `[in, d − out)` of
  clip-local time, where `d` is the clip duration and `in`/`out` are the
  effective In and Out durations after FX-018's proportional scaling. It is
  inactive while In or Out plays (TX-022). Choosing In or Out never clears
  Loop, and choosing Loop never clears In or Out.

**Edge cases:**
- Combination with Entrance/Exit → per the slot policy above.
- Text clip whose effective In + Out fill the whole clip → Loop's window is
  empty and it has no effect; the stored Loop is kept, so lengthening the clip
  brings it back.
- Split → both pieces keep the animation. The right piece's phase offset is set
  to the split position, so looping motion is seamless across the split.
  Drift presets on each piece restart over the piece's own length; a hint
  offers "keep continuous drift", which converts them to keyframes.
- Very short clips (< one cycle) → a partial cycle.
- A repeat count longer than the clip → plays the cycles that fit; the count
  is kept. After a split the count is measured from the original start
  through the phase offset, so the right piece plays only the remaining
  cycles.
- Combined presets on a split → each piece restarts over its own length, like
  drifts, with the same "keep continuous drift" hint.
- Speed changes → timeline-time based, so cycle length is unchanged.

**Undo:** As FX-018 (`slot: Continuous`).

**Model notes:** `ContinuousSpec { preset, cycle: Option<RationalTime>, repeats: Option<u16>, portion: Option<f64>, strength: f64, phase_offset: RationalTime, params: ParamSet }`
(`repeats: None` = fill clip; `portion` only for combined presets; `params`
holds choices such as Direction). The preset manifest says which kind (loop,
drift, combined) a preset is.
`SlotPolicy::{Exclusive, LoopBetween}` and a pure
`slot_policy(kind) -> SlotPolicy` live in `freecut-timeline`. `SetClipAnimation`
consults it to decide which other slots to clear, and `animation_offsets` uses
it to compute the Continuous window. Both policies are unit-tested, including
the effective-duration scaling on short text clips.

*FreeCut decision (S2 model, 2026-10-13):* the three slots share one type,
`ClipAnimations { entrance, exit, continuous: Option<Arc<AnimationSpec>> }` on every clip
(empty on audio clips), with `AnimationSpec { preset, duration, easing, direction, params:
AnimParams { unit, overlap, order, seed, pace, intensity, travel }, phase }`. For Continuous,
`duration` is the cycle and `phase` is the phase offset; `repeats` and `portion` arrive with the
presets that need them. Loops are evaluated at clip-local time plus `phase` (not from the start
of the text window), so the split rule's `phase += left length` continues every loop exactly.
Proportional scaling rounds In down and gives Out the rest, so the two never overlap. Clips
shorter than two frames play nothing. `SetClipAnimation` snaps In and Out to frames, at least
0.1 s rounded up to a frame and at most what the other slot leaves of the clip (a clip without
room stores the minimum), keeps Loop cycles within 0.1–10 s, keeps the phase a Loop already
has, and clamps intensity and overlap to 1000.

*FreeCut decision (S2 app, 2026-10-13):* video, image and element clips get the Motion lens
(Picture, Motion and, for video, Sound), with the same timing strip, slot picker and grid as
text, offering whole-layer clip presets (`fc.clip.…`) only. The timeline draws each clip's
effective In and Out as a darker region with a diagonal ramp line (rising for In, falling for
Out), so the shape tells them apart, not colour alone. An animation whose preset this build
lacks shows "Unavailable animation" with **Remove**; *Replace…* is the grid itself.

*FreeCut decision (S2 app follow-up, 2026-10-13):* the Continuous (Loop) grid is grouped by
kind in FX-019's order: **Loops**, **Drifts**, then **In and out** (combined presets, offered on
video, image and element clips only), each group sorted by name, with "None" in a row of its own
above the groups. Headings show only when the grid has more than one group, so a text clip's Loop
grid (loops only) stays one plain grid; In and Out grids are one group. A preset the loader
couldn't play shows at the end of its group as a disabled **Unavailable animation** tile with the
reason in its tooltip (TX-022). Presets whose motion depends on the seed (a random order,
Jitter shake's seeded channels) show the Seed row with its dice.

### FX-020 Person-tracking effects — later

**What the user sees:** A People tab in the Effects library with person-aware
effects that follow the people in a clip: a glowing outline around a person;
background-only treatments (blur, desaturate, darken or replace-color the
background while the person stays untouched); person color-pop (person in color,
the rest in grayscale); afterimage trails that follow the person; particles
emitted from the silhouette; offset shadow/clone copies of the person. The tab
groups them into three sub-tabs (FreeCut's own names): **Portrait**
(background treatments, color pop), **Playful** (clones, trails, particles)
and **Stylized** (glowing outline, solid silhouette fill, comic outline).

**Inputs / controls** (FreeCut defaults; each effect shows only the controls it
uses): intensity 0–100 (default 70); color (default white for outlines and
glows); outline thickness 0–100 (default 20); glow radius 0–100 (default 30);
trail length 1–30 frames (default 8); temporal smoothing 0–100 (default 50);
target: all people (default). "Largest person only" comes later.

**Behavior / output:** Clip effects only (they need one specific source). When
applied, FreeCut queues a local, background **analysis job** for the clip's
used source range that produces a per-frame person mask (segmentation) and
optionally body keypoints. Results are cached on disk, keyed by media hash and
model version. Until the analysis is ready, the preview shows the clip without
the effect and an "Analyzing… n %" badge. Export waits for (or starts) the
analysis. Masks are sampled at L6 time via source time, so trims and speed
changes reuse the cache.

**Edge cases:**
- No person detected in a frame → person-specific parts render nothing;
  background treatments then apply to the whole frame (consistent with an empty
  mask).
- Several people → union mask (a per-person choice is a later refinement).
- Fast motion or motion blur → mask edges lag; smoothing parameter and feather.
- Trim beyond the analyzed range → incremental analysis of the new range only.
- Media relinked to a different file → cache key changes, so it is re-analyzed.
- Analysis fails or the model is not installed → effect shows an error badge,
  renders as bypass, and offers "Retry" / "Install model".
- Several People effects applied while an analysis is running → they share
  the one job for that media and range (AI-001's result key), so a second
  request never starts a second analysis and is never refused; "Retry" is
  disabled while the job runs (FreeCut default; the reference app refuses a
  second effect while one is still processing).
- Compound clip (TL-040) → refused with the hint "Apply it to the clip inside
  the compound clip": the analysis is keyed to one media source (FreeCut
  default; analysing the compound's rendered output is the alternative).
- Two background treatments on one clip (e.g., blur and replace-color) →
  they share an exclusive group, so the second asks to replace the first
  (FX-010).
- Proxies (PM/EX) → analysis may run on the proxy; the mask is upscaled with
  edge-aware filtering.

**Undo:** Adding, removing and parameter changes as FX-010/FX-012. Analysis
results are cache, not project state, so they are not undoable.

**Model notes:** Inference runs outside the UI thread through the AI area's local
model runner (see `ai-tools.md`). This spec only consumes `PersonMaskTrack` /
`PoseTrack` lookups. Candidate local models: MediaPipe selfie segmentation and
pose landmarker (Apache-2.0; model-card terms to verify), Robust Video Matting
(GPL-3.0, compatible with FreeCut's GPL-3.0-or-later), MoveNet (Apache-2.0; to
verify).

### FX-021 Face-tracking effects — later

**What the user sees:** Face-aware effects that follow detected faces: face-region
blur or pixelation (privacy); glow or sparkle around the eyes; face-region color
tint; cartoon outline on the face only.

**Inputs / controls** (FreeCut defaults): intensity 0–100 (default 70; for
privacy blur/pixelation, 100); padding around the face 0–100 % of the face box
(default 20 %); feather 0–100 (default 30); which faces: all (default).

**Behavior / output:** As FX-020, but with a face-detection and landmark analysis
track. Effects apply in layer space at L6 using the tracked face boxes and
landmarks.

**Edge cases:**
- Faces entering or leaving the frame → effect fades over 3 frames (FreeCut
  default) to avoid popping.
- Profile or occluded faces → detection gaps are filled by interpolation for up
  to 10 frames.
- Privacy use → a missed detection in a frame is a real risk; the UI says
  detection is automatic and can miss faces, and offers a manual mask (TR) as a
  fallback.

**Undo:** As FX-010/FX-012.

**Model notes:** Same analysis-cache mechanism as FX-020. Candidate models:
MediaPipe face detector/landmarker (Apache-2.0; to verify), OpenCV Zoo YuNet
(MIT, per the model directory's README). No beauty retouching or face reshaping
here (see Out of scope).

### FX-022 Motion blur for animated movement — later

**What the user sees:** A per-clip "Motion blur" toggle that blurs fast entrance,
exit, continuous and keyframed movement the way a camera shutter would. It
blurs only movement that FreeCut itself creates; blur for motion recorded in
the footage is FX-028, and the two can be combined.

**Inputs / controls:** Off by default; shutter angle 0–360° (default 180°);
quality: 4/8/16 sub-frame samples (default 8).

**Behavior / output:** L8 renders the layer at several sub-frame times across
the shutter interval (rational sub-frame times) and averages them. This is
applied only when the transform changes within the interval.

**Edge cases:**
- Static clips → no cost (skipped).
- Combined with transition shaders that have their own motion blur → independent.
- Export at a different frame rate → the shutter is based on the output frame
  duration.

**Undo:** `SetClipMotionBlur { old, new }`.

**Model notes:** `Clip.motion_blur: Option<MotionBlur>`; the render crate owns
the accumulation.

### FX-023 Custom wipe transitions from user images — later

**What the user sees:** "Import wipe pattern…" in the Transitions library accepts
a grayscale image. It becomes a wipe in which darker areas switch first, with
softness and invert controls.

**Inputs / controls:** Image file (PNG/JPEG/WebP); softness 0–100 % (default
10 %); invert toggle.

**Behavior / output:** The image is decoded off the UI thread, converted to an
8-bit luminance map, and stored as a project media reference (PM), so the
project stays portable when collected.

**Edge cases:**
- Color image → converted to luminance.
- Non-canvas aspect ratio → stretched to fill (FreeCut default), with a "fit"
  option.
- Missing file on load → transition renders as a dissolve, with a relink
  prompt (PM).

**Undo:** As FX-002/FX-005. Importing the pattern itself is a media-library
operation (PM).

**Model notes:** `TransitionKindId` `"freecut.luma"` with a `pattern: MediaRef`
parameter (the timeline stores only the reference). Decoding the image is done
by `freecut-media` in a background job; `freecut-render` uploads the luminance
map as a texture.

### FX-024 Saved effect presets — later

**What the user sees:** "Save as preset…" on a clip's effect stack (optionally
with its animations). Saved presets appear under "Saved presets" in the Effects
library and apply in one click.

**Inputs / controls:** Preset name; include animations (checkbox).

**Behavior / output:** Stored as a versioned JSON file in the user's FreeCut
config directory, never in the cloud. Export and import of preset files are
supported.

**Edge cases:**
- A preset that references an effect kind unknown to this build → it imports,
  and the unknown entries are marked unavailable.
- Name collisions → "(2)" suffix.

**Undo:** Applying a preset is one compound step. Saving or deleting presets is
config (not undoable; confirm on delete).

**Model notes:** Reuses the serde types of `EffectInstance` / `ClipAnimations`
from `freecut-project`. Reading and writing preset files is done by
`freecut-app` off the UI thread; applying a preset is a pure
`freecut-timeline` compound op.

### FX-025 Custom shader effects (advanced) — later

**What the user sees:** Advanced users can load a WGSL file plus a small manifest
as a custom effect or transition. *Install pack…* adds a whole community pack
of effects, transitions and animation presets from a local file, which then
shows up as its own category (named after the pack) in each library. This is
FreeCut's local, account-free replacement for downloadable preset libraries.

**Inputs / controls:** File picker; the parameters are declared by the manifest.
A pack is a folder or `.zip` with one manifest per item (effects and
transitions with their WGSL; animation presets as data-only curves), a pack
manifest (name, version, author, license) and an optional license file.
Packs are uninstalled from their category's context menu.

**Behavior / output:** The shader is validated (naga) and run with fixed bindings
(input textures, time, progress, parameters). It is stored in the project as an
embedded text asset, so the project stays self-contained. Installed packs live
in the user's FreeCut config directory; a project that uses a pack item embeds
that item's definition (shader text or animation curves) in the same way, so
it renders on machines without the pack. FreeCut never downloads packs
itself.

**Edge cases:**
- Compile error → shown with a line number; effect bypassed.
- A shader that takes too long (GPU timeout risk) → a per-pass watchdog; the
  effect is disabled after a device-lost recovery.
- Opening a project with custom shaders → the user is asked once before they
  run (they are code). Installing a pack that contains shaders asks the same
  question once; a pack of animation curves only does not.
- Pack item ids that use FreeCut's reserved `freecut.` namespace, or collide
  with an installed item → the pack is refused with a list of the clashing ids.
- The same pack installed again → a newer version replaces the old one after
  confirmation; an older or equal version asks before downgrading.
- Uninstalling a pack whose items are used in an open project → those uses
  keep working from the project's embedded copies.
- A pack item that fails validation → that item is skipped and listed; the
  rest of the pack installs.

**Undo:** As FX-010/FX-011. Installing and uninstalling packs is user
configuration, not a project edit, so it is not undoable (uninstall asks for
confirmation).

**Model notes:** Validation and execution live in `freecut-render`; the shader
text and manifest are stored by `freecut-project`; the timeline only references
the custom kind id. Pack install, unzip and validation run in `freecut-app`
off the UI thread; packs register with the same `EffectRegistry` and
`TransitionRegistry` as bundled items, under the pack's namespace. Deferred
past v1 because of GPU robustness and the stability of the shader API.

### FX-026 Apply an effect to all clips — v1

**What the user sees:** *Apply to all clips…* in a clip effect's row menu in
the inspector (and in an Effects-library tile's context menu) copies that
effect onto many clips in one step. A small dialog sets the scope and shows
how many clips will get the effect and how many are skipped, and a toast
reports the result, e.g. "Added to 24 clips, skipped 3 (locked track)".

**Inputs / controls:**
- Scope: All video and image clips (default) / Clips on this track /
  Selected clips (the same scopes as CL-025's Apply to all for color).
- "Skip clips that already have this effect" (default on).
- Source: from a clip's effect row, the copy carries that effect's current
  parameters, keyframes and region; from a library tile, the effect's
  defaults.

**Behavior / output:**
- Each target gets an independent copy (new id, same seed, so seeded patterns
  match) appended to the end of its stack, exactly as FX-010's add. There is
  no live link between the copies.
- Keyframes are placed with TL-034's default keyframe timing, measured in
  clip-local timeline time and converted through each target's time map, as
  FX-017's Effects group does.
- A region (FX-027) is copied without its tracking binding.
- The dialog footer points to the alternative: "To control many clips with
  one set of settings, use an effect layer (FX-011)."
- Effects that need an analysis (FX-020, FX-021, FX-028, FX-029) queue one
  analysis per target source; the dialog says how many will start.

**Edge cases:**
- The source clip is never a target.
- Clips on locked tracks, text, stickers, captions and effect or adjustment
  items → skipped and counted in the dialog and the toast.
- Media-type mismatch (FX-010, e.g., a stills-only effect on video clips) →
  skipped and counted.
- A target that already has a member of the effect's exclusive group
  (FX-010) → skipped and counted; Apply to all never replaces silently.
- Compound clips count as video clips (FX-010); effects that need per-source
  analysis skip them.
- No eligible target → nothing changes, no undo entry, and the toast says why.
- Thousands of clips → one model-only op; render stacks are rebuilt lazily,
  so the UI stays responsive.

**Undo:** One compound `ApplyEffectToClips { added: Vec<(ClipId, index, EffectInstance)> }`.
Undo removes exactly the added instances and nothing else.

**Model notes:** A pure batch op in `freecut-timeline` built from FX-010's
`AddClipEffect` and the same `EffectTraits` checks; scope resolution is a pure
function over the sequence and the selection. Unit-tested with mixed eligible
and ineligible targets, and for exact reversibility.

### FX-027 Effect regions (masks on effects) — v1

**What the user sees:** Each clip effect row and each effect layer has a
**Region** control: *Whole frame* (default) or a shape. With a shape, the
effect applies only inside it, for example a blur or pixelation over a licence
plate, or a glow only on the sky; *Invert* applies it everywhere except
there. While the Region control is focused, the viewer shows the region's
outline with handles. *Track…* makes the region follow a moving object. All
of this is free; nothing about regions is gated.

**Inputs / controls:**
- Shape: Whole frame, or TR-017's shapes (Linear, Band, Ellipse, Rectangle,
  Heart, Star) with their size, rotation and roundness controls.
- Feather and Invert as TR-018 (same ranges and defaults).
- On-viewer handles as TR-019.
- Center, size, rotation and feather can be keyframed (TR-027).
- *Track…* opens AI-008's tracking flow (direction, follow scale and
  rotation, quality) with this region as the thing that follows; *Remove
  tracking* and *Bake to keyframes* work as in AI-008.
- Space: a clip effect's region lives in clip space and moves with the clip,
  like a clip mask (TR-017); an effect layer's region lives in canvas space.

**Behavior / output:**
- The region gives a coverage `c` (0–1) per pixel from the same
  signed-distance evaluator as TR-017, with feather and invert applied as in
  TR-018.
- Clip effect at FX-001's L6: `out = in + c × (effect(in) − in)` on
  premultiplied RGBA.
- Effect layer at FX-001 step 6: `acc = acc + c × mix × (effect(acc) − acc)`.
- Effects that sample neighbours (blur, glow) read the whole input, so the
  region's edge shows a true blur of the surrounding pixels rather than a
  seam; feather softens the boundary.
- Tracked region: its center (and, if chosen, scale and rotation) follows
  AI-008's track samples plus the stored offset, with region keyframes added
  on top. For a clip effect the tracked clip is the clip itself; for an
  effect layer it is the clip chosen in AI-008 (default: the topmost video
  clip under the layer at the playhead). AI-008 already lists blur or mosaic
  effect regions among the things it can track; this feature defines them.

**Edge cases:**
- One region per effect in v1; several regions with combine modes follow
  TR-020 later.
- Region entirely outside the frame → no visible result; the effect is kept.
- Invert is not offered with *Whole frame*.
- Disabled effect → its region is ignored.
- Region on an effect of a still image clip → *Track…* is disabled with the
  hint "Nothing moves in a still image". Animated images (GIF, WebP) can be
  tracked like video (FreeCut default; the reference app does not offer
  tracking on GIF clips).
- The tracker loses the target → AI-008's lost and continue flow.
- Tracked clip deleted → AI-008's rule: the region keeps its last position, a
  warning badge appears, and undo restores the link.
- Clips combined into a compound clip (TL-040) → regions and tracking on
  effects of the inner clips keep working, because the binding refers to the
  clip and its source time. FreeCut never strips tracking as a side effect
  (the reference app warns before such edits remove it). Break apart drops
  effects on the compound clip itself, with TL-040's warning.
- Split → both pieces keep the region; tracked samples are in source time, so
  both stay on target.
- Copy and paste → FX-017.
- Privacy use (hiding a face or a plate) → the Region panel notes that
  tracking can drift and suggests stepping through the result; automatic face
  blur is AI-021.

**Undo:** `SetEffectRegion { target, effect_id, old: Option<EffectRegion>, new }`;
drags are coalesced into one step per gesture. Region keyframes use TR's
keyframe ops targeted at `(item, effect_id, region parameter)`; tracking ops
are AI-008's.

**Model notes:** `EffectRegion { mask: Mask, tracking: Option<TrackBinding> }`,
where `Mask` is TR-017's type and the binding is AI-008's, stored in
`EffectInstance.region` (FX-010). Files without the field load as `None`
(whole frame). Coverage reuses TR-017's WGSL SDF code and CPU reference;
golden tests at 16×16 check that pixels with `c = 0` are bit-identical to the
input and pixels with `c = 1` equal the unmasked effect.

### FX-028 Motion blur from footage — later

**What the user sees:** In a video clip's inspector, a **Motion blur from
footage** section with an on/off switch. Turning it on starts a background
analysis ("Analyzing motion… n %" on the clip and a row in the task list, with
Cancel). When it finishes, moving subjects and camera pans in the footage get
a smooth directional blur, as if shot with a slower shutter. A toast reports
success, cancellation or failure (with Retry).

**Inputs / controls** (FreeCut defaults; the option lists match what the
reference app's UI offers):
- Amount 0–100 (default 50): blur length as a share of the motion between two
  source frames (100 = the whole frame-to-frame motion, like a 360° shutter).
- Blend 0–100 % (default 100 %): mix of the blurred and the original picture.
- Direction: Backward (trail behind the motion), Forward (ahead of it) or Both
  (centered; default).
- Passes: 1, 2, 4 or 6 (default 2). More passes follow curved motion more
  smoothly and cost more render time.

**Behavior / output:**
- The analysis estimates dense optical flow between neighbouring source
  frames for the clip's used source range plus one frame on each side, off
  the UI thread, with a non-ML estimator. Results go to the media cache
  (PM-074), keyed by the media fingerprint, the range and the estimator
  version; the job is a row in PM-073's task list.
- Rendering happens at FX-001's L2c: a shader samples along the flow vectors
  (taps spread over the chosen direction, repeated per pass). The blur length
  is Amount × |local speed| from the time map (TR-036), so sped-up footage
  blurs more and slow motion less.
- Only the flow is cached, so every control change previews at once when the
  analysis exists, multi-pass included (FreeCut default; the reference app
  cannot preview multi-pass results before applying them).
- Before the analysis is ready, preview shows the clip unblurred with the
  progress badge; export waits for (or starts) the analysis.
- Several selected clips → each gets its own setting and analysis in one step;
  nothing has to be combined first (FreeCut default; the reference app asks
  for the clips to be combined into one compound clip first).

**Edge cases:**
- Stills, freeze-frame stills (TR-044), text and element clips → not offered
  (no motion between frames).
- Compound clips → refused with the hint "Turn it on for the clips inside"
  (flow is per media source).
- Shot changes or heavy occlusion inside a clip → where forward and backward
  flow disagree beyond a threshold the blur is suppressed, so it never smears
  across a cut inside the footage.
- Reversed clips (TR-043) → Forward and Backward refer to playback direction
  (the flow is negated).
- Frame-blend sampling (TR-041) → each blended source frame uses its own flow.
- Trim beyond the analyzed range → only the new range is analyzed.
- Analysis cancelled → the switch stays on, nothing is blurred, and the
  section shows "Analysis cancelled — Retry".
- Analysis fails → error badge, renders unblurred, error toast with Retry
  (UI-040).
- Proxy editing on (PM-080, PM-081) → preview uses flow from the proxy,
  scaled; export uses full-resolution flow.
- Combined with FX-022 or a transition's own blur → independent; they stack.

**Undo:** `SetFootageMotionBlur { clip, old: Option<FootageMotionBlur>, new }`
(one compound step for several clips; slider drags coalesced). The analysis
is cache, not project state, and is not undoable.

**Model notes:** `Clip.footage_motion_blur: Option<FootageMotionBlur { amount: u8, blend: u8, direction: BlurDirection, passes: BlurPasses }>`
with `BlurDirection::{Backward, Forward, Both}` and
`BlurPasses::{One, Two, Four, Six}` in `freecut-timeline`. The flow estimator
(a pyramidal dense estimator in the DIS or Farnebäck family) is shared with
TR-042's optical-flow interpolation; which crate hosts it is decided together
with TR-042 (GPU compute in `freecut-render`, or `freecut-media`). The L2c
shader lives in `freecut-render`, with golden tests on synthetic moving
patterns.

### FX-029 Photo parallax — later

**What the user sees:** In Effects › 3D-style, **Photo parallax** turns a still
photo into a short camera move with depth: near things move more than the
background, so the photo seems to have layers. Applied to an image clip, it
plays across the clip.

**Inputs / controls** (FreeCut defaults):
- Move: Push in (default), Pull out, Pan (Direction: left, right, up, down),
  Orbit.
- Amount 0–100 (default 40).
- Focus depth 0–100 (default 50): the depth that stays still; nearer parts
  move one way and farther parts the other.
- Edge fill for gaps revealed behind near objects: Stretch (default), Mirror
  or Blur.
- Depth source: *Automatic* (local depth model) or *Gradient* (manual: angle
  0–359°, default 90° = bottom near, top far; start and end 0–100 %).
  Default: Automatic when the model is installed, otherwise Gradient.
- Easing: any TR-031 preset except Hold (default ease-in-out).

**Behavior / output:**
- A clip effect at L6, stills only, in an exclusive group (one per clip,
  FX-010).
- Automatic depth: an analysis job runs a local monocular depth model on the
  source image through the AI area's model runner. The depth map is cached in
  the media cache (PM-074), keyed by the image fingerprint and the model
  version. It is cropped with the image (L3), so depth and picture stay
  aligned.
- The shader displaces each pixel by its depth relative to the focus depth
  times the camera offset. The offset moves from the clip's visible start to
  its visible end in timeline time, like FX-019's drifts.
- Gradient depth needs no model and no analysis, so the effect always works.

**Edge cases:**
- Video or animated-image clip → refused with "Photo parallax works on still
  images" (FX-010 media rule).
- Model not installed, or analysis failed → falls back to Gradient with a
  notice and *Install model* (AI-002 handles the explicit-consent download).
- Transparent PNG → depth only where alpha > 0; transparent areas stay
  transparent.
- Very large images → depth is estimated on a copy whose long side is at most
  1,024 px and upsampled with edge-aware filtering.
- High Amount → Edge fill hides the gaps; even at 100 the camera offset is
  bounded to 10 % of the layer size (FreeCut default).
- Split → each piece restarts its move, with FX-019's "keep continuous" hint.
- Analysis still running → preview shows the gradient result with a progress
  badge, then switches.

**Undo:** As FX-010/FX-012. The depth map is cache and is not undoable.

**Model notes:** The effect stores only its parameters and its `DepthSource`;
the depth map is an analysis record. The AI area needs a "depth map" analysis
kind for this (not yet specified in `ai-tools.md`). Candidate local models:
MiDaS small (MIT) and Depth Anything V2 Small (Apache-2.0; its larger
variants carry a non-commercial license and are excluded). Both licenses,
model cards included, must be verified before bundling or offering a
download. The displacement shader lives in `freecut-render`, with golden
tests on a synthetic depth ramp.

### FX-030 Animation motion definitions and preset manifest — v1

**What the user sees:** No new panel. Every tile in the Entrance, Exit and
Continuous grids (FX-018, FX-019) behaves exactly as this catalogue defines,
and each preset that moves, scales, turns or blurs shows an **Intensity**
slider under its duration. TX-056 builds the text catalogue on the same
definitions.

**Inputs / controls:**
- **Intensity:** 0–100, default 50 (FreeCut default). Written `k` = Intensity
  / 50 below: 0 removes the movement, 50 is the default amplitude, 100
  doubles it. Fades and wipes have no Intensity. FX-019's Strength is this
  same control.
- **Travel** for slides: **From edge** (default) starts the layer just outside
  the canvas edge on the chosen side, measured from its bounds at rest;
  **Short** starts 0.15·k of the canvas short edge away. Ignored by presets
  that do not slide.
- Duration, easing override and Direction: as FX-018 and FX-019.
- Distances below are fractions of the canvas short edge (`se`); text uses
  em instead (TX-056).

*Entrance presets* (motion from the start of the slot to rest):

| Preset | Motion | Defaults |
|---|---|---|
| Fade in | Opacity 0 → 1. | 0.5 s, ease-out |
| Grow from small | Scale 1 − 0.4·k → 1; opacity reaches 1 at 40%. | 0.5 s, ease-out cubic |
| Shrink from large | Scale 1 + 0.4·k → 1; opacity reaches 1 at 40%. | 0.5 s, ease-out cubic |
| Slide in | From the chosen side, per Travel. | 0.5 s, ease-out cubic, from the left |
| Rise with a small bounce | Starts 0.12·k se below, passes rest by 8% of that distance and settles; opacity reaches 1 at 30%. | 0.6 s, back-out |
| Drop in | Falls from 0.2·k se above with two decaying bounces. | 0.7 s, bounce-out |
| 2D spin in | Rotation −180·k° → 0 with scale 0.5 → 1. | 0.6 s, ease-out cubic |
| 3D flip in | Turns 90° → 0 about the vertical axis (from the left or right) or the horizontal axis (from the top or bottom), with perspective at 1.5 × the layer width. | 0.5 s, ease-out |
| Swing in | Turns about the edge on the chosen side from −60·k° to rest, overshooting once. | 0.8 s, elastic (one swing) |
| Blur in | Blur 0.02·k se → 0; opacity reaches 1 at 50%. | 0.5 s, ease-out |
| Wipe reveal | A straight edge from the chosen side reveals the layer, with a soft edge of 8% of the layer size. | 0.5 s, ease-in-out sine, from the left |
| Shake in | Side-to-side shake of ±0.03·k se at 12 Hz that dies away to rest; opacity reaches 1 at 20%. | 0.5 s |
| Pop with overshoot | Scale 0 → 1 + 0.15·k at 60% → 1. | 0.4 s, back-out |
| Stretch in | Width 1 + 0.6·k and height 1 − 0.4·k → rest, with one small wobble. | 0.5 s |
| White flash reveal | Starts tinted white at 100·k% and fades the tint by 40% of the slot; opacity reaches 1 at 10%. One flash only. | 0.4 s |
| Squash-and-stretch pop | Height 0 → 1.25 (width 0.8) → 0.85 (width 1.1) → 1, scaled by k around 1. | 0.6 s |

*Exit presets:* each entrance played backwards in time, from rest to its
start pose, with the easing mirrored. Directional exits name where they go;
the default is the opposite side of the entrance default (slide out to the
right), so a default in and out read as one movement.

*Continuous presets:*

| Preset | Kind | Motion | Default cycle |
|---|---|---|---|
| Pendulum sway | loop | ±8·k° about the top centre, sine. | 1.6 s |
| Spin loop | loop | One full turn per cycle, linear; Direction clockwise or counter-clockwise. No Intensity. | 2.0 s |
| Bounce loop | loop | Hops 0.05·k se, shaped like a bouncing ball. | 0.8 s |
| Heartbeat pulse | loop | Scale 1 + 0.08·k, then 1 + 0.05·k, in the first 40% of the cycle; rest after. | 1.0 s |
| Jitter shake | loop | Smooth seeded jitter of ±0.01·k se and ±1.5·k°, changing direction 12 times a second. | continuous |
| Float | loop | Up and down ±0.02·k se, sine. | 2.4 s |
| Rotate-and-zoom cycle | loop | ±6·k° and scale 1 ± 0.05·k, sine, a quarter cycle apart. | 2.0 s |
| Slow zoom-in drift | drift | Scale 1 → 1 + 0.1·k across the clip, ease-in-out. | — |
| Slow zoom-out drift | drift | Scale 1 + 0.1·k → 1 across the clip, ease-in-out. | — |
| Pan drift | drift | Moves 0.05·k se in the chosen direction across the clip, linear. | — |
| Combined presets | combined | The matching entrance over the first portion, a through-motion in the middle (zoom through: scale keeps growing by 0.1·k; spin in and out: one slow turn; slide through: keeps moving at the entrance's end speed; pop in then fade out: rest), and the matching exit over the last portion. | portion 30% (FX-019) |

**Behavior / output:**
- Every preset ends an entrance exactly at rest and starts an exit exactly at
  rest, so there is no jump where the slot meets the rest of the clip.
- Offsets are relative to the clip's own keyframed transform and are folded
  into it at L8 (FX-001): positions add, scale factors multiply, rotations
  add, all about the transform anchor. A tracked overlay (AI-008) gets the
  offset after its tracked position. Opacity multiplies at L9.
- **From edge** needs the layer's bounds at rest. The frame plan carries them
  (text from TX's layout, elements from OV's shape bounds, media from the
  fitted source rectangle), so the evaluator stays pure.
- **Photosensitivity rule:** no preset may change brightness or opacity
  sharply more than 3 times in any second (the widely published three-flash
  guideline). White flash reveal has a single flash; flicker-type loops are capped at 2
  dips per second. A unit test samples every manifest and checks it.
- Seeded presets (jitter, shake) use a seed stored in the spec's params, so
  preview, export and every OS agree.

**Edge cases:**
- Intensity 0 on a moving preset: it plays as a pure fade (if it has one) or
  does nothing, and the tile is still shown as selected.
- Slide **From edge** on a layer already off-canvas at rest: it starts the
  same distance further out, so it still moves.
- 3D flip on a layer rotated in 2D by keyframes: the 3D turn is applied
  before the 2D rotation, so the flip axis stays the layer's own.
- Swing about an edge of a cropped clip: the pivot is the visible (cropped)
  edge.
- Stretch or squash on a layer with a mask (TR): the mask moves with the
  layer, because both are inside L8's transform.
- A manifest whose channels reach an unknown property: refused at load and
  shown as **Unavailable animation** (FX-018).

**Undo:** n/a for the catalogue itself. Choosing presets and changing
Intensity or Travel are FX-018's `SetClipAnimation`.

**Model notes:** A preset is a JSON manifest in `resources/animations/`
(text presets in `resources/text-animations/`, TX-056):
`PresetManifest { id, kind: Entrance | Exit | Loop | Drift | Combined, units:
Option<UnitSet>, directions: Option<Vec<Direction>>, default_duration,
easing, pivot: Pivot, channels: Vec<Channel { property: TranslateX |
TranslateY | Scale | ScaleX | ScaleY | Rotate | TurnX | TurnY | Opacity | Blur
| Reveal | Tint | Style(StyleChannel), curve: Curve, amplitude: f64, unit:
Se | Em | Deg | Ratio, scales_with_intensity: bool }> }`. Exits are derived
from entrances unless a manifest of its own exists. `Curve` reuses TR-031's
easing presets plus sampled curves (monotone cubic through up to 16 points).
`intensity: PerMille` and `travel: Travel::{FromEdge, Short}` live in
`AnimationSpec.params`. Evaluation is the pure `animation_offsets` of FX-018
in `freecut-timeline`; amplitudes are `f64` (non-time scalars, TR's
convention) and time is rational. Each manifest has golden samples at 0, 25,
50, 75 and 100% of its slot.

*FreeCut decision (S2 model, 2026-10-13):* the typed manifest lives in
`freecut_timeline::motion` (parsed from JSON by `freecut-project`). Additions to the shape
above: `defaults: ParamDefaults { overlap, order, pace }` (TX-056's per-preset defaults) and the
unit `Travel` (the slide distance, written for a slide from the left; directions turn it).
Curves: `Eased` (the slot easing, `1 − E(p)` toward rest), `Ease(Interp)` (a fixed easing),
`Sampled { points, eased }` (monotone cubic, Fritsch–Carlson), `Sine` (one cycle) and `Step`
(typewriter). Translations add, scale, opacity and reveal multiply by `1 + v`, rotations and
turns add. Exits without their own manifest play the entrance backwards (`p → 1 − p`); an
easing override on such an exit is the exit's own curve. Unknown channel names parse to `None`,
which makes the manifest unavailable. `animation_offsets(manifest, spec, slot, local_t, window,
rest_bounds)` takes the effective window instead of the clip length, so it never repeats the
window rule. The photosensitivity test counts swings of opacity or tint of at least 10 % (a
flash is two opposing swings) in any one second of each preset at its defaults: at most 3, and
at most 2 for loops. Built-in samples for tests: Fade, Rise, Slide, Pop, Typewriter, Wave.

*FreeCut decision (S2 project, 2026-10-13):* the manifest file format. One JSON object per
file, named `<id>.json`: `schema` (1; a newer schema is ignored with a log line, so a preset
made for a newer FreeCut is missing and clips that chose it show **Unavailable animation**),
`id`, `kind` (`entrance`, `exit`, `loop`, `drift`, `combined`), optional `units` and
`directions` (first = default), `default_duration_ms` (whole milliseconds, exact in ticks),
`easing` (TR-031 names as in project files: `ease-out-cubic`, `overshoot-out`, …), optional
`pivot` (`anchor` default, `top-center`, `bottom-center`, `edge`), optional `defaults`
(`overlap` per-mille, `order`, `pace_ms` 20–1000) and `channels`: `{"property": "translate_y",
"curve": {"type": "eased" | "ease" (+ "easing") | "sampled" (+ "points": [[t, v], …], "eased")
| "sine" | "step"}, "amplitude", "unit": "se" | "em" | "deg" | "ratio" | "travel",
"scales_with_intensity"}`. Unknown keys are refused (a typo must not change a preset
silently); an unknown channel name parses but makes the preset unavailable. The bundled
presets are compiled in from `resources/animations/` and `resources/text-animations/`
(`freecut_project::presets::BUNDLED`, with a test that the table matches both directories and
that the sample files equal the timeline's built-in samples).

*FreeCut decision (S2 catalogue, 2026-10-13):* the whole catalogue above and TX-056's is
bundled. Clip presets have ids of their own (`fc.clip.in.grow`, `fc.clip.loop.pendulum`,
`fc.clip.drift.pan`, `fc.clip.combined.zoom-through`); exits play the entrance backwards
(`fc.clip.out.x`). Choices where the table leaves room: an unnamed easing is ease-out (quad);
a named overshoot amount (back-out, elastic, "one small wobble") is drawn by the curve's points
with a linear slot easing, as the Pop sample does (Rise with a small bounce passes rest by 8 %
at 55 %; Swing in overshoots by 12 % at 60 %; Stretch in wobbles 10 % at 65 %); Swing in is a 2D
turn about the middle of the chosen edge; Squash-and-stretch reaches height 1.25 at 40 % and
0.85 at 70 %; Shake in places its 12 Hz peaks at the default 0.5 s, so a longer slot shakes
slower; Rotate-and-zoom turns first and zooms a quarter cycle later; Pan drift's direction is
where it moves to (default right); drifts and combined presets store 5 s as their nominal
length. Combined presets are drawn over the whole window at the default 30 % portion (zoom
through: grow from small, +0.1·k through the middle, then grows away by 0.4·k and fades; spin in
and out: −180·k° to rest, one turn, on by +180·k° with scale 0.5 at both ends; slide through:
in from the chosen side, 0.1 travel through the middle, out to the far side; pop in then fade
out); the portion control waits for the engine. Motions that need a channel the engine lacks
(seeded per-unit offsets and turns, seeded jitter, flicker and sparkle moments, a pull toward
the line centre, a line-clipped rise, a 3D turn whose axis follows the direction) ship as
files naming that channel and load as **Unavailable animation** until it exists:
`resources/README.md` lists them. `freecut-project/tests/catalogue.rs` checks each preset
against these tables, golden samples, the rest rules, Intensity scaling and the
photosensitivity rule (also at Intensity 0 and 100, and at the 0.1 s minimum length).

*FreeCut decision (S2 engine channels, 2026-10-13):* the engine now has the waiting channels
(TX-056 describes the text ones). `turn_from_side` (3D flip in) is a 3D turn whose axis follows
the effective direction: about the vertical axis, positive from the left and negative from the
right; about the horizontal axis, positive from the top and negative from the bottom. A default
Out (to the right) therefore ends at −90°, the mirror of In's start, as Out mirrors slides and
rotations. `jitter.*` (Jitter shake) is smooth seeded noise, 12 turns a second, never past
±amplitude, periodic per cycle; it moves no brightness, so it can't flash. Seeded channels key on
`AnimParams::seed` and the unit index (0 for a whole layer); the dice button belongs on every
preset whose `PresetManifest::uses_seed()` holds. Whole-layer offsets carry what a matrix can't:
`LayerMotion { blur_px, reveal, reveal_from, reveal_feather, tint }`, `reveal_feather` being the
Wipe reveal's soft edge (8 % of the layer size along the wipe, the edge at the `reveal` share and
fading over that width on the hidden side). The photosensitivity checker
(`motion::photosensitivity`) reports flashes and dips per second over opacity, white tint, fill
lightening and glow, at Intensity 0, 50 and 100, the default length and (In and Out) 0.1 s, and
several seeds and units when a seeded channel changes brightness; a preset is offered when it has
at most 3 flashes (2 for loops) and 2 dips in any second. `PresetTable` computes this once per
preset and shares it between clones (`availability(id)`: Ready, Unsafe, Unavailable with the
loader's reason, Unknown). TX-022's loop seam rule applies to any Continuous slot that an In
precedes or an Out follows, which only the text policy allows.

*FreeCut decision (review fixes, 2026-10-13):* shortening a Loop's cycle can never make it
strobe. Each loop has a shortest safe cycle (`motion::min_safe_cycle`, cached by
`PresetCatalog::min_cycle`): the shortest cycle on a 10 ms grid from which every cycle up to the
default passes the photosensitivity checker; 0.1 s for loops that don't change brightness. The
Cycle control's lower bound is this floor; a shorter stored cycle (an older or hand-edited file)
is kept as stored and plays at the floor. The checker also counts the white tint and fill
lightening together, as the renderer adds them per glyph, and a hue turn as the relative
luminance of six saturated fills turned by it (blue to yellow is a large change).

## Interactions with other areas

- **Timeline (TL, `timeline-editing.md`):** owns time representation, tracks,
  track kinds (including the `Caption` kind), layering (TL-004's three visual
  bands and strict layering; TL-007's Flexible stacking, under which the
  Caption band still stays on top), item overlap rules, selection, magnetic main
  track, ripple and linking behavior, split (TL-023), and undo grouping. FX-001
  renders TL's validated track order and does not decide layering itself.
  Transitions keep TL-004's rule that clips on one track never overlap: FX-002
  is built from a head trim of the incoming clip plus TL's ripple of later
  items. TL also owns *Paste attributes…* (TL-034): the command, the attribute
  clipboard and the `PropBag::category()` registry that FX-017 registers its
  Effects and Animations groups through. Effect layers are a TL item kind.
  Compound clips (TL-040) take ordinary clip effects; FX refuses only the
  effects that need per-source analysis on them (FX-020, FX-028), and effect
  regions on inner clips survive compounding (FX-027).
- **Transform, masks, keying, keyframes and speed (TR,
  `clip-transform-and-speed.md`):** owns crop (L3), keying (L4), masks (L7),
  base transform and canvas fit/background (L8, canvas), opacity keyframes (L9),
  the placement of the blend dropdown (TR-016), stabilization (L3), its own
  Paste-attributes groups (TR-034, under TL-034), and the keyframe and easing
  model used by FX-005, FX-014 and FX-018. TR-027's anchoring is canonical for
  FX: keyframes are source-anchored, and a split keeps all keys on both halves
  with no boundary keys. The time map (TR-036) feeds L1 and supplies the pre-roll
  and post-roll that transitions read. Animations (FX-018/019) are composed on
  top of TR's transform, never replacing it. The per-frame order in FX-001 is
  authoritative over TR's "recommended" order. Effect regions (FX-027) reuse
  TR's `Mask` type, its shapes, feather and invert (TR-017, TR-018) and its
  on-canvas handles (TR-019); TR keeps owning those, FX owns only how a region
  limits an effect. FX-028's optical-flow estimator is shared with TR-042.
- **Color and filters (CL, `color-and-filters.md`):** owns L5,
  filter/adjustment layers, and the color conversion done at L2 (CL-002,
  including HDR tone mapping to SDR). Adjustment layers use the same "affects
  everything below" semantics and render-walk position as effect layers
  (FX-011), and under Flexible stacking (TL-007) both can sit lower in the
  stack. FX-026 uses the same scopes and skip-and-count rules as CL-025's
  Apply to all.
- **Text and captions (TX, `text-and-captions.md`):** text has its own In/Out/Loop
  presets and rasterization and enters FX-001 at L8. Text clips use FX's
  animation slots with the "Loop between In and Out" policy (FX-019, TX-022).
  Effect layers above text affect it. TX-025 owns the caption payload; caption
  tracks live in the top Caption band, which FX-001 composites after all other
  visual tracks, so effect and adjustment layers never reach captions. Styled
  text looks (multi-stroke, gradient and textured "word-art" presets, and how
  they combine with a background box) are text styling owned by TX (TX-010 to
  TX-014, TX-019), not entries in FX's effect library. FX-019's combined
  in-and-out presets are not offered on text clips.
- **Overlays and stickers (OV, `overlays-and-stickers.md`):** owns `BlendMode`
  (OV-004): the list, the formulas, serialization and hover preview, which
  FX-001 L10 executes. Element styling feeds FX-001: corner-radius coverage
  (OV-005) at L7, and drop shadow and outline (OV-006), border (OV-005) and clip
  frame (OV-029) at L8b. Stickers and shapes enter at L8 and use FX's animation
  slots with the exclusive policy (OV-018). An animated sticker's own frame
  loop (OV-013's loop mode) is separate from FX-019's Repeat setting, which
  only counts motion cycles.
- **Audio (AU, `audio.md`):** owns the crossfade gain laws (AU-029) that FX-007
  uses, and what happens to a transition's audio when audio is detached
  (AU-003). Clip volume, fades and keyframes multiply with the FX-007 transition
  crossfade. Volume keyframes follow TR-027 on split, like every other keyframe.
  FX owns the transition object itself.
- **AI tools (AI, `ai-tools.md`):** owns local model management, inference and
  analysis caching, background removal (L4), face retouch (AI-020, run at
  FX-001's L2b on raw source pixels) and any segmentation and landmark tracks
  consumed by FX-020/FX-021. AI-008's motion tracker drives tracked effect
  regions (FX-027); AI-021 adds FX-021's privacy effect. AI-001's result key
  lets several People effects share one analysis (FX-020), and AI-002 handles
  model downloads. FX-029 needs a "depth map" analysis kind that
  `ai-tools.md` does not define yet.
- **Preview and export (EX, `preview-and-export.md`):** owns playback policy,
  preview resolution, render cache and output conversion. It must use the
  FX-001 pipeline unchanged.
- **Projects and media (PM, `project-and-media.md`):** owns project frame rate,
  canvas, missing-media placeholders, media references (FX-023) and proxies
  (PM-080, PM-081, used by FX-028). PM-073's task list shows the analysis jobs
  of FX-028 and FX-029, and PM-074's media cache stores their flow and depth
  results.
- **Workspace (UI, `workspace-and-shortcuts.md`):** owns the library panel
  layout and its search, favorites and empty states (UI-006), hover-preview
  plumbing (UI-007), the inspector framework, toasts and the task indicator
  (UI-040; used by FX-002's removal notices, FX-026's summary and FX-028's
  results) and key bindings (e.g., the FX-009 shortcut, and `Mod+Alt+V` for
  *Paste attributes…*).

## Out of scope (and why)

- **Online effect, transition or animation stores and downloads**, including
  download-on-first-use with its failed-download and retry states, and
  creator marketplaces where users publish effects for others (with review,
  approval and usage counts). FreeCut ships everything it offers, offline.
  There are no accounts and no cloud. Local substitutes: the bundled
  libraries, packs installed from files (FX-025) and saved presets (FX-024).
  The reference app's content-review checklist survives only as FreeCut's own
  acceptance tests (FX-004, FX-015).
- **Paid tiers, premium badges and free-use counters** on effects,
  transitions, clip animations, text animations or effect masks, and any
  export-time list of premium items. Everything in FreeCut is free and open
  source; nothing is locked, counted or gated.
- **Account-bound favorites** (signing in to star an item). Favorites are
  local and per user (FX-016, UI-006).
- **Server-side style transfer and prompt-driven "AI looks"** (a clip restyled
  from a prompt or a style picker, processed in a server queue with credits).
  They need cloud models and a credit economy. Local substitutes: the
  shader-based Stylize and 3D-style looks (FX-015), photo parallax with a
  local depth model (FX-029), and packs; a local open style-transfer model
  could be considered later with the AI area.
- **Server processing queues for heavy effects.** Heavy work (the analyses
  behind FX-020, FX-021, FX-028 and FX-029) runs locally as background jobs
  with progress and cancel (PM-073), so there is no remote wait, no "one job
  at a time" refusal and nothing to refund.
- **Trending, popular and promotional categories driven by usage data.**
  FreeCut collects no telemetry, so it has no popularity signal. Categories
  describe the look, and an empty search suggests the user's own recent items
  (FX-016).
- **Machine-learning "smart" transitions that pick or morph between shots
  automatically**, including generated in-between clips made from a text
  prompt for one cut or for every cut. These are ML-heavy, cloud-bound in the
  reference app and loosely defined; they can be reconsidered with the AI area
  later. Substitutes: the transition library (FX-004), custom wipes from
  images (FX-023) and Apply to every cut (FX-006).
- **Beauty retouching, face reshaping and body reshaping as effects.** These
  are ML-heavy and ethically sensitive, and not part of the effects library.
  Face retouch is AI-020 (later); FX only gives it its pipeline slot (L2b).
  Body reshaping is out of scope for the AI area too.
- **Sound effects attached automatically to transitions.** A transition never
  inserts audio by itself; that would hide edits from the user and tie FX to
  the sound library. Users place a whoosh or hit from AU's own sound-effects
  library (AU-031) or their own files on an audio track.
- **Third-party plugin standards (OpenFX, frei0r, etc.).** These are large API
  surfaces. They can be considered after FX-025 proves the shader API.
- **3D scene compositing, particle simulators and physics.** Too large for an
  everyday editor; our particle looks are procedural 2D shaders.
- **Template-driven automatic edits that place effects and transitions for the
  user.** Owned by OV/AI if ever built.

## Clean-room notes

- Every transition, effect and animation preset is FreeCut's own WGSL or Rust
  implementation of generic, well-known techniques (blend, wipe, blur, displace,
  scale/translate curves). Names are FreeCut's own plain descriptions ("Cross
  dissolve", "Push left", "Analog tape look"). No reference-app preset or
  category names are used anywhere in the UI, ids or code (e.g., the
  person-aware effects live under a "People" tab with FreeCut's own name).
- Optional porting source: the gl-transitions collection (MIT licensed as a
  repository; each transition carries its own license header, which must be
  checked and recorded per file before porting to WGSL).
- The pack format of FX-025 is FreeCut's own manifest format. FreeCut ships no
  third-party packs; users install packs they obtained themselves, and each
  pack carries its own license file.
- FX-028's optical flow is our own implementation of published, non-ML
  algorithms (dense pyramidal flow in the DIS or Farnebäck family).
- Grayscale wipe maps, shape outlines and particle sprites: generated
  procedurally or drawn by contributors and released CC0. Tile preview footage
  and stills: our own CC0 recordings.
- The camcorder overlay timestamp uses a bundled OFL-licensed font (e.g., a mono
  font from Google Fonts under OFL-1.1).
- ML models for FX-020/FX-021/FX-029: local open-source models only
  (candidates and their licenses listed in the features; each must be
  verified, including the model card, before bundling or offering a download
  from the model's own upstream). No reference-app models.
- Behavior facts recorded here come from the public tutorials and help pages
  listed in Sources (e.g., the half-of-shorter-clip duration cap and the
  playhead-position rule for quick add from a third-party tutorial; effect
  layers affecting only tracks below them from a third-party explainer of track
  layering), and from the reference app's English UI labels, taken from the
  bundled English localization file of a local install (see Provenance in the
  [README](README.md)) and paraphrased (e.g., the motion-blur option lists,
  the glow direction choices, the 1:1/16:9/9:16 test set, the
  transition-placement and track conflicts noted in FX-002). Nothing was
  decompiled, disassembled or traced, and no assets, project files or network
  traffic were inspected. No UI wording, preset, effect, transition or
  animation names were copied; every label in this spec is FreeCut's own.
  Parameter ranges and defaults marked "FreeCut default" are our own choices.
- No reference-app screenshots, effect packs, LUTs, icons or UI artwork are
  committed or used as tile thumbnails.

## Sources

- CapCut Help — Why can't I see transitions in CapCut — https://www.capcut.com/help/transitions-in-capcut
- CapCut Help (French) — What to do if transitions don't work — https://www.capcut.com/fr-fr/help/transitions-not-working
- CapCut Help — Why are effects not applying — https://www.capcut.com/help/effectss-not-applying-in-capcut
- CapCut Help — How to add a fade-in effect — https://www.capcut.com/help/add-a-fade-in-effect
- CapCut Tools — Video transitions feature page — https://www.capcut.com/tools/video-transition
- CapCut Resource — TikTok transitions tutorial — https://www.capcut.com/resource/tiktok-transitions
- CapCut Resource — Epic transitions template tutorial — https://www.capcut.com/resource/epic-transitions-template-tutorial
- CapCut Resource — AI effects guide — https://www.capcut.com/resource/ai-effect
- CapCut Resource — Video effects guidance — https://www.capcut.com/resource/video-effects-guidance
- CapCut Create — Blend modes article — https://www.capcut.com/create/blend-modes-creative-video-photo-effects
- CapCut Create — Adjustment layers for multiple clips — https://www.capcut.com/create/adjustment-layers-for-multiple-clips
- TourBox — How to add transitions in CapCut — https://www.tourboxtech.com/en/news/capcut-transitions.html
- TourBox — What is the free layer in CapCut — https://www.tourboxtech.com/en/news/what-is-free-layer-in-capcut.html
- TourBox — How to blur on CapCut — https://www.tourboxtech.com/en/news/how-to-blur-on-capcut.html
- Primal Video — CapCut for PC & Mac tutorial (2026) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- Skillademia — How to add transitions in CapCut — https://skillademia.com/capcut/how-to-add-transitions-in-capcut/
- antwort.net — Finding video effects in CapCut PC (German) — https://www.antwort.net/a/tutorial/so-suchst-du-video-effekte-in-capcut-pc-2024--ein-komplettes-tutorial.html
- antwort.net — Logo animation in CapCut PC (German) — https://www.antwort.net/a/tutorial/logo-animation-in-capcut-pc-erstellen-das-ultimative-tutorial.html
- antwort.net — Fade in/out in CapCut (German) — https://www.antwort.net/a/tutorial/capcut-tutorial-2024-so-erstellst-du-perfekte-ein-und-ausblendungen-fade-inout.html
- Platzi — CapCut course, animations and effects (Spanish) — https://platzi.com/cursos/capcut/animaciones-y-efectos-de-video-en-capcut/
- Dailymotion — CapCut animation walkthrough — https://www.dailymotion.com/video/x9o8mx4
- capcutguide.com — CapCut effects overview — https://capcutguide.com/capcut-effects/
- Kdenlive documentation — Mixes / same-track transitions — https://docs.kdenlive.org/en/compositing/transitions/mixes.html
- KDE Discuss — Mixes on the same track — https://discuss.kde.org/t/mixes-how-to-transition-them-in-the-same-track/4202
- Shotcut forum — Creating and removing transitions — https://forum.shotcut.org/t/creating-and-removing-transitions/12958
- Shotcut forum — Transition properties locked — https://forum.shotcut.org/t/transition-properties-locked/44482
- OpenShot user guide — Transitions — https://en.openshot.org/files/user-guide/transitions.html
- tryuncle — DaVinci Resolve transitions — https://tryuncle.com/learn/davinci-resolve/transitions
- Blackmagic forum — Can't place transitions between clips — https://forum.blackmagicdesign.com/viewtopic.php?p=812270
- W3C — Compositing and Blending Level 1 — https://www.w3.org/TR/compositing-1/
- Wikipedia — Blend modes — https://en.wikipedia.org/wiki/Blend_modes
- Google MediaPipe — Pose landmarker overview — https://developers.google.cn/mediapipe/solutions/vision/pose_landmarker
- GL Transitions — https://gl-transitions.com/
- OpenCV Zoo — YuNet face detection model README — https://github.com/opencv/opencv_zoo/blob/main/models/face_detection_yunet/README.md
- MiDaS monocular depth estimation repository — https://github.com/isl-org/MiDaS
- Depth Anything V2 repository — https://github.com/DepthAnything/Depth-Anything-V2
- Kroeger et al., Fast Optical Flow using Dense Inverse Search (arXiv) — https://arxiv.org/abs/1603.03590
- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
