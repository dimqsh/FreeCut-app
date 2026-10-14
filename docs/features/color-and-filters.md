# Color, adjustments & filters

> Status: draft · Owns: color stack and its in-item processing order, source color interpretation, HDR-to-SDR tone mapping and export color tags, color parameter conventions, filter/look presets (bundled and user), LUT assets and LUT parsing, adjustment clip content (filter/adjustment layers; the track kind is TL's), color-mask lists inside a color stack (the mask type and shapes are TR's), classic (model-free) video denoise, deflicker and dehaze, color scopes · Crates: freecut-timeline, freecut-render, freecut-media, freecut-project, freecut-app

## Summary

This area lets a FreeCut user change how footage looks: apply a ready-made look (a "filter") with an intensity slider, correct exposure and white balance, target single colors with HSL, shape tone with curves and color wheels, load their own `.cube`/`.3dl` LUTs, and clean up noisy or flickering footage. Any of these can go on one clip, or on an adjustment clip that grades every layer beneath it for as long as it spans. Helpers cover auto color, matching a shot to a reference frame, a white-balance picker, scopes, before/after compare, reset, apply-to-all, and saving a grade as a reusable look. Everything runs locally (today on FreeCut's CPU renderer, later also on the GPU), every bundled look and LUT is FreeCut's own, and nothing is behind a paywall or needs an account. The inspector presents all of this as the **Look** lens with five pages (CL-043), and the **Adjust** tab of the asset panel holds blank adjustment clips, saved looks and LUTs (CL-044). The current build round is [Adjustments round 1](#adjustments-round-1--next-build-scope).

Other areas are referred to by prefix; each prefix links to the spec that owns those concepts: PM [project-and-media.md](project-and-media.md), TL [timeline-editing.md](timeline-editing.md), TR [clip-transform-and-speed.md](clip-transform-and-speed.md), TX [text-and-captions.md](text-and-captions.md), AU [audio.md](audio.md), FX [effects-transitions-animations.md](effects-transitions-animations.md), OV [overlays-and-stickers.md](overlays-and-stickers.md), AI [ai-tools.md](ai-tools.md), EX [preview-and-export.md](preview-and-export.md), UI [workspace-and-shortcuts.md](workspace-and-shortcuts.md).

## Features

### CL-001 Color stack on visual items — v1

**What the user sees:** Selecting a video clip, image clip, raster overlay or adjustment clip shows a **Color** tab in the inspector (the **Look** lens of the UI concept; "Color tab" in this spec means that lens). Its layout is CL-043: five pages, **Basic**, **HSL**, **Curves**, **Wheels** and **Masks**. The Basic page holds the sections Auto, Match, Light, Color, LUT, Filter and Detail & finish. Detail & finish holds Denoise, Deflicker, Sharpen (which also softens), Texture, Clarity, Fade, Vignette and Grain, and later Dehaze. A section that holds non-default values shows a dot. Its header has an on/off switch that turns the section off without losing its values or keyframes. Each section shows its everyday controls first; a **More** disclosure reveals the rest (for example contrast pivot, texture, dehaze, and the vignette and grain sub-controls), and whether it is open is remembered per section as a UI preference. The tab header holds the master bypass (CL-023), the overall **Amount** slider (CL-035), a **Scopes** button that opens the Scopes panel (CL-021) and the tab menu (Save look…, Reset all color, Color position). On the timeline, a clip with any active color settings shows a small color badge. Reference: the reference app's adjust panel is likewise split into a basic part and an expandable part with extra parameters.

**Inputs / controls:** No controls of its own. It is the container the other CL features write into.

**Behavior / output:** Each eligible item owns one *color stack*. Where the stack runs in the frame is owned by FX: it is layer stage **L5** of the per-layer pipeline in [FX-001](effects-transitions-animations.md) (after source sampling, normalization, crop and keying; before clip effects, mask, transform, opacity and blending). CL owns only the order of the stages *inside* the stack, which is fixed as follows (FreeCut default). Right after the input transform comes footage repair (deflicker, then denoise, then dehaze once it ships), corrections come before creative looks, and texture comes last, so grain is never sharpened or graded:

| # | Stage | Feature |
|---|-------|---------|
| 1 | Input transform (log → display) | CL-032 (later) |
| 2 | Deflicker gain | CL-034 |
| 3 | Denoise (spatial, optional temporal) | CL-033 |
| 3b | Dehaze | CL-036 (later) |
| 4 | Color match transform | CL-020, CL-039 |
| 5 | White balance (temperature, tint) | CL-008 |
| 6 | Exposure (linear light) | CL-007 |
| 7 | Tone: brightness, contrast (with pivot), highlights, shadows, whites, blacks, tone lift | CL-007 |
| 8 | Color wheels (primary, or log mode later) | CL-015, CL-038 (later) |
| 9 | Curves (master, then R/G/B) | CL-014 |
| 10 | Hue/luma secondary curves | CL-029 (later) |
| 11 | HSL per hue band, then custom color targets | CL-013, CL-037 (later) |
| 12 | Hue rotation, saturation, vibrance | CL-008 |
| 13 | User LUT × intensity | CL-017 |
| 14 | Filter (look preset) × intensity, plus look parameters | CL-004, CL-040 (later) |
| 15 | Clarity, texture, sharpen or soften | CL-009 |
| 16 | Fade | CL-012 |
| 17 | Vignette | CL-010 |
| 18 | Grain | CL-011 |
| 19 | Amount and color-mask blend back toward the stage-3b output | CL-035, CL-031 |

Two controls are not stages of their own. **Auto** (CL-019) adds its stored offsets to the slider values of stages 5–7 and 12 before the stack is evaluated. **Skin protection** (CL-030) computes one skin mask from the stage-4 output and blends stages 5 and 11–12 (Color scope) and stage 13 (LUT scope) back toward their own inputs on skin-colored pixels.

Stages 1, 2, 4–14, 16, 17 and 19 change each pixel independently (stage 2 multiplies by a per-frame gain read from analysis data). The renderer fuses them into one shader pass. Stages 3, 3b, 15 and 18 look at neighboring pixels or frames, or need noise, so they run as extra passes, and only when non-zero. When denoise or dehaze is active, stages 1–2 are folded into the sampling of the first of those passes, and the fused pass starts at stage 4. Stages 2 and 3 are the only ones that read other source frames (CL-033, CL-034). Stage 19 needs the stage-3b output as its blend base, so the renderer keeps that image (in registers of the fused pass, or as the cleanup pass's output texture) until the end of the stack. A stack where every stage sits at its default is an identity stack, and the renderer skips it entirely. Amount 0 (CL-035) leaves only the cleanup stages 1–3b to run.

The parameter conventions below are FreeCut defaults, used across the whole area:
- Bipolar sliders run from −100 to +100 and default to 0. Unipolar sliders run from 0 to 100. Intensities run from 0 to 100 % and default to 100. All step by 1 in the UI.
- Every control is a continuous slider. Some reference-app detail controls offer only three steps (weak, standard, strong); FreeCut does not, because a slider covers those steps and keyframes need a continuous value.
- Typed values outside a range are clamped and the field flashes, per UI-009, with a tooltip naming the limit ("Limited to +100"). The reference app shows a notice that the value was capped; the UI-009 flash serves the same purpose without a toast.
- Spatial sizes (denoise radius, dehaze neighborhood, sharpen and soften radius, texture and clarity radius, grain size, vignette size) are relative to frame height. A preview at half resolution and a 4K export therefore look the same.
- Color math always runs in floating point: 16-bit float RGB on the GPU path, `f32` on the CPU path (README C52), whose frames are 8-bit straight-alpha RGBA in and out. Values between stages may go outside 0–1. The final value is clamped only when the frame is composited or encoded. There is no precision switch. The reference app offers floating-point processing as an optional mode that costs speed; in FreeCut, float is the baseline.

**Edge cases:**
- Text items do not get a color stack. TX owns their fill colors. An adjustment clip above text still affects the text.
- Missing or offline media renders FreeCut's offline placeholder without the color stack, so the warning stays readable. The stack is kept and comes back when the media is relinked.
- Splitting a clip (TL-023) gives both halves an identical copy of the stack. Color keyframes are split according to TR's keyframe rules ([TR-027](clip-transform-and-speed.md)).
- Duplicating or copy-pasting a clip (TL-032, TL-033) copies its stack by value. Copies are never linked.
- A freeze frame or still image gets the same stack. Animated stages such as grain still change every output frame. The temporal stages (deflicker, and denoise's temporal mode) need neighboring source frames, so they are unavailable on images and freeze-frame stills (CL-033, CL-034).
- Clips with an alpha channel are un-premultiplied before the stack runs and re-premultiplied afterwards. Fully transparent pixels stay transparent, so grain and vignette never paint into transparent areas.
- Items on a locked track show the Color tab read-only.
- A control that cannot be used in the current state (Temporal on an image, Deflicker on an adjustment clip, anything on a locked track) is disabled, and its tooltip says why (UI-009). Clicking it does nothing and creates no undo entry. The reference app blocks such edits with a toast instead.
- Compound clips (TL-040) carry their own color stack, applied to the flattened nested result like any video clip. Masks and the per-pixel stages need no preparation step. The temporal stages (CL-033 temporal mode, CL-034) are disabled on compound clips with "Apply inside the compound clip", because each neighboring frame would need a full nested render.
- Glow, blur and other image effects are FX effects (FX-015), not color controls. They appear in the Effects library, not in the Color tab.

**Undo:** n/a (this is the container). Each edit to it is an op defined by the feature that makes the edit.

**Model notes:**
- `freecut-timeline`: `ColorStack { cleanup: Cleanup, match_xf: Option<MatchTransform>, basic: BasicAdjust, wheels: ColorWheels, curves: Curves, hsl: Hsl, lut: Option<LutSlot>, filter: Option<FilterSlot>, finish: Finish, amount: u8, sections_enabled: SectionMask }`, with `Cleanup { denoise: Denoise, deflicker: Deflicker }` (CL-033, CL-034). `Cleanup` belongs to the Basic section for bypass, reset and copy/paste. `amount` is CL-035's. `ColorStack::is_identity()` reports a stack that the renderer can skip. `auto: Option<AutoCorrection>` (CL-019) and `masks` (CL-031) are v1 fields; `sections_enabled` names one bit per section of CL-043 (Auto, Match, Light, Color, LUT, Filter, Detail, HSL, Curves, Wheels, Masks), and `bypass: bool` is CL-023's master switch. Later features add fields through project-format migrations with identity defaults: `Cleanup.dehaze` (CL-036), log wheels (CL-038), HSL custom targets (CL-037), look parameters (CL-040) and `position` (CL-042).
- Non-time values are fixed-point integers. This is CL's own choice under the project-wide rule recorded once in TR's shared conventions ([clip-transform-and-speed.md](clip-transform-and-speed.md)): only *time* must be rational, and each owning spec picks its own non-time scalar type. Slider levels are `i16`, percentages are `u8` 0–100, curve coordinates are `u16` 0–10000, and wheel pucks are `i16` x/y in −1000..=1000. This keeps equality exact for undo tests and keeps serialization stable. The `ParamSpec { min, max, default }` table lives in the timeline crate and is the single source of truth for every range.
- Keyframable fields are TR-027's generic `Animatable<T: Interpolate>` over these fixed-point types. CL's types implement `Interpolate` the way TR-027 describes for fixed-point scalars: interpolate in `f64`, round to the type's step when evaluated, never when stored. The steps are fine enough (about 0.03 EV for exposure, 1 % for intensities) that animated values do not visibly step.
- A `ColorParamId` enum names every scalar slider, so `SetColorParam` can address it. TR keyframe tracks key off the keyframable ones (CL-028).
- `freecut-render` compiles a stack into a `CompiledColorStack`: a uniform block plus cached curve and LUT textures. It builds this on the render thread, never on the UI thread.
- `freecut-project` serializes the stack and omits sections that are at their defaults.

**FreeCut CPU path (2026-10-13, README C52).** Until the wgpu compositor exists, `freecut_render::cpu::Compositor` runs the stack for preview and export alike, and its results are the goldens the GPU path must match later. Where this paragraph differs from the model notes above, it wins for the build.
- *Model in code.* Every visual clip has `color: Option<Arc<ColorStack>>` (`None` is the identity default, so clips without a grade cost nothing to clone or save). Video, image and adjustment clips may carry one; text, audio, sticker and effect clips may not (`validate` refuses it). An edit that leaves a stack at identity with no keys stores `None`. Adjustment clips are `ClipContent::Generated { kind: Adjustment }` and use the same `color` field, so CL-006's `ItemKind::Adjustment { color }` is this field.
- *Working space.* Display-referred Rec.709 code values, as decoded (CL-002 and README §5.4), with straight alpha. Linear light only inside exposure, white balance and deflicker gain, through a pure γ 2.4 power with the sign mirrored for negative values (README §6.1, G5). Hue work (hue rotation, HSL shifts, match) uses Oklab computed from those γ 2.4-linear values. Luma is Rec.709-weighted code values. RGB of fully transparent pixels is never changed; alpha is never changed.
- *Precision.* The 8-bit input is read as `f32` 0–1, every stage runs in `f32` with the transcendental functions of the `libm` crate (bit-identical on every OS, README §5.5), and the result is quantized once at the end of the stack: `round(clamp(v, 0, 1) × 255)`. No dither in this round.
- *Baking.* The per-pixel stages that come before any spatial or position-dependent stage (4–14, then 16 and the Amount blend while nothing spatial, positional or masked follows) are baked into one RGB lattice over the 8-bit input cube, 33³ by default and 65³ when a curve is steep or a user LUT is finer than 33. Each pixel is then one tetrahedral lookup. Identity stacks are skipped, and a stack whose only active section is the LUT applies the user's LUT directly. The bake is cached by a digest of the evaluated values, so a static grade bakes once and a keyframed one re-bakes only when an evaluated integer value changes.
- *Where it runs.* A clip's stack grades its decoded picture in layer space, at the size the compositor asked for, before mip levels and placement. An adjustment clip's stack grades the frame drawn so far, in canvas space at output resolution. Spatial and masked stages run as separate passes after the lattice.

**K1 as built (2026-10-14).** Where this differs from the paragraphs above, it wins for the build; the measurements are in `crates/freecut-render/tests/color.rs`.
- *Hue rotation (stage 12) turns the Rec.709 luma/colour-difference plane of code values* (`Cb = (B − Y)/1.8556`, `Cr = (R − Y)/1.5748`) instead of Oklab. Measured against the 33³ lattice, the Oklab turn of γ 2.4-linear light was up to 19/255 off at Hue ±100: a turn pushes saturated colours out of gamut, and re-encoding a linear value that crosses 0 (infinite slope) and Oklab's cube root near the primaries are too steep to interpolate. The plane turn is linear in code values, so the lattice reproduces it exactly, and it keeps CL-008's promises: luma stays where it was, ±100 give the same picture, greys never move. HSL (K2) and Match (K5) meet the same question and decide it with their own measurements.
- *Saturation and Vibrance mix toward Rec.709 luma of code values* (`y + s·(c − y)`), so Saturation −100 gives R = G = B exactly. Vibrance's skin band is a hue angle of code values with red largest (about 15°–50°).
- *Curve shapes (FreeCut defaults the spec left open):* Brightness is a rational curve through (0.5, 0.707) at +100 and (0.5, 0.25) at −100 (where powers of 0.5 and 2 pass) with finite end slopes; Contrast is an S-curve through the Pivot with slope 2^(Contrast/100) there, a power toe and shoulder above 1× and a rational lift below; Blacks and Whites move their end of the range by up to 0.2; Highlights and Shadows add luma-weighted offsets `y⁴(1−y)²` and `y²(1−y)⁴` (peaks 0.15 at 2/3 and 1/3), flat at both ends; Tone lift adds `0.15·y(1−y)/0.25` at ±100, continued smoothly past 0 and 1. Every tone curve has a finite slope, so the lattice follows it: every slider alone at ±100 and every pair of sliders at +100 or −100 stays within 2/255 max and 0.4/255 mean (worst 1.95/255); all fourteen at +100 at once is 8/255 and is only checked for finite values. White balance applies normalized gains in linear light (log2 gains ±0.3 at ±100), keeping a grey's Rec.709 luma within 0.2 %.
- *Text clips have no Look lens* (CL-001's edge case); a title selected with a video shares the lens and is skipped by its edits.
- *Live drags coalesce* into one undo entry the way every inspector slider does (TL-043), with `Esc` reverting; the preview re-renders from the document each frame, so no separate `ColorDraft` override was needed.
- *A section or Reset all that removes keyframes* says how many in the status line and undo brings them back (the Placement card's rule), instead of asking first.
- *The pages after Basic* show an empty state ("Curves are coming in this build") while slices K2–K6 are open, instead of being hidden (CL-043's UI-004 rule resumes once the round is built). Since K2 only Masks does. The header holds the master switch (labelled **Grade**) and Amount; **Save look…** in the footer is disabled until K4.

**K2 as built (2026-10-14).** HSL, Curves and Wheels (stages 8, 9 and 11). Where this differs from CL-013 to CL-015, it wins for the build; the measurements are in `crates/freecut-render/tests/color.rs`.
- *Model.* `ColorStack` gains `hsl: Hsl { bands: [HslBand { hue, sat, light }; 8] }` (indexed by `HueBand`), `curves: Curves { luma, red, green, blue: Curve }` and `wheels: ColorWheels { shadows, midtones, highlights, offset: Wheel, strength }`. `Curve::new` refuses (with a `CurveError` naming the rule) fewer than 2 or more than 16 points, x not increasing by at least 100 (0.01), a left endpoint past x 2500, a right endpoint before x 7500, and coordinates above 10000. `Wheel::new` pulls a puck outside the disk back onto its rim along the same direction, in integers. `ColorParamId` gains `Hsl { band, channel }`, `WheelLuma(WheelId)` and `WheelStrength`; pucks are keyed as `PropPath::ColorPuck(WheelId)` with `PropValue::Puck { x, y }` (`color.wheels.<wheel>.puck`). `ColorEdit` gains `SetCurve { channel, curve }` and `SetPuck { wheel, x, y }`, plus `ResetBand(HueBand)` (CL-013's right-click reset of one swatch) and `ResetWheel(WheelId)` (a wheel's reset button: puck and luminance); both resets remove that band's or wheel's keys in the same entry, as a section reset does. Curves take no keys (CL-014).
- *Hue measure (HSL).* Band membership uses the HSV hue of the code values (`max − min` chroma), so the band centres are CL-013's degrees. A pixel's weights are raised-cosine between its two neighbouring centres and sum to 1, wrapping from magenta (300°) to red (360°). They are scaled by `smoothstep(0, 0.25, chroma)` for hue and saturation and by its square for lightness, so greys never move and grey walls stay put. The hue shift (±30° at ±100) turns the same luma/colour-difference plane as K1's Hue, so it keeps luma; saturation mixes toward luma (−100 on a fully weighted pixel gives grey); lightness lerps toward white (positive) or black (negative) by up to 50 %.
- *Wheels.* Each puck is a colour-difference vector (x along Cb, y along Cr, as on a vectorscope) turned into an RGB offset with zero Rec.709 luma, so a puck move never changes luma, for any pixel. The offset is weighted by the pixel's luma the way lift, gamma and gain weigh it: `(1 − y)` for Shadows (±0.10 at black), `4y(1 − y)` for Midtones (±0.10 at mid-grey, about gamma 0.75–1.33), `y` for Highlights (±0.25 at white, gain 0.75–1.25×) and 1 for Offset (±0.10). The luminance sliders are the standard per-channel curves applied to all three channels equally: lift `x + l(1 − x)`, gain `x(1 + g)`, a rational midtone curve through (0.5, 0.595) at +100 and (0.5, 0.398) at −100 (finite slopes at both ends, unlike a pure power), and offset `x + o`. Strength scales every puck and luminance value.
- *Curves.* Fritsch–Carlson monotone cubic through the points, flat (clipping) outside the endpoints, baked to a 1024-entry `f32` table read with linear interpolation; channel values are clamped into 0–1 before the lookup. The lattice is 65³ when an active table's steepest step exceeds a slope of 4.
- *Eyedropper.* `color::probe_hsl_input` averages the HSL-stage input of the opaque pixels in a 5×5 window; `color::hsl_band_at` names the band with the highest membership, or none below 5 % chroma or 3 % luma. The HSL page's eyedropper button is not drawn yet: picking needs a one-shot readback of the clip's source picture from the preview worker (the stage readback CL-013 names), which this slice does not add.
- *Measured.* Every HSL slider alone at ±100, every puck at the rim in five directions, every luminance at ±100, and gentle to steep curves stay within 1.2/255 max and 0.07/255 mean against `eval_pixel` (HSL and Wheels below 0.4/255); the steep case bakes at 65³. Greys never move under any HSL setting, the hue sweep around 0°/360° has no jump, and puck moves keep Rec.709 luma within 10⁻⁵. `COLOR_ALGO_VERSION` is 2.
- *Pages (CL-043).* HSL: eight swatches (a dot on edited bands, right-click resets one band, the chosen band is view state) above the chosen band's Hue, Saturation and Lightness rows. Curves: Luma, Red, Green and Blue tabs (a dot on changed curves) over a square editor drawn with the renderer's own curve table — click adds a point, drag moves it (Shift: vertical only), double-click or dragging it out of the graph removes it, a 17th point is refused with a status-line sentence — and **Reset curve**; when the clip has colour keyframes the editor says curves take none. Wheels: Strength, the four wheels two by two (each with its reset and its puck's keyframe navigator; drag moves the puck, Shift drags at a tenth of the speed, pressing away from the puck brings it there, double-click centres it), then the four luminance rows. Each section has the usual switch, reset badge, summary ("Orange · Blue", "Luma · Red", "Shadows · Strength 80 %") and `...` menu. Curves' **Auto** waits for K5 (it shares CL-019's analysis), and the histogram behind the editor for CL-021.
- *Format.* Version 11 (project-format.md › *HSL, Curves and Wheels*).

**K3 as built (2026-10-14).** The LUT section (stage 13), skin protection, the LUT library and FreeCut's bundled LUTs. Where this differs from CL-016, CL-017, CL-030 and CL-045, it wins for the build; the measurements are in `crates/freecut-render/tests/color_k3.rs`.
- *Model.* `ColorStack.lut: Option<LutSlot>` with `LutSlot { lut: LutRef { hash, file_name }, intensity, interp: LutInterp, skin: SkinProtect }`, and `BasicAdjust.skin: SkinProtect` for the Color scope (`SkinProtect { on, amount }`, off at 70 by default). `ColorParamId` gains `LutIntensity` and `SkinAmount(SkinScope)` (keyable; the LUT's sliders exist only while there is a LUT). `ColorEdit` gains `SetLut(Option<LutSlot>)`, `SetSkin { scope, on }` and `SetLutInterp(LutInterp)` (FreeCut addition: the interpolation under More needed an edit of its own). Picking a LUT for a clip that has one keeps that clip's Intensity, interpolation and skin protection (CL-017's "replaces it and keeps the intensity"), so one pick works across a multi-selection; `SetLut(None)` removes the LUT section's keys in the same entry. The LUT section's reset keeps the file (CL-017) and the Color section's reset switches its skin protection off. Skin protection alone changes nothing, so a stack with only a switch on still renders as identity.
- *Mask.* CL-030's three weights, each shoulder a raised cosine, computed from the stack input. The Color scope blends white balance and stages 11–12 back toward their inputs; while it is active, white balance is applied on its own and exposure after the blend. The LUT scope multiplies Intensity by `1 − mask × amount`.
- *Accuracy.* With an active skin scope the lattice is 65³: at 33³ the mask's hue shoulders (5°–12°, 42°–50°) were up to 8.8/255 off `eval_pixel`; at 65³ the worst case measured is 1.5/255 (mean 0.006/255). A LUT stage without skin stays within 0.4/255 at 33³ for a 17³ table. Identity tables (2³ and 33³) are exact through both the fast path and the lattice with both interpolations; the LUT-only fast path and the bake agree within 1/255.
- *Missing LUTs.* A slot whose file the LUT source can't give is left out before compiling, so the stage renders as identity, and the compositor reports it (`take_missing_luts`). The app shows **LUT missing** on the clip's timeline strip and a sentence under the select ("… can't be found, so the LUT does nothing. Import it again to bring it back"). Listing missing LUTs in the export preflight (EX) waits for a later slice.
- *Library and import.* `freecut_project::luts::LutLibrary` keeps `index.json` and one copy per file named by its hash in `<app data>/luts`, without parsing (README C54); re-importing the same bytes keeps one entry ("already in the library") and shows a removed entry again. `freecut_app::jobs::luts::import` runs on the job pool: folders are searched (non-LUT files are skipped and counted), each file is size-checked, parsed with `freecut_media::lut` and added; a broken file never stops the rest, and the summary names each failure with its line ("broken-a.cube (line 2: …)"), resampling notices and skipped keywords. The summary is a toast in this build, not a dialog. LUT files dropped on the window import the same way. The library index loads on the pool at startup; until it has, no LUT is called missing for being unknown. Rename, Remove (hide) and their persistence exist in `LutLibrary`; their UI comes with K4's Adjust tab, as do thumbnails and *Show file*.
- *Section (CL-017, CL-043).* The select lists **None**, then FreeCut's bundled LUTs under a **Bundled** heading, the library's under **Imported**, and **Import LUT…** last (headings are rows that can't be picked). With a LUT: **Intensity** (keyable), **Skin protection** with its **Amount** while on, and **Interpolation** under More. The section's switch shows only once a LUT is chosen. The Color section ends with its own **Skin protection** row and Amount.
- *Bundled set (CL-045).* Eight 33³ `.cube` files in `assets/luts/` (Contrast · Neutral 01, Warm · Gentle 01, Cool · Gentle 01, Faded · Print 01, Split · Teal and Warm 01, Mono · Neutral 01, Mono · Warm 01, Night · Blue 01), each baked by FreeCut's own stack from a recipe in `assets/luts/recipes/` (one or more passes of slider values, curves and pucks), with `manifest.json` (id, name, tags, rev, summary, file, hash). `crates/freecut-render/tests/bundled_luts.rs` re-bakes them byte for byte and checks that black and white stay within ±0.08 of a neutral grey; an ignored test of the same file regenerates them. All are CC0-1.0 (`assets/ASSETS.md`). The app compiles them in and its LUT source looks in this set before the library.
- *Format.* Version 12 (project-format.md › *The LUT slot and skin protection*). `COLOR_ALGO_VERSION` is 3.

**K4 as built (2026-10-14).** Adjustment clips (CL-006), saved looks (CL-026) and the Adjust tab (CL-044). Where this differs from those features, it wins for the build; the render tests are in `crates/freecut-render/tests/color_k4.rs`, the model tests in `crates/freecut-timeline/tests/adjust.rs`.
- *Adding one.* `EditRequest::AddAdjustmentClip { seq, span: AdjustSpan::{Selection(Vec<ClipId>), AtPlayhead(Tick)}, color: Option<Arc<ColorStack>> }` makes one `Generated { kind: Adjustment }` clip (one undo entry, selected afterward, `OpKind::AddAdjustmentClip`). Span: the union of the listed clips' ranges, or 3 s from the playhead (`default_adjustment_duration`, on the frame grid, never before 0). Placement: the lowest unlocked, visible `Adjustment` track above every track with content in the span that is free there; otherwise a new `Adjustment` track directly above the topmost track with content in the span (strict layering keeps it in the upper band, above every video track). A blank clip stores no stack. The attached-items pass (TL-029) runs as after any insert.
- *Plan and render.* `frame_plan` emits `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })` in the bottom-to-top walk for an adjustment clip whose evaluated stack is not identity; a blank, bypassed or Mix 0 clip, or one on a hidden track, is left out. The compositor draws the layers collected so far into the frame, then grades the whole frame (background included) in place through the same compiled-stack cache as clips, in canvas space at output resolution; Mix is the stack's Amount, folded into the lattice. Layers above are untouched, stacked adjustments run bottom-up, and the bytes don't depend on the thread count or on preview versus export (tested with three clips, a title and a keyed Mix). Text under an adjustment clip is graded with everything else (CL-001's edge case).
- *Applying sections.* `ColorEdit::ApplySections(Arc<SectionSnapshot>)` (`ColorOp::Look`) copies the snapshot's sections — values and switches — onto every listed clip that takes a stack and removes those sections' keys in the same entry; Amount, bypass and the other sections stay. `SectionSnapshot::new(stack, mask)` keeps only the masked sections (others at their defaults, Amount never). CL-025's `ReplaceColorSections` will be this edit.
- *Look files.* `freecut_project::look`: `LookFile { name, tags, created, snapshot }` with `read_look` / `write_look` for look format 1 (`{"format": "freecut-look", "look_version": 1, "name", "tags", "created", "sections", "color", "luts"}`; `color` is the project format's colour object, `luts` repeats the LUT files for other readers), and `LookLibrary` over `<app data>/looks/<random id>.fclook`. A look holds only `Section::LOOK` (Light, Color, LUT, Filter, HSL, Curves, Wheels), never Auto, Match, Masks, Detail's repair stages or Amount. A newer `look_version` is refused as "Made with a newer FreeCut (look format N)" and listed, never deleted; a duplicate name gets " (2)". Delete removes the file for good after a confirmation that says so, because FreeCut has no trash crate (features README › Color's open question).
- *Save look…* The Look lens footer's button opens a sheet (UI-064's shape): a name ("Look N" suggested), the sections as chips pre-chosen where the clip's values at the playhead differ from the defaults (Filter isn't offered while it has no values), and what is never saved (Amount, Auto, Match, masks; keyframes when the clip has colour keys). It is disabled with "Nothing to save — all color settings are at defaults" when nothing differs. The file is written on the job pool, then the Adjust tab shows Saved and a toast names the look. No live thumbnail yet.
- *Presets.* The lens header's first row is a **Presets** select of the saved looks; picking one applies it to the lens's clips as one entry.
- *Adjust tab (CL-044).* A shelf tab in the Grade group. Side list: **New** › *Blank* (the *Adjustment clip* tile), **Saved** › *Looks* (with files FreeCut can't read listed as *Unavailable* with their reason) and **LUTs** › *All*, *Bundled*, *Imported*. A click on the Adjustment clip tile, its `+` or `Enter` adds a blank adjustment clip with CL-006's span; a saved look or a LUT applies to the selected clips that take a stack ("Look applied to 2 clips", "LUT added", "LUT replaced (was …)"), and its `+` adds an adjustment clip carrying it. With nothing selected the status line says "Select a clip first, or use + to add an adjustment clip". The LUT rows have Import and search; a saved look's context menu has Apply, Add as an adjustment clip and Delete…. Tiles show FreeCut glyphs, not rendered thumbnails, and hover preview and dragging tiles onto the timeline wait for a later slice.
- *Format.* None: an adjustment clip's stack is K1's `color` field.

**K5 as built (2026-10-14).** Auto color (CL-019) and color match (CL-020). Where this differs from those features, it wins for the build; the render tests are in `crates/freecut-render/tests/color_k5.rs`, the model tests in `crates/freecut-timeline/tests/color.rs`, the analysis job's in `crates/freecut-app/src/jobs/color.rs`.
- *Model.* `ColorStack.auto: Option<AutoCorrection { offsets: AutoOffsets, style: AutoStyle, source: AutoSource, strength }>` and `ColorStack.match_xf: Option<MatchTransform { gain, offset (×10⁴), intensity, reference: MatchRef }>`, with `MatchRef::Clip { clip, at, graded }` (the frame's content time, and whether it was measured with its own grade) or `MatchRef::Media { media, at }`; CL-039's `group` waits for that feature. `ColorParamId::{AutoStrength, MatchIntensity}` are keyable and exist only while their result does, like the LUT's sliders. `ColorEdit::SetAuto(Some)` keeps a clip's Strength and Strength keys (Re-run), `SetMatch(Some)` its Intensity (Re-match), and `None` removes the result with its keys in the same entry; a section reset removes the result too. `ApplyAutoToSliders` adds the offsets at the *static* Strength into the sliders (a keyed slider's keys all move by its offset), clamps them, and removes the correction and its Strength keys, as one entry.
- *Offsets under the sliders.* `ColorStack::effective_basic()` gives the renderer each Light and Color slider as its manual value while its section is on (its default while off) plus Auto's offset × Strength / 100 (rounded half away from zero) while the Auto section is on, clamped into the slider's range. So switching Auto off leaves the manual values, and switching Light off leaves Auto's exposure. Auto is therefore no stage of its own and costs nothing in the bake.
- *Match space (measured).* The transfer is a gain and offset per channel of Rec.709 `Y'`, `Cb` and `Cr` of code values (the plane K1 chose for Hue), blended toward the input by Intensity in code values. CL-020's Oklab of γ 2.4-linear light was up to 34/255 off the 33³ lattice (18/255 at 65³) for ordinary transforms, because a/b gains push saturated colours through zero in linear light, where re-encoding is infinitely steep; the `Y'CbCr` transform is affine in code values and the lattice reproduces it exactly. Limits (FreeCut defaults): every gain 0.5–2× (CL-020 named L only), the `Cb`/`Cr` means move by at most 0.1, offsets stay within ±1; a limited result says so in the status line. A target or reference whose `Y'` spread is below 0.01 gives "Not enough detail to match". Equal statistics give exactly the identity, which the renderer skips.
- *Auto's analysis (FreeCut defaults).* Frames at most 512 px wide (decoded at that size, or box-filtered in integers), pixels with alpha above one half, histograms in integers and sums in `f64` in a fixed order. A spread from the 0.5 % to the 99.5 % point under 8/255 in luma and every channel is "Nothing to correct". White balance makes the average linear light of the usable pixels (no channel at 0.98, luma at least 0.05) that are nearly grey (channel spread at most 20 % of the largest, which keeps skin out) neutral, or of every usable pixel when fewer than one in twenty are nearly grey (plain gray world); Temperature and Tint are solved exactly for the evaluator's gains. Exposure lifts a white point below 0.96 to 0.98 (at most 1.5 EV), or brings a clipped picture whose median luma is above 0.62 toward 0.5. Blacks and Whites then move the 0.5 % and 99.5 % luma points to 0.02 and 0.98, but only ever stretch: a point already within 0.02 of its target, or past it, stays. Contrast moves a luma spread (after those) below 0.12 up toward 0.18, or one above 0.28 down, by at most ±30. Vivid adds Vibrance +20 and Saturation +5. A run whose offsets are all 0 says "Already balanced" and stores nothing. Auto measures the stack input (after the clip's match); Match measures the target before its own match, so neither compounds.
- *The job.* `freecut_app::jobs::color`: `plan` reads the document on the UI thread (five frames at the middles of five equal parts of the used range, or the frame at the playhead clamped into the clip; a still is one frame; a reference clip's frame with its grade at that time unless *ungraded*), `run` decodes in exact mode on the job pool with render's `MediaImages` (no new `freecut-media` helper) and measures, and `edit_for` commits on the UI thread unless the clip was deleted, its track locked, or its colour, content or keys changed meanwhile (then it says so and changes nothing). One analysis runs at a time; Cancel drops it. PM-073's task list doesn't exist yet, so the section shows the spinner and Cancel itself.
- *Sections (CL-043).* **Auto** sits first on the Basic page: **Analyze** (Whole clip, Frame at playhead) and **Style** (Natural, Vivid), remembered as view state, and the **Auto** button (**Re-run** once there is a correction); with a correction, its offsets as read-only rows ("Exposure +12", with "· limited" when the slider can't take all of it), **Strength** with its keyframe navigator, and **Apply to sliders**. **Match** shows **Match color…**, which enters pick mode: the section says "Click a frame of another clip on the timeline to match to" with **Use ungraded source**, **Use frame at playhead** (the topmost other visible video or image clip under the playhead, or the clip itself) and **Cancel** (or Esc); the timeline shows a crosshair over clips, starts no drags, and a click on a clip takes its frame under the pointer without changing the selection; selecting another clip ends pick mode. With a match: **Reference** (the clip's name and time, "· ungraded"), **Intensity**, **Re-match** (disabled once the reference clip is gone) and **Change reference…**. The undo labels are "Auto Color" and "Match Color".
- *Not built in K5.* Multi-clip runs (**Auto (N clips)**, **Auto color all clips…**, **Only clips that need it**, CL-039's match groups), **Use image file…** (the model and file format already hold `MatchRef::Media`), Auto and Match on adjustment clips (the composite below; the buttons are disabled with a sentence), and Curves' **Auto**.
- *Format.* Version 13 (project-format.md › *Auto and Match*). `COLOR_ALGO_VERSION` is 4.

**K6 as built (2026-10-14).** Color masks (CL-031) and the shared mask core of TR-017, TR-018 and TR-020. Where this differs from those features, it wins for the build; the render tests are in `crates/freecut-render/tests/color_k6.rs`, the model tests in `crates/freecut-timeline/tests/color.rs`, the Stage's geometry in `crates/freecut-app/src/ui/stage.rs`.
- *Mask core (`freecut_timeline::model::mask`).* `Mask { shape: MaskShape::{Linear, Band { width }, Ellipse, Rectangle { roundness }}, center, size, linked, rotation, feather, invert }` in fixed point: centre and size are `MaskPoint`s in ten-thousandths of the item's **shorter side** (centre ±3 shorter sides from the item's centre, +y down; width and height 0–400 %), rotation an `Angle` (±360°), feather, roundness and band width per mille (feather and roundness 0–100 %, band width 0–200 %). TR-017 measures the band width against the clip's height; the core uses the shorter side like every other length, which is the same on landscape footage. Defaults are TR-017's (an ellipse 50 % and linked, a rectangle 60 % × 40 %, a band 50 %, no feather). Switching shape keeps centre, size, rotation, feather and invert and starts the new shape's own value at its default. `linked` is a UI rule (width and height change together, keeping their ratio); the size is stored whole.
- *Color masks.* `ColorStack.masks: Vec<ColorMask { id, mask, expand, mode: MaskCombine }>`, at most 8, ids nonzero and unique (a new mask takes the smallest free id; keys name masks by id, so reordering never moves a key). Masks alone change nothing: a stack with masks and no grade is the identity, and the Masks section's switch off means full coverage. Reset of the Masks section removes every mask; Reset all removes them too.
- *Combine (FreeCut decision).* Masks join in list order with TR-020's Add (max), Subtract (`c·(1 − m)`) and Intersect (min). The coverage before the first mask is nothing when the first mask adds and everything otherwise, so a list that starts with Subtract grades everything but that shape.
- *Edits.* `ColorEdit::{AddMask(Mask), RemoveMask(id), ReorderMask { id, to }, SetMask(ColorMask), SetMaskMode { id, mode }, SetMaskParam { id, param, value }}`; `SetMaskParam` is CL-031's `SetColorMaskParam` (one value, one entry per drag) and maps to the property's key path, so a keyed value is edited through its keys like a keyed slider. `RemoveMask` removes the mask's keys in the same entry; `SetMask` to another shape removes the keys of the values it lacks. The undo label is "Color Mask".
- *Keys.* `PropPath::ColorMask { id, param: ColorMaskParam }` as `color.masks.<id>.<center|size|rotation|feather|roundness|band_width|expand>`; centre and size are `PropValue::MaskPoint` (straight-line interpolation per coordinate), rotation an `Angle`, the ratios `Ratio`, Expand a `Level`. They belong to the Masks section's group (**Key this card** keys every value of every mask). A key of a mask the stack no longer has is ignored.
- *Coverage (`freecut_render::shape::mask`).* Signed distances in shorter sides: Linear `q.y` (the side above the edge is kept), Band `|q.y| − w/2`, Ellipse the first-order `f/|∇f|`, Rectangle the rounded-box distance with radius `roundness × min(w, h)/2`. Expand moves the edge by up to ±10 % of the shorter side, feather is a smoothstep across 0–25 % of the shorter side centred on the edge but never under one pixel (a hard edge is antialiased), a shape with no area covers nothing, then invert. Plain `f32`; the rotation's sine and cosine come from `libm`.
- *Grading.* A stack whose masks limit it compiles its lattice from the stack without masks and at Amount 100 (cached by that digest, so a moving mask never re-bakes) and blends every pixel `base + (graded − base) × amount × coverage` in `f32` before the single quantization; full coverage writes the lattice value itself and none leaves the pixel. Alpha is never touched. A clip's masks are evaluated in its display frame, mapped onto the stored picture through its rotation, mirroring and pixel aspect, so they follow the clip's transform and don't depend on the decoded size; an adjustment clip's in canvas space at output resolution. The digest covers the active masks; `COLOR_ALGO_VERSION` is 5.
- *Masks page (CL-043).* One clip at a time (masks are per clip; with several graded clips it says "Select one clip to edit its masks"). Chips "1 · Ellipse" in list order (right-click: Move earlier, Move later, Delete mask), **Add mask** with FreeCut's own painted glyphs for the four shapes (greyed with "A clip holds at most 8 masks"); with none, "No masks: the grade covers the whole picture. Add a mask to grade only part of it." For the chosen mask: **Shape** (the glyph picker), **Combine**, **Invert**, **Position X/Y**, **Width/Height** and **Keep ratio** (ellipse and rectangle), **Rotation**, **Feather**, **Roundness** or **Band width**, **Expand**, each value row with its keyframe navigator, then move buttons and **Delete mask**. The chosen mask is view state. Reset all, while the clips hold masks, asks in the footer first ("Also removes 2 masks?") and its result names them.
- *Stage (TR-019).* While the page shows a mask of the one selected clip, the Stage draws its outline (Expand included), its feather band dashed, and handles: the centre (move), the right and bottom edges and the corner (size; linked keeps the ratio, Shift keeps it on the corner), a knob past the shape (rotation; along the middle for a line or a band; Shift snaps to 15°), a small handle outside the edge (feather) and a band's edge (width). Each drag is one undo entry, Esc cancels it, a keyed value takes a key at the playhead. Not built: the roundness handle (the row sets it), arrow nudges, and hiding the clip's own outline while a mask is edited.
- *Not built in K6.* Heart and Star (TR-017 adds them to the core), the pen and brush masks, motion-tracked masks (AI-008), Paste attributes of the Color masks category, the export preflight line for masks, and the `color_bench` mask line (≤ 2 ms per mask) measured in release.
- *Format.* Version 14 (project-format.md › *Color masks*). Saved looks never hold masks.

### CL-002 Color-managed decode and encode baseline — mvp

**What the user sees:** Nothing visible, and that is the point. Colors in the preview and in an exported MP4 match the source file. Exporting does not wash out, darken or shift skin tones. In media info (PM), a clip whose color tags were missing shows "Color: assumed BT.709".

**Inputs / controls:** No user controls in mvp. A per-clip "Interpret color as…" override (BT.601 / BT.709, limited / full range) is v1. It sits in the clip's media properties (PM).

**Behavior / output:**
- Decode uses the stream's tagged matrix, range and transfer. Untagged SD (height < 720) is treated as BT.601 and untagged HD or larger as BT.709. Untagged YUV defaults to limited range and untagged JPEG/PNG to full range.
- The shader converts YUV to RGB using the correct matrix and range. This runs in FX's L2 normalize stage ([FX-001](effects-transitions-animations.md)); CL defines which conversion is used. The working space is Rec.709 primaries with gamma-encoded values, stored in `rgba16float` textures, so 10-bit sources keep their precision.
- Images (PNG/JPEG/WebP) are treated as sRGB-encoded with Rec.709 primaries.
- Export (EX) writes BT.709, limited-range YUV and tags the output stream (primaries, transfer and matrix all set to BT.709) so players do not guess.
- The preview shows working-space values directly, without an ICC display transform (FreeCut default, the same as typical consumer editors). Display calibration is out of scope (see below).

**Edge cases:**
- HDR sources (PQ/HLG, BT.2020) are tone-mapped to SDR Rec.709 with a filmic tone curve in the same L2 conversion shader (`freecut-render`), so phone HDR footage does not look washed out. The curve keeps hue by scaling R, G and B together, places HDR reference white (203 cd/m², BT.2408) at about 90 % of the SDR signal, and rolls off everything above it with a BT.2390-style shoulder up to the source's peak. That peak comes from the stream's mastering or content-light metadata, or is assumed to be 1000 cd/m² for HLG and for untagged PQ. BT.2020 colors outside Rec.709 are compressed softly toward the gamut edge, not clipped per channel. The clip shows a one-time notice that the HDR source was converted to SDR. In mvp the conversion is fully automatic; v1 adds per-media options (CL-041). This is FreeCut's single HDR rule: FX-001's HDR edge case and PM's thumbnails point here instead of defining their own. SDR Rec.709 stays the only working and output space, and true HDR output is out of scope (see Out of scope). Alternative: the reference app converts clips among HLG, PQ and SDR in all six directions, SDR to HDR included, inside an HDR-capable project. FreeCut converts only HDR to SDR.
- The preview is always SDR, even on an HDR-capable display. There is no "display can't show HDR" fallback because nothing is ever shown in HDR.
- SDR sources tagged with BT.2020 primaries (some phones and cameras) are converted to Rec.709 primaries with the same soft gamut compression, with no tone mapping.
- Wrong tags are a common problem with screen recordings. The v1 override fixes them, and changing it is undoable.
- Mixed SD, HD and 4K clips on one timeline each decode with their own matrix.
- Wide-gamut images such as Display-P3 PNGs are treated as Rec.709 in mvp. Their colors may oversaturate slightly, which is documented.

**Undo:** mvp: n/a, since nothing is user-editable. v1 override: `SetColorInterpretation { media, old, new }`, which swaps the values back. CL-041's HDR options are part of the same override and use the same op.

**Model notes:** `freecut-media` probes the tags and exposes `ColorInfo { matrix, range, transfer, primaries }` on media metadata (PM). `freecut-render` owns the conversion shaders. `freecut-media` (via EX) sets the encoder tags. The interpretation override lives on the media reference (PM), not on the clip.

**FreeCut CPU path (2026-10-12):** until the L2 shader exists, the HDR → SDR conversion runs on decode in `freecut_media::color` (the CPU reference the shader's goldens will use; media-io.md implementation note), so preview, export and thumbnails all show the converted picture. Settled defaults: light is divided by 203 cd/m² and BT.2390's EETF maps the source peak onto SDR white, which places reference white at about 90 % for a 1000 cd/m² source (HLG 75 % → 230 of 255) and about 83 % for a 4000 cd/m² master; the soft gamut compression starts at 80 % of the distance to the Rec.709 edge.
- FreeCut default (app, 2026-10-12): the one-time notice is a status-line sentence, "<file> is HDR video (HLG): FreeCut converts it to SDR for preview and export" (PQ or HLG), or "N files are HDR video: …" when several arrive together. It appears when a PQ or HLG item is first imported, and again for each such item when a project is opened; within the open project it is never repeated for the same media item. Detection reads the default video stream's transfer tag: the media library's poster job (run for every video item on import and on open, off the UI thread) reports it with the picture. SDR with BT.2020 primaries is gamut-mapped silently and gets no notice. No HDR badge on media tiles in mvp. Each poster job carries the project it was started for; one that finishes after another project opened is dropped, so neither its picture nor its notice lands on an item of the new project that has the same id.

### CL-003 Filter library browser — v1

**What the user sees:** A **Filters** tab in the left asset panel. It shows a grid of thumbnails, each one the current frame of the selected clip (or the frame under the playhead) rendered through that look. Category chips run along the top: All, Favorites, Natural, Warm, Cool, Film, Black & white, Vivid, Moody, Pastel, Night, Food & product (FreeCut categories). There is a search box, and each thumbnail has a star toggle for favorites. A first item called **None** removes a filter. Reference: the reference app's filter library likewise groups looks into categories and offers search, an entry that clears the filter, and favorites. Its favorites need a sign-in and its looks download on first use. In FreeCut, every look is on disk from install and favorites are a local preference.

**Inputs / controls:**
- Single-click selects a thumbnail. Hovering a thumbnail previews that look live in the program monitor (CL-004).
- Double-click or Enter applies the look to the selected clips. Dragging a thumbnail onto a clip applies it to that clip.
- The "+" button on a thumbnail, or dragging it onto empty track space, creates an adjustment clip (CL-006).
- The search box matches name and tags, case- and accent-insensitive.
- Star toggles a favorite. Favorites are stored globally in app settings, not in the project.

**Behavior / output:**
- Thumbnails are 160×90 px. They render lazily on the render thread for the visible grid only, and are cached against (preset id, source frame hash).
- When the selected clip or playhead frame changes, thumbnails regenerate, debounced by 300 ms (FreeCut default).
- With no media on the timeline, thumbnails use a built-in neutral sample image: FreeCut's own CC0 test image with color patches plus skin, sky and foliage (any person shown has signed a model release). Alternative: the reference app needs media on the timeline before a filter can be previewed. FreeCut always has something to preview.
- There are no download, retry or "failed to download" states, because nothing is fetched. The only unavailable state is a missing user look or LUT file (CL-026, CL-016).

**Edge cases:**
- With nothing selected, double-click does nothing and shows a hint: "Select a clip or use + to add a filter layer".
- If the selection is all text or audio, applying is disabled and the tooltip explains why.
- A favorite whose preset no longer exists (a removed user look) is dropped quietly from Favorites.
- Thumbnails of very dark or blank frames still render. The grid never blocks on a decode, and cells show a placeholder until ready.
- A search with no matches shows an empty state with a "Clear search" button.
- An empty **Favorites** category explains the star toggle.
- The user's saved looks and LUTs are not listed here. They live in the Adjust tab (CL-044), so each saved item has one home. (Revised 2026-10-13; an earlier draft had a **My looks** chip here.)

**Undo:** n/a (view state). Favorites are an app preference and are not on the undo stack. Applying a look goes through CL-004 and CL-006.

**Model notes:** `freecut-app` owns the browser UI and the thumbnail job queue. `freecut-render` exposes a function that renders a stack onto a small frame. The preset catalog (bundled plus user) is loaded by `freecut-project::presets` on a worker thread at startup.

### CL-004 Apply filter to a clip, with intensity — v1

**What the user sees:** The clip's inspector Filter section shows the applied look's name and thumbnail and an **Intensity** slider. A small "×" removes the filter. While a filter thumbnail is hovered, the program monitor temporarily shows the clip with that look. Moving off the thumbnail restores the clip.

**Inputs / controls:**
- Intensity: 0–100 %, default 100 (FreeCut default). It can be keyframed (CL-028).
- One filter slot per color stack. Applying another look replaces the current one and keeps the current intensity, unless the old filter was None, in which case intensity starts at 100.

**Behavior / output:**
- Stage 14 output = lerp(input, look(input), intensity / 100).
- A look is our preset recipe (CL-005). It is baked into a 33³ 3D LUT for the tonal and color part. It may also add finishing amounts for grain, vignette and fade, and these are scaled by intensity and *added* to the stack's own finishing values, clamped to each range.
- Applying to N selected clips sets the filter on each of them in one operation.
- A short status toast confirms what happened and says whether the look was added or replaced: "Look added", "Look replaced (was <name>)", or "Look added to 5 clips". Apply to all goes through CL-025.
- Some bundled looks expose extra parameters under the Intensity slider (CL-040, later).
- Reference: public guides describe consumer editors capping filter and LUT intensity at 100 with one filter per clip. The reference app's UI shows one filter per clip too, with separate confirmations for adding and replacing. We match that, and multiple looks can be layered with adjustment clips.

**Edge cases:**
- Intensity 0 keeps the filter assigned but makes it invisible. The badge shows it as present but at 0.
- A project from a newer FreeCut might name a bundled preset this build lacks. The filter then shows "Unknown look (id …)", renders as identity and keeps its data, and saving the project keeps the id. The clip's color badge switches to a warning state ("Missing look") instead of failing silently, and export preflight (EX-036) lists the affected clips without blocking export.
- If a look's baked LUT cannot be loaded (for example a damaged install), applying it is refused with "Couldn't apply this look". Nothing changes and no undo entry is created. Existing uses render as identity with the missing-look badge.
- Hover preview never creates an undo entry and never marks the project dirty.
- Applying to a clip on a locked track is refused with a toast. In a mixed selection, the locked clips are skipped and the toast reports how many.
- Applying to an image or freeze frame works the same way.

**Undo:** `SetFilter { item, old: Option<FilterSlot>, new: Option<FilterSlot> }` per item, grouped into one transaction for multi-select. Undo restores each old slot. Intensity drags coalesce into one `SetColorParam` per gesture (see CL-007 Undo).

**Model notes:** `FilterSlot { preset: PresetId, preset_rev: u32, intensity: u8 }` in `freecut-timeline`. `PresetId` is a stable string such as `freecut.look.warm.03`, or `user.<uuid>` for user looks applied as a filter (see CL-026). `freecut-render` caches each baked look as a 3D texture keyed by (id, rev).

### CL-005 Bundled FreeCut look presets — v1

**What the user sees:** About 30 looks ship with v1 (FreeCut default), grouped by category. Each has a plain descriptive name made of category plus a short descriptor and a number, for example a warm faded film look, a cool teal-shadow look, a high-contrast mono look, a soft pastel look, or a punchy food look. A tooltip says in one line what the look does.

**Inputs / controls:** None beyond CL-003 and CL-004.

**Behavior / output:**
- Each look is a *recipe*: a JSON description of stack values (white balance, tone, wheels, curves, HSL, saturation, optional grain, vignette and fade) written by FreeCut contributors. A recipe can carry finishing amounts as well as the tonal part, so one bundled look already covers what the reference app offers as a separate bundle of a filter plus other look settings. Users build their own combined looks with CL-026.
- At build time a deterministic `xtask` bakes the point-wise part of each recipe into a 33³ `.cube` file. The renderer loads that LUT.
- Recipes and baked LUTs both live in the repo under `assets/looks/`, licensed CC0-1.0.
- Recipes are versioned (`rev`). Changing a look's appearance bumps `rev` and keeps the old rev's LUT, so existing projects render exactly as before (FreeCut rule).

**Edge cases:**
- A look is never deleted from the catalog, only hidden as deprecated, so old projects keep resolving it.
- A look should be close to neutral on clipped whites and pure blacks, so it doesn't paint borders. Baking checks this: (0,0,0) and (1,1,1) must map within ±0.08 of a neutral gray.
- A monochrome look at 50 % intensity gives half desaturation. That is expected.

**Undo:** n/a (shipped content).

**Model notes:** Recipes are parsed with the same serde types as user looks (CL-026). The baking tool reuses `freecut-render`'s CPU reference implementation of the stack, which also serves as the golden-test oracle for the WGSL shaders.

### CL-006 Adjustment clips (filter and adjustment layers) — v1

**What the user sees:** A timeline item with no media, drawn as a tinted bar labeled with what it carries ("Look: <name>", "Adjustment", or "LUT: <file>"). Selecting it shows the same Color tab as a clip, plus a **Mix** slider. Everything visible on tracks *below* it, for as long as it spans, takes on its grade. It can be added blank from the Adjust tab's **Adjustment clip** tile (CL-044) or the timeline's Add menu ("Adjustment clip"), or already graded from a look in the Filters browser ("+"), a saved look or a LUT in the Adjust tab ("+"). It has no Picture lens: its transform and opacity are not used, and Mix is its only strength control.

**Inputs / controls:**
- Full color stack: filter, basic, HSL, curves, wheels, LUT, finish.
- Mix: 0–100 %, default 100. It can be keyframed and blends between the composite below (after the adjustment clip's own denoise, if any) and the adjusted composite. It is the same control as the overall Amount on clips (CL-035), labeled Mix here.
- Start and duration are trimmed and moved like any clip (TL).
- Default span on creation (FreeCut default, overriding TL's generic 3 s for this item kind): the union time range of the selected clips if any, otherwise 3 s starting at the playhead.

**Behavior / output:**
- Adjustment clips live on `Adjustment` tracks, a track kind defined in [TL-004](timeline-editing.md). Layering is TL's: under the default strict layering (TL-004), `Adjustment` tracks sit above every `Video` track; with **Flexible stacking** ([TL-007](timeline-editing.md)) the user can move one lower so it affects only the tracks beneath it.
- Placement of a new adjustment clip mirrors effect layers ([FX-011](effects-transitions-animations.md)): dropped on an existing `Adjustment` track if one is targeted and the span is free there, otherwise on a new `Adjustment` track created directly above the topmost track with content in that span (TL track-creation rules).
- Rendering follows step 6 of [FX-001](effects-transitions-animations.md), whose walk visits tracks in TL's validated bottom-to-top order: the input is the accumulator (everything beneath it, including the canvas background), the stack is applied to it, and the output replaces the accumulator, blended by Mix. Tracks above are not affected. Stacked adjustment clips run bottom-up.
- Spatial parameters (vignette, grain, sharpen) use canvas coordinates, not any one clip's frame.
- The canvas background (TR) counts as "below", so an adjustment over letterboxed areas also grades the background fill.
- Audio is never affected.
- Reference: public guides describe the same split in other editors: a look applied straight to one clip, or added as a layer and stretched over many clips. The reference app's library likewise has an entry that creates an adjustment clip on its own track, affecting every layer below for its duration.

**Edge cases:**
- Minimum duration is one frame. Trimming to zero is refused, the same rule as other clips (TL).
- An adjustment clip with nothing below it in part of its span grades only the background there.
- With Flexible stacking, an `Adjustment` track moved directly above the main track grades only the main track and the background; overlay tracks above it are untouched.
- A span that starts or ends in the middle of a clip changes that clip's look abruptly at the boundary. There is no automatic ramp, so users ramp with Mix keyframes. A hint appears the first time this happens.
- An adjustment clip over a transition grades the blended result.
- Splitting an adjustment clip copies its stack to both halves.
- A hidden `Adjustment` track ([TL-008](timeline-editing.md)) disables its clips in preview and export.
- On a locked track ([TL-009](timeline-editing.md)), the adjustment clip still renders but cannot be edited.
- An adjustment clip that ends after the last media item extends the sequence, because sequence duration is the end of the last item on any track ([TL-003](timeline-editing.md)). The extra frames show the graded canvas background.

**Undo:** Created with `InsertItem { AdjustmentClip }` and removed with `RemoveItem` (TL generic ops, which restore the full stack on undo). Moves and trims are TL ops. Color edits use the same ops as clip color edits. Mix uses `SetColorParam` with `ColorParamId::Amount` (CL-035), one entry per drag.

**Model notes:** `freecut-timeline`: `ItemKind::Adjustment { color: ColorStack }` on an `Adjustment` track (TL-004). Mix is stored as `color.amount` (CL-035), so clips and adjustment clips share one field and one `ColorParamId`. It has no media reference. Render order and compositing belong to FX; this spec only fixes that an adjustment clip applies its color stack to the composite below it. An adjustment clip carries color only: effects that should affect many layers go on FX's separate effect layers (FX-011), which share the same position in the render walk.

**FreeCut CPU path (2026-10-13, README C52).** In code an adjustment clip is `ClipContent::Generated { kind: Adjustment }` with its stack in the clip's `color` field (CL-001's CPU-path note). `frame_plan` emits it as `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })`, in the same bottom-to-top walk as layers, and leaves it out when its evaluated stack is identity or bypassed. The compositor draws every layer collected so far into the frame, then grades the frame in place at output resolution and blends by Mix; the translucent placeholder that generated layers draw today is no longer drawn for adjustment clips. A blank adjustment clip is valid and renders nothing until it is graded.

### CL-007 Light adjustments — v1

**What the user sees:** In Basic › Light, a column of labeled sliders, each with a numeric field and a reset arrow that appears when the value is not the default.

**Inputs / controls:** All of these are FreeCut defaults, since neither public sources nor the observed UI labels give ranges. The reference app's light section has the same core set (exposure, brightness, contrast, highlights, shadows, whites, blacks). It also has a few labels that FreeCut maps onto existing controls rather than duplicating them: its midtone control is **Brightness**; its backlight-correction and adaptive-lightness controls are covered by **Tone lift**; its overall light-level control is **Exposure**; its middle-gray reference is the contrast **Pivot**. Its detail-restoration control is either ML enhancement, which belongs to AI-019, or local tone mapping, which is the later edge-aware version of Highlights/Shadows noted below.
- **Exposure:** −100..+100, default 0. Maps linearly to −3..+3 EV.
- **Brightness:** −100..+100. A midtone power curve that keeps black and white fixed (exponent about 2.0 at −100, 0.5 at +100).
- **Contrast:** −100..+100. An S-curve pivoting at the Pivot value (by default the encoded value of 18 % gray). The midpoint slope runs from 0.5× to 2×, with soft shoulders and toes so it never hard-clips.
- **Pivot** (under More): −100..+100, default 0. 0 is the encoded value of 18 % gray (about 0.41), and ±100 moves the pivot to about 0.21 / 0.61. A lower pivot makes contrast brighten more of the image, and a higher pivot makes it darken more. It has no effect while Contrast is 0.
- **Highlights:** −100..+100. Brightens or darkens tones above mid-gray using a smooth luma weight. Negative recovers bright areas.
- **Shadows:** −100..+100. The same for tones below mid-gray. Positive lifts dark areas.
- **Whites:** −100..+100. Moves the top end of the tonal range: positive pushes toward clipping, negative pulls whites down.
- **Blacks:** −100..+100. Moves the bottom end: negative crushes, positive lifts.
- **Tone lift:** −100..+100. A combined control: positive brightens shadows and midtones while compressing highlights, to even out a backlit or flat shot. Negative does the reverse.

**Behavior / output:**
- Exposure is applied in linear light: decode the transfer, multiply by 2^EV, re-encode. The other controls work on gamma-encoded values.
- Highlights and shadows are global tone-range operations in v1, with no spatial local tone mapping. A local or edge-aware version is a later improvement behind the same slider.

**Edge cases:**
- All sliders at ±100 at once must never give NaN or Inf. Covered by a property test over corner values.
- Exposure +100 on an already bright clip clips to white. That is allowed and visible on the histogram (CL-021).
- Highlights −100 on a clip that is already clipped can't bring back detail. It only turns the clipped area gray, and that is expected.
- Keyboard entry outside the range is clamped to the range, and the field flashes with a "Limited to ±100" tooltip (UI-009). Non-numeric text reverts to the previous value.
- Fine adjustment: arrow keys step ±1 and Shift+arrow steps ±10 (the UI area owns the bindings).

**Undo:**
- `SetColorParam { item, param: ColorParamId, old, new }`.
- A drag gesture is one undo entry. While dragging, the UI sends a *transient override* to the renderer that does not touch the model. On release it commits one op from the value at drag start to the final value.
- Undo writes back the old value. If the param has keyframes, the op targets the keyframe at the playhead instead (TR rules).

**Model notes:** `BasicAdjust { exposure, brightness, contrast, contrast_pivot, highlights, shadows, whites, blacks, tone_lift, temperature, tint, hue, saturation, vibrance: i16 }` in `freecut-timeline`, with validation against the `ParamSpec` table. The math lives in `freecut-render` (WGSL plus a CPU reference). Golden tests compare both on fixed test charts.

### CL-008 Color balance: temperature, tint, saturation, vibrance — v1

**What the user sees:** In Basic › Color, five sliders. The temperature track is drawn with a blue-to-amber gradient, the tint track with a green-to-magenta gradient and the hue track with a rainbow. Reference: the reference app's color section has the same set (temperature, tint, hue, saturation, and a vibrance-like color-boost slider), plus a skin-protection toggle, which is FreeCut's CL-030.

**Inputs / controls:**
- **Temperature:** −100 (cooler, bluer) to +100 (warmer, more amber), default 0. About ±3000 K equivalent around D65 (FreeCut approximation).
- **Tint:** −100 (green) to +100 (magenta), default 0.
- **Hue:** −100..+100, default 0. Rotates every hue around the neutral axis by up to ±180° (FreeCut default mapping, 1.8° per step), in Oklab so lightness stays put.
- **Saturation:** −100 (fully monochrome, Rec.709 luma weights) to +100 (2× chroma), default 0.
- **Vibrance:** −100..+100, default 0. Saturation weighted toward low-saturation pixels, and reduced across the skin-hue band (roughly 15°–50°) so faces don't go orange.

**Behavior / output:**
- White balance (stage 5) applies per-channel gains in linear light along the blue/amber and green/magenta axes. The gains are normalized so overall luminance stays roughly constant.
- Hue rotation, then saturation, then vibrance (stage 12) run after HSL, so they act on the result of per-band edits.

**Edge cases:**
- Saturation −100 followed by a colored LUT or filter still produces color, because stage order is fixed and documented.
- Hue +100 and −100 give the same picture (a 180° turn either way). Gray pixels never change under hue rotation. A keyframed hue sweep from −100 to +100 cycles once through every hue, which is a valid creative effect.
- For a targeted color change, the user should use HSL (CL-013) or custom targets (CL-037). The tooltip says so, because global hue rotation also shifts skin.
- Temperature on a black-and-white source only tints it. That is expected.
- Vibrance on an already saturated image changes little, by design.
- The white-balance picker (CL-018) and auto color (CL-019) write these same sliders, and their results can be edited.

**Undo:** `SetColorParam` (same coalescing as CL-007).

**Model notes:** Fields live in `BasicAdjust`. The math lives in `freecut-render`.

### CL-009 Detail: sharpen and clarity — v1

**What the user sees:** In Basic › Detail & finish, **Sharpen** and **Clarity** sliders, with **Texture** under More. The three work at three scales of detail: fine edges, mid-size surface detail, and large local contrast. Reference: the reference app's detail section lists sharpen, clarity, texture and a separate soften control. FreeCut folds soften into the negative half of Sharpen. Alternative: a separate Soften slider, rejected because sharpening and softening the same clip at once has no use.

**Inputs / controls:**
- **Sharpen:** −100..+100, default 0. Positive is an unsharp mask on luma. Radius is about 0.1 % of frame height (about 1 px at 1080p) and maximum amount is 1.5. Negative softens: a gentle Gaussian blur of luma and chroma, reaching a radius of about 0.3 % of frame height at −100.
- **Texture:** −100..+100, default 0. Local contrast on luma at a medium radius (about 0.5 % of frame height). Positive brings out surface detail such as fabric, foliage and stone. Negative smooths mid-size detail such as skin texture, while edges blur less than with Soften.
- **Clarity:** −100..+100, default 0. Local contrast on luma with a large radius (about 2 % of frame height), weighted toward midtones to limit halos. Negative values soften.

**Behavior / output:**
- All three run in one spatial pass with separable blurs (stage 15), only when at least one is non-zero. Clarity applies first, then texture, then sharpen or soften.
- Radii scale with the frame being rendered, so a half-resolution preview matches the full-resolution export.
- On an adjustment clip, the radii use canvas height.

**Edge cases:**
- Clips smaller than about 64 px tall (tiny PiP sources) clamp the radius to at least 1 px.
- Clip edges use clamp-to-edge sampling, so no dark halo appears at the frame border. Transparent borders of keyed clips are excluded from the blur weights.
- High sharpen on noisy footage amplifies noise. This is expected, and the tooltip warns about it and points to Denoise (CL-033), which runs earlier in the stack and so reduces what sharpening can amplify.
- Soften is not a blur effect. It tops out at a light softening. Strong or shaped blurs are FX effects (FX-015), and the Soften tooltip points there. Face-aware skin smoothing belongs to AI (AI-020).
- Positive Texture and negative Clarity at once is allowed (crisp surface detail with flattened large-scale contrast). The two radii are far enough apart that they do not cancel.
- Performance: if all three values are 0, the pass is skipped. During playback, if the GPU budget is exceeded, the preview may lower preview resolution (EX) but never skips the pass.

**Undo:** `SetColorParam`.

**Model notes:** The fields live in `Finish { sharpen: i16, texture: i16, clarity: i16, ... }`. `freecut-render` provides the blur pass, reused by FX blur effects if FX wants it.

### CL-010 Vignette — v1

**What the user sees:** A **Vignette** slider in Detail & finish. A disclosure arrow reveals Size, Feather and Roundness. The reference app's detail section has a spread-type control near vignette; Size covers it.

**Inputs / controls:**
- **Amount:** −100..+100, default 0. Positive darkens the edges and negative lightens them (FreeCut convention).
- **Size:** 0–100, default 50. Where the falloff begins, measured from the center.
- **Feather:** 0–100, default 50. How soft the falloff is.
- **Roundness:** −100..+100, default 0. 0 is an ellipse that matches the frame's aspect ratio and +100 is a circle. Negative values move toward a rounded rectangle.

**Behavior / output:**
- An elliptical mask centered on the frame, applied after LUT, filter and fade (stage 17).
- Darkening multiplies gamma-encoded values. Lightening uses a screen blend toward white.
- On a clip, the vignette lives in the clip's own frame, so it moves and scales with a PiP. On an adjustment clip it lives in canvas space.

**Edge cases:**
- Feather 0 gives a hard edge. A 1-px antialiasing band is kept to avoid aliasing.
- Size 100 with feather 0 has no visible effect inside the frame.
- Very wide (21:9) or tall (9:16) frames: roundness 0 follows the frame's aspect ratio, so corners darken evenly.
- Off-center vignettes (center x/y) are later.

**Undo:** `SetColorParam` per control.

**Model notes:** The fields live in `Finish { vignette_amount, vignette_size, vignette_feather, vignette_roundness }`.

### CL-011 Film grain — v1

**What the user sees:** A **Grain** slider in Detail & finish, with a disclosure for Size, Color grain and Animated.

**Inputs / controls:**
- **Amount:** 0–100, default 0.
- **Size:** 0–100, default 25. Maps to a grain feature size of roughly 0.05 % to 0.4 % of frame height.
- **Color grain:** toggle, default off (monochrome grain).
- **Animated:** toggle, default on. When off, the grain pattern is frozen.

**Behavior / output:**
- Blue-noise-like grain is generated in the shader, weighted by luma so there is less grain in pure black and pure white.
- The grain is deterministic, following FX's determinism rules ([FX-001](effects-transitions-animations.md)): a stored per-item grain seed is hashed with the source frame index for clips, or with the timeline frame index for adjustment clips. Scrubbing back to a frame shows exactly the same grain, and preview matches export.

**Edge cases:**
- Speed-changed clips (TR) seed by *source* frame, so a 0.5× clip shows each grain pattern twice. That is acceptable, and documented.
- A still image with Animated on shows grain that changes every output frame. With Animated off, the grain is static.
- Duplicating or copying a clip copies the grain settings but generates a fresh seed, so two duplicates show different grain and avoid the obvious "same noise" look. Splitting keeps the seed, so grain continues seamlessly across the cut.
- Export at a different frame rate from the preview gives the same grain per source frame.

**Undo:** `SetColorParam`; toggles use `SetColorFlag { item, flag, old, new }`. The seed is not user-editable; it is restored with the item by TL's insert/remove ops.

**Model notes:** The fields live in `Finish { grain_amount, grain_size, grain_color: bool, grain_animated: bool, grain_seed: u32 }`. The seed is assigned when the item is created or duplicated (the caller supplies it, so the timeline crate stays deterministic and testable). The noise function is in `freecut-render` and is shared with the CPU reference so golden tests can match bit-for-bit within a tolerance.

### CL-012 Fade (matte) — v1

**What the user sees:** A **Fade** slider in Detail & finish that gives a washed-out, matte, lifted-black look.

**Inputs / controls:** **Fade:** 0–100, default 0.

**Behavior / output:** Stage 16. At 100 it raises output black to about 0.15 (encoded), compresses white to about 0.95, and lowers saturation a little (about 10 %). The response is linear in between.

**Edge cases:**
- Fade is a color operation. It is unrelated to clip opacity fades or audio fades (TR/AU). The inspector tooltip says so, to avoid confusion.
- Fade after a strong LUT still lifts blacks, because stage order puts fade after the LUT on purpose.

**Undo:** `SetColorParam`.

**Model notes:** `Finish { fade: u8 }`.

### CL-013 HSL per hue band — v1

**What the user sees:** An HSL section with eight round color swatches: red, orange, yellow, green, cyan, blue, purple, magenta. The selected swatch has three sliders under it: Hue, Saturation, Lightness. Swatches that have been edited show a dot. An eyedropper button lets the user click the preview to pick the band. Reference: public descriptions agree on per-color hue, saturation and lightness sliders plus an on-image color picker in the desktop app. Reported band counts differ (seven or eight), and FreeCut uses eight. The reference app's UI labels suggest the usual eight swatches too, plus up to 10 custom colors picked from the image, which FreeCut schedules as CL-037.

**Inputs / controls:**
- Per band: **Hue** −100..+100 (maps to ±30° shift), **Saturation** −100..+100 (−100 fully removes that band's color), **Lightness** −100..+100 (±50 % luminance change, weighted by band membership). All default to 0.
- Band centers in degrees: red 0, orange 30, yellow 60, green 120, cyan 180, blue 240, purple 270, magenta 300.
- Eyedropper: click the preview to select the band with the highest membership at the sampled 5×5-pixel average.
- Section reset, plus a per-band reset via right-click on the swatch.

**Behavior / output:**
- Each pixel's membership in each band uses a smooth (raised-cosine) falloff between neighboring centers, and the weights sum to 1, so there are no seams.
- Membership is scaled down for low-saturation pixels, reaching zero at gray, so neutral grays never shift hue.
- Lightness changes are weighted by saturation too, which keeps gray walls stable.
- The eyedropper samples the image as it enters the HSL stage (after wheels and curves), because that is what HSL acts on. The renderer provides a one-shot stage readback for this.

**Edge cases:**
- A pixel exactly between two bands is split 50/50. Opposite hue shifts on neighboring bands can then cancel. Documented behavior.
- Picking a gray or near-black pixel (saturation < 5 % or luma < 3 %) gives "No distinct color here" and no band changes.
- Skin usually falls in orange and red, and large hue shifts there look unnatural. Skin protection (CL-030) holds HSL changes off skin when its Color scope is on.
- Hue shifts never wrap more than ±30°. So "make red into blue" is not possible here, and users should use a mask or chroma tools (TR).
- A color that sits between two band centers (a brand teal, say) is split across both bands. Custom targets (CL-037, later) handle it precisely.
- Keyed clips: transparent pixels are excluded from picking.

**Undo:** `SetColorParam` with `ColorParamId::Hsl { band, channel }`. Eyedropper band selection is view state (it only changes which swatch is selected).

**Model notes:** `Hsl { bands: [HslBand; 8] }` and `HslBand { hue: i16, sat: i16, light: i16 }` in `freecut-timeline`. `freecut-render` packs the bands into a uniform array, and the stage readback is used for the picker.

### CL-014 Curves — v1

**What the user sees:** A square curve editor with a faint histogram behind it. Tabs pick **Luma** (master), **Red**, **Green**, **Blue**. Each curve starts as a straight diagonal. An **Auto** button above the editor fits the R, G and B curves for the user. Reference: the reference app's curves have the same four channels behind a channel selector, plus an automatic curve option.

**Inputs / controls:**
- Click on the curve to add a point, and drag a point to move it.
- Double-click a point, or drag it out of the graph, to delete it.
- Endpoints always exist and move freely up and down. Horizontally, the left endpoint stays within x 0–0.25 and the right endpoint within x 0.75–1. Moving an endpoint inward clips everything beyond it, like a levels black or white point.
- Maximum 16 points per curve (FreeCut default). Points keep strictly increasing x with a minimum gap of 0.01.
- Per-curve reset and "Reset all curves".
- Holding Shift while dragging restricts the move to vertical (the UI area owns the binding).
- **Auto:** uses the same analysis source as auto color (CL-019): by default the whole clip (5 frames), or the frame at the playhead from its dropdown.

**Behavior / output:**
- Interpolation is monotone cubic (Fritsch–Carlson), so the curve never overshoots or inverts between points.
- Each curve is baked to a 1024-entry 1D texture.
- Luma applies equally to R, G and B first. Then the R, G and B curves apply per channel.
- Out-of-range values (above 1 from earlier stages) are clamped to [0,1] before the curve lookup.
- **Auto** measures each channel at the curves stage input (after wheels), on downscaled frames. It moves each channel's endpoints to that channel's 0.5 % and 99.5 % percentiles, which stretches contrast and removes a cast in the darks and brights. It then adds one midpoint that moves the channel's median toward the median of the three channels, which neutralizes a midtone cast. The Luma curve is left untouched. The result is three ordinary editable curves.

**Edge cases:**
- Trying to add a 17th point does nothing and shows a tooltip.
- Dragging a point past its neighbor's x is clamped at the neighbor x ± 0.01, so points never swap order.
- A curve set to a flat horizontal line is allowed. It produces a solid level, and that is the user's intent.
- A curve inverted from top-left to bottom-right is allowed. It produces a negative image.
- Curves cannot be keyframed in v1. They are static per item, and the editor says so when keyframes exist elsewhere. Curve keyframing is later.
- Auto on a nearly uniform frame (black, a solid color, a title card) shows "Nothing to correct" and changes nothing. Auto replaces any points already on the R, G and B curves. If any of them has user points, it asks first.
- Auto endpoints are clamped to the endpoint limits above (left x ≤ 0.25, right x ≥ 0.75), so very flat footage is only partly stretched.

**Undo:**
- `SetCurve { item, channel, old: Vec<CurvePoint>, new: Vec<CurvePoint> }`. A drag gesture coalesces into one op.
- Add and delete point are their own `SetCurve` ops.
- Auto is one transaction of three `SetCurve` ops (R, G, B).
- Undo restores the exact old point list.

**Model notes:** `Curves { luma, red, green, blue: Curve }` and `Curve { points: SmallVec<[CurvePoint; 16]> }`, with `CurvePoint { x: u16, y: u16 }` in 0..=10000. Validation (sorted, gaps, count) lives in `freecut-timeline`. Baking and textures live in `freecut-render`, cached by curve content hash. `freecut-render::color::stats::auto_curves(stats) -> [Curve; 3]` is a pure function, tested with synthetic images that have known casts and clipped ranges.

### CL-015 Color wheels — v1

**What the user sees:** Four color wheels side by side: **Shadows**, **Midtones**, **Highlights**, **Offset**. Each has a draggable puck, a luminance slider underneath, and a reset button. A **Strength** slider for the whole section sits above them. Reference: public pages describe shadows, midtones, highlights and offset wheels, plus a separate log-style set. The reference app's UI has the same four wheels, a double-click reset and a log mode switch. FreeCut ships the primary set in v1, and log-range wheels are later (CL-038).

**Inputs / controls:**
- Puck: drag within the disk. Angle sets the hue and distance from the center sets the strength. Double-click resets. Holding a modifier gives fine drag at 1/10 speed (UI owns the binding).
- Luminance per wheel: −100..+100, default 0.
- Strength: 0–100 %, default 100. Scales all four wheels.
- Puck-at-rim maximums (FreeCut defaults): shadows (lift) ±0.10, midtones (gamma) 0.75–1.33, highlights (gain) 0.75–1.25×, offset ±0.10.

**Behavior / output:**
- The standard lift/gamma/gain/offset model.
- Each puck converts to an RGB offset vector with zero net luma, so moving a puck changes hue and not brightness. Luminance sliders change all channels equally.
- Lift affects mostly the darks, gamma the mids and gain the brights. Offset shifts everything.

**Edge cases:**
- All wheels at their rim maximums plus luminance at the extremes must stay finite. Covered by the corner-value property test.
- Dragging the puck outside the disk clamps to the rim.
- Shadows wheel on a clip with crushed blacks: lift raises them uniformly, which is expected.
- Pucks and luminance can be keyframed (CL-028). The puck interpolates in Cartesian space, so it travels in a straight line through the disk.

**Undo:** `SetColorParam` for luminance and strength. `SetWheelPuck { item, wheel, old, new }` for the puck, with one op per drag gesture.

**Model notes:** `ColorWheels { shadows, midtones, highlights, offset: Wheel, strength: u8 }`, with `Wheel { x: i16, y: i16, luma: i16 }`. The invariant x² + y² ≤ 1000² is enforced in the setter. The math lives in `freecut-render`.

### CL-016 LUT import and LUT library — v1

**What the user sees:** The LUT library is browsed in the Adjust tab's **LUTs** group (CL-044): an **Import** button in the toolbar and a grid of the bundled (CL-045) and imported LUTs, each with a thumbnail of the current frame graded by it. Right-click on an imported LUT offers Rename, Show file, and Remove from library. In the Look lens, the LUT section's select lists **None** (which clears the LUT slot of the selected clips, CL-017), then the bundled and imported LUTs, and ends with **Import LUT…**. Reference: public guides report that desktop editors in this class import `.cube` and `.3dl` from an Adjustment/LUT panel and allow batch import of many files at once. The reference app's UI confirms the two formats, a pick-from-imported list and a choice that clears the slot.

**Inputs / controls:**
- Import: a file dialog with multi-select, or drag-and-drop of files or folders onto the panel.
- Accepted formats: `.cube` (1D and 3D) and `.3dl`. `.dat` / `.m3d` / `.csp` are later. ASC CDL files (`.cdl`, `.cc`, `.ccc`) are later too. They would load into the same slot as a small per-pixel slope/offset/power/saturation transform. The reference app's UI hints at CDL support, but this is unconfirmed.
- Library size: no fixed cap (FreeCut default). A single import batch of more than 500 files asks for confirmation first, because each file is parsed and thumbnailed. Alternative: the reference app caps the imported-LUT count and skips the extra files in a batch with a notice.
- Display name: defaults to the cube `TITLE` if present, otherwise the file name. It can be edited, 1–64 characters.

**Behavior / output:**
- Parsing runs on a worker thread, never the UI thread.
- `.cube` handling:
  - Accepts `LUT_3D_SIZE` 2–129 as-is. Sizes 130–256 are resampled to 65³ with a notice.
  - `LUT_1D_SIZE` up to 65536.
  - Honors `DOMAIN_MIN`/`DOMAIN_MAX` (and the `LUT_3D_INPUT_RANGE` variant). Ignores comments and unknown keywords.
  - The full validation rules are listed under *`.cube` and `.3dl` validation* below.
- `.3dl` handling: reads the input mesh line. Infers output bit depth from the largest value (10, 12 or 16 bit), and also accepts float `.3dl`.
- Imported files are **copied** into FreeCut's user LUT library folder (in the app data dir) and identified by a content hash of the file bytes (XXH3-128, the hash the workspace already uses; see Model notes).

**`.cube` and `.3dl` validation (FreeCut rules, 2026-10-13).** Written from Adobe's public Cube LUT 1.0 specification and public `.3dl` descriptions; the parsers are our own.
- *Text.* UTF-8 or Latin-1, an optional UTF-8 BOM, LF or CRLF line ends, trailing whitespace and blank lines anywhere. `#` starts a comment that runs to the end of the line. Numbers use `.` as the decimal mark, with an optional sign and exponent; a file that uses `,` as a decimal mark is rejected with that reason, never guessed. Files over 600 MB are refused before parsing (a 256³ table is about 500 MB of text).
- *`.cube` keywords.* Every keyword line comes before the first data line; a keyword after data is malformed. `TITLE "…"` is optional (quotes required, at most 255 characters, used as the default display name). `LUT_1D_SIZE n` takes 2–65536 and `LUT_3D_SIZE n` takes 2–256; at least one is required. `DOMAIN_MIN r g b` and `DOMAIN_MAX r g b` default to 0 0 0 and 1 1 1; every value must be finite and each maximum must exceed its minimum. The older `LUT_1D_INPUT_RANGE lo hi` and `LUT_3D_INPUT_RANGE lo hi` set the same domain on all three channels; a file that mixes them with `DOMAIN_*` is rejected as unsupported. Other upper-case keywords are skipped and counted, and the import summary mentions them.
- *`.cube` data.* Each data line holds exactly three finite numbers. A 1D table has `n` lines and a 3D table `n³` lines, red varying fastest (index = r + n·g + n²·b). A file with both sizes holds the 1D lines first and is applied as a shaper (1D, then 3D). Fewer or more lines than declared is malformed, with the expected and found counts. Output values may lie outside 0–1 and are kept; the stack clamps only at its end.
- *`.3dl`.* The first non-comment line is the input mesh: 2–65 strictly increasing integers starting at 0. Its length is the cube size `n`, and its last value gives the input bit depth (1023 → 10 bit, 4095 → 12 bit, 65535 → 16 bit; others are rejected). Then `n³` lines of three integers follow with blue varying fastest and red slowest (the reverse of `.cube`). The output scale comes from the largest value in the file (≤ 1023 → 10 bit, ≤ 4095 → 12 bit, else 16 bit). A file whose data contains a decimal point is a float `.3dl`, with outputs taken as 0–1. The mesh values are the lattice's input positions: a mesh within one code value of even spacing (the usual 0, 64, …, 960, 1023) is treated as even, and any other mesh is resampled onto an even lattice of the same size, with a notice.
- *Errors.* Every rejection names the line number and one reason ("line 12: expected 3 numbers, found 2", "35 937 data lines expected, 35 936 found"). Malformed and unsupported are reported separately, as above.
- *Identity.* A table that maps every lattice point to itself within 1/65535 imports normally, and its library entry says "No visible change".
- *Domain at render time.* The input is clamped to the domain, then mapped to 0–1 per channel before the lookup. A 1D table interpolates linearly per channel. Projects reference the hash plus the original file name (PM handles project-level collection and relink).
- Re-importing an identical file reuses the existing entry, and the dialog reports "Already in library".
- Thumbnails render lazily, as in CL-003.

**Edge cases:**
- A file with any other extension is rejected before parsing, with the hint "FreeCut can import .cube and .3dl LUT files". In a dropped folder, non-LUT files are skipped silently and only counted in the summary.
- Malformed files (wrong number of rows, non-numeric values, NaN/Inf, size outside 2–256, missing size keyword) are rejected with a one-line reason and the line number. In a multi-file import, the good files still import and a summary lists the failures.
- A `.cube` or `.3dl` file that parses but uses a feature FreeCut does not support (for example an unknown required keyword, or a `.3dl` mesh line that doesn't match the data) is rejected as "unsupported LUT" with the reason, not as malformed.
- A file with Windows line endings, a BOM, or trailing whitespace parses fine.
- A 1D `.cube` is applied per channel, the same as a curve.
- A file with both 1D and 3D sections applies the 1D shaper first, then the 3D table.
- LUTs meant for log footage applied to Rec.709 footage look wrong (washed out or overly contrasty). This is a user choice. The panel hint suggests the input slot (CL-032), which comes later.
- Removing a LUT from the library while projects still use it: projects keep rendering if the hashed copy exists. "Remove" only hides the entry, and deletion is a separate "Delete file" action with confirmation. If the file is truly missing, items show a missing-LUT badge, the stage renders as identity, and a Relink action opens a file dialog that checks the hash.
- Very large libraries (500+ LUTs) need a virtualized list. Thumbnails only render for visible rows.

**Undo:** Library management is app-level, not project state, so it is not on the project undo stack. Destructive library actions ask for confirmation.

**Model notes:**
- `freecut-media::lut` holds `parse_cube(&[u8]) -> Result<Lut, LutError>` and `parse_3dl(&[u8]) -> Result<Lut, LutError>`. These are pure functions with unit tests, and the module builds without the crate's `ffmpeg` feature, so `freecut-render` can use it with no FFmpeg. File reading runs on worker threads.
- `Lut { title: Option<String>, shaper: Option<Lut1d>, cube: Option<Lut3d>, domain_min: [f32; 3], domain_max: [f32; 3], skipped_keywords: u32, notices: Vec<LutNotice> }`, with `Lut1d { size, data: Vec<[f32; 3]> }` and `Lut3d { size, data: Vec<[f32; 3]> }` (red fastest; `.3dl` is reordered on parse). A 1D-only `.cube` is held in `shaper`. `LutNotice::{Resampled { from, to }, MeshResampled { size }}` carries the two resampling notices above to the import summary. `Lut::apply_tetrahedral`, `Lut::apply_trilinear`, `Lut3d::sample_*`, `Lut1d::sample` and `Lut::is_identity` are the shared `f32` sampling (domain clamp and normalize, shaper, then 3D table), and `Lut3d::from_fn` builds a table from a function (FreeCut's own bundled looks). `LutError { line: Option<u32>, kind: LutErrorKind }` uses `thiserror`, with `LutErrorKind::{Malformed(Malformed), Unsupported(Unsupported), TooLarge, DecimalComma}`; `Malformed` and `Unsupported` have one variant per validation rule, so callers match on the rule and the app words it.
- Input-range keywords in a shaper file (FreeCut rule, 2026-10-14): `LUT_1D_INPUT_RANGE` sets the file's domain; a `LUT_3D_INPUT_RANGE` other than `0 1` after a shaper is unsupported, because the 3D table would need a second domain. An input-range keyword for a table the file does not have is malformed. In a 3D-only file `LUT_3D_INPUT_RANGE` sets the domain. A short table is blamed on its size keyword's line, a surplus one on the first extra data line, and a `.3dl` mesh mismatch on the mesh line.
- `LutRef { hash: [u8; 16], file_name: String }` is a plain value type defined in `freecut-timeline` (no I/O), because `LutSlot` in the color stack holds it and the timeline crate depends on nothing internal. `freecut-project` serializes it (the hash as 32 hex digits), and PM's collect/relink resolves it to a file. The hash is XXH3-128 of the file bytes (FreeCut default, 2026-10-13): the workspace already ships `xxhash-rust`, and the hash only has to tell the user's own files apart. Alternative: SHA-256, which an earlier draft named; it needs a new dependency and buys collision resistance against attackers, which a local library does not need.
- `freecut-project::luts` holds the library index (`luts/index.json` in the app data dir: hash, display name, original file name, kind, size, import date) and its read and write functions, called on worker threads. The library is app state, not project state.
- `freecut-render` keeps 3D LUTs as `f32` lattices on the CPU path (C52) and will upload them as `rgba16float` 3D textures on the GPU path; both are cached and shared by hash.

### CL-017 Apply LUT with intensity — v1

**What the user sees:** Clicking a LUT in the library (CL-044) applies it to the selected clips. The LUT section on the clip's Basic page (CL-043) then shows a **LUT** select with the LUT's name, an **Intensity** slider, the **Skin protection** switch and amount (CL-030), and under More the interpolation; the section's reset badge and on/off switch sit in its header. Choosing **None** in the select removes the LUT. The "+" button on a library entry creates an adjustment clip carrying that LUT (CL-006). Reference: public guides recommend this layer route, with a conversion LUT on each clip and a creative LUT on a layer above.

**Inputs / controls:**
- Intensity: 0–100 %, default 100, keyframable. FreeCut matches the reported cap of 100 and does not extrapolate. The reference app's UI labels show no LUT intensity, but public guides report one, so FreeCut keeps it.
- Remove: the "×" button or the library's **None** entry clears the slot. Section reset (CL-024) keeps the LUT and sets intensity and interpolation back to their defaults. The reference app has a single restore action whose exact scope is unclear. FreeCut keeps the two separate so that a reset never drops a LUT choice.
- Interpolation (advanced disclosure): Tetrahedral (default) or Trilinear.

**Behavior / output:**
- Stage 13 output = lerp(input, LUT(input), intensity / 100).
- Input is clamped to the LUT domain and then normalized.
- LUTs are assumed to expect Rec.709 gamma-encoded input (FreeCut default), which suits most creative LUTs.
- There is one user LUT slot per stack. More LUTs can be layered with adjustment clips.
- CPU path (C52): the LUT is sampled inside the stack's bake like every other per-pixel stage. When the LUT section is the only active one (no skin protection, Amount 100), the bake is skipped and the user's own table is applied directly with the chosen interpolation, so the file's precision is used as is.

**Edge cases:**
- Applying a LUT to a clip that already has one replaces it and keeps the intensity.
- A LUT that is missing at render time renders as identity, with a warning badge on the clip and in the export preflight (EX). Export is not blocked, but the preflight lists the affected items.
- A 2³ LUT is valid and coarse, so it renders blocky gradients. That is the user's choice.
- Intensity 0 keeps the LUT reference but makes it invisible.

**Undo:** `SetLut { item, old: Option<LutSlot>, new: Option<LutSlot> }`. Intensity uses `SetColorParam`.

**Model notes:** `LutSlot { lut: LutRef, intensity: u8, interp: LutInterp, skin: SkinProtect }` (`LutInterp::{Tetrahedral, Trilinear}`; the name avoids the keyframe `Interp`). On the GPU path, tetrahedral interpolation is done manually in WGSL with `textureLoad` and trilinear uses hardware filtering. On the CPU path both are plain `f32` code shared with the bake.

### CL-018 White-balance picker — v1

**What the user sees:** An eyedropper next to Temperature and Tint. The user clicks something in the preview that should be neutral (a white wall or a gray card), and the cast disappears.

**Inputs / controls:** Click on the preview. It samples a 5×5 average at preview resolution. Esc cancels pick mode.

**Behavior / output:**
- Samples the image as it enters the white-balance stage (after deflicker, denoise and any color-match transform), solves for the temperature and tint that make the sample neutral, and writes those two sliders.
- Other sliders are untouched, and the result can be edited.

**Edge cases:**
- Samples that are clipped (any channel ≥ 0.98) or too dark (luma < 0.05) are rejected with a short reason.
- If the solution needs more than ±100, the sliders clamp and a notice says the cast is stronger than the controls can fully remove.
- Picking on an adjustment clip samples the composite below it.
- Clicking outside the item's visible area (for example the letterbox) gives "Pick inside the selected clip".

**Undo:** One `SetColorParams { item, changes: [(Temperature, old, new), (Tint, old, new)] }` transaction.

**Model notes:** The solver is a pure function in `freecut-render::color::stats` (`solve_white_balance(rgb_sample) -> (i16, i16)`) with unit tests. The readback goes through the render thread.

### CL-019 Auto color — v1

**What the user sees:** An **Auto** section at the top of the Basic page (CL-043). Before any run its body is one **Auto** button with a dropdown. One click gives a neutral, well-exposed starting point. After a run the section lists what the correction changes as read-only rows ("Exposure +12", "Temperature −8", "Blacks −4"), with a **Strength** slider, **Re-run** and **Apply to sliders**. The header carries the usual on/off switch and reset badge. With several clips selected the button reads **Auto (N clips)**, and the Color tab menu offers **Auto color all clips…**. Reference: public pages describe a one-click auto adjust/AI correction in the desktop app's adjustment panel. The reference app's UI also lets the user change the result's intensity and style afterwards, keeps the automatic result apart from manual changes so it can be removed on its own, and has a project-wide pass that improves the colors of every clip its assistant flagged. Its automatic result and manual adjustments replace each other. FreeCut's version uses plain statistics, not a model, and stacks the automatic result under the user's own sliders instead of replacing them.

**Inputs / controls:**
- The Auto button (Re-run after the first run). Its dropdown offers "Analyze frame at playhead" or "Analyze whole clip" (default: whole clip, using 5 frames evenly spaced across the used range, so the result doesn't depend on a single frame), and the **Style**: Natural (default) corrects only; Vivid also adds Vibrance +20 and Saturation +5 (FreeCut defaults). The source and Style are remembered as UI preferences and stored with the result.
- **Strength:** 0–100 %, default 100, keyframable (CL-028). It scales the stored offsets when the stack is evaluated, so it can be changed or animated at any time without re-running.
- **Apply to sliders:** adds the offsets at the current Strength into the Light and Color sliders (each clamped to its range) and removes the correction, as one entry. The user then sees and edits plain slider values, which was this feature's only behavior in an earlier draft.
- On/off switch: off keeps the correction but does not apply it. Reset badge: removes the correction and any Strength keyframes (CL-024).
- Multi-clip runs (**Auto (N clips)**, or **Auto color all clips…** with scope All visual clips or Clips on this track) add **Only clips that need it** (default on). It skips clips whose computed offsets are within ±5 on every slider.

**Behavior / output:**
- Analysis computes per-channel percentiles (0.5 % / 99.5 %) and gray-world means on frames downscaled to at most 512 px wide, at the stack *input*: after the cleanup stages and the color match (stage 4), before any slider. Neither the user's sliders nor an earlier Auto result are part of what is measured, so re-running never compounds.
- It solves for offsets of Temperature, Tint, Exposure, Whites, Blacks and Contrast (plus Vibrance and Saturation with Vivid) that put the black and white points near 0.02 and 0.98 and the average near neutral, each clamped to its slider range.
- The offsets are stored, not written into the sliders. When the stack is evaluated, each affected slider's value is `clamp(manual + offset × strength / 100)`. So a manual Exposure of +10 over an Auto offset of +12 shows +22, and turning Auto off leaves the manual +10.
- The result is static for the clip and not computed per frame, which avoids flicker.
- Analysis runs on a worker thread, and a spinner shows on the button. It decodes in exact mode from the original media when online, otherwise from the proxy, and sums in `f64` in a fixed pixel order, so the same clip gives the same offsets on every run and every OS.
- Multi-clip runs analyze each clip on its own, as one background job in the task list (PM-073) with per-clip progress. The user can keep editing. When the job finishes, every result is committed in one transaction, and a summary lists clips that were changed, skipped as already fine, or skipped with a reason.

**Edge cases:**
- Nearly uniform frames (a black screen, a solid color, a title card) give "Nothing to correct", and nothing changes.
- Intentionally stylized footage (a sunset, stage lighting) may be "corrected" away. That is expected: switch the section off, lower Strength, or undo. The tooltip mentions it.
- A clip with an existing filter or LUT: analysis runs on the stack input, so the look still sits on top of the correction.
- Analyzing a very long clip still samples only 5 frames, so it is bounded in time.
- Re-run replaces the stored offsets; keyframes on Strength are kept.
- Cancel: clicking the spinner cancels, and no op is committed. Cancelling a multi-clip job commits nothing either.
- In a multi-clip job, a clip that is deleted, moved to a locked track or color-edited while the job runs is skipped at commit and listed, so the job never overwrites a newer edit. Locked-track clips are skipped from the start and counted. Text and adjustment clips are never included.
- Auto results are footage-specific, so saved looks (CL-026) and Apply to all (CL-025) leave them out; Paste attributes copies them with the Basic category.
- A slider already at +100 with a positive offset stays at +100; the row in the Auto section shows the offset it would like and a "limited" hint.

**Undo:** A run is `SetAuto { item, old: Option<AutoCorrection>, new }`; a multi-clip run is one transaction holding one `SetAuto` per changed clip. Strength is `SetColorParam` (`ColorParamId::AutoStrength`). **Apply to sliders** is one entry made of a `SetColorParams` and a `SetAuto(None)`. Reset is `SetAuto(None)` plus the removal of Strength keys, one entry.

**Model notes:** `AutoCorrection { offsets: AutoOffsets, style: AutoStyle, source: AutoSource, strength: u8 }` with `AutoOffsets { exposure, contrast, whites, blacks, temperature, tint, vibrance, saturation: i16 }` in `freecut-timeline`, stored as `ColorStack.auto: Option<AutoCorrection>`. `ColorStack::effective_basic()` returns the slider values with the offsets applied; the renderer only ever sees those. `freecut-render::color::stats::auto_color(&[FrameStats], AutoStyle) -> Result<AutoOffsets, ColorStatsError>` is pure and tested with synthetic images that have known casts. Frame decode comes from `freecut-media` on a worker. The app runs the job (`freecut-app::jobs::color`) and commits the op, and runs the multi-clip job through PM-073's task list.

### CL-020 Color match to a reference — v1

**What the user sees:**
- With a clip selected, the user clicks **Match color…** in the **Match** section of the Basic page (CL-043), which enters pick mode.
- They click any frame of another clip on the timeline (the timeline shows a frame scrubber on hover), or choose "Use image file…".
- The selected clip's colors shift to resemble the reference.
- The Match section then shows the reference's thumbnail and name, an **Intensity** slider (0–100 %, default 100, keyframable), **Re-match** and **Change reference…**. Its header switch turns the match off without losing it, and its reset badge removes it.
- While the analysis runs, the Match section shows a progress bar with Cancel. The result appears when it finishes.
- Reference: public guides describe picking a reference frame on the timeline and then adjusting an intensity. They note results are close rather than identical. The reference app's UI shows the same flow: choose a timeline frame or an imported still, apply, wait through a progress state, see the result. Matching many clips to one reference is CL-039.

**Inputs / controls:** Reference frame (timeline frame or image), intensity, re-match, remove. In pick mode, a small bar shows the chosen reference's thumbnail with **Match** and **Cancel**. **Match** stays disabled until a reference frame or image has been chosen. Any local frame or image can be the reference. There are no restricted library items.

**Behavior / output:**
- Computes the mean and standard deviation of the reference frame and the target clip (5 frames sampled across the used range, measured at the target's stack input, CL-019) in the Oklab perceptual space.
- Deterministic (FreeCut rule): frames are decoded in exact mode and downscaled to at most 512 px wide, statistics are summed in `f64` in a fixed pixel order over pixels with alpha above one half, and the result is rounded to the stored fixed-point values, so the same inputs give the same transform on every run and OS.
- Stores a per-channel gain and offset (L, a, b) transform that moves the target statistics onto the reference.
- Applies the transform at stage 4. Output = lerp(input, matched, intensity).
- The rest of the stack works on top as normal.
- The reference is recorded (item id and source time, or the image's media ref) so "Re-match" can re-run the computation after the source changes.

**Edge cases:**
- A reference clip that is later deleted keeps the stored transform. Re-match is disabled and the tooltip explains why.
- A very different reference (a night shot matched to day) gives extreme gains. They are limited to 0.5–2× in L and ±0.1 in a/b, with a notice.
- A target frame that is black or uniform gives "Not enough detail to match".
- Matching a clip to itself is allowed and gives an identity transform.
- The reference's own color stack: by default the match uses the reference *as graded* (its output), which matches what the user sees. A checkbox offers "use ungraded source" instead.
- Cancelling during analysis commits nothing. Pressing Esc in pick mode leaves it without changes.

**Undo:** `SetMatch { item, old: Option<MatchTransform>, new: Option<MatchTransform> }`, labeled "Match color: <clip name>" in the undo menu and history (TL-043, TL-044). Intensity uses `SetColorParam` (`ColorParamId::MatchIntensity`). The section switch is `SetSectionEnabled` (CL-023).

**Model notes:**
- `MatchTransform { gain: [i32; 3], offset: [i32; 3] (fixed-point ×10⁴), intensity: u8, reference: MatchRef, group: Option<MatchGroupId> }`. It lives in `freecut-timeline`. `group` is CL-039's and is `None` for a single match. `MatchRef` is either (item id, source time as a rational tick value per TL) or a PM media reference for an image file.
- `freecut-render::color::stats::{oklab_stats(&Picture) -> OklabStats, match_stats(target: &[OklabStats], reference: &OklabStats) -> Result<MatchGains, ColorStatsError>}` are pure; the app wraps the gains with the reference and intensity into a `MatchTransform`. The transform is a per-pixel stage, so on the CPU path it is part of the bake (C52).
- Uses Reinhard-style statistics transfer from published literature.

### CL-021 Histogram — v1

**What the user sees:** A dockable **Scopes** panel (UI owns docking, UI-056) showing a histogram of the current program frame. The **Scopes** button in the Color tab header opens it next to the inspector, so the scope is visible while grading. Reference: the reference app also offers a scope while grading. Its labels suggest a waveform-type view, which FreeCut schedules as CL-022. Modes are Luma, RGB overlay, and RGB per channel (three stacked histograms; not to be confused with the waveform parade in CL-022). Small triangles in the corners light up when more than 0.5 % of pixels are clipped at 0 or 1.

**Inputs / controls:** Mode selector, log/linear vertical scale toggle (default linear), and source selector: Program output (default), or Selected item (stack output only, before compositing). Selected item is later.

**Behavior / output:**
- Computed on the GPU (compute shader with atomic bins, 256 bins per channel) from the preview frame, downscaled to at most 512 px wide.
- Updates on every displayed frame while paused, and at most 15 Hz during playback (FreeCut default).
- The curve editor (CL-014) shows the same luma histogram behind the curve.

**Edge cases:**
- Panel hidden means no computation runs.
- Preview at reduced resolution or with proxies gives an approximate histogram. A label shows "approx." when proxies are active.
- Empty timeline or a gap shows a flat histogram at black.
- HDR sources show the post-tone-map histogram, since that is what will be exported.

**Undo:** n/a (view state).

**Model notes:** `freecut-render::scopes::histogram` (compute) returns `HistogramData` and `freecut-app` draws it with egui painter. Nothing is stored in the model. Visibility and mode are UI preferences.

### CL-022 Waveform, RGB parade and vectorscope — later

**What the user sees:** Extra modes in the Scopes panel:
- **Waveform:** luma by image column, 0–100 scale with 10-step graticule.
- **RGB parade:** three side-by-side channel waveforms.
- **Vectorscope:** hue and saturation on a circle, with 75 % color targets and a skin-tone line.

**Inputs / controls:** Mode, brightness/gain of the trace, graticule on/off, and Rec.709 luma weights (fixed).

**Behavior / output:** GPU compute into an accumulation texture from the downscaled program frame, with the same update policy as CL-021.

**Edge cases:**
- 9:16 frames: the waveform x-axis maps to frame width, so it is narrower in detail. That is accepted.
- Vectorscope on monochrome footage shows a dot at the center.
- Interlaced or odd-size frames: scopes read the deinterlaced preview frame (FX/EX decide deinterlacing).

**Undo:** n/a (view state).

**Model notes:** `freecut-render::scopes::{waveform, vectorscope}`. Reference: free editors such as Kdenlive and DaVinci Resolve provide these scopes, and they are what make grading by numbers possible.

### CL-023 Bypass and before/after compare — v1

**What the user sees:**
- Every color section header has an on/off switch (CL-043), and the Color tab header has a master switch that bypasses the whole stack for the selected item.
- A **Compare** button in the preview toolbar adds a draggable vertical split line. The left side shows "before" (the item without its color stack, or for an adjustment clip, the composite without that clip) and the right side shows "after".
- A keyboard shortcut (UI area) toggles a full-frame before/after.

**Inputs / controls:** Per-section toggles, master toggle, compare split position (0–100 % of width, default 50), and the compare shortcut.

**Behavior / output:**
- Section and master toggles are **persisted** in the project. They change the output and are exported as set.
- The compare split is **view state** only. It affects the preview and never the export.
- A preview-only "Disable all color" switch in preview quality settings (EX) is a performance aid. It is ignored on export and shows a persistent warning badge while active.

**Edge cases:**
- Compare with nothing selected compares the whole program with and without *all* color stacks and adjustment clips.
- Compare during playback works, at the cost of rendering two paths. On a slow GPU the preview may drop to a lower resolution automatically (EX).
- A bypassed section's sliders stay editable, and edits are visible once it is re-enabled. The section shows a hint "Section is off".

**Undo:** Section and master toggles: `SetSectionEnabled { item, section, old, new }`. Compare: n/a (view state).

**Model notes:** `SectionMask` bitflags in `ColorStack`. The compare split is held by `freecut-app` and passed to `freecut-render` as a render option.

### CL-024 Reset — v1

**What the user sees:** Three levels of reset:
- **Per control:** double-click the slider label, or click its reset arrow.
- **Per section:** the reset badge in each section header (CL-043), or **Reset section** in its `...` menu.
- **Whole stack:** "Reset all color" in the Color tab's menu. It asks for confirmation when more than one section is non-default, or when the stack has color masks (CL-031), which it also removes. It also sets Amount back to 100 % (CL-035).

**Inputs / controls:** As above. Reset in the Filter section sets intensity back to 100. Removing the filter itself is the separate "None"/"×" action.

**Behavior / output:** Restores defaults from the `ParamSpec` table. Resetting a section also removes that section's keyframes for the item (with confirmation if keyframes exist).

**Edge cases:**
- Reset on a locked item is disabled.
- Reset on a multi-selection applies to every selected item in one transaction.
- Resetting a section that is already at defaults is a no-op and does not create an undo entry.
- Reset does not touch the LUT library or user presets.

**Undo:**
- `ResetColorSection { item, section, old: SectionSnapshot }` restores the snapshot, including keyframes.
- `ResetColorStack { item, old: ColorStack + keyframes }`.
- A single-control reset is a `SetColorParam`.

**Model notes:** `SectionSnapshot` is an enum over the section structs plus their keyframe tracks (from TR). Snapshots are taken by value.

### CL-025 Apply to all, and copy/paste color — v1

**What the user sees:**
- Each color section, plus the Filter and LUT sections, has an **Apply to all** button.
- A dialog lists the scope: All visual clips (default), Clips on this track, or Selected clips. It shows the count of affected clips, a count of skipped locked-track clips, and a checkbox "Also overwrite clips that already have settings in this section" (default on).
- Copy/paste of color between clips goes through **Paste attributes…** (`Mod+Alt+V`, that is Ctrl+Alt+V or Cmd+Option+V; UI owns the binding). TL ([TL-034](timeline-editing.md)) owns that command, its checklist dialog, the attribute clipboard and the `PropBag::category()` registry. CL registers each Color-tab section from CL-001 as its own category in that registry: Filter, Basic, HSL, Curves, Wheels and LUT. Basic carries color match, denoise, deflicker, dehaze, the overall Amount and the color position (CL-042). Once CL-031 ships, **Color masks** becomes a further category, unticked by default. Right-click **Copy color** / **Paste color** are shortcuts into the same TL-034 flow with only the color categories ticked.
- Reference: public guides describe an apply-to-all action for color settings and a copy/paste of effects and adjustments between clips in the desktop app. The reference app's UI has apply-to-all for both filters and adjustments, and leaves masks out of it with a notice.

**Inputs / controls:** Scope, overwrite toggle, and, for paste, TL-034's category checklist (its UI and defaults are TL-034's; CL supplies the color categories).

**Behavior / output:**
- Copies the source item's section values *by value* onto the targets. Targets keep independent copies, with no live link.
- Keyframes in copied sections are placed using TL-034's keyframe timing option (rescale to the target's duration, or keep absolute offsets), the same rule TR-034 applies to its own categories. Apply to all uses the same placement with TL-034's default.
- Adjustment clips are excluded from "All visual clips" unless the source is itself an adjustment clip, in which case the scope means all adjustment clips.
- Copied attributes live in TL-034's attribute clipboard. CL defines no clipboard of its own.

**Edge cases:**
- Clips on locked tracks are skipped and counted in the result toast.
- Pasting a LUT or filter that is missing on this machine copies the reference anyway and shows the missing badge.
- Applying to 1,000+ clips is one op, and the UI stays responsive because it is a model-only change. Re-compiling stacks for render happens lazily.
- Color-match transforms (CL-020) are included in copies, and their reference is kept.
- If the selection includes text items, they are skipped since text has no stack.
- Applying "to all" from a clip whose section is at defaults works as a mass reset of that section, and the dialog says so.
- Pasting a Basic section with deflicker on onto a clip whose media has no cached analysis queues the analysis job (CL-034) after the paste commits.
- Deflicker or temporal-denoise values that land on an image, freeze-frame still or adjustment clip are kept but inactive (CL-033, CL-034), and the paste summary lists them.
- Color masks (CL-031) are never copied by Apply to all, because a mask drawn for one shot rarely fits another. The result toast says "Color masks were not copied". They are copied only through Paste attributes when the user ticks the Color masks category.
- An HSL section with custom targets (CL-037) copies the targets with it. They are part of the HSL section.

**Undo:** One `ReplaceColorSections { targets: Vec<(ItemId, SectionSnapshot)>, sections, new }` transaction. Undo restores each target's own snapshot. When pasted through TL-034 together with other categories, it is one part of TL-034's single compound op.

**Model notes:** The op lives in `freecut-timeline`. CL registers its color sections through TL-034's `PropBag::category()` registry, each category carrying an `Option<SectionSnapshot>`. There is no separate attributes struct, and CL defines no clipboard of its own. The attribute clipboard and its lifetime are TL-034's.

### CL-026 Save as look preset (user looks) — v1

**What the user sees:**
- **Save look…** in the Color tab menu or footer (CL-043) opens a dialog (UI-064) with:
  - a name field;
  - a section checklist, pre-ticked with the sections that are non-default;
  - a live thumbnail.
- Saved looks appear in the Adjust tab's **Saved** group (CL-044) and in the **Presets** select at the top of the Color tab.
- Applying a user look copies its section values into the item. The values can be edited, which is different from a bundled filter's single intensity.
- Each user look can be renamed, duplicated or deleted from its context menu.
- A look saved with the Filter section ticked together with other sections is FreeCut's combined look: one click applies a filter and its finishing adjustments together.
- Reference: public pages mention that HSL and wheel settings can be stored and recalled later in other projects. The reference app's UI lets the user save the adjustment stack as a personal preset, lists those presets with an empty-state placeholder, and allows deleting them. Its team brand-kit list of presets is a paid, account-bound feature. FreeCut's local equivalent is this library plus look files (CL-027).

**Inputs / controls:** Name (1–64 characters; a duplicate gets an auto " (2)" suffix), included sections, and optional tags. Alternative: the reference app rejects duplicate names with a warning, and rejects names containing `\ : / * ? < > |`. FreeCut allows any Unicode name and auto-suffixes duplicates, because the file name is never the display name.

**Behavior / output:**
- Stored as one JSON file per look (`<uuid>.fclook`) in the user presets folder (app config dir). Looks are global across projects.
- The file holds a format version, name, tags, created date, the included sections, and LUT references (hash and file name).
- Applying a look replaces only the included sections on the target, and is undoable.
- Deleting a look never changes projects that already used it, because values were copied.
- Deleting asks for confirmation and moves the `.fclook` file to the operating system's trash, so it can be recovered from there. Where no trash is available, the dialog says the deletion is permanent. Alternative: the reference app deletes presets permanently, bypassing any trash, after a warning.
- No count limit (FreeCut default). The list is virtualized, so hundreds of looks stay responsive. The reference app's paid brand-kit tier caps adjustment presets at 200. Nothing in FreeCut is capped.

**Edge cases:**
- A look that references a LUT missing from the library shows a warning badge. Applying it applies everything else and marks the LUT slot missing.
- A corrupt or unknown-version `.fclook` file is listed as unavailable with the reason. It is never silently deleted. A file from a newer FreeCut reads "Made with a newer FreeCut (look format N)". There is no update prompt, and FreeCut never gates a feature behind an update. The reference app asks the user to update the app in this case.
- A file named with illegal characters: the file name is always the uuid, so the display name can contain any Unicode.
- Saving a look with no non-default sections is refused with "Nothing to save — all color settings are at defaults".
- Keyframes are **not** saved in looks. The values at the playhead are saved instead, and the dialog says so.
- Denoise, deflicker and dehaze (CL-033, CL-034, CL-036) are never saved in a look, because they repair one particular piece of footage rather than define a look. The dialog notes that they were left out.
- Color masks (CL-031), Auto corrections and color matches (CL-019, CL-020), the overall Amount (CL-035) and the color position (CL-042) are not saved either. Auto and match results describe one piece of footage, like denoise. Masks belong to one shot's framing, and Amount is how much of a look an item uses. The dialog notes any masks it left out, as the reference app does.

**Undo:** Saving, renaming and deleting looks are app-level library actions, not project undo. Applying a look is `ReplaceColorSections` (same op as CL-025).

**Model notes:** `freecut-project::look` holds `LookFile { version: u32, name, tags, sections: Vec<SectionValue>, luts: Vec<LutRef> }`, with migrations like the project format. Reading and writing go through `freecut-app` worker threads.

### CL-027 Share looks and export a grade as LUT — later

**What the user sees:** **Export look…** writes a `.fclook` file that a friend can import. With several looks selected in the Adjust tab's Saved group (CL-044), **Export looks…** writes one `.fclooks` bundle, and **Import looks…** reads either kind. A team shares a house style this way, the local counterpart of a cloud brand kit (see Out of scope). **Export as .cube…** bakes the selected item's per-pixel color stages into a 33³ (or 65³) `.cube` LUT for use in other software.

**Inputs / controls:** Export path and LUT size (17, 33 or 65; default 33). For bundles: the looks to include, and **Include LUT files** (default on), which copies the user LUTs the looks reference into the bundle.

**Behavior / output:**
- LUT export bakes stages 4–14 and 16 (it omits the input transform, deflicker, denoise, clarity, sharpen, vignette and grain, which are footage-specific, temporal, spatial or random) with domain 0–1 and Rec.709 encoded input.
- The `TITLE` is set to the look name.

**Edge cases:**
- Spatial stages are listed in the export dialog as "not included".
- Keyframed values are baked at the playhead.
- Importing a `.fclook` whose LUT hash is unknown prompts the user to locate the LUT file. LUTs carried in a bundle go into the LUT library (CL-016), and identical ones are reused by hash.
- Importing a look that is byte-identical to one already in the library skips it. A look with the same name but different content is added with the " (2)" suffix (CL-026).
- A bundle with some unreadable entries imports the rest and lists the failures.
- The user is responsible for the license of any LUT they share. The export dialog says so when Include LUT files is on.

**Undo:** n/a (file export and import are not timeline edits).

**Model notes:** The bake uses the `freecut-render` CPU reference stack. The writer is in `freecut-media::lut`. A `.fclooks` bundle is a zip of `.fclook` files plus a `luts/` folder named by hash, written and read by `freecut-project::look` on worker threads.

### CL-028 Keyframed color parameters — v1

**What the user sees:** Each keyframable color control has a keyframe diamond beside it (TR's standard UI, [TR-027](clip-transform-and-speed.md) and TR-028). Users can animate exposure, temperature, filter or LUT intensity, the overall Amount (Mix on adjustment clips), and so on over time. Reference: public guides describe keyframing effect parameters such as color and filters in the desktop editor.

**Inputs / controls:**
- Keyframable: every scalar slider in Basic, Detail & finish, and HSL (per band and channel), including Denoise amount and Detail, Hue, Pivot and Texture; wheel pucks (x/y) and luminance; wheel strength; filter intensity; LUT intensity; match intensity; and the overall Amount (CL-035), which is Mix on adjustment clips. Later features add their sliders: Dehaze (CL-036), custom-target H/S/L (CL-037), log-wheel pucks and luminance (CL-038) and look parameters (CL-040).
- Not keyframable in v1: curve point sets, LUT or filter choice (discrete), toggles, HSL band selection, denoise temporal frames, every deflicker setting (its strength and window shape a multi-frame analysis, so they stay static per clip), Auto's Style and Strength (they are run settings, not stored values), custom-target centers and ranges, the log-wheel range boundaries, and the color position.

**Behavior / output:**
- TR owns the keyframe model, interpolation and easing. This area only registers `ColorParamId`s with their ranges.
- TR-027 interpolates CL's fixed-point values in `f64` and rounds the result to the parameter's step when evaluated. Keyframe values are stored as integers.

**Edge cases:**
- A keyframed param edited while the playhead is between keyframes adds a keyframe, following TR's auto-key rules.
- Resetting a keyframed control (CL-024) clears its keyframes after confirmation.
- Speed-ramped clips: keyframes stay in clip time (TR rules), so a grade animated on content stays in sync with it.
- Copy/paste and apply-to-all copy keyframes (CL-025), but saving a look does not (CL-026).

**Undo:** TR's keyframe ops (`AddKeyframe`, `MoveKeyframe`, `SetKeyframeValue`, `RemoveKeyframe`) applied to `ColorParamId` targets.

**Model notes:** Each keyframable field is an `Animatable<T>` (TR-027) over CL's fixed-point types, which implement TR's `Interpolate`. `ColorParamId` names each field so ops and TR's keyframe UI can address it, and the `ParamSpec` table supplies its name, range and default. `freecut-render` takes a per-frame evaluated `ColorStackValues` snapshot.

**FreeCut keyframes in code (2026-10-13, README C52).** Color keys use the clip's existing `ClipKeys` (TR-027 as built): `PropPath::Color(ColorParamId)` with the value kind `ValueKind::Level` and `PropValue::Level(i16)` for every scalar (the range comes from `ParamSpec`; a key outside it is clamped on load), and `PropPath::ColorPuck(WheelId)` with `PropValue::Puck { x: i16, y: i16 }` for wheel pucks (interpolated per coordinate, then pulled back onto the disk). Mask parameters use `PropPath::ColorMask { id, param }` (CL-031). Interpolation runs in `f64` from the exact rational progress and rounds half away from zero to the step when evaluated. `Clip::color_at(content_t) -> Option<Arc<ColorStack>>` returns the stack with every keyed field evaluated, the same `Arc` when the clip has no color keys; `frame_plan` puts that in the plan, so `ColorStackValues` is simply an evaluated `ColorStack`. Keyframe edits go through the existing `EditRequest::EditKeys`, which accepts these paths where it accepts transform paths.

### CL-029 Secondary hue and luma curves — later

**What the user sees:** Extra curve tabs: **Hue vs Hue**, **Hue vs Saturation**, **Hue vs Luma**, **Luma vs Saturation**, **Saturation vs Saturation**. Each shows a flat line across a horizontal axis (hue around the rainbow, or luma or saturation from 0 to 1), with points that push the vertical value up or down.

**Inputs / controls:** The same point editing as CL-014. The hue axis wraps around (points near 360° affect 0°). Max 16 points.

**Behavior / output:** Stage 10. Gives finer control than the eight HSL bands, so the user can target any hue width.

**Edge cases:**
- The wrap seam must be continuous. A golden test checks this.
- Gray pixels are unaffected by hue-based curves, the same as HSL.
- A curve whose points all sit on the neutral line is an identity curve; the stage is skipped.
- Combined with large HSL band shifts (CL-013), the two stages stack in the fixed order (secondary curves first), which can push a hue further than either alone.

**Undo:** `SetCurve` with secondary channel ids.

**Model notes:** `SecondaryCurves` inside `ColorStack`, baked to 1D textures. Reference: DaVinci Resolve (free) offers this family of curves.

### CL-030 Skin protection — v1

**What the user sees:** A **Skin protection** switch with an **Amount** slider in two sections of the Basic page. In the Color section it keeps white balance, hue, saturation, vibrance and HSL changes off faces and hands, so a strong teal grade does not turn people grey or green. In the LUT section it keeps the LUT's color off skin, which matters most with creative LUTs. Reference: the reference app shows a skin-tone protection toggle among its basic color controls and with its LUT. FreeCut's is a classic color-range mask with no face detection, and it is free. (Promoted from later to v1 on 2026-10-13: the LUT case is not covered by Vibrance's built-in skin hold.)

**Inputs / controls:** Per scope (Color, LUT): the switch, default off, and **Amount** 0–100, default 70 (FreeCut default), keyframable (CL-028). The Amount row shows only while the switch is on.

**Behavior / output:**
- One skin mask `m` per pixel, computed from the stage-4 output (the color entering white balance), so it does not move as sliders change the picture. With hue, saturation and value taken HSV-style from the gamma-encoded RGB, `m` is the product of three smooth weights (FreeCut defaults): hue fully inside 12°–42°, falling to zero at 5° and 50° with raised-cosine shoulders; saturation fully inside 0.12–0.65, zero below 0.06 and above 0.80; value full above 0.15, zero below 0.08.
- Color scope, amount `a`: stage 5's output becomes `lerp(stage5(x), x, m·a)`, and the output of stages 11–12 becomes `lerp(out, in, m·a)` with `in` the stage-11 input. Exposure, tone, wheels and curves stay unprotected, because brightness and contrast are meant to reach faces.
- LUT scope, amount `a`: the LUT's effective intensity per pixel is `intensity × (1 − m·a)`.
- Both scopes are per-pixel functions of the stack input, so on the CPU path they are part of the bake (C52) and cost nothing extra per pixel.

**Edge cases:**
- Orange or wooden objects are protected too, because the mask is hue-based. This is documented in the tooltip.
- Under very warm lighting most of the frame can fall inside the skin range, and the grade looks weaker. Lowering Amount helps.
- Switch on while nothing in that scope changes the picture: no visible change.
- Monochrome footage has no skin-hue pixels, so the switch does nothing.
- HSL edits on the red and orange bands are partly cancelled on skin, by design. The HSL page shows a one-line hint while the Color scope is on.
- The switch belongs to its section: section off, reset, copy/paste and saved looks (CL-024 to CL-026) treat it with the Color or LUT section.

**Undo:** The switch is `SetColorFlag` (`ColorEdit::SetSkin { scope, on }`); Amount is `SetColorParam` with `ColorParamId::SkinAmount(scope)`, one entry per drag.

**Model notes:** `SkinProtect { on: bool, amount: u8 }`, stored as `BasicAdjust.skin` (Color scope) and `LutSlot.skin` (LUT scope), with `SkinScope::{Color, Lut}` naming them. The mask function is shared by the bake and the reference evaluator in `freecut-render::color`, with unit tests on fixed patches (skin samples inside, sky, foliage and grey outside).

### CL-031 Masked (local) color adjustments — v1

**What the user sees:** The **Masks** page of the Color tab (CL-043), with a strip of mask chips at its top. (Promoted from later to v1 on 2026-10-13.) **Add mask** offers the shapes. Each mask appears as a chip labeled with its number and shape ("1 · Ellipse"). While a chip is selected, the preview shows that mask's outline and handles, the same as clip masks (TR-019). The whole grade then applies only inside the masks. The user can brighten a face, darken a sky or warm one side of the frame. An adjustment clip with a mask grades only that region of everything below it. Reference: the reference app's UI offers color masks in six shapes (line, parallel band, rectangle, circle, freehand brush, edge-following brush). An item can hold several masks, each with feather, expansion, position, size, rotation and invert. Public reviews describe the feature as a paid tier; FreeCut's version is free.

**Inputs / controls:**
- **Shapes:** TR-017's shapes (Linear, Band, Ellipse, Rectangle, Heart, Star), plus the pen path once TR ships it (TR-021). Color masks reuse TR's `Mask` type, ranges, defaults, feather and invert (TR-018) and on-canvas editor (TR-019) instead of defining their own. The first build (slice K6) ships **Linear**, **Band** (a mirrored pair of linear edges), **Ellipse** and **Rectangle**, and builds the shared mask core for them (type, signed-distance coverage, feather, invert, combine modes, on-canvas handles), which TR's clip masks then reuse; Heart and Star join when TR-017 adds them to the core.
- **Expand:** −100..+100, default 0, keyframable. This is the one control color masks add. It grows or shrinks the shape's edge by up to ±10 % of the item's shorter side, as an offset on the signed distance before feathering.
- **Several masks:** up to 8 per stack (FreeCut default, the same cap as TR-020). They combine in list order with TR-020's modes: Add (default), Subtract, Intersect. Drag to reorder, and delete from the chip's menu.
- **Brush and edge-following brush:** a painted mask needs a stroke-based mask shape that TR does not define yet. CL asks TR to add one, with strokes stored as resolution-independent vector paths, a brush size and an eraser. The edge-following brush needs a segmentation model and would come from AI-015's refinement prompts. Until those exist, **Add mask** lists only TR's shapes.

**Behavior / output:**
- Stage 19 of the stack (CL-001): final = lerp(base, graded, amount × coverage). Coverage is the combined, feathered mask value, and base is the stage-3b output. The cleanup stages (deflicker, denoise, dehaze) always apply to the whole item.
- Color masks only limit the grade. They never change the item's alpha. The clip's own mask (TR-017, FX-001 L7) is separate, and the two do not affect each other.
- On a clip, masks live in the clip's own frame (layer space, before the L8 transform), so they move and scale with a PiP. On an adjustment clip, they live in canvas space.
- Mask parameters can be keyframed through TR-027, the same as TR-018's list, and can follow motion-track data (AI-008) like clip masks.

**Edge cases:**
- A zero-area mask (or one fully outside the frame) makes the grade invisible. Invert on it applies the grade everywhere.
- With no masks, the grade covers the whole item. This is the default.
- Apply to all (CL-025) never copies masks, and Save look (CL-026) never stores them. Both say so.
- Reset all color (CL-024) removes masks after a confirmation that says how many. Pasting the Color masks category through Paste attributes replaces the target's masks after the same kind of warning.
- Compound clips (TL-040): masks apply to the flattened result with no preparation step. Alternative: the reference app needs a pre-processing step on compound clips before they can be masked or tracked.
- Masks, and later brush strokes, are stored in the project document. There is no separate mask-data folder, so the reference app's failure case, where brush data can't be saved because its folder can't be created, cannot happen.
- A section's bypass (CL-023) leaves the masks in place. The master bypass turns off the whole stack, masks included. Mask outlines show only while the Masks page is open.
- Spatial stages (sharpen, clarity, vignette, grain) are computed on the whole item and then blended by coverage, so a feathered edge never shows a seam.

**Undo:** `AddColorMask { item, index, mask }`, `RemoveColorMask { item, index, mask }` (restores in place), `ReorderColorMask { item, from, to }`, `SetColorMaskMode { item, id, old, new }`, and `SetColorMaskParam { item, id, param, old, new }` (one per drag, like TR's `SetMaskParam`), or TR's keyframe ops for animated parameters.

**Model notes:** `ColorStack.masks: SmallVec<[ColorMask; 8]>` with `ColorMask { id: u16, mask: Mask /* TR-017's type */, expand: Animatable<i16>, mode: MaskCombine /* TR-020 */ }` in `freecut-timeline`. Coverage reuses TR's SDF shader code and CPU reference in `freecut-render`. Depends on FX-001's order only through L5, where the stack runs. Alternative considered: binding the grade to the clip's own TR mask. It was rejected because darkening a sky would then also cut the clip's alpha.

**FreeCut CPU path (2026-10-13, README C52).** Coverage is a separate per-pixel pass, never part of the bake. For a clip it is evaluated in the item's display frame (TR-017's clip space) and mapped onto the stored picture through its rotation and mirroring, at the size the compositor graded; for an adjustment clip it is evaluated in canvas space at output resolution. Each shape is a signed distance in units of the item's shorter side; Expand offsets it, feather (TR-018) turns it into coverage with a smoothstep across the feather width, invert takes `1 − c`, and masks combine in list order (Add = max, Subtract = `c·(1 − m)`, Intersect = min). A stack with masks keeps the stage-3b image as the blend base and blends after the last stage, `lerp(base, graded, amount × coverage)` in `f32`, before the single quantization. The Masks section's switch off means full coverage. Mask outlines show only while the Masks page is open.

### CL-032 Input transform for log and camera footage — later

**What the user sees:** An **Input** dropdown at the top of the Color tab: None (default), a set of camera log-to-Rec.709 conversions, or Custom LUT…. The chosen conversion turns flat log footage into normal-looking video before any grading.

**Inputs / controls:** Transform choice, custom input LUT, and an exposure offset (−2..+2 EV, default 0).

**Behavior / output:**
- Stage 1. Built-in conversions use camera makers' published log curve formulas and gamut matrices, followed by a FreeCut tone-map to Rec.709.
- Custom LUTs here use the same import as CL-016 but in a separate slot. Until this feature ships, users can put a conversion LUT in the LUT slot (CL-017).

**Edge cases:**
- Choosing a log conversion on non-log footage gives an over-contrasty image, which is the user's choice. The media's probe hint (PM) can suggest "This looks like log footage".
- A shaper-plus-3D `.cube` LUT works in this slot as in CL-016.

**Undo:** `SetInputTransform { item, old, new }`.

**Model notes:** `InputTransform` enum in the timeline. The formulas live in `freecut-render` with golden tests against published reference values.

### CL-033 Video noise reduction (Denoise) — v1

**What the user sees:** A **Denoise** slider at the top of Basic › Detail & finish, above Sharpen. Raising it smooths the speckle and color blotches of low-light phone footage while edges stay crisp, live in the preview. A disclosure arrow reveals **Temporal**, **Temporal frames** and **Detail**. When Temporal is on, the clip's color badge shows a small "T". Reference: the reference editor's public desktop enhancement page lists noise reduction among its clean-up tools for low-quality footage. FreeCut's version is a classic, model-free filter that runs live; ML denoising stays in AI-019 as derived media.

**Inputs / controls:** All FreeCut defaults, since public sources give no ranges.
- **Denoise:** 0–100, default 0. Keyframable (CL-028).
- **Temporal:** toggle, default off. Adds averaging across neighboring source frames, which cleans static and slow-moving shots much more than the spatial filter alone.
- **Temporal frames:** 3 or 5 (one or two frames on each side), default 3. Shown only while Temporal is on.
- **Detail:** 0–100, default 50. Keyframable. Higher keeps more fine texture (and a little more noise); lower gives a smoother, flatter picture.

**Behavior / output:**
- Stage 3 of the stack (CL-001): after deflicker and before color match and every grading stage. Contrast, curves and sharpening therefore work on a cleaned image and do not amplify the noise. Grain (stage 18) is added afterwards, so "clean it, then add even grain" works.
- **Spatial filter** (whenever Denoise > 0): an edge-preserving, bilateral-type filter, run separately on luma and on chroma (Rec.709 luma plus two color-difference channels). Chroma gets twice the luma radius and a higher threshold, because color blotches are the most visible part of phone noise. Radii are relative to frame height (luma about 0.2 %, chroma about 0.4 %, so about 2 px and 4 px at 1080p). Denoise sets the edge threshold and the filter strength. Detail lowers the threshold so that fine texture above it survives.
- **Temporal filter** (when on): for each pixel, the pixels at the same position in the neighboring source frames are averaged in. Each neighbor's weight falls to zero where it differs from the current frame by more than a threshold set by Denoise. Static areas get strong averaging. Moving areas fall back to the spatial filter alone, which avoids ghost trails without any motion estimation. The spatial filter then runs on the result with a lower threshold.
- **Neighbors are source frames.** Let `s` be the source time that L1 sampled for this output frame through TR's time map (TR-036). The neighbors are the adjacent source frames around `s`. Each one goes through the same L1–L4 processing (decode, normalize, stabilize, crop, key) and stack stages 1–2. With TR-041's frame blend, the window is built around the blended center from single neighboring source frames, and the motion-adaptive weights keep this from smearing. Speed, reverse and output frame rate never change which frames are averaged.
- **Determinism (FX-001 rules):** the filter is non-recursive. A frame's result depends only on the source frames in its fixed window, never on previously rendered output. Scrubbing, rendering out of order, playback and background export therefore produce identical frames. Recursive temporal filters that feed back their own previous output, such as the temporal part of FFmpeg's hqdn3d, are rejected for this reason. One WGSL implementation serves preview and export.
- **Performance:** during playback the renderer keeps a ring buffer of the last 3 or 5 processed source frames per clip, so each source frame is decoded and pre-processed once. Reverse playback fills the buffer backwards. Scrubbing decodes the window on demand. If the GPU or decode budget is exceeded, preview lowers its resolution (EX) and never turns the temporal pass off.
- Denoise 0 is identity: the pass and all neighbor fetches are skipped whatever the other settings say.

**Edge cases:**
- Images and freeze-frame stills have no neighbors. Temporal is greyed out with "Needs video", and the spatial filter applies alone. A freeze segment that holds one source time of a video clip (TR-036) uses that time's real neighbors, so the frozen picture is one stable denoised frame for its whole length.
- Adjustment clips: the spatial filter runs on the composite below, in canvas space. Temporal is greyed out ("Clip-only") because it would need several full composites per frame. A Temporal value pasted onto an adjustment clip is kept but inactive. Compound clips (TL-040) work the same way: the spatial filter applies, and Temporal is greyed out with "Apply inside the compound clip" (CL-001).
- Trim points: the window reaches into handles beyond the clip's in and out points wherever the media has frames, so trimming or splitting never changes a remaining frame. At the first and last frame of the media the window is cut short and the weights renormalized.
- Speed: a 0.5× clip shows each denoised source frame twice. A 4× clip needs neighbors that playback would otherwise skip, so decode cost rises and preview may drop resolution.
- A cut inside the source media: neighbors from the other shot differ strongly and get zero weight, so the two shots never blend.
- Fast motion, or low-contrast moving edges, at high Denoise with Temporal on can leave a faint trail. Lowering Denoise or turning Temporal off removes it, and the tooltip says so. Motion-compensated temporal denoise (sharing TR-042's optical flow once that ships) is a later improvement.
- Keyed clips: transparent pixels get zero weight and alpha is never changed, so color does not bleed into keyed-out areas.
- Denoise 100 with Detail 0 gives a plastic, painted look. That is the user's choice, and the default Detail keeps texture.
- 8-bit sources with smooth gradients (sky, plain walls): strong denoise can reveal banding. Light grain (CL-011) hides it, and the tooltip suggests it.
- Proxies: the preview denoises the proxy frame. Radii are relative, but proxies are already softer, so the proxy preview can look slightly cleaner than the export. Full-quality preview (EX) shows the export result exactly.
- Interlaced sources are deinterlaced before the stack (FX/EX).
- HDR sources are tone-mapped at L2 (CL-002) before denoise runs.
- Items on a locked track show the controls read-only.

**Undo:** Denoise and Detail: `SetColorParam`, one entry per drag (CL-007 rules). Temporal: `SetColorFlag { item, flag, old, new }`. Temporal frames: `SetColorParam` with `ColorParamId::DenoiseRadius`. Undo writes back the old value.

**Model notes:**
- `freecut-timeline`: `Denoise { amount: u8, detail: u8, temporal: bool, radius: u8 /* 1..=2 */, algo_rev: u16 }` inside `ColorStack.cleanup`. It is part of the Basic section for bypass, reset and copy/paste (CL-023 to CL-025), and saved looks leave it out (CL-026).
- `algo_rev` records which algorithm revision the clip was set up with. FreeCut keeps every shipped revision's shader so old projects render the same, as CL-005 does for looks. A project that names an unknown, newer revision renders with the newest known one and shows a one-time notice.
- `freecut-render`: WGSL passes plus a CPU reference for golden tests. The tests use synthetic noise of known strength on test charts, and one test checks that a frame's output is identical whether frames are rendered forward, backward or out of order.
- Neighbor frames come through the same source-frame decode path as L1 (`freecut-media`, off the UI thread).

### CL-034 Deflicker — v1

**What the user sees:** A **Deflicker** toggle with a **Strength** slider in Basic › Detail & finish, below Denoise. Turning it on starts a short analysis, with a progress bar on the clip and in the inspector, as stabilization does (TR-026). When it finishes, frame-to-frame brightness jumps in timelapses and the pulsing or rolling dark bands from LED or mains lighting are evened out. Slow real changes, such as a sunset or a cloud passing, are kept. A disclosure arrow reveals **Window**, **Mode** and **Even out color**. Reference: open tools such as FFmpeg document a moving-average deflicker for the same problem. FreeCut implements its own, with a cached analysis so it plays live.

**Inputs / controls:** All FreeCut defaults.
- **Deflicker:** toggle, default off.
- **Strength:** 0–100, default 100. How much of the computed correction is applied.
- **Window:** 3–31 source frames, odd values only, default 7. The length of the moving average that defines "steady" brightness. Longer windows remove slower flicker but also follow real lighting changes more slowly.
- **Mode:** **Global** (default) applies one correction per frame, for timelapse exposure jumps and whole-frame pulsing. **Banded** applies a separate correction to each of 32 horizontal bands, blended smoothly across rows, for the rolling dark stripes that rolling-shutter phone sensors record under LED or mains light.
- **Even out color:** toggle, default off. Also corrects frame-to-frame color shifts, for lights whose color changes as they flicker.

**Behavior / output:**
1. **Analysis.** Enabling queues a background job (PM-073). It decodes the clip's used source range plus half a window of handles on each side, at an analysis size of 256 px tall. For every source frame it records the linear-light mean of R, G and B in each of 32 horizontal bands. Clipped pixels (any channel ≥ 0.98) and near-black pixels (luma < 0.02) are left out, and alpha weights the rest. Whole-frame and luma figures are derived from the band figures, so switching Mode or Even out color never needs a new analysis.
2. **Target and gain.** For each source frame and each band (or the whole frame in Global mode), the target level is a Gaussian-weighted moving average of the log levels over the window centered on that frame. The gain is 2^((target − measured) × Strength / 100), limited to ±1 EV. With Even out color, per-channel gains are computed the same way and normalized so their combined luminance matches the brightness gain.
3. **Real changes.** A jump between consecutive frames larger than 1.5 EV is treated as a cut or a real lighting change, such as a lamp switched on. The window does not reach across it, so the change is kept rather than smeared over several frames.
4. **Apply.** Stage 2 of the stack multiplies each pixel by its gain in linear light, the same way exposure does. In Banded mode the gain is looked up by the pixel's source row, with crop and stabilization offsets taken into account, and interpolated linearly between band centers. A soft shoulder keeps values near 1.0 close to white, so clipped highlights do not turn gray in frames the correction darkens.
- The per-frame figures are deterministic, regenerable data in the media cache (PM-074). They are keyed by media fingerprint, stream, analysis size and algorithm revision, and stored per source frame by timestamp. Each frame's figures are independent of the others, so one cache entry serves every clip of the same media and simply grows when a trim extends the clip. Inward trims, splits and duplicates reuse it.
- Gains are a pure function of the figures and the settings, computed off the UI thread, so preview and export use identical values.
- Analysis always reads the original media, never a proxy, so preview and export see the same figures.

**Edge cases:**
- Preview before the analysis finishes, or after the cache entry was evicted: the clip renders uncorrected with a badge while the analysis (re)runs. An export started before the data exists waits for it, with progress, rather than exporting flickering footage (the same rule as TR-026).
- Original media offline: no analysis runs, the clip shows "Deflicker needs the original media" and renders uncorrected, and export preflight (EX) lists it.
- Images, freeze-frame stills and adjustment clips: the toggle is disabled ("Needs video"). Compound clips (TL-040): disabled with "Apply inside the compound clip" (CL-001). Deflicker settings pasted onto any of these are kept but inactive. Image sequences imported as video (PM) are supported; they are the main timelapse case.
- A clip shorter than the window uses handles. At the ends of the media the window is cut short and renormalized. A media file with a single frame has no neighbors, so its gain is 1.
- Speed, reverse and freeze segments: gains belong to source frames, so each source frame always shows its own correction. Flicker that only appears after a large speed-up (slow source changes squeezed into fast output changes) needs a longer Window, and the tooltip says so.
- Large bright or dark objects moving through the frame shift the measured level and can cause slight pumping, more so in Banded mode. Lower Strength, a longer Window or Global mode reduces it. Documented.
- Very deep LED banding, with stripes darker than the ±1 EV limit, is only partly removed. The inspector says "Some frames exceed the correction limit".
- Variable-frame-rate media: frames are keyed by source timestamp, and the window counts source frames, not seconds.
- Strength 0 with the toggle on is identity, and the analysis data is kept.
- Turning the toggle off during analysis cancels the job. Figures already written stay valid and are reused next time.
- Lengthening Window, or trimming outward, analyzes only the extra handle frames.
- With TR-041's frame blend, an output frame mixes several source frames. Stage 2 then uses the same blend-weighted mix of their gains, which matches the per-frame result closely because neighboring gains differ only by the flicker being removed.
- With Temporal denoise (CL-033): deflicker runs first, so the denoiser compares frames at equal brightness and can average more.

**Undo:** Toggle, Mode and Even out color: `SetColorFlag { item, flag, old, new }`. Strength and Window: `SetColorParam`, one entry per drag (Window is validated to odd values). Analysis data is cache, not model state, so undo never deletes it, and redoing an enable reuses the cached figures.

**Model notes:**
- `freecut-timeline`: `Deflicker { enabled: bool, strength: u8, window: u8, banded: bool, per_channel: bool, algo_rev: u16 }` inside `ColorStack.cleanup`. It is part of the Basic section for bypass, reset and copy/paste, is not keyframable in v1 (CL-028), and is left out of saved looks (CL-026). `algo_rev` follows the same rule as in CL-033.
- `freecut-render::color::stats::{band_levels(frame) -> BandLevels, deflicker_gains(series, params) -> Vec<FrameGain>}` are pure functions. Their unit tests use synthetic sequences: random exposure steps, sine pulsing, rolling bands, a slow ramp that must survive, and a real step that must not be smeared.
- `freecut-media` decodes frames in a PM-073 job and stores the series through `freecut-media::cache` (PM-074). `freecut-app` shows progress and badges.

### CL-035 Overall color amount — v1

**What the user sees:** An **Amount** slider in the Color tab header, next to the master bypass (CL-023). Dragging it down fades the whole grade (look, LUT, corrections and finishing) back toward the ungraded picture, which tones down a strong grade without touching a dozen sliders. On adjustment clips the same control is labeled **Mix** (CL-006). Reference: the reference app's adjust panel has an overall intensity control that scales its whole adjustment stack.

**Inputs / controls:** Amount 0–100 %, default 100, step 1, keyframable (CL-028). Double-click or the reset arrow sets it back to 100.

**Behavior / output:**
- Stage 19 of the stack (CL-001): final = lerp(base, graded, amount / 100). Here base is the image after the cleanup stages 1–3b (input transform, deflicker, denoise, dehaze), and graded is the stage-18 output. Lowering Amount brings back the ungraded color, not the noise or flicker.
- With color masks (CL-031), the blend factor is amount × mask coverage.
- It runs in the fused per-pixel pass at no extra cost. At Amount 0, the renderer skips stages 4–18 and runs only the cleanup stages.

**Edge cases:**
- Amount multiplies with the filter, LUT and match intensities rather than replacing them. A filter at 50 % under Amount 50 % shows about a quarter of its change.
- Grain, vignette and sharpen fade with Amount like everything else. Grain at Amount 50 is half as strong.
- Amount 0 while sections hold values: the color badge shows a hollow state, meaning "graded, amount 0".
- Bypass versus Amount 0: the master bypass (CL-023) also skips the cleanup stages, while Amount 0 keeps them.
- Amount is not part of any section. Section bypass and section reset leave it alone. "Reset all color" (CL-024) and its own reset arrow restore it. In copy/paste it travels with the Basic category (CL-025). Saved looks do not store it (CL-026).
- Multi-selection: Amount is set on every selected item in one transaction (UI-010).

**Undo:** `SetColorParam` with `ColorParamId::Amount`, one entry per drag (CL-007 rules).

**Model notes:** `ColorStack.amount: u8`, default 100. For adjustment clips this replaces the earlier separate `mix` field. `ItemKind::Adjustment { color: ColorStack }` reads Mix from `color.amount`, so clips and adjustment clips share one control, one `ColorParamId` and one keyframe track. `ColorStack::is_identity()` also returns true when Amount is 0 and the cleanup stages are at their defaults.

### CL-036 Dehaze — later

**What the user sees:** A **Dehaze** slider under More in Basic › Detail & finish. Positive values cut through haze, fog, smog or underwater murk, bringing back contrast and color in distant parts of the picture. Negative values add a soft atmospheric haze. Reference: the reference app's detail section lists a dehaze control.

**Inputs / controls:** Dehaze −100..+100, default 0, keyframable (CL-028). The More menu also has **Re-estimate haze color**.

**Behavior / output:**
- Stage 3b of the stack (CL-001): after denoise, before color match and white balance, so every correction and look works on the cleared image.
- Positive values use classic dark-channel-prior dehazing from published literature (He, Sun and Tang, 2009). The haze amount per pixel is estimated from the darkest color channel over a small neighborhood (about 1 % of frame height). An edge-aware guided filter refines that estimate so halos stay small. The scene is then recovered from the estimated haze color and transmission. At +100, up to 90 % of the estimated haze is removed. Transmission never drops below 0.1, which limits noise blow-up.
- Negative values mix in a flat veil of the haze color, weighted toward bright areas, up to 40 % at −100.
- The haze color is estimated once, the first time Dehaze leaves 0. A worker thread samples 5 frames spread over the clip's used range, the same way as CL-019, and the color is stored with the setting. The result is therefore static and never flickers from frame to frame. Until the estimate is ready, the preview uses the frame at the playhead. **Re-estimate haze color** runs it again, for example after a trim.

**Edge cases:**
- On haze-free footage, positive Dehaze mostly adds contrast and saturation, and high values look harsh. This is expected, and the tooltip says so.
- Bright skies can read as haze. The transmission floor and the guided filter keep them from going noisy or dark. On 8-bit sources, strong dehaze can reveal banding in skies, so the tooltip suggests light grain (CL-011).
- Dehaze lifts noise in hazy areas. Because it runs after denoise (CL-033), a little denoise helps.
- Adjustment clips: it runs on the composite below, in canvas space. The haze color is estimated from the composite at the playhead and stays fixed until re-estimated.
- Images and freeze frames: the estimate comes from the single image.
- Keyed clips: transparent pixels are left out of the estimate and the filter weights.
- A black or uniform frame gives a neutral haze estimate and a finite result. Covered by the corner-value property test.

**Undo:** `SetColorParam` for the slider. Re-estimating is `SetDehazeAirlight { item, old, new }`, so undo restores the previous haze color.

**Model notes:** `Cleanup.dehaze: Dehaze { amount: i16, airlight: Option<[u16; 3]> }` (fixed-point 0..=10000). It belongs to the Basic section for bypass, reset and copy/paste. Like denoise and deflicker, it is left out of saved looks, because it depends on the footage. `freecut-render`: a min-filter pass and a guided-filter pass (both separable), plus a CPU reference for golden tests. `freecut-render::color::stats::estimate_airlight(frames) -> [u16; 3]` is a pure function.

### CL-037 Custom color targets (HSL picks) — later

**What the user sees:** Below the eight HSL swatches (CL-013), a **+ Pick color** button. The user clicks a spot in the preview, and a new swatch appears filled with that color. It has its own **Hue**, **Saturation** and **Lightness** sliders, plus **Range**, which sets how wide a band of hues around the picked color is affected. Use it for a brand color or a product shade that falls between two fixed bands. Reference: the reference app lets the user pick up to 10 custom colors from the image and adjust each one on its own.

**Inputs / controls:**
- Up to 10 custom targets per stack, the same as the observed limit.
- Per target: **Hue** −100..+100 (±30°), **Saturation** −100..+100 and **Lightness** −100..+100, all default 0 and keyframable. **Range** 0–100, default 25, maps to a hue half-width of 5°–45° (default 15°, FreeCut default) and is not keyframable.
- Picking samples a 5×5 average at the HSL stage input, through the same readback as CL-013's eyedropper. The target stores the picked hue as its center.
- Right-click a target swatch for Re-pick, Reset or Remove.

**Behavior / output:**
- Stage 11, after the eight fixed bands, with targets in list order. Membership falls off with a raised cosine across the half-width around the center hue. It is scaled down for low-saturation pixels, as in CL-013, so grays never shift.
- Targets work independently of the fixed bands, and a pixel can be changed by both: fixed band first, then target.

**Edge cases:**
- Picking a gray or near-black pixel gives "No distinct color here", and no target is added.
- At 10 targets, **+ Pick color** is disabled with the tooltip "Remove a custom color first". Alternative: the reference app replaces the oldest target, after a confirmation, when an 11th is added. FreeCut never throws settings away as a side effect.
- Picking a hue within 3° of an existing target selects that target instead of adding a duplicate.
- Overlapping ranges are allowed. Both targets apply in order, and their shifts add up.
- Keyed clips: transparent pixels can't be picked. Adjustment clips: picking samples the composite below.
- Targets belong to the HSL section, so they are bypassed, reset, copied (CL-025) and saved in looks (CL-026) with it.

**Undo:** `AddColorTarget { item, index, target }`, `RemoveColorTarget { item, index, target }` (restores in place), Re-pick and Range as `SetColorTarget { item, id, old, new }`, and the sliders as `SetColorParam` with `ColorParamId::HslTarget { id, channel }`.

**Model notes:** `Hsl { bands: [HslBand; 8], targets: SmallVec<[HslTarget; 10]> }` with `HslTarget { id: u16, center_hue: u16 /* hundredths of a degree */, range: u8, hue: i16, sat: i16, light: i16 }` in `freecut-timeline`. The count limit is validated there. `id` is stable within the stack, so keyframe tracks survive reordering and removal. `freecut-render` passes the targets as a fixed uniform array of 10.

### CL-038 Log-range color wheels — later

**What the user sees:** A **Primary / Log** switch at the top of the Wheels section (CL-015). Log mode shows four wheels, **Shadow**, **Midtone**, **Highlight** and **Offset**, plus **Low range** and **High range** sliders that set where shadows end and highlights begin. Each log wheel touches only its own narrow tonal band, which suits flat footage and precise work, where primary wheels feel too broad. Reference: the reference app's wheels have a log mode, and public pages describe a separate log-style wheel set.

**Inputs / controls:**
- Puck and luminance per wheel as in CL-015. At the rim, each band's puck adds up to ±0.10 (FreeCut default).
- **Low range:** 5–50 % of the tonal range, default 33 %. **High range:** 50–95 %, default 66 % (FreeCut defaults). Neither is keyframable.
- The Wheels section's **Strength** slider (CL-015) scales the log wheels too.

**Behavior / output:**
- Stage 8 runs either the primary set or the log set, whichever mode is active.
- Each log band's puck adds an RGB offset with zero net luma, weighted by the pixel's membership in that band of encoded luma. Each boundary has a smooth raised-cosine crossover 10 % wide. Offset shifts everything. Unlike lift, gamma and gain, the bands barely overlap, so a highlight push leaves the shadows alone.
- Each mode keeps its own values. Switching modes changes which set renders and loses nothing.

**Edge cases:**
- Low range is kept at least 5 % below High range. Dragging one past the other pushes it along.
- Pucks dragged outside the disk clamp to the rim.
- Log footage without an input transform (CL-032) works, but the bands sit at different tones than on Rec.709 footage. The tooltip suggests adjusting the ranges.
- Keyframes on the inactive mode's pucks are kept and come back when the user switches back.
- Saved looks (CL-026) store the mode, both sets and both ranges.

**Undo:** `SetWheelMode { item, old, new }`. Pucks use `SetWheelPuck` with a log wheel id. Ranges and luminance use `SetColorParam`.

**Model notes:** `ColorWheels` gains `mode: WheelMode { Primary, Log }` and `log: LogWheels { shadow, midtone, highlight, offset: Wheel, low_range: u16, high_range: u16 }` (0..=10000), added by a project-format migration with neutral defaults. The math lives in `freecut-render`, with corner-value property tests as in CL-015.

### CL-039 Match several clips to one reference — v1

**What the user sees:** With several clips selected, **Match color…** (CL-020) enters pick mode once, and after the user picks one reference frame or image, every selected clip is matched to it. The Color tab menu also offers **Match all clips…**, which does the same for every visual clip on the timeline. Matched clips form a *match group*. Each one shows CL-020's Match row with "Group: <reference name>", plus **Change reference…** and **Remove from all**. Reference: the reference app has a project-wide consistency action that matches every clip to one reference frame, lets the user change that reference later, and removes it as a unit.

**Inputs / controls:** Targets: Selected clips (the default when 2 or more are selected), or All visual clips. The reference frame or image. Intensity stays per clip (CL-020, default 100). **Replace existing matches** (default on) controls clips that already have a match.

**Behavior / output:**
- Each target gets its own CL-020 transform against the shared reference, from 5 frames sampled across its used range. The work runs as one background job (PM-073) with per-clip progress, and the user can keep editing.
- When the job finishes, every result is committed in one transaction. Clips that couldn't be matched (for example "Not enough detail to match") stay unchanged, and a summary lists them next to the matched ones.
- **Change reference…** re-runs the job for every member with the new reference, again as one transaction. **Remove from all** clears every member's match in one transaction.

**Edge cases:**
- The reference clip itself, if selected, is skipped and noted in the summary.
- Locked-track clips are skipped and counted. Text and adjustment clips are never targets.
- A clip deleted, moved to a locked track or re-matched while the job runs is skipped at commit and listed.
- A member that is later matched on its own to a different reference (CL-020 Re-match) leaves the group.
- Deleting the reference clip keeps every stored transform. Change reference… still works, and Re-match is disabled, as in CL-020.
- Cancelling the job commits nothing.
- With **Replace existing matches** off, clips that already have a match are skipped. The dialog says how many.
- Very large selections stay bounded at 5 frames per clip. Progress shows "12 of 340".

**Undo:** One `SetMatchGroup { group, changes: Vec<(ItemId, Option<MatchTransform>, Option<MatchTransform>)> }` transaction per run, Change reference or Remove from all. Undo restores each clip's previous transform, or no transform.

**Model notes:** `MatchTransform.group: Option<MatchGroupId>` (CL-020). There is no separate group registry. A group is the set of clips that share an id, and each member's `MatchRef` holds the reference. The job lives in `freecut-app` and uses `freecut-render::color::stats::match_stats` per clip.

### CL-040 Look parameters — later

**What the user sees:** Some bundled looks show a **Look settings** disclosure under the Intensity slider in the Filter section. It holds up to three extra sliders that the look defines, such as warmth, grain or fade for a film-style look. Reference: the reference app shows an extra parameter panel for some filters, beyond the intensity slider.

**Inputs / controls:**
- Up to 3 parameters per look (FreeCut default), each bipolar (−100..+100) or unipolar (0–100) as the recipe declares, with the recipe's default. All are keyframable.
- Names come from a fixed FreeCut vocabulary so they translate cleanly: Warmth, Tint, Contrast, Saturation, Fade, Grain, Vignette.

**Behavior / output:**
- A parameter never re-bakes the look's LUT. Each one adds an offset to one of the stack's own controls (temperature, tint, contrast, saturation, fade, grain amount or vignette amount). The offset is scaled by the filter intensity and added to the item's own value, then clamped to that control's range. This is the same mechanism a recipe already uses for its finishing amounts (CL-004).
- Choosing a different look resets the parameters to that look's defaults.

**Edge cases:**
- A look whose new `rev` drops a parameter: projects pinned to the old rev keep it (CL-005).
- An unknown look keeps its parameter values as data, as with CL-004's unknown-look rule.
- A parameter offset plus a slider already at its limit clamps.
- User looks (CL-026) have no parameters. They copy editable sections instead.

**Undo:** `SetColorParam` with `ColorParamId::LookParam(index)`.

**Model notes:** `FilterSlot` gains `params: SmallVec<[i16; 3]>`. The recipe schema (CL-005) gains `params: [{ id, target: ColorParamId, min, max, default, scale }]`, and the bake check rejects targets outside the vocabulary.

### CL-041 HDR source conversion options — v1

**What the user sees:** For HDR media (PQ or HLG), the media's **Interpret…** dialog (PM-040) has an **HDR to SDR** group. Clips of that media show a small "HDR → SDR" chip in the Color tab that opens the same settings. Most users never touch them, because CL-002's automatic conversion is the default. Reference: the reference app offers an automatic HDR conversion toggle and a peak-brightness setting in nits.

**Inputs / controls (FreeCut defaults):**
- **Method:** **Tone map** (default) rolls highlights off smoothly. **Clip** has no roll-off: everything above SDR white clips, which gives brighter midtones and loses highlight detail.
- **Source peak:** **Auto** (default) reads the stream's mastering-display or content-light metadata, else uses 1000 cd/m². Manual values run from 400 to 10000 cd/m².
- **Exposure before conversion:** −2..+2 EV, default 0. Applied in linear light before the tone curve, so it decides which highlights get rolled off. CL-007's Exposure, by contrast, works on the already converted image.
- **Gamut:** **Compress** (default) or **Clip**. This decides how BT.2020 colors outside Rec.709 are brought in.

**Behavior / output:**
- Runs in CL-002's L2 conversion shader, before the color stack. The defaults reproduce CL-002's automatic behavior exactly.
- PQ is decoded to absolute luminance. HLG goes through the BT.2100 inverse OETF and an OOTF for a 1000 cd/m² nominal display.
- The settings live on the media, so every clip of that media shares them. Preview and export use the same shader.

**Edge cases:**
- Wrong or extreme metadata (for example a 10000 cd/m² peak on a dim video) makes the picture too dark. A manual Source peak fixes it.
- SDR media: the group is hidden.
- HDR and SDR clips on one timeline are converted independently. Color match (CL-020, CL-039) evens them out.
- Dolby Vision metadata is ignored, and the base layer is used (EX keeps proprietary HDR formats out of scope).
- Clip method on very bright PQ content: large areas clip to white. This is documented in the tooltip.
- Proxies (PM-080) are 8-bit SDR files made with the settings current when they were generated. Changing these settings marks that media's proxies Stale, and they regenerate on demand. Thumbnails and filmstrips (PM-070, PM-071) regenerate as well.
- Offline media: the settings can still be edited, and they apply when the media is relinked.

**Undo:** `SetColorInterpretation { media, old, new }`, CL-002's op. The HDR options are part of the same override.

**Model notes:** `ColorOverride`, the `color` field of PM-040's `Interpretation`, gains `hdr: Option<HdrToSdr { method: ToneMapMethod, source_peak: Option<u16>, pre_exposure: i16 /* hundredths of an EV */, gamut: GamutMode }>`. `None` means automatic. `freecut-render` implements the curve in the L2 shader with a CPU reference. Golden tests on synthetic PQ and HLG ramps check that the curve is monotonic, that reference white lands within tolerance, and that the defaults match CL-002.

### CL-042 Grade after clip effects — later

**What the user sees:** In the Color tab menu, **Color position: Before effects** (default) or **After effects**. With After effects, the grade also colors whatever the clip's effects add. For example, a glow or light-leak effect takes on the same warm look as the footage. Reference: the reference app has a setting that decides whether color adjustments run before or after a clip's other properties.

**Inputs / controls:** A two-way choice per item, for clips, compound clips and raster overlays. Adjustment clips have no clip effects, so they don't show it.

**Behavior / output:**
- Before effects is FX-001's L5. After effects moves stack stages 4–19 to just after L6 (clip effects), still before the mask and transform, so the grade stays in layer space. Cleanup stages 1–3b always stay at L5, so denoise and deflicker never see effect output. In this mode the stage-19 blend base (Amount, masks) is the L6 output, so Amount 0 still shows the effects.
- FX-001 owns the stage order. This feature needs FX-001 to define the alternate slot, and it cannot ship until FX does. Until then, the workaround is an adjustment clip above the clip (CL-006).

**Edge cases:**
- Effects with frame history (echo, trails) in After effects mode: the grade colors their output.
- Keying (L4) is unaffected in both modes.
- An item with no clip effects renders identically in both modes, and the menu shows the choice as having no effect.
- Copy/paste: the position travels with the Basic category (CL-025). Saved looks do not store it (CL-026).

**Undo:** `SetColorPosition { item, old, new }`.

**Model notes:** `ColorStack.position: ColorPosition { BeforeEffects, AfterEffects }`, added by a project-format migration that defaults to `BeforeEffects`. FX's `FramePlan` stage enum would gain the alternate slot.

### CL-043 Color tab pages and section controls — v1

**What the user sees:** The Color tab (the **Look** lens of the UI concept, §6.3) of a video clip, image clip, raster overlay or adjustment clip, laid out top to bottom as:
1. A header row: the **Presets** select (saved looks, CL-026), the master on/off switch (CL-023), **Amount** with its keyframe navigator (**Mix** on adjustment clips, CL-035), the **Scopes** button (CL-021) and the `...` menu (Save look…, Reset all color, Auto color all clips…, Match all clips…, Color position).
2. A page strip, a segmented control with five pages: **Basic**, **HSL**, **Curves**, **Wheels**, **Masks**. A page that holds any non-default value shows the 4 pt accent dot after its label.
3. The chosen page's sections, in the inspector's section grammar (ui-concept §6.2).
4. A footer with **Save look…** (CL-026) and **Reset all** (CL-024).

| Page | Sections, top to bottom |
|---|---|
| Basic | **Auto** (CL-019), **Match** (CL-020, CL-039), **Light** (CL-007), **Color** (CL-008, with Skin protection, CL-030), **LUT** (CL-017, with Skin protection), **Filter** (CL-004), **Detail & finish** (CL-009 to CL-012, CL-033, CL-034) |
| HSL | **HSL** (CL-013; custom targets CL-037 later) |
| Curves | **Curves** (CL-014; secondary curves CL-029 later) |
| Wheels | **Wheels** (CL-015; log mode CL-038 later) |
| Masks | **Masks** (CL-031) |

Reference: the reference app's adjust panel also splits basic sliders, HSL, curves, a color wheel and masks into separate tabs, and its basic part has foldable sections with their own on/off, reset and keyframe controls. FreeCut lays this out in its own lens grammar, with its own section set and names.

**Inputs / controls:**
- Page strip: click a page; with the strip focused, Left and Right move between pages (UI-031 owns any further bindings). The page is remembered per selection kind (UI-008's tab memory).
- Every section header: title, chevron, a summary while folded ("Exposure +12 · Contrast −5", "LUT: <name> · 80 %", "off"), the help badge, the reset badge (dimmed when the section is at defaults), the on/off switch, and the `...` menu (Reset section, Copy settings, Paste settings, Apply to all…). The switch is CL-023's section toggle; switching a section off keeps its values and keyframes and folds it.
- Rows follow ui-concept §6.2: the label scrubs on drag and resets on double-click, an accent dot marks a non-default value, a slider rail and number field set the value, and keyframable rows (CL-028) carry the keyframe navigator in the gutter.
- Sections that hold a choice show the action while nothing is chosen, with their switch disabled: Auto shows **Auto**, Match shows **Match color…**, LUT shows the LUT select at None, Filter shows **Browse looks** (opens the Filters tab).

**Behavior / output:**
- Pages, folding and the remembered page are view state. Nothing in the model changes until a value does.
- Each section is one bit of the stack's `SectionMask`: Auto, Match, Light, Color, LUT, Filter, Detail, HSL, Curves, Wheels, Masks. The copy/paste categories of CL-025 stay coarser: Basic (Auto, Match, Light, Color, Detail and the overall Amount), LUT, Filter, HSL, Curves, Wheels, and Color masks.
- A page or section whose feature is not built yet is hidden, not disabled (the UI-004 rule). The first build shows only the Basic page with Light and Color (slice K1).
- Adjustment clips show the same lens with Amount labelled **Mix**. Their masks live in canvas space (CL-031).

**Edge cases:**
- Several items selected (UI-010): shared values show as usual, and a value that differs shows "—". The Masks page shows "Masks are edited one item at a time" instead of its chips, and Match shows its multi-clip form (CL-039).
- Items on a locked track show every page read-only (UI-008's banner).
- Text, audio, sticker and effect items have no Color tab.
- Switching off a section whose rows have keyframes keeps the keys; the navigators dim until the section is on again.
- Reset badge on a section with keyframes asks first, as CL-024 says.
- In the Compact density the page labels become icons with tooltips.

**Undo:** n/a for pages and folding. The switches are `SetSectionEnabled` and `SetColorBypass` (CL-023); every row edits through the op of its feature.

**Model notes:** `Section` enum and `SectionMask` (a `u16` of bits, all on by default) in `freecut-timeline::model::color`, with `Section::page() -> ColorPage` and `Section::ALL` in display order, so the UI and the reset and paste code share one list. The lens lives in `freecut-app::ui::inspector::look`, built from the existing section and row widgets; live drags send a `ColorDraft { clip, stack }` with the preview request, the way text drafts do, and commit one entry on release (CL-007).

### CL-044 Adjust tab in the asset panel — v1

**What the user sees:** The **Adjust** tab in the Shelf's Grade group (ui-concept §8.1, UI-004). Its side list has three groups: **New**, with one tile, *Adjustment clip*; **Saved**, the user's saved looks (CL-026); and **LUTs**, with *All*, *Bundled* (CL-045) and *Imported* (CL-016), followed by the usual Mine group (Favorites, Recent). The toolbar holds the search field, the sort select and, on the LUTs rows, **Import**. Tiles are 16:9 thumbnails of the selected clip's frame (or the frame under the playhead) through each item, rendered as in CL-003; the Adjustment clip tile shows FreeCut's own neutral glyph. Reference: the reference app's adjustment tab likewise offers a blank custom adjustment, the user's saved adjustments and a LUT list with an import action. FreeCut's saved items are the same local library as its looks, with no count limit and no sign-in.

**Inputs / controls:**
- *Adjustment clip:* the hover "+", double-click or Enter adds a blank adjustment clip (identity stack) with CL-006's default span and placement, selects it and opens its Color tab on the Basic page. Dragging the tile onto the timeline places it where it is dropped (TL's drop rules for adjustment tracks).
- *Saved look:* a click applies it to the selected clips (CL-026's apply). "+" or a drag onto empty track space creates an adjustment clip carrying it; a drag onto a clip applies it to that clip. Right-click: Rename, Duplicate, Delete, Export look… (CL-027, later).
- *LUT:* a click puts it in the LUT slot of the selected clips (CL-017). "+" or a drag onto empty track space creates an adjustment clip carrying it; a drag onto a clip applies it there. Right-click on an imported LUT: Rename, Show file, Remove from library; on a bundled LUT: Show file.
- *Import:* a file dialog with multi-select, or files and folders dropped on the tab (CL-016's import, with its summary).
- Hovering a tile previews it live in the player, as CL-004's hover preview does: no undo entry, the project stays clean.

**Behavior / output:**
- Applying from the tab follows CL-004's toast wording: "LUT added", "LUT replaced (was <name>)", "Look applied to 5 clips".
- With nothing selected a click does nothing and the status line says "Select a clip first, or use + to add an adjustment clip".
- The library index and the saved looks load on a worker at startup; tiles show placeholders until their thumbnails arrive (UI-007).
- The **Filters** tab keeps FreeCut's bundled looks only (CL-003).

**Edge cases:**
- Empty Saved group: "Grades you save with Save look… appear here", with no button (saving starts from a graded clip).
- Empty Imported group: an empty state with the Import button.
- A saved look that references a missing LUT shows a warning badge (CL-026); an imported LUT whose file is missing shows the missing badge and a Relink action (CL-016).
- Selections of text or audio only: applying is disabled and the tooltip says why. Locked-track clips are skipped and counted in the toast.
- Removing an imported LUT that open projects use only hides it (CL-016).

**Undo:** Adding an adjustment clip is TL's `InsertItem` (one entry, together with its stack). Applying a look or a LUT uses CL-026's and CL-017's ops. Library management (import, rename, remove, delete) is app-level and not on the project undo stack.

**Model notes:** `freecut-app::ui::assets::adjust` builds the tab on UI-006 and UI-007's grid. Inserting goes through `EditRequest::AddAdjustmentClip { seq, span: AdjustSpan::{Selection(Vec<ClipId>), AtPlayhead(Tick)}, color: Option<Arc<ColorStack>> }`, which picks the track by CL-006's placement rule. The LUT index is `freecut-project::luts` (CL-016) and the saved looks are `freecut-project::look` (CL-026); both are read and written on workers.

### CL-045 Bundled FreeCut LUTs — v1

**What the user sees:** The **Bundled** group of the Adjust tab's LUTs lists eight LUTs that ship with FreeCut (FreeCut default): a neutral contrast boost, a gentle warm and a gentle cool balance, a faded print, a teal-shadow and warm-highlight grade, a neutral mono, a warm mono and a blue night look. They follow CL-005's naming scheme of category, descriptor and number ("Contrast · Neutral 01"), and a tooltip says in one line what each does. They also appear in the LUT section's select, above the imported ones.

**Inputs / controls:** None beyond CL-017 and CL-044. Bundled LUTs cannot be renamed or removed; **Show file** reveals the `.cube` so the user can take it to other software.

**Behavior / output:**
- Each LUT is a 33³ `.cube` file under `assets/luts/`, generated from a FreeCut-written recipe (`assets/luts/recipes/*.json`, the same recipe format as CL-005's looks) by FreeCut's own CPU reference stack (C52). `assets/luts/manifest.json` lists id, display name, tags, `rev`, file name and hash.
- Generation is deterministic: a test re-bakes every recipe and compares the bytes with the checked-in file, and an ignored test of the same file regenerates them on purpose.
- Bundled LUTs are referenced like imported ones (`LutRef` hash and file name, CL-016). The resolver looks in the bundled set first, so a project that uses one never depends on the user's library.
- Changing a bundled LUT ships a new file with a new hash and keeps the old file, so existing projects render exactly as before (CL-005's rule).
- Recipes, files and manifest are licensed CC0-1.0 and listed in `assets/ASSETS.md`.

**Edge cases:**
- Each bundled LUT keeps pure black and pure white within ±0.08 of a neutral grey (CL-005's bake check), so it never paints borders.
- A project that names a bundled LUT this build lacks (made by a newer FreeCut) shows the missing-LUT badge and renders the stage as identity (CL-017).
- The mono LUTs at partial intensity give partial desaturation, as CL-005 says for mono looks.

**Undo:** n/a (shipped content).

**Model notes:** `freecut-render` exposes the bundled set through its `LutSource` (C52), loading the files with `freecut-media::lut::parse_cube`. The app adds the bundled directory from its resources at startup.

## Adjustments round 1 — next build scope

This build answers the user's request for an adjustment workflow: a Basic page with automatic adjustment, color match, light and color corrections and a LUT with skin protection, the HSL, Curves, Wheels and Masks pages, and an Adjust tab with blank adjustment clips, saved adjustments and LUTs. Everything stays free and local. It pulls the features below forward from M7 (see the [roadmap](README.md#roadmap)); their priority stays v1. CL-030 and CL-031 were promoted from later to v1 for it.

**Builds in this round:** CL-001 (the stack, its order and conventions, for the stages listed per slice), CL-006, CL-007, CL-008, CL-013, CL-014, CL-015, CL-016, CL-017, CL-019, CL-020, CL-023 (section and master switches; Compare stays for later), CL-024, CL-026, CL-028, CL-030, CL-031, CL-035, CL-043, CL-044, CL-045. Groundwork, only as far as these need it: TR-017, TR-018 and TR-019's mask core (four shapes, feather, invert, combine, on-canvas handles), UI-006 and UI-007 grids for the Adjust tab, UI-064 for Save look…, and TL-034's color categories for Paste attributes.

**Stays v1, after this round:** the Filters tab and bundled looks (CL-003, CL-004, CL-005), Detail & finish and the cleanup stages (CL-009 to CL-012, CL-033, CL-034, slice K7 below), the white-balance picker (CL-018), scopes (CL-021), Compare (CL-023), Apply to all (CL-025), Match all clips (CL-039), and HDR options (CL-041). **Stays later:** CL-022, CL-027, CL-029, CL-032, CL-036 to CL-038, CL-040, CL-042.

**Demo.** A user selects a clip, warms and brightens it with a keyframed exposure ramp, pulls the greens toward teal on the HSL page, draws an S-curve, pushes the shadows wheel toward blue, imports a folder of `.cube` LUTs (the broken ones are listed with line numbers), applies one at 60 % with skin protection, runs Auto and matches the shot to another clip, saves the grade, drops it as an adjustment clip over three clips, limits it to an ellipse, and exports a file whose frames equal the preview.

**CPU color pipeline (decided 2026-10-13, README C52; details in render-compositor.md §17).**
- *Working space:* display-referred Rec.709 code values, straight alpha, as decoded (CL-002). Linear light (γ 2.4) only inside exposure, white balance and deflicker gain; Oklab for hue work.
- *Order:* CL-001's stage table, unchanged. Auto offsets fold into slider values; skin protection blends inside stages 5, 11–12 and 13.
- *Precision:* 8-bit in, `f32` through the stack with `libm` math, one rounding to 8 bits at the end, no dither.
- *Bake:* the per-pixel run of stages (4–14, plus 16 and the Amount blend when nothing spatial or masked follows) becomes one lattice over the 8-bit input cube, 33³ (65³ for steep curves or fine user LUTs), applied with tetrahedral interpolation. Identity stacks are skipped; a LUT-only stack uses the user's table directly. Bakes are cached by a digest of the evaluated values.
- *Budget at 1080p, release build, 8 threads, reference machine:* a 33³ bake ≤ 3 ms, grading one full-frame layer ≤ 4 ms, an adjustment clip over the whole frame ≤ 4 ms, each mask ≤ 2 ms; a frame with one graded 1080p clip and one adjustment clip stays under 16 ms (today's three-layer frame is about 5 ms).
- *Keyframes:* `PropPath::Color` keys in `ClipKeys`, evaluated by `Clip::color_at`; `frame_plan` puts the evaluated stack on `LayerPlan.color` and emits `PlanNode::Adjust` for adjustment clips.
- *Export parity:* preview and export use the same compositor and plan; every color pass is per-pixel independent and banded across threads with identical arithmetic, so a frame's bytes do not depend on thread count, frame order or which of the two drew it.

**Slices, in build order.** Each slice ends with `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` green, an apply → undo → redo test for every new edit, nothing decoded, rendered or read from disk on the UI thread, and each project-format addition behind its own version bump with an additive migration step that adds identity defaults.

- **K1 — Color stack model, Light and Color, CPU grading, Basic page.**
  - *Crates:* `timeline`, `render`, `project`, `app`.
  - *Contracts:* `freecut_timeline::model::color` with `ColorStack { sections_enabled: SectionMask, bypass: bool, amount: u8, basic: BasicAdjust }` (later slices add fields), `BasicAdjust` (the fourteen `i16` sliders of CL-007 and CL-008), `Section`, `SectionMask`, `ColorParamId` and `color_param_spec(ColorParamId) -> ColorParamSpec { min, max, default }` (CL's `ParamSpec` table, named apart from the effect `ParamSpec` of README C37), `ColorStack::{is_identity, clamped}`; `Clip.color: Option<Arc<ColorStack>>` with the validation rule of CL-001's CPU-path note; `PropPath::Color`, `ValueKind::Level`, `PropValue::Level` and `Clip::color_at` (CL-028); `EditRequest::EditColor { seq, clips, edit: ColorEdit }` with `ColorEdit::{SetParam(ColorParamId, i16), SetParams(Vec<(ColorParamId, i16)>), SetSectionEnabled(Section, bool), SetBypass(bool), ResetSection(Section), ResetAll}`, skipping ineligible clips, refusing when none is eligible, and removing a reset section's keys in the same entry; `LayerPlan.color: Option<Arc<ColorStack>>` (evaluated; `None` when absent, identity or bypassed). In `freecut-render`: `color::eval_pixel(&ColorStack, [f32; 3]) -> [f32; 3]` (the reference evaluator, also the oracle), `color::bake(&ColorStack, BakeSize) -> Arc<Lattice>`, `color::grade(&CompiledStack, &Picture, threads) -> Picture`, `CompiledStack` cached by `ColorDigest` (XXH3-128 of the canonical evaluated values plus `COLOR_ALGO_VERSION`), and the compositor's `GradeCache` (8 entries) keyed by source picture identity and digest. The next project-format version (7 when this was written) adds an optional `color` object to the clip DTO, with default sections omitted; it serves every clip kind that may carry a stack, adjustment clips included. App: the Color tab with the header row (bypass switch, Amount), the page strip showing only Basic, and the Light and Color sections with rails, number fields, reset badges, switches and keyframe navigators; `ColorDraft` on the preview request for live drags.
  - *Tests:* `ColorParamSpec` ranges and defaults; clamping; `is_identity`; every `ColorEdit` apply → undo → redo; section reset removes its keys and undo restores them; `color_at` interpolation and rounding; `validate` refuses a stack on a text clip. Render: a forced identity bake returns every 8-bit input unchanged; lattice versus `eval_pixel` on a 32³ sample of 8-bit inputs for a set of corner stacks, max error ≤ 2/255 and mean ≤ 0.4/255; all sliders at ±100 stay finite (property test); exposure is monotonic; Hue +100 equals −100; Saturation −100 gives R = G = B; temperature and tint keep Rec.709 luma within 2 % on a grey ramp; 1 and 8 threads give identical bytes; preview `Compositor` and `ExportRenderer` give identical bytes for the same plan; checked-in golden PNGs of a FreeCut test chart per slider extreme. Project: round trip, a format-6 file opens with no stack, schema-hash lock updated.
  - *Acceptance:* Exposure +50 on a 1080p clip shows while dragging and commits one entry on release; undo restores it; a keyframed temperature ramp plays and exports identically; the `color_bench` ignored test meets the K1 budget lines.
- **K2 — HSL, Curves and Wheels pages.**
  - *Crates:* `timeline`, `render`, `project`, `app`.
  - *Contracts:* `Hsl { bands: [HslBand; 8] }`, `Curves { luma, red, green, blue: Curve }` with `Curve::new(points)` validating 2–16 points, x strictly increasing with gaps ≥ 100 (0.01), the endpoint x limits of CL-014; `ColorWheels { shadows, midtones, highlights, offset: Wheel, strength: u8 }` with the disk invariant enforced by `Wheel::new`; `ColorParamId::{Hsl { band, channel }, WheelLuma(WheelId), WheelStrength}`; `PropPath::ColorPuck(WheelId)` with `PropValue::Puck`; `ColorEdit::{SetCurve { channel, points }, SetPuck { wheel, x, y }}`. Render: stages 8, 9 and 11 in `eval_pixel`; curves as 1024-entry `f32` tables (Fritsch–Carlson); `BakeSize` 65 when a baked curve's steepest segment exceeds a slope of 4; `color::probe_hsl_input(stack, picture, point) -> [f32; 3]` (5×5 average) for CL-013's eyedropper. The next project-format version adds `hsl`, `curves` and `wheels`. App: the HSL page (eight swatches with dots, three rails, eyedropper on the player), the Curves page (an egui painter editor with channel tabs; the histogram behind it waits for CL-021), the Wheels page (2 × 2 wheels with puck drag and fine drag, luminance rails, Strength).
  - *Tests:* band weights sum to 1 and greys never shift; the hue wrap at 0°/360° is continuous; a curve never overshoots between points; a 17th point is refused; puck moves keep Rec.709 luma; corner-value property with every new control at its extremes; a steep curve baked at 65 meets the accuracy bound; apply → undo → redo for the new edits.
  - *Acceptance:* the three pages edit live in preview, and the exported frames match the preview's.
- **K3 — LUTs: import, library, bundled set, LUT section, skin protection.**
  - *Crates:* `media`, `project`, `timeline`, `render`, `app`.
  - *Contracts:* `freecut_media::lut::{parse_cube, parse_3dl, Lut, LutError}` (no FFmpeg); `freecut_project::luts::{LutLibrary, LutEntry}` (the app-data index: read, write, `add_file(bytes, entry)` that stores the copy by hash; no parsing, because `freecut-project` may not depend on `freecut-media`) and `freecut_app::jobs::luts::import(paths) -> ImportReport` (on a worker: reads each file, validates it with `freecut_media::lut`, then adds it through `LutLibrary`; architecture C54); `LutRef`, `LutSlot`, `LutInterp`, `SkinProtect`, `SkinScope`, `ColorParamId::{LutIntensity, SkinAmount(SkinScope)}`, `ColorEdit::{SetLut(Option<LutSlot>), SetSkin { scope, on }}`; `freecut_render::color::LutSource` (`fn lut(&self, hash: &[u8; 16]) -> Option<Arc<Lut>>`) handed to `Compositor::with_luts`, with `take_missing_luts()` for badges and export preflight; the LUT-only fast path; the skin mask in `eval_pixel` and the bake; `assets/luts/` with recipes, manifest and the eight files of CL-045. The next project-format version adds `lut` and `skin`. App: the LUT section (select grouped None / Bundled / Imported / Import LUT…, Intensity, Skin protection and its Amount, interpolation under More), the Color section's Skin protection rows, the import job with its summary.
  - *Tests:* parser fixtures written by hand: identity 2³, an inverting 17³, a 1D 1024, a shaper with a 3D table, domain 0–2, the input-range variant, CRLF with BOM, comments and a title; one failing fixture per rule of CL-016's validation list, each checking the line number and reason; `.3dl` at 10 and 12 bits, float, and a mismatched mesh. Importing the same file twice keeps one entry. Tetrahedral and trilinear both reproduce an identity table exactly. The skin mask is high on skin patches and zero on sky, foliage and grey. The fast path agrees with the bake path within 1/255. A missing LUT renders identity and is reported. The bundled files re-bake byte-identically.
  - *Acceptance:* importing 50 files with 3 broken ones adds 47 and lists the 3 with line numbers; a LUT at 60 % with skin protection keeps faces natural in preview and export.
- **K4 — Adjustment clips, saved looks and the Adjust tab.**
  - *Crates:* `timeline`, `render`, `project`, `app`.
  - *Contracts:* `EditRequest::AddAdjustmentClip` (CL-044) with CL-006's span and placement rules; `ColorEdit::ApplySections(Arc<SectionSnapshot>)` (CL-025's `ReplaceColorSections`); `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })`, with the adjustment clip's transform and opacity ignored; the compositor's flush-then-grade step in canvas space at output resolution, blended by Mix; `freecut_project::look::{LookFile, read_look, write_look}` for `.fclook` version 1 (sections Light, Color, LUT, Filter, HSL, Curves, Wheels; never Auto, Match, Masks, cleanup or Amount). No project-format change: an adjustment clip's stack is K1's `color` field. App: the Adjust tab (CL-044), the Save look… dialog on UI-064, the Presets select in the Color tab header.
  - *Tests:* an adjustment clip changes only tracks below it; a hidden adjustment track changes nothing; Mix 0 is identity; the canvas background under it is graded; two stacked adjustment clips apply bottom-up; a blank adjustment clip renders nothing; placement on a free existing adjustment track versus a new track; look files round-trip, and a newer look version is listed as unavailable; `ApplySections` apply → undo → redo.
  - *Acceptance:* the demo's adjustment-clip steps, with preview equal to export.
- **K5 — Auto and Match.**
  - *Crates:* `render`, `media`, `timeline`, `project`, `app`.
  - *Contracts:* `AutoCorrection`, `AutoOffsets`, `MatchTransform` (CL-019, CL-020); `ColorParamId::{AutoStrength, MatchIntensity}`; `ColorEdit::{SetAuto(Option<AutoCorrection>), ApplyAutoToSliders, SetMatch(Option<MatchTransform>)}`; `ColorStack::effective_basic()`; `freecut_render::color::stats::{frame_stats, auto_color, oklab_stats, match_stats}` (pure, `f64`, fixed summation order) and `color::stack_input(&ColorStack, &Picture) -> Picture` (cleanup and match only); `freecut-app::jobs::color` sampling 5 frames in exact mode at analysis size on a worker, through PM-073's task list, with a staleness check at commit. The next project-format version adds `auto` and `match`. App: the Auto and Match sections, pick mode for the match reference (timeline frame or image file).
  - *Tests:* synthetic casts move toward neutral; a uniform frame gives "Nothing to correct"; a re-run gives identical offsets; Strength scales the effective sliders; Apply to sliders adds and clamps; matching a clip to itself is identity; gains are limited as CL-020 says; repeated runs and thread counts give identical results; a clip edited during the job is skipped at commit.
  - *Acceptance:* Auto and Match produce visible, undoable, keyframable results that export identically.
- **K6 — Color masks.**
  - *Crates:* `timeline`, `render`, `project`, `app`.
  - *Contracts:* the shared mask core in `freecut_timeline::model::mask` (TR-017's field set in fixed point: `Mask { shape: MaskShape::{Linear, Band { width }, Ellipse, Rectangle { roundness }}, center, size, linked, rotation, feather, invert }`, with TR-017 and TR-018's ranges; Heart and Star are added by TR-017 later), `ColorMask { id, mask, expand, mode: MaskCombine }`, `ColorStack.masks` (at most 8), `PropPath::ColorMask { id, param }`, `ColorEdit::{AddMask, RemoveMask, ReorderMask, SetMask, SetMaskMode}`; `freecut_render::shape::mask::coverage` and the final blend of CL-031's CPU-path note. The next project-format version adds `masks`. App: the Masks page (chips, Add mask with four shapes, rows, combine mode, invert, delete) and on-canvas handles in `ui::stage` for center, size, rotation and feather (one entry per drag, Esc cancels).
  - *Tests:* coverage goldens per shape; feather is monotonic; invert is the complement; the three combine modes; a zero-size mask hides the grade; masks never change alpha; clip masks follow the clip's transform; adjustment-clip masks stay in canvas space; Reset all removes masks after naming the count.
  - *Acceptance:* a grade limited to an ellipse and keyframed to follow a subject plays and exports identically.
- **K7 (optional, after this round) — Detail & finish.** CL-009 to CL-012 as CPU passes after the lattice (separable blurs shared with the text effects, the vignette and the seeded grain of CL-011), then CL-033 and CL-034. Not part of the user's request; listed so the pipeline above leaves room for them.

## Interactions with other areas

- **Timeline & core editing (TL, [timeline-editing.md](timeline-editing.md)):** Owns time representation, items, tracks and track kinds (including the `Adjustment` track kind), layering (strict layering by default, TL-004, and **Flexible stacking**, TL-007), sequence duration (TL-003), selection, locking, visibility, split, duplicate, the item clipboard, and undo/redo semantics (TL-043). Also owns **Paste attributes** (TL-034): the command, the attribute clipboard and the `PropBag::category()` registry, into which CL registers its color sections (CL-025). CL owns the content of `Adjustment` items and the per-clip color bags, plus the color ops. One UI gesture is one undo entry, and slider drags commit once on release via transient render overrides. Split and duplicate copy color stacks by value.
- **Transform, canvas, masks, keying, keyframes & speed (TR, [clip-transform-and-speed.md](clip-transform-and-speed.md)):** Owns keyframes and easing through the generic `Animatable<T: Interpolate>` (TR-027), and records once, in its shared conventions, the project-wide rule that only time must be rational; CL's fixed-point types plug into that engine and CL registers `ColorParamId`s. Owns masks: the `Mask` type, its shapes, feather, invert, combine modes (TR-017–TR-020) and the on-canvas mask editor. CL-031's color masks reuse all of these and add only Expand. Slice K6 of Adjustments round 1 builds the shared mask core (Linear, Band, Ellipse and Rectangle, feather, invert, combine modes, on-canvas handles) in TR's names, so TR-017 to TR-019 extend it rather than duplicate it. A painted (brush) color mask waits for TR to define a stroke-based mask shape. Color keyframes use TR-027's `ClipKeys` with CL's property paths (CL-028). Owns canvas background fill (graded by adjustment clips) and speed/retiming: grain seeds and keyframes follow clip/source time, and CL-033/CL-034 find their neighboring source frames through the time map (TR-036) and frame sampling (TR-041). Stabilization (TR-026) runs at L3, before the stack, so temporal neighbors are already aligned.
- **Effects, transitions & clip animations (FX, [effects-transitions-animations.md](effects-transitions-animations.md)):** Owns the per-frame render and compositing order (FX-001), which walks TL's validated bottom-to-top track order. The color stack is layer stage L5 (after crop and keying, before clip effects, mask and transform), so grading never breaks a key. Denoise therefore runs after keying too and does not clean the key's input. The temporal stages (CL-033, CL-034) need L1–L4 evaluated for neighboring source frames as well; FX-001's walk and stage order are unchanged. Adjustment clips render at FX-001 step 6, the same position as FX's effect layers (FX-011). Transitions blend each clip's already graded frames, and an adjustment clip above a transition grades the blend. FX also owns the determinism rules that the grain seed and the temporal stages follow. FX-001's HDR edge case points to CL-002. Glow, blur and other image effects are FX's (FX-015). CL's soften, texture and clarity are the only blur-based controls in the color stack. CL-042 (later) asks FX-001 for an alternate slot after L6, so a grade can run after clip effects. FX decides whether to add it.
- **Projects & media library (PM, [project-and-media.md](project-and-media.md)):** Owns media references, probe data (color tags used by CL-002), the Interpret dialog (PM-040) whose color part, including CL-041's HDR options, is CL's `ColorOverride`, proxies (used by auto color, match and scopes when originals are offline; never by deflicker analysis; marked Stale when CL-041 settings change), image sequences imported as video (deflicker's main timelapse case), background jobs (PM-073, which also run multi-clip auto color and match groups, CL-019 and CL-039) and the media cache (PM-074) that holds deflicker analysis, and project collection and relink. LUT files are a referenced asset type here: hash plus file name, with relink by hash.
- **Preview player & export (EX, [preview-and-export.md](preview-and-export.md)):** Export tags BT.709 color metadata (CL-002). Preview quality and resolution scaling affect scopes and spatial stages (radii are resolution-independent). Temporal denoise raises decode cost, and preview may lower its resolution but never skips the pass. Export waits for pending deflicker analysis, and its preflight lists missing LUTs or looks and clips whose deflicker cannot run because the original media is offline. The preview-only "Disable all color" switch is ignored on export.
- **Overlays, stickers, shapes & templates (OV, [overlays-and-stickers.md](overlays-and-stickers.md)):** Raster stickers and image overlays are visual items with a color stack. Vector shapes, like text, are not. Templates (OV) may include looks by `PresetId`. OV-043's local brand kit holds logos and a color palette. Shared grading presets are CL's user looks and look bundles (CL-026, CL-027), not part of the kit.
- **Text, titles & captions (TX, [text-and-captions.md](text-and-captions.md)):** Text has no color stack but is affected by adjustment clips above it. Text colors are sRGB and enter the working space defined in CL-002. Text style presets are TX's own system, parallel to user looks, and CL defines nothing for them.
- **Smart / AI-assisted tools (AI, [ai-tools.md](ai-tools.md)):** ML-based relighting, enhancement (AI-019's upscaling and ML denoising as derived media), colorization and face-aware skin tools belong there. CL-019 auto color, CL-020 color match, CL-033 denoise and CL-034 deflicker are classic, model-free and live. AI-019 and AI's interactions hand "classic video noise reduction" and "flicker removal" to CL, and those handoffs land on CL-033 and CL-034. Denoise can be used together with an AI-019 enhanced clip; it then filters the derived media like any source. CL-036 dehaze and CL-037 custom color targets are classic too. An edge-following brush for color masks (CL-031) would reuse AI-015's refinement prompts and model. ML detail restoration belongs to AI-019, and face-aware skin smoothing to AI-020. Color masks can follow AI-008 motion-track data like any TR mask. AI relighting and its presets stay in AI's Out of scope.
- **Workspace layout, inspector & shortcuts (UI, [workspace-and-shortcuts.md](workspace-and-shortcuts.md)):** CL defines the contents of the Filters and Adjust tabs of UI-004 (CL-003, CL-044) and the pages and sections of the Color tab (CL-043), built on UI-006 and UI-007's grids, UI-064's save dialog and the UI concept's section grammar. Owns the inspector tab layout, Scopes panel docking (UI-056, opened by the Color tab's Scopes button), control conventions (UI-009: clamping with a flash, disabled controls that explain why), multi-selection editing (UI-010), the storage of UI preferences such as the remembered **More** disclosures and Auto's Style and Strength (UI-018), and key bindings (compare toggle, fine-drag modifiers, reset, and **Paste attributes…** on `Mod+Alt+V`, which ships in v1 with TL-034).
- **Audio (AU, [audio.md](audio.md)):** No interaction. Adjustment clips never affect audio.

## Out of scope (and why)

- **Online or downloadable filter stores, premium/locked looks, accounts:** FreeCut is local-only and free. All looks are bundled or user-made. This covers the reference app's on-demand filter downloads and their failed-download/retry state, paid or store-sold filter packs, paywall badges on color tools (masks, match, auto color), and rules that keep premium items out of saved presets or out of color match. Local substitute: bundled CC0 looks (CL-005), user LUTs (CL-016) and user looks (CL-026). Every tool is available to everyone, and any local frame or image can be a match reference (CL-020).
- **Favorites, presets or looks tied to a sign-in or synced through a cloud:** No accounts. Favorites are a local app preference (CL-003), and user looks are local files (CL-026). Moving them between machines is a file copy or a look bundle (CL-027).
- **Team spaces and brand-kit presets (shared adjustment and text-style presets, capped at 200 per type in a paid tier):** No collaboration server and no caps. Local substitute: user looks with no count limit (CL-026), shared as `.fclook` / `.fclooks` files (CL-027), plus OV-043's local brand kit for logos and palette. Text style presets are TX's.
- **Publishing looks to a creator marketplace (single filters or packs, with covers and review):** No store, review or pricing. Local substitute: Export look… and Export as .cube… (CL-027), which produce ordinary files the user can share however they like.
- **Assistant and agent color suggestions (an assistant panel that proposes filters, or a cloud pass that improves the colors of clips it flagged):** These need a cloud assistant. Local substitute: multi-clip auto color with **Only clips that need it** (CL-019), and match groups (CL-039). Once CL's edit requests exist, the opt-in assistant (AS-006, assistant.md) can propose color changes as ordinary proposals that the user accepts; FreeCut still runs no cloud pass of its own.
- **Cloud style transfer and generated looks:** Server-side models with credits. FreeCut's looks, LUTs and FX effects cover stylized looks (see AI's Out of scope).
- **Version-gated presets with forced update prompts:** FreeCut never gates a feature on an app update. A look from a newer FreeCut is listed as unavailable with the reason (CL-026).
- **An HDR working space, HDR grading and HDR/wide-gamut output (PQ/HLG, P3, BT.2020), ACES/OCIO pipelines:** Large and niche for the everyday target. SDR Rec.709 is the single working and output space, and HDR inputs are tone-mapped to SDR (CL-002, with v1 options in CL-041). This also excludes what the reference app's UI shows: a switchable project working color space; SDR output in BT.2020 primaries, or 10-bit HDR output with the HLG or PQ transfer of BT.2100; a vivid-HDR mode; an output peak-brightness setting; SDR-to-HDR up-conversion; and HLG↔PQ clip conversion. EX lists HDR export in its own Out of scope; HDR output would only be revisited together with an HDR working space here, never on its own. OCIO may be revisited after v1.
- **An optional precision mode:** The reference app offers floating-point color processing as an opt-in mode at a performance cost. FreeCut always processes in floating point (16-bit float on the GPU path, `f32` on the CPU path; CL-001), so there is no switch to offer.
- **ML denoising and ML flicker repair:** These need models and belong to AI (AI-019 produces ML-denoised derived media). CL-033 and CL-034 stay classic and model-free so they can run live and identically in preview and export.
- **Recursive (feedback) temporal denoising:** Its output depends on which frames were rendered before, so scrubbing and export could differ from playback. CL-033 uses a fixed, non-recursive window instead.
- **Node-based grading graphs:** The fixed stage order plus stackable adjustment clips covers the everyday workflow, and a node graph would add a lot of complexity.
- **Color-chart (ColorChecker-style) calibration:** Niche, and needs chart detection.
- **Display calibration / ICC-managed preview:** OS- and hardware-specific. The preview shows working-space values directly (CL-002).
- **ML relighting (and relighting presets), AI colorization, AI style transfer, face-aware skin smoothing, ML detail restoration:** These belong to the AI area (local open models, if any). Relighting is in AI's Out of scope; colorization is AI-028, detail restoration AI-019, skin smoothing AI-020.
- **Tracking power windows:** Masks and tracking belong to TR/AI. CL-031 reuses TR's mask type and AI-008's track data and adds no tracker of its own.
- **Three-step (weak / standard / strong) detail controls:** Continuous sliders cover them (CL-001 conventions).
- **Bundling any third-party commercial or proprietary LUTs or filters:** Licensing. See the clean-room notes.

## Clean-room notes

- **Looks/filters:** FreeCut contributors write every bundled look as an original recipe (CL-005). Both the recipes and the baked `.cube` files are licensed **CC0-1.0** and live under `assets/looks/`. Look names follow FreeCut's own "category + descriptor + number" scheme. No CapCut preset name, look, LUT or thumbnail is copied or imitated by sampling.
- **LUTs:** We ship no third-party LUT packs. The eight bundled LUTs (CL-045) are generated from recipes FreeCut contributors wrote, by FreeCut's own CPU stack, and are CC0-1.0 under `assets/luts/`; none is sampled from, measured against or named after a reference-app LUT or filter. Users import their own `.cube`/`.3dl` files. A third-party pack would be accepted only if it is explicitly CC0 or CC-BY, with the license and attribution recorded in `assets/ASSETS.md`. The default is none.
- **Sample image for thumbnails (CL-003):** A FreeCut-made CC0-1.0 test image (color patches plus a contributor-shot photo with a signed model release), with no stock or CapCut imagery.
- **Algorithms:** All are from public literature or open specs, implemented from scratch:
  - Lift/gamma/gain/offset (as in the published ASC CDL concept)
  - Fritsch–Carlson monotone cubic interpolation
  - Lattice baking of per-pixel color transforms with tetrahedral lookup (the standard 3D-LUT technique; CL-001's CPU path)
  - Hue/saturation/value range masks for skin protection (CL-030), with FreeCut's own range constants
  - Signed-distance shape masks with smoothstep feathering (CL-031)
  - Tetrahedral and trilinear 3D-LUT interpolation
  - Gray-world and white-patch white balance
  - Reinhard et al. statistics-based color transfer
  - The Oklab color space (public, permissively licensed reference)
  - Rec.601, Rec.709 and Rec.2100 equations
  - Camera vendors' publicly published log formulas (CL-032)
  - The bilateral filter (Tomasi and Manduchi, 1998) and motion-adaptive temporal averaging, for denoise (CL-033)
  - Moving-average brightness equalization, for deflicker (CL-034)
  - The dark channel prior (He, Sun and Tang, 2009) and the guided filter (He, Sun and Tang, 2010), for dehaze (CL-036)
  - The BT.2390 EETF, BT.2100 HLG equations and BT.2408 reference white, for HDR-to-SDR conversion (CL-002, CL-041)
- **Denoise and deflicker:** Our own WGSL and Rust code, with no model weights and no downloaded assets. FFmpeg's public filter documentation (its deflicker, hqdn3d, atadenoise and bm3d entries) and Kdenlive's list of noise-removal effects were read only to see which controls and window sizes are common. FreeCut does not call those filters, because the live GPU path must match export exactly. Names such as "Denoise", "Deflicker" and "Even out color" are plain generic terms.
- **File formats:** `.cube` follows Adobe's public Cube LUT 1.0 specification. `.3dl` follows publicly documented layouts. The parsers are our own code. FFmpeg's and Shotcut's supported-format lists were used only to cross-check which formats matter.
- **UI:** Our own icons, swatches and layout, with no CapCut artwork or screenshots. Slider names are generic photographic terms, and the combined shadow-lift/highlight-compress control has FreeCut's own name, **Tone lift**. Other labels in this spec (Amount, Pivot, Texture, Dehaze, Pick color, Range, Look settings, Match all clips, Color position, Only clips that need it, Skin protection, Apply to sliders, Adjustment clip, Saved, Bundled, the page names Basic, HSL, Curves, Wheels and Masks) are FreeCut's own wording or plain generic terms. None is copied from the reference app's UI text; where the reference app's label for the same control differs only slightly, FreeCut picked a different phrasing on purpose (its skin-tone toggle, its automatic-adjustment and blank-adjustment entries). None is copied from the reference app's UI text.
- **Research method:** Public web pages (help and marketing pages, third-party tutorials, other editors' docs), plus a paraphrased inventory of the English UI labels of a local CapCut desktop 9.5.0 install, taken from its bundled English localization file (see Provenance in the [README](README.md)). Nothing was decompiled, disassembled or traced, and no assets, traffic, project files or APIs of CapCut were inspected. No label text, preset name or filter name was copied. The inventory only told us which capabilities, limits and option lists exist (for example the two LUT formats, the 10-custom-color limit, the forbidden preset-name characters), and every one is described here in our own words.

## Sources

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- ITU-R BT.2390, High dynamic range television for production and international programme exchange (ITU report) — https://www.itu.int/pub/R-REP-BT.2390
- ITU-R BT.2408, Guidance for operational practices in HDR television production (ITU report) — https://www.itu.int/pub/R-REP-BT.2408
- Master Color Grading in CapCut (CapCut resource page) — https://www.capcut.com/resource/capcut-color-grading
- Effortlessly Change Colors in Video with CapCut's Tools (CapCut resource page) — https://capcut.com/resource/how-to-change-colors-in-video-with-capcut
- Unlock Professional Looks with LUTs for Color Grading (CapCut resource page) — https://www.capcut.com/resource/luts-for-color-grading
- Unlock Creative Potential with HSL Color Grading (CapCut tool page) — https://www.capcut.com/tools/hsl-color
- Enhance Video with Color Wheel (CapCut tool page) — https://www.capcut.com/tools/color-wheel
- CapCut Filter (CapCut resource page) — https://capcut.com/resource/capcut-filter
- Color Grading vs Color Correction (CapCut resource page) — https://www.capcut.com/resource/color-grading-vs-color-correction
- A Comprehensive Guide to Keyframes in CapCut PC (CapCut resource page) — https://www.capcut.com/resource/how-to-add-keyframes-in-capcut
- AI Color Matcher (CapCut tool page, search result) — https://capcut.com/tools/match-color-with-ai
- How to Use Adjustment Layers to Apply Effects to Multiple Clips (CapCut "create" page) — https://www.capcut.com/create/adjustment-layers-for-multiple-clips
- How to color grade video in CapCut (Filmora) — https://filmora.wondershare.com/advanced-video-editing/how-to-color-grade-video-in-capcut.html
- HSL in CapCut (Filmora, French) — https://filmora.wondershare.fr/video-editor-review/hsl-capcut.html
- The Ultimate Guide to LUTs in CapCut (Miracamp) — https://www.miracamp.com/learn/capcut/the-ultimate-guide-to-luts
- How to Copy Effects Between Clips in CapCut (Miracamp) — https://www.miracamp.com/learn/capcut/how-to-copy-effects-between-clips
- Loading LUTs in CapCut Desktop (SparkleStock docs) — https://sparklestock.com/docs/load-capcut-desktop-luts
- How to quickly add a LUT to CapCut (Gamut, search snippet only) — https://gamut.io/how-to-quickly-add-a-lut-to-capcut/
- What Is the Free Layer in CapCut? (TourBox) — https://www.tourboxtech.com/en/news/what-is-free-layer-in-capcut.html
- CapCut for PC & Mac: Desktop Tutorial for Beginners (Primal Video) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- CapCut Tutorial Class 12: Colour Grading (CG Heaven) — https://cgheven.com/tutorials/capcut-tutorial-for-beginners-class-12-colour-grading-in-capcut-english
- Cinematic Color Grading in CapCut, transcript (GoTranscript) — https://gotranscript.com/public/transform-your-videos-a-step-by-step-guide-to-cinematic-color-grading-in-capcut
- Quick Tip: CapCut Color-Grading (College of Wooster tech blog) — https://inside.wooster.edu/technology/?p=47681
- How to Quickly Add Filters to Videos in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-quickly-add-filters-to-videos-in-capcut--cms-108764
- How to Adjust Highlights and Shadows in CapCut (Envato Tuts+) — https://photography.tutsplus.com/tutorials/how-to-adjust-highlights-and-shadows-in-capcut--cms-108825
- LUT (3D) filter documentation (Shotcut forum docs) — https://forum.shotcut.org/pub/lut-3d
- Color Grading Control: Lift Value (Shotcut forum) — https://forum.shotcut.org/t/color-grading-control-lift-value/5534
- HSL Primaries effect (Kdenlive docs) — https://docs.kdenlive.org/effects_and_filters/video_effects/color_image_correction/hsl_primaries.html
- Waveform and RGB Parade scopes (Kdenlive docs) — https://docs.kdenlive.org/de/tips_and_tricks/scopes/waveform_and_rgb_parade.html
- Apply LUT effect (KDE UserBase, Kdenlive manual archive) — https://userbase.kde.org/Kdenlive/Manual/Effects/Colour_Correction/ApplyLUT
- FFmpeg lut3d filter introduction (ffmpeg-devel mailing list, 2013) — https://ffmpeg.org/pipermail/ffmpeg-devel/2013-May/144014.html
- FFmpeg ticket #10431 on .cube DOMAIN keywords — https://fftrac-bg.ffmpeg.org/ticket/10431
- Iridas/Adobe .cube reader (colour-science docs) — https://colour.readthedocs.io/en/develop/_modules/colour/io/luts/iridas_cube.html
- LUT effect docs, 3D size limits (Remotion) — https://www.remotion.dev/docs/effects/lut
- Four Automatic Color Correction Features in DaVinci Resolve 20 (Larry Jordan) — https://larryjordan.com/articles/the-automatic-color-correction-features-in-davinci-resolve-20-v/
- How to Color Match Shots in DaVinci Resolve 20 (Larry Jordan) — https://larryjordan.com/articles/how-to-color-match-clips-in-davinci-resolve-20/
- Color Match in Resolve (Emerson College support) — https://support.emerson.edu/hc/en-us/articles/21709302725531-Color-Match-in-Resolve
- AI Video Enhancer, desktop (CapCut tool page; lists noise reduction among clean-up tools) — https://www.capcut.com/tools/ai-video-enhancer
- FFmpeg Filters Documentation (deflicker, hqdn3d, atadenoise, bm3d sections) — https://ffmpeg.org/ffmpeg-filters.html
- Grain and Noise video effects (Kdenlive docs) — https://docs.kdenlive.org/en/effects_and_filters/video_effects/grain_and_noise.html
