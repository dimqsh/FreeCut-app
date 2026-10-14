# GPU compositor, effects, transitions & blend modes

> Status: draft · Crates: freecut-render (reads `FramePlan` from freecut-timeline, decoded frames from freecut-media; hands finished frames to the preview engine and the export job)

This document designs the per-frame compositor: how a `FramePlan` for time `t`
becomes a wgpu pass graph, how each layer is normalized, keyed, graded, masked,
transformed and blended, how transitions and effect layers mix, how textures are
pooled, and how color is handled at the input and output edges.

It builds on two sibling designs and does not repeat them:
- `docs/architecture/timeline-model.md` owns `Tick`, the document model, transition
  geometry (`Coverage::Transition { elapsed, duration, .. }`) and the pure queries
  that build a `FramePlan`.
- `docs/architecture/playback-engine.md` owns the preview scheduler, the `PresentRing`
  (4 × `Rgba8Unorm` slots), the egui paint callback, the RAM/disk render caches and
  the `render_key`.

**Decisions at a glance**
1. **Working space (as the specs fix it):** premultiplied `Rgba16Float`, Rec.709
   primaries, display-referred (gamma-encoded) code values. Linear light only
   inside the operations that need it: exposure, white balance, gamut and HDR
   conversion.
2. **One fixed stage order** (FX-001 L1–L10), enforced in one place: a
   `LayerChain` builder that refuses an out-of-order stage.
3. **Two textures per layer at most on the common path.** The source is
   normalized once (YUV to RGBA16F, cached). Then a single *place* pass does mask,
   transform, edge antialiasing, opacity and Normal blend. It uses inverse
   mapping, so 3D (projective) presets and perspective come for free.
4. **Blend modes:** Normal uses fixed-function premultiplied source-over. The other
   15 modes use the W3C Compositing Level 1 formula in a shader that reads a copy of
   the backdrop's bounding box.
5. **Transitions:** two transparent canvas-sized layers go through a registered
   WGSL shader. Progress is exact mid-frame (FX-002), and easing gives
   complementary weights. The result is blended once with the incoming clip's mode.
6. **YUV upload:** planes become `R8Unorm`/`Rg8Unorm`, or `R16Unorm` when the
   feature is present. A single 3×4 affine is computed in Rust from (matrix, range,
   bit depth, packing), and the frame's own chroma siting is honored. We do not use
   wgpu's `NV12`/`P010` formats, because they are unavailable on Metal.
7. **Rendering only, no compute on the core path.** This works on the GLES fallback
   (UI-028). Compute is used only for scopes, with a CPU fallback.
8. **The renderer never evaluates keyframes or time maps.** It receives evaluated
   values plus exact ticks. It converts to `f32` only when it writes uniforms.

---

## What FreeCut needs

### Functional requirements

| Area | Spec IDs | What the compositor must do |
|---|---|---|
| Frame pipeline | FX-001 | Evaluate the frame at rational `t`. Clear to the background, then walk visible tracks bottom → top. Per visual layer run L1 Source sample → L2 Normalize → L3 Stabilize+crop → L4 Key → L5 Color → L6 Clip effects → L7 Mask → L8 Transform → L9 Opacity → L10 Blend. Transition pairs, effect layers and adjustment layers sit at fixed positions. One shader set serves preview and export. |
| Transitions | FX-002, FX-003, FX-004, FX-005, FX-008, FX-023 | Exact mid-frame progress `(elapsed + frame/2) / D`. Easing is clamped for dissolve kinds and passed through for motion kinds. About 40 kinds: dissolve, dip, wipes, iris, push/slide, zoom, 3D card/cube, light, glitch, and luma maps from user images. Unknown kinds fall back to dissolve. A shader that fails to compile hides the kind and renders a dissolve. |
| Clip effects & stack | FX-010, FX-012, FX-013, FX-014, FX-015 | An ordered effect list in layer space, from manifests with typed parameters. Up to 8 frames of history for temporal effects. Multi-pass effects declare intermediates. Large radii are downsampled internally. Each effect gets a 32×32 golden test. |
| Effect / adjustment layers | FX-011, CL-006 | Input is the accumulator (everything below, including the background). Output replaces it, mixed by a keyframable Mix. "Main track only" is resolved by the planner into clip effects. |
| Animations | FX-018, FX-019, FX-022 (later) | Translation, scale, 2D rotation, 3D rotation with perspective, opacity, blur and wipe reveal are folded into L8/L9. Motion blur samples L8 at sub-frame times (later). |
| Transform & framing | TR-001…TR-006, TR-002 | Flip, then scale and rotate about the anchor, then translate. Fit/Fill/Stretch/Original use the cropped display size as the 100 % baseline. Positions are fractions of the canvas. |
| Crop | TR-009, TR-010 | Crop is a sub-rectangle in source space plus a straighten angle, applied before masks and keys. |
| Background | TR-011…TR-014 | Solid color, blurred cover-scaled copy (from the L3 output, blurred at ¼ canvas), or an image. Backgrounds cross-fade during main-track transitions. |
| Opacity & blend | TR-015, TR-016, OV-004 | The blend is weighted by the alpha left after key, mask and opacity. 16 modes: the W3C formulas on un-premultiplied color, then source-over. Linear Burn, Add and Subtract are clamped arithmetic. The element's border, outline and shadow are assembled first, and the whole layer is blended once. |
| Masks | TR-017…TR-021 | SDF shapes (linear, band, ellipse, rounded rect, heart, star) in clip space after crop and before flip. Feather is centred on the edge and spans 0–25 % of the short side. Invert is applied after feather. Later: up to 8 masks (Add = max, Subtract = a·(1−b), Intersect = min) and Bezier paths (non-zero rule). |
| Keying | TR-022…TR-025, AI-014 | Chroma distance in BT.709 Y′CbCr with tolerance and softness, spill removal, and edge shrink (0–4 px at canvas resolution). Later a luma key. AI mattes (source-frame indexed, warped by stabilization) multiply alpha at L4. Matte preview is view-only. |
| Color | CL-001, CL-002, CL-007…CL-017, CL-021, CL-023 | Fixed 16-stage stack order. Exposure and white balance in linear light, everything else on encoded values. Curves become 1024-entry 1D textures. LUTs are 3D textures with tetrahedral (default) or trilinear interpolation. Identity stacks are skipped. Includes a GPU histogram and a compare split. |
| Overlay styling | OV-005, OV-006, OV-029 | Rounded-rect clip and border via SDF in the place pass. Outline via jump flooding. Shadow via a blurred, offset alpha copy. Static elements cache these. |
| Preview | EX-001, EX-004, EX-005, EX-006, EX-014, EX-016, EX-017 | Resolution-independent parameters, so ½ and ¼ previews look the same. Procedural offline/error slates. Snapshot via `render_still`. Render keys include shader versions. |
| Export | EX-022, EX-024, EX-032, EX-033 (later), EX-048 (later) | Output resample folded into the canvas transform. RGB→Y′CbCr BT.709 limited with deterministic dither and 4:2:0 chroma. Up to 3 frames in flight on readback. Later: HDR10/HLG and alpha output. |
| Settings & degradation | UI-028, FX-004/015/025 | Adapter choice. The GL fallback has no compute. Effects unsupported on the adapter are marked unavailable. Custom WGSL comes later. |

### Non-functional requirements (derived)

- **Determinism (FX-001).** For a given plan, sources and device, output is a pure
  function of those inputs. There is no wall clock, no RNG state and no dependence
  on delivery order. Noise is a hash of `(seed, frame index, normalized position)`.
- **Resolution independence.** Every spatial parameter reaches a shader in pixels
  that are computed from a fraction and a reference length at the current render
  size. Noise uses normalized coordinates, never raw pixel indices.
- **Rational time.** `Tick` (1/705 600 000 s) up to the uniform write. Phases of
  periodic motion are reduced in `f64` before the `f32` cast.
- **Threads.** Nothing in this crate runs on the UI thread. One compositor instance
  per thread: preview scheduler, export job, thumbnail worker.
- **Errors.** `thiserror` in `freecut-render`. Per-item problems degrade
  (bypass, identity, dissolve, slate) and are reported. They never abort the frame.
- **No `unsafe`** in v1. wgpu's safe API covers everything here. Zero-copy hardware
  interop (EX-018 "later") is the only foreseeable reason to add some, and it would
  need `// SAFETY:` comments.

### Spec gaps this design resolves or flags

These items are written here so that the owning spec can be updated. Each one is
repeated under *Risks & open questions*.

- **G1. CL-001 fusion vs stage 13.** CL-001 says stages 1–12, 14 and 15 are fused
  into one pass and stage 13 (sharpen/clarity) runs as an extra pass. But 13 sits
  *between* 12 and 14. Running fade (14) and vignette (15) before sharpening would
  break the stated order. Design: fuse 1–12 into pass A. If 13 is active, run it,
  then pass B = 14–16 (grain is pointwise given a hash, so it fuses). If 13 is
  inactive, fuse 1–12 and 14–16 into one pass.
- **G2. EX-032 chroma siting.** "Left-sited samples … using a box filter" is not
  consistent. A 2-tap box filter yields *center*-sited chroma. For MPEG-2/H.264
  left siting we filter horizontally with [1 2 1]/4 centred on even columns and
  vertically with a 2-tap average. The stream is then tagged
  `chroma_sample_location = left`.
- **G3. FX-023 8-bit luma maps.** With softness near 0 and long durations, an 8-bit
  map reveals in 256 visible steps. Store imported maps as `R16Float`, upsampled
  from 8-bit sources with a small blur. Procedural wipes are analytic.
- **G4. FX-001 L6 layer-space size.** If the layer texture is exactly the 100 %
  fit size, a 4K clip zoomed to 200 % in a 1080p project is upsampled from 1080p
  and loses the detail the user can see in other editors. Design: the render
  resolution of layer space is `fit × render_scale × k`, where `k` covers the
  zoom and is quantized in √2 steps. The *semantics* do not change, because
  parameters stay relative to the layer. Needs FX sign-off.
- **G5. Which transfer function is "linear light"?** CL-007/CL-008 apply
  exposure and white balance "in linear light" but don't name the decode. Design:
  a pure power of 2.4 (BT.1886 with zero black level) and its inverse. This matches
  how Rec.709 video is viewed. Images and text pass through as code values.
- **G6. OV-004 component modes.** W3C defines Hue/Saturation/Color/Luminosity with
  Lum = 0.3 R + 0.59 G + 0.11 B, not the Rec.709 weights. We follow W3C exactly, as
  the spec asks, and document the choice.
- **G7. One transform evaluator.** TR-007 and TR-019 need the evaluated transform
  and its inverse in `freecut-timeline` (hit testing, mask handles). TR-001's model
  notes say "freecut-render builds the clip's 2D affine". To avoid two
  implementations, the plan carries the evaluated `layer_to_canvas` matrix. It is a
  3×3 homography, because FX-018 has 3D flips with perspective. The renderer only
  scales it to pixels.

---

## How the reference editors do it

All paths are relative to each repository root. Algorithms are paraphrased in our
own words. We copy no code.

### Olive (GPL-3.0-or-later; `olive/LICENSE`, file headers)

**Architecture: a pull-based node graph.**
- `app/node/traverser.cpp` (`NodeTraverser::GenerateTable`, `ProcessInput`) walks
  upstream from the viewer for a `TimeRange`.
- Each node's `Value()` pushes a *job description* (`ShaderJob`,
  `ColorTransformJob`, `FootageJob`), not pixels.
- `NodeTraverser::ResolveJobs` later executes the jobs depth-first.
- Every job is created with the sequence's video params
  (`Texture::Job(globals.vparams(), job)` in
  `app/node/distort/transform/transformdistortnode.cpp` and
  `app/node/block/transition/transition.cpp`). So every intermediate is a
  full-canvas texture, even for a small picture-in-picture.

**Tracks and compositing.**
- `app/node/output/track/track.cpp` (`Track::Value`) passes the active block
  through.
- Layers are merged by `MergeNode` (`app/node/math/merge/merge.cpp`) using
  `app/shaders/alphaover.frag`: a premultiplied "over" (base scaled by
  1 − blend.a, plus blend).

**Transform.**
- `MatrixGenerator::GenerateMatrix` (`app/node/generator/matrix/matrix.cpp`)
  composes translate → rotate → scale → anchor offset.
- `TransformDistortNode::AdjustMatrixByResolutions` folds in sequence/texture
  resolution and Fit/Fill/Stretch.
- Interpolation is a per-node choice of nearest, bilinear or mipmapped bilinear.
  Mipmaps are made with `glGenerateMipmap` in
  `app/render/opengl/openglrenderer.cpp`.
- The transformed quad is drawn by `app/shaders/default.vert`/`default.frag`.
  We found no edge antialiasing or MSAA in that path, so rotated clips have
  stair-stepped edges.

**Transitions.**
- `TransitionBlock` has in/out offsets and a "center offset" for dual transitions.
- Progress is computed in `double` from the time in seconds (`GetTotalProgress`,
  `GetInProgress`, `GetOutProgress`).
- A curve option (linear, exponential, logarithmic) is applied to *each side
  independently* in `app/shaders/crossdissolve.frag`. With the exponential curve
  the weights are (1−p)² and p², which sum to 0.5 at the midpoint, so the image
  dips in brightness. `diptoblack.frag` splits the pair into out and in halves.

**Color.**
- `app/node/color/colormanager/colormanager.cpp` sets the OCIO scene-linear role
  as the reference space.
- Footage is converted on load in
  `app/render/renderprocessor.cpp` (`ProcessVideoFootage`).
- `app/shaders/colormanage.frag` un-associates alpha, runs the OCIO shader and
  re-associates. It guards α = 0.
- Preview renders in F16 and export in F32 (`app/config/config.cpp`,
  `OfflinePixelFormat` / `OnlinePixelFormat`).
- A comment in `ProcessVideoFootage` explains that stills are cached, because
  re-uploading and color-managing them every frame is wasteful.

**YUV upload.**
- `app/codec/ffmpeg/ffmpegdecoder.cpp` uploads Y, U and V as single-channel
  textures.
- Chroma plane size is `width()/2`, which truncates odd sizes.
- `app/shaders/yuv2rgb.frag`:
  - takes coefficients from `sws_getCoefficients`;
  - uses a luma offset of 16/256 instead of 16/255;
  - in the full-range path, reverses the luma expansion *after* the matrix, which
    leaves chroma about 2 % low;
  - samples chroma at the same normalized coordinate as luma, which implies
    centre siting.

**Texture pool.**
- `Renderer::CreateTexture` / `DestroyTexture` in `app/render/renderer.cpp`
  reuse textures keyed by (w, h, depth, format, channels).
- Textures are evicted after 5 s idle (`MAX_TEXTURE_LIFE` in
  `app/render/renderer.h`).
- A long comment in `DestroyTexture` describes a race with GL shared contexts
  when a texture is freed on another thread.

**Masks and keys.**
- Polygon masks are rasterized on the CPU with `QPainter`
  (`app/node/generator/polygon/polygon.cpp`).
- Feather is two passes of Gaussian blur (`app/node/distort/mask/mask.cpp`), so
  its cost grows with the feather radius.
- `app/shaders/shape.frag` has hard-edged rectangles and roughly 1 px AA on
  ellipses.
- `app/shaders/chromakey.frag`:
  - un-premultiplies first;
  - measures distance in CIE Lab *including L*;
  - supports garbage/core mattes.
- `app/shaders/despill.frag` limits the key channel to an average of the others.

**Keyframes.** `app/node/node.cpp` (`GetSplitValueAtTimeOnTrack`) keyframes each
component of a vector separately and solves Bezier x→y numerically.

**Resolution independence.** `app/node/filter/blur/blur.cpp` passes the texture's
`virtual_resolution()`, the full-resolution size, so radii mean the same at a
reduced preview divider.

### MLT (framework/core/plus/qt LGPL-2.1-or-later; `modules/movit` and `modules/plus/interp.h` GPL-2.0-or-later — all GPL-3-compatible)

**Model.**
- Each track's producer feeds a tractor.
- *Transitions* are planted in a field between `a_track` and `b_track` and
  composite into the a-frame (`src/framework/mlt_transition.c`).
- Progress is `(position − in) / (out − in + 1)`, with 0.5 for a one-frame
  transition (`mlt_transition_get_progress`). It starts at exactly 0 on the
  first frame and never reaches 1.

**`composite`** (`src/modules/core/transition_composite.c`):
- 8-bit YUV 4:2:2 with a separate straight alpha plane.
- Luma-map wipes use fixed-point smoothstep (`composite_line_yuv`).
- Output alpha is accumulated with a **bitwise OR** of the mix and the existing
  alpha, not with source-over.

**`luma`** (`src/modules/core/transition_luma.c`):
- 16-bit luma maps.
- The position is stretched by `(1 + softness)` before
  `smoothstep(luma, luma + softness, pos)`, so the wipe really starts at all-A and
  ends at all-B even when soft (`luma_composite_rgba32`).
- `reverse` and `invert` interact: invert swaps a/b and toggles reverse.
- The `alpha_over` and `fix_background_alpha` flags exist only because the frames
  are straight-alpha 4:2:2.
- `src/framework/mlt_luma_map.c` generates wipe maps procedurally (bands, offset,
  flip, flop, quarter, rotate).

**`qtblend`** (`src/modules/qt/transition_qtblend.cpp`):
- `QPainter` composition modes, transform and opacity. Images are wrapped as
  straight `Format_RGBA8888` (`src/modules/qt/common.cpp`, `convert_mlt_to_qimage`),
  and Qt premultiplies internally.
- A comment referring to bug #1228 explains that the source is pre-downscaled
  by swscale in geometric steps of 0.85× (`adjust_mlt_mipmap_size` in
  `src/modules/qt/common.cpp`, `MLT_QT_MIPMAP_STEP = 0.85` in
  `src/modules/qt/common.h`). This avoids QPainter aliasing and keeps requested
  sizes stable during animations.
- An opaque, full-frame, Normal layer short-circuits to the top image.
- The mode can be overridden per clip by `filter_qtblend_mode.c`.

**`affine`** (`src/modules/plus/transition_affine.c`):
- Bilinear/bicubic interpolation in `src/modules/plus/interp.h` (`interpBL_b32`).
- Color and alpha are interpolated separately on **straight** alpha, which
  produces dark or colored fringes at soft edges.

**`chroma`** (`src/modules/plus/filter_chroma.c`): a hard in/out box test on U/V
using BT.601 coefficients whatever the source, with no softness.

**`movit` module (GPU, linear light):**
- A movit `EffectChain` is rebuilt only when a *fingerprint* of the graph changes.
  Parameters are set every frame (`build_fingerprint`, `finalize_movit_chain` in
  `src/modules/movit/filter_movit_convert.cpp`).
- Chroma position is hard-coded to left/centre with a TODO (same file, in
  `get_format_from_properties`).
- `transition_movit_luma.cpp` squares the progress before mixing. Its comment
  explains that this makes the linear-light mix look perceptually even;
  without it the dissolve looks uneven.
- `filter_glsl_manager.cpp`:
  - texture pool keyed (w, h, format) with no eviction;
  - at most one frame in flight, enforced with a fence (`render_frame_texture`);
  - readback through a PBO;
  - a comment notes that the texture pool can show frames out of order with
    NVIDIA threaded optimizations.
- `mlt_movit_input.cpp` treats RGB input as Rec.709 gamma. Its comment notes
  that declaring it sRGB shifted the levels.

**`avformat`.**
- `src/modules/avformat/common.c`: `mlt_get_sws_flags` picks swscale flags per
  conversion direction, with comments about chroma bleeding. `mlt_set_luma_transfer`
  maps colorspace and range to swscale coefficients.
- Untagged colorspace defaults by height: < 576 → SMPTE 170M, < 720 → BT.470BG,
  else BT.709 (`src/framework/mlt_image.c`, `mlt_image_default_colorspace`).

**`placebo` module.** libplacebo/Vulkan with one global GPU and render mutex
(`src/modules/placebo/gpu_context.c`). It is a modern GPU path, but serialized.

### Shotcut (GPL-3.0-or-later)

- **Track compositing is an MLT transition per video track.**
  `src/models/multitrackmodel.cpp` (`getVideoBlendTransition`) looks for
  `qtblend` (CPU), `movit.overlay` (GPU) or `frei0r.cairoblend`.
- **Same-track transitions** (`MultitrackModel::addTransition`, same file):
  - make a playlist *mix* of the overlap;
  - add `luma` (or `movit.luma_mix`) for video and `mix` for audio;
  - set `alpha_over` and `fix_background_alpha` for clips with alpha.
- **Processing modes.** `src/settings.cpp` (`processingMode`, mode table) defines
  Native8Cpu, Native10Cpu, Linear10Cpu and Linear10GpuCpu.
  - **Linear8Cpu was removed** and silently mapped to Native8.
  - `src/docks/encodedock.cpp` refuses to export 8-bit linear.
  - `src/videowidget.cpp` sets `mlt_image_format=rgba64` and `mlt_color_trc=linear`
    for the linear modes.
  - Lesson: linear light needs ≥ 16-bit buffers, and mixing in linear changes the
    look users expect.
- **Preview scaling** is the consumer `scale` property (`src/videowidget.cpp`).

### Kdenlive (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL for code; `data/effects/**.xml`, `data/transitions/**.xml` and `data/lumas/{HD,PAL}/**` are CC0-1.0 per `REUSE.toml`)

- **Manifests.** Effects and transitions are XML manifests
  (`data/effects/README.md`). They declare parameter type, min/max/default, a
  display `factor`, placeholders like `%width`/`%contentHeight`, and `unique`
  (once per clip).
- **Parameter migrations.** Per-effect upgrade scripts in
  `data/effects/update/*.js` (for example `frei0r.curves.js`) migrate parameter
  meaning by version. This is the lesson for our manifest `version` and Rust
  migrations.
- **Track compositing is generated.**
  `src/timeline2/model/timelineitemmodel.cpp` (`buildTrackCompositing`) plants one
  `always_active` composite transition per video track against track 0, tagged
  `internal_added = 237`.
- **Same-track mixes** (`src/timeline2/model/trackmodel.cpp`, `createMix`):
  - clips alternate between two sub-playlists;
  - the luma transition's a/b direction is flipped according to which
    sub-playlist the incoming clip is on (`reverse = getSubPlaylistIndex() == 0`
    → `set_tracks(1, 0)`);
  - this hidden structure leaks into transition semantics.
- **Odd mode lists.** `data/transitions/qtblend.xml` exposes Qt's whole
  composition list, including bitwise modes that make no sense for video.
- **Two fades on one switch.** `data/effects/fade_from_black.xml` switches one
  "fade" between a brightness ramp and an alpha ramp.
- **CC0 assets.** `data/lumas/HD/*.pgm` are 8-bit, 1920×1080 luma maps, CC0.
  They are legally reusable as wipe patterns (G3 applies).

### libopenshot (LGPL-3.0-or-later)

- **CPU only.** `QPainter` on `QImage::Format_RGBA8888_Premultiplied` (8-bit)
  everywhere (`src/Frame.cpp`).
- **Timeline** (`src/Timeline.cpp`, `GetFrame`):
  - frame numbers come from `llround(position × fps)` in floating seconds;
  - there is one top clip per layer;
  - clips are composited in layer order via `add_layer`.
- **Clip order** (`src/Clip.cpp`, `GetFrame`): time mapping → effects (before) →
  `apply_keyframes` (transform + opacity drawn onto a transparent canvas) →
  effects (after) → `apply_background`.
- **The clip's composite mode is set in both `apply_keyframes` and
  `apply_background`.** The blend is applied once against a transparent canvas
  and again against the real backdrop.
- **Transitions are cross-track.** They are a `Mask` effect (luma wipe driven by
  brightness/contrast keyframes) on the upper layer (`src/effects/Mask.cpp`).
- **Keying.** `src/effects/ChromaKey.cpp` has many methods (HSV, HSL, CIE LCh,
  CIE distance, Y′CbCr distance with a halo ramp). A long comment explains why
  keying premultiplied 8-bit pixels loses precision. The lesson: key before
  premultiplication, in float.
- **LUTs.** `src/effects/ColorMap.cpp` un-premultiplies, normalizes to
  `DOMAIN_MIN/MAX`, does trilinear lookup with red fastest (`.cube` order), lerps
  by intensity and re-premultiplies.
- **Keyframes.** `src/KeyFrame.cpp` (`InterpolateBezierCurve`) bisects with no
  iteration limit (`do { … } while (true)`) and an error tolerance of 0.01 in
  frame units.
- **Blend list.** `src/Clip.cpp` (`composite_choices`) maps a short list of names
  to Qt composition modes.

### OpenTimelineIO (Apache-2.0)

- **Transitions take no time.** `src/opentimelineio/transition.h` gives them
  `in_offset`/`out_offset`. In `src/opentimelineio/track.cpp`,
  `Track::range_of_child_at_index` starts a transition `in_offset` before the cut,
  and `Track::handles_of_child` reports the neighbours' extra media as handles.
  `timeline-model.md` adopts this geometry.
- **`flatten_stack` is not a compositor.** `src/opentimelineio/stackAlgorithm.cpp`
  treats every visible non-gap item as opaque. It has no alpha, blend or effect
  model, so it cannot drive a renderer.
- **Effects are opaque.** `src/opentimelineio/effect.h` stores only a name, an
  enabled flag and metadata.

### Comparison

| Concern | Olive | MLT/Shotcut/Kdenlive | libopenshot | **FreeCut** |
|---|---|---|---|---|
| Precision / space | F16/F32, scene-linear (OCIO) | 8-bit YUV422 or RGBA; optional 16-bit linear (Shotcut) or movit linear | 8-bit premultiplied sRGB | **RGBA16F premultiplied, Rec.709 code values; linear only inside specific ops** |
| Alpha | associated after conversion | straight, plus separate alpha plane | premultiplied 8-bit | **premultiplied from L2 on, float** |
| Graph | lazy node graph, canvas-size jobs | per-track transitions into a-frame | per-clip QPainter passes | **plan → typed pass graph; layer-size textures; bbox placement** |
| Same-track transition | block with offsets, `double` progress | playlist mix + luma; two sub-playlists (Kdenlive) | cross-track mask | **explicit outgoing/incoming pair, exact mid-frame progress** |
| Blend modes | over only (Merge) | Qt / cairo / movit modes | Qt modes, applied twice | **W3C L1, applied once at L10** |
| Edge AA | none on transformed quads | QPainter AA hints | QPainter smooth transform | **analytic coverage in the place pass** |
| Downscale | optional mipmaps | 0.85×-step swscale pre-downscale (qtblend, bug #1228) | QPainter | **mip chain + anisotropic sampling; √2-quantized raster sizes** |
| Masks | CPU polygon + blur feather | luma/shape filters | luma mask effect | **SDF + smoothstep feather, derivative AA** |
| Texture pool | keyed, 5 s idle eviction | keyed, no eviction (movit) | n/a (CPU) | **keyed + size buckets + idle-frame and byte-budget eviction** |
| Frames in flight | job queue | 1 (movit fence) | n/a | **preview 1–2, export 3 (bounded by readback ring)** |

### Lessons carried into FreeCut

Each lesson is referenced as P*n* in the design below.

- **P1.** Interpolating straight-alpha pixels makes fringes (MLT `interp.h`).
  → Premultiply at L2. All filtering is on premultiplied values.
- **P2.** Keying after 8-bit premultiplication loses the very precision the key
  needs (libopenshot `ChromaKey.cpp` comment). → Key in float, on un-premultiplied
  color, before any alpha multiply.
- **P3.** OR-ing alpha (MLT `composite`) is not compositing. → Source-over alpha
  everywhere.
- **P4.** Easing each side of a dissolve separately breaks complementarity (Olive
  `crossdissolve.frag`). → Ease `p` once, then use weights `1 − e` and `e`.
- **P5.** Mixing and blending in linear light look different from what users
  expect. movit squares the progress, and Shotcut dropped 8-bit linear. → Code
  value working space (as the spec says). Any linear mode must be ≥ 16-bit float.
- **P6.** Floating time for progress and frame selection (Olive `double`,
  libopenshot `llround`). → Ticks to the edge, exact mid-frame progress.
- **P7.** Downscaling without prefiltering aliases (MLT bug #1228). Animated sizes
  churn caches. → Mips plus anisotropic filtering. Quantize raster and layer
  scales in √2 steps.
- **P8.** Transformed quads without edge AA (Olive). → Analytic coverage from a
  signed distance and `fwidth`.
- **P9 – P11.** Chroma siting ignored or hard-coded, odd chroma sizes truncated,
  ad-hoc range and bit-depth constants (Olive `yuv2rgb.frag`, movit TODO). → One
  affine computed and unit-tested in Rust, siting from the frame, `ceil` plane
  sizes.
- **P12.** Pools without eviction, or time-only eviction (movit, Olive). → Byte
  budget plus idle-frame eviction, and size buckets.
- **P13.** Reusing a texture another consumer still reads (movit's NVIDIA note,
  Olive's GL race). → Present slots are never pooled. Readback buffers return to
  the ring only after `map_async` completes. One queue orders everything.
- **P14.** Unbounded numeric solvers (libopenshot keyframes). → The renderer
  evaluates no curves. The timeline's solver has a fixed iteration count (TR-032).
- **P15.** Blending twice (libopenshot). → Exactly one blend per layer, at L10.
- **P16.** Hard keys and CPU-rasterized, blur-feathered masks (MLT chroma, Olive
  mask). → Soft ramps, SDF coverage, analytic feather.
- **P17.** Transition direction that depends on hidden structure (Kdenlive
  sub-playlists). → The plan names *outgoing* and *incoming* explicitly.
- **P18.** Soft wipes that never complete, and 8-bit stepping. → `(1 + s)`
  stretch (as MLT `luma` does), float maps, analytic wipes.
- **P19.** One frame in flight starves the GPU; unlimited in-flight blows memory.
  → A bounded ring per consumer.
- **P20.** A canvas-size texture for every node (Olive). → Layer-sized
  intermediates; placement drawn only over the transformed bounding box.
- **P21.** Fast paths must be provably identical (MLT qtblend's opaque
  short-circuit is). → Every fast path has a golden equality test.
- **P22.** Parameter meaning changes across versions (Kdenlive update scripts).
  → Manifest `version` plus pure Rust migrations.
- **P23.** Mis-declaring RGB inputs as sRGB vs 709 shifts levels (movit comment).
  → RGB code values pass through. Only explicit operations decode.

---

## Recommended FreeCut design

### 1. Boundaries and data flow

```
freecut-timeline (pure)        freecut-media                freecut-render (this doc)
  SequenceSnapshot               DecoderPool / FrameSource     Compositor
  frame_plan(t) -> FramePlan --> DecodedFrame (CPU planes) --> upload/normalize (L1-L2)
  evaluated values, Ticks        ColorInfo, chroma location    compile -> PassGraph -> encode -> submit
                                                                outputs: PresentSlot | ExportFrame | Still
```

- `freecut-timeline` builds the plan. It resolves visibility, coverage, time maps,
  keyframes, easing evaluation, animation offsets and the transform matrix. It has
  no GPU types (FX-001 model notes, timeline-model.md §13).
- `freecut-media` supplies decoded frames as CPU planes with their color
  description. Render depends on media for the `DecodedFrame` type and the
  `DecoderPool` (behind our `FrameSource` trait, so tests can feed synthetic
  frames).
- `freecut-render` owns the following. Nothing else does:
  - all GPU state: pipelines, pools, caches;
  - all WGSL;
  - effect and transition registries and manifests;
  - color conversion math;
  - the CPU reference implementations used by tests and by look baking (CL-005).
- The preview scheduler (playback-engine.md) and the export job each own one
  `Compositor` on their own thread. The scheduler calls `render` with its plan,
  target and `PresentSlot`. The export job calls it with an `ExportFrame`.

### 2. Working space, precision and alpha

**Working texture.** `Rgba16Float`, premultiplied, Rec.709 primaries, values are
display-referred R′G′B′ code values in nominal [0, 1]. It is renderable, blendable
and filterable in core WebGPU, so no feature is needed. It has 11 significant bits,
which is better than the 1/1023 step of 10-bit sources everywhere in [0, 1].

**Never use `*Srgb` formats in the pipeline.** The values are already encoded, and
an sRGB view would decode them again on every sample. The preview's present slot
is `Rgba8Unorm` for the same reason (egui-wgpu's `register_native_texture` also
requires it). playback-engine.md handles an sRGB swapchain in its blit.

**Where each operation runs:**

| Operation | Domain | Why |
|---|---|---|
| YUV → R′G′B′, range expansion | code values | definition (BT.601/709/2020) |
| HDR → SDR tone map, BT.2020 → 709 gamut | linear light (in the L2 pass) | physics (CL-002) |
| Exposure, white balance (CL-007, CL-008) | linear via γ = 2.4 decode/encode (G5) | spec says linear light |
| Tone, curves, wheels, HSL, LUT, looks, fade, vignette | encoded values | spec; LUTs expect 709 gamma (CL-017) |
| Chroma key distance | encoded BT.709 Y′CbCr | TR-022 |
| Resampling, mips, blur, sharpen | encoded, premultiplied | matches consumer editors; linear-light resampling is a later option |
| Dissolves, wipes, transitions | encoded, premultiplied | FX-003; avoids P5 |
| Blend modes, opacity, source-over | encoded, premultiplied | W3C formulas are defined on encoded values |
| Output Y′CbCr | code values | EX-032 |

**Alpha rules (P1, P2).**
1. Straight-alpha sources (PM-040 `Straight`, most PNG and ProRes 4444) are
   premultiplied **once**, in the L2 pass.
2. Every non-linear per-pixel operation un-premultiplies first, then
   re-premultiplies. This includes the color stack, keys, blend functions and
   transfer functions. The un-premultiply is guarded:
   `c.a ≤ 1e-6 ⇒ rgb = 0`. This prevents NaNs and keeps "transparent stays
   transparent" (CL-001).
3. Values may leave [0, 1] between stages (CL-001). They are clamped at L10:
   alpha to [0, 1], then RGB to [0, α].
4. A NaN guard runs at the end of the color pass and the effect passes. A NaN in
   a blur would otherwise spread across the frame.

### 3. Color management at the edges

**Input (CL-002, PM-035/040).** `freecut-media` reports
`ColorInfo { matrix, range, transfer, primaries, chroma_location, alpha }`.
Interpretation overrides are already applied. If tags are missing:
- matrix: BT.601 when height < 720, else BT.709;
- range: limited for YUV, full for JPEG/PNG;
- chroma location: left for 4:2:0, which is the FFmpeg/H.264 default.

| Source | L2 conversion |
|---|---|
| Rec.709 / sRGB / untagged SDR | affine YUV→R′G′B′ only. RGB code values pass through (P23). |
| BT.601 (525/625) | matrix 601. Primaries are treated as 709 (difference ≪ 1 ΔE₀₀ for typical content). A switch exists for later. |
| BT.2020 SDR, Display-P3 images | MVP: P3 is treated as 709 (CL-002). v1: decode γ → 3×3 gamut matrix → soft clip → encode. |
| PQ / HLG (HDR) | EOTF to linear nits (PQ per SMPTE ST 2084; HLG inverse OETF + OOTF, system γ 1.2 at 1000 nits). Divide by reference white 203 cd/m² (BT.2408). Tone map with BT.2390 EETF on max(R,G,B), using source peak from MaxCLL/mastering metadata (default 1000 nits). Then 2020→709 gamut matrix and soft clip, then γ 2.4 encode. One L2 shader variant (CL-002 asks for a "simple filmic curve"; BT.2390 is a published, parameter-free choice). |

**Output (EX-032).** One pass reads the accumulator and writes Y′ and CbCr
targets:
- BT.709 matrix, limited range;
- chroma filtered per G2;
- deterministic ordered dither (8×8 Bayer, or a 64×64 procedural blue-noise tile).
  The dither is applied to code values before quantization and seeded by the
  output frame index.

GIF and image outputs bypass Y′CbCr (EX-043/047). Tags are written by
`freecut-media`.

**Preview display.** The present blit shows working values directly. There is no
ICC transform (CL-002 out of scope), and sRGB swapchains are handled in the blit
(playback-engine.md).

**HDR later (EX-033).** A project-level `WorkingSpace` enum gains a second
variant, `LinearRec2020 { reference_white_nits }`. Every shader reads its
transfer and luma weights through three helpers in `common.wgsl` (`ws_to_linear`,
`ws_from_linear`, `ws_luma`), driven by an override constant. That keeps the
switch contained. Changing it changes how dissolves and blends look (P5), and the
UI must say so.

### 4. The `FramePlan` contract (timeline → render)

The type lives in `freecut-timeline`. This is what the compositor needs from it.
Names follow timeline-model.md (`Tick`, `ClipId`, `TransitionKindId`,
`ParamSet`).

```rust
// freecut-timeline::plan — pure data, no GPU types (FX-001 model notes).
pub struct FramePlan {
    pub seq_version: u64,
    pub time: Tick,                    // sequence time of this output frame
    pub frame: Tick,                   // sequence frame duration (progress, history)
    pub frame_index: i64,              // seeds for canvas-space effects (CL-011)
    pub canvas: CanvasSpec,            // project px, PAR, background colour (code values)
    pub background: BackgroundPlan,    // FX-001 step 1, may cross-fade (TR-011 edge case)
    pub nodes: Vec<PlanNode>,          // bottom -> top; hidden tracks already removed
}

pub enum PlanNode {
    Layer(LayerPlan),
    Transition(TransitionPlan),
    EffectLayer(EffectLayerPlan),      // FX-011 "everything below"; "main track only" became clip effects
    Adjustment(AdjustmentPlan),        // CL-006
}

pub struct LayerPlan {
    pub clip: ClipId,
    pub source: LayerSource,
    pub crop: CropRect,                     // fractions of display size + straighten (TR-009/010)
    pub stabilize: Option<DMat3>,           // TR-026 correction in source-normalized space
    pub key: Option<KeyValues>,             // TR-022..025 + AI-014 matte ref, evaluated
    pub color: Option<ColorStackValues>,    // CL-001; None == identity (skipped)
    pub effects: Vec<EffectPlan>,           // enabled only, list order (FX-013)
    pub mask: Option<MaskValues>,           // TR-017/018, clip space
    pub style: Option<LayerStyle>,          // OV-005/006/029
    pub layer_to_canvas: DMat3,             // G7: TR transform + FX-018/019 offsets, canvas-normalized
    pub display_size: (u32, u32),           // cropped, rotated, PAR-corrected source size (TR-002)
    pub opacity: f64,                       // TR-015 x animation factor, already clamped to 0..=1
    pub blend: BlendMode,                   // OV-004
    pub time: LocalTime,                    // clip-local Tick, source frame index (seeds, history)
}

pub enum LayerSource {
    Video { asset: AssetId, samples: SmallVec<[(Tick, f32); 2]> }, // TR-036/041: weights sum to 1
    Still { asset: AssetId },
    Raster(RasterRequest),                  // text / shape / sticker; rasterized by render (TX, OV)
    Slate { kind: SlateKind, file_name: String }, // EX-005; never a full path
}

pub struct TransitionPlan {
    pub kind: TransitionKindId,             // unknown kinds kept verbatim (FX-002)
    pub params: ParamSet,                   // static; transitions are not keyframed (FX-005)
    pub easing: Easing,                     // TR-031 type; evaluated by render via timeline's pure fn
    pub elapsed: Tick, pub duration: Tick,  // timeline-model.md §6: two integers, division in render
    pub outgoing: LayerPlan,
    pub incoming: LayerPlan,                // its `blend` is used at L10 (FX-001 step 5)
}
```

**Plan invariants.** Checked by `debug_assert` in the compiler and by the
timeline's tests:
- `nodes` are already in render order.
- `opacity` and every `ParamValue` are finite and clamped to their specs.
- `layer_to_canvas` is invertible, or the layer is omitted.
- Sample weights sum to 1.
- A plan never contains a hidden track's items.

### 5. Compiling a plan into a pass graph

Compiling is CPU-only and costs microseconds. It runs every frame, with no
graph-level caching to invalidate after edits. Pipelines are cached by key
(§11), which takes the place of movit's chain fingerprint.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {                // FX-001 order; the derive(Ord) *is* the order
    Source,                     // L1
    Normalize,                  // L2
    CropResample,               // L3
    Key,                        // L4
    Color,                      // L5
    Effect,                     // L6
    Place,                      // L7-L10 fused
}

/// One layer's chain. The only way to append passes for a layer.
pub(crate) struct LayerChain<'g> { g: &'g mut GraphBuilder, last: Stage, head: ResId }

impl LayerChain<'_> {
    pub(crate) fn push(&mut self, stage: Stage, pass: PassDesc) -> ResId {
        // FX-001: the order is enforced here and nowhere else.
        debug_assert!(stage >= self.last, "stage {stage:?} after {:?}", self.last);
        self.last = stage;
        self.head = self.g.add_pass(stage, pass, self.head);
        self.head
    }
}

pub(crate) fn compile(plan: &FramePlan, cx: &CompileCx) -> Result<PassGraph, RenderError> {
    let mut g = GraphBuilder::new(cx.target_px);
    let mut acc = g.background(&plan.background, cx);                 // step 1
    for node in &plan.nodes {                                         // step 2, bottom -> top
        acc = match node {
            PlanNode::Layer(l) => {
                let placed = layer_chain(&mut g, l, cx)?;             // L1-L9 (Place may defer)
                g.blend(acc, placed, l.blend)                         // L10
            }
            PlanNode::Transition(t) => {
                let a = layer_to_transparent_canvas(&mut g, &t.outgoing, cx)?;
                let b = layer_to_transparent_canvas(&mut g, &t.incoming, cx)?;
                let mixed = g.transition(t, a, b, cx)?;               // FX-001 step 5
                g.blend(acc, mixed, t.incoming.blend)
            }
            PlanNode::EffectLayer(e) => g.effect_layer(acc, e, cx)?,  // step 6
            PlanNode::Adjustment(a) => g.adjustment(acc, a, cx)?,
        };
    }
    Ok(g.finish(acc))                                                 // step 7: output pass added by target
}
```

**Fusion rules and fast paths.** Each must equal the slow path in a golden test
(P21).

| Case | Passes |
|---|---|
| Plain video clip, Normal blend, no key, color, effects or style | Normalize (cached per source frame) → **Place directly onto the accumulator**. That is 2 passes, and 1 when the source frame is already in `SourceCache`. |
| + crop or straighten only | Crop is a UV rectangle and straighten an extra rotation in the place transform. No extra pass, because placement resamples anyway. |
| + key and/or color | CropResample → one fused *pointwise* pass: key (unpremultiply once) + color stack A (G1). Then the optional sharpen/clarity pass and color stack B. Then Place. |
| + effects | Each effect adds its declared passes in layer space before Place. |
| Opacity 0, or bbox outside the target | The layer is skipped. FX-001 allows this if the output is identical, and a test checks it. |
| Advanced blend mode | Place writes into the accumulator copy path (§7). |

**Resources.** Each `ResId` has a descriptor (size, format, mips) and a last-use
index. Executing the graph acquires textures from the pool on first write and
releases them after their last read. The accumulator ping-pongs only where a pass
must read it (effect layers, adjustment layers, advanced blends).

### 6. The layer pipeline in detail

#### L1 / L2 — upload and normalize

`DecodedFrame` from `freecut-media` uses one of a few layouts. Anything else is
converted in media with swscale to `Yuv444P16` or `Rgba16`, using the accurate
rounding flags whose direction-dependent choice MLT documents in `common.c`.

| Layout | Textures (usage `COPY_DST | TEXTURE_BINDING`) | Notes |
|---|---|---|---|
| `Yuv420P/422P/444P` 8-bit | 3 × `R8Unorm` | chroma size `ceil(w / sx) × ceil(h / sy)` (P10) |
| `Nv12` (hardware decode output) | `R8Unorm` + `Rg8Unorm` | wgpu's `TextureFormat::NV12` is DX12/Vulkan only, so it's not used |
| 10/12-bit planar LSB (`yuv420p10le`, …) | `R16Unorm` × 3 if `TEXTURE_FORMAT_16BIT_NORM` (filterable), else `R16Uint` with `textureLoad` and manual bilinear | packing `Unorm16Lsb` / `Uint16Lsb` |
| `P010` (MSB-aligned) | `R16Unorm` + `Rg16Unorm` | packing `Unorm16Msb`: code = sample × 65535 / 64 |
| `Yuva*` (VP8/VP9 alpha, ProRes 4444) | plus an alpha plane | alpha mode from PM-040 |
| `Rgba8`, `Bgra8`, `Rgba16` | one RGBA texture | straight alpha is premultiplied in L2 |

`Queue::write_texture` accepts FFmpeg's `linesize` directly as `bytes_per_row`.
Unlike buffer copies, it has no 256-byte alignment rule.

The affine is computed in Rust and unit-tested (P9–P11):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvMatrix { Bt601, Bt709, Bt2020Ncl, Smpte240m, Fcc }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YuvRange { Limited, Full }
/// How a stored texture sample maps back to the integer code value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Packing { Unorm8, Unorm16Lsb, Unorm16Msb, Uint16Lsb }

impl YuvMatrix {
    fn kr_kb(self) -> (f64, f64) {
        match self {
            Self::Bt601 => (0.299, 0.114),
            Self::Bt709 => (0.2126, 0.0722),
            Self::Bt2020Ncl => (0.2627, 0.0593),
            Self::Smpte240m => (0.212, 0.087),
            Self::Fcc => (0.30, 0.11),
        }
    }
}

/// Rows of R'G'B' = M * (sY, sCb, sCr, 1) for raw samples s. `bits` in 8..=16.
pub fn yuv_to_rgb_affine(m: YuvMatrix, r: YuvRange, bits: u32, p: Packing) -> [[f64; 4]; 3] {
    let (kr, kb) = m.kr_kb();
    let kg = 1.0 - kr - kb;
    let to_code = match p {
        Packing::Unorm8 => 255.0,
        Packing::Unorm16Lsb => 65535.0,
        Packing::Unorm16Msb => 65535.0 / f64::from(1u32 << (16 - bits)),
        Packing::Uint16Lsb => 1.0,
    };
    let k = f64::from(1u32 << (bits - 8));
    let max = f64::from((1u32 << bits) - 1);
    let (y0, ys, c0, cs) = match r {
        YuvRange::Limited => (16.0 * k, 219.0 * k, 128.0 * k, 224.0 * k),
        YuvRange::Full => (0.0, max, f64::from(1u32 << (bits - 1)), max), // BT.2100 full range
    };
    let (ay, by, ac, bc) = (to_code / ys, -y0 / ys, to_code / cs, -c0 / cs);
    let (rv, bu) = (2.0 * (1.0 - kr), 2.0 * (1.0 - kb)); // R' = Y' + rv*Cr, B' = Y' + bu*Cb
    let (gu, gv) = (-kb * bu / kg, -kr * rv / kg);       // G' = Y' + gu*Cb + gv*Cr
    [
        [ay, 0.0, rv * ac, by + rv * bc],
        [ay, gu * ac, gv * ac, by + (gu + gv) * bc],
        [ay, bu * ac, 0.0, by + bu * bc],
    ]
}
```

**Chroma siting.** Let `s` be the chroma sample's position across a subsampled
axis: 0 = co-sited with the first luma sample ("left"/"top"), 0.5 = centred. The
chroma texel coordinate for a luma pixel centre `u` is `u / 2 + (0.25 − 0.5·s)`.
Non-subsampled axes add 0. The shader samples chroma with a linear sampler at that
coordinate, divided by the chroma texture size. Odd sizes therefore come out right
without special cases.

**Interlaced sources.** They are de-interlaced in `freecut-media` before upload
(PM-040 field order). The renderer only ever sees progressive frames.

**`SourceCache`.** The normalized output is a source-resolution `Rgba16Float`,
premultiplied, with mips created on demand. Its key is
`(AssetId, fingerprint, interpretation hash, source frame index, decode scale)`.
- Stills hit for their whole duration (the Olive lesson).
- Slow motion, freeze frames and scrubbing back also hit.
- Frame blending (TR-041) normalizes each sample and averages them by weight in
  one extra pass. At most 16 samples (TR-041 cap).

#### L3 — stabilize, crop, straighten, and layer-space resolution

- **Layer space** is the cropped source mapped to its 100 % fit size (TR-002).
  Spatial parameters are fractions of the layer height (FX-001).
- **The pixel size of layer space (G4):**

  `layer_px = round_even(fit_px × render_scale × k)`, where
  `k = quantize_up_sqrt2(clamp(σ_max, 1, source_px / fit_px))`.

  - `σ_max` is the largest singular value of the linear part of `layer_to_canvas`.
    For a homography, it is the maximum over the four corners' Jacobians.
  - Quantizing up to powers of √2 keeps the size stable while a zoom animates.
    This is the same idea as MLT's 0.85× mipmap step (P7), but coarser, so
    fewer textures are reallocated. Because it rounds up, it never undersamples.
  - Raster layers (text, vector stickers) use the same rule, with their own
    natural size instead of `source_px`.
- **L3 resample pass.** It runs only when a layer-space texture is needed (key,
  color, effects or style present). It samples the `SourceCache` texture through
  `crop_uv × straighten × stabilize`, using trilinear + anisotropic filtering when
  minifying.
- **Matte warp.** AI mattes (AI-014) and stabilization use the same mapping, so
  the matte stays registered.

#### L4 — keying

The fused pointwise pass does the following on un-premultiplied input:
1. **Chroma key.**
   - `d = ‖CbCr(px) − CbCr(key)‖` in BT.709 Y′CbCr of the working values (TR-022).
   - `k = smoothstep(tol, tol + soft, d)`, with `soft ≥ 1/1024` so it never
     divides by zero.
   - Tolerance and softness are mapped from the 0–100 sliders by a table in
     `KeyValues`, so the GPU and CPU reference agree.
2. **Spill (TR-023).** Remove the key hue's chroma component from the remaining
   pixels: project CbCr onto the key direction and subtract the positive part,
   scaled by `spill / 100`. Luma is preserved.
3. **AI matte.** Multiply by the AI matte sample (AI-014). Its feather, choke and
   despill run first, in the same pass where they are pointwise. Choke uses the
   erosion pass below.
4. **Luma key (later, TR-025)** multiplies in.

**Edge shrink** (TR-023, 0–4 px at canvas resolution) is a separate pass: a
separable min filter on alpha. Its radius is converted to layer pixels through the
canvas scale, so preview and export match.

**Matte preview (TR-024)** is a render flag. The selected layer's place pass
outputs `vec4(α, α, α, 1)` instead of color.

#### L5 — color stack

Compiled once per change into `CompiledColorStack` (CL-001 model notes):
- a uniform block of all scalars, as evaluated `f32`;
- one `1024 × 1` `Rgba16Float` curves texture: master in A, R/G/B in RGB
  (CL-014);
- the user LUT and filter look as `Rgba16Float` 3D textures, shared by content
  hash (CL-004, CL-016).

Pass layout follows G1. Implementation notes:

- **LUT sampling (CL-017).**
  - Clamp the input to `DOMAIN_MIN/MAX`, then normalize.
  - Trilinear uses the hardware sampler at `(c·(N − 1) + 0.5) / N`. Forgetting
    the half-texel remap shrinks the LUT's range and flattens contrast.
  - Tetrahedral uses `textureLoad` of the 4 lattice points of the containing
    tetrahedron (selected by ordering the fractional parts).
  - `.cube` data is red-fastest, so it uploads straight into x = R, y = G, z = B
    (as libopenshot `ColorMap.cpp` indexes it). Rows are padded RGB → RGBA.
  - 65³ × 8 B = 2.2 MB, so any number of shared LUTs is cheap.
- **Linear-light stages** use `ws_to_linear` / `ws_from_linear` (γ 2.4, G5) on
  un-premultiplied values. Negative values are mirrored (`sign(x)·|x|^γ`) so
  out-of-range intermediates stay finite.
- **Spatial stages.** Sharpen and clarity (stage 13) and the TR-012 background
  blur share one separable Gaussian. When the radius exceeds ~24 px it switches to
  a downsample pyramid. Radii come from fractions of the frame height × the
  reference length (CL-009).
- **Grain (stage 16)** uses a PCG-style integer hash of
  `(seed, frame_index, floor(uv / cell))`. `cell` is a fraction of the reference
  height, never a pixel index, so grain looks the same at any preview scale and
  across the √2 layer-resolution steps.
  - Clip grain seeds by *source* frame index.
  - Adjustment layers seed by *timeline* frame index (CL-011).
- **Before/after compare (CL-023)** renders the frame twice: the "before" plan
  has color stacks removed. The display blit picks per pixel by the split x.

#### L6 — clip effects

```rust
pub struct EffectManifest {
    pub id: &'static str,                 // "freecut.blur.gaussian" (namespaced, FX-010)
    pub version: u32,                     // bump when a parameter's meaning changes (P22)
    pub params: &'static [ParamSpec],
    pub history_frames: u8,               // 0..=8 (FX-010 temporal effects)
    pub clip_only: bool,
    pub needs_compute: bool,              // hidden on adapters without compute (UI-028)
}

pub enum ParamKind {
    Scalar { min: f64, max: f64, default: f64 },
    Length { default: f64, of: RefLength },   // fraction -> px at render time (resolution independence)
    Angle { default_deg: f64 },
    Color { default: [u8; 4] },               // code values; enters the working space as-is
    Point { default: [f64; 2] },              // normalized; -> px via the layer or canvas size
    Toggle { default: bool },
    Choice { options: &'static [&'static str], default: u32 },
    Seed,
}

pub trait Effect: Send + Sync + 'static {
    fn manifest(&self) -> &'static EffectManifest;
    /// Upgrades params stored by an older `version` (pure; run at project load).
    /// Revised in review: moved to `freecut_timeline::effects::EffectCatalog::migrate` (risk 11).
    fn migrate(&self, from: u32, params: ParamSet) -> ParamSet { let _ = from; params }
    /// Records this effect's passes. `input` is premultiplied working space.
    fn encode(&self, b: &mut PassBuilder<'_>, input: ResId, fx: &EffectInput<'_>)
        -> Result<ResId, RenderError>;
}

pub struct EffectInput<'a> {
    pub values: &'a ParamSet,      // evaluated by the timeline at local time (FX-014)
    pub ref_height_px: f32,        // layer or canvas height at this render size
    pub time: LocalTimeF,          // seconds (f64 reduced to f32), frame index, phase helpers
    pub seed: u64,
    pub history: &'a [ResId],      // previous L6 inputs of this clip, newest first
}
```

- **Uniform packing is generic.** The packer walks `params` and converts each
  `ParamKind`: `Length` × `ref_height_px`, `Angle` → radians, `Point` →
  layer pixels. An effect therefore cannot receive raw resolution-dependent
  values by accident.
- **Registry status.** `EffectRegistry` validates every manifest at startup.
  Every pipeline is built on a worker thread inside an error scope. wgpu 30's
  `ErrorScopeGuard` is `!Send`, so push and pop happen on that worker. The result
  is published as `RegistryStatus`. Kinds whose shader failed are hidden from the
  library, and existing uses are bypassed with a warning (FX-015).
- **Temporal effects (FX-010).** The plan lists up to 8 history times for the
  clip, never across a cut. `HistoryCache` keeps post-L5 layer textures keyed by
  `(clip, source frame index, layer_px)`. A miss re-renders that frame's L1–L5,
  so a seek reproduces exactly what continuous playback would have produced.
- **Large radii (FX-015).** Blur-like effects use a dual-filter downsample /
  upsample pyramid once the radius exceeds the separable-kernel budget. Cost
  stays bounded at 8K.
- **Custom WGSL (FX-025, later).**
  - Validated with naga against a fixed binding interface.
  - Executed like built-ins.
  - wgpu exposes no per-pass GPU watchdog, so a pathological shader risks a TDR
    and device loss (see Risks).

#### L7–L10 — the place pass

A single fragment pass is drawn over the clipped bounding box of the transformed
layer. It works by inverse mapping: each target pixel computes where it lands in
layer space. One pass therefore gives:
- perspective-correct 3D presets;
- correct mip LOD from implicit derivatives;
- mask, rounded corners and edge AA in the same coordinate frame.

```wgsl
struct PlaceU {
  canvas_to_layer: mat3x3<f32>, // target px (pixel centres) -> layer px, origin at layer centre, pre-flip
  half_size: vec2<f32>,         // layer half extent, layer px
  corner_radius: f32,           // OV-005, layer px
  opacity: f32,                 // L9: TR-015 x animation factor
  content: vec4<f32>,           // uv origin (xy) and scale (zw) of the content rect in a pooled texture
  mask: MaskU,                  // L7 (see mask.wgsl)
};
@group(0) @binding(0) var<uniform> u: PlaceU;
@group(0) @binding(1) var layer_tex: texture_2d<f32>;
@group(0) @binding(2) var layer_smp: sampler; // linear + mip linear + anisotropy, clamp-to-edge

@fragment
fn fs_place(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
  let h = u.canvas_to_layer * vec3<f32>(frag.xy, 1.0);
  let p = h.xy / max(h.z, 1e-6);                                   // perspective divide
  let uv = u.content.xy + (p / (2.0 * u.half_size) + 0.5) * u.content.zw;
  let texel = textureSample(layer_tex, layer_smp, uv);             // must stay in uniform control flow
  let d = sd_round_rect(p, u.half_size, u.corner_radius);          // < 0 inside, layer px
  let aa = max(fwidth(d), 1e-4);                                   // layer px per target px
  var cov = clamp(0.5 - d / aa, 0.0, 1.0);                         // analytic edge AA (P8)
  cov *= mask_coverage(p, u.mask, aa);                             // L7, clip space
  cov *= select(0.0, 1.0, h.z > 0.0);                              // 3D presets: drop points behind the eye
  let a = clamp(texel.a, 0.0, 1.0);
  let src = vec4<f32>(clamp(texel.rgb, vec3<f32>(0.0), vec3<f32>(a)), a); // CL-001 clamp at composite
  return src * (cov * u.opacity);                                  // Normal: blend One, OneMinusSrcAlpha
}
```

**Matrix.** `canvas_to_layer` is the inverse of the plan's `layer_to_canvas`,
after it is scaled from canvas-normalized units to target pixels and from layer
units to layer pixels. The plan's matrix already encodes TR's order: flip, scale
and rotate about the anchor, translate. The renderer adds no transform logic of
its own (G7).

**Flip.** `p` is pre-flip clip space. Masks are evaluated in unflipped clip space
and therefore flip with the image (TR-003, TR-009).

**Uniformity pitfall.** WGSL only allows `textureSample` and `fwidth` in uniform
control flow. Both must be computed before any branch or `discard`. naga rejects
violations when the shader is validated in CI.

**Sampler.** `anisotropy_clamp = 16`, which requires all three filters to be
`Linear`, and clamp-to-edge. Edge texels replicate under clamp, but coverage
zeroes them outside the rectangle, so there is no smear. Mips are generated for
the layer texture only when `σ_min < 0.75`, the minification threshold computed
on the CPU.

**Mask SDFs (TR-017/018).**
- Shapes are evaluated in **layer pixels** (isotropic), never in normalized UV.
  This keeps circles round and feather uniform on non-square clips.
- Feather `f` (0–25 % of the short side, in px) is centred on the edge:
  `c = 1 − smoothstep(−f/2, f/2, d)`.
- With `f = 0`, the derivative AA band from the place pass gives a clean 1 px edge.
- Invert after feather: `c = 1 − c`.
- **Shapes:**
  - linear: a half-plane through the centre;
  - band: `|n·p| − w/2`;
  - ellipse: the first-order estimate `d ≈ f / |∇f|` with `f = |p / r| − 1`. It
    is exact on the edge and good across the feather band. If very wide feathers
    on elongated ellipses look uneven, switch to the exact closed-form solution;
  - rounded rectangle;
  - star: polygon distance (5 points);
  - heart: two circles and a rotated square.
  The ellipse, star and heart SDFs are derived in-house from geometry; we copy no
  code.
- **Later (TR-020).** Multiple masks combine in list order: max, a·(1−b), min.
- **Later (TR-021).** Bezier paths: a coverage pass (non-zero winding) into an
  `R8Unorm` mask texture, which is then feathered with the distance-blur pyramid.

**Styling (OV-005/006/029).**
- Border and rounded corners are SDF terms in the same pass. Border width stays
  constant on the canvas by dividing by σ.
- Outline and shadow need the transformed alpha, so for styled layers the order
  becomes:
  1. place into a transparent bbox-sized texture;
  2. jump-flood the distance field (log₂ N fragment passes, so it works without
     compute);
  3. compose shadow → outline → content;
  4. blend once at L10 (OV-004: "the whole clip group is blended once").
- Static elements cache the styled result keyed by `(content hash, size, rotation)`.

### 7. Blend mode math

**Normal** is fixed-function premultiplied source-over (`One`,
`OneMinusSrcAlpha` for both color and alpha) straight onto the accumulator. That
is the common case, with no extra copies.

**The other modes** need the backdrop:
1. `copy_texture_to_texture` the accumulator's bbox into a pooled `backdrop`
   texture.
2. Run the place pass with blending disabled. It writes
   `composite_w3c(src, backdrop, mode)`.

```wgsl
fn unpremul(c: vec4<f32>) -> vec3<f32> { return select(vec3<f32>(0.0), c.rgb / c.a, c.a > 1e-6); }

// W3C Compositing and Blending Level 1, general formula, with premultiplied in/out.
fn composite_w3c(s: vec4<f32>, b: vec4<f32>, mode: u32) -> vec4<f32> {
  let cs = clamp(unpremul(s), vec3<f32>(0.0), vec3<f32>(1.0));
  let cb = clamp(unpremul(b), vec3<f32>(0.0), vec3<f32>(1.0));
  let mixed = (1.0 - b.a) * cs + b.a * blend_fn(cb, cs, mode); // backdrop alpha weights the blend
  let rgb = s.a * mixed + (1.0 - s.a) * b.rgb;                 // b.rgb is already premultiplied
  return vec4<f32>(rgb, s.a + b.a * (1.0 - s.a));
}
```

`blend_fn(Cb, Cs)` per channel. These are the W3C definitions, with Linear Burn,
Add and Subtract as OV-004's clamped arithmetic:

| Mode | B(Cb, Cs) |
|---|---|
| Darken / Lighten | min / max |
| Multiply | Cb·Cs |
| Screen | Cb + Cs − Cb·Cs |
| Color Burn | Cb = 1 → 1; Cs = 0 → 0; else 1 − min(1, (1 − Cb)/Cs) |
| Color Dodge | Cb = 0 → 0; Cs = 1 → 1; else min(1, Cb/(1 − Cs)) |
| Linear Burn | max(0, Cb + Cs − 1) |
| Add | min(1, Cb + Cs) |
| Overlay | HardLight(Cs, Cb) (operands swapped) |
| Hard Light | Cs ≤ ½ → Multiply(Cb, 2Cs); else Screen(Cb, 2Cs − 1) |
| Soft Light | Cs ≤ ½ → Cb − (1 − 2Cs)·Cb·(1 − Cb); else Cb + (2Cs − 1)·(D(Cb) − Cb), with D(x) = x ≤ ¼ ? ((16x − 12)x + 4)x : √x |
| Difference / Exclusion | \|Cb − Cs\| / Cb + Cs − 2Cb·Cs |
| Subtract | max(0, Cb − Cs) |
| Hue / Saturation / Color / Luminosity | W3C non-separable SetLum/SetSat/ClipColor with Lum = 0.3R + 0.59G + 0.11B (G6) |

**Rules:**
- The alpha that weights everything is the one after key, mask and opacity
  (TR-016), so keyed-out pixels contribute nothing in any mode.
- A transition result is blended once with the **incoming** clip's mode
  (FX-001 step 5, OV-004).
- Adjustment and effect layers have no blend mode (OV-004 edge case).
- The order of the mode table is the `BlendMode` enum order in the timeline crate.
  The renderer maps it with an exhaustive `match`, never with `as u32` on the
  enum. An unknown mode string loads as Normal (OV-004) in the timeline crate, not
  here.

### 8. Transitions

**Progress.** Computed once per frame. The two integers come from timeline-model.md §6:

```rust
/// FX-002: p = (elapsed + frame/2) / duration, exact in ticks, converted once.
pub fn transition_progress(elapsed: Tick, frame: Tick, duration: Tick) -> f64 {
    let num = 2 * i128::from(elapsed.as_i64()) + i128::from(frame.as_i64());
    let den = 2 * i128::from(duration.as_i64()).max(1);
    (num as f64 / den as f64).clamp(0.0, 1.0)
}
```

- A 1-frame transition gives exactly 0.5. MLT special-cases this; for us it falls
  out of the formula.
- The easing value `e = easing.eval(p)` (TR-031, timeline's pure function) is
  clamped to [0, 1] for dissolve-type kinds and passed through for motion kinds
  (FX-005).
- "Reverse direction" mirrors the *pattern*: luma `l → 1 − l`, direction
  `θ → θ + 180°`. It never swaps outgoing and incoming. This avoids MLT's
  reverse/invert entanglement and Kdenlive's sub-playlist flips (P17).

**Inputs.** Outgoing and incoming are each rendered L1–L9 into **transparent**
target-sized textures, and those are bbox-limited where the kind allows. The
transition shader writes a premultiplied result, which goes through L10.
- Premultiplied arithmetic makes alpha sources dissolve correctly. MLT needed the
  `alpha_over`/`fix_background_alpha` flags only because it worked on straight
  4:2:2 frames.
- Backgrounds of two main-track clips cross-fade with the same `e` (TR-011). The
  planner emits `BackgroundPlan::CrossFade`.

**Built-in formulas.** All on premultiplied RGBA, and all our own:

| Kind | Result |
|---|---|
| Dissolve (FX-003) | `A·(1 − e) + B·e` — complementary weights (P4) |
| Dip to color | `e < ½`: `mix(A, C, 2e)`; else `mix(C, B, 2e − 1)` (C premultiplied, opaque) |
| Additive dissolve | `A·min(1, 2(1 − e)) + B·min(1, 2e)`, then RGB clamped to [0, α] and α to [0, 1]. Exactly A at e = 0 and B at e = 1; at e = ½ it is A + B (the brightening). |
| Luma / pattern wipe | `w = smoothstep(l, l + s, e·(1 + s))`, then `A·(1 − w) + B·w`. The `(1 + s)` stretch is the MLT `luma` behaviour (P18). `s ≥ 1/4096`. Dark switches first (FX-023). |
| Straight wipe at angle θ | analytic `l = (dir·(px − c)) / extent + ½`. `extent` is the max over the canvas corners, so the wipe starts on the first frame and ends on the last |
| Clock / radial | `l = fract(atan2(dy, dx)/2π + offset)` in aspect-correct pixels |
| Iris (circle, diamond, rect) | `l = shape_distance(px − c) / max_corner_distance` |
| Blinds / checkerboard | `l` from `fract(px·count/size)` with per-cell offsets |
| Push / slide / uncover | A and B sampled with translated UVs. Outside [0, 1] is transparent (bounds test, not clamp). Optional motion blur: N taps along the direction (FX-005) |
| Zoom / spin / 3D card, cube | each side is a homography of A or B, sampled with the place-pass machinery |

**Registry (FX-004).**
- `TransitionRegistry` holds manifests compiled into the binary: id, name,
  category, tags, `ParamSpec`s, default duration, default easing, and
  `clamp_eased: bool`.
- Unknown or failed kinds render as `freecut.dissolve` and set a warning
  (FX-002/004).
- **Imported wipe patterns (FX-023).**
  - Decoded off-thread by `freecut-media`.
  - Converted to luminance and uploaded as `R16Float` (G3).
  - Stretched to fill.
  - Cached by `AssetId`.
- **Porting gl-transitions (MIT repo).** Only per file, after checking that
  file's own license header (FX clean-room notes).

### 9. Effect layers, adjustment layers and background

- **Effect layer (FX-011).**
  - `fx = effect_chain(acc)` at target resolution, with
    `ref_height_px = target height`.
  - `acc' = acc + (fx − acc)·mix`. This is a premultiplied lerp. `mix` is
    keyframed, so it is evaluated.
  - Effect-local time is used for seeds and animation.
  - An effect layer over a transition affects the mixed result, because the walk
    already put it into the accumulator.
- **Adjustment layer (CL-006).** The same, with a color stack in canvas space:
  vignette and grain use canvas coordinates, and grain is seeded by the timeline
  frame index.
- **Accumulator ping-pong.** Both need to read and write the accumulator, so they
  swap it with a pooled twin.
- **Background (FX-001 step 1, TR-011…014):**

| Mode | How |
|---|---|
| Color | The accumulator's load op is `Clear` with the code values as given. They are not converted, because the target is not `*Srgb`. |
| Blur (TR-012) | Take the main-track clip's L3 output, cover-scaled, at ¼ target size. Gaussian with σ from 0–8 % of the canvas diagonal, using the pyramid above 24 px. Draw upscaled to full size. It follows speed, reverse and freeze, because it reuses the same normalized frame. |
| Image (TR-013) | Place pass with Fit/Fill/Stretch; first frame only for animated images in v1 |
| Transparent export (EX-048, later) | Clear to (0, 0, 0, 0). Blur and image backgrounds are dropped, and preflight warns. |

### 10. Parameter animation, time and determinism

- **The timeline evaluates everything time-dependent.** This includes keyframes
  (TR-027, source-time anchored), easing, animation presets (FX-018/019), speed
  maps, effect and color parameters, and Mix. The renderer receives values.
  - This keeps one implementation of every curve.
  - The renderer can never disagree with what the inspector shows (P14).
- **Time-dependent shader inputs.**
  - `LocalTimeF { seconds: f32, frame_index: i64 }`.
  - `seconds` is computed as `ticks as f64 / 705_600_000.0`.
  - For periodic motion, the phase `fract(seconds / period)` is reduced in `f64`
    first. After an hour, `f32` seconds have about 0.25 ms resolution, and
    `sin(ω t)` at high ω would visibly jitter.
- **Randomness.** `pcg_hash(seed_lo ^ hash(frame_index) ^ cell)` (Jarzynski &
  Olano, "Hash Functions for GPU Rendering", JCGT 2020 — a public algorithm),
  written in WGSL with `u32` arithmetic.
  - Nothing reads wall-clock time, and there are no atomics-based random streams.
  - Split keeps seeds (FX-010) and duplicates re-seed (CL-011). Both are timeline
    rules; the renderer just reads `seed`.
- **Preview = export (FX-001).** The same plan and shaders are used. Only
  `render_scale` differs. A golden test enforces equality "within resampling".
- **Same device.** Rendering the same plan twice is bit-identical. A test enforces
  this.
- **Across GPUs and backends** results are not bit-identical (fma contraction,
  transcendental precision), so golden tests use tolerances (Test strategy).

### 11. Resources: pools, caches, uniforms, pipelines

**`TexturePool`** (P12, P13):

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TexKey { pub w: u32, pub h: u32, pub format: wgpu::TextureFormat, pub mips: u32 }

pub(crate) struct TexturePool {
    free: HashMap<TexKey, Vec<Idle>>,   // Idle { tex: wgpu::Texture, last_used_frame: u64 }
    live_bytes: u64,
    idle_bytes: u64,
    budget_bytes: u64,                  // RenderConfig; default 1.5 GiB (no portable VRAM query); revised: set by the app's MemoryBudget (README §5.12)
    frame: u64,
}

impl TexturePool {
    /// Rounds w/h up to a 64 px bucket; the caller gets a content rect (`PlaceU::content`).
    pub fn acquire(&mut self, gpu: &GpuContext, want: TexKey) -> Result<Pooled, RenderError>;
    pub fn release(&mut self, t: Pooled);
    /// Destroys idle textures unused for >120 frames, then LRU down to budget.
    pub fn end_frame(&mut self);
}
```

- **Usages.** All pooled textures are created with
  `RENDER_ATTACHMENT | TEXTURE_BINDING | COPY_SRC | COPY_DST`, so any free texture
  of the right size can be reused.
- **Present slots are separate.** They are owned by the preview ring, are never
  pooled, and are never written while shown (playback-engine.md). This addresses
  P13.
- **Eviction** calls `Texture::destroy()`, so memory is returned now rather than
  when the last handle drops.
- **Memory guide.** One canvas `Rgba16Float` texture is:
  - 1080p: 16.6 MB;
  - 4K: 66 MB;
  - 8K: 265 MB.

  A 4K frame with a transition and an advanced blend peaks at about 6 canvas-sized
  textures plus layer textures, roughly 0.5 GB. This is why intermediates are
  layer- or bbox-sized (P20).

**Other caches.** All are LRU with byte budgets inside the pool's budget:
- `SourceCache` — normalized sources (§6);
- `RasterCache` — text/sticker rasters keyed by `(content hash, √2-quantized raster size)`;
- `LutCache` and `CurveCache` — keyed by content hash and revision (CL-004);
- `HistoryCache` — temporal effects;
- `StyleCache` — OV-006.

All are cleared on device loss.

**Uniforms.** There is one per-frame arena buffer. All pass uniforms are written
with one `write_buffer` and bound with dynamic offsets, aligned to
`min_uniform_buffer_offset_alignment` (256 by default).
- **Layout discipline.** Rust structs are `#[repr(C)]` +
  `bytemuck::Pod`, and composed only of `[f32; 4]`, `[u32; 4]` and 3 × `[f32; 4]`
  for a `mat3x3`. WGSL pads a `mat3x3<f32>` column to 16 bytes, which is a classic
  mismatch.
- **Layout test.** A test parses every shader with naga, computes each uniform
  struct's layout with `naga::proc::Layouter`, and asserts that size and member
  offsets equal `size_of`/`offset_of!` of the Rust twin.

**Pipelines.**
- `PipelineCache: HashMap<PipelineKey, wgpu::RenderPipeline>`, where the key is
  `(shader id, entry, target format, blend state, override constants)`.
- Specialization uses WGSL `override` constants passed through
  `PipelineCompilationOptions::constants`, for example `HAS_MASK` and
  `ADVANCED_BLEND`. This avoids both an uber-shader and per-feature source
  variants.
- Common pipelines are created at startup on a worker thread, which avoids
  first-use hitches.
- **Samplers** are created once: `linear_clamp`, `nearest_clamp`,
  `trilinear_aniso_clamp`, `linear_repeat` (patterns).

**Shader source.**
- A shared prelude (`common.wgsl`: color helpers, `unpremul`, hash, SDFs) is
  concatenated at compile time with `concat!(include_str!(…))`.
- Each composed module is validated in tests with naga: parse + `Validator`, all
  capabilities off except those we request.
- naga_oil-style `#import` can come later if the library grows.

### 12. Threading, device sharing and presentation

- **The device.**
  > **Revised (review, 2026-10-12; README §2.3, C43).** `WgpuSetup::Existing` with an
  > app-created device loses what eframe's `CreateNew` path does: adapter choice with
  > `compatible_surface` (`egui-wgpu-0.36.2/src/lib.rs`), the injected display handle that
  > is "Required on some platforms (e.g. Wayland with GLES)" (`setup.rs:165,186`), and
  > egui's 8192 px / GL downlevel limits (`setup.rs:261-273`). eframe now creates the
  > device with `WgpuSetup::CreateNew`, using our `device_descriptor` closure (the feature and
  > limit requests below, made from `adapter.features()`/`limits()`) and our
  > `native_adapter_selector` (UI-028). `GpuContext` is built from
  > `CreationContext::wgpu_render_state`; the error handler and device-lost callback are
  > installed right after (`wgpu-30.0.1/src/api/device.rs:506,681`). The list below is the
  > original plan.
  - `freecut-app` creates the `wgpu::Instance`, adapter (UI-028 choice),
    `Device` and `Queue`.
  - It installs an uncaptured-error handler. The default handler panics.
  - It installs a device-lost callback.
  - It hands them to eframe via `egui_wgpu::WgpuSetup::Existing`. egui-wgpu
    0.36.2 depends on wgpu 30.0, so these are the same types and textures need no
    copy.
  - `GpuContext` is a clone of `Device` + `Queue` plus negotiated capabilities
    (`GpuCaps`).
- **Features requested when available:**
  - `TEXTURE_FORMAT_16BIT_NORM` (Vulkan/DX12/Metal/desktop GL), for 10-bit planes;
  - `TIMESTAMP_QUERY`, optional, for EX-017 GPU timings.
  - Limits are raised to the adapter's `max_texture_dimension_2d`. The default is
    8192, which 8K UHD (7680) fits, but large stills need more (TR-002 edge case).
- **One `Compositor` per thread.** It owns its pools and caches. It is `Send` but
  not `Sync`.
  - Preview, export and thumbnails each have their own. They share
    `Arc<ShaderLibrary>` and the pipeline cache behind a `RwLock` that is written
    only when a new key is created.
  - All submit to the one `Queue`, which orders their work (P13).

```rust
pub struct Compositor { /* GpuContext, pools, caches, Arc<ShaderLibrary>, registries */ }

impl Compositor {
    pub fn new(gpu: GpuContext, cfg: RenderConfig, lib: Arc<ShaderLibrary>) -> Result<Self, RenderError>;

    /// Composites `plan` at `target` size into `out`. Blocks only on `frames` (decode) and,
    /// for export, on the oldest readback in flight. Never called on the UI thread.
    pub fn render(
        &mut self,
        plan: &FramePlan,
        target: &RenderTarget,          // resolved pixel size + proxy policy (playback-engine.md)
        out: Output<'_>,
        frames: &dyn FrameSource,
        cancel: &CancelToken,
    ) -> Result<FrameReport, RenderError>;
}

pub enum Output<'a> {
    Present(&'a PresentSlot),           // Rgba8Unorm ring slot (playback-engine.md)
    Export(&'a mut ExportFrame),        // Y'CbCr planes -> readback ring (EX-022)
    Still(&'a mut StillImage),          // EX-014 snapshot, CL-003 thumbnails, FX-004 tiles
    Texture(&'a wgpu::TextureView),     // tests, scopes
}

pub trait FrameSource: Send + Sync {
    /// Implemented by freecut-media's DecoderPool; tests use synthetic frames.
    fn frame(&self, asset: AssetId, at: Tick, mode: SeekMode) -> Result<Arc<DecodedFrame>, DecodeError>;
}

pub struct FrameReport {
    pub warnings: SmallVec<[RenderWarning; 2]>, // unavailable effect, missing LUT, slate used, ...
    pub gpu_time: Option<Duration>,             // TIMESTAMP_QUERY when available (EX-017)
}
```

- **Export pipeline (EX-022).**
  - The Y′ and CbCr targets are `R8Unorm`/`Rg8Unorm` for 8-bit, or
    `R16Uint`/`Rg16Uint` for 10-bit. Integer code values are written directly,
    LSB-aligned, which matches `yuv420p10le`. `R16Unorm` is not renderable in core.
  - They are copied to a ring of 3 `MAP_READ` staging buffers with rows padded to
    256 bytes.
  - `map_async` + `device.poll(PollType::Wait { submission_index, timeout })`
    waits on the oldest only. The padding is stripped into the encoder's frame.
  - Frames go into the bounded channel (8) to the encoder thread.
- **Preview.** One or two frames in flight, set by the scheduler's ring
  back-pressure (playback-engine.md). The compositor never waits on the GPU for
  preview.
- **Long GPU work.** Heavy frames (8K, many passes) are split into several
  submissions to stay under OS GPU-timeout thresholds (Windows TDR ~2 s). Large
  blurs use pyramids.

### 13. Output passes

- **Present.** Accumulator → `Rgba8Unorm` slot:
  - resampled when the target size and slot size differ;
  - ordered dither before 8-bit quantization;
  - optional matte preview, compare split, and checkerboard under transparency
    (EX-048).

  Zoom, pan and guides are done by the egui paint callback (playback-engine.md).
- **Export.** Y′CbCr per §3 and G2. The EX-042 Fit/Fill reframe is folded into
  the canvas-to-output transform: the canvas is rendered directly at output
  geometry, never resampled twice (EX-042 model notes).
- **Still / thumbnails.** `Rgba8Unorm` or `Rgba16Float` readback through the same
  padded-row helper.

### 14. Errors, degradation and device loss

```rust
#[derive(Debug, thiserror::Error)]
pub enum RenderError {                          // superset of the variants in playback-engine.md
    #[error("GPU device lost: {0}")]
    DeviceLost(String),
    #[error("out of GPU memory while allocating {what} ({bytes} bytes)")]
    OutOfMemory { what: &'static str, bytes: u64 },
    #[error("invalid render size {0}x{1}")]
    InvalidSize(u32, u32),
    #[error("{w}x{h} exceeds the device limit of {max} px")]
    TooLarge { w: u32, h: u32, max: u32 },
    #[error(transparent)]
    Decode(#[from] DecodeError),                // export only; preview substitutes a slate
    #[error("readback failed")]
    Readback(#[from] wgpu::BufferAsyncError),
    #[error("cancelled")]
    Cancelled,
}
```

- **Degrade, don't fail.** These become `RenderWarning`s, never errors:
  - unknown effect → bypass;
  - unknown transition → dissolve;
  - shader unavailable → bypass or dissolve;
  - missing LUT or look → identity (CL-017);
  - offline media → slate (EX-005);
  - decode error in preview → error slate;
  - analysis not ready → effect bypassed with a badge (FX-020).
- **Allocation failures** are caught with an `OutOfMemory` error scope around pool
  growth. The pool evicts everything idle and retries once before returning
  `OutOfMemory`.
- **Device loss.** The callback sets a flag. The next `render` returns
  `DeviceLost`, and all caches are dropped. The app recreates the device and
  compositors (EX-001; see Risks for the eframe side).

### 15. Module layout

```
crates/freecut-render/
  src/lib.rs            Compositor, RenderConfig, Output, FrameReport, RenderError, RenderWarning
  src/gpu.rs            GpuContext, GpuCaps, feature/limit negotiation, device-lost flag
  src/compile/          FramePlan -> PassGraph (CPU only): stage.rs, chain.rs, fuse.rs, layer_res.rs, bbox.rs
  src/exec/             graph executor, uniform arena, readback ring, padded-row helper
  src/pool.rs           TexturePool + SourceCache, RasterCache, LutCache, HistoryCache, StyleCache
  src/upload/           DecodedFrame -> planes, yuv.rs (affine, siting), alpha modes, frame blend
  src/color/            working space, transfer fns, hdr.rs, compiled stack, lut.rs, curves.rs
  src/layer/            crop.rs (L3), key.rs (L4), color pass (L5), place.rs (L7-L10), mask.rs, style.rs
  src/blend.rs          BlendMode -> shader index; CPU reference of every W3C formula
  src/transitions/      registry + built-ins + luma maps
  src/effects/          registry, manifests, packer, built-ins, history
  src/background.rs     TR-011..014
  src/output/           present, export (Y'CbCr + dither), still
  src/slate.rs          EX-005 procedural slates
  src/scopes/           CL-021 histogram (compute; CPU fallback)
  src/preview/          scheduler, present ring (playback-engine.md)
  src/text/             TX-owned layout/raster (text-and-captions.md)
  src/reference/        CPU reference implementations (tests, LUT baking, CL-005)
  shaders/              common.wgsl + one file per pass family
  tests/                naga validation, uniform layout, GPU golden tests
```

### 16. How this meets the project conventions

| Convention | How |
|---|---|
| Time is rational, floats only at the edge | The plan carries `Tick`. Progress and phases are computed from integers in `f64`, then cast to `f32` once in the uniform packer. |
| Never decode, render or do file I/O on the UI thread | The compositor lives on the scheduler, export and thumbnail threads. The UI only registers and samples present slots. |
| Every edit goes through undo | The renderer never edits. Preview overrides (FX-016, UI) are plan patches applied in render, never model writes. |
| `thiserror` in libraries | `RenderError`, `RenderWarning`. |
| `unsafe` only in media/render with `// SAFETY:` | None needed in v1. |
| Model crate stays pure | No GPU types leak into `freecut-timeline`. `FramePlan` is plain data. |

### 17. Interim CPU compositor (M4/M5 slice, 2026-10-12, README C44)

The wgpu design above is not built yet. Until it is, `freecut_render::cpu::Compositor`
is the one renderer for preview (`freecut-app/src/preview.rs`) and export
(`freecut_render::export::ExportRenderer`, driven by `freecut-app::jobs::export`), and
it is the CPU reference the GPU path must match in its goldens. What it does today:

- **Pixel format.** Straight-alpha sRGB RGBA8 in and out (not the premultiplied
  `Rgba16Float` working space of §2). Export composites the result over black before
  the encoder.
- **Sampling and blending.** Bilinear sampling with 8-bit fixed-point weights; colours
  are weighted by alpha so transparent texels never darken an edge; edges clamp. The
  "over" blend uses integer arithmetic with rounded division. Only Normal blend; layer
  opacity (`LayerPlan.opacity`) multiplies layer alpha.
- **Edge antialiasing** (FreeCut default). Each output pixel is a 1 × 1 box; its
  coverage by the layer rectangle is measured against each pair of opposite edges,
  perpendicular to them in output pixels (the source coordinate divided by the length
  of its gradient), and the two fractions are multiplied into the layer alpha
  (quantized to 0–255). Pixels within half a pixel outside an edge are drawn with
  partial coverage, sampling the picture clamped to its border. Integer-aligned edges
  are exact (coverage 0 or 1, so un-rotated, whole-pixel layers are unchanged);
  rotated layers get smooth edges and sub-pixel motion moves edges smoothly. The
  separable box is exact for axis-aligned edges and a close approximation at corners
  of rotated layers. Deterministic: plain `f64` arithmetic per pixel.
- **Minification** (FreeCut default). When a picture steps √2 or more source texels
  per output pixel (the longer of the two output axes), it is sampled from a mip level:
  level `k = round(log2 ρ)`, each level a 2 × 2 alpha-weighted box reduction of the one
  above (sizes rounded up). Levels are built on first use and kept while the picture is
  alive (at most 32 pictures), so a paused preview or a still builds them once.
  Bilinear within the level; no trilinear blend between levels. Video layers drawn at
  less than 1/√2 of their display size on both axes are asked for with
  `LayerImages::video_scaled`, and `MediaImages` then converts the frame at about the
  drawn size with swscale (`VideoDecoder::frame_at_size`): the 1080p-in-a-panel and
  4K-to-720p cases neither convert full-size RGBA nor alias.
- **Output scaling.** Canvas → output uses independent x and y scales; `fit_size`
  gives the preview's aspect-preserving, never-upscaling size and `export::fit_rect`
  the export's centred, letterboxed rectangle (exact integers, upscaling allowed).
- **Orientation.** Display rotation and mirroring live on `Picture` (mirror first,
  then rotate, the same convention as `freecut-media`'s `orient_rgba`), and are mapped
  back to the stored picture when sampling.
- **Missing pictures and placeholders.** A missing or invalid picture draws the
  offline slate (`SLATE`). Generated sticker/effect/adjustment layers draw the
  translucent `PLACEHOLDER` until their renderers exist. Text layers go through
  `LayerImages::text`, which defaults to `TextLayer::from_plan`; an empty text clip
  draws nothing (TX-002).
- **Text rasters** are made at the final output size: the scale is the larger axis
  length of `output · layer_to_canvas`, limited so neither raster side exceeds
  0.75 × 8192 px. For an upright, unmirrored layer the scale is used exactly, and when
  the x and y scales agree to within half a pixel over the raster (the preview's
  `fit_size` rounding qualifies) its position is rounded to whole output pixels, so it
  is copied 1:1 (crisp, TX-004); other layers are resampled from a raster whose scale
  is rounded up to 1/64 so nearby scales share a cache entry. Layout itself never depends on that
  scale (text-and-subtitles.md, implementation note).
- **Determinism.** Rows are split into bands across threads and every pixel is
  computed independently, so output bytes are identical for any thread count and
  frame order. Release timing at 1080p with 3 layers (edge antialiasing included):
  about 5 ms on 8 threads, about 23 ms on 1 thread.
- **Decoding stays outside.** Pictures come from the `LayerImages` trait;
  `freecut_render::media::MediaImages` (default feature `media`, which turns on
  `freecut-media/ffmpeg`) keeps one decoder per asset. Without the feature the crate
  builds with no FFmpeg. A file that can't be opened or a frame that can't be decoded
  draws the slate and is reported through `LayerImages::take_failures` (an offline
  file, with no path, is not a failure). `ExportRenderer::render_frame` turns a failure
  into `ExportRenderError::Decode { clip, at, reason }` (EX-022 `decode-error`), unless
  `with_slates_for_unreadable()` was set, which draws the slate and lists the frame in
  `unreadable()`.
- **HDR sources** are tone-mapped to SDR Rec.709 during decode (CL-002) by
  `freecut-media`'s CPU reference curve, before the 8-bit RGBA the compositor sees; see
  media-io.md's implementation note. The L2 shader of §2 will replace it.
- **No `unsafe`.** `freecut-render` keeps `#![forbid(unsafe_code)]`.
- **Premultiplied text layers** (README C46, Text round 2). Text rasters are
  premultiplied RGBA (text-and-subtitles.md, "Round 2 render implementation") and drawn
  as `PreparedSource::Premultiplied`: bilinear sampling weights all four channels alike,
  layer opacity and edge coverage scale all four, and the blend is premultiplied "over"
  onto the straight-alpha frame (`out = src + dst·(1 − αs)`, through premultiplied form for
  a translucent destination). So a glow pixel with α = 0 adds light (over black it shows,
  over white it saturates) and a pixel with nothing never darkens. Light falling on a fully
  transparent destination pixel is dropped (straight alpha can't hold it); canvases are
  opaque in practice. Premultiplied mip levels average all four channels. Other layers
  stay straight alpha. Release timing at 1080p of one static styled title (outline,
  shadow, glow; 120 px bold): about 3 ms per frame cached on 8 threads (5 ms on 1), about 90 ms for the first, uncached frame that rasterizes the effects (measured on a heavily loaded machine; about 22 ms since S2's blur passes, below).
- **Per-unit text motion** (S2, README C46). `TextLayer::motion: Option<TextMotion>`
  carries the per-unit animation of the frame (text-and-subtitles.md, "Round 2 render
  implementation: per-unit animation"); `TextLayer::from_plan` fills it from the plan's
  `LayerMotion` (the timeline's `TextAnimPlan` and the Reveal key, adapted by
  `freecut_render::motion`, text-and-subtitles.md "the plan adapter"). Frames are planned
  with the preset catalogue and layout-measured text boxes by `Compositor::plan` (export:
  `ExportRenderer::with_presets`). `prepare_text` then calls
  `TextRenderer::rasterize_animated`: the glyphs are posed in the text's local frame
  (TX-022 step 3) and the styled layers drawn around them (step 4), and the raster is
  placed by `layer_to_canvas` exactly as a still one, which is where the frame plan folds
  the slot's block offsets into the keyframed transform (step 5). Animated rasters are
  cached by the spec plus a digest of the per-glyph state, in at most 16 MB of the 64 MB
  text cache, so a playing animation never evicts still titles; a state that looks as the
  rest does reuses the still raster.
- **Gaussian blur passes** accumulate whole rows tap by tap in kernel order (both
  passes row-major, so they vectorize); each output's sum is formed in the same order as
  before, so results are bit-identical (the goldens are unchanged) and the blurs run
  about four times faster than with the column-strided vertical pass. Timing at 1080p of
  a 160 px bold title with outline, shadow and glow, measured on a heavily loaded machine
  (load average about 25, so numbers vary by ±50 %): the first, uncached still frame takes
  22–60 ms in release (was about 130 ms); while TX-056's Letter rise plays, every frame is
  a new raster and costs about as much, 39–50 ms on average in release and debug alike
  (this crate builds at opt-level 2), with spikes to about 100 ms; the per-unit work
  itself (evaluation, posing, the opacity plane) is a small part of it. The rest is the
  silhouette's signed distance field (two exact distance transforms, about 13 ms) and the
  blurs. The acceptance target (the preview plays such a title at the sequence rate on
  the reference machine) needs a measurement on an unloaded machine; the distance
  transform is the next thing to profile.
- **Whole-layer 3D turns** (S2b; FX-030's 3D flip, TX-056's Flip open). The frame plan
  folds the turn into `layer_to_canvas` with its perspective row (eye at 1.5 × the layer
  width), and the compositor draws any layer whose matrix has a bottom row other than
  exactly `[0 0 1]` projectively: the output → source map is the inverse
  `text::projective::Homography` (the glyph sampler's), each pixel's preimage is
  `(X / W, Y / W)`, and pixels whose preimage has `W ≤ 0` (at or behind the eye) are not
  drawn. Edge coverage uses the map's gradient at the pixel (the map is close to affine
  across one pixel), bounds are the projected corners (everything, when a corner is behind
  the eye), and the mip level is the least minified of the corners, so the near side stays
  sharp. An edge-on layer (determinant ≈ 0) draws nothing. *Back faces:* a layer turned
  past 90° shows its back mirrored, like a transparent sheet, as glyph turns do; the matrix
  alone can't tell a back face from a mirrored (flipped) layer, and no preset turns a whole
  layer that far. Text in perspective is rasterized at the largest local scale over its
  box corners and resampled. Affine layers without a wipe or tint keep the old path bit
  for bit (the goldens are unchanged).
- **Whole-layer blur, wipe and tint** (S2b, `cpu::LayerEffects` from the plan's
  `LayerMotion`). *Wipe* (FX-030's Wipe reveal): the share `u` across the layer rectangle
  from the starting side (Left, Right, Up = from the top, Down) is a ratio of linear forms of
  the output pixel (exact under perspective); the layer shows `clamp((e − u)/f + ½)` with
  the soft edge `f` = 8 % of the layer (`WIPE_FEATHER`) and `e = shown·(1 + f) − f/2`, so
  `shown = 0` hides everything and `shown = 1` is exactly the still layer; `u` is clamped
  to the rectangle so text glow and shadow beyond the box follow the nearest edge. Text
  wipes run over the box at rest. *Tint* (White flash reveal): straight colours move
  `k`/255 of the way to the tint colour; premultiplied ones to the tint at the pixel's own
  alpha (`c + (T·α − c)·k`), so coverage and alpha are kept and a full tint is exactly the
  tint colour (added glow light fades toward it too). *Blur* (Blur in): the layer, wipe
  and tint included, is drawn alone into a premultiplied buffer in output pixels covering
  the layer plus the blur's reach (clipped to the frame grown by that reach, so light from
  just outside bleeds in), each channel is blurred with the text effects' separable
  Gaussian (σ = radius / 2, the radius in canvas px scaled to the output) with
  power-of-two downsampling above 8 px (`LAYER_BLUR_DOWNSAMPLE_ABOVE`; text effects keep
  32 px), and the result is drawn 1:1 as a premultiplied layer. Rows and channels are split
  across threads with the same sequential arithmetic, so bytes don't depend on the thread
  count. Blurred layers are cached (4 entries, 96 MB) by everything that shapes them: the
  source picture's identity (held, so it can't be reused), the map, opacity, wipe, tint,
  radius and region, so a paused frame or a settled layer is blurred once. Release timing
  at 1080p of a 1920 × 1080 picture on a loaded 11-core machine (load ≈ 13), default
  threads: plain 3.2 ms, turned 35° 9.3 ms, turned and blurred 12 / 21.6 / 40 px about
  30 / 29 / 33 ms uncached and 3 ms cached (the 3-layer frame of above is unchanged:
  23.7 ms on 1 thread, 5.6 ms on 8).
- **Per-unit style channels and options** (S2b). `GlyphAnim::tint` moves a glyph's fill
  toward `TextMotion::tint_color` (white by default) at its own alpha; outline, glow and
  shadow keep their colours, and the per-glyph state digest (the raster cache key) includes
  it. The plan adapter maps `style.lightness` (Shimmer's band of brightness: each unit's loop
  delay makes it travel) and the white tint onto it. `motion::TextMotionOptions` carries what
  the model doesn't yet: Typewriter's **Caret** (off by default; drawn for presets whose
  letters appear with a step, `motion::is_stepped`, in In and Out slots; TX-056's blink is
  `CARET_BLINK`, 1 s) and the Decode step (`DecodeStep::Frames(2)` by default, TX-056's two
  frames at the plan's frame rate, or a fixed time). `Compositor::with_text_motion_options`
  applies them to every text layer it draws from a plan; export uses the compositor it is
  given, so preview and export agree when both are configured alike.

#### Color stack on the CPU path (README C52; K1 built 2026-10-14)

Adjustments round 1 (color-and-filters.md, slices K1–K6) grades on this compositor. The GPU
design of §6 L5 stays the target; this is the reference it will be checked against.

- **Inputs.** `LayerPlan.color: Option<Arc<ColorStack>>` holds the clip's stack with every
  keyed value already evaluated by the timeline (`Clip::color_at`, §10's rule that the
  timeline evaluates everything time-dependent). `None` means nothing to do: no stack, an
  identity stack or the master bypass. Adjustment clips arrive as
  `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })` in the bottom-to-top walk.
  User LUTs come from a `LutSource` (`fn lut(&self, hash: &[u8; 16]) -> Option<Arc<Lut>>`)
  given to `Compositor::with_luts`; the app's implementation looks in the bundled set
  (CL-045), then the user library, and loads on first use with `freecut_media::lut`.
- **Module layout.** `freecut_render::color`: `eval` (the reference evaluator
  `eval_pixel(&ColorStack, [f32; 3]) -> [f32; 3]` for the per-pixel stages, one function per
  stage in CL-001's order, shared with the bake), `bake` (`Lattice` and `BakeSize`), `apply`
  (lattice lookup, LUT-only fast path, final blend), `skin` (CL-030's mask), `curve` (1024-entry
  tables), `digest` (`ColorDigest`) and `CompiledStack` (the lattice plus the stages that run
  after it). `freecut_render::color::stats` holds CL-019 and CL-020's pure statistics.
  `freecut_render::shape::mask` holds the shared mask coverage (CL-031, TR-017).
- **Working space and math.** Straight-alpha RGBA8 code values, read as `f32` 0–1; the stack
  never touches alpha and skips pixels with α = 0. `ws_to_linear(v) = sign(v)·|v|^2.4` and its
  inverse (G5) bracket exposure, white balance and deflicker gain. *K1 measured:* Hue turns the
  Rec.709 luma/colour-difference plane of code values, not Oklab of linear light, because the
  Oklab turn was up to 19/255 off against the 33³ lattice (color-and-filters.md › *K1 as built*);
  saturation and vibrance mix toward code-value luma. Every transcendental call (`powf`, `cbrtf`, `atan2f`, `sinf`, `cosf`, `expf`)
  goes through the `libm` crate, never `std`'s platform functions, so a bake is bit-identical
  on every OS (§5.5). Rust never fuses multiply-adds on its own, so plain `f32` arithmetic is
  deterministic.
- **Compile.** `CompiledStack::new(&ColorStack, &dyn LutSource) -> CompiledStack` decides the
  path:
  - *identity* (`ColorStack::is_identity()`): never reached, the plan already says `None`;
  - *LUT only* (only the LUT section active, no skin protection, Amount 100): the user's own
    lattice, sampled with its interpolation and domain, lerped by intensity;
  - *lattice*: the per-pixel run starting at stage 4 is baked. The run ends before the first
    stage that is spatial, position-dependent or random (stage 15 sharpen/clarity/texture,
    stage 17 vignette, stage 18 grain); stage 16 fade and the stage-19 Amount blend are folded
    in only when nothing of that kind follows and the stack has no masks. What is not folded
    runs after the lattice as `TailStage`s on `f32` values.
  - Cleanup stages (deflicker, denoise; slice K7) run before the lattice as their own passes.
- **As built in K1.** `freecut_render::color` holds `eval` (`Evaluator`, `eval_pixel`), `bake`
  (`Lattice`, `BakeSize`, `bake`, `bake_with`), `grade`, `quantize`, `CompiledStack`,
  `ColorDigest` and the compositor's `GradeCache` (16 compiled stacks, 8 graded pictures); the
  LUT, skin, curve and stats modules arrive with their slices. Stages 5–6 are per-channel gains,
  so the bake works them out once per node value. Measured (release, 11-core machine under load
  ≈ 11): bake 33³ 1.2 ms on 8 threads (5.6 ms on 1), grading a 1080p picture 2.7 ms on 8
  threads (10.7 ms on 1); `color_bench` reprints these.
- **As built in K2.** `color::{curve, hsl, wheels}` add stages 8 (`WheelStage`: luma-free
  puck offsets weighted `1 − y`, `4y(1 − y)`, `y`, 1; lift, gain, a rational midtone curve and
  offset for the luminance sliders; all scaled by Strength), 9 (`CurvesStage`: 1024-entry
  Fritsch–Carlson `CurveTable`s, Luma then R/G/B, values clamped into 0–1 before the lookup)
  and 11 (`HslStage`: HSV-hue raised-cosine weights between the two neighbouring band centres,
  faded by chroma). `BakeSize::for_stack` picks 65³ when the steepest slope a channel goes
  through (the Luma table's times its own) exceeds `BakeSize::STEEP_SLOPE` = 4.
  `ColorDigest` now covers HSL, Curves (every point) and Wheels (pucks, luminance, Strength)
  while their sections are on, and `COLOR_ALGO_VERSION` is 2. `probe_hsl_input(stack,
  picture, x, y)` and `hsl_band_at(rgb)` serve CL-013's eyedropper; `Evaluator::hsl_input`
  is the stage-11 input. Measurements: color-and-filters.md › CL-001 › *K2 as built*.
- **As built in K3.** `color::{lut, skin}` add stage 13 and CL-030's mask. `LutSource` (`Send +
  Sync + Debug`, `lut(&hash) -> Option<Arc<Lut>>`) comes with `NoLuts` and the in-memory
  `LutSet`; `Compositor::with_luts`/`set_luts` take an `Arc<dyn LutSource>` (none: every LUT is
  missing). `Evaluator::with_luts`/`with_table` look the table up once; `eval_pixel_with` is the
  oracle with LUTs. The skin mask (`skin_mask`, HSV of the stack input with raised-cosine
  shoulders) blends stage 5 and stages 11–12 back toward their inputs (Color scope; while that
  scope is active white balance is applied apart from exposure so the blend sits between them)
  and scales the LUT's intensity (LUT scope). `CompiledStack::with_luts` picks the LUT-only
  fast path (only the LUT section active, no LUT skin protection, Amount 100: the user's table
  with its own interpolation and domain, per pixel) or the lattice; `CompiledStack::baked`
  forces the lattice for tests. `BakeSize::for_evaluator` is 65³ for a steep curve, a user
  table finer than 33³ or an active skin scope. A slot whose table its source lacks is dropped
  before compiling (the stage renders as identity, CL-017) and collected for
  `Compositor::take_missing_luts`; the digest then is the digest of the stack without its LUT,
  and a found LUT's hash, interpolation and skin switch are part of the digest. FreeCut's eight
  bundled LUTs (`assets/luts/`, CL-045) are baked from their recipes through `eval_pixel` by
  `tests/bundled_luts.rs`, which also checks that the checked-in files re-bake byte for byte.
- **As built in K4.** `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })` is
  emitted only for a non-identity evaluated stack. `render_into` draws the drawables collected
  so far with the banded `draw_rows` (now `Compositor::draw`), clears the list, and grades the
  frame in place with `color::grade_in_place` through `GradeCache::compiled` (the same
  digest-keyed cache of compiled stacks as clips; a missing LUT is collected the same way). The
  background is part of the frame, so it is graded; layers drawn later are not. Stages that
  look at neighbours (K7) and masks (K6) will run inside the same step. `tests/color_k4.rs`
  checks order, bottom-up stacking, Mix 0, thread counts, a golden PNG and preview == export
  for an adjustment clip over three clips and a title.
- **As built in K5.** Stage 4 is `color::matching::MatchStage`: `gain · v + offset` per
  Rec.709 `Y'`, `Cb`, `Cr` channel of code values, then `lerp(input, matched, intensity)`; it
  is affine, so the lattice reproduces it exactly, and while it is active the bake evaluates
  whole pixels (stages 5–6 are no longer per channel). The skin mask reads the stage-4 output
  and the Amount blend goes back to the stage-3b input. The evaluator reads the Light and Color
  sliders from `ColorStack::effective_basic()` (Auto's offsets at Strength added, each section's
  switch applied), so Auto is not a stage. `ColorDigest` hashes those effective sliders and an
  active match (`COLOR_ALGO_VERSION` 4). `color::stats` holds the pure analysis: `frame_stats`
  (per-channel and luma histograms of pixels with alpha above one half, and linear-light sums
  of the usable and the near-grey pixels), `auto_color` (white balance from the near-grey
  average, exposure and the range ends from the 0.5 %/99.5 % luma points, contrast from the
  resulting luma spread), `ycc_stats`/`match_stats` (Reinhard-style transfer with CL-020's
  limits), `analysis_picture` (integer box filter to ≤ 512 px) and `stack_input` (stage 4 on
  a picture). `tests/color_k5.rs` checks the casts, refusals, determinism across threads,
  Strength against plain sliders (same digest), self-match identity, limits, the lattice, two
  golden PNGs and preview == export with keyed Strength and Intensity.
- **As built in K6.** `freecut_render::shape::mask` holds `coverage(&ColorMask, item_px,
  point)`, `combined`, `MaskSet` (masks compiled once per grade) and `MaskSpace` (a buffer's
  pixel centres in shorter sides of the item: `MaskSpace::picture` maps a stored picture into
  its display frame through rotation, mirroring and pixel aspect, `MaskSpace::canvas` is an
  adjustment clip's frame). `CompiledStack` carries the masks and Amount; its lattice is the
  stack's without masks at Amount 100, and `GradeCache` keys that lattice by the unmasked digest
  and the graded picture by the full digest and the display size. `grade_displayed(stack,
  picture, display, threads)` is `grade` for a picture shown in a display frame (the compositor
  passes `LayerPlan::display_size`); `grade` uses the picture's upright size and
  `grade_in_place` the frame itself. Full coverage writes the lattice value, none leaves the
  pixel, alpha is never touched. `tests/color_k6.rs` checks coverage goldens per shape and for
  the three combine modes, monotonic feather, invert as the complement, a zero-size mask, alpha,
  clip masks following the clip and adjustment masks staying on the canvas, thread counts, a
  graded golden and preview == export for a keyed ellipse. `COLOR_ALGO_VERSION` is 5.
- **Bake.** `bake(&ColorStack, BakeSize) -> Arc<Lattice>`: for every node `(i, j, k)` of an
  `n³` grid (`n` = 33, or 65 when a curve's steepest baked segment exceeds a slope of 4 or the
  user LUT is larger than 33³), the input `(i, j, k) / (n − 1)` goes through `eval_pixel`'s
  folded stages, and the `f32` RGB result is stored unclamped, red fastest. Rows of the grid
  are split across threads; each node is independent, so the bytes do not depend on the thread
  count. Cost: 35 937 evaluations for 33³, 274 625 for 65³.
- **Apply.** Per pixel, three 256-entry tables (built once per lattice size) give each 8-bit
  channel's cell index and fraction (`v·(n − 1)/255`). The cell's fractions are sorted to pick
  one of its six tetrahedra, and four lattice values are blended with the standard tetrahedral
  weights. Tail stages follow, then the single quantization
  `round(clamp(v, 0, 1)·255)`. Trilinear (CL-017's option) is only used for the user's table on
  the LUT-only path and inside the bake's LUT stage.
- **Clips.** `Compositor::prepare` grades the picture right after `LayerImages` returns it
  (after `video_scaled`, so a small picture-in-picture is graded small) and before
  `Prepared::picture` builds mips, so mip levels are made from graded pixels. Spatial radii and
  mask coordinates are relative to the picture's display height, so they do not depend on the
  decoded size. Graded pictures are new `Arc<Picture>`s held in a `GradeCache` of 8 entries
  keyed by (source picture pointer, kept alive by the entry, and `ColorDigest`); a paused frame
  or a still is graded once. Slates and text are never graded (CL-001).
- **Adjustment clips.** `render_into` walks the nodes in order and collects drawables as now.
  At an `Adjust` node it draws what it has collected into the frame (the existing banded
  `draw_rows`), clears the list, and grades the frame in place in bands: lattice, tail stages,
  masks in canvas coordinates scaled to the output, and the Mix blend. Grain on an adjustment
  clip seeds by `frame_index` (CL-011, K7). The canvas background is part of the frame, so it
  is graded too (CL-006). Generated adjustment layers no longer draw `PLACEHOLDER`.
- **Masks (K6).** Coverage per pixel from `shape::mask::coverage(&ColorMask, item_size,
  point) -> f32` (signed distance in units of the item's shorter side, Expand, smoothstep
  feather, invert, list-order combine), then `lerp(base, graded, amount × coverage)` per
  channel in `f32`, where `base` is the lattice input (the stage-3b image). For a clip the
  point is the picture pixel mapped to the display frame through `Picture::rotation` and
  `mirrored`.
- **Caches and keys.** `ColorDigest` = XXH3-128 over a canonical byte encoding of the
  evaluated stack, the LUT hashes it names, the bake size and `COLOR_ALGO_VERSION` (bumped
  whenever any color math changes, like `RENDER_ABI_VERSION`). Lattices: 16 entries,
  shared by digest. When `render_key` (§5.5) lands it hashes the same digest.
- **Budget (release, 1080p, 8 threads, reference machine).** Bake 33³ ≤ 3 ms (≤ 15 ms on one
  thread); grading one full-frame picture ≤ 4 ms (≤ 25 ms on one thread); an adjustment clip
  over the whole frame ≤ 4 ms; each mask ≤ 2 ms. A keyframed grade re-bakes only when an
  evaluated integer value changes, so a slow ramp re-bakes on some frames, not all. An ignored
  test, `color_bench`, measures these lines like the text timings above.
- **Parity tests.** `eval_pixel` versus the lattice on a 32³ sample of 8-bit inputs for a set
  of corner stacks (max ≤ 2/255, mean ≤ 0.4/255; 65³ for the steep-curve cases); a forced
  identity bake reproduces all 8-bit inputs; 1 and 8 threads give identical bytes; the preview
  `Compositor` and `ExportRenderer` give identical bytes for one plan; golden PNGs of FreeCut's
  test chart per control extreme, checked in and later used as the GPU goldens.

Still to do on this path (not the GPU design): blend modes, clip masks (color masks
share their core, above), effects and transitions;
trilinear filtering between mip levels; per-glyph hue and glow modulation (only the
tint/lightness style channel has a per-glyph form).

---

## Rust ecosystem

Versions were checked with `cargo info` on this machine (October 2026).

### Use

| Crate | Version | License | Why |
|---|---|---|---|
| `wgpu` | 30.0.1 | MIT OR Apache-2.0 | GPU API. Pinned to the major that egui-wgpu uses. |
| `egui-wgpu` / `eframe` (app side) | 0.36.2 | MIT OR Apache-2.0 | Depends on `wgpu 30.0`, so the device and textures are shared. *Revised in review:* `WgpuSetup::CreateNew` with our device descriptor and adapter selector, not `Existing` (README §2.3). |
| `naga` (dev; runtime for FX-025 later) | 30.0.1 | MIT OR Apache-2.0 | WGSL parse + validate in CI. `proc::Layouter` drives the uniform layout tests. |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | `Pod` uniform structs and plane uploads |
| `glam` | 0.34.1 | MIT OR Apache-2.0 | `DMat3` homographies (plan, f64), `Mat3`/`Vec2` (f32 upload). Shared with the timeline crate. |
| `half` | 2.7.1 | MIT OR Apache-2.0 | f16 upload of luma maps and LUTs; f16 readback in tests |
| `smallvec` | 1.16.3 | MIT OR Apache-2.0 | Pass inputs, frame-blend samples (avoid the 2.0 beta) |
| `thiserror` | 2 | MIT OR Apache-2.0 | Errors (project rule) |
| `pollster` (dev) | 1.0.1 | Apache-2.0 / MIT | Block on adapter/device futures in headless tests |
| `image` (dev) | 0.25.10 | MIT OR Apache-2.0 | 16-bit PNG golden files |
| `proptest` (dev) | 1.12.0 | MIT OR Apache-2.0 | Property tests for blend/key/mask/YUV math |

### Consider later

| Crate | Version | License | Note |
|---|---|---|---|
| `naga_oil` | 0.23.0 | MIT OR Apache-2.0 | `#import` composition, depends on naga 30. Adopt only if the `concat!` prelude becomes painful. |
| `wgsl_to_wgpu` | 0.19.0 | MIT | Generates Rust bind-group and uniform types from WGSL, removing a class of layout bugs. It needs a build script. Weigh against our naga layout test. |
| `encase` | 0.13.0 | MIT-0 | WGSL layout derive. 0.13 has no glam integration, so it adds a proc macro and conversions for little gain over the all-`vec4` discipline. |
| `lyon_tessellation` | 1.0.22 | MIT OR Apache-2.0 | Option for Bezier path masks (TR-021) and vector stickers. A GPU coverage pass is the alternative. |
| `xxhash-rust` | 0.8.19 | BSL-1.0 | Stable 128-bit hashes for `RenderKey` and cache keys (playback-engine.md also uses XXH3) |

### Avoid

- **OpenCV bindings, frei0r, FFmpeg avfilter for rendering.** These are CPU paths
  with version-dependent output, which breaks preview = export determinism. They
  are heavy native dependencies, and frei0r plugin licenses vary per plugin.
- **OCIO via FFI.** There is no maintained Rust binding, and it needs a C++
  toolchain on all three OSes. Our fixed Rec.709/2020 + PQ/HLG math covers the
  specs. Revisit for ACES (CL out of scope).
- **`ash` / `vulkano` / `glium` directly.** wgpu covers everything here on all
  three OSes. GL is deprecated on macOS.
- **wgpu `TextureFormat::NV12`/`P010` and `ExternalTexture`.** NV12/P010 are not
  supported on Metal and are sample-only, unfilterable. `EXTERNAL_TEXTURE` is
  marked in progress and is not on Vulkan.
- **Runtime CPU resizing (`image::imageops`, `fast_image_resize`).** The runtime
  path is GPU only. CPU code is limited to `reference/` for tests.
- **Copying shader code from Shadertoy, gl-transitions or blog posts** without a
  per-file license check (FX and CL clean-room notes). Inigo Quilez's SDF articles
  are fine as *technique* references; we derive our own code.

---

## Test strategy

**1. CPU reference first (`src/reference/`, always runs, no GPU).**
- One pure Rust function per per-pixel shader stage:
  - YUV affine + siting;
  - transfer functions, HDR tone map;
  - key, spill;
  - every blend mode;
  - dissolve, wipe, luma wipe;
  - mask SDFs + feather;
  - color stack stages;
  - LUT trilinear/tetrahedral;
  - Y′CbCr output + dither.
- Unit tests on hand-computed values:
  - BT.709 limited 8-bit: Y 235 / C 128 → (1, 1, 1); Y 16 → 0;
  - 10-bit LSB vs P010 MSB give identical results;
  - full range vs limited;
  - each W3C mode on the pairs (0.25, 0.75), (0, 1), (1, 0);
  - Color Dodge and Burn at 0 and 1.
- **Property tests (proptest):**
  - **Blend.** All modes map [0, 1] inputs to [0, 1]. Normal with αs = 1 gives
    src, and with αs = 0 gives dst.
  - **Transitions.** Dissolve at e = 0 / 1 gives A / B. Luma-wipe output is
    monotone in p, all-A at p = 0 and all-B at p = 1 for every softness.
  - **Masks.** Coverage is monotone in feather. Invert(invert) is the identity.
  - **Round trips.** Premultiply/unpremultiply round-trips for α > 1e-3.
    RGB→Y′CbCr→RGB stays within quantization.
  - **Color stack corners.** Every color stack corner value is finite (CL-007,
    CL-015).

**2. Shader hygiene (no GPU).**
- Every composed WGSL module parses and validates with naga, including the
  uniformity rules (catches P8-style derivative misuse).
- Every uniform struct's naga layout equals its Rust twin (size + member offsets).
- Every manifest's parameter specs are well-formed: defaults inside ranges, and
  `Length` parameters declare a reference.

**3. Compiler tests (no GPU).** Feed `FramePlan` fixtures and assert:
- stage order (try to push out of order: a debug panic, tested with
  `#[should_panic]`);
- which fast path was taken;
- pass counts;
- bbox clipping;
- that the opacity-0 and off-canvas skips happen;
- resource lifetimes (no texture read after release; peak live bytes under a
  bound);
- that `transition_progress` is exact for NTSC frame durations, 1-frame windows
  and windows of up to 10 s.

**4. GPU golden tests (`tests/gpu_*.rs`).**
- **Harness.**
  - A headless `GpuTestContext` picks an adapter from `WGPU_BACKEND`.
  - If none is found, tests are skipped with a message, unless
    `FREECUT_REQUIRE_GPU=1`, which fails CI.
  - CI: Linux with Mesa lavapipe (software Vulkan, real shader execution), and
    the macOS runner (Metal). Windows WARP (DX12) later.
- **Size.** 16×16 to 64×64 targets.
- **Comparison.**
  - GPU vs CPU reference for every stage, with tolerance |Δ| ≤ 2/1023 per channel
    (float targets).
  - Golden images: 16-bit PNG per effect and transition at a fixed seed and time
    (FX-015 requires 32×32 per effect), tolerance ≤ 1/255 per channel or
    PSNR ≥ 50 dB.
  - Goldens are regenerated only by an explicit `cargo xtask bless-goldens`, and
    the diff images are kept as CI artifacts.
- **Cases:**
  - **YUV.** Planar 8/10-bit, NV12, P010, odd sizes (17×9), left vs centre
    siting. A chroma edge must land within ¼ pixel of the reference.
  - **Alpha.** A straight-alpha gradient with garbage RGB under α = 0 shows no
    fringe after downscale and rotation (P1).
  - **Edge AA.** A rotated white quad on black has edge coverage symmetric within
    1/255 and no stairs (P8).
  - **Transforms.** Flip, then rotate and scale about the anchor, matching the
    timeline's matrix applied to corner points.
  - **Masks.** Each SDF at feather 0 and 50. Non-square clips keep circles round.
  - **Blend modes.** Every mode over a 3-tone backdrop with partial source alpha.
  - **Transitions.** Every kind at e ∈ {0, 0.25, 0.5, 1}. Alpha sources over a
    lower track.
  - **Effect and adjustment layers** with Mix 0 / 0.5 / 1.
  - **LUTs.** An identity 33³ LUT is within 1e-3 under both interpolations. A
    known 2³ LUT gives exact corners.
  - **Fast-path equality** (P21): fast and slow paths are byte-identical on the
    same device.
  - **Determinism.** Rendering twice is bitwise equal. Grain is identical at
    frame n and different at n + 1.
  - **Resolution independence.** Render at scale 1 and ½, downsample the first,
    and require PSNR ≥ 40 dB for every effect and transition (FX-001).
  - **Layer-resolution steps.** Grain and patterns are unchanged when `k` crosses
    a √2 step.
  - **Output.** A known RGB ramp gives BT.709 limited code values exactly
    (EX-032). Dither is deterministic per frame index. Chroma siting follows G2.
  - **Readback.** Odd widths (row padding) and a 3-deep ring with out-of-order
    `map_async` completion.
  - **Device-lost path.** A simulated flag drops caches and returns `DeviceLost`.
    After recreation, the same plan renders the same bits.

**5. Fixtures.**
- Mostly synthetic, generated in test code:
  - YUV planes with known codes;
  - SMPTE-like bars computed from formulas;
  - alpha checkerboards;
  - 2³ and 33³ LUTs;
  - procedural luma maps.
- A handful of tiny encoded files (H.264 4:2:0 BT.709 limited; JPEG full range;
  ProRes 4444 with alpha; VP9 with alpha; PQ HEVC 10-bit; an odd-size file).
  - Generated once by a documented `xtask gen-fixtures` script with the system
    FFmpeg 8.1.1.
  - Committed under `crates/freecut-render/tests/fixtures/` (each < 100 KB).
  - CC0, our own content.
- Optionally, Kdenlive's CC0 `data/lumas/HD/*.pgm` as real-world wipe maps, with
  attribution in `assets/ASSETS.md`.

**6. Benchmarks (later).** `cargo bench` for:
- 1080p and 4K frames with N layers, a transition and a 3-effect stack, on lavapipe
  and Metal;
- tracked so that FX-001's "heavy stacks degrade only frame rate" stays true.

---

## Risks & open questions

1. **G4 layer-space resolution.** Proposal: `k` from the transform's σ, quantized
   in √2 steps. It keeps zoomed 4K sharp, at up to 4× pixel cost per zoomed layer.
   FX-001 needs to accept "layer space is a size in units, not in pixels".
   *Recommendation:* accept. Cap `k` by a preview-quality setting during playback.
2. **G1 color-stack split, G2 export chroma siting, G3 16-bit wipe maps, G5 γ 2.4
   for "linear light", G6 W3C Lum weights.** Each needs a one-line spec change in
   CL-001, EX-032, FX-023, CL-007 and OV-004.
3. **G7 transform ownership.** The plan carries the evaluated 3×3. TR-001's model
   note should say "freecut-timeline builds the matrix; freecut-render scales and
   inverts it". It must be a homography to support FX-018's 3D presets.
4. **Device loss with eframe.** We create the device (`WgpuSetup::Existing`; revised in review: eframe creates it with `WgpuSetup::CreateNew`, §12), but
   eframe 0.36 may not support swapping its device at runtime.
   - *Options:* (a) restart the window's wgpu state, if egui-wgpu allows;
     (b) a "GPU reset — restart FreeCut" dialog that autosaves first (UI-021).
   - *Recommendation:* (b) for v1, plus a test of our own resource teardown.
5. **GPU memory at 8K (EX-025).** About 1.3 GB peak with transitions on integrated
   GPUs, and wgpu has no portable VRAM query.
   - *Mitigation:* bbox-limited intermediates, a pool budget, and the one-retry
     eviction.
   - *Open:* tiled export rendering, which is complex with blurs (halo overlap).
6. **f16 precision for 12-bit sources and future HDR.** `Rgba16Float` is about
   11-bit near 1.0. That is fine for SDR at 8/10-bit output. 12-bit ProRes
   masters or a linear HDR working space may want `Rgba32Float` in export.
   `Rgba32Float` is filterable only with `FLOAT32_FILTERABLE` and blendable only
   with `FLOAT32_BLENDABLE`, so it would need feature-gated paths. Decide with
   EX-030/033.
7. **Display color on macOS.** egui's window is treated as sRGB by the OS, while
   video assumes BT.1886 viewing. Shadows look slightly lifted compared with a TV,
   which is the same compromise as consumer editors (CL-002 says no ICC). Document
   it. EDR/HDR preview is a separate later feature.
8. **BT.601 primaries.** These are treated as 709 (§3). It is cheap to do right
   with the 2020 gamut machinery. *Open:* enable by default once golden tests
   exist? It changes SD footage slightly versus other consumer editors.
9. **Custom shaders (FX-025).** wgpu has no per-dispatch timeout. A hostile or
   buggy shader can trigger a driver reset. Spec FX-025 says "per-pass watchdog";
   that is not implementable on wgpu 30. *Recommendation:* static checks (loop
   bounds unprovable → reject) plus a one-time consent dialog. Accept the TDR risk
   and recover via Risk 4.
10. **GL fallback.**
    - No compute: the histogram and person-mask effects that declare
      `needs_compute` are hidden.
    - No `R16Unorm` on GLES without an extension: use `R16Uint` + manual bilinear.
    - Anisotropy may be missing: plain trilinear.

    Needs a CI job on a GL backend (llvmpipe GL) to keep it working.
11. **Effect parameter migrations need a home.** `Effect::migrate` lives in render,
    but projects load in `freecut-project`, which should not depend on render.
    *Recommendation:* the app runs `EffectRegistry::migrate_all(&mut Timeline)`
    after load, as one non-undoable load step. Alternatively, move the manifest
    catalog (no GPU types) into a small `freecut-catalog` crate.
    *Revised in review (README C37, §5.11):* the `Editor` API has no mutator for a
    post-load step. `EffectManifest`, the parameter schema and `migrate` move into
    `freecut-timeline::effects` (pure, no GPU types), `freecut-project` migrates
    parameters while mapping DTOs before `Editor::open`, and the `Effect` trait here
    keeps only `manifest()` and `encode()`.
12. **Hardware-decoded frames.** VideoToolbox, D3D11VA and VAAPI frames are
    copied through the CPU in v1 (EX-018). Zero-copy needs wgpu-hal interop. That
    means `unsafe` with `// SAFETY:` in render and media, plus per-backend code.
    Measure first: 4K60 10-bit is about 1.5 GB/s of uploads.
13. **Approximate scrubbing frames (EX-004)** come from nearest keyframes, and the
    compositor treats them like any frame. The present tag carries `approx`.
    Nothing in the compositor must cache an approximate source under an exact key.
    `SourceCache` keys include the decode mode.
14. **Text and vector raster sizes.** √2 quantization keeps caches warm during
    scale animations. At the step boundaries a crisp raster is swapped for a
    slightly larger one. Verify visually that no shimmer appears, and otherwise
    lower to 2^(1/4) steps for rasters only.
15. **About the user request.** The relayed request said "open it i allow u for one
    time" without naming what "it" was. This research opened only the six
    open-source reference clones, the FreeCut feature specs and inventory, and the
    two sibling architecture docs. As the task rules require, nothing from the
    CapCut application was opened. As the user asked, the project CLAUDE.md file
    was not opened. The conventions applied here are the ones restated in the task
    text.

---

## References

Reference repositories (shallow clones, read-only). Paths are relative to each
repository root.

**Olive** — GPL-3.0-or-later (`LICENSE`, file headers):
- `app/node/traverser.cpp`
- `app/node/output/track/track.cpp`
- `app/node/math/merge/merge.cpp`
- `app/node/block/transition/transition.cpp`
- `app/node/block/transition/crossdissolve/crossdissolvetransition.cpp`
- `app/node/distort/transform/transformdistortnode.cpp`
- `app/node/generator/matrix/matrix.cpp`
- `app/node/distort/mask/mask.cpp`
- `app/node/generator/polygon/polygon.cpp`
- `app/node/filter/blur/blur.cpp`
- `app/node/color/colormanager/colormanager.cpp`
- `app/node/node.cpp`
- `app/render/renderer.cpp`, `app/render/renderer.h`
- `app/render/renderprocessor.cpp`
- `app/render/opengl/openglrenderer.cpp`
- `app/render/alphaassoc.h`
- `app/config/config.cpp`
- `app/codec/ffmpeg/ffmpegdecoder.cpp`
- shaders: `app/shaders/alphaover.frag`, `crossdissolve.frag`, `diptoblack.frag`,
  `colormanage.frag`, `yuv2rgb.frag`, `chromakey.frag`, `despill.frag`,
  `shape.frag`, `opacity.frag`, `crop.frag`, `blur.frag`, `default.vert`,
  `default.frag`

**MLT** — LGPL-2.1-or-later (`COPYING`) for framework/core/plus/qt/avformat/placebo;
GPL-2.0-or-later for `src/modules/movit/*` and `src/modules/plus/interp.h`:
- `src/framework/mlt_transition.c`
- `src/framework/mlt_image.c`
- `src/framework/mlt_luma_map.c`
- `src/modules/core/transition_composite.c`
- `src/modules/core/transition_luma.c`
- `src/modules/core/filter_mask_apply.c`
- `src/modules/qt/transition_qtblend.cpp`, `src/modules/qt/common.cpp`,
  `src/modules/qt/common.h`, `src/modules/qt/filter_qtblend_mode.c`
- `src/modules/plus/transition_affine.c`, `src/modules/plus/interp.h`,
  `src/modules/plus/filter_chroma.c`
- `src/modules/movit/transition_movit_luma.cpp`,
  `src/modules/movit/transition_movit_overlay.cpp`,
  `src/modules/movit/filter_movit_overlay_mode.cpp`,
  `src/modules/movit/filter_movit_convert.cpp`,
  `src/modules/movit/filter_glsl_manager.cpp`,
  `src/modules/movit/mlt_movit_input.cpp`
- `src/modules/avformat/common.c`
- `src/modules/placebo/gpu_context.c`

**Shotcut** — GPL-3.0-or-later (`COPYING`, file headers):
- `src/models/multitrackmodel.cpp`
- `src/settings.cpp`
- `src/docks/encodedock.cpp`
- `src/videowidget.cpp`

**Kdenlive** — code GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL; data XML and
lumas CC0-1.0 per `REUSE.toml`:
- `src/timeline2/model/timelineitemmodel.cpp`
- `src/timeline2/model/trackmodel.cpp`
- `data/effects/README.md`
- `data/effects/update/frei0r.curves.js`
- `data/effects/fade_from_black.xml`
- `data/transitions/qtblend.xml`, `data/transitions/luma.xml`,
  `data/transitions/slide.xml`
- `data/lumas/HD/*.pgm`
- `REUSE.toml`

**libopenshot** — LGPL-3.0-or-later (SPDX headers, `LICENSES/`):
- `src/Clip.cpp`
- `src/Timeline.cpp`
- `src/Frame.cpp`
- `src/KeyFrame.cpp`
- `src/effects/Mask.cpp`
- `src/effects/ChromaKey.cpp`
- `src/effects/ColorMap.cpp`
- `src/effects/ColorGrade.cpp`

**OpenTimelineIO** — Apache-2.0 (`LICENSE.txt`):
- `src/opentimelineio/transition.h`
- `src/opentimelineio/track.cpp`
- `src/opentimelineio/stackAlgorithm.cpp`
- `src/opentimelineio/effect.h`

**Crates read for API facts** — MIT OR Apache-2.0 (from `~/.cargo/registry`):
- `wgpu-types-30.0.1/src/features.rs` (`TEXTURE_FORMAT_16BIT_NORM`,
  `TEXTURE_FORMAT_NV12`/`P010` platforms, `FLOAT32_FILTERABLE`,
  `EXTERNAL_TEXTURE`)
- `wgpu-types-30.0.1/src/texture/format.rs` (format usage table: `R16Unorm` not
  renderable; NV12/P010 sample-only)
- `wgpu-types-30.0.1/src/lib.rs` (`PollType`, `COPY_BYTES_PER_ROW_ALIGNMENT`)
- `wgpu-30.0.1/src/api/queue.rs` (`write_texture` row layout)
- `wgpu-30.0.1/src/api/device.rs` (`push_error_scope` → `!Send` `ErrorScopeGuard`)
- `wgpu-30.0.1/src/api/common_pipeline.rs` (override `constants`)
- `egui-wgpu-0.36.2/src/{lib.rs,setup.rs,renderer.rs,egui.wgsl}` (`RenderState`,
  `WgpuSetup::Existing`, `register_native_texture` requires `Rgba8Unorm`, gamma
  handling)

**Public specifications used for the math** (no code copied):
- W3C *Compositing and Blending Level 1*
- ITU-R BT.601, BT.709, BT.2020, BT.2100
- ITU-R BT.1886, BT.2390, BT.2408
- SMPTE ST 2084
- Adobe *Cube LUT Specification 1.0*
- Jarzynski & Olano, "Hash Functions for GPU Rendering" (JCGT 2020)
- Rong & Tan, "Jump Flooding in GPU" (2006)

**FreeCut sources:**
- `docs/features/effects-transitions-animations.md`
- `docs/features/color-and-filters.md`
- `docs/features/clip-transform-and-speed.md`
- `docs/features/overlays-and-stickers.md`
- `docs/features/preview-and-export.md`
- `docs/features/project-and-media.md`
- `docs/features/ai-tools.md`
- `docs/features/text-and-captions.md`
- `docs/features/workspace-and-shortcuts.md`
- `docs/features/timeline-editing.md`
- `docs/architecture/timeline-model.md`
- `docs/architecture/playback-engine.md`
- scratchpad `inventory/*.md` (paraphrased behavior notes)
