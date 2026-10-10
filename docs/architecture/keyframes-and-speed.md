# Keyframes, easing, speed ramps & retiming

> Status: draft · Crates: freecut-timeline, freecut-render, freecut-audio (also touches freecut-media, freecut-project, freecut-app)

This document designs two closely linked subsystems:
- **Keyframes and easing:** how any numeric property changes over time.
- **Retiming:** constant speed, speed curves, reverse, frame sampling, and audio that follows
  the picture.

They are designed together because keyframes on media clips live in **source time**. A
keyframe is evaluated at "the source instant the picture shows". The retime map defines that
instant. Behavior is owned by the feature specs (`docs/features/clip-transform-and-speed.md`
TR-027…TR-044, `audio.md` AU-008/AU-023/AU-024, `effects-transitions-animations.md`
FX-014/FX-018/FX-019, `timeline-editing.md` TL-001/TL-011/TL-020/TL-023/TL-031). This
document says how to build it, and where the specs need a decision first.

The design in one paragraph:
- Every animatable value is an `Animatable<T>`. It holds either a static value or a sorted
  `KeyframeTrack<T>` whose times are integer ticks on the clip's **key clock**.
- The key clock is source time for media clips and clip-local time with a content offset for
  everything else.
- Every media clip owns one exact, strictly monotone **time map** (`TimeMap`). It has three
  parts:
  - a shape: linear for constant speed, or a compiled piecewise-quartic for curves;
  - a direction;
  - a **window** (`window_start`) that says where the clip sits inside the map.
- Trim and split only move windows, so they cannot change a single rendered frame or sample.
- Speed and reverse edits rebuild the map from the clip's nominal source range, so repeated
  edits never drift.
- `freecut-render`, `freecut-audio` and `freecut-media` only call the map's pure query
  functions (`source_time_at`, `source_span`, `local_rate`). They never re-derive time.

---

## What FreeCut needs

### Requirements

| # | Requirement | Spec |
|---|---|---|
| K1 | One generic keyframe engine for every numeric property. It serves this area (position as one 2D property, scale, rotation, anchor, opacity, mask parameters) and others: color (CL), volume and pan (AU), effect parameters (FX), text (TX). It must be generic over the value type. | TR-027, FX-014, AU-008 |
| K2 | No keys means one static value. Before the first key the value is the first key's value, and after the last key it is the last key's value. Between keys, the **earlier** key's interpolation applies. Adding the first key captures the static value, and removing the last key writes its value back to the static value. | TR-027 |
| K3 | **Anchoring:** media clips store key times in source time. Stills, text, shapes and freeze stills use clip-local time. Keys outside the trimmed range are kept and still shape the curve at the edge. A split keeps **all** keys on both halves and inserts **no** boundary key. | TR-027 |
| K4 | Interpolation types: Linear (default), Hold, Ease in / out / in-out in four strengths (sine, quadratic, cubic, exponential), Overshoot in and out, Elastic out, Bounce out, and Custom cubic-Bézier with x ∈ [0,1] and y ∈ [−1,2]. Overshooting values are clamped when evaluated, never when stored. | TR-031, TR-032 |
| K5 | The Bézier solve must be deterministic, with a fixed iteration count, so that preview and export agree. | TR-032 |
| K6 | Ops for add, remove, set value, move, set interpolation, copy and paste. Multi-select, auto-key, and per-property and global navigation. Each op is undoable and has a reversal test. | TR-028…TR-033 |
| K7 | Value types: `f64`, 2D vectors, fixed-point integers (rounded only when evaluated), linear-light colors, and gain in dB with a −∞ sentinel (AU-008 rules). Rotation interpolates numerically, so 0°→720° spins twice. | TR-027, AU-008 |
| R1 | Each media clip has a time map with modes Normal (constant `r` ∈ [0.1, 100], exact rational) and Curve (2 to 32 points, `v` ∈ [0.1, 10], minimum gap 1 % of the clip, smooth monotone cubic or linear). Each clip also has `reverse` and `frame_sampling`. | TR-036, TR-037, TR-039, TR-040 |
| R2 | `T = D / r` rounded to the nearest frame, minimum 1 frame. Speed changes alter `T`, never `D`. Trim and split are exact, and a split never changes any rendered frame or audio sample. Extending a clip past the authored range continues at the edge speed. | TR-036 |
| R3 | `source_time_at`, `timeline_duration` and `local_rate` are the only implementation of the mapping. Render, audio and media never re-implement it. | TR-036, TR-038 |
| R4 | Frame choice:<br>- Nearest frame by PTS, which also handles VFR sources.<br>- Frame blend: at most 16 samples for speed-ups, a cross-fade for slow motion.<br>- Optical flow (later): cached on disk, falling back to blend until the cache is ready. Export waits for the cache. | TR-041, TR-042 |
| R5 | Reverse is an instant model change. Keys are mirrored inside the current source range, so the animation keeps its place on the timeline, and toggling twice restores the original. Backward decode or caching is done by media. | TR-043 |
| R6 | Freeze takes `s₀` from the map and bakes animated values into the still. | TR-044, TL-031 |
| R7 | Audio follows the map: varispeed when *Keep pitch* is off, time-stretch when it is on (the default). Ratios vary continuously, and audio and picture stay within one block of each other with no drift. Speeds below 0.05× are silent. Pitch shift and formants are applied in the same pass. | TR-038, AU-023, AU-024 |
| R8 | Entrance, exit and continuous animations are **timeline-time**, edge-anchored presets. They compose with keyframes and are not keyframes. | FX-018, FX-019 |
| R9 | Frame-rate conform, footage rate interpretation and project packaging must update key times and maps consistently. | PM (project-and-media.md §frame rate, interpret rate, package) |

Limits used below: speed 0.1×–100× (Normal) and 0.1×–10× (curve points). At most 32 curve
points. Resulting duration at least 1 frame. Blend uses at most 16 source frames per output
frame. Keep-pitch audio is silent below 0.05×.

### Hard invariants (derived; each one is a test in §Test strategy)

1. **Split invariance.** For every frame and sample, splitting a clip at a frame boundary
   leaves the rendered video bit-identical. Varispeed audio is bit-identical too, and keep-pitch
   audio is equal within a stated tolerance.
2. **Monotone maps.** For a forward map, `t₁ < t₂ ⇒ s(t₁) < s(t₂)` (strict). A reversed map is
   strictly decreasing.
3. **Reverse round trip.** Reversing twice gives keys and interpolations identical to the
   original, and a `Retime` equal after canonicalization.
4. **Edge exactness.**
   - Normal: `s(0)` equals the stored source start exactly for an unsplit clip. For a split half
     it is a value whose floor is the source start. Rounding `T` can make the last frame reach up
     to half a frame × `r` past `out`; readers clamp.
   - Curve, as compiled: `s(T)` equals the source end exactly.
5. **No drift.** A speed change and its inverse (1× → 0.3× → 1×) restore `T` exactly. An audio
   stream consumes exactly the source samples between the map's values at its first and last
   block edges.
6. **Key-instant exactness.** When a frame is sampled at the time a key was created, it
   evaluates to that key's stored value bit-exactly. This holds after splits, speed changes and
   reverse.
7. **Determinism across operating systems.** Time math uses only integer arithmetic. Value
   easing uses the `libm` crate, never the platform's libm, so golden hashes match on macOS,
   Windows and Linux.

### Spec inconsistencies this design resolves (need owner sign-off)

| # | Conflict | Resolution proposed here |
|---|---|---|
| C1 | TL-023 ("interpolated value inserted as a keyframe on both sides") and AU-008 ("boundary keyframes are added at the split point") contradict TR-027 ("both halves keep all keyframes, no boundary keyframe"). `edit-ops-and-undo.md` lists this as open question 1. | Follow TR-027. A boundary key turns one eased or Bézier segment into two segments with the same easing, which changes the motion. Kdenlive does exactly this (see below). Keeping all keys on both halves, plus window-preserving maps (§5.4), is the only rule that is exactly invariant. TL-023 and AU-008 need edits. |
| C2 | TL-011 derives `D` from `T` ("source_duration = timeline_duration × s, rounded down"). TR-036 derives `T` from `D`. | Both, depending on the op, as `timeline-model.md` §5 already proposes: **speed edits keep D and recompute T; trims and splits keep T and recompute D** from the exact map. |
| C3 | TR-036 says the curve integral `P` is "computable exactly (rational arithmetic, `i128` intermediate)". | False at tick resolution. A 10-minute segment needs `h⁴ ≈ 3.2·10⁴⁶`, which exceeds the `i128` maximum of 1.7·10³⁸ (measured). §5.6 uses integer-coefficient polynomials evaluated in 256-bit integers. |
| C4 | TR-036 trims and splits curves by "Hermite subdivision at the cut, remaining points re-normalized". | Exact subdivision has unbounded denominators, so storing it means rounding, and rounding breaks invariant 1. Use **windows** instead (§5.4). The authored curve is never cut by trim or split, and is re-authored only when the user edits the curve. |
| C5 | TR-036 defines the reverse map as `s(t) = out − t·r`. With "the frame whose interval contains s(t)", `t = 0` selects the frame **after** the range. `timeline-model.md` raised this too. | Reversed clips use **left-continuous** frame selection: the frame whose interval `(a, b]` contains `s`, i.e. "the last frame starting strictly before `s`" (§5.8). This needs no special case for curves. |
| C6 | TR-031's preset list has Elastic out and Bounce out but no "in" versions, and no way to reverse a Hold. TR-043 mirrors keys under reverse. | Add Elastic in, Bounce in, Overshoot in and an internal `HoldAtStart` to the enum so that mirroring is an exact involution. The UI may keep showing only the spec's list. |
| C7 | TR-027 says generated clips use clip-local time, and that a split leaves output unchanged. For a keyed text clip both cannot be true: the right half's clock would restart at 0. | Give generated and still clips a `content_offset: Tick`. The key clock becomes `t + content_offset`. A head trim or a split adds to the offset (§4.6). |
| C8 | AU-023 model notes put *Keep pitch* in `Retime.preserve_pitch`. TR-038 puts it in `ClipAudio.preserve_pitch`. | Use `ClipAudio.preserve_pitch`, as TR-038 says. It is an audio setting, and detached audio clips (TL-030) can then differ from their video. |

---

## How the reference editors do it

Paths are relative to each repository root, and each heading gives the license. Algorithms are
described in our own words. No reference code is reproduced here.

### libopenshot (LGPL-3.0-or-later)

**Key files:**
- `src/Point.h`, `src/Point.cpp`, `src/Coordinate.h`
- `src/KeyFrame.h`, `src/KeyFrame.cpp`
- `src/Clip.cpp`: `End()` at L428, `apply_timemapping()` at L587, `GetOrCreateFrame()` at L751
- `src/Timeline.cpp`: per-clip frame numbers around L1074–1090, volume ramp at L693
- `tests/KeyFrame.cpp`, `tests/Clip.cpp` ("time remapping", GIF time mapping)

**Data.**
- A `Keyframe` is a sorted `std::vector<Point>`.
- A `Point` has:
  - a coordinate `co`, where `X` is a **frame number** stored as a `double` and `Y` is the value;
  - left and right Bézier handles stored as **fractions of the segment** (0…1 of Δx and Δy);
  - an `InterpolationType` (BEZIER, LINEAR, CONSTANT);
  - an AUTO/MANUAL handle type.
- The default handles give an ease-in-out shape.
- The interpolation of a segment is taken from the **right** (end) point
  (`InterpolateBetween`, `KeyFrame.cpp` L80). The test "use segment end point interpolation"
  pins this behavior.
- `AddPoint` with an existing X overwrites that point (L131).
- Time is frames, so a keyframe file is tied to one frame rate.

**Bézier evaluation** (`InterpolateBezierCurve`, L44):
- It bisects the curve parameter until the x error is below 0.01 frames.
- The loop has **no iteration cap**. A handle with x outside [0,1] makes x(τ) non-monotone and
  can stop it from converging.

**Bulk edits.**
- `ScalePoints` (L522) rounds every X to an integer frame. It carries TODOs for points that
  collide and for negative scales.
- `FlipPoints` (L536) swaps only the Y values. A TODO admits the handles are not mirrored.

**Retime.**
- A special `time` Keyframe on the clip maps clip frame → source frame. It is evaluated with
  `GetLong`, i.e. rounded to whole frames, so there is no sub-frame precision.
- Reverse is a decreasing curve. The clip length is the curve's last X (`Clip::End`, L428).
- Direction comes from `IsIncreasing` (L292), which **scans up to 600 frames ahead** for a
  value change.
- Audio (`apply_timemapping`):
  - takes the curve's delta per frame, collects `|delta| × samples-per-frame` source samples
    across source frames, and resamples them into the frame;
  - is plain varispeed with no pitch preservation;
  - keeps state between calls (`previous_location`, and a resampler that is reset when the
    position jumps by more than 2 frames or the direction flips). The audio therefore
    **depends on the order in which frames are requested**: a seek or a parallel export
    produces different samples than continuous playback.

**Property keys.** They are evaluated at "clip frame number" = timeline frame − position +
`Start` (`Timeline.cpp` ~L1080). Keys therefore stay with the content on a head trim, but they
are in **output** frames, not mapped through `time`. A speed change does not stretch them.

**Volume.** Volume is ramped linearly inside each frame from the previous frame's value
(`Timeline.cpp` L693), so the automation resolution is one frame.

**What is worth porting** (LGPL-3.0-or-later is GPL-3-compatible): only the idea of
segment-relative handles, which keep their shape when keys move or stretch. The code is too
float- and frame-bound to reuse.

### Olive (GPL-3.0-or-later)

**Key files:**
- `app/node/keyframe.h`, `app/node/keyframe.cpp`: `set_type` at L99, `valid_bezier_control_in/out` at L154+
- `app/node/node.cpp`: keyframe evaluation around L420–540
- `app/node/value.cpp`: split values, around L95
- `app/node/block/clip/clip.cpp`:
  - `set_length_and_media_out` L95, `set_length_and_media_in` L110
  - `SequenceToMediaTime` L155, `MediaToSequenceTime` L201
  - `InputTimeAdjustment` L465
- `app/node/time/timeremap/timeremap.cpp`
- `app/node/output/track/track.cpp`: audio speed, reverse and pitch around L628–720
- `app/audio/audioprocessor.cpp`: the `atempo` chain at L87
- `app/timeline/timelineundosplit.cpp`: L32
- `app/dialog/speedduration/speeddurationdialog.cpp`

**Data.**
- `NodeKeyframe` has:
  - a `rational` time;
  - a `QVariant` value;
  - a type: Linear, Hold or Bezier;
  - in/out Bézier handles stored as **absolute offsets** in seconds and value units, relative
    to the key.
- At evaluation the handles are clamped so they never reach past the neighbouring key
  (`valid_bezier_control_*`).
- Multi-component values (vec2/3/4, color) are keyed **per component**: each has its own track
  (`value.cpp` `split_normal_value_into_track_values`).

**Evaluation** (`node.cpp`):
- A binary search finds the segment.
- At a key time it returns that key's value exactly.
- Hold applies when the earlier key is Hold.
- If both keys are Bézier the segment is a cubic Bézier. If only one is, it is a quadratic with
  that key's handle. Otherwise it is linear.
- The Bézier x→y solver lives in the `olive-core` submodule, which is **empty in this clone**
  (`ext/core`), so it could not be inspected.

**Retime.**
- `ClipBlock` has `speed` (a `double`), `reverse` and `maintain_audio_pitch`.
- `SequenceToMediaTime`:
  - reverses as `length − t`;
  - multiplies by the speed through `rational::fromDouble(t.toDouble() * speed)`, which loses
    exactness;
  - takes a `qFuzzyCompare(speed, 1.0)` shortcut.
- Speed 0 means "hold the in-point". The inverse then returns NaN, with a comment saying the
  author does not know what to return.
- Nodes that feed a clip are evaluated in **media time** (`InputTimeAdjustment`), so Olive's
  keyframes are effectively source-anchored, like FreeCut's.
- A reversed clip's head trim does not move `media_in`, but its tail trim does
  (`set_length_and_media_out/in`). This is the correct bookkeeping for reverse.
- `TimeRemapNode` is a keyed rational "time" input, After-Effects style. Its inverse is
  commented out, so source changes cannot be mapped back to output ranges for cache
  invalidation.

**Split** (`BlockSplitCommand::prepare`) copies the whole node graph, so both halves keep all
keyframes.

**Audio** (`track.cpp`):
- For every render block a **new** `AudioProcessor` runs FFmpeg `atempo`, chained in steps
  between 0.5× and 2× (`audioprocessor.cpp` L87).
- The FIXME at `track.cpp` L668 admits that "users may hear clicks and pops" because the
  tempo filter is fed discontinuous blocks.
- Varispeed and reverse are applied to the samples after that.

### MLT (LGPL-2.1-or-later; `src/modules/rubberband` is GPL-2.0-or-later)

**Key files:**
- `src/framework/mlt_types.h`: `mlt_keyframe_type`, L183–221
- `src/framework/mlt_animation.c`:
  - item parsing, with negative positions at L453
  - `mlt_animation_get_item` L477, `mlt_animation_shift_frames` L1015
  - `catmull_rom_interpolate` L1117, the easing family L1160–1297, `interpolate_value` L1299,
    `interpolate_item` L1400
- `src/modules/core/link_timeremap.c` (and `.yml`):
  - `integrate_source_time` L59, `link_get_audio` L124
  - blend and nearest images at L349 and L464, `link_get_frame` L525
- `src/modules/core/producer_timewarp.c`
- `src/modules/rubberband/filter_rbpitch.cpp`

**Animation.**
- An animation is a linked list of items: integer frame, is-key flag, keyframe type, and the
  value as a property string.
- Keys can be written as clock strings, which are converted to frames through fps. **Negative
  positions count from the end** (L453), so a key can be anchored to the clip's out point.
- Types:
  - discrete and linear;
  - three Catmull-Rom variants: uniform (may overshoot), centripetal with natural slopes, and
    centripetal with zero slopes;
  - the full Penner families (sine, quad, cubic, quart, quint, expo, circ, back, elastic,
    bounce), each in in, out and in-out.
- Missing neighbours are synthesized for Catmull-Rom by pushing a duplicated point 10 000 frames
  away. This is a hack that slightly bends the first and last segments.
- Colors are interpolated per 8-bit sRGB channel, i.e. not in linear light.
- Lookup walks the list linearly.
- Several of the closed forms are not exact at their endpoints. The `back` in/out variants give
  `0.9999999999999999` at p = 1 and `1.1e-16` at p = 0 (measured by re-evaluating the formulas).
  MLT hides this by returning the stored value at key positions.

**`timewarp` producer.**
- The speed is parsed from the resource string `timewarp:<speed>:<file>`.
- The producer is a clip producer built with a **fake profile** whose frame rate is
  `fps / |speed|`, multiplied by 1000 to keep precision (L267).
- Reverse maps a position to `out − position`. MLT's `out` is inclusive, so this is correct
  there.
- Audio relabels the sample rate as `rate × |speed|` and resamples.
- `warp_pitch` adds `rbpitch` with `pitchscale = 1/|speed|` (it pitch-shifts after the
  varispeed). It is disabled below 0.1× (L171).

**`timeremap` link.**
- It takes either `time_map` (output → source seconds, keyframed) or `speed_map` (speed against
  output time, keyframed).
- `speed_map` is **integrated numerically, one frame at a time** (a rectangle rule) and cached
  incrementally (`integrate_source_time`). A far seek restarts the integration, so seeking to
  frame N costs O(N) keyframe lookups, and float error accumulates.
- Source frames are collected with `floor(source_time × fps)`.
- Image modes:
  - `nearest`;
  - `blend`: an **equal-weight** average of every source frame in the interval, at most 10
    frames, in the stored pixel format. Encoded values are averaged, not linear light. With one
    frame in the interval it falls back to nearest, so slow motion never cross-fades.
- Audio:
  - a sample-continuity heuristic of ±4 samples (L177);
  - **silence outside 0.1×–10×** (L152);
  - reverse by flipping each frame's chunk;
  - speed applied as a sample-rate relabel plus a resampler, or `rbpitch`.

**`rbpitch`.**
- It wraps a stateful Rubber Band real-time stretcher.
- `pitchscale` is clamped to 0.05–50 and the sample rate to 10 k–300 k (L90). The source
  comment gives library crashes at extreme values as the reason.
- When the stretcher starves, it **re-feeds the same input** (L165).
- It reports latency (L213) but never compensates it, so audio lags by the stretcher latency.

**Portable** (LGPL-2.1+ is GPL-3-compatible): the catalogue of easing families, and the
"keys relative to the end" idea. The time-remap integrator should not be ported.

### Shotcut (GPL-3.0-or-later)

**Key files:**
- `src/widgets/avformatproducerwidget.cpp`: `newProducer` at L92 builds the timewarp resource
  and works around locale decimal separators
- `src/docks/timelinedock.cpp`: L2520–2560 rescales in/out on a speed change
- `src/mltcontroller.cpp`: `adjustFilters` L1356, `shiftKeyframes` L1450, `adjustFilter` L1522,
  `adjustClipFilters` L1664
- `src/qml/filters/speed/` (`meta_forward_reverse.qml`, `SpeedUI.qml`)

**Two kinds of animation.**
- *Simple* keyframes (`animIn`/`animOut` durations) are anchored to the clip edges and
  **regenerated** after every trim.
- *Advanced* keyframes are shifted by the in-delta on a head trim, so they stay with the
  content (`shiftKeyframes`).
- Keys that would get a negative position are **deleted** after inserting an interpolated key
  at 0. The comment explains that users could not reach them. This is lossy: extending the head
  again does not bring them back.

**Speed.**
- The timewarp producer is rebuilt on every change.
- In and out are rescaled by `oldSpeed/newSpeed` with `qRound` (L2535), so repeated changes
  drift.
- Filters are adjusted as if the clip had been trimmed, so keys are not stretched.
- The speed has to be formatted by MLT, not Qt, to avoid locale mismatches
  (`avformatproducerwidget.cpp`).
- The *Speed: Forward & Reverse* filter puts `timeremap` with a keyframed `speed_map` from −10×
  to 10× in front of the user. It sets `allowTrim: false` so its keys are not shifted on trims
  (`meta_forward_reverse.qml` L19).

### Kdenlive (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)

**Key files:**
- `src/definitions.h` L211: the `KeyframeType` subset of MLT's types
- `src/assets/keyframes/model/keyframemodel*.cpp`
- `src/assets/keyframes/model/keyframemodellist.cpp`: `resizeKeyframes` at L783
- `src/effects/effectstack/model/effectstackmodel.cpp`: `adjustStackLength` at L854
- `src/timeline2/model/clipmodel.cpp`: `refreshProducerFromBin` L868, `useTimewarpProducer` L1065
- `src/dialogs/timeremap.cpp`: `RemapView::updateBeforeSpeed` L1174, the HACK at L1292
- `src/utils/gentime.h`, `src/utils/gentime.cpp`

**Time.** `GenTime` is a `double` of seconds. Its comparisons use a **global, mutable** epsilon
set to `0.9/fps` (`gentime.cpp` L116). Two times less than 0.9 frames apart compare as equal.

**Trims destroy keys** (`resizeKeyframes`):
- On a head trim, an interpolated key is inserted at the new in point and every key before it
  is deleted.
- A tail trim does the same at the end.
- When only two keys exist, the last one is *moved* instead (L868), which stretches the
  animation.
- Inserting boundary keys keeps the old interpolation type on both halves, which changes eased
  motion.

**Fades.** Fade effects are re-anchored to the clip edges in `adjustStackLength`. This is the
same split as Shotcut: edge-anchored animations versus content keys.

**Speed** (`useTimewarpProducer`):
- The speed is a `double`.
- The producer is rebuilt as a timewarp producer.
- In and out are remapped with `qRound64` (`refreshProducerFromBin`, L876–890), so they drift.
- A comment at L1115 admits that the computed duration can overshoot the possible one by a
  few frames, so it is corrected afterwards.
- Undo has to restore in and out explicitly.
- Effects are adjusted like a trim, so keys are not rescaled.

**Time remap** (`timeremap.cpp`):
- Keys are integer output frame → integer source frame.
- Each segment's speed is Δsource/Δoutput. Editing one segment's speed moves every later key
  (ripple) so their speeds stay the same.
- The map is serialized as clock strings, i.e. seconds.
- A HACK comment (L1292) places the last key one frame later, so that the final frame comes
  out right. This is an off-by-one workaround.
- Requested speeds are rounded to whole frames, so the stored speed differs from the typed one.

GPL-3.0-only code may be studied, but copying it would make the combined work GPL-3.0-only.
Nothing here is worth porting anyway.

### OpenTimelineIO (Apache-2.0)

**Key files:**
- `src/opentime/rationalTime.h` (L229, L408)
- `src/opentimelineio/linearTimeWarp.h`, `freezeFrame.h`, `timeEffect.h`
- `docs/tutorials/otio-timeline-structure.md` (L19)
- `docs/tutorials/otio-serialized-schema.md` (L363)
- `docs/tutorials/time-ranges.md`

**Model.**
- `RationalTime` is `(double value, double rate)`, and `to_frames()` **truncates**.
- `LinearTimeWarp(time_scalar: double)` leaves the item's duration unchanged (schema doc L363):
  the scalar changes how fast media is consumed inside an unchanged `source_range`.
- `FreezeFrame` is a time effect.
- The structure doc (L19) leaves the effect's impact to each application. Time effects are
  metadata.
- There is no keyframe or curve schema in core.

**Relevance.** OTIO is the interchange target. Normal speed exports as `LinearTimeWarp(r)`. A
reversed clip conventionally exports as `LinearTimeWarp(-r)`; check this against the target
NLE. Freeze stills export as `FreezeFrame`. Curves cannot be represented and must be baked or
put in metadata (§11).

### Side by side

| | libopenshot | Olive | MLT | Shotcut | Kdenlive | **FreeCut** |
|---|---|---|---|---|---|---|
| Key time unit | frame number (`double`) | `rational` seconds | int frame (or clock string) | MLT | `double` s, eps 0.9 frame | **`Tick` (1/705.6 M s)** |
| Key anchoring | clip frame + trim-in, output frames | media time | filter frames | edge-anchored *simple* + content-shifted *advanced* | producer frames; trims delete/insert keys | **source time (media); clip-local + offset (generated)** |
| Segment easing stored on | end point | both ends combine | start item | MLT | MLT | **start key (outgoing)** |
| Bézier handles | fractions of segment | absolute offsets, clamped | — (Catmull-Rom) | MLT | MLT | **fractions (CSS-style), Q16 fixed point** |
| Bézier solve | unbounded bisection | (in olive-core) | — | — | — | **fixed 48-step bisection** |
| Retime representation | `time` keyframe frame→frame | `double` speed / keyed time node | fake-fps producer / keyed speed or time map | MLT | MLT + remap dialog | **exact linear map or compiled quartic, with a window** |
| Curve integration | n/a (time curve) | n/a | per-frame Euler sum | MLT | MLT | **closed form, exact** |
| Reverse | decreasing curve | `length − t` | `out − pos` (inclusive out) | MLT | negative speed | **direction in map + left-continuous frame rule** |
| Split | n/a | copies graph | — | — | inserts boundary keys | **windows; keys kept on both halves** |
| Keep-pitch | none | per-block `atempo` (clicks) | Rubber Band, no latency compensation | MLT | MLT | **continuous stretcher with pre-roll + latency compensation** |
| Frame blend | none | none | equal weights, encoded values, speed-up only | MLT | MLT | **overlap weights, linear light, + slow-mo cross-fade** |

### Pitfalls they hit (what this design defends against)

| # | Pitfall | Where | Defense |
|---|---|---|---|
| P1 | Time as `double` or float frames: epsilons, truncation, drift | Kdenlive `GenTime`; OTIO `to_frames`; Olive `fromDouble` | Integer `Tick`; exact `Rational`; I256 curves (§3, §5) |
| P2 | Speed changes rescale in and out with rounding, which drifts | Kdenlive `refreshProducerFromBin`; Shotcut `timelinedock.cpp` L2535 | Speed edits keep D, so T is a pure function of (D, r) (§5.5) |
| P3 | Trims delete keys, or insert boundary keys that change easing | Kdenlive `resizeKeyframes`; Shotcut `shiftKeyframes` | Keys are never touched by trim or split (§4.7) |
| P4 | Speed changes do not stretch keys (treated as a trim) | Kdenlive, Shotcut, libopenshot | Keys live in source time, so they stretch with the footage for free |
| P5 | Numeric per-frame integration of speed: O(N) seeks, float drift | MLT `integrate_source_time` | Closed-form prefix sums, O(log k) per query (§5.6) |
| P6 | Unbounded or non-monotone Bézier solve | libopenshot L44 | x handles clamped to [0,1]; fixed iteration count (§4.3) |
| P7 | Easing endpoints not exact; platform `libm` differences | MLT `back_*` | Key instants return stored values; p=0/1 special-cased; `libm` crate (§4.2) |
| P8 | Mirroring keys forgets the handles | libopenshot `FlipPoints` | `Interpolation::mirrored()`, an exact involution in Q16 (§4.3) |
| P9 | Reverse selects the frame after the range at t=0 | Olive `length − t`; TR-036 wording | Direction-aware frame containment (§5.8) |
| P10 | Audio depends on request order or seeks | libopenshot `previous_location`; MLT continuity margin | Stateless varispeed; positions from absolute map values; stretcher pre-roll (§7) |
| P11 | A tempo filter fed discontinuous blocks clicks | Olive FIXME | One persistent stretcher per clip stream, reset only on seek or edit |
| P12 | Stretcher latency not compensated; starvation re-feeds input | MLT `rbpitch` | Latency compensation and pre-roll; exact input counts per block (§7.3) |
| P13 | Blending encoded values, equal weights, no slow-mo blend | MLT `link_get_image_blend` | Linear-light, overlap-weighted, cross-fade for slow motion (§6) |
| P14 | Locale-formatted numbers in persisted strings | Shotcut, Kdenlive (clock strings) | Integers and `[num, den]` pairs only in JSON (§10) |
| P15 | Catmull-Rom overshoot can make speed negative (source runs backwards) | MLT smooth types in `speed_map` | Monotone (Steffen) Hermite slopes keep every speed between its neighbours (§5.6) |
| P16 | Inverse map missing, so caches cannot be invalidated | Olive `TimeRemapNode::OutputTimeAdjustment` (commented out) | `timeline_offset_of` exact for Normal; binary search for curves (§5.7) |

---

## Recommended FreeCut design

### 1. Overview

```text
                 freecut-timeline (pure, no I/O, #![forbid(unsafe_code)])
 ┌──────────────────────────────────────────────────────────────────────────┐
 │ anim/      Animatable<T> ─ KeyframeTrack<T> ─ Interpolation (easing)      │
 │              ▲ evaluated at key_time_at(clip, t)                           │
 │ retime/    Retime { mode, reverse, sampling, TimeMap{shape, dir, window} } │
 │              source_time_at · source_span · local_rate · timeline_offset_of│
 │              plan_frame(grid) → FramePlan        freeze_settings_at        │
 └───────────────▲────────────────────────▲─────────────────────▲───────────┘
                 │ FramePlan              │ BlockPlan            │ SourceFrameGrid
        freecut-render                freecut-audio          freecut-media
   nearest / blend (WGSL, linear    varispeed reader /      PTS index, reverse-
   light) / flow cache fallback     stretcher w/ pre-roll   friendly GOP cache
```

### 2. Module layout

This extends `timeline-model.md` §2: `retime.rs` grows into a directory, and `anim/` is new.

```text
crates/freecut-timeline/src/
  anim/
    mod.rs        Animatable<T>, KeyframeTrack<T>, Keyframe<T>, KEY_SNAP
    interp.rs     Interpolation, EasePreset, Strength, CubicBezier, Q16, easing math (libm)
    value.rs      Interpolate trait + impls for f64, Vec2, FixedStep<i32>; CL/AU types impl it in their modules
    path.rs       PropPath, PropValue, AnimatableDyn (type-erased access for ops/UI)
    edit.rs       pure helpers: mirror_track, bake_at, paste_plan, conform_times
    ops.rs        request builders for TR-028..033 (produce edit-ops ReplaceClip changes)
  retime/
    mod.rs        Retime, RetimeMode, FrameSampling, public query fns
    speed.rs      Speed (validated Rational), presets, ranges
    map.rs        TimeMap, MapShape, Dir, SourceInstant, canonicalize
    curve.rs      SpeedCurve (authoring), Steffen slopes, CompiledCurve (I256 polys), presets data
    sampling.rs   SourceFrameGrid trait (Cfr, Vfr), plan_frame → FramePlan, FRAME_TOL
    edit.rs       split_at / trim_head / trim_tail / set_mode / set_reverse helpers (SplitAt impl)
    freeze.rs     freeze_settings_at
crates/freecut-render/src/retime/    blend.rs (WGSL pass), flow.rs (FlowProvider trait, fallback)
crates/freecut-audio/src/retime/     plan.rs (BlockPlan), varispeed.rs, stretch.rs (Stretcher trait + impls)
crates/freecut-media/src/            frame_index.rs (PTS→Tick grid), reverse_cache.rs, flow_job.rs (later)
crates/freecut-project/src/          serde shapes + migrations for anim/retime (§10)
crates/freecut-app/src/              inspector diamonds, keyframe lanes, easing & speed graph widgets (f64 drawing only)
```

### 3. Time vocabulary used here

- `Tick`, `Rational`, `Rounding` and `TimeError` are as defined in `timeline-model.md` §3. One
  tick is 1/705 600 000 s. Every frame duration and audio sample period is an integer number of
  ticks.
- **Map time `u`** is output ticks measured from the origin of a clip's time map. Clip-local time
  `t` maps to `u = window_start + t`.
- **`SourceInstant`** is the result of a map query. The exact value is a rational with a large
  denominator, so it is never stored. Callers get exact floor, ceil and comparison results,
  plus a sub-tick fraction used only for DSP and drawing.

```rust
/// An exact point in source time, as returned by the time map.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SourceInstant {
    floor: Tick,        // ⌊s⌋, exact
    on_tick: bool,      // s == floor exactly
    sub_tick: f32,      // (s − floor) ∈ [0,1) — DSP phase / UI only, never compared
}
impl SourceInstant {
    pub fn floor(self) -> Tick { self.floor }
    pub fn ceil(self) -> Tick { if self.on_tick { self.floor } else { self.floor.next() } }
    /// Exact comparisons against integer ticks (PTS, keys, range ends).
    pub fn ge(self, x: Tick) -> bool { self.floor >= x }
    pub fn gt(self, x: Tick) -> bool { self.floor > x || (self.floor == x && !self.on_tick) }
}
```

Fixed rules used by this subsystem, in addition to `timeline-model.md`'s rounding table:

| Quantity | Rule |
|---|---|
| Key time created at playhead | `floor(source_time_at(clip, t_frame))` (media), `t_frame + content_offset` (generated) |
| Key evaluation instant | same function as creation, so the frame where a key was made hits it exactly (invariant 6) |
| `KEY_SNAP` | evaluation instants within ±1 tick of a key count as the key; keys are kept ≥ 3 ticks apart |
| `FRAME_TOL` | frame lookup treats `s` within 2 ticks of a PTS as equal to it (absorbs `NearestTiesEven` PTS rounding, < 1.5 ns, and sub-tick anchors) |
| Speed → T | `max(1 frame, nearest_frame(D / r))`, ties to the shorter (TL-001) |
| Curve knots | `round(u_i · T)` to integer ticks at compile time; the normalization then makes `s(T) = D` exact |

### 4. Keyframes

#### 4.1 Types

```rust
/// A property that may be animated. `static_value` is kept while keys exist so
/// that removing the last key writes back without a jump (TR-027).
#[derive(Clone, Debug, PartialEq)]
pub struct Animatable<T> {
    static_value: T,
    keys: Option<Arc<KeyframeTrack<T>>>,   // Arc: cheap snapshots and ReplaceClip undo
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe<T> {
    pub time: Tick,               // on the owner's key clock (§4.6)
    pub value: T,
    pub interp: Interpolation,    // easing of the segment that STARTS here
}

/// Invariants (enforced by every constructor and mutator, checked by validate()):
/// non-empty; times strictly increasing; consecutive times ≥ 3 ticks apart.
#[derive(Clone, Debug, PartialEq)]
pub struct KeyframeTrack<T> { keys: Vec<Keyframe<T>> }
```

Why one track per property (Position is one `Vec2` track), unlike Olive's per-component tracks:
- The spec's UI has one diamond per property.
- Copy, paste and navigation stay simple.
- X and Y cannot be eased differently, which is acceptable for a CapCut-class editor. If that is
  ever needed, a separate `PositionX/PositionY` property is an additive change.

#### 4.2 Evaluation

```rust
pub const KEY_SNAP: i64 = 1; // ticks

impl<T: Interpolate> KeyframeTrack<T> {
    pub fn evaluate(&self, at: Tick) -> T {
        let k = &self.keys;
        let at = at.raw();
        let i = k.partition_point(|key| key.time.raw() <= at + KEY_SNAP);
        if i == 0 { return k[0].value.clone(); }                    // before first key
        let a = &k[i - 1];
        if (at - a.time.raw()).abs() <= KEY_SNAP || i == k.len() {  // on a key, or after last
            return a.value.clone();
        }
        let b = &k[i];
        // Exact progress (at − a)/(b − a): both are integers, so one division.
        let p = (at - a.time.raw()) as f64 / (b.time.raw() - a.time.raw()) as f64;
        match a.interp.ease(p) {
            Eased::Start => a.value.clone(),
            Eased::End => b.value.clone(),
            Eased::At(e) => T::interpolate(&a.value, &b.value, e),
        }
    }
}
```

**Rules.**
- At key instants the stored value is returned, never a lerp. This matters because
  `a + (b − a)·1` is not always `b` in floating point.
- `p` is computed only strictly inside a segment.
- Clamping (opacity, feather, scale ≥ 1 %) is done by the property owner after evaluation, using
  its `PropertySpec { min, max, step }`. It is never done in the track.
- Evaluation cost is O(log n). Dense keys (one per frame) are fine.

#### 4.3 Interpolation and easing

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Linear,
    Hold,              // value of this key until the next key
    HoldAtStart,       // internal: produced by reversing a Hold (jumps right after this key)
    Ease(EasePreset),
    Bezier(CubicBezier),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EasePreset {
    In(Strength), Out(Strength), InOut(Strength),        // TR-031 "four strengths"
    OvershootIn, OvershootOut, ElasticIn, ElasticOut, BounceIn, BounceOut,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Strength { Sine, Quad, Cubic, Expo }

/// 16.16 fixed point: mirroring (1 − x) is exact, so reverse∘reverse == identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Q16(i32);
/// CSS-style timing curve from (0,0) to (1,1). x ∈ [0,1] (clamped), y ∈ [−1, 2] (TR-032).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CubicBezier { x1: Q16, y1: Q16, x2: Q16, y2: Q16 }

pub enum Eased { Start, End, At(f64) }
```

**Math.** Every preset is written from its mathematical definition, and `e(0) = 0`, `e(1) = 1`
are never reached because of the rule above.
- `In(s)(p)`:
  - Sine: `1 − cos(πp/2)`
  - Quad: `p²`
  - Cubic: `p³`
  - Expo: `2^(10p − 10)`
- `Out(s)(p) = 1 − In(s)(1 − p)`.
- `InOut(s)` joins `In` on the first half with `Out` on the second.
- Overshoot ("back"): `p²((c+1)p − c)` with `c ≈ 1.70158`, which gives a 10 % overshoot.
- Elastic out: an exponentially decaying sinusoid around 1.
- Bounce out: four decaying parabolic arcs.
- Each "in" variant is the point-mirror of its "out" variant.

**Transcendental functions.** `sin`, `cos` and `exp2` come from the `libm` crate, not `f64::sin`.
`std` calls the platform libm, whose last-ulp results differ between macOS, glibc and MSVC, and
that would break cross-OS golden hashes (invariant 7).

**Bézier solve (TR-032).** With x handles clamped to [0,1], x(τ) is monotone, so the root is
unique. The solve uses a fixed 48 bisection steps (τ error 2⁻⁴⁸). It uses only `+ − × /`, so it
is bit-identical everywhere, and it costs about 300 flops. The sketch below was compiled and
tested in a scratch crate (endpoints, the linear curve, and mirror properties):

```rust
impl CubicBezier {
    fn coord(a: f64, b: f64, t: f64) -> f64 {           // B(t) with P0=0, P3=1
        let mt = 1.0 - t;
        3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
    }
    pub fn ease(self, p: f64) -> f64 {
        let (x1, x2) = (self.x1.to_f64(), self.x2.to_f64());
        let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            if Self::coord(x1, x2, mid) < p { lo = mid } else { hi = mid }
        }
        Self::coord(self.y1.to_f64(), self.y2.to_f64(), 0.5 * (lo + hi))
    }
    /// Time-reversal: e'(p) = 1 − e(1 − p). Exact in Q16.
    pub fn mirrored(self) -> Self {
        Self { x1: self.x2.one_minus(), y1: self.y2.one_minus(),
               x2: self.x1.one_minus(), y2: self.y1.one_minus() }
    }
}
```

**`Interpolation::mirrored()`** is an involution. It maps:
- `In(s) ↔ Out(s)`, and `InOut(s) → InOut(s)`;
- `OvershootIn ↔ OvershootOut`, `ElasticIn ↔ ElasticOut`, `BounceIn ↔ BounceOut`;
- `Hold ↔ HoldAtStart`;
- `Linear → Linear`;
- Bézier as shown above.

Floats cannot do this exactly: `1 − (1 − 0.1) = 0.09999999999999998` in `f64`, so Bézier
handles are Q16. The scratch tests confirm `mirrored(mirrored(x)) == x` bit-exactly, and
`|mirrored(e)(p) − (1 − e(1 − p))| < 1e-12` for every preset.

**Hold under reverse.**
- `Hold` on a key means the value is `v_a` on `[t_a, t_b)` and jumps at `t_b`.
- After mirroring, the segment runs from the mirrored `t_b` to the mirrored `t_a`. It must show
  `v_b` exactly at its start and `v_a` everywhere after. That is `HoldAtStart`.
- The UI labels both "Hold".

#### 4.4 Value types

```rust
/// Values that can be keyed. `e` is eased progress; it may leave [0,1] for overshoot presets.
pub trait Interpolate: Clone + PartialEq + Send + Sync + 'static {
    fn interpolate(a: &Self, b: &Self, e: f64) -> Self;
}
impl Interpolate for f64 { fn interpolate(a: &f64, b: &f64, e: f64) -> f64 { a + (b - a) * e } }
impl Interpolate for Vec2 { /* component-wise, same e */ }
/// Fixed-point scalars of other areas: interpolate in f64, round to `STEP` at evaluation only.
pub struct FixedStep<const STEP: i32>(pub i32);
```

Implementations owned by other areas, in their own modules:
- **`LinearRgba` (CL).** Interpolates in linear light, not in 8-bit sRGB as MLT does.
- **`GainDb` (AU-008).** Interpolates in dB. A segment that touches −∞ interpolates in linear
  amplitude.
- **Booleans and enums.** Not animatable in v1. Enum effect parameters (FX-014) step: their
  `interpolate` returns `a`, and evaluation at the key returns `b`.

#### 4.5 Property addressing (ops, UI, paste)

```rust
pub enum PropOwner { Transform, Mask(MaskId), Effect(EffectId), Audio, Color(ColorLayerId), Text }
pub struct PropPath { pub owner: PropOwner, pub key: PropKey }   // PropKey: interned &'static str
pub enum PropValue { Scalar(f64), Vec2(Vec2), Fixed(i32), Color(LinearRgba), Gain(GainDb) }

/// Type-erased access used by ops and the UI. Blanket-implemented for Animatable<T>
/// where T: Interpolate + Into<PropValue> + TryFrom<PropValue>.
pub trait AnimatableDyn {
    fn key_times(&self) -> SmallVec<[Tick; 8]>;
    fn value_at(&self, at: Tick) -> PropValue;
    fn upsert(&mut self, at: Tick, v: PropValue) -> Result<Option<KeySnapshot>, KeyframeError>;
    fn remove(&mut self, at: Tick) -> Result<KeySnapshot, KeyframeError>;
    fn set_interp(&mut self, at: Tick, i: Interpolation) -> Result<Interpolation, KeyframeError>;
    fn move_key(&mut self, from: Tick, to: Tick) -> Result<MoveOutcome, KeyframeError>;
}
/// Implemented by each property bag (TR transform, FX params, AU audio, CL, TX).
pub trait PropBag {
    fn animatable(&self, key: PropKey) -> Option<&dyn AnimatableDyn>;
    fn animatable_mut(&mut self, key: PropKey) -> Option<&mut dyn AnimatableDyn>;
}
```

`KeySnapshot` holds the removed or replaced key (time, value, interpolation). Undo needs it,
and it also covers the first-key and last-key static-value cases (TR-028).

#### 4.6 The key clock per clip kind

```rust
pub fn key_time_at(clip: &Clip, t: Tick) -> Result<Tick, TimeError> {
    match clip.content() {
        ClipContent::Media { .. } | ClipContent::Compound { .. } =>
            Ok(retime::source_time_at(clip, t)?.floor()),
        _ => t.checked_add(clip.content_offset()),   // stills, text, shapes, freeze stills
    }
}
```

**`content_offset`** is a new `Tick` field on non-media clips. It defaults to 0, a head trim of
`Δ` adds `Δ`, and the right half of a split gets `+ t_c`. This is what makes "split leaves output
unchanged" true for keyed text and stickers (C7). The same field gives FX-019 its "phase offset"
for free.

#### 4.7 Keys under edits

| Edit | Media clip (source clock) | Generated or still (clip-local clock) | Owner |
|---|---|---|---|
| Trim head or tail (TL-020) | keys unchanged; out-of-range keys kept and inactive; timeline shows edge markers | head trim: `content_offset += Δ`; keys unchanged | TL calls `retime::edit::trim_*` |
| Split (TL-023) | both halves get the **same** `Arc<KeyframeTrack>`; nothing inserted | right half `content_offset += t_c` | TR's `SplitAt` impl |
| Speed or curve change | keys unchanged; they stretch with the footage because the map changed | n/a (no retime) | — |
| Reverse (TR-043) | `mirror_track(axis = src.start + src.end)`: times `axis − t`, order reversed, interpolations `mirrored()`, the dangling last interpolation carried over (tested) | n/a | `SetReverse` stores nothing extra: mirroring is its own inverse |
| Freeze (TR-044) | still gets values baked at the playhead; no keys | — | `freeze_settings_at` |
| Copy/paste (TR-033) | spacing preserved in **timeline** time: source → `timeline_offset_of(src_clip)` → offset from earliest → target `t` → `key_time_at(target)` | same, with the target's offset | `anim::edit::paste_plan` |
| Replace media (TL-035) | shift all keys by `new.src.start − old.src.start` | n/a | TL op calls `anim::edit::shift_times` |
| Interpret footage rate (PM) | the media's source time is rescaled by `old/new`: keys `floor(t · old/new)`; maps rebuilt | n/a | PM op |
| Conform sequence rate (PM) | unchanged (source time) | snap to the new grid; if two keys collide, keep the later one, with a report | `anim::edit::conform_times` |
| Package project (PM) | shift by the same offset applied to `source` | n/a | PM op |
| Detach audio (TL-030) | the audio clip gets a clone of the `Retime` and its volume keys (`Arc` share) | — | TL op |

**Hold and Elastic under reverse** need the enum additions in C6. Continuous segments evaluate
equal to within `1e-12` relative after reversal, not bit-identical: `a + (b−a)e` and
`b + (a−b)(1−e)` differ by 1 ulp, which the scratch test showed. Key instants and Hold jumps
are exact. Golden tests must therefore not hash reversed frames against forward frames.

#### 4.8 Ops (undo)

Keyframe edits are request builders that produce the generic edit op `ReplaceClip` from
`edit-ops-and-undo.md`, swapping one `Arc` property bag. Each has a reversal test:

| Op (spec) | Stored for undo | Notes |
|---|---|---|
| `AddKeyframe` / `RemoveKeyframe` (TR-028) | the `KeySnapshot` and, for the first or last key, the static value | the diamond on a multi-selection adds one `Tx` with N changes |
| `SetKeyframeValue` (TR-028) | old and new value | creates a key when auto-key is on |
| `MoveKeyframes` (TR-029) | `Vec<(PropPath, old, new)>` plus replaced keys | a key cannot jump its neighbour; dropping onto a key replaces it; the drag happens in timeline time and is converted through the map |
| `SetInterpolation` (TR-031/032) | `Vec<(PropPath, time, old, new)>` | graph drags coalesce per gesture |
| `PasteKeyframes` (TR-033) | added plus replaced | keys whose property is missing on the target are skipped and listed |
| `RemoveAllKeyframes` | the whole old `Arc<KeyframeTrack>` | cheap thanks to `Arc` |

### 5. Retiming

#### 5.1 Model

```rust
/// On ClipContent::Media and ::Compound (timeline-model.md §5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Retime {
    mode: RetimeMode,                 // what the user authored (serialized)
    reverse: bool,                    // serialized
    pub frame_sampling: FrameSampling,// serialized
    map: TimeMap,                     // exact map + window (anchor/window serialized, shape rebuilt)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetimeMode { Normal(Speed), Curve(Arc<SpeedCurve>) }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrameSampling { Nearest, Blend, OpticalFlow }

/// Validated exact speed: 1/10 ≤ r ≤ 100 for Normal (TR-037).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Speed(Rational);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir { Forward, Reverse }

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimeMap {
    shape: MapShape,
    dir: Dir,
    window_start: Tick,   // map time of the clip's first output instant
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum MapShape {
    /// s(u) = (anchor_num ± r.num·u) / r.den   — exact in i128
    Linear { speed: Speed, anchor_num: i128 },
    /// s(u) = anchor ± offset(u); offset is the compiled curve, exact
    Curve { anchor: Tick, compiled: Arc<CompiledCurve> },
}
```

The nominal `source: SourceRange` (from `timeline-model.md`) stays the authority for **limits
and speed edits**. The map is the authority for **rendering**. The invariant, checked by
`validate()`, is:
- forward: `floor(s(0)) == source.start`;
- reverse: `ceil(s(0)) == source.end`;
- `s(T)` is within `[source.end − half frame × r_end, source.end]` for Normal, and exactly
  `source.end` (forward) or `source.start` (reverse) for a curve that has not been trimmed since
  it was compiled.

#### 5.2 Constant speed, exactly

- `s(u) = A ± r·u`. The anchor `A` is stored as a numerator over `r.den`.
- At creation `A` is an integer tick: `anchor_num = source.start · r.den`.
- After splits, `A` stays a multiple of `1/r.den`, so every query is one `i128`
  multiply-add plus one Euclidean division. The magnitudes are about 10¹³ × 10¹³ = 10²⁶, well
  inside the `i128` maximum of 1.7·10³⁸.

```rust
impl TimeMap {
    pub fn source_at(&self, t: Tick) -> Result<SourceInstant, TimeError> {
        let u = i128::from(self.window_start.checked_add(t)?.raw());
        match &self.shape {
            MapShape::Linear { speed, anchor_num } => {
                let off = i128::from(speed.num()).checked_mul(u).ok_or(TimeError::Overflow)?;
                let num = match self.dir {
                    Dir::Forward => anchor_num.checked_add(off),
                    Dir::Reverse => anchor_num.checked_sub(off),
                }.ok_or(TimeError::Overflow)?;
                SourceInstant::from_ratio(num, i128::from(speed.den()))
            }
            MapShape::Curve { anchor, compiled } => {
                let off = compiled.offset_at(u)?;            // exact (I256), see §5.6
                SourceInstant::from_anchor_offset(*anchor, off, self.dir)
            }
        }
    }
}
```

#### 5.3 Reverse

- The direction is part of the map, so a reversed map is an ordinary strictly decreasing map,
  and windows (§5.4) work unchanged.
- **Toggling** (`SetReverse`) rebuilds the map in canonical form:
  - `window_start = 0`;
  - anchor at `source.end` when switching to reverse, or at `source.start` when switching to
    forward;
  - same shape;
  - keys mirrored about `source.start + source.end`.
- The nominal `source` range never changes when toggling, so the mirror axis is the same both
  ways. Mirroring twice is exactly the identity (invariant 3).
- A split half's anchor may sit at a sub-tick position, so canonicalizing can move the map by
  less than one tick. `KEY_SNAP` and `FRAME_TOL` absorb that.
- The speed shape stays in timeline order, as TR-043 requires: for a curve,
  `s_rev(t) = S_end − offset(t)`.

#### 5.4 Windows: trim and split without touching the map

- **Split at `t_c`:**
  - the left half keeps the map and gets `T = t_c`;
  - the right half gets an identical map with `window_start + t_c`;
  - the shared shape is `Arc`-cloned;
  - both halves then evaluate *the same function at the same map time*, so every frame and
    sample is identical by construction (invariant 1).
- **Head trim by `Δ`:** `window_start += Δ` and `T −= Δ`. A negative `Δ` extends the clip:
  `u` may go below 0, where curves continue at the edge speed (TR-036).
- **Tail trim:** only `T` changes.
- After either trim, the nominal `source` is recomputed from the map: `floor(s(0))` and
  `ceil(s(T))`, swapped for reverse. Limits are checked against `available`.

This replaces TR-036's "Hermite subdivision and re-normalization" (C4) and needs no new
`ReplaceClip` fields. `edit-ops-and-undo.md`'s trim and split code already delegates to TR's
helpers. The helpers are:

```rust
impl Retime {
    pub fn split_at(&self, t_c: Tick) -> Result<(Retime, Retime), RetimeError>;
    pub fn trim_head(&self, delta: Tick) -> Result<Retime, RetimeError>;
    pub fn trim_tail(&self, new_len: Tick) -> Result<Retime, RetimeError>; // validates source limits
}
```

#### 5.5 Speed edits and duration (no drift)

`SetRetimeMode(Normal(r'))`:
1. `D = source.duration` (nominal, unchanged).
2. `T' = max(frame, nearest_frame(D / r'))`.
3. Build a canonical map: `window_start = 0`, anchor `source.start` (forward) or `source.end`
   (reverse).
4. The ripple change set comes from edit-ops (TR-037).

`D` is never derived from a previous `T`, so 1× → 0.3× → 1× restores `T` exactly. This is the
Kdenlive and Shotcut drift (P2), fixed by construction. A typed duration (TR-037) gives
`r' = D / T_target` exactly, as a `Rational` clamped to [0.1, 100]. Both values fit in `i64`.

#### 5.6 Speed curves

**Authoring form** (what the editor and the project file store):

```rust
pub struct SpeedCurve {
    pub points: Vec<SpeedPoint>,     // 2..=32; u strictly increasing; u[0]=0, u[last]=1; gap ≥ 1/100
    pub shape: CurveShape,           // Smooth (monotone cubic Hermite) | Linear
}
pub struct SpeedPoint {
    pub u: Rational,     // normalized output progress, on a 1/10_000 grid
    pub v: Rational,     // speed 0.1..=10, on a 1/1_000 grid
    pub slope: Rational, // dv/du, computed (never typed), rounded toward 0 to 1/10_000
}
```

**Slopes.**
- Slopes are computed by **Steffen's monotone method**. It uses only `+ − × ÷`, `min`, `abs`
  and `sign`, so the result is exactly rational. Fritsch–Carlson needs a square root.
- Each Hermite segment then stays between its two point speeds, so `v ≥ 0.1` everywhere and the
  map is strictly monotone (invariant 2, P15).
- Rounding toward zero preserves that bound.
- A scratch property test checked "no overshoot" over random data with exact rationals.

**Compilation** (`CompiledCurve::compile(curve, D, frame)`):
- It runs once per curve commit, on the UI thread: it is O(points) BigRational work, under a
  millisecond for 32 points.
- The result is shared through `Arc`.

Steps:
1. `mean = Σ hᵢ·((vᵢ + vᵢ₊₁)/2 + hᵢ(mᵢ − mᵢ₊₁)/12)` exactly. The Linear shape drops the slope
   term.
2. `T = max(frame, nearest_frame(D / mean))`.
3. Knots `τᵢ = round(uᵢ·T)` in integer ticks. Merge collisions; they only happen on clips a few
   frames long.
4. For each segment `i`, the offset over `k = u − τᵢ` ticks is a quartic
   `(C₀ + C₁k + C₂k² + C₃k³ + C₄k⁴) / Q`, where `C₀` carries the prefix sum. It is computed
   exactly with `num-rational::BigRational`, scaled by `D / I_total` so that
   **`offset(T) = D` exactly**, and converted to integer coefficients over a common
   denominator `Q`.
5. Store `Cⱼ` and `Q` as `ethnum::I256`. Add two linear edge segments (`u < 0` and `u > T`) at
   the scaled edge speeds.

**Evaluation** (render, audio, keys):
- `partition_point` over at most 34 knots.
- Horner's rule in `I256` with **checked** ops, then one floor division.
- If anything overflows, fall back to the same formula in `num-bigint::BigInt`, then return
  `RetimeError::CurveOverflow` if it still cannot be represented.
- No allocation on the fast path.

Measured on this machine (scratch benchmarks, release build):

| Evaluation strategy | Cost per evaluation |
|---|---|
| naive `BigRational` (gcd on every op) | 46–146 µs, too slow for per-frame use |
| integer numerator, `BigInt`, no gcd | 0.62 µs |
| integer numerator, `I256` | **0.11 µs** |

| Clip length (32 random points, 59.94 fps) | Largest compiled coefficient | Matches BigRational oracle (20 000 random instants) | `offset(T) == D` |
|---|---|---|---|
| 36 s | 180 bits | yes | yes |
| 1 h | 208 bits | yes | yes |
| 4 h | 222 bits | yes | yes |
| 24 h | 235 bits (I256 holds 255; Horner intermediates are checked) | yes | yes |

**Effective speed.** The normalization scales the authored speeds by
`k = D / (T · mean)`. Since `|T − D/mean| ≤ ½ frame`, `|k − 1| ≤ ½ frame / T`. That is under 1 %
on clips of 50 frames or more, and under 0.1 % on clips of 500 frames or more (about 17 s at
30 fps). The UI shows the authored values.

**Presets (TR-039).** Presets are plain point lists in `curve.rs` (Accelerate, Decelerate,
Slow center, Fast center, Pulse, Double dip). Thumbnails are drawn from the same data, with
`f64` used for drawing only.

**Editing a curve on a trimmed clip.** The editor shows the window's part of the curve.
Committing re-authors the curve over the current window (an explicit user edit, so a fresh
compile from the nominal `source` is allowed) and resets `window_start` to 0.

#### 5.7 Public query functions (the only mapping implementation, TR-036/TR-038)

```rust
/// Exact source instant at clip-local t. t may lie outside [0, T) (transition handles).
pub fn source_time_at(clip: &Clip, t: Tick) -> Result<SourceInstant, TimeError>;
/// Source interval covered by output [t0, t1) — for blending and audio blocks.
pub fn source_span(clip: &Clip, t0: Tick, t1: Tick) -> Result<(SourceInstant, SourceInstant), TimeError>;
pub fn timeline_len(clip: &Clip) -> Tick;                   // spec name: timeline_duration
/// Instantaneous |ds/dt|, for display, badges and quality decisions only — never for positions.
pub fn local_rate(clip: &Clip, t: Tick) -> Rational;
/// Inverse: the clip-local time (floored to the tick) at which source instant s is shown.
/// Exact division for Normal; binary search over ticks with the forward map for curves.
pub fn timeline_offset_of(clip: &Clip, s: Tick) -> Option<Tick>;
pub fn key_time_at(clip: &Clip, t: Tick) -> Result<Tick, TimeError>;   // §4.6
```

Keyframe navigation (TR-030) and timeline drawing of keys use `timeline_offset_of`. "Next
keyframe" finds the timeline frame whose interval contains the key's instant with a binary
search over frames using the forward map (TR-030). It never needs a symbolic inverse of a curve.

#### 5.8 Frame selection (fixes C5)

```rust
pub const FRAME_TOL: i64 = 2; // ticks

/// PTS starts of the source's frames in ticks; CFR is arithmetic, VFR is an index.
/// Object-safe (freeze_settings_at takes `&dyn SourceFrameGrid`).
pub trait SourceFrameGrid: Send + Sync {
    fn len(&self) -> usize;
    fn start(&self, i: usize) -> Tick;
    /// Number of frames whose start satisfies a monotone predicate (true…true false…false).
    /// The default is a binary search; `Cfr` overrides it with arithmetic.
    fn count_starting(&self, pred: &dyn Fn(Tick) -> bool) -> usize {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi { let mid = (lo + hi) / 2; if pred(self.start(mid)) { lo = mid + 1 } else { hi = mid } }
        lo
    }
}
/// Forward: the frame whose [start, next) contains s.
/// Reverse: the frame whose (start, next] contains s — "the last frame starting strictly before s".
pub fn frame_at(grid: &dyn SourceFrameGrid, s: SourceInstant, dir: Dir) -> usize {
    let n = match dir {
        Dir::Forward => grid.count_starting(&|p| p.raw() <= s.floor().raw() + FRAME_TOL),
        Dir::Reverse => grid.count_starting(&|p| p.raw() + FRAME_TOL < s.ceil().raw()),
    };
    n.saturating_sub(1)
}
```

**Why this rule.**
- With exact arithmetic, a reversed 1× clip shows frames N−1, N−2, …, 0. A reversed clip at any
  speed starts on the last frame before `out`.
- For curves, `out − (t + frame)·r` is not even defined, but this rule still is.
- The scratch tests check both directions at 1×.
- The result is clamped to the frames that intersect the nominal source range. That covers the
  half-frame overrun from rounding `T`.

**Grids** come from `freecut-media`. For CFR streams the grid is arithmetic and uses
`ticks_from_stream` from `timeline-model.md` §3.5. For VFR streams it is a PTS index built at
probe time. Either way the grid reaches the timeline crate as plain data.

#### 5.9 Freeze frame (TR-044)

```rust
pub fn freeze_settings_at(clip: &Clip, playhead_t: Tick, grid: Option<&dyn SourceFrameGrid>)
    -> Result<(Tick, FrozenSettings), RetimeError>;
```

- `s₀` is the **start of the frame `plan_frame` would display**, using the clip's sampling;
  for Blend, it is the frame with the largest weight. The still therefore shows exactly the
  picture the user saw, even on reversed clips, where a naive `floor(s)` would pick the next
  frame.
- Without a grid (the index is not built yet): forward uses `floor(s)`, reverse uses
  `ceil(s) − 1 − FRAME_TOL`.
- Animated settings are baked with `Animatable::value_at(key_time_at(clip, t))`.

#### 5.10 Retime ops

These are request builders over edit-ops' `ReplaceClip`, which carries the ripple changes:

| Op | Changes |
|---|---|
| `SetRetimeMode { old, new, ripple }` | rebuilds the map; replaces `T` and the downstream ripple |
| `SetReverse { old, new }` | rebuilds the map, mirrors the keys, keeps `T` |
| `SetFrameSampling { old, new }` | sampling field only |

Validation errors:
- `TooShort`: the result would be under 1 frame.
- `SpeedOutOfRange`.
- `CurveTooManyPoints`, `PointsTooClose`.
- `NotRetimable`: stills, text and generated clips (TR-036).
- `Locked`: comes from TL.

### 6. Frame sampling in `freecut-render` (TR-041/042)

```rust
pub enum FramePlan {
    Single(FrameIdx),
    Blend(SmallVec<[(FrameIdx, f32); 16]>),        // weights sum to 1
    Synthesize { a: FrameIdx, b: FrameIdx, phase: f32 },   // optical flow, later
}
/// Pure (freecut-timeline::retime::sampling); called by the render scheduler per output frame.
pub fn plan_frame(clip: &Clip, t: Tick, frame: Tick, grid: &dyn SourceFrameGrid)
    -> Result<FramePlan, TimeError>;
```

**Nearest.** `Single(frame_at(s(t)))`.

**Blend.** The plan is built from `source_span(t, t + frame)`:
- **Speed-up** (the span covers two or more frame starts): each frame is weighted by the
  overlap between its display interval and the span, computed in integer ticks and normalized
  to `f32`.
  - If more than 16 frames qualify, take 16 evenly spaced frame indices with equal weights.
  - The selection is deterministic, which caps decode work at 100×.
- **Slow motion** (the span contains at most one frame start): take the span midpoint
  `s_mid`, find the two frames whose **centers** bracket it (`center_j ≤ s_mid < center_{j+1}`),
  and cross-fade them with `α = (s_mid − center_j)/(center_{j+1} − center_j)`. Outside the first
  and last centers, use a single frame.
  - At 1× with matching rates the span is exactly one source frame, so `s_mid` equals that
    frame's center, `α = 0`, and the setting is a no-op, as TR-041 requires.
  - All terms are integer ticks except the final `f32` weight.
- **Rendering.** The WGSL pass accumulates into `rgba16float` in **linear light with
  premultiplied alpha** after the decode/color-convert stage. Averaging encoded values, as MLT
  does, darkens fast motion.

**Optical flow** (later):
- `Synthesize` asks a `FlowProvider` (a render-side trait) for the in-between image.
- On a cache miss the scheduler falls back to the Blend plan for preview and enqueues a job:
  first `freecut-media` with FFmpeg's motion-compensated interpolation, later a wgpu flow
  shader.
- Export waits for the cache (TR-042).
- The cache key is `(asset, a, b, phase quantized to 1/256, flow algo version)`.

**Decode for reverse.** `freecut-media` builds the reverse-friendly GOP cache around the
playhead. Export decodes backwards GOP by GOP and never depends on the cache (TR-043). The
render scheduler requests frames in the plan's order, and decoding is never done on the UI
thread.

### 7. Audio in `freecut-audio` (TR-038, AU-023, AU-024)

#### 7.1 Block planning (exact, drift-free)

- The project sample rate `sr` divides `TICKS_PER_SECOND`, so `K = 705 600 000 / sr` ticks per
  sample is an integer (14 700 at 48 kHz).
- The mixer worker processes blocks of `B` samples (for example 256), **aligned to absolute
  sequence sample indices**, not clip-relative ones.
- For each clip active in block `[n₀, n₀+B)`:

```rust
pub struct BlockPlan {
    pub dir: Dir,
    pub src_first: i64,      // ⌊s(t₀)·sr_src/TPS⌋ (forward) — first source sample index
    pub src_len: usize,      // ⌊s(t₁)…⌋ − ⌊s(t₀)…⌋ : telescopes exactly → zero drift
    pub pos0: f64, pub pos1: f64, // fractional source positions at block edges (DSP only)
    pub silent: bool,        // average rate < 0.05× (AU-023) or outside the nominal source range
}
```

- `t₀ = n₀·K − clip.start` and `t₁ = t₀ + B·K`.
- `s(t₁)` may be evaluated past the clip's end, because the map extends beyond the window.
  That is what makes a split invisible to audio.
- Source positions use the source's own `K_src`.

#### 7.2 Varispeed (*Keep pitch* off): stateless and bit-exact

- Output sample `n` reads the source at position `p_n`, interpolated linearly between `pos0` and
  `pos1` within the block. The curvature error over a 5 ms block is negligible.
- The read is a band-limited interpolation:
  - a Kaiser-windowed sinc from a precomputed table (deterministic, no runtime `sin`);
  - a cutoff of `min(1, 1/rate)` for anti-aliasing when speeding up;
  - a tap count that widens with the rate, capped.
- **No state.** Each output sample depends only on `p_n` and the media samples. Seeks, parallel
  export chunks and splits all give identical bits.
- Taps may read media samples outside the clip's source range (real handles). Output positions
  past the nominal `out` produce silence (TR-036).
- Pitch moves by `12·log₂(rate)` semitones automatically.
- Reverse needs no special case: the positions simply decrease.

#### 7.3 *Keep pitch* on: one persistent stretcher per clip stream

```rust
pub trait Stretcher: Send {
    fn reset(&mut self);
    fn input_latency(&self) -> usize;
    fn output_latency(&self) -> usize;
    /// Pre-roll with `history` (source samples just before the start, in playback order).
    fn preroll(&mut self, history: &[f32], rate: f64);
    /// Consume exactly `input` frames, produce exactly `output.len()` frames.
    fn process(&mut self, input: &[f32], output: &mut [f32]);
    fn set_transpose(&mut self, semitones: f32, preserve_formants: bool); // AU-024, AU-022 pitch part
}
```

**Per block.**
- Feed exactly `src_len` source frames. They are read backwards for reverse; reversing the input
  sequence is all reverse requires.
- Ask for exactly `B` output frames.
- The ratio `src_len / B` follows the curve block by block (AU-023's "continuously changing
  ratios").
- Because `src_len` telescopes, nothing drifts, there is no continuity heuristic like MLT's, and
  there is no starvation re-feed.

**On start, seek or edit.**
- Reset the stretcher.
- Pre-roll with `input_latency + 100 ms` of source before `s(t₀)`.
- Start the input `output_latency × rate` samples early and discard the first
  `output_latency` output samples, so that output sample 0 corresponds to `s(t₀)`.
- An impulse-alignment test pins this to ±1 ms.
- Export uses the same rule from the export range start, so export is deterministic. Preview
  matches export only after the pre-roll; tests use a tolerance.

**Where the stretcher lives.**
- In the mixer worker thread, keyed by `(clip id, stream epoch)`. The epoch bumps on any edit
  that changes the clip's map or audio settings.
- The cpal callback only reads the mixer's ring buffer. Map evaluation never runs in the
  real-time callback, and the BigInt fallback can never allocate there.

**Extreme rates.**
- An average block rate below 0.05× is silent, with a 5 ms fade at each transition (AU-023).
- Above the engine's comfortable range (about 4×), quality degrades but output continues.
- Freeze stills and Hold segments are silent.

**Engines.**
- Default: `signalsmith-stretch`.
- A pure-Rust WSOLA fallback that we write.
- Rubber Band optional (§Rust ecosystem).
- The choice is a feature flag behind the trait.

### 8. Ownership and threading

**Model types.**
- `Animatable`, `KeyframeTrack`, `Retime`, `TimeMap` and `CompiledCurve` are plain immutable
  data: `Clone + PartialEq + Send + Sync`, no interior mutability, no caches.
- Large parts sit behind `Arc`, so `ReplaceClip` undo and the render/audio snapshots
  (`timeline-model.md` §14) share them.

**What runs where.**

| Work | Thread | Cost |
|---|---|---|
| Edit ops, including curve compilation | UI thread | microseconds to under a millisecond |
| Inverse tables for drawing keys on long curve clips (`timeline_offset_of` per visible frame) | UI thread, f64 approximation (UI edge) | — |
| Exact inverses | only for commits | — |
| Render planning (`plan_frame`) | render scheduler thread | — |
| Audio block planning and stretching | mixer worker thread | — |
| Reverse caches and optical flow | background jobs in `freecut-media`; never block the UI | — |

**Caches** live in the consumers, keyed by `(clip id, retime hash, frame)`. For example, the
render cache key from `preview-and-export.md` includes the keyframe values evaluated at `t`.
Determinism (invariant 7) is what makes those keys stable across sessions and OSes.

### 9. Errors (`thiserror`, library crates)

```rust
#[derive(Debug, thiserror::Error)]
pub enum RetimeError {
    #[error("speed {0} is outside the allowed range")]       SpeedOutOfRange(Speed),
    #[error("the result would be shorter than one frame")]   TooShort,
    #[error("a speed curve needs 2 to 32 points, got {0}")]  CurveTooManyPoints(usize),
    #[error("speed curve points are closer than 1% of the clip")] PointsTooClose,
    #[error("this clip has no source time (still, text or generated)")] NotRetimable,
    #[error("the source range would exceed the media")]     SourceExceeded,
    #[error("speed curve too long to evaluate exactly")]     CurveOverflow,
    #[error(transparent)] Time(#[from] TimeError),
}
#[derive(Debug, thiserror::Error)]
pub enum KeyframeError {
    #[error("no animatable property {0:?}")]                 UnknownProperty(PropPath),
    #[error("no keyframe at {0:?}")]                          NoKeyAt(Tick),
    #[error("move the playhead over the clip")]              PlayheadOutsideClip,  // TR-028 hint
    #[error("value type does not match the property")]      ValueType,
    #[error(transparent)] Time(#[from] TimeError),
}
```

There is no `unwrap()` or `expect()` outside tests. The one structural invariant ("a track is
non-empty") is enforced by construction, and the indexing in `evaluate` has a
`// INVARIANT:` comment pointing at the constructor. All tick arithmetic is checked
(`TimeError::Overflow`, TL-001).

### 10. Project format (`freecut-project`)

```json
"retime": {
  "mode": { "normal": [137, 100] },
  "reverse": false,
  "sampling": "nearest",
  "map": { "anchor": [1234500, 100], "window_start": 0 }
},
"retime_curve_example": {
  "mode": { "curve": { "shape": "smooth",
            "points": [ { "u": [0,1], "v": [500,1000], "slope": [0,1] }, { "u": [1,1], "v": [4,1], "slope": [0,1] } ],
            "compiled_for": { "len": 352800000, "source_len": 705600000 } } },
  "reverse": false, "sampling": "blend",
  "map": { "anchor": 0, "window_start": 0 }
},
"opacity": { "static": 1.0,
  "keys": [ { "t": 1176000000, "v": 0.0, "i": "linear" },
            { "t": 1411200000, "v": 1.0, "i": { "bezier": [16384, 0, 49152, 65536] } } ] }
```

- Rationals are written as `[num, den]` integer pairs, ticks as integers, and Bézier handles as
  Q16 integers.
- There are **no** locale strings (P14) and no floats for time. `f64` values are serialized with
  serde_json's round-trip-exact formatting.
- Keys are written sorted, and the loader validates sorting, spacing and ranges, refusing
  invalid files with a clear error.
- `CompiledCurve` is **not** stored. It is rebuilt on load from `(points, compiled_for)`, and
  compilation is deterministic integer math, so the rebuild is identical.
- `content_offset` is stored on non-media clips.
- Schema changes go through `project-format.md`'s versioned migrations, for example adding
  `HoldAtStart` and the new presets.

### 11. Interchange notes

| FreeCut | OTIO export |
|---|---|
| Normal speed | `LinearTimeWarp(time_scalar = r)` with `source_range.duration = T` (OTIO's convention: the warp does not change the item's duration) |
| Reverse | `-r`; confirm the convention per target |
| Curves | not representable: export a baked `LinearTimeWarp` per segment (split clips), or keep them in FreeCut metadata |
| Keys | not in the OTIO core schema; metadata only |

The FCPXML and EDL adapters in `project-format.md` follow the same rule.

### 12. How this meets the project conventions

- **Time is never a float in the model.**
  - Ticks and `Rational` everywhere.
  - Curve maps use I256 or BigInt integers.
  - Floats appear only in value interpolation (non-time quantities), DSP and UI drawing.
- **No I/O, FFmpeg or GPU in `freecut-timeline`.** Frame grids arrive as plain data, and blend
  and flow live in render.
- **Nothing heavy on the UI thread.** Map evaluation is pure and cheap. Reverse caches, flow and
  stretching run on workers.
- **Every edit goes through undo**, with a reversal test per op (§4.8, §5.10).
- **Errors:** `thiserror` in the libraries; `anyhow` stays in `freecut-app`.
- **No `unsafe` here.** `signalsmith-stretch` contains its own FFI internally. Our audio crate
  adds none, so the "unsafe only in media/render" rule holds.

---

## Rust ecosystem

| Crate | Version | License | Use |
|---|---|---|---|
| `num-rational` | 0.4.2 (default `num-bigint` feature → `num-bigint` 0.4.x) | MIT OR Apache-2.0 | `BigRational` for curve **compilation** and the test oracle only. Do not add `num-bigint` 0.5 directly: its types would not match `num-rational` 0.4's. |
| `num-integer` | 0.1.47 | MIT OR Apache-2.0 | `lcm` and `div_floor` while compiling |
| `ethnum` | 1.5.3 | MIT OR Apache-2.0 | `I256` for exact curve evaluation; fixed size, no allocation, 0.11 µs/eval measured |
| `libm` | 0.2.16 | MIT | `sin`, `cos` and `exp2` for easing, bit-identical on all OSes (invariant 7) |
| `smallvec` | 1.16.3 (stable; 2.0 is beta) | MIT OR Apache-2.0 | blend tap lists, key-time lists |
| `signalsmith-stretch` | 0.1.3 | MIT (wraps the MIT C++ library; needs a C++ compiler) | Keep-pitch engine. It has `seek` pre-roll, input and output latency, variable ratio per call, transpose and formant factor (AU-024). |
| `rubato` | 5.0.1 | MIT OR Apache-2.0 | Project-rate resampling (AU chain step 6), not varispeed. 5.0.1 is 10 days old; 4.x is the fallback if it regresses. |
| `proptest` | 1.x (1.12.0) | MIT OR Apache-2.0 | property tests |
| `insta` | 1.x (1.49.0) | Apache-2.0 | golden snapshots (frame index tables, easing tables, curve maps) |
| `approx` | 0.5.1 | Apache-2.0 | float tolerances in value tests |

**Considered and rejected (or optional):**

| Crate or tool | Why not |
|---|---|
| `rubberband` 0.1.5 / `rubberband-sys` 0.1.4 (GPL-2.0-or-later, compatible) | Optional engine only. It needs the system librubberband, the crate is barely used, and MLT documents crashes at extreme ratios. |
| `bungee-rs` 0.1.1 (MPL-2.0, compatible) | Young. Worth benchmarking against signalsmith before v1. |
| `timestretch` 0.15.0, `wsola` 0.1.0 (MIT) | EDM-oriented or tiny. Our own WSOLA fallback is simpler to keep deterministic. |
| `keyframe` 1.1.1, `splines` 5.0.0, `interpolation`/easing crates | Time is `f32`/`f64` keys, no rational clock, no exact mirroring. The easing math is about 60 lines. |
| `kurbo` for the Bézier solve | Correct but heavy for one 15-line function. We want a fixed-iteration solve anyway. |
| `rug` (GMP, LGPL) | C dependency and dynamic-linking constraints; `ethnum` and `num-bigint` suffice. |
| FFmpeg `atempo` through `ffmpeg-next` for keep-pitch | A stateful filter graph per clip. No smooth variable ratio for curves. This is the per-block clicks Olive's FIXME describes. |
| `rubato` for varispeed | A stateful resampler breaks split and seek bit-exactness. A stateless table sinc fixes that. |

---

## Test strategy

### Unit tests (`freecut-timeline`, no I/O)

**Keyframe evaluation.**
- Empty → static. One key → constant. Before the first and after the last key.
- At a key, the result is bit-exact (`assert_eq!` on `f64`), including ±1 tick.
- Hold and HoldAtStart.
- Overshoot values pass through unclamped; owners clamp.
- First and last key add/remove round-trips the static value.

**Easing.**
- A 101-point table per preset, as an `insta` snapshot of `to_bits()` hex, so the CI matrix
  catches libm drift.
- `In`/`Out`/`InOut` are monotone; Overshoot, Elastic and Bounce leave [0,1] as designed.
- `mirrored(mirrored(x)) == x` exactly for every variant.
- `|mirrored(e)(p) − (1 − e(1 − p))| < 1e-12`.

**Bézier.**
- (0,0,1,1) is linear within 1e-12.
- Agreement with a 200-step bisection reference within 1e-13.
- x handles outside [0,1] are clamped.
- 10⁶ random handles all finish in 48 steps (no hangs, P6).

**Linear map.**
- 1×, 2×, 0.1×, 100× and 137/100 against hand-computed tables.
- Reverse 1× shows frames N−1…0.
- `floor(s(0)) == source.start`.
- 24-hour clips at 0.1× and 100× do not overflow (checked arithmetic returns `Err`, never
  panics).

**Curve compile.**
- `offset(T) == D` exactly.
- Matches the BigRational oracle at 20 000 random instants per curve, for 36 s, 1 h, 4 h and
  24 h clips.
- Linear shape matches its quadratic closed form.
- Edge extrapolation runs at the edge speed.
- Presets compile on 0.5 s and 10 min clips.

**Steffen slopes.** Never overshoot (exact rational sampling). Rounding toward zero keeps the
bound.

**Speed edit math.** 1× → 0.3× → 1× restores `T`. A typed duration gives `r = D/T` exactly and
clamps.

**Ops.** Each op applied then inverted is the identity (`PartialEq` on `Clip`), including the
first/last-key static-value cases and paste with replaced keys.

### Property tests (`proptest`)

The two marked "already in scratch" passed in the scratch crate.

- **Split invariance** (invariant 1, already in scratch for Linear):
  - Inputs: random speed (or curve), direction, source start, window and cut.
  - Assertion: `source_time_at` of the whole clip at `t_c + t` equals the right half at `t`,
    for every `t`.
  - The same for `plan_frame`, keyframe values (including generated clips with
    `content_offset`), and varispeed block output bits.
- **Monotonicity** (invariant 2): random valid curves give a strictly increasing map (or strictly
  decreasing when reversed), and `T ≥ 1 frame`.
- **Reverse round trip** (invariant 3, already in scratch for keys):
  - mirroring twice gives identical tracks;
  - `SetReverse` twice gives identical keys and a `Retime` equal after `canonicalize()`;
  - evaluated values match within 1e-12, and Hold jumps happen at the same timeline instants.
- **Key-instant exactness** (invariant 6): for random speeds and splits, a key created at frame
  `t` evaluates to its stored value at `t`.
- **Audio drift** (invariant 5): over random block sequences, the sum of `src_len` equals
  `⌊s(end)⌋ − ⌊s(start)⌋` in source samples.

### Golden tests

| Golden | Snapshot | How it is produced |
|---|---|---|
| **Frame-index tables** (`insta` YAML) | source frame index for the first 240 output frames | every speed in {0.1, 0.25, 0.5, 1, 1.37, 2, 4, 10, 100} × source/sequence rates {23.976, 24, 25, 29.97, 30, 50, 59.94, 60} × direction × sampling (Nearest, plus Blend tap lists) |
| **Speed-curve maps** | per-frame `floor(s)` | each preset on a 10 s clip at 30 and 59.94 fps |
| **Rendered frames** (`freecut-render`, GPU tests where available) | hash of a few frames | 2× blend, 0.25× cross-fade, reverse 1×, from a frame-counter fixture |

Rendered-frame hashes compare only forward against forward. Reversed frames are not hashed
against forward frames, because continuous key values may differ by 1 ulp (§4.7).

### Integration tests (`freecut-media`, `freecut-audio`)

**Frame counter fixture.**
- A synthetic video whose frame `i` encodes `i` in a block of pixels, generated at test time by
  `freecut-media`'s encoder:
  - 30 fps CFR with a 1/15360 time base, so PTS rounding is exercised;
  - VFR with irregular PTS;
  - 59.94 fps.
- Asserts that the decoded index equals `plan_frame`'s choice for every frame, at the speeds
  above, including reverse.

**Impulse train audio fixture.**
- Generated in Rust, for example clicks every 100 ms at 48 kHz and 44.1 kHz.
- Varispeed: each click appears at its mapped output sample ±1, and split versus unsplit output
  is bit-identical.
- Keep-pitch: each click appears within ±1 ms of its mapped time. Split versus unsplit
  difference is below −40 dBFS RMS after 100 ms.

**Sine fixture.**
- Keep-pitch at 0.5× and 2× keeps the FFT peak within 1 %.
- Varispeed shifts the pitch by `rate` within 1 %.
- AU-024: +12 semitones doubles the frequency.

**Curve A/V sync.** On a curve clip, the click positions in audio match the frame-counter
indices in video within one audio block (TR-038).

### Fixtures and CI

- No binary media is committed. Every fixture is generated by test code: the frame-counter
  video through `freecut-media`, audio in Rust.
- Easing and map goldens run on macOS, Windows and Linux CI. Any difference fails the build.
- The scratch benchmarks for curve evaluation are turned into a `criterion`-style bench
  (optional), so a regression past 1 µs per evaluation is visible.

---

## Risks & open questions

1. **Spec sign-off on C1–C8**:
   - split boundary keys;
   - T/D derivation;
   - the `i128` claim;
   - windows instead of subdivision;
   - reverse frame containment;
   - the extra enum variants;
   - `content_offset`;
   - the location of the *Keep pitch* flag.

   TL-023, AU-008, AU-023 and TR-036 need text edits. `edit-ops-and-undo.md` (trim and split
   helper wording) and `timeline-model.md` (reverse pitfall note) are answered by §5.4 and §5.8.
2. **I256 margin on very long curve clips.** Coefficients reach 235 of 255 bits at 24 hours.
   Beyond that we fall back to BigInt (about 0.6 µs) or return `CurveOverflow`. **Proposal:**
   cap curve clips at 12 hours of source and say so in the UI.
3. **Keep-pitch engine.**
   - `signalsmith-stretch` builds C++ through `cc`; it needs CI on MSVC and on the Apple and
     Linux toolchains.
   - Quality at speech rates of 2–4× and at curve ramps has to be compared by ear against
     Rubber Band and Bungee before v1.
   - The trait keeps the choice reversible.
   - Its pre-roll and latency API is from the crate docs (`seek`, `input_latency`,
     `output_latency`) and must be verified with the impulse test.
4. **Optical flow (TR-042).**
   - Whether FFmpeg's motion-interpolation filter is acceptable: the license of the specific
     FFmpeg build (Homebrew FFmpeg is configured with GPL parts, which is fine for a
     GPL-3.0-or-later app), quality, and the speed of a 4K offline job.
   - A wgpu flow shader is the long-term option, and an ML model would belong to `ai-tools.md`.
5. **Frame blend at 100× on long-GOP 4K.** Up to 16 decodes per output frame. Preview may need
   proxies (PM). Export is correct but slow, so we need a progress estimate.
6. **Packaging, interpret-rate and replace-media** all rewrite source time (§4.7). Each needs a
   test that keys and the map move together. PM and TL owners must call TR's helpers rather
   than edit `source` directly.
7. **Inverse lookups for drawing.** Exact inverses on long curve clips are about 45 exact
   evaluations each. The UI uses an `f64` sampled inverse for drawing only. Make sure no code
   path commits a time derived from it (code review rule, and a debug assertion that commit
   paths use the exact inverse).
8. **VFR without a full PTS index.** Until the probe-time index exists, frame choice for VFR uses
   the container's average frame duration. The still from a freeze and the frame shown can then
   disagree, so freeze waits for the index (it is fast: demux only, no decode).
9. **Compound clips with retime.** `source` is nested-sequence time, and the grid is the parent
   rate's frame grid (TL-040). Nested audio through a stretcher needs the nested mix rendered
   first. Confirm that this is in scope for v1.
10. **Beat-sync interactions** (inventory §10.5 and §11): applying a curve deletes beat markers,
    adding beat markers removes speed, and speed is blocked while motion tracking is on. These
    are owned by other areas, but they must be checks in `SetRetimeMode`'s request builder, not
    UI-only.

---

## References

**FreeCut (internal)**
- `docs/features/clip-transform-and-speed.md`: TR-027…TR-044, shared conventions
- `docs/features/audio.md`: signal chain, AU-008, AU-023, AU-024
- `docs/features/effects-transitions-animations.md`: FX-014, FX-018, FX-019
- `docs/features/timeline-editing.md`: TL-001, TL-011, TL-020, TL-023, TL-031
- `docs/features/project-and-media.md`: frame-rate conform, interpret rate, packaging
- `docs/architecture/timeline-model.md`: `Tick`, `Rational`, rounding table, `ticks_from_stream`
- `docs/architecture/edit-ops-and-undo.md`: `ReplaceClip`, trim/split/speed ops, open question 1
- Inventory notes `clip-transform-and-speed.md` §9, §10, §12 (scratchpad, paraphrased)

**libopenshot** (LGPL-3.0-or-later)
- `src/Point.h`, `src/Point.cpp`, `src/Coordinate.h`
- `src/KeyFrame.h`, `src/KeyFrame.cpp` (L44, L80, L131, L292, L522, L536)
- `src/Clip.cpp` (L428, L587, L751)
- `src/Timeline.cpp` (L693, ~L1074–1090)
- `tests/KeyFrame.cpp`, `tests/Clip.cpp`

**Olive** (GPL-3.0-or-later)
- `app/node/keyframe.h`, `app/node/keyframe.cpp` (L99, L154)
- `app/node/node.cpp` (~L420–540)
- `app/node/value.cpp` (~L95)
- `app/node/block/clip/clip.cpp` (L95, L110, L155, L201, L465)
- `app/node/time/timeremap/timeremap.cpp`
- `app/node/output/track/track.cpp` (L628–720, FIXME L668)
- `app/audio/audioprocessor.cpp` (L87)
- `app/timeline/timelineundosplit.cpp` (L32)
- `app/dialog/speedduration/speeddurationdialog.cpp`
- `ext/core` is an empty submodule in this clone

**MLT** (LGPL-2.1-or-later; `src/modules/rubberband` GPL-2.0-or-later)
- `src/framework/mlt_types.h` (L183–221)
- `src/framework/mlt_animation.c` (L453, L477, L1015, L1117, L1237, L1299, L1400)
- `src/modules/core/link_timeremap.c` (L59, L124, L152, L177, L347–349, L464, L525)
- `src/modules/core/link_timeremap.yml`
- `src/modules/core/producer_timewarp.c` (L92, L124, L171, L267)
- `src/modules/rubberband/filter_rbpitch.cpp` (L90, L165, L213)

**Shotcut** (GPL-3.0-or-later)
- `src/widgets/avformatproducerwidget.cpp` (L92)
- `src/docks/timelinedock.cpp` (L2520–2560)
- `src/mltcontroller.cpp` (L1356, L1450, L1522, L1664)
- `src/qml/filters/speed/meta_forward_reverse.qml` (L19)
- `src/qml/filters/speed/SpeedUI.qml`

**Kdenlive** (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)
- `src/definitions.h` (L211)
- `src/assets/keyframes/model/keyframemodellist.cpp` (L783, L868)
- `src/effects/effectstack/model/effectstackmodel.cpp` (L854)
- `src/timeline2/model/clipmodel.cpp` (L868, L1065, L1115)
- `src/dialogs/timeremap.cpp` (L1174, L1292)
- `src/utils/gentime.cpp` (L9, L116)

**OpenTimelineIO** (Apache-2.0)
- `src/opentime/rationalTime.h` (L229, L408)
- `src/opentimelineio/linearTimeWarp.h`, `src/opentimelineio/freezeFrame.h`,
  `src/opentimelineio/timeEffect.h`
- `docs/tutorials/otio-timeline-structure.md` (L19)
- `docs/tutorials/otio-serialized-schema.md` (L363)
- `docs/tutorials/time-ranges.md`

**Methods (public literature, implemented from the math):**
- Steffen, M., "A simple method for monotonic interpolation in one dimension", *Astronomy &
  Astrophysics* 239 (1990)
- cubic Hermite and cubic Bézier timing curves (CSS Easing Functions Level 1)
- Penner-style easing families
- Kaiser-windowed sinc interpolation
