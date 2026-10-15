# Text, titles & captions

> Status: draft · Owns: text clip payload (rich text, `TextStyle`, text layout), font registry and fallback rules, font collections, which properties and behaviours each text lens holds (TX-054; UI-008 and the UI concept own the frame and its presentation), bubble and tag boxes around text (TX-055), text-specific animation presets and their motion catalogue (In / Out / Loop and per-unit reveals, TX-056), text style presets and text templates, caption track payload (segments, track style, secondary line, word timings and word states, caption line rules, caption looks, progressive caption display, caption emoji), subtitle file formats (import/export), and how recognition, translation, speech, dictation and lyric results land in text and on caption tracks (the jobs, models and dialogs themselves belong to [ai-tools.md](ai-tools.md)) · Crates: freecut-timeline, freecut-render, freecut-project, freecut-media, freecut-app (TX has no ML dependency; all inference runs in `freecut-analysis`, defined in [ai-tools.md](ai-tools.md))

## Summary

FreeCut users put words on screen as free-floating **text clips** (titles, labels, callouts), as **captions** (time-coded segments on a dedicated caption track that share one track style), or as **text templates** (ready-made groups of styled, animated text layers). Text can be fully styled (font, size, solid or gradient fill, outline, shadow, glow, background box, spacing, alignment, emphasis, curve) and animated with in, out and loop presets, including per-character and per-word reveals. Captions can be typed by hand, built from a pasted script, imported from or exported to SRT, WebVTT, ASS, LRC and TXT, shown with a second-language line, given an animated caption look with word-by-word display, and burned into or muxed alongside the exported video. What the reference editor does through cloud services (speech-to-text, translation, text-to-speech, lyric recognition, dictation, emoji suggestions) is done in FreeCut by the local smart tools of [ai-tools.md](ai-tools.md). This spec defines only how their results become text, caption tracks, segments and links.

## Features

### TX-001 Add a text clip — mvp
**What the user sees:** The Text panel in the left browser area has an **Add text** tile, rendered in the current default text style. The panel's other sections are **My looks** (TX-020), **Looks** (TX-019), **Templates** (TX-024) and, later, **Caption looks** (TX-051), listed in the Text Drawer below. Each uses UI-006's library frame (search, category chips, favorites, recent). Clicking the **Add text** tile adds a text clip at the playhead. Dragging the tile drops a clip at a chosen track and time. The new clip shows the placeholder text **"Your text"** (FreeCut default), centered on the canvas. It is selected, the inspector switches to its Text tab, and on-canvas editing starts with all the placeholder text selected, so typing replaces it.
- *Text Drawer contents (v1):* in the Shelf and Drawer of the UI concept ([ui-concept §8](../design/ui-concept.md#8-shelf-and-drawer-the-asset-browser), which owns the presentation), the Text Drawer lists, in this order: **Add text**; **My looks**, the user's saved looks (TX-020), first so personal presets are one click away; **Looks**, FreeCut's style presets (TX-019); **Templates** (TX-024); later **Caption looks** (TX-051); and **Captions**, with entry tiles for a subtitle file (TX-031) and, later, captions from speech (TX-038; the tile shows that the speech model is needed until AI-002 has installed it). The caption tiles open the Captions Drawer with that action started. **Long text…** (TX-030) sits next to Add text.
- In the Drawer a click on a **My looks** or **Looks** tile always *adds* a new text clip at the playhead in that look, whatever is selected (the UI concept's rule "the Shelf is for adding, the Inspector is for changing", [ui-concept §4](../design/ui-concept.md#4-navigation-model)), so every look also works as an *add text* button (TX-020). To *apply* a look to selected text clips, the user clicks it in the Presets lens (TX-054), drops the tile onto a text clip on the Stage or the timeline, or picks **Apply to selection** in the tile's context menu.
**Inputs / controls:**
- Insert position: the playhead (click), or the drop point (drag).
- Duration: 3 s (FreeCut default). A preference allows 0.5–60 s. The value is rounded to whole frames of the project frame rate.
- Target track: text clips live on `Text` tracks (TL-004 in [timeline-editing.md](timeline-editing.md)). TL's placement rules pick an unlocked `Text` track with a free gap for the whole duration, or create a new one (TL-006).
- Style: the user's default text style (TX-020, from v1). Without one, and always in the mvp, the factory default applies: the bundled sans family (Noto Sans, OFL-1.1) in Bold, size 64 ref px, white `#FFFFFF` fill, center alignment, auto-width box, and no stroke, shadow, glow or box.
**Behavior / output:** A `TextClip` is created and rendered in preview and export like any other visual clip. The clip's transform (position, scale, rotation, opacity) comes from TR. Its default position is the canvas center.
**Edge cases:**
- Playhead at or past the timeline end: the clip is placed at the playhead, which extends the timeline.
- Every `Text` track is locked, or none has room: a new `Text` track is created (TL-006).
- Track limit reached (TL-006): the insert is refused with TL's toast, and nothing changes.
- Inserting during playback: playback pauses, the clip is inserted at the paused time, and edit mode starts.
- Empty project (no media yet): this is allowed. The text clip alone defines the timeline length. The reference editor refuses to add text while its main track is empty. FreeCut allows it, because a title card on its own is a valid way to start.
- Portrait vs landscape canvas: sizes are in ref px, which scale with the canvas's short edge (see the conventions below), so the clip looks the same relative to the frame.
- Dropping onto an occupied range: the overwrite/insert rules from TL apply. Text clips never overlap other clips on the same track.
**Undo:** TL's `InsertClips { clips, ripple_changes, created_tracks }` (TL-012) with one text clip. Undo removes the clip, and also removes any track the op created.
**Model notes:**
- *Conventions for every TX feature:*
  - Area prefixes (TL, TR, FX, OV, AU, CL, AI, EX, PM, UI) refer to the specs listed under [Interactions with other areas](#interactions-with-other-areas); for example TL-006 is a feature in [timeline-editing.md](timeline-editing.md).
  - Time is the rational type from TL (TL-001). Defaults given in seconds are converted on input.
  - Lengths in a text style are in **ref px**: pixels on a reference canvas whose short edge is 1080 px. At render time they are multiplied by `canvas_short_edge / 1080`. This is the only text-scaling rule in FreeCut. When the canvas ratio or resolution changes, PM-011 ([project-and-media.md](project-and-media.md)) rescales text by it, not by canvas height. On a portrait canvas the short edge is the width.
  - Only time has to be rational (TL-001, project CLAUDE.md). Each spec picks the storage type for its own non-time scalars, a rule recorded once in TR's shared conventions ([clip-transform-and-speed.md](clip-transform-and-speed.md)). TX stores style scalars as fixed-point integers (lengths in 1/100 ref px, percentages in per-mille, angles in 1/10 degree), for exact equality in tests and stable project files. When a text property is keyframed (TX-021), it is wrapped in TR-027's generic `Animatable<T: Interpolate>` with these fixed-point types as `T`. Floats appear only at the UI and render edge.
  - Colors are sRGB `Rgba8` with straight alpha.
- In `freecut-timeline`: `ClipKind::Text(TextClipData { content: RichText, style: TextStyle, layout: TextLayout, origin: TextOrigin })`. Animations are stored in FX's per-clip `ClipAnimations` (TX-022), and the transform in TR's per-clip transform. `TextOrigin` is one of `Manual | Template{id, layer} | ConvertedFromCaption`.
- Layout and rasterization are in `freecut-render::text`. The panel and tile are in `freecut-app`. Persistence is in `freecut-project`.
- *FreeCut mvp model (2026-10-12):* `ClipContent::Text(Arc<TextClipData>)` in `freecut-timeline`. `RichText` is a newtype over the plain `String` until spans arrive (TX-015). `TextStyle { font: FontRef { family }, size: RefPx, fill: Rgba8, bold, italic }`, where `RefPx(i32)` counts 1/100 ref px (64 ref px = `6400`); the other `FontRef` fields and the `Fill` enum come with TX-003's face picker and TX-014. `TextOrigin` has only `Manual` so far. The default length is `DEFAULT_TEXT_DURATION` (3 s), rounded to the nearest frame on insert (72 frames at 23.976 fps). `InsertTarget::FirstFree` puts a text clip on the lowest unlocked `Text` track free over its span, else on a new `Text` track at the top of the upper band. The model accepts any content length; the 10,000-character limit (TX-002) is enforced by the editing UI only.
- *FreeCut mvp UI (2026-10-12):* the asset panel's **Text** tab (UI-004) holds one **Add text** tile. The tile shows "Your text" in white on a dark card, drawn as a bold egui label rather than a raster of the default style; the clip itself always renders through `freecut_render`. A click pauses playback, inserts `NewClip::text(TextClipData::default(), default_text_duration(rate))` at the playhead with `InsertTarget::FirstFree`, selects it, and focuses the inspector's content field with all the text selected (TX-002's interim edit mode). Dragging the tile onto the timeline inserts at the drop time on the track under the pointer (`InsertTarget::Track`; TL's rules move it to the nearest Text track or create one), or `FirstFree` below the last track. The panel's Styles and Templates tabs and UI-006's library frame are not built yet.

### TX-002 Edit text content — mvp
**What the user sees:**
- Double-clicking a text clip in the preview, or pressing Enter while one is selected, enters on-canvas edit mode, with a caret, a selection highlight and a dashed box. The inspector also has a multi-line text field that mirrors the content.
- Standard editing keys work: arrow keys, Home/End, word jumps, select all, word delete, cut/copy/paste. The OS input method works for CJK composition, and the OS emoji picker works.
- In edit mode, **Enter** inserts a line break, and **Esc**, **Ctrl/Cmd+Enter**, or a click outside the text commits the edit.
- Double-clicking a text clip on the timeline moves the playhead into the clip (to its start if the playhead is outside it) and enters edit mode.
**Inputs / controls:** Plain UTF-8 text. Pasted rich text is reduced to plain text. CRLF becomes LF, and tabs become 4 spaces (FreeCut default). There is a soft limit of 10,000 characters per clip (FreeCut default).
**Behavior / output:** Layout updates live on every keystroke. Clip duration and transform do not change.
**Edge cases:**
- Empty content: this is allowed. The clip renders nothing. While selected, it shows a dashed placeholder box in the preview, and the timeline labels it "(empty text)".
- Reaching the character limit: further input is refused, with a notice. Paste is truncated, with a notice.
- Several text clips selected: the content field is hidden and on-canvas editing is unavailable. Style controls still apply to all of them (UI-010).
- Fields that require content (a style name, a template name, the TX-030 text box) disable their confirm button while empty. The content of a text clip itself may be empty.
- Clip on a locked track: edit mode is refused, with a "track is locked" hint.
- Fully transparent or off-canvas text: the inspector field still edits it.
- Editing when per-range styles exist (TX-015): a typed character takes the style of the character before the caret, or the character after it when the caret is at position 0.
- Display case transform active (TX-008): edit mode temporarily shows the text as typed, so caret positions map one to one.
- Undo pressed inside edit mode: this first walks the field's local undo history. Only after the session is committed does it go to the project undo stack.
**Undo:** `SetTextContent { clip, before: RichText, after: RichText }`. One entry per edit session, created on commit. A session with no net change creates no entry.
**FreeCut mvp request (2026-10-12):** `EditRequest::SetTextContent { seq, clip, content }` holds the whole new content; refused for a non-text clip or a locked track, and a no-op when the content is unchanged.
**FreeCut mvp interim default (2026-10-12):** on-canvas edit mode (caret, selection highlight, dashed box) is not built yet. Its place is taken by the inspector's multi-line content field: a double-click on a text clip in the player or on the timeline (which also moves the playhead into the clip), and the Text tab's add, focus that field with all its text selected. Typing edits a session buffer in `freecut-app` (`ui::props::ContentSession`); the preview shows the draft live through a transient override in the preview request, not through the document. The session commits as one `SetTextContent`, or nothing when there is no net change, when the field loses focus (a click elsewhere, Tab), on Esc or Ctrl/Cmd+Enter, or when the selection changes. Enter inserts a line break. Commit normalizes CRLF and lone CR to LF and tabs to 4 spaces. The field refuses input past 10,000 characters and says so; a paste is truncated. Undo inside the field walks the field's own history; the project undo stack is untouched until commit. On a locked track the field is read-only. Enter on exactly one selected text clip does what a double-click does: it focuses the content field with all text selected and brings the playhead into the clip (built 2026-10-14; the recorder panel keeps Enter while it is open). Not built yet: the dashed placeholder box for empty text (an empty clip can't be picked on the canvas yet), and IME-specific handling beyond what egui provides.
**Model notes:** Content is stored exactly as typed, with no Unicode normalization. Search (TX-034) normalizes on the fly. Span ranges are byte offsets snapped to grapheme-cluster boundaries and are adjusted on every insert or delete. The edit-session buffer lives in `freecut-app`, and the commit produces one timeline op.
- App note (FreeCut defaults, `freecut-app`, 2026-10-12): an edit session never loses its typing. A commit that would land while another gesture is open (a press-drag on the canvas keeps the field focused, and may select another clip) waits: the draft stays in the preview and commits on the first frame after the drag ends. Closing the window, ⌘Q, New or opening another project (menu or dropped file) commit the session first, so the usual "unsaved changes" question covers it; a draft that still waits for a drag counts as unsaved.

### TX-003 Font picker (bundled + system fonts) — mvp
**What the user sees:** A font dropdown with a search box. Each family name is drawn in its own face, rendered lazily as the list scrolls. The list has sections for **Recent** (last 10), **Bundled**, **Imported** (TX-018, from v1) and **System**. A second dropdown lists the family's faces, such as Light, Regular, Bold and Black Italic. Hovering a family previews it on the selected clip. A favorites star arrives in v1 (see Edge cases). From v1, **Category** chips filter the list: Sans, Serif, Mono, Display, Handwriting and Symbol. Categories come from each font's metadata (OS/2 family class, PANOSE, the monospace flag), with a guess from the name as a fallback (FreeCut default). User-made collections are TX-050 (later).
**Inputs / controls:** Family name and face. A face is a weight from 100 to 900, italic on or off, and a stretch value. Default: Noto Sans Bold.
**Behavior / output:**
- System fonts are discovered on a background thread at startup. Until the scan finishes, only bundled fonts are listed, with a spinner.
- Choosing a family applies it to the whole clip, or to the selected range while in edit mode (TX-015, v1).
- Hover previews are transient and create no undo entry. Leaving the list restores the previous font.
**Edge cases:**
- Thousands of installed fonts: the list is virtualized, and only visible names are shaped.
- Font files that fail to parse are skipped and logged, never shown.
- Symbol-only fonts with no Latin glyphs: the name is drawn in the UI font, followed by a sample of the font's own glyphs.
- Variable fonts: named instances are listed as faces. Free axis values come later.
- The same family is found in two sources: imported wins over system, and system wins over bundled. The inspector's info popover shows which file is in use.
- Color and emoji fonts (COLR, CBDT, sbix) are listed. They are mainly used through fallback (TX-006).
- Favorites (a star toggle stored in app preferences) ship in v1. The rest of this feature is mvp. Favorites appear as their own section above Recent. They have no fixed cap and never need an account (FreeCut default). The reference editor caps favorites and ties them to sign-in.
- The Imported section with no imported fonts shows an **Add font file…** button (TX-018) instead of an empty list.
**Undo:** `SetTextStyle { clip, before: TextStylePatch, after: TextStylePatch }`. One entry per font selection.
**FreeCut mvp UI (2026-10-12):** the inspector's Text section has a family dropdown with a search box: Noto Sans (bundled) first, then every family of `FontRegistry::family_names()` (sorted, hidden families left out). A spinner shows while the system scan runs; the list fills in when it finishes, and the preview re-renders because the registry generation is part of its request key. A family that isn't installed shows as "Name (missing)". Names are drawn in the UI font. Sections, per-family previews, the face dropdown and hover previews are not built yet. Picking a family is one `SetTextStyle` entry for every selected text clip.
**Model notes:** In timeline: `FontRef { family: String, weight: u16, italic: bool, stretch: u16, postscript_name: Option<String> }`. The PostScript name is a hint used to re-find the exact face. `freecut-render::text::FontRegistry` owns discovery, using the system font database, and loading, which never runs on the UI thread. The app reads a snapshot of the registry.

### TX-004 Size, fill color, bold and italic — mvp
**What the user sees:** In the inspector Text tab:
- A **Size** slider plus a numeric field.
- A **Color** swatch that opens a picker with an HSV square, hex entry, alpha and recent swatches. A preview eyedropper is added in v1.
- **B** and **I** toggle buttons.
**Inputs / controls:**
- Size: 4–1000 ref px, default 64. Typed values are clamped to this range.
- Fill: `Rgba8`, default opaque white.
- Bold toggle: picks the family's nearest face with weight ≥ 600. If the family has none, bold is synthesized by emboldening the outline by about 2% of the em.
- Italic toggle: picks an italic or oblique face. If the family has none, a synthetic slant of 12° is applied.
**Behavior / output:** Size changes the layout, so wrapping and box size update. TR's scale does not change the layout. Glyphs are rasterized at the final output resolution, both for size and for TR scale, so text never looks upscaled from a bitmap.
**Edge cases:**
- Fill alpha 0 with an outline (TX-010) gives hollow, outline-only text. This is a valid look.
- Bold is never synthesized when a real bold face exists. The UI marks synthetic faces with "(synthetic)".
- Huge sizes overflow the canvas and are clipped by the canvas at render time, with no error.
- Text colors are sRGB and are converted into the working color space as defined in [color-and-filters.md](color-and-filters.md).
- The color swatch opens UI-009's shared color field (hex, alpha, recent colors, and from v1 an eyedropper). The reference editor also keeps a saved personal palette next to fixed built-in colors (adding a duplicate is refused, built-ins cannot be deleted). Such a palette would belong to UI-009's shared picker, which also receives OV-043's brand colors. TX does not define its own.
- Overall clip opacity is TR's opacity, not a text property. Fill alpha and clip opacity multiply together.
**Undo:** `SetTextStyle` patch. Slider drags coalesce into one entry when the mouse is released.
**FreeCut mvp request (2026-10-12):** `EditRequest::SetTextStyle { seq, clips, edit: TextStyleEdit }` sets one value (`Font`, `Size`, `Fill`, `Bold` or `Italic`) on every listed text clip, the role `TextStylePatch` will take over. Non-text clips are skipped, an all-non-text selection is refused, any listed clip on a locked track refuses the edit, sizes are clamped to 4–1000 ref px, and unchanged clips record nothing.
**FreeCut mvp UI (2026-10-12):** Size is a logarithmic slider with a typed value (ref px, 4–1000); a slider drag is one undo entry. Color is egui's colour button with RGBA picker; changes while the pointer is held in the picker merge into one entry (`DragGesture::coalesce_held`). **B** and **I** are toggle buttons. Every control applies to all selected text clips; the values shown are the first one's.
**Model notes:** `TextStyle { font: FontRef, size: Fixed, fill: Fill, bold: bool, italic: bool, decoration, case, letter_spacing, line_spacing, stroke, shadow, glow, background, curve }`. `Fill` is `Solid(Rgba8)`, `LinearGradient(..)` or `RadialGradient(..)` from TX-014, `Image(..)` from TX-046, or `WordCycle(..)` from TX-047. `TextStylePatch` mirrors the struct with `Option` fields and is used by ops, presets and overrides.

### TX-005 Paragraph layout: alignment, line breaks and wrapping — mvp
**What the user sees:** Alignment buttons for left, center, right and justify. A **Box** toggle switches between **Auto width** and **Fixed width**. In fixed-width mode the preview shows side handles for dragging the width, and the text re-wraps live while dragging. Dragging a side handle of an auto-width box switches it to Fixed width at the dragged width. Holding Alt while dragging resizes the box symmetrically around its center; otherwise the opposite edge stays put. Corner handles are TR's scale (TR-007), not layout. A vertical anchor control offers top, middle and bottom.
**Inputs / controls:**
- Alignment: default center. Justify leaves the last line start-aligned.
- Box mode: default Auto width, where lines break only at manual line breaks. Fixed width wraps at a width of 20–10,000 ref px.
- Vertical anchor: default middle. It sets which edge stays put as lines are added or removed.
**Behavior / output:**
- Line breaking follows the Unicode line-breaking rules. CJK text may break between ideographs.
- A single word wider than the box breaks mid-word as a last resort.
- The text box's anchor is the TR transform anchor, which defaults to the center.
- Layout results (line boxes and the bounding box) are exposed to the app for selection handles and snapping.
**Edge cases:**
- Fixed width narrower than one glyph: the width is clamped to at least 1 em, so each glyph sits on its own line.
- RTL paragraphs: alignment is visual (left means left). Paragraph direction is detected automatically from the content.
- Trailing spaces are ignored for alignment. Empty lines are kept and count toward the height.
- Switching from Fixed to Auto width keeps only manual breaks.
- Curved text (TX-016) forces auto width while the curve is non-zero. The fixed width is remembered and restored when the curve returns to 0.
- A text clip attached to a motion track (AI-008): resizing the box keeps the tracking binding. A one-sided resize adjusts the binding's offset so the fixed edge stays put. The reference editor drops tracking when a tracked text box is resized; FreeCut does not.
**Undo:** `SetTextLayout { clip, before, after }`. Dragging a width handle coalesces into one entry. When the drag also changes the anchor position or a tracking offset, those changes are part of the same entry.
**FreeCut mvp model (2026-10-12):** `TextLayout { align: HAlign::{Left, Center, Right}, box_width: Option<RefPx> /* None = auto width */, line_spacing: PerMille /* TX-009, 500–3000, default 1000 */ }`. Justify and the vertical anchor come later. `EditRequest::SetTextLayout { seq, clips, edit: TextLayoutEdit::{Align, BoxWidth, LineSpacing} }` follows the `SetTextStyle` rules; a fixed width is clamped to 20–10,000 ref px. The renderer lays text out in canvas px (ref px × `canvas_short_edge / 1080`) with the box centre as the layer origin; the clip transform (TR-001) is applied after layout, see `freecut_timeline::plan`.
**FreeCut mvp UI (2026-10-12):** Align is three buttons (Left, Center, Right). Box is a **Fixed width** checkbox plus a width field in ref px; switching it on starts from the box width measured in the latest preview frame, or 600 ref px when the clip isn't on screen. Line spacing is a percentage field (50–300 %). Field drags are one undo entry. The preview thread reports each text box's laid-out size with every frame (`preview::LayerBox`), so selection outlines and hit-testing use the renderer's own layout, never one made on the UI thread. Side handles for the width are not built yet.
**Model notes:** `TextLayout { align: HAlign, box_mode: BoxMode::{Auto, FixedWidth(Fixed)}, v_anchor: VAnchor }`. `freecut-render::text::measure(content, style, layout, canvas) -> TextMetrics` is pure given a font snapshot, is cached per clip revision, and runs off the UI thread. The app uses the last result.

### TX-006 Unicode, complex scripts and font fallback — mvp
**What the user sees:** Text in any script renders correctly:
- Arabic and Hebrew join and run right to left.
- Indic conjuncts form, and Thai, CJK and emoji (including ZWJ sequences, skin tones and flags) display.
- If the chosen font lacks some characters, they still appear, drawn from a fallback font. The inspector shows a small "fallback used" hint that lists the affected characters.
**Inputs / controls:**
- An optional per-clip **Language** tag (BCP 47). The default is **Auto**, taken from the OS locale. The tag selects locale-specific glyph forms (for example, Japanese vs Simplified Chinese Han forms) and case mapping (for example, Turkish dotted i).
- A preference selects the emoji font: bundled color emoji (default) or a system emoji font (later).
**Behavior / output:**
- Shaping uses a HarfBuzz-compatible shaper, and bidi uses the Unicode bidirectional algorithm.
- Fallback is resolved per character, in this order: selected font → bundled Noto font for that script → system fallback fonts → a visible "missing glyph" box.
- Caret movement and selection step by grapheme cluster.
**Edge cases:**
- A style span boundary inside a grapheme cluster (for example, between a base letter and its combining mark) is snapped outward to the cluster boundary.
- Mixed LTR and RTL text with numbers is ordered per the bidi algorithm. Selection may appear split visually, which is expected.
- Fallback fonts have different vertical metrics: each line's height uses the tallest font on that line, so glyphs never overlap the line above.
- Stroke and shadow on joined scripts: the outline is computed from the union of the run's glyph shapes, so joins show no seams. This avoids glyph parts vanishing at high stroke widths, a failure mode reported for another editor.
- Bitmap color emoji with stroke, shadow or glow: the effect uses the emoji's alpha silhouette.
- Bundled CJK coverage is large (about 15–20 MB per region). The mvp ships without it and relies on system CJK fonts. An optional Noto CJK font pack (OFL-1.1) is offered as a separate download in v1. Whether to bundle it by default is still an open decision.
- No font on the system covers a character: the "missing glyph" box is drawn, and the "fallback used" hint names the character.
- Emoji render from the bundled color emoji font on every supported OS, so they do not depend on the OS emoji font. The reference editor needs a recent Windows release before emoji appear at all.
**Undo:** n/a (rendering behavior). Changing the language tag is a `SetTextStyle` patch.
**Model notes:** Content is UTF-8, and spans use grapheme-aligned byte ranges. All shaping, bidi and fallback live in `freecut-render::text`, built on an open Rust text stack (see Clean-room notes). The model stores only `FontRef` and the language tag, never resolved fallback fonts.

### TX-007 Missing fonts when opening a project — v1
**What the user sees:** Until this feature ships, a missing font renders through the TX-006 fallback chain, and the stored `FontRef` is kept, so nothing is lost on save. When a project is opened, or text is pasted or a template inserted, a non-blocking banner lists any fonts that are not available. Each missing font offers these actions:
- **Replace…** opens the font picker. The replacement applies to every clip that uses that font.
- **Locate file…** imports the font file (TX-018, v1).
- **Keep** renders with a substitute and remembers the original.

Affected clips show a warning badge on the timeline and in the inspector. The export dialog repeats the warning. The user can still export.
**Inputs / controls:** Choice of replacement font. **Rescan fonts** command, under Help or the font menu.
**Behavior / output:**
- The stored `FontRef` is never overwritten automatically. For rendering, the closest available face is substituted, matched by generic class (serif, sans, mono, script, guessed from the name and font metadata) plus weight and italic.
- Common cross-OS aliases map to metric-compatible open fonts: Arial/Helvetica → Liberation Sans, Times New Roman → Liberation Serif, Courier New → Liberation Mono.
- A substituted clip is marked as substituted in the render state, not in the model.
**Edge cases:**
- Font present but a different version (fingerprint mismatch): there is no banner. The info popover notes the version difference.
- The user installs the font while the project is open: **Rescan fonts** re-resolves it, and the badges clear.
- Templates or presets that reference a missing font go through the same flow.
- Bundled fonts updated in a newer FreeCut release may reflow slightly. This is documented in release notes, and FreeCut does not keep old font versions.
- Missing font plus per-range overrides: each span is resolved independently.
- The banner names each missing font, says how many clips use it, and states that a substitute is rendered until it is resolved.
- An imported font whose file was deleted from FreeCut's font folder outside FreeCut: the Imported list shows it as **file missing** (TX-018), and the clips that use it go through this flow.
- A font file FreeCut cannot use (for example a Type 1 or bitmap-only font found by the system scan) is never offered in the picker and is logged as unsupported. A project that names such a font treats it as missing and goes through this flow. Imported files of that kind are rejected at import (TX-018).
- FreeCut never downloads fonts, so there is no download or retry state.
**Undo:** `ReplaceFontEverywhere { from: FontRef, to: FontRef, touched: Vec<(ItemId, TextStylePatch before/after)> }`. Undo restores every clip and caption track touched. "Keep" is view state (n/a).
**Model notes:** Resolution status is computed by `FontRegistry::resolve(&FontRef) -> Resolved | Substituted(face) | Missing`. Collecting imported fonts into the project's `Assets/` folder belongs to [project-and-media.md](project-and-media.md) (PM-101, using PM-042's asset index). It copies only fonts whose embedding permissions allow it. Restricted fonts are skipped, with the reason shown.

### TX-008 Underline, strikethrough and letter case — v1
**What the user sees:** **U** and **S** toggle buttons, and a **Case** dropdown with As typed, UPPERCASE, lowercase, Title Case and Sentence case. In edit mode with a range selected, the same four target cases are also offered as **Change case** commands (context menu and inspector), which rewrite the selected characters themselves.
- *Case control (v1):* in the Style lens's Type card (TX-054; presentation in ui-concept §7.1) the case is a segmented control of four one-click choices: As typed, UPPERCASE, lowercase and Title Case. Sentence case is a fifth choice behind the card's **More** rows; while it is active no segment is lit and the row reads "Sentence case". Hovering a segment previews it on the Stage (UI-007).
- While on-canvas editing has a range selected, the case segments run **Change case** on the selection instead (ui-concept §7.3 scopes Style changes to a selected range), and their tooltips say so ("Rewrite selection in capitals"). Without a selection in edit mode they keep their display meaning.
**Inputs / controls:** Underline on/off and strikethrough on/off, both default off. Line thickness and position come from the font's metrics, and the color follows the fill. Case defaults to As typed.
**Behavior / output:** The case transform applies only to display. The stored content stays as typed, so switching back restores it exactly. Case mapping uses the clip's language tag. Title Case capitalizes the first letter of every word, with no small-word exceptions (FreeCut default). Sentence case capitalizes the first letter after each sentence end and lowercases the rest. **Change case** is different: it edits the stored content of the selection only, like typing, and the display transform still applies on top. The reference editor offers only this one-click rewrite; FreeCut offers both, because the display transform keeps the original text recoverable. Underlines and strike lines are drawn per line, follow curved baselines, and get the stroke, shadow and glow like glyphs do.
**Edge cases:**
- Case mappings that change length (for example, ß → SS): rendering handles them. Edit mode shows the text as typed (see TX-002).
- Scripts without case: the transform has no effect.
- Underlines run continuously through descenders in v1. Gaps around descenders ("skip ink") come later.
- Underline across a fixed-width wrap: each line gets its own underline.
- **Change case** on a selection that crosses style spans: spans are kept, and their ranges are adjusted when the mapping changes the length (TX-015).
- **Change case** on caption cells (TX-026) works the same way on the selected text.
- Several clips selected with different cases: no segment is lit, and a click sets that case on all of them (UI-010).
- A clip in Sentence case: choosing a segment replaces it; Sentence case stays available under More.
- The reference editor's case buttons rewrite the text itself. FreeCut's control changes only the display, so choosing As typed restores the original capitals exactly.
**Undo:** `SetTextStyle` patch for the display transform. **Change case** is `SetRichText` (TX-015), one entry per command; until TX-015 ships (it stays v1 after Text round 2), it is a `SetTextContent` of the whole new content, made from the content field's selection. A case choice on several clips is one `SetTextStyle` entry.
**Model notes:** `decoration: Decoration { underline: bool, strike: bool }` and `case: CaseTransform::{AsTyped, Upper, Lower, Title, Sentence}`. The case mapping functions are pure, locale-aware and shared by the display transform and **Change case**.

### TX-009 Letter and line spacing — v1
**What the user sees:** Two sliders with numeric fields: **Letter spacing** and **Line spacing**.
**Inputs / controls:**
- Letter spacing: −20% to +100% of the font size, default 0, step 1%.
- Line spacing: 50%–300% of the font's natural line height, default 100%.
**Behavior / output:** Letter spacing is added after each grapheme cluster except the last on each line, so centered text stays centered. In cursive joining scripts, spacing is never inserted inside a joined run. Only gaps between unjoined clusters grow, so letters never break apart.
**Edge cases:**
- Negative spacing makes glyphs overlap. Outlines are drawn in a pass beneath all fills, so one letter's outline never cuts into its neighbor.
- Line spacing below 100% with per-line background boxes (TX-013): the boxes overlap, but they are composited as one shape, so no darker band appears.
- Keyframed letter spacing (TX-021) reflows every frame, which is expected.
**Undo:** `SetTextStyle` patch.
**Model notes:** `letter_spacing: PerMille` (of the font size) and `line_spacing: PerMille` (of the natural line height).

### TX-010 Outline (stroke) — v1
**What the user sees:** An **Outline** section with an enable checkbox, a color swatch, a width slider and opacity.
**Inputs / controls:**
- Color: default black.
- Width: 0–30% of the font size, default 6%. Being relative to size, the width stays proportional when the text is resized or a preset is applied.
- Opacity: 0–100%, default 100%.
- Position: **Outside** (default), **Center** or **Inside** the glyph edge.
- Joins are round in v1. Miter joins come later. Dashed outlines and several stacked outlines are TX-048 (later).
**Behavior / output:** The outline follows the glyph outlines, including per-range sizes. It stays crisp at any scale.
- **Outside:** drawn behind the fill, so the whole width shows outside the letters.
- **Center:** drawn over the fill, so half the width covers the edge of the letters.
- **Inside:** drawn over the fill and clipped to the glyph shape, so the letters keep their outer size.
**Edge cases:**
- Width 0: equivalent to off.
- Bounds: layout, alignment and the background box (TX-013) ignore the outline. Selection handles and snapping use the visual bounds, which include an Outside or Center outline. Changing the position therefore changes the handles but never the wrapping.
- Very thick outlines fill in counters (the holes in letters like o and e). This is accepted. The reference editor allows much thicker outlines; FreeCut caps the width at 30% because beyond that letters fill in completely. Layered thick looks use stacked outlines (TX-048).
- Outlines that extend past the canvas are clipped by the canvas, with no error.
- Inside outlines wider than a stroke of the letter fill it completely. This is accepted.
- Negative letter spacing (TX-009): the guarantee that an outline never cuts into a neighboring letter holds for Outside outlines. Center outlines can cover the edge of an overlapping neighbor, which is expected.
- Emoji and bitmap glyphs get a silhouette outline (TX-006).
- Fill alpha 0 gives hollow text.
- Per-range stroke color overrides are allowed (TX-015). Width stays clip-wide.
**Undo:** `SetTextStyle` patch.
**Model notes:** `stroke: Option<Stroke { color: Rgba8, width: PerMille, opacity: PerMille, position: StrokePosition::{Outside, Center, Inside} }>`, stored as `Layer<Stroke>` (TX-054). Rendering uses distance fields or path offsetting. The distance field's spread must cover the maximum width (30% of the em).

### TX-011 Drop shadow — v1
**What the user sees:** A **Shadow** section with an enable checkbox, color, opacity, blur, distance and angle, plus a small dial for the angle.
**Inputs / controls:**
- Color: default black.
- Opacity: 0–100%, default 50%.
- Blur: 0–50% of the font size, default 10%.
- Distance: 0–100% of the font size, default 8%.
- Angle: 0–359°, default 45°, measured clockwise from the right, so the default points down and to the right.
- **Include background box**: default off.
**Behavior / output:** The shadow is cast by the combined fill and outline shape. With the include option on, the background box casts one too. Layer order from back to front is: background box, shadow, glow, outline, fill, decorations.
**Edge cases:**
- Distance 0 with blur gives a soft halo.
- Shadows extending past the canvas are clipped.
- At 4K, very large blur is capped at 256 device px of radius and computed at reduced resolution, so preview stays real-time.
- The shadow fades together with text animations and clip opacity.
- A blurred shadow never shows a hard cut-off edge: the shadow buffer is padded by three times the blur radius before blurring. The reference editor warns that its blurred shadows can show such an edge.
- Inner shadows, long (extruded) shadows and several stacked shadows are TX-048 (later).
**Undo:** `SetTextStyle` patch.
**Model notes:** `shadow: Option<Shadow { color, opacity: PerMille, blur: PerMille, distance: PerMille, angle: Deci<Degrees>, include_box: bool }>`, stored as `Layer<Shadow>` (TX-054). TX-048 adds a `kind` field to `Shadow`.

### TX-012 Glow — v1
**What the user sees:** A **Glow** section with an enable checkbox, color, intensity and radius.
**Inputs / controls:**
- Color: default white.
- Intensity: 0–100%, default 60%.
- Radius: 0–100% of the font size, default 25%.
- Offset: distance 0–50% of the font size (default 0) and an angle (default 45°, measured as in TX-011), for a glow that leans one way.
- Direction: **Outward** (default), **Inward** or **Both**. Inward glow lights the letters from their edges toward the middle, clipped to the glyph shape. Both draws the two at the same intensity and radius.
**Behavior / output:** A soft outer glow drawn behind the outline and fill, blended additively against what lies beneath. It suits neon-like looks. An inward glow is drawn over the fill and under the decorations, so an outline set to Inside (TX-010) still sits on top of it. Glow is computed from these parameters at render time; no glow preset is ever downloaded (the reference editor fetches its glow looks on demand and fails offline).
**Edge cases:**
- A dark glow color acts like a centered soft shadow.
- Glow plus shadow: both are drawn, in the order defined in TX-011.
- Large radii use the same blur cap as TX-011.
- A blurred-text look is not a glow. It is FX's blur effect (FX-015) applied to the text clip, or the blur In and Out presets (TX-022).
**Undo:** `SetTextStyle` patch.
**Model notes:** `glow: Option<Glow { color, intensity: PerMille, radius: PerMille, offset: PerMille, angle: Deci<Degrees>, direction: GlowDirection::{Outward, Inward, Both} }>`, stored as `Layer<Glow>` (TX-054). Older projects load with `Outward`, so no migration is needed.

### TX-013 Background box — v1
**What the user sees:** A **Background** section with an enable checkbox, color, opacity, a **Shape** choice (Block, Per line or Per character), horizontal and vertical padding, a corner roundness slider, a **Height** choice and an optional border.
**Inputs / controls:**
- Color: default black.
- Opacity: 0–100%, default 70%.
- Shape: default Block for text clips. The default caption track style uses Per line (TX-028). **Per character** gives each grapheme cluster its own box, for letter-tile looks.
- Padding: horizontal 0–100% of the font size (default 25%) and vertical 0–100% (default 15%).
- Roundness: 0–100%, default 20%. At 100% the radius is half the box height, which makes a pill shape.
- Height: **Full** (default) or **Lower half**, a highlighter-pen look that covers only the lower part of each line, from just below the baseline to half the x-height (Per line and Per character only).
- Border: off by default. Color (default white) and width 0–20% of the font size (default 3%).
**Behavior / output:**
- The box hugs the laid-out text bounds, including letter spacing but excluding stroke, shadow and glow, plus the padding.
- In Per line and Per character mode, each line or character gets its own box. All the boxes are drawn into one mask and then composited once at the chosen opacity, so where they overlap there is no extra darkening. The border is drawn around the outline of that combined mask.
**Edge cases:**
- Empty lines in Per line mode get no box. Whitespace gets no box in Per character mode. Empty text gets no box at all.
- Per character boxes move with their glyph during per-unit animations (TX-023).
- Speech bubbles, callouts and tags around a text clip are TX-055's Bubble card, next to Background in the Box & bubble lens. Only one of the two is on at a time (TX-054). Stand-alone bubble elements without text stay OV's (OV-028, later), which reuse TX-055's shapes.
- The reference editor does not allow its text backgrounds together with its ready-made text-effect looks. FreeCut has no such restriction, because its style presets (TX-019) are plain style values.
- Curved text: the control is disabled, with a tooltip. The setting is kept and returns when the curve is 0.
- Per-unit reveal animations (TX-023): the box covers only the glyphs revealed so far, line by line. This is a FreeCut default and suits progressive captions.
- Justified text: in Per line mode the boxes span the justified width.
**Undo:** `SetTextStyle` patch.
**Model notes:** `background: Option<TextBox { color, opacity, shape: BoxShape::{Block, PerLine, PerChar}, pad_x: PerMille, pad_y: PerMille, roundness: PerMille, height: BoxHeight::{Full, LowerHalf}, border: Option<(Rgba8, PerMille)> }>`. TX-054 stores it as `Layer<TextBox>`, and the bubble is a separate layer (TX-055).

### TX-014 Gradient fill — v1
**What the user sees:** The fill section offers **Solid**, **Linear gradient** or **Radial gradient**. The gradient editor is a bar with draggable color stops, an angle dial (linear) or a center point and spread slider (radial), and a reverse button.
**Inputs / controls:**
- 2–8 stops, each with a color (alpha allowed) and a position from 0 to 100%.
- Linear angle: 0–359°, default 90°. 0° runs left to right and 90° runs top to bottom.
- Radial center: x and y as 0–100% of the extent's bounds, default 50%, 50%. The center can also be dragged in the preview.
- Radial spread: the radius at the last stop, 10–200% of half the extent's diagonal, default 100%.
- Extent: the whole text block (default) or each line separately.
**Behavior / output:** The gradient is mapped across the extent's bounds in its rest pose. When characters animate, each one keeps the color it had at rest. A radial gradient is circular in canvas space, so a wide text block shows a circle, not an ellipse (FreeCut default).
**Edge cases:**
- Two stops at the same position make a hard edge.
- Deleting down to one stop is blocked, because a gradient needs at least two.
- A per-range solid color override (TX-015) wins over the block gradient for that range.
- Switching between linear and radial keeps the stops.
- Image (texture) fills are TX-046 and word-by-word color cycling is TX-047 (both later). Using text as a mask over media belongs to TR masks.
- Gradients on outline or shadow come later.
**Undo:** `SetTextStyle` patch.
**Model notes:** `Fill::LinearGradient { angle: Deci<Degrees>, stops: SmallVec<[(PerMille, Rgba8); 8]>, extent: FillExtent }` and `Fill::RadialGradient { center: (PerMille, PerMille), spread: PerMille, stops, extent: FillExtent }`, with `FillExtent::{Block, Line}` (TX-046 adds `Char`).

### TX-015 Per-range (rich-text) styling — v1
**What the user sees:** In edit mode the user selects part of the text and changes its font, size, fill, bold, italic, underline, strikethrough or outline color. Only that range changes. When the selection mixes values, the inspector shows a dash for that control. A **Clear range formatting** button resets the selection to the clip style.
**Inputs / controls:** The properties above. Block-level properties cannot be set per range. These are alignment, spacing, outline width, shadow, glow, background box, curve and animation.
**Behavior / output:** Spans override the clip style only for the properties they set. The resolved style for a character is the clip style with the span overrides applied on top.
**Edge cases:**
- Typing at a span boundary inherits from the preceding character (TX-002).
- Deleting all of a span's text removes the span.
- Adjacent spans with identical overrides are merged automatically. This keeps the model canonical and makes equality comparisons in tests reliable.
- Find and replace (TX-034): the replacement takes the style of the first replaced character.
- A selection that mixes several styles: every control can still be changed, and the change applies to the whole selection. The reference editor lets the user edit only one style run at a time in that case.
- Copy style (TX-020) carries the clip style only, never the spans.
- Import and export: SRT and VTT `<b> <i> <u>` and SRT `<font color>` map to spans, and the same tags are written back on export when that option is on (TX-032).
- The case transform applies after spans.
**Undo:** `SetRichText { item, before: RichText, after: RichText }`. One entry per range-style change.
**Model notes:** `RichText { text: String, spans: Vec<Span { range: Range<u32>, ov: TextStylePatch, origin: SpanOrigin }> }`. `origin` is `Manual` for everything in this feature. TX-038 and TX-040 add the other origins. Its invariants are: spans sorted, non-overlapping, non-empty, grapheme-aligned, and merged when both overrides and origin are equal. Property-based tests cover these invariants across random edits.

### TX-016 Curved text — v1
**What the user sees:** A **Curve** slider with a numeric field. The preview shows the text bending along an arc.
**Inputs / controls:** Curve amount −100 to +100, default 0 (off).
- Positive values bend the line into an arch: the ends drop and the text reads over the top of a circle.
- Negative values bend it into a bowl.
- At ±100 the line spans a full 360° circle. The arc angle equals |curve| × 3.6°, and the radius follows from the line's length.
**Behavior / output:** Each glyph sits on the arc and is rotated to the tangent. Multi-line text goes on concentric arcs with a shared center, spaced by the line height. Alignment is measured along the arc. Underlines follow the arc.
**Edge cases:**
- Fixed-width wrapping is suspended while curved (TX-005), and the background box is disabled (TX-013), in all its shapes.
- Underlined text can be curved: the underline follows the arc (TX-008). The reference editor refuses to curve underlined text; FreeCut does not need to, because decorations are drawn along each run's baseline.
- Every other style (fills, outlines, shadows, glow) works on curved text. Only the background box and the fixed width are suspended. The reference editor also refuses some of its ready-made text looks on curved text.
- At ±100, letter spacing changes the circumference, so the radius grows while the text still closes the circle.
- Per-character animations (TX-023) run in each glyph's local arc frame.
- RTL text along an arc keeps visual order.
- Selection handles show the arc's bounding box. Rotation comes from TR.
**Undo:** `SetTextStyle` patch.
**Model notes:** `curve: PerMille` (−1000..1000). The layout transform is in `freecut-render::text`. Hit-testing for the caret in curved text uses inverse mapping along the arc.

### TX-017 Vertical text layout — later
**What the user sees:** A **Direction** option that switches to vertical layout, with columns running top to bottom and ordered right to left, for CJK titles.
**Inputs / controls:** Horizontal or vertical (default horizontal). For Latin runs inside vertical text: rotated sideways (default) or upright.
**Behavior / output:** Uses the font's vertical metrics and vertical glyph alternates. Punctuation uses its vertical forms.
**Edge cases:**
- Fonts without vertical metrics: each glyph advances by the em height.
- Curved text (TX-016): the curve control is disabled in vertical mode. A stored curve is kept and returns when the direction is set back to horizontal.
- Latin words and numbers inside a vertical column: rotated 90° clockwise by default, or set upright one by one when **Upright** is chosen.
- Fixed-width box (TX-005): in vertical mode the fixed dimension becomes the column height, and columns wrap leftwards.
- Background box (TX-013) in Per line mode: one box per column.
- Caption tracks: vertical layout is not offered for captions.
**Undo:** `SetTextLayout` patch.
**Model notes:** `TextLayout.direction: Horizontal | Vertical { latin: Rotated | Upright }`. This is deferred because Rust shaping-stack support for vertical layout is limited.

### TX-018 Import a font file — v1
**What the user sees:** **Add font file…** and **Add font folder…** in the font picker. The user can also drag `.ttf`, `.otf`, `.ttc` or `.otc` files, or a folder, onto the font list. Imported fonts appear under **Imported** in every project. An info popover shows the font's name, version, designer and license text from its metadata.
- A folder import scans the folder and its subfolders, then asks for confirmation with the number of font files found.
- Import shows progress in the background-task list (PM-073) and ends with a short summary: imported, skipped as duplicates, failed.
- The Imported list supports multi-selection with a running count ("3 selected"), **Remove selected** and **Remove all imported**. Removal asks for confirmation, with a **Don't ask again** checkbox stored in preferences.
**Inputs / controls:** One or more font files, or a folder. There is no limit on the number of files per import (FreeCut default; the reference editor takes at most 9 at a time). WOFF and WOFF2 come later.
**Behavior / output:** Each file is validated and then copied into FreeCut's per-user font folder, under the OS app-data directory. FreeCut does not install it into the operating system. All of this runs on a background thread. No account or network is involved.
**Edge cases:**
- Corrupt or unsupported file (for example Type 1 or bitmap-only fonts): an error message is shown and nothing is copied.
- Partial failure: the files that validated are imported. The summary lists each file that was not, with its reason (corrupt, unsupported format, duplicate, unreadable).
- Same PostScript name and version already imported: the file is skipped as a duplicate. If the version differs, the user is asked whether to replace it.
- The same font is also installed system-wide: the imported copy wins (TX-003).
- Removing an imported font that is in use in the open project: a confirmation lists how many clips use it. Those clips then go through the TX-007 flow, unless the project was packaged with its fonts (PM-101), in which case they render from the project's `Assets/` folder (PM-042). The reference editor keeps the look of text whose font was removed; FreeCut keeps no hidden copy, so it reports the font as missing instead.
- System fonts and bundled fonts cannot be removed from FreeCut; only imported ones can.
- An imported file deleted from FreeCut's font folder outside FreeCut: on the next scan the font is listed as **file missing**, with **Locate…** and **Remove** (TX-007).
- Fonts whose embedding flags are restricted: these can be imported for local rendering. They are flagged in the info popover and excluded from "collect fonts".
**Undo:** n/a. This is the app-level font library, not a project edit. Removal asks for confirmation instead.
**Model notes:** `FontRegistry` sources: `Bundled | System | UserImported(PathBuf)`. The folder location and preferences belong to the app. The project stores only `FontRef`.

### TX-019 Built-in text style presets — v1
**What the user sees:** A **Looks** grid (called Styles in earlier drafts) in the Text Drawer and in the inspector's Presets lens, with about 36 FreeCut-designed looks, each shown as a thumbnail of "Aa". The grid has UI-006's search field and category chips. Clicking a style applies it to the selected text clips, or to the caption track or segments (TX-028). Hovering previews it. **Apply to all text clips** in a style's context menu applies it to every text clip on unlocked tracks in the sequence. Planned groups and examples:
- **Clean:** plain white, plain black.
- **Two-tone:** about 12 pairs, each combining a fill color with a contrasting outline, box or shadow color (for example dark fill with a light outline, light fill with a yellow box). The pairs come from FreeCut's own palette.
- **Outlined:** white with a dark outline, dark with a white outline, warm yellow with a dark outline.
- **Boxed:** white on a dark label, dark on a light pill, translucent per-line caption box.
- **Shadowed:** soft drop shadow, hard offset retro shadow.
- **Glow:** several neon-like color pairs.
- **Gradient:** warm two-tone, cool two-tone.
- **Display:** condensed caps with tight tracking.
- **Statement:** bolder stylized looks, built from the same style parameters as every other preset rather than kept in a separate effect library. Each is defined by its values, not by artwork:
  - *Sticker cut:* fill as chosen, a white Outside outline at 18% of the size, then a soft drop shadow (opacity 35%, blur 12%, distance 6%). Reads like a die-cut sticker.
  - *Comic punch:* warm yellow fill, a dark Outside outline at 14%, and a hard offset shadow (blur 0, distance 10%, angle 45°) in the outline colour. Bold face.
  - *Neon tube:* fill at 0% alpha, a thin bright Center outline at 4%, and a Both-direction glow (TX-012) in the outline colour, intensity 80%, radius 30%.
  - *Highlighter:* dark fill with a Per line box, Height Lower half, in a light marker colour at 85%.
  - *Label tag:* white fill on a dark Tag bubble (TX-055) with 30% roundness and no tail.
  - *Candy stripe:* a linear gradient (TX-014) of two light colours at 90°, a white Outside outline at 8%, and a soft shadow.
  - *Retro block:* a long shadow (TX-048) at 135°, length 25%, in a darker shade of the fill. Shown only once TX-048 ships.
  - *Dusty board:* a procedural dusty texture fill (TX-046) with a thin Center outline in the fill colour at 40% opacity. Shown only once TX-046 ships.
  - *Rainbow words:* a word-colour cycle (TX-047) over five FreeCut palette colours with a dark outline. Shown only once TX-047 ships.
- **Capability gating:** each preset file names the style features it needs. A preset that needs a feature this build does not have is not listed, so the grid never offers a look it cannot render. A project made with a newer build keeps its resolved values (Behavior below); the inspector shows the unknown parts as **Not supported in this version** and keeps them on save.
**Inputs / controls:** The chosen preset.
**Behavior / output:** A preset is a partial style. It may set fill, gradient, outline, shadow, glow, background or bubble (TX-055), case, letter spacing and, optionally, a bundled font. It never changes size, alignment, layout, content or animation. Per-range overrides are kept. The user can clear them with TX-015. The same grid is the Text Drawer's **Looks** chip (TX-001) and the Presets lens's **Looks** card (TX-054).
- Lengths in a preset are relative to the font size (TX-010 to TX-013), so one preset looks the same at every size.
- A preset owns the whole look: fill, outline, shadow, glow, background, bubble, case and letter spacing it does not mention are first returned to the default text style's values, then its own values are applied. Applying one preset after another therefore never leaves parts of the first behind. The font changes only when the preset names one. **Clear styling** in the Looks card's `...` menu returns the clip to the default text style (TX-020) in one step, keeping content, size, layout and animations.
**Edge cases:**
- Several clips selected: one undo op covers all of them.
- Preset font missing, for example in a Linux distro build that unbundles fonts: the TX-007 fallback applies.
- A preset with a background box applied to curved text: the box is stored but stays inactive.
- Preset files from a newer schema are ignored, with a log entry.
- In edit mode with a range selected, a preset still applies to the whole clip (FreeCut default). The reference editor's ready-made looks also never apply to part of a clip. Range styling stays with TX-015's individual controls.
- **Apply to all text clips** skips clips on locked tracks and reports how many were skipped.
- Every preset is generated by FreeCut's renderer from its parameters. Nothing is downloaded, so every preset works offline.
- A property the preset sets that already has keyframes (TX-021) keeps its keyframes, which win while they exist. The Looks card shows a hint, "3 animated properties kept their keyframes", with **Remove those keyframes**, which is its own undo entry.
**Undo:** `ApplyTextPreset { targets, before: Vec<TextStylePatch>, after }`. **Apply to all text clips** is one op over all targets.
**Model notes:** Presets are JSON in `resources/text-presets/` with a schema version, loaded by `freecut-app`. `TextStylePatch` is in `freecut-timeline`, and preset (de)serialization is in `freecut-project`. Preset IDs are stable strings such as `fc.style.outline-white`. The project stores the resolved values, not preset IDs, so projects never depend on the preset library.

### TX-020 Saved styles, copy/paste style and default style — v1
**What the user sees:**
- **Save look…** (called Save style in earlier drafts) stores the current clip's look under a name in **My looks**. Saved looks can be renamed, deleted, and exported or imported as a `.json` style pack.
- The text clip context menu has **Copy style** and **Paste style**. The shortcut is assigned in [workspace-and-shortcuts.md](workspace-and-shortcuts.md).
- **Set as default for new text** makes TX-001 use this style.
- *Save look (v1):* the footer of the Style and Box & bubble lenses (TX-054) has **Save look…**, which opens UI-064's shared dialog. Its checklist for text is: **Style** (always on: everything TX's Text style category holds, including a bubble, TX-055), **Animations** (In, Out and Loop with their settings; default on when the clip has any), **Layout** (alignment, box mode and width; default off) and **Text** (the content itself; default off, so a preset is a look rather than a fixed phrase). The dialog suggests the name "My look N".
- Saved looks appear in the Text Drawer's **My looks** chip (TX-001) and in the Presets lens's **My looks** card (TX-054). Both are views of one library. Other kinds keep their own saved-look libraries under the same name in their own Drawer and lens (for example colour looks, CL-026); each is a separate `PresetLibrary<T>` of UI-064. Each tile is drawn by FreeCut's renderer with the preset's own look on "Aa", or on the first word of its saved text, with its In animation playing on hover.
- Clicking a **My looks** tile in the Drawer adds a new text clip at the playhead with that preset (TX-001's insert, then the apply), so a saved preset also works as a personal "add text" button; dragging it to an empty place on the timeline does the same at the drop point. Clicking it in the Presets lens, or dropping it onto a text clip, applies it to the selected or target clips (TX-001).
**Inputs / controls:** Style name, the save checklist, and the file path for import or export.
**Behavior / output:** User styles live in the app config directory, not in the project. Applying one copies its values into the clip.
- Applying a preset with **Animations** replaces the clip's In, Out and Loop slots with the saved ones, through FX's slot model (TX-022). Applying one without **Animations** leaves the clip's animations alone. **Layout** and **Text** work the same way. Size is part of Style.
- **Copy style** copies the clip-level style only. Per-range spans (TX-015) are not copied, and pasting keeps the target's own spans (FreeCut default). The reference editor leaves rich-text clips out of style copy and paste entirely.
- TX registers one category, **Text style**, in TL-034's Paste attributes registry: font, size, fill, decorations, case, spacing, outline, shadow, glow, background and curve, but not content, layout or animations (animations travel in FX-017's category). It is checked by default. **Paste style** is a shortcut that pastes only this category from TL-034's attribute clipboard.
**Edge cases:**
- Paste style onto a non-text clip: disabled.
- Paste onto a caption segment: becomes that segment's override. Paste onto a selected caption track: becomes the track style.
- Default style uses a font that has since been removed: new text renders with fallback and a missing-font badge.
- Corrupt import file: an error is shown and nothing is imported.
- Name collision: a suffix such as " (2)" is added.
- Deleting a saved style is permanent (there is no trash), so it asks for confirmation.
- There are no premium materials, so any style can be saved.
- A preset saved with **Text** applied to several selected clips: the content is applied only when one clip is selected. With several, the content part is skipped and a hint says so, so a click never overwrites many texts at once.
- A preset whose saved In + Out are longer than the target clip: stored as saved, and FX-018's proportional scaling applies (TX-022).
- A preset saved from a clip with keyframes: keyframes are not saved (FreeCut default), because their times only make sense on the original clip. The dialog's summary says "Keyframes are not included".
- A preset saved with a font that is later removed: applying it goes through TX-007.
**Undo:** Paste and apply are `SetTextStyle` patches. Paste through **Paste attributes…** is part of TL-034's compound op. Applying a preset that carries animations, layout or text is one compound `ApplyTextPreset` with the matching `SetClipAnimation`, `SetTextLayout` and `SetTextContent` changes. Adding a new clip from a My looks or Looks tile is one compound op of TL's `InsertClips` and that apply. Library management is preferences (n/a), and deleting asks for confirmation.
**Model notes:** The style pack schema is in `freecut-project`. The library store and clipboard are in `freecut-app`. A saved preset is `TextPreset { schema, id, name, style: TextStylePatch, animations: Option<ClipAnimations>, layout: Option<TextLayout>, content: Option<RichText> }`, written through UI-064's `PresetLibrary<TextPreset>`. Built-in presets (TX-019) use the same type with only `style` set.

### TX-021 Keyframable text properties — v1
**What the user sees:** Keyframe diamonds, from TR, next to these text properties:
- Fill color (solid) and size.
- Letter spacing and line spacing.
- Outline color and width.
- Shadow color, opacity, blur, distance and angle.
- Glow intensity and radius.
- Background color, opacity and roundness.
- Curve and gradient angle.
- **Reveal**, which is the percentage of characters visible, for custom typewriter effects.
- Outline opacity, glow color and offset distance, background border width, and the bubble's tail tip, tail width and bend (TX-055).
- Gradient stop colors (TX-014), stop by stop; stop positions and count are not keyframable.

*The keyframe row pattern (v1, the same on every row above):* each animatable row ends with the UI concept's keyframe navigator in the row gutter ([ui-concept §13.6](../design/ui-concept.md#136-keyframe-navigator--custom)): ‹ previous key, UI-009's diamond, › next key (TR-028, TR-030). The arrows show only once the property is animated, so rows without animation stay quiet. Each card's `...` menu adds card-wide keyframe commands, which the command bar also offers:
- **Key this card** adds a key at the playhead to every animatable property of the card, with its current value. **Remove this card's keys here** removes the card's keys at the playhead.
- **Previous key in card** and **Next key in card** jump across the union of the card's keyframe times.
- A collapsed card with anything animated ends its summary chip with a small filled diamond, so animated cards are visible without opening them.
- A card that is switched off keeps its keyframes (see Edge cases), and its chip still shows the diamond.
**Inputs / controls:** Defined by TR: add or remove keyframes, easing, and the curve editor.
**Behavior / output:** Interpolation follows TR's rules, and colors interpolate in linear-light RGB as TR and CL define. Content, font family, alignment and the on/off toggles cannot be keyframed. Changing an animated property away from a key adds a key at the playhead, following TR-028's auto-key preference.
**Edge cases:**
- Keyframing size reflows the text each frame, so wrap points can jump. The inspector suggests TR scale for smooth growth.
- A property that is switched off (for example, outline disabled) keeps its keyframes, inactive.
- Trimming and splitting follow TR's keyframe rules.
- Keyframes combine with preset animations as defined in TX-022.
- The playhead outside the clip: diamonds and arrows are disabled, as TR-028 defines.
- **Key this card** on several selected clips: keys are added on each clip under the playhead, as TR-028's multi-selection rule says.
- **Reset card** (TX-054) removes that card's keyframes together with its values, in one undo entry.
- Image fills, word-colour fills and stacked extra outlines and shadows (TX-046 to TX-048) are not keyframable in this version. Their rows show no diamond.
**Undo:** TR's keyframe ops. **Key this card** is one compound entry over the card's properties.
**Model notes:** `AnimatableProperty::Text(TextProp)`, registered with TR's keyframe system. The card grouping is a pure table in `freecut-timeline`, `fn card_props(card: TextCard) -> &'static [TextProp]`, shared by the card keyframe commands, **Reset card** (TX-054) and the command bar. Values use TX's fixed-point style types (TX-001 conventions) and `Rgba8`, wrapped in TR-027's `Animatable<T: Interpolate>`. TX implements `Interpolate` for these types as TR-027 prescribes: fixed-point values interpolate in `f64` and are rounded to their step when evaluated, never when stored, and colors interpolate in linear light.
*FreeCut decision (S2 model, 2026-10-13):* keyable text properties are `PropPath::Text(TextPropPath)` for the animatable rows, `PropPath::FillStopColor(N)` per gradient stop and `PropPath::Reveal`; keys live in the clip's `ClipKeys` (TR-027's S2 note). A keyed property the current style lacks (the solid colour under a gradient) keeps its keys, inactive. Reveal has no static control: without keys every character shows, and removing its last key returns to that. Style angles take the shorter way round. **Key this card** and **Remove this card's keys here** are `KeyEdit::KeyCard` and `KeyEdit::ClearCard` (`PropGroup::Text(card)`; `PropGroup::Transform` is TR-028's "all transform properties"); `ResetText` removes the scope's keys in the same entry, the Box & bubble lens also the transform's.

### TX-022 Text animations: In, Out and Loop — v1
**What the user sees:** The inspector's **Motion** lens (called the Animation tab in earlier drafts) with **In**, **Loop** and **Out** slots. Each is a grid of FreeCut-designed presets with motion thumbnails that play on hover, plus **None**, and a duration (or speed) slider below. On the timeline, the clip shows shaded regions at its start and end that match the In and Out lengths.

Planned presets, with generic names:
- **In:** fade, slide from each of the four edges, zoom in, zoom out, pop with overshoot, blur in, left-to-right wipe reveal, drop with bounce, spin in, vertical flip-open.
- **Out:** the mirror of each In preset.
- **Loop:** pulse, float, wobble, shake, flicker, swing, hue cycle, breathing glow.
The full list, with each preset's motion and default parameters, is TX-056's catalogue.

*Motion lens (v1):* [ui-concept §7.2 and §13.11](../design/ui-concept.md#72-motion-lens) own its presentation. This spec fixes its behaviour:
- The **timing strip** stands for the whole clip, split into In, Loop and Out. Dragging the join between In and Loop sets the In length, and the join between Loop and Out sets the Out length (frame-snapped, within FX-018's limits, one undo entry per drag). Clicking a segment selects that slot's preset grid. An empty slot shows as an empty segment.
- The preset grid of the selected slot, with UI-006's search. **Pieces** (block, line, word or letter) sets TX-023's unit and narrows the grid to presets that support it.
- The selected preset's controls: Length (or **Pace**, below), Easing, Direction where the preset has one, Intensity, and TX-023's Overlap and Order for per-unit presets.
- A **Keyframes** card lists every animated property of the clip (TX-021), and a **Follow** card holds motion tracking (AI-008), so one lens answers what moves and when.
- Hovering a tile plays it on the Stage (UI-007).
**Inputs / controls:**
- In and Out durations, easing override, and the duration limits (0.1 s minimum, at most the clip duration minus the other slot's duration, snapped to frames) are those of FX-018 in [effects-transitions-animations.md](effects-transitions-animations.md). Default 0.5 s each, unless the preset names its own default (TX-056).
- Loop cycle length and strength use FX-019's controls and ranges; strength is the same control as Intensity below.
- **Intensity** (0–100, default 50) on every preset that moves, scales, rotates or blurs: it scales the preset's amplitude as FX-030 defines. Fades and pure reveals have no intensity.
- **Pace** (per-unit In and Out presets on letters or words, TX-023): instead of a total duration, the user may set a time per unit, 0.02–1.0 s (default 0.06 s per letter for typewriter presets, 0.15 s per word; FreeCut defaults). Line units have no Pace, because how many lines there are depends on layout, which the model does not compute. The slot duration is then computed as `units × pace × (1 − overlap) + pace × overlap`, rounded to frames, and shown read-only next to the field. A switch toggles Duration and Pace; switching keeps the current length.
**Behavior / output:**
- Text clips use FX's clip-animation slot model (FX-018 Entrance/Exit, FX-019 Continuous), which applies to video, image, text and element clips. In this spec those slots are labeled In, Out and Loop, and offer text presets that can move individual glyphs (TX-023) as well as the whole block.
- In runs over the first `in` of the clip and Out over the last `out`, as FX-018 defines.
- FX-018 and FX-019 set the slot policy per clip kind. Video, image and element clips use the exclusive policy, where choosing Continuous clears Entrance and Exit, and the reverse. Text clips use the coexisting policy: Loop runs together with In and Out. This spec defines the text window. Loop runs only between In and Out, in `[in, d − out)` of clip-local time, and is inactive while In or Out plays (FreeCut default). On a text clip, choosing In or Out never clears Loop, and choosing Loop never clears In or Out.
- Animation offsets are applied on top of the TR transform and keyframes, at the point in the render order that FX-001 defines for text (L8–L10).
- *Composition order (v1).* At a clip-local time `t`, a text clip is evaluated in this order. Each step reads only the results of the steps before it:
  1. **Style at `t`:** keyframed style values (TX-021) are evaluated. Loop presets that modulate style (hue cycle, breathing glow) then modulate the evaluated values; they never write keyframes.
  2. **Layout:** the text is laid out once from the style at rest (letter spacing and size included). Animations never reflow it (TX-005, R7 in the text architecture).
  3. **Per-unit motion** (TX-023): each glyph gets the offset, scale, rotation, opacity and blur of its unit's preset at `t`, in the text's local frame and about the unit's own centre. The **Reveal** keyframe (TX-021) and a per-unit reveal multiply: a glyph shows only when both let it.
  4. **Raster:** box or bubble, shadow, glow, outline, fill and decorations are drawn around the moved glyphs (TX-011's order), so outlines and boxes follow the animated letters.
  5. **Whole-block motion:** the active slot's block offsets are folded into the clip's keyframed transform at FX-001's L8: position offsets add, scale factors multiply, rotation offsets add, all about the transform anchor. A motion-tracking binding (AI-008) is part of that transform: position = tracked point + offset + keyframes, and then the animation offset is added, so a sliding title still slides relative to the tracked object.
  6. **Opacity:** animation opacity multiplies clip opacity (TR-015) and fill alpha at L9.
- Only one of In, Loop and Out is active at any `t` (the text window rule above), so their offsets never add up. Per-unit and block motion of the same slot both apply.
- **Loop seams.** In ends at rest and Out starts at rest, but a Loop's cycle runs from the clip start plus its phase, so at `in` it would start mid-cycle and at `d − out` it would stop mid-cycle: a visible jump. So a Loop eases its strength in over the first `min(0.25 s, window / 4)` of its window when an In plays before it, and out over the last `min(0.25 s, window / 4)` when an Out follows it (smoothstep; every amplitude is scaled, so the cycle and the per-unit delays keep running). The pose is then at rest on both sides of each seam. A Loop that starts at the clip start or ends at the clip end has no ramp there, so a split (whose pieces lose In or Out) stays seamless (FreeCut default).
- **Pace** results are stored as the duration they produce, plus the pace, so editing the text recomputes the duration from the new unit count. An edit that makes In + Out longer than the clip is handled by FX-018's proportional scaling, and the timing strip shows the shortened segments hatched, with the hint "Shortened to fit the clip".
- UI-043's **Reduce motion** stops tiles from playing on hover (UI-007), and a still frame at 60% of the motion is shown instead. The player always shows the real motion, because it is the user's content.
**Edge cases:**
- In + Out longer than the clip (for example, after trimming): handled as in FX-018. Effective durations are scaled proportionally, and the stored values are untouched.
- In + Out fill the whole clip, or more: the Loop window `[in, d − out)` is empty, so Loop does not play. Loop stays stored and plays again once the clip is long enough.
- Clips shorter than 2 frames skip animation. Choosing a preset on such a clip is still allowed: the preset is stored, and a hint says the clip is too short to show it. The reference editor refuses the choice instead.
- Splitting a text clip (TL-023): In and Out follow FX-018's split rule (In stays on the left part, Out on the right part). Loop stays on both parts, and the right part's phase offset is set as in FX-019, so the motion continues seamlessly.
- Trimming the clip start shifts the Loop phase. A "lock phase to timeline time" option comes later.
- Captions: a track-level animation applies to every segment individually, and segment overrides are possible (TX-028).
- Dragging a timing-strip join past the other slot: the drag stops at the other slot's edge, so In + Out never exceed the clip through the strip. Typed values follow FX-018's limits.
- Pace on a clip whose text is empty or only spaces: the duration falls back to the 0.1 s minimum, and the hint says there is nothing to reveal.
- A **Pieces** choice that the selected preset does not support: the preset stays, Pieces snaps back to the preset's unit, and a hint names the units the preset offers.
- An Out preset whose motion leaves the canvas (slide out to an edge): the clip still ends at its own end. Nothing is held past the clip.
**Undo:** FX's `SetClipAnimation { clip, slot: Entrance|Exit|Continuous, old, new }` (FX-018). A timing-strip drag that changes one join is one entry; Duration, Pace, Easing, Direction and Intensity changes are entries of the same op, coalesced per gesture. Under the text policy no other slot changes, so the op on a text clip never carries exclusivity side effects. For caption tracks and segments, the same change is recorded by `SetCaptionTrackStyle` or `SetCaptionOverrides` (TX-028). Split handling is part of TL's split op.
**Model notes:**
- Text clips store FX's `ClipAnimations { entrance, exit, continuous }` (FX-018). `AnimationSpec.preset` holds a text preset ID, and `AnimationSpec.params` carries the text-specific `AnimParams` of TX-023. Loop presets are stored as FX's `ContinuousSpec`.
- TX defines only the text preset data: normalized curves for offset, scale, rotation, opacity, blur and reveal, stored in `resources/text-animations/`. Preset IDs are stable, for example `fc.in.fade`.
- FX's slot logic reads the policy from the clip kind (FX-018, FX-019). For `ClipKind::Text` it keeps all three slots and hands the Loop evaluator the `[in, d − out)` window. Caption tracks and segments use the same text policy.
- Evaluation into per-glyph transforms lives in `freecut-render::text::animate`, because it needs the layout. Whole-block offsets reuse FX's `animation_offsets` evaluator.
- Intensity is FX's common `intensity: PerMille` in `AnimationSpec.params` (FX-030). Pace is `AnimParams.pace: Option<RationalTime>` (TX-023); when set, `AnimationSpec.duration` is recomputed by a pure `pace_duration(units, pace, overlap, frame_rate) -> RationalTime` in `freecut-timeline` on every content edit, inside the same op as the edit, so undo restores both. Units are counted with Unicode grapheme and word segmentation, which needs no layout.
- The composition order above is fixed in code: `freecut-timeline::plan` produces the style at `t` and the folded block transform; `freecut-render::text::animate` applies steps 3 and 4. Tests pin the order with a clip that has a keyframed position, a tracking binding, a slide In and a per-letter rise at once.
- *FreeCut decision (S2 model, 2026-10-13):* `frame_plan_with(doc, seq, t, inputs, &AnimContext { presets, text_bounds })` evaluates steps 1, 5 and 6 and hands the rest to the renderer in `LayerPlan::motion` (blur, wipe, tint, the Reveal key, and `TextAnimPlan { slot, local_t, window, spec, manifest, unit, direction, progress }` for a playing per-unit preset); plain `frame_plan` has no catalogue and plays keyframes only. Without the renderer's laid-out box (`text_bounds`) distances use an estimate (0.55 em per character, 1.2 em per line). A 3D turn makes `layer_to_canvas` projective. Pace counts the spec's own unit (the app stores the effective unit with Pace) and is dropped on line and block units. Units are counted without tables: grapheme clusters approximated (combining marks, joiners, variation selectors, emoji modifiers, tags, flag pairs) and words split at white space, each ideograph or kana alone.
- *FreeCut decision (S2 engine follow-up, 2026-10-13):* the seam rule is `freecut_timeline::motion::seam_envelope(window, clip_len, local_t)`, applied by `frame_plan_with` to the playing Continuous slot (whole-block offsets, the style modulation of step 1, and per-unit work). `TextAnimPlan` gains `envelope` (the value at `local_t`; its `manifest` already has its amplitudes scaled by it, so units posed from it ease without further work), `frame` (the sequence frame, for Decode's and Flicker's "2 frames"), `style` (which of hue, glow, lightness and tint the preset modulates per unit, so the renderer lightens Shimmer's band glyph by glyph from each unit pose's `lightness`) and `unit_pose(p, scale, unit)`. The renderer poses units with `unit_pose` (or `motion::pose_with`) and a `UnitContext { index, to_line_center, line_height, frame, amount }`: `index` is the unit's reading-order index among timed units (seeded channels key on it, so the Order permutation never changes a unit's own motion), `to_line_center` the vector from the unit's centre (TX-023's centres) to its line's box centre, `line_height` its line's height, all in the lengths of its `MotionScale` (em in the renderer). A pose's `line_clip` asks for the unit to be clipped to its line's box. `LayerMotion` gains `reveal_feather` (FX-030's 8 % soft edge while a whole-layer wipe plays).
- *FreeCut decision (review fixes, 2026-10-13):* a split never changes a frame of a Loop near its seams. The Loop slot keeps, after a split, the seams it had on the unsplit clip (`AnimationSpec::seams`: In's end and Out's start in loop time, clip-local time plus the phase, and the ramp length), so `seam_envelope(spec, window, clip_len, local_t)` still eases a piece in or out at a seam that now lies beyond its edge, with the unsplit ramp. A stored seam that falls inside a piece (its In or Out removed since) is ignored: the Loop then starts or ends with the clip, unramped. Per-unit style channels draw per glyph: a unit's hue turn turns its fill and outline (before its tint), and its glow factor scales the glow around it (where glyphs overlap, the hue is that of the glyph covering the pixel most, never an average of angles, and the glow factor is their coverage-weighted mean; a factor never lifts the glow past full intensity); with Glow off, a per-unit glow preset plays on the same temporary glow as Breathing glow. The text architecture's ligature policy holds for the clip's whole length: a clip with a slot whose effective unit is Letter is shaped without `liga`, `clig` and `dlig` (`LayerPlan::ligatures`), so its rest pose has the same glyphs as its animated frames; letter spacing ≠ 0 turns them off too. A letter that still owns no glyph (a font ligature beyond these features) folds into the letter before it and takes no time.
- *FreeCut decision (S2 app, 2026-10-13):* the timing strip shows an empty In or Out as a hollow segment 15 % of the strip wide, so it can be clicked; only a set In or Out has a join. Dragging a join stores a plain length (it switches Pace off). Hovering a tile plays it in the tile only (the Stage preview of a hovered tile is not built yet). The text grids offer TX-056's presets and the clip grids FX-030's (`fc.clip.…` ids, `resources/README.md`); an Out without a file of its own is offered as the mirror of its In (`fc.out.x` for `fc.in.x`). A preset over FX-030's flash limit is never offered. **Pieces** is a filter of the grid, remembered per slot for the session, and also sets the chosen preset's unit when it moves that unit; otherwise a hint names the units it offers. The Pace switch keeps the length by choosing the pace that gives it (within 0.02–1 s) and stores the effective unit with it. Tile pictures are FreeCut's own sample, "Abc" on a 4:3 canvas, rendered on a worker thread: the still is the pose 60 % of the way through the motion; a hovered tile's loop plays at 12 frames a second, at most 4 s, one tile at a time. Preview and export plan frames with the same catalogue (`Compositor::plan`), so they match frame for frame.
- *FreeCut decision (S2 app follow-up, 2026-10-13):* the grids are built once per loaded catalogue (`anim::PresetGrids`, rebuilt only when the catalogue changes), never per repaint. FX-030's full photosensitivity check runs once per preset on a worker thread after loading (`PresetTable::precompute`), and the grids read its cached answer (`PresetTable::availability`); a preset over the limits is still never offered. A preset the catalogue lists as unavailable (its file needs a channel or schema this build lacks) is shown, not hidden: a dimmed tile with a warning sign at the end of the group its id names (an unavailable In also as its mirrored Out), which can't be chosen and whose tooltip reads "Unavailable animation" with the loader's reason (FX-018). Pieces doesn't filter these tiles, since without a manifest the pieces are unknown. Every bundled preset currently loads, so none shows today. The Loop grid is grouped by FX-019's kinds (see FX-019).
- *FreeCut decision (review fixes, 2026-10-13):* a tile chosen while **Pieces** shows a unit stores that unit when the preset moves it (the grid only offers such presets), so Pieces and the animation always agree. When In and Out are *Shortened to fit the clip*, a join drag also stores the other slot at the length it shows, in the same undo entry. The dragged join then follows the pointer and the other segment stays where it is drawn. Showing a length longer than the clip leaves room for never edits it: number fields clamp only what the user enters. A hovered tile's loop is the sample's (the preset's default settings, a paced preset as long as its Pace makes "Abc"). The Motion lens counts it from the same times the poster worker renders, also on the chosen tile.

### TX-023 Per-character, per-word and per-line animations — v1
**What the user sees:** Presets in the In, Out and Loop grids that animate text piece by piece. Examples:
- **In:** typewriter (characters appear one at a time), fade by letter, rise by letter, word pop, word slide-up, line slide.
- **Loop:** a wave across letters and bouncing letters.
Each preset offers **Unit**, **Overlap** and **Order** controls.
The full list of per-unit presets and their motions is TX-056's catalogue.
**Inputs / controls:**
- Unit: character (grapheme), word or line, limited to the units each preset allows.
- Overlap: 0% (strictly one after another) to 100% (all together), default 50%.
- Order: forward, backward, center-out, edges-in or random, default forward. The random order uses a seed stored in the project, so it is deterministic. **Edges-in** starts with the first and last units and ends in the middle.
- **Pace** instead of a total duration: TX-022.
- **New seed** (a dice button next to Order when Random is chosen) picks another seed. It is an ordinary undoable change.
**Behavior / output:** The In or Out duration is divided among the units according to the overlap. A **Sync to speech** option comes later (TX-039). It uses caption word timings instead of an even split.
- Each unit animates about its own centre: a glyph's ink-box centre for letters, the word's box centre for words, and the line's box centre for lines. Rotations and scales therefore never swing a letter around the middle of the whole block.
- Out presets play each unit's motion backwards in time, but the unit order still follows Order: with forward, the first unit leaves first (FreeCut default), which reads naturally for exits. The erase preset, which removes letters from the end, uses Order backward by default (TX-056).
- Loop presets with units use a phase offset per unit instead of windows: unit `i` runs the loop with a delay of `i × cycle × (1 − overlap) / units`, so a wave travels across the text and wraps seamlessly.
**Edge cases:**
- Whitespace units take no time.
- In RTL text, "forward" means reading order.
- A ZWJ emoji sequence counts as one unit.
- Curved and per-range-styled text work normally.
- Very many units in a short duration: more than one unit may reveal per frame, deterministically.
- Background box behavior during reveals is described in TX-013. A bubble (TX-055) is not revealed per unit: it follows the block motion and is fully shown from the first frame of the slot.
- Cursive-joining scripts (Arabic and similar): letter-unit presets degrade to word units, so joined letters never come apart (the text architecture's ligature policy). The Unit control shows the effective unit.
- Changing the text while an In is set: windows are recomputed from the new unit count. With Pace on, the duration changes too (TX-022).
- A single unit (one word with Unit = word): the preset runs on it alone over the whole slot.
**Undo:** Same as TX-022.
**Model notes:** `AnimParams { unit: Option<AnimUnit>, overlap: PerMille, order: AnimOrder::{Forward, Backward, CenterOut, EdgesIn, Random}, seed: u32, pace: Option<RationalTime> }`, carried in FX's `AnimationSpec.params` (TX-022). The loop phase offset per unit is a pure function next to the window split. The split of a duration into per-unit windows is a pure function over rational time, unit-tested in `freecut-render::text::animate`.

### TX-024 Text templates — v1
**What the user sees:** A **Templates** tab in the Text panel with FreeCut-designed templates. Planned kinds are title cards, lower thirds (name and role), chapter headings, a quote card, an end card with a generic call to action (no platform logos), a countdown number and a bulleted list reveal. They are grouped by kind, about 8 groups (FreeCut default), and can be filtered by the language of their placeholder text. Single templates can be starred (UI-006), and favorites need no account. Thumbnails animate on hover. Clicking or dragging inserts a template at the playhead as a **group** of text layers, and optionally shape layers from OV. Each layer shows placeholder text that can be edited in place. **Save selection as template…** turns the selected text clips into a user template.
**Inputs / controls:** Template choice. A default duration per template, for example 4 s. Group duration can be changed by trimming the group.
**Behavior / output:**
- Insertion creates the clips on as many tracks as the template needs, as one group (TL-039).
- Each layer keeps its style, transform and animations.
- Stretching the group stretches each layer's hold time. Animation durations stay fixed.
- **Ungroup** (TL-039) releases the layers as independent clips, and a short toast (UI-040) confirms it.
- A project never depends on the template library: the inserted layers are stored by value, so a template removed later causes no "missing template" state. The reference editor flags templates that are missing from a project.
**Edge cases:**
- A layer may restrict its placeholder input, for example to digits for a countdown. Other characters are refused with a hint. The rule travels in the inserted clip's `TextOrigin::Template` and stops applying after Ungroup.
- Templates are bundled or user-made and never downloaded, so there are no download, retry or "still downloading" states.
- The reference editor can apply a chosen template's look to every text clip. FreeCut does that with style presets (TX-019, **Apply to all text clips**). Templates only insert.
- Not enough free tracks: tracks are created (TL-006). If TL's track limit would be exceeded, insertion is refused and nothing changes.
- A template font is missing: TX-007 applies.
- Templates built in a portrait project and inserted into a landscape one adapt, because positions are canvas-normalized and sizes are in ref px. Layers that end up off-canvas are allowed and flagged in the inspector.
- Shrinking the group below In + Out is handled as in TX-022.
- Template schema newer than the app: insertion is refused with an explanation.
- User templates may contain only text and shape layers in v1. Templates with media layers come later.
**Undo:** `InsertTemplate`, one compound op made of TL's `InsertClips` (with `created_tracks`) and `Group` (TL-039), reversed as a unit. **Save selection as template…** writes a file to the user template folder and is not a project edit (n/a).
**Model notes:** `TextTemplate { schema, name, group, language: Option<LangTag>, duration, layers: Vec<TemplateLayer { track_offset: u8, start: RationalTime, duration: RationalTime, kind: Text(TextClipData) | Shape(ShapeRef), placeholder: bool, input: InputRule::{Any, Digits, AlphaNumeric}, transform: TransformData, animations: ClipAnimations }> }`. `ShapeRef` is OV's shape type (OV-024), `TransformData` is TR's and `ClipAnimations` is FX's (TX-022). JSON (de)serialization is in `freecut-project`, the insert op is in `freecut-timeline`, and the browser UI is in `freecut-app`. General project templates and stickers are owned by [overlays-and-stickers.md](overlays-and-stickers.md).

### TX-025 Caption tracks and segments — v1
**What the user sees:** A **Caption** track kind in the timeline (TL-004). It has its own header icon and color, and shows segments as compact blocks labeled with their text. Each caption track has a name and a language. **Add caption** (in the Captions panel, or from the timeline track menu) creates a segment at the playhead, and creates a caption track if none exists. The track's eye toggle controls both preview and burn-in, and its lock works as in TL.
- With no caption track yet, the Captions panel lists the ways to start: **Add caption**, **Import subtitles…** (TX-031), **Captions from text…** (TX-030) and, later, **Generate captions from speech…** (TX-038). The Shelf's Captions drawer (TX-057) offers the same starts as side-list rows.
- Later, the context menu of a video or audio clip also offers **Generate captions for this clip** (TX-038, with AI-010's scope set to that clip).
- Pickers that choose a caption track (export, find scope, conversion targets) include **None** where no track is a valid answer.
**Inputs / controls:**
- Manual segment duration: 2 s or up to the next segment's start, whichever is shorter (FreeCut default).
- Track name and language (BCP 47).
- Default track style: Noto Sans Bold, 48 ref px, white fill, black outline at 6%, soft shadow, anchored bottom-center 8% above the canvas's bottom edge, maximum width 80% of the canvas, wrapping on.
**Behavior / output:**
- Segments on one track never overlap, but gaps are allowed.
- Several caption tracks may exist, for example one per language. Only visible tracks render.
- Caption tracks always sit at the top of the visual stack, so burned-in captions are never covered by overlays or altered by effect and adjustment layers. TL-004 defines three visual bands, bottom to top: Video, then Text, Sticker, Effect and Adjustment, then Caption. The Caption band stays on top even when Flexible stacking (TL-007) is on. FX-001 step 2 composites the Caption band after all other visual tracks. Among caption tracks, the timeline order decides. Users who want captions under other layers convert them to text clips (TX-035).
**Edge cases:**
- Segment minimum duration is one frame. Shorter imported cues are extended to one frame, with a warning.
- **Add caption** with less than one frame free before the next segment: refused, with the hint "split instead" (TX-027). The reference editor needs at least 1 s of room; FreeCut needs one frame.
- **Add caption** at or past the end of the timeline: allowed. The segment extends the timeline, because TL-003 counts caption segments in the duration.
- Locked caption tracks reject all edits, including batch styling.
- Ripple edits elsewhere move segments according to TL's ripple rules.
- Deleting the last segment keeps the empty track, because the track carries a name, language, style and rules. TL-006 exempts empty `Caption` tracks from its automatic removal of empty tracks. **Delete track** still removes it.
- Dragging a caption track header: the track moves only among caption tracks, under strict layering and under Flexible stacking alike (TL-007).
- Dropping a text clip, sticker or media onto a caption track: TL-004 redirects it to the nearest compatible track, because a `Caption` track accepts only caption segments. Turning a text clip into captions is an explicit command (TX-035).
- Hidden tracks are not burned in but can still be exported as files (TX-032).
- Removing a caption track whose segments have linked speech clips (TX-043): the linked clips are removed in the same op, as for segment deletion.
**Undo:** `AddCaptionTrack`, `RemoveCaptionTrack` (stores all segments and any linked speech clips it removed), `InsertCaptionSegment`, `SetCaptionRange`.
**Model notes:**
- TL-004 defines `TrackKind::Caption`, which accepts only caption segments. Its band, its top position even under Flexible stacking (TL-007), and its exemption from auto-removal (TL-006) are TL's. TX-025 owns the payload:
  - `CaptionTrackData { name, language: Option<LangTag>, style: TextStyle, layout: CaptionLayout { anchor, offset, max_width: PerMille, align }, animations: ClipAnimations (FX-018, see TX-022), secondary: Option<SecondaryLine>, rules: CaptionRules, segments: Vec<CaptionSegment> }`. Segments are sorted by start and never overlap.
  - `CaptionSegment { id, range: TimeRange, text: RichText, secondary_text: Option<RichText>, style_override: Option<TextStylePatch>, layout_override: Option<CaptionLayoutPatch>, words: Option<Vec<WordTiming>>, speaker: Option<SpeakerId>, origin: CaptionOrigin::{Manual, Imported, Recognized{model, lang}, Lyrics, Script, Translated}, edited: bool }`
  - `edited` is set by any text, range or style op on a segment whose origin is not `Manual`. Generators (TX-038, TX-041, TX-044) use it so they never overwrite captions the user has corrected.
  - Later features extend these structs, and each defines its fields: `CaptionTrackData` gains `speakers` (TX-042), word-state styles (TX-039), the emphasis style (TX-040), `display` (TX-052), the look's variants and decorations (TX-051) and `emoji_settings` (TX-053). `CaptionSegment` gains `secondary_check` (TX-036) and `emoji` (TX-053).
- All of this is pure data in `freecut-timeline`.

### TX-026 Caption editor panel — v1
**What the user sees:** A dockable **Captions** panel that lists the active caption track's segments in a table with these columns: index, start, end, duration, text (inline-editable, multi-line) and a reading-speed indicator. The indicator turns amber when characters per second exceed the track limit (default 20 cps) or a line exceeds the maximum length (TX-037).
- Clicking a row moves the playhead to the segment's start and selects the segment on the timeline. Double-clicking a row's index starts playback from that segment's start.
- During playback the current row is highlighted. Auto-scroll can be toggled.
- A dropdown at the top switches the active caption track.
- A filter box narrows the list to rows whose primary or secondary text matches, with a "No matching captions" empty state. **Find and replace** opens TX-034's bar scoped to this track.
- A toolbar holds **Add caption**, **Clear all captions**, **Duplicate track** and a **Wrap** toggle that switches the track layout's wrapping (TX-025).
- Characters the track font and its fallbacks cannot render (TX-006) are underlined in the cell. Spell checking is out of scope.
**Inputs / controls:**
- Text cells:
  - **Enter** in the middle of the text splits at the caret (TX-027). **Enter** at the very start or end commits the cell and moves to the next row, because a split there would do nothing.
  - **Shift+Enter** or **Ctrl/Cmd+Enter** inserts a line break. Both are accepted; the reference editor uses the second.
  - **Backspace** at the start of the text, with nothing selected, merges the segment into the one above. **Delete** at the end merges the next segment into this one (Fn+Delete on Mac keyboards without a forward-delete key). Both follow TX-027's merge rules.
  - **Esc** cancels the cell edit, and clicking elsewhere commits.
- Start and end cells accept typed timecode.
- Selection works by click, Shift+click (range), Ctrl/Cmd+click (toggle) and select all.
**Behavior / output:** Committed edits update the preview immediately. Typing a time moves that edge of the segment.
**Edge cases:**
- End before start: the edit is rejected, the cell turns red and reverts.
- New end overlapping the next segment: clamped to the next segment's start (default). Holding Alt while committing pushes the next segment instead, if there is room.
- 5,000+ segments: the list is virtualized.
- Empty text: allowed, flagged in the list and skipped on export.
- Locked track: the panel is read-only.
- Panel and timeline selection stay synchronized in both directions.
- A third line typed into a track whose maximum is 2 lines (TX-037): allowed, with the amber warning; re-flow fixes it. The reference editor refuses a third line and asks for a new caption instead.
- **Clear all captions** asks for confirmation with the segment count, and keeps the empty track (TX-025).
- **Duplicate track** copies the segments, style, rules, look and secondary lines into a new caption track named "<name> (copy)", placed just above the original. It is FreeCut's way to keep an alternative version of a caption set. Linked speech clips (TX-043) are not duplicated.
- Backspace merge on the first row, or Delete merge on the last row: nothing happens.
**Undo:** `SetCaptionText { seg, before, after }` and `SetCaptionRange { seg, before, after }`, one per committed cell edit. Keyboard merges are TX-027's `MergeCaptions`. **Clear all captions** is `DeleteCaptions { ripple: false }` over the whole track. **Duplicate track** is `AddCaptionTrack` carrying the copied segments.
**Model notes:** The panel and its view state live in `freecut-app`. Ops are in `freecut-timeline`. The reading-speed check is a pure function in timeline (`CaptionRules::check(&segment) -> Vec<Warning>`).

### TX-027 Split, merge, insert and delete captions — v1
**What the user sees:** In the Captions panel and the caption context menu: **Split at caret** (Enter), **Split at playhead** (the timeline razor on a caption segment), **Merge selected**, **Insert caption before/after**, **Delete**, and **Delete and close gap**.
**Inputs / controls:** The selection, the caret position or playhead, and the merge joiner (a space by default, or a line break when **Keep lines** is on).
**Behavior / output:**
- **Split at caret:** text before the caret stays and the rest moves into a new segment. The split time is the word boundary time if word timings exist (TX-039). Otherwise it is proportional to the non-space character counts of the two halves, rounded to a frame.
- **Split at playhead:** the time is exact. The text split point is estimated from the character proportion and snapped to the nearest word boundary. The new boundary is briefly highlighted so the user can correct it.
- **Merge:** the merged segment runs from the first start to the last end. Texts are joined, the first segment's overrides and speaker are kept, and word timings and secondary texts are concatenated.
- **Insert after:** the new segment starts at the current segment's end and lasts 2 s, or until the next start if that is sooner.
- **Delete** leaves a gap. **Delete and close gap** ripples only that caption track.
**Edge cases:**
- Split with the caret at the very start or end: no-op (in the Captions panel, Enter commits instead, TX-026).
- A split that would create a segment shorter than one frame: refused, with a hint.
- A caret inside a grapheme cluster is snapped to the cluster boundary before splitting.
- Split and merge stay available on tracks with a caption look (TX-051) and on lyric tracks. The look is re-applied to the resulting segments. The reference editor blocks split and merge on its animated caption templates and lyric templates.
- Merging non-adjacent segments (with other segments between them): disabled.
- Merging across a gap: the gap is absorbed.
- Merging segments with different speakers keeps the first speaker and shows a warning.
- Insert when there is no room (gap shorter than one frame): disabled, with the hint "split instead".
- The secondary text (TX-036) is split by the same character proportion. It is marked "check translation".
- Deleting segments that have linked speech clips (TX-043): **Delete** and **Delete and close gap** also delete those clips, in the same op.
- Splitting or merging segments that have linked speech clips: the links follow the rules in TX-043.
**Undo:** `SplitCaption`, `MergeCaptions`, `InsertCaption`, `DeleteCaptions { ripple: bool }`. Each stores the full before and after segment lists for the affected range. `DeleteCaptions` also stores the linked speech clips it removed, so undo restores segments and clips together.
**Model notes:** These are pure ops in `freecut-timeline`, with unit tests for each edge case listed above.

### TX-028 Caption track style and batch styling — v1
**What the user sees:** Selecting a caption track header or any segment shows the text inspector with a **Scope** switch: **All captions on this track** (the default) or **Selected captions only**. Every styling, layout and animation control from TX-004 to TX-016 and TX-022 to TX-023 is available.
- Segments with individual overrides show a dot.
- **Reset to track style** clears the overrides on the selected segments.
- Dragging a caption in the preview moves all captions on the track when the scope is All, or only that segment when the scope is Selected.
**Inputs / controls:** The scope switch, the style controls, and an **Also clear individual overrides** checkbox (default off) when applying a preset to the whole track.
**Behavior / output:** A segment's resolved style is the track style, then the segment override, then the rich-text spans. Spans always win. When speaker labels ship (TX-042), the speaker's style is applied between the track style and the segment override. When caption looks ship (TX-051), the look's variant for the segment applies right after the track style.
- The main line and the secondary line (TX-036) are styled separately. Changes in the inspector's Secondary line section with scope All change the secondary style of every caption and leave the main style alone.
- **Reset track style** restores the default caption track style (TX-025) and removes any caption look (TX-051), after a confirmation. Text, timing and segment overrides are kept.
**Edge cases:**
- Changing the track font while some segments override the font: those segments keep their override, and the indicator dot shows this.
- Selection spanning two caption tracks: scope Selected applies the patch to each segment. Scope All applies to each track's style.
- A segment wraps to more lines than the track maximum (TX-037): it gets a warning badge and is never shrunk automatically.
- Locked track: the controls are disabled.
- Applying to All with **Also clear individual overrides** on: a confirmation states how many segments will lose their overrides.
**Undo:** `SetCaptionTrackStyle { track, before, after }` and `SetCaptionOverrides { segs, before, after }`. Applying a preset with clearing on is one compound op. **Reset track style** is one compound op of `SetCaptionTrackStyle` and, when a look was applied, TX-051's `ApplyCaptionLook`.
**Model notes:** Resolution is `resolve_caption_style(&track, &seg) -> TextStyle`, a pure function in timeline. Rendering asks for the resolved style, per frame and per visible segment.

### TX-029 Caption timing tools — v1
**What the user sees:** A **Timing** menu in the Captions panel that acts on the selection, or on the whole track if nothing is selected.
- **Shift by…:** a signed offset entered as timecode or frames.
- **Change frame rate…:** a source fps and a target fps, with common pairs such as 25 → 23.976 and 23.976 → 25.
- **Two-point sync…:** aligns the first and last selected segments to two playhead positions, with linear mapping in between.
- **Snap to frames:** rounds segment edges to the nearest frame boundary.
- **Fix short captions:** extends segments shorter than the track minimum duration (TX-037, default 0.8 s) into the free space after them.
- **Close small gaps:** closes gaps shorter than N, which defaults to 2 frames.
- **Nudge start/end by one frame:** shortcuts defined in UI.
**Inputs / controls:** As listed above.
**Behavior / output:** Each command produces new ranges for the affected segments in one op.
**Edge cases:**
- A shift that would push a segment before 0: refused. The dialog offers to drop or trim the segments that would fall before 0.
- Rounding after a stretch creates an overlap: the earlier segment's end is clamped to the next segment's start.
- Two-point sync with both source points at the same time: disabled.
- Locked track: refused.
- Segments with word timings: the word timings are transformed with the same mapping.
**Undo:** `RetimeCaptions { track, before: Vec<(SegId, TimeRange, Option<Words>)>, after }`.
**Model notes:** The mapping functions are pure, use rational arithmetic and round half to even. Tests include the 25 ↔ 23.976 round trip.

### TX-030 Script to captions and tap-to-sync — v1
**What the user sees:** A **Captions from text…** dialog, opened from the Captions panel. The same dialog opens as **Long text…** from the Text panel. It is a resizable long-text editor with a character count. The user pastes or types text or opens a `.txt` file, chooses the output, how to split it and how to time it, and then either places it automatically or uses tap-to-sync.
- Output: **Break into captions** (default from the Captions panel) or **Add as one block** (default from the Text panel). One block becomes a single text clip at the playhead (TX-001), or a single caption segment when a caption track is the target.
- Split modes: one caption per line (default), per sentence, every N words (default 6; N = 1 gives one word per caption), or every N characters (default: the track's maximum characters per line, TX-037).
- Timing modes: **Distribute** across a range (the timeline in–out range, the selected clip, or the whole timeline), weighted by character count. Or **Tap to sync**.
- In tap-to-sync, playback starts and the user presses a key at the start of each next caption. Each press ends the previous caption. Backspace steps back one caption, and pause and resume work normally.
- With AI-010's **Align known text** (captions slice CS3), a third timing mode, **Align to speech**, hands the split lines to AI-010's **Align known text** mode in [ai-tools.md](ai-tools.md). If its speech model is not installed, choosing the mode opens the model manager prompt (AI-002).
- From the later tier, a collapsible **Speech** side panel picks a voice and previews it (TX-043, AI-025). With **Also generate speech** ticked, placing the text also starts generating its speech clips.
**Inputs / controls:** Script text (up to 100,000 characters, FreeCut default), output, split mode, N, timing mode, range, and the target caption track (new by default). For Align to speech, also the clip whose audio is aligned.
**Behavior / output:** Segments are created with `origin: Script`. Distribute and tap-to-sync are the non-ML path for typing a known script or song lyrics and syncing it by hand. Align to speech (CS3) times the same lines from Whisper word timestamps and lands through `ReplaceCaptions` (TX-038) with `origin: Script`, as AI-010 defines.
**Edge cases:**
- Empty lines are skipped. Empty or whitespace-only text disables Place, Tap to sync and the Speech panel's preview.
- Pasting past the 100,000-character limit truncates the paste, with a notice.
- Closing the dialog with unplaced text asks whether to discard it.
- Sentence mode on text with no sentence punctuation (for example unpunctuated CJK text): the dialog falls back to the character mode and says so.
- **Add as one block** into a caption segment that breaks the track's line rules (TX-037): the segment is created with the usual warning, and a hint offers re-flow. Nothing is cut. The reference editor truncates such a caption and warns.
- **Add as one block** as a text clip longer than TX-002's 10,000-character limit: refused, with a hint to split into captions or shorten the text.
- Distribute gives a segment less than 0.3 s: a warning is shown and the segment is still created.
- Tap-to-sync reaches the timeline end with lines left: the remaining lines are placed after the last one, 2 s each, and flagged.
- The user stops early: the segments synced so far are kept, and the remainder is dropped unless the user ticks **Append unsynced lines**.
- The script contains SRT-style timecodes: the dialog offers to switch to the import path (TX-031).
- Align to speech cannot place some lines (the text does not match the audio): those lines are listed, placed after the last aligned line at 2 s each, and flagged, as in TX-044.
**Undo:** One `InsertCaptionSegments { track, segs, created_track }` op, created when the dialog or the tap session finishes. Align to speech uses one `ReplaceCaptions` op (TX-038). **Add as one block** is TL's `InsertClips` with one text clip (as TX-001), or `InsertCaptionSegments` with one segment. With **Also generate speech**, the text lands first as its own op, and the speech job lands later as TX-043's `GenerateSpeech` op. Typing in the dialog is not a project edit.
**Model notes:** Splitting and distribution are pure functions in timeline. The tap session lives in `freecut-app` and records timestamps from the playback clock (EX), converted to rational time.

### TX-031 Import subtitle files — v1
**What the user sees:** **Import subtitles…** in the Captions panel, or a subtitle file dragged onto the timeline or panel. A dialog opens with these parts:
- Detected format.
- Text encoding: detected automatically, with an override dropdown and a live preview of the first cues.
- **Place at:** timeline start (default), playhead, or start of the selected clip.
- **Into:** new caption track (default), replace an existing track's contents, or attach as the secondary language of an existing track (TX-036).
- Optional frame-rate conversion.
- An **Ignore styling** checkbox.

After import, a summary reports counts: imported cues, skipped cues (with line numbers), cues moved to overflow tracks, and stripped tags.
**Inputs / controls:** Supported formats:
- **SRT:** `<b>`, `<i>` and `<u>` tags, and `<font color>`.
- **WebVTT:** cue timings. Cue settings for line, position and align are mapped approximately to the layout override. `<b> <i> <u>` tags are supported. `<v Name>` maps to the speaker. NOTE, STYLE and REGION blocks are skipped and reported.
- **ASS/SSA:**
  - Styles: the most-used style becomes the track style, and other styles become segment overrides. Font size is converted using the file's script resolution.
  - Dialogue events become segments, and comment events are skipped.
  - Supported override tags: bold, italic, underline, strike, colors, font name, font size, alignment, and hard and soft line breaks. Other tags are stripped and counted.
  - Karaoke tags become word timings (TX-039).
- **TXT with timecodes:** SRT-like blocks without index numbers.
- **LRC:** line timestamps. Metadata tags are skipped, and the offset tag is applied. Enhanced word timestamps become word timings. A line ends where the next one starts, and the last line lasts 4 s (FreeCut default).
- **SBV.**
- TTML/DFXP, SCC and EBU-STL are not supported (see Out of scope).
- Several files can be dropped or picked at once. Each goes into its own new caption track, and the summary lists them per file (FreeCut default). The reference editor takes one file per import.
- Files over 64 MB are refused as unlikely to be subtitles (FreeCut default). The reference editor refuses files over 3 MB; FreeCut parses large files in the background with progress instead.
**Behavior / output:** The file is read and decoded on a background thread. Encoding is detected from the byte-order mark (UTF-8 or UTF-16), then by checking whether the bytes are valid UTF-8, then with a legacy-encoding detector. Cue times in milliseconds are converted to rational time exactly when the project timebase allows. Otherwise they are rounded to the nearest tick, half to even. Times are not snapped to frames unless the user asks (TX-029).
**Edge cases:**
- A malformed timing line skips that cue and is reported with its line number. Missing SRT index numbers are fine.
- End before start, or zero length: the cue is skipped and reported.
- Overlapping cues, as in some SRT files and ASS layers, go to extra caption tracks, one per overlap depth. The summary lists them.
- An empty or unrecognized file shows an error ("check the file and try again") and makes no change.
- Text without any timing (a TXT file with no timecodes, or an LRC file with no timestamps): the dialog offers the TX-030 path instead.
- Untimed lines inside an otherwise timed LRC file follow the previous line, 2 s each (FreeCut default), and are flagged in the summary.
- 50,000+ cues: parsing shows progress, and insertion is still one op.
- HTML entities and non-breaking spaces are decoded. Mixed line endings are tolerated.
- A UTF-8 byte-order mark never shows up as a stray character in the first cue.
- A file timed for a timeline that started at a non-zero timecode (for example, 01:00:00:00): the dialog detects that the first cue is after one hour and offers to subtract the hour.
**Undo:** `ImportSubtitles { created_tracks, inserted, replaced: Option<Vec<CaptionSegment>> }` is one compound op.
**Model notes:** Parsers live in `freecut-project::subtitles` as pure functions, `parse(&str, Format) -> Result<SubtitleDoc, ParseError>` plus a `ParseReport`. They do no I/O, so fixture tests are easy. File reading and encoding detection run on a background job in the app. Converting a `SubtitleDoc` into timeline ops is pure.

### TX-032 Export subtitle files — v1
**What the user sees:** **Export subtitles…** in the Captions panel. The same options appear as **Also save caption file** in the export dialog (EX-045).
**Inputs / controls:**
- Track or tracks.
- Format: SRT (default), WebVTT, ASS, plain-text transcript (TXT, with optional timestamps), or LRC.
- Range: whole timeline, or in–out range, with an option to rebase times to the range start.
- Bilingual content: primary only, secondary only, or both as two lines.
- Encoding: UTF-8 without byte-order mark (default) or with one.
- Line endings: CRLF (default for SRT) or LF.
- **Include style tags**: default off for SRT and VTT, on for ASS.
- **Include plain text clips as captions**: default off. When on, text clips in the range are converted into cues.
- **Export screens as separate cues** (later, with TX-052): default off. When on, a track with progressive display writes one cue per screen instead of one per segment.
**Behavior / output:**
- Segments are sorted and renumbered from 1. Times are rounded to milliseconds, half up. Empty segments are skipped.
- ASS export writes:
  - the script resolution as the project canvas;
  - a style built from the track style, with ref px converted to canvas px, colors converted to ASS color order, outline and shadow, numpad alignment and margins taken from the layout;
  - segment overrides as override tags;
  - fade presets as the ASS fade tag. Other animations are dropped.
- Writing goes to a temporary file in the target folder, which is then renamed.
**Edge cases:**
- File names depend on where the export starts:
  - **Export subtitles…** in the Captions panel names files after the project: `<project>.<track-language-or-name>.<ext>`. With several tracks, one file is written per track into the chosen folder.
  - Sidecars from the export dialog are named after the video file stem, `<video file stem>.<lang-or-track>.<ext>`, and are written next to the video (EX-045). Name collisions follow EX-045.
- Segments partly outside the range are trimmed, and segments fully outside are dropped.
- Rounding makes end equal to start: end is moved 1 ms later.
- Hidden tracks can be exported.
- Write failure (permissions, disk full): an error is shown and no partial file remains.
- Export is entirely local and never needs a network connection. The reference editor can fail caption export on network errors.
- ASS export names fonts but does not embed them. A note lists fonts that a player on another computer may lack.
- Caption emoji (TX-053) and look decorations (TX-051) are not written to files; they appear only in burned-in video.
- Captions that were typed, imported or scripted export just like recognized ones. The reference editor exports only captions it recognized automatically. FreeCut lifts that restriction.
**Undo:** n/a (file output).
**Model notes:** Writers live in `freecut-project::subtitles` (`write(&SubtitleDoc, Format, WriteOptions) -> String`). Building the `SubtitleDoc` from the timeline is pure. File I/O runs on a background thread.

### TX-033 Captions in the exported video — v1
**What the user sees:** A **Captions** section in the export dialog (owned by [preview-and-export.md](preview-and-export.md)). Each caption track gets one of: **Burn into video** (default for visible tracks), **Embed as subtitle stream**, or **Don't include**. The section also offers the sidecar file option from TX-032, with files named after the video as EX-045 defines.
**Inputs / controls:** Per-track mode. For embedded streams: the language tag (from the track) and a default-track flag, which goes to the first embedded track.
**Behavior / output:**
- **Burn-in** renders captions into the frames with full styling.
- **Embed** muxes a text subtitle stream:
  - MP4 and MOV: the plain-text subtitle codec those containers support. Styling is lost.
  - MKV: SRT, or ASS to keep styling.
  - WebM: WebVTT.
- Plain text clips are always rendered as graphics.
**Edge cases:**
- A container that cannot carry subtitles (for example, an animated GIF, or audio-only export): Embed is disabled, with a tooltip.
- Burn-in and Embed together: allowed. A note warns that captions may appear twice in players.
- Hidden tracks are never burned in.
- Bilingual tracks: embedded streams carry both lines as two lines.
- Range exports rebase the stream times.
- Per-range styling is lost in plain-text codecs. A note lists what is dropped.
**Undo:** n/a. The choices are export settings stored with the project, as EX defines.
**Model notes:** The timeline provides resolved cues. `freecut-media` muxes subtitle streams through FFmpeg's subtitle encoders. Burn-in goes through the normal `freecut-render` path.

### TX-034 Find and replace in text and captions — v1
**What the user sees:** **Find** (Ctrl/Cmd+F while the Captions panel or a text clip has focus) opens a bar with these parts:
- Find and Replace fields.
- **Match case** and **Whole word** toggles.
- **Scope:** current caption track, all caption tracks, all text clips, or everything.
- **Include secondary lines**, default on.
- A result count, Previous and Next (which move the playhead to the match), **Replace** and **Replace all**.
**Inputs / controls:** As listed above. Regular expressions come later.
**Behavior / output:** Matching is Unicode-aware. It uses NFC-normalized, case-folded comparison unless Match case is on. Matching runs on the stored content, so the display case transform has no effect.
**Edge cases:**
- Matches that cross style spans are allowed. The replacement takes the first character's style (TX-015).
- An empty replacement deletes the matched text. Segments left empty are kept and flagged.
- Locked tracks are skipped and reported ("N matches on locked tracks were not changed").
- Searching 5,000 segments must not block the UI: it runs incrementally with a result cap shown.
**Undo:** `ReplaceText { edits: Vec<(ItemId, RichText before, RichText after)> }`. Replace all is one op.
**Model notes:** Matching and replacing are pure functions in timeline over `RichText`. The search bar is in the app. Spell checking (open dictionaries) comes later.

### TX-035 Convert between text clips and captions — v1
**What the user sees:** In context menus: **Convert to captions** on selected text clips, and **Convert to text clips** on selected caption segments.
**Inputs / controls:**
- Convert to captions: the target caption track, either new or existing.
- Convert to text clips: **Keep caption segments**, default off.
**Behavior / output:**
- **To captions:** each text clip becomes a segment with the same time range and text. Style differences from the track style become segment overrides. Position becomes a layout override. Animations are copied.
- **To text clips:** each segment becomes a free text clip on a `Text` track (TL-004), which TL's placement rules pick or create as in TX-001. It gets the resolved style and an absolute position, and the segments are removed unless Keep is on.
**Edge cases:**
- Overlapping text clips go to separate caption tracks, one per overlap depth.
- Text clips with rotation, scale or keyframes: a confirmation lists what will be lost, because captions only have position.
- Locked source or target track: refused.
- Text clips inside a template group: the clip is converted and the group membership is dropped, with a warning.
**Undo:** A single compound op (`ConvertTextToCaptions` or `ConvertCaptionsToText`) with full before and after state.
**Model notes:** Pure ops in timeline that use `resolve_caption_style` (TX-028).

### TX-036 Bilingual captions (secondary line) — v1
**What the user sees:** A caption track can show a second line under, or over, each caption. Typical uses are a translation or a phonetic reading. The Captions panel gains a second text column. The inspector has a **Secondary line** section with its own style and placement. A **Swap primary/secondary** command is available.
**Inputs / controls:**
- Secondary text, typed manually, imported (TX-031, "attach as secondary"), or produced by translation (TX-041, later). The Captions drawer's **Second line** choice (TX-057) picks the source when captions are generated; the engines and their order are listed in the [captions build](#captions--speech-build--scope) decisions.
- Secondary style: by default the same font as the primary line at 75% size, with a light-gray fill.
- Placement: below (default) or above.
- Gap: 0–100% of the primary size, default 10%.
- **Display:** **Both lines** (default), **Primary only**, or **Secondary only** (for example a translation-only version of the video).
**Behavior / output:** Both lines share the segment's timing and are laid out as one stacked block. Each line wraps on its own. Secondary only lays out the secondary line in the primary line's position, with the secondary style.
**Edge cases:**
- Attaching an imported file: each incoming cue attaches to the existing segment it overlaps most. Cues with no overlap attach to the nearest segment midpoint within 0.5 s, or are reported as unmatched and dropped.
- Segments with no secondary text show only the primary line.
- Animations (TX-022 and TX-023) run on both lines at the same time.
- Split and merge carry the secondary text as described in TX-027.
- Mixed directions (for example, an RTL secondary under an LTR primary) are each laid out correctly.
- Export choices are described in TX-032 and TX-033.
- Not enough room for both lines (the stacked block would leave EX-011's title-safe area): the segment gets a warning badge. Nothing is shrunk or dropped automatically.
- Editing the primary text of a segment that has secondary text marks the secondary line **check translation** (shown in the Captions panel), as split does (TX-027). TX-041's **Update translations** clears the mark. The reference editor asks on every edit whether to update the second line.
- The track language and the secondary language can be changed at any time. Changing them never re-translates. The reference editor locks the main language after its translation is generated.**Undo:** `SetCaptionText` with a field selector (primary or secondary), `SetSecondaryStyle` (style, placement, gap and display), and `SwapCaptionLines` (an involution, so applying it again reverses it). Setting or clearing **check translation** is part of the op that caused it.
**Model notes:** `secondary_text: Option<RichText>` and `secondary_check: bool` per segment. `SecondaryLine { style: TextStyle, placement, gap: PerMille, display: LineDisplay::{Both, PrimaryOnly, SecondaryOnly} }` per track.

### TX-037 Caption line rules and re-flow — v1
**What the user sees:** Track settings for caption rules, live warnings in the Captions panel, and a **Re-flow captions** command for the selection or the whole track.
**Inputs / controls:** Per-track `CaptionRules`. This is the single definition of caption line rules; the Auto captions dialog (AI-010 in [ai-tools.md](ai-tools.md)) and translation (AI-013) read and edit these same rules. All values are FreeCut defaults.
- Maximum characters per line, counted in grapheme clusters: 10–80, default 42 for landscape and square canvases and 24 for portrait. For CJK track languages the default is 16.
- Maximum lines: 1–3, default 2.
- Maximum duration: 1–10 s, default 6 s.
- Minimum duration: 0.3–3 s, default 0.8 s.
- Break at sentence punctuation: default on.
- Maximum reading speed: 10–40 characters per second, default 20. This produces warnings only.
**Behavior / output:**
- Re-flow re-breaks lines into balanced lengths. It prefers punctuation, then spaces, and never breaks inside a word. CJK text may break between characters.
- Segments that still exceed the line or duration limits are split. Time is divided by word timings when they exist, otherwise in proportion to characters.
- **Keep manual line breaks** is an option, default off.
**Edge cases:**
- A single word longer than the limit goes on its own line.
- Per-range spans are preserved through re-flow.
- Bilingual segments: split points come from the primary line, and the secondary line is re-broken on its own.
- Rules apply to recognition output (TX-038) automatically.
- The rules warn and re-flow but never block typing. The reference editor hard-limits a caption to 2 lines at entry.
- Changing the canvas from landscape to portrait (PM-011) does not rewrite existing rules. Only new tracks get the portrait default.
**Undo:** `ReflowCaptions { track, before: Vec<CaptionSegment>, after }`. Editing the rules themselves is `SetCaptionRules { track, before, after }`.
**Model notes:** `CaptionRules` and `reflow()` are pure in timeline, with golden tests per script (Latin, CJK, Arabic).

### TX-038 Recognized captions from speech — v1
**What the user sees:** The Captions panel has a **Generate captions from speech…** entry. It opens the Auto captions dialog of [ai-tools.md](ai-tools.md) (AI-010, which runs transcription from AI-009 when needed). When the job finishes, the result appears as an ordinary caption track named "<Language> (auto)" in the default caption track style (TX-025). Its segments can be edited, styled, split, retimed and exported like typed ones. Segments that are still exactly as generated show a small "auto" marker in the Captions panel.
**Inputs / controls:**
- The dialog (scope, language, model quality, speech or song-lyrics source, filler marking, keyword emphasis, output mode) and its defaults are defined by AI-010.
- The line rules shown in that dialog are the target track's `CaptionRules` (TX-037). A new track starts with the TX-037 defaults.
- Output target: a new caption track, or replacing earlier auto captions on an existing track, as chosen in AI-010's output setting.
- TX adds one control to AI-010's dialog, **Caption style** for the receiving track: the track default (TX-025), a style preset or saved style (TX-019, TX-020), or a caption look (TX-051). The default is the receiving track's current style, so new captions arrive already styled.
**Behavior / output:**
- Recognition works per media file, not on a rendered mix of the timeline. AI-009 transcribes each media file once, and AI-010 maps the words of each clip's used source range onto the timeline through that clip's time map. The same transcripts also feed transcript editing (AI-011). TX never sees audio.
- AI-010 hands TX a list of caption drafts. Each draft has text, a timeline range, optional word timings, optional filler and keyword tags, and the detected language.
- TX turns the drafts into `CaptionSegment`s with `origin: Recognized { model, lang }`, `edited: false`, and `words` filled when timings exist (TX-039).
- The drafts pass through TX-037 re-flow, so the result obeys the track's rules.
- Filler tags become rich-text spans with `SpanOrigin::Filler`, so they can be styled, or removed by AI-012. Keyword tags are applied as in TX-040. *FreeCut decision (captions build, 2026-10-13):* until TX-015's spans exist, filler tags are stored as `WordFlags::FILLER` on the segment's word timings and drawn with the look's Filler word style; with the drawer's **Leave them out** choice the filler words are removed from the drafts before re-flow.
- A new track takes its language tag from the detected language.
- In replace mode, only segments in scope with origin `Recognized` and `edited: false` are replaced. Typed, imported, scripted, translated or corrected segments are never replaced.
- Re-running recognition into the same track keeps the track's style, rules, look, animations and secondary-line settings. Before the job starts, a confirmation states how many segments will be replaced. The user can re-run with another language when detection was wrong.
- Captions created or refreshed from the transcript panel (AI-011) land through the same op and the same rules.
**Edge cases:**
- No length cap: AI-009 processes long media in chunks. The reference editor refuses media over 2 hours and asks the user to split it first.
- A custom word list that biases recognition would be an input to AI-009's recognizer, not a TX setting. TX-040's keyword list only drives emphasis.
- Cleaning up the wording of generated captions is AI-012's filler and pause cleanup. Spell checking is out of scope.
- The reference editor locks its interface while many caption clips are added. FreeCut applies the whole result as one op on the model, so nothing is locked.
- Target track locked: Generate is disabled for that track, with a hint.
- A draft shorter than one frame after mapping to timeline time: extended to one frame (TX-025).
- Overlapping drafts (two voices at the same time on different tracks): when AI-010's overlap option asks for a second track, the overlapping drafts go to an extra caption track, as TX-031 does for overlapping cues. Otherwise AI-010 has already shortened them.
- In replace mode, a draft that collides with a kept manual segment: the draft is trimmed to the free space. If less than one frame remains, it is dropped and listed in AI-010's report.
- A draft that is empty after the user chose to drop filler words: skipped.
- The timeline changed while the job ran: AI-001 and AI-010 decide whether to apply. TX applies only what it receives, as one op.
**Undo:** `ReplaceCaptions { track, created_track: Option<TrackId>, removed: Vec<CaptionSegment>, inserted: Vec<CaptionSegment> }`, one compound op. This is the canonical op for every generated caption result. AI-010's auto captions, its **Re-sync captions** and its **Align known text** mode (TX-030, TX-044) all land through it, and [ai-tools.md](ai-tools.md) refers to it by this name. Undo restores the removed segments and deletes a track the op created.
**Model notes:**
- `CaptionDraft` is plain data defined in `freecut-timeline` (text, range, word timings, tags, language), so `freecut-analysis` can produce it while the timeline depends on nothing internal.
- `ReplaceCaptions`, the draft-to-segment conversion and the re-flow are pure code in `freecut-timeline`, with unit tests for the replace-only-unedited rule.
- Transcription, models, the job system and the model manager belong to AI (`freecut-analysis`; AI-001, AI-002, AI-009, AI-010). The per-media `Transcript` is AI-009's. TX has no ML dependency.

### TX-039 Word timings and active-word highlight — v1
**What the user sees:** For segments that have word timings (from recognition, ASS karaoke tags or enhanced LRC), the caption style gains an **Active word** sub-style. Its options are a color, a scale and a highlight pill behind the word. Two more sub-styles cover the other word states: **Spoken** (words already said) and **Upcoming** (words not yet said), each with a color and an opacity. A **Reveal words as spoken** option is also available. Animations that play when a word or line changes state belong to caption looks (TX-051). The timeline shows small ticks at word boundaries. Word ticks can be dragged in the timeline to correct them (later polish).
**Inputs / controls:** All FreeCut defaults:
- Active-word color: a warm yellow, `#FFC83D`.
- Scale: 100–150%, default 115%.
- Pill: off by default. Color default black, opacity 0–100% (default 70%).
- Reveal toggle: default off.
- Spoken and Upcoming: color unset by default (the track fill applies), opacity 0–100%, default 100%. With these defaults the two states look like plain text until the user sets them.
**Behavior / output:** At each frame every word is in exactly one state: **Active** (the last word whose span started at or before the frame), **Spoken** (words before the active one) or **Upcoming** (words that have not started). Each state's sub-style applies on top of the resolved segment style, and keyword emphasis (TX-040) applies on top of all three. In reveal mode, Spoken and Active words are visible and Upcoming words are hidden.
**Edge cases:**
- Between two words (a pause): the previous word stays active until the next one starts (FreeCut default), so the highlight does not flicker. It becomes Spoken only once the next word becomes active. Before the first word starts, every word is Upcoming.
- Editing text after timings exist: the old and new word sequences are aligned. Unchanged words keep their times, and changed or new words get interpolated times.
- Split and merge carry the timings (TX-027).
- Word times outside the segment range are clamped.
- The scaled active word would overflow the line: the line is laid out once at rest size, and the scaled word may overlap its neighbors slightly, so the text never reflows while playing.
- No timings: the controls are disabled, with an explanation and a pointer to Auto captions (AI-010).
- Tap-to-sync (TX-030) could record word timings in a later version.
**Undo:** Style changes are `SetCaptionTrackStyle`. Timing edits are `SetWordTimings { seg, before, after }`.
**Model notes:** `WordTiming { range: TimeRange, text_range: Range<u32> }` lives in timeline. The track stores `WordStateStyles { active: ActiveWordStyle, spoken: WordStateStyle, upcoming: WordStateStyle, reveal: bool }`, where `WordStateStyle { color: Option<Rgba8>, opacity: PerMille }`. The state of a word at a time is a pure function in timeline. The style evaluation is in `freecut-render::text`.

### TX-040 Automatic keyword emphasis — later
**What the user sees:** **Emphasize keywords** on a caption track, or on selected segments, applies an emphasis style to chosen words. The emphasis style defaults to a contrasting color and +10% size (FreeCut default). There are two ways to choose words:
- **Rules:** numbers, words from a user keyword list, and ALL-CAPS words.
- **Automatic:** the local keyword heuristic that AI-010 defines for its Emphasize keywords option. It skips function words, prefers numbers, capitalized names and rare words, and caps the density. TX-040 runs the same heuristic, so the result is the same whether emphasis was chosen during auto captions or later.

The user can also mark words by hand: **Mark as keyword** and **Unmark** on selected words in a caption cell or in on-canvas edit mode. **Clear automatic emphasis** removes only detected keywords, and **Clear all emphasis** removes manual marks too. Detection and applying show progress in the background-task list for large tracks.
**Inputs / controls:** Mode, keyword list, and the emphasis style (color, size change, optional bold). The density cap of the Automatic mode is AI-010's.
**Behavior / output:** Detected emphasis is written as rich-text spans with `SpanOrigin::AutoEmphasis`, so **Clear automatic emphasis** and re-running affect only those spans. Hand-marked keywords are spans with `SpanOrigin::ManualEmphasis`, which re-running never touches. Other manual spans are kept. Keyword tags that arrive with auto captions (TX-038) are written as `AutoEmphasis`. Both emphasis origins take the track's emphasis style, so changing that style updates every keyword span on the track in one op.
**Edge cases:**
- Detection finds no keywords: a message, and no change.
- **Emphasize keywords** in Rules mode with an empty keyword list and every rule off: the Apply button is disabled.
- Re-running detection never removes hand-marked keywords. The reference editor replaces hand-picked keywords with detected ones, after a warning.
- Moving keywords onto their own line or screen is a progressive-display option (TX-052).
- Languages without spaces between words (CJK): only the numbers and keyword-list rules work, unless a word segmenter is available (for example the ICU4X segmenter, Unicode-3.0 license).
- Re-running after text edits recomputes the automatic spans.
- An automatic span that crosses a manual span: it is cut at the manual span's edges, and the manual style wins.
- Emphasis inside a word that also has the active-word style: the active style wins while the word is active.
- Locked track: refused.
**Undo:** A compound `SetRichText` over the affected segments, one per command (apply, mark, unmark, clear, change of emphasis style).
**Model notes:** `SpanOrigin::{Manual, ManualEmphasis, AutoEmphasis, Filler}` is stored on each span (TX-015). The emphasis style is a `TextStylePatch` on `CaptionTrackData`. The Rules mode is a pure function in `freecut-timeline`. The Automatic mode calls AI's heuristic in `freecut-analysis`, which needs no model. The app runs it and applies the result as one op.

### TX-041 Translated captions — later
**What the user sees:** **Translate captions…** on a caption track opens the translation dialog of [ai-tools.md](ai-tools.md) (AI-013). TX defines where the result goes. AI-013 offers three outputs:
- **New caption track** (AI-013's default): the source track's timing and style, named "<Language> (translated)" and tagged with the target language.
- **Bilingual:** the translation becomes each segment's secondary line (TX-036), and the track's secondary display is set to Both lines.
- **Replace text:** the primary text is replaced.
**Inputs / controls:** Target language, source language, output mode and whether the line rules are applied are defined by AI-013. The rules themselves are the track's `CaptionRules` (TX-037).
**Behavior / output:**
- AI-013 translates segments in context windows. It owns the window size and the rule that splits each window's translation back onto its segments. TX receives exactly one translated text per source segment and never changes timing.
- Translated segments keep the source segment's range, speaker and whole-segment overrides. New or changed segments get `origin: Translated` and `edited: false`.
- Per-range spans are not carried over, because word positions change. Automatic emphasis can be re-run (TX-040).
- Word timings are dropped for translated text, because they no longer match the words. The active-word highlight is off for those segments.
- When AI-013's rule option is on, the text is re-wrapped with TX-037 re-flow. Over-long results get warnings and are never shortened automatically.
- **Update translations** on a bilingual track re-translates only the segments marked **check translation** (TX-036) and clears the mark. It runs the same AI-013 job and lands as one op. The reference editor can re-translate automatically after each edit; FreeCut keeps that a deliberate command, so background jobs never change the project unasked.
**Edge cases:**
- Translation fails for some segments: the others land, and the failures are listed. On a bilingual track the failed segments keep **check translation**, so **Update translations** retries them.
- Bilingual output works while filler cleanup (AI-012) is in use. The reference editor does not allow bilingual captions together with its filler-word removal.
- Bilingual output when segments already have secondary text: the user is asked whether to overwrite. Secondary texts that were edited by hand (`edited`) are listed separately.
- Replace text: the original text can only be recovered with undo, and the dialog says so.
- Segments with empty text are skipped.
- RTL target language: layout follows TX-006, and a secondary line uses its own direction (TX-036).
- Locked target track: refused before the job starts.
**Undo:** One op: `ReplaceCaptions` (TX-038) for a new track, or `TranslateCaptions { track, field: Primary | Secondary, before, after }` for bilingual and replace output and for **Update translations**.
**Model notes:** Merging the translated texts into segments is a pure op in `freecut-timeline`. Its input is one text per segment id, so it needs no knowledge of context windows. Translation engines, models, context windows and split-back belong to AI-013 (`freecut-analysis`). The models are listed with their licenses in its license table.

### TX-042 Speaker labels — later
**What the user sees:** Each caption segment can have a speaker (Speaker 1, Speaker 2, and so on, renameable), set from the Captions panel or the segment context menu. The track style can set per-speaker overrides such as fill color. An export option prefixes names (SRT and TXT), writes voice tags (WebVTT) or fills the ASS name field.
**Inputs / controls:** Speaker list per track (name and color), the per-segment speaker, and the export option **Include speaker names** (default off).
**Behavior / output:**
- Speakers are assigned by hand, or filled automatically from imported WebVTT voice tags and ASS name fields (TX-031), without ML. Import stores the speaker field from v1, so no data is lost before this UI ships.
- A segment's resolved style becomes track style, then speaker style, then segment override, then spans (extending TX-028).
- Automatic speaker detection (diarization) is not offered, because [ai-tools.md](ai-tools.md) lists it as out of scope. If AI adds it later, it would fill speakers through the same op.
**Edge cases:**
- A segment that covers two speakers: the user splits it (TX-027) and assigns each part.
- Merging segments with different speakers keeps the first speaker (TX-027).
- Deleting a speaker that segments use: after a confirmation showing the count, those segments become unassigned.
- Imported voice tags with the same name map to one speaker. Two speakers given the same name by hand stay distinct by id.
- Exporting to formats with no speaker field (LRC): names are dropped unless the prefix option is on.
**Undo:** `SetCaptionSpeakers { segs, before, after }` and `SetSpeakerTable { track, before, after }`.
**Model notes:** `SpeakerId`, plus `speakers: Vec<Speaker { id, name, style: TextStylePatch }>` on `CaptionTrackData`. Pure data in timeline. `resolve_caption_style` (TX-028) gains the speaker step.

### TX-043 Speech from text and captions — later
**What the user sees:** **Speak text…** on selected text clips or caption segments, **Speak all text clips** in the Text panel's menu, and **Read all captions** on a caption track. TX-030's long-text dialog embeds the same controls as a side panel. Each command opens the voice dialog of [ai-tools.md](ai-tools.md) (AI-025), which owns the voice list, voice preview and synthesis. This feature owns the speech controls listed below and how the result lands. Generated audio clips show an **out of date** badge with **Regenerate** when their source text changes.
- *Speech card:* the Content lens of a text clip has a **Speech** card (TX-054, ui-concept §7). It embeds the same controls inline: the voice list from AI-025 (installed local voices only, with **Preview**), the controls below, and **Speak text**. Once the clip has linked speech, the card shows the linked clip's name, its state (up to date or out of date), **Regenerate**, **Select speech clip** and **Unlink**. Without any installed voice the card explains the local engines and opens AI-002's model manager; nothing is downloaded without that explicit step.
- Every engine runs on the user's machine (AI-025). There is no server voice, no voice catalogue to download per use, and no character quota.
**Inputs / controls:** All FreeCut defaults.
- Source: the selected text clips, all text clips in the sequence, the selected caption segments, or all captions on a track.
- Speed: 0.5–2.0×, default 1.0×, step 0.05. It is passed to the engine as the speaking rate, so pitch is not affected.
- Pitch: −12 to +12 semitones, default 0. It is written to the generated clip as AU-024's pitch shift, applied after synthesis.
- Volume: the generated clip's initial volume, in AU-005's range, default 0.0 dB.
- **Fit to caption** ("match text duration to speech"): default off.
- **Keep in sync**: default off.
- Clips: **One clip per text clip or caption** (default) or **One merged clip**.
- **Use this voice for all**: applies the chosen voice and settings to every source in the run, instead of asking per language.
- **Read secondary line instead**: default off, for bilingual tracks (TX-036).
- Size limit: 10,000 characters per generated clip, the TX-002 limit. A run may cover any number of clips. There is no quota of any kind.
**Behavior / output:**
- The text handed to the speech engine is the stored content (TX-002). The display case transform (TX-008) is not applied, line breaks become spaces, and style spans are ignored.
- For caption segments, the primary line is read unless the option above is set.
- AI-025 splits long text into sentences at full stops, question marks, exclamation marks and their CJK equivalents, never at commas. One sentence per line reads most naturally, and the dialog says so.
- Each generated clip is placed at its source's start on an audio track picked by TL's placement rules, and linked to its source (TL-011). The source text clip or segment shows a small speaker icon.
- When **Fit to caption** is on, TX applies it to captions by moving the segment's end. The end never moves past the next segment's start. A segment that cannot fit keeps the longest possible range and is flagged in the Captions panel.
- Each generated clip stores a hash of the text TX handed over. Any TX op that changes that text (TX-002, TX-008's Change case, TX-015, TX-026, TX-034, TX-041) makes the hash stale, which shows the out-of-date badge.
- With **Keep in sync** off, committing a text edit on a voiced source shows a toast offering **Regenerate now**. With it on, the clip is regenerated in the background after the edit commits (AI-001), and the new audio lands as its own undo entry, "Regenerate speech".
**Edge cases:**
- Empty segments, or segments with only emoji: skipped and listed. A run with no readable text at all: Generate is disabled.
- **One merged clip** over the 10,000-character limit: refused, with a hint to generate one clip per caption.
- A voice whose language does not match the text: allowed, with AI-025's hint. The reference editor lets such a voice be previewed but not used.
- **Keep in sync** with the speech model removed: the clip stays out of date, and the badge explains why.
- The reference editor caps characters per request and per month and charges for some voices. FreeCut has no caps beyond the size limit above.
- A segment split or merged (TX-027) after speech was generated: the linked clips stay and are marked out of date. After a split, the link stays with the left part, which keeps the segment id. After a merge, the links of the absorbed segments move to the merged segment.
- Deleting a caption segment deletes its linked speech clip in the same op (`DeleteCaptions`, TX-027). Removing the whole caption track does the same (TX-025). Undo restores both.
- Deleting the speech clip alone: the segment stays, and the link is dissolved by TL-011's rules.
- Locked caption track with **Fit to caption** on: segment ranges are not changed, and the dialog warns before generation.
- Locked target audio track: TL's placement rules pick or create another audio track.
**Undo:** `GenerateSpeech { inserted_clips, links, range_changes }`, owned here and referenced by AI-025, one per run. When **Fit to caption** moves segment ends, the op includes TX's `SetCaptionRange` changes. Undo removes the clips and links and restores the ranges. **Regenerate** swaps the clip's media through TL-035's `ReplaceSource`, keeping its position, volume and fades.
**Model notes:** Text extraction (`speech_text(&item) -> String`) and the text hash are pure functions in `freecut-timeline`. Speech clips are tied to their source through TL-011's links (`LinkId`). Link members are `LinkMember::Clip(ClipId)` for the speech clip and for a source text clip, and `LinkMember::CaptionSegment(TrackId, SegId)` for a source caption segment. How the members are stored is TL-011's. The run's settings are `SpeechSettings { speed: PerMille, pitch: Cents, volume_db, fit_to_caption: bool, keep_in_sync: bool, merge: bool, read_secondary: bool }`, kept per user as the last-used values and stored on each generated clip so **Regenerate** repeats them. Synthesis, voices, sentence chunking and WAV writing belong to AI-025 (`freecut-analysis`, `freecut-media`).

### TX-044 Lyric captions — v1
**What the user sees:** Three ways to put lyrics on a caption track:
- **From the music:** the Auto captions dialog (AI-010) with source **Song lyrics**, which isolates the vocals (AI-023) before recognition.
- **Align pasted lyrics:** the user pastes the lyrics, and AI-010's **Align known text** mode ([ai-tools.md](ai-tools.md)) times each line against the selected music clip, with word timings when possible. It uses Whisper word timestamps, with MIT-licensed weights only.
- **Without ML (v1):** LRC import (TX-031), pasting the lyrics and using tap-to-sync (TX-030), or typing them by hand in the Captions panel (TX-026).

A lyric track is an ordinary caption track. Its secondary line (TX-036) suits a translation or a romanized reading of each line, and main and secondary lines are styled separately for the whole track (TX-028). The track menu adds **Delete all lyrics**, which removes every segment whose origin is `Lyrics` on that track, and the ML dialogs offer **Replace existing lyrics** (default on when the target track already holds lyrics).

The two ML paths produce segments with `origin: Lyrics`. The non-ML paths keep their own origins: `Imported` for LRC (TX-031) and `Script` for tap-to-sync (TX-030). In every case the default style is the caption track default (TX-025) with the active-word highlight (TX-039) switched on when word timings exist.
**Inputs / controls:** For the ML paths, AI-010's controls; choosing the language is recommended for singing. For alignment: the pasted text, the music clip, and line splitting as in TX-030 (one caption per line by default). The alignment controls themselves are AI-010's.
**Behavior / output:**
- Each segment is one sung line. Recognized lyrics are split at pauses by AI-010. Aligned lyrics keep the user's own lines.
- Word timings, when present, drive karaoke-style highlighting (TX-039).
- *Music (captions build, 2026-10-13):* the recognizer runs with a lyrics profile: the vocals are separated first when AI-023's pack is installed (otherwise the form offers to install it or to try without, with a note that accuracy drops), voice activity uses a lower threshold, and a pause of 300 ms or more ends a line, so each caption is one sung line. Voice activity under 3 % of the clip gives "No vocals detected". Lines whose words were recognized with low confidence are flagged for review in the caption list. Details: [speech-and-captions.md §5.5](../architecture/speech-and-captions.md#55-music-and-lyrics).
- Results of both ML paths are applied through `ReplaceCaptions` (TX-038) with `origin: Lyrics`. The same Align known text mode, started from TX-030 for a spoken script, lands with `origin: Script` instead.
**Edge cases:**
- Instrumental or very noisy music: "No vocals detected", and no track is created.
- Repeated choruses are recognized independently. In alignment mode, repeated lines in the pasted text are matched in order.
- Pasted lyrics that do not match the audio (wrong song, skipped verse): lines that cannot be placed are listed, then placed after the last aligned line, 2 s each, and flagged, as in TX-030.
- Heavy reverb or pitch correction lowers accuracy. The result is flagged as needing review.
- Looking up lyrics online is out of scope.
- **Replace existing lyrics** replaces only `Lyrics` segments that are still unedited, like TX-038's replace mode. Lyrics the user corrected are kept.
- Lyric tracks keep every editing command: split, merge, text edits and caption looks (TX-051). The reference editor blocks split and merge while a lyric template is applied, drops a template's motion-blur look when lyric text is edited, and blocks editing while lyrics are being fitted to the music. In FreeCut the job runs on a snapshot (AI-001), so editing stays possible, and a look is simply re-applied after edits.
- Recognition language: any language AI-009 supports. The reference editor limits lyric recognition and lyric templates to fewer languages.
**Undo:** `ReplaceCaptions` (TX-038) with `origin: Lyrics`, one op per run. **Delete all lyrics** is `DeleteCaptions { ripple: false }`. The non-ML paths use the ops of TX-030 and TX-031.
**Model notes:** Lyric segments are ordinary `CaptionSegment`s. Recognition, alignment and vocal isolation belong to AI-010 and AI-023 (`freecut-analysis`). AI-010's Align known text mode reuses the local Whisper model (MIT weights) and its word timestamps to match the pasted words. Alignment models with non-commercial licenses, such as the CC-BY-NC-4.0 MMS alignment weights, must not be used. Placing aligned lines on the track is a pure function in `freecut-timeline`.

### TX-045 Dictate text with the local speech model — later
**What the user sees:** A microphone button in the inspector's text field (TX-002), in caption cells (TX-026) and in TX-030's long-text box. Clicking it starts listening: the button pulses and a small level meter shows the input. Clicking again, or pressing Esc, stops listening. The button then shows a transcribing spinner, and the recognized text is inserted at the caret, replacing any selection.
**Inputs / controls:** All FreeCut defaults.
- Input device: the device chosen for voice-over recording (AU-027).
- Language: the target's language tag (the caption track language, or the text clip's tag from TX-006). Without one, Auto-detect.
- Auto-stop: after 60 s of listening, after 5 s of silence, or when the target field's character limit (TX-002) would be reached.
- Punctuation comes from the recognizer. Spoken commands such as "new line" are not interpreted in this version.
**Behavior / output:**
- Audio is captured through `freecut-audio` into memory and is never written to the project or to disk.
- AI-009's local speech model, installed through AI-002's model manager, recognizes it off the UI thread as a short AI-001 job. No network is used.
- The text joins the open edit session, so dictation and typing in one session form a single undo entry.
**Edge cases:**
- Speech model not installed: the button opens AI-002's model-manager prompt, and nothing is recorded.
- No microphone, or OS permission denied: the button is disabled, and its tooltip explains how to grant access, as AU-027's recording panel does.
- Recording shorter than 0.5 s, or no speech recognized: "Nothing recognized", and nothing is inserted.
- Recognized text longer than the remaining character budget: cut at a word boundary, with a notice.
- The user keeps typing while the result is being transcribed: the text is inserted where the caret was when dictation started, shifted by any edits made before that point.
- Playback running: dictation pauses playback first, so the program audio does not feed the microphone.
- Locked track: the button is disabled along with the field.
**Undo:** Part of the surrounding edit session: `SetTextContent` (TX-002) for text clips, `SetCaptionText` (TX-026) for caption cells. Dictation in TX-030's dialog is not a project edit until the dialog places its text.
**Model notes:** The dictation session lives in `freecut-app`, capture in `freecut-audio`, and recognition in `freecut-analysis` through AI-009's model (no word timings needed). `freecut-timeline` is untouched until the edit is committed.

### TX-046 Image (texture) fill — later
**What the user sees:** The fill section gains **Image**: the letters are filled with a picture instead of a color. A thumbnail opens a picker with the project's still images (PM) and a small bundled set of FreeCut-made textures (paper, chalk, brushed metal, wood grain, noise, halftone), generated procedurally by FreeCut.
**Inputs / controls:**
- Image: a still image in the project's media, or a bundled texture. Video sources come later.
- Mapping: **Stretch** (default; fills the extent, aspect ignored), **None** (the image at its own size, centered on the extent) or **Tile**, with a tile scale of 10–400% (default 100%).
- Extent: the whole text block (default), each line, or each character.
- Opacity: 0–100%, default 100%.
**Behavior / output:** The image is mapped in the text's rest pose, as gradients are (TX-014), so animated characters carry their part of the image with them. Image alpha and fill opacity multiply. Outlines, shadows and glow work as with any other fill.
**Edge cases:**
- The image's media goes offline: the fill falls back to white, and PM's offline badge shows on the clip. Relinking (PM-091) restores it.
- Very large images are mip-mapped, so preview stays real-time. Export samples the full resolution.
- A per-range solid color (TX-015) wins over the image for its range.
- Busy textures can hurt legibility. TX-049 flags them.
- Many textured clips on screen: each image is uploaded to the GPU once and shared.
- A saved style (TX-020) with an image fill keeps its own copy of the image in the style library. Applying it in another project imports that copy as a media item.
- Curved text: the per-character extent follows each glyph along the arc.
**Undo:** `SetTextStyle` patch.
**Model notes:** `Fill::Image { source: TextureSource::{Media(MediaId), Bundled(&'static str)}, mapping: ImageMapping::{Stretch, None, Tile(PerMille)}, extent: FillExtent::{Block, Line, Char}, opacity: PerMille }`. The image is an ordinary media item (PM-042 treats images used for rendering as media, not assets). Decoding runs in `freecut-media` off the UI thread, and sampling is in `freecut-render::text`. Bundled textures are generated at build time by FreeCut's own code.

### TX-047 Multi-color word fill — later
**What the user sees:** A **Word colors** fill: each word gets the next color from a short list, either in order or shuffled. It suits punchy social captions.
**Inputs / controls:** All FreeCut defaults.
- Colors: 2–8 swatches, default white, warm yellow and light green.
- Order: **In order** (default) or **Random**. Random uses a seed stored with the style, so it is deterministic. With three or more colors, Random never gives two neighboring words the same color.
- Unit: word (default) or line.
- On caption tracks: **Restart for each caption** (default) or continue across captions.
**Behavior / output:** Colors are computed from the text at render time, in reading order, with Unicode word boundaries. They are never stored per word, so editing the text simply recolors it. Spaces and punctuation take the color of the word before them.
**Edge cases:**
- Text without spaces between words (CJK): a word segmenter is used when available (TX-040), otherwise each character counts as a word.
- A per-range solid color (TX-015) wins for its range and does not advance the cycle.
- Per-unit animations (TX-023) and the active-word style (TX-039) apply on top.
- Applied to a caption track style, every segment is colored independently unless the cycle continues across captions.
**Undo:** `SetTextStyle` or `SetCaptionTrackStyle` patch.
**Model notes:** `Fill::WordCycle { colors: SmallVec<[Rgba8; 8]>, order: CycleOrder::{InOrder, Random { seed: u32 }}, unit: CycleUnit::{Word, Line}, restart_per_segment: bool }`. The pure `word_colors(text, &fill, start_index) -> Vec<(Range<u32>, Rgba8)>` lives in `freecut-render::text` and is unit-tested.

### TX-048 Stacked outlines and shadows, inner and long shadows — later
**What the user sees:** The Outline and Shadow sections gain **+ Add**, so one text can carry several outlines and several shadows. Each entry has its own controls, an eye toggle and a drag handle for order. The shadow type adds **Inner** (a shadow inside the letters) and **Long** (a solid shadow swept from the letters at an angle, for a 2D block-3D look). Outlines gain a **Dashed** style.
**Inputs / controls:** All FreeCut defaults.
- Up to 4 outlines and 4 shadows per text.
- Each outline: TX-010's controls, plus Style **Solid** (default) or **Dashed** with dash length and gap of 10–400% of the outline width (defaults 200% and 100%).
- Inner shadow: TX-011's color, opacity, blur, distance and angle.
- Long shadow: color, opacity, length 0–100% of the font size (default 20%), angle (default 45°, measured as in TX-011), and **Fade** (solid, or fading toward the end; default solid).
**Behavior / output:**
- Outside and Center outlines stack outward: the first sits on the glyph edge, and each next one wraps around the outer edge of the one before. Inside outlines stack inward in the same way.
- Drop and long shadows are drawn in list order beneath the outlines. Inner shadows are drawn over the fill and clipped to it.
- A long shadow is the text's silhouette swept along the angle and drawn as one shape, so it shows no banding.
**Edge cases:**
- Dashes follow each closed glyph contour and restart on every contour. Animating the dash phase comes later.
- Long shadows on curved text are computed after curving, so they follow the arc.
- Per-range outline color (TX-015) affects the first outline only.
- Keyframes (TX-021) reach the first outline and the first shadow only in this version.
- Each extra layer costs a render pass. At 4K, TX-011's blur cap applies to each.
- Hiding an entry with its eye keeps its settings and skips its pass.
**Undo:** `SetTextStyle` patch. Adding, removing, reordering and hiding entries are patches too.
**Model notes:** `Shadow` gains `kind: ShadowKind::{Drop, Inner, Long { length: PerMille, fade: bool }}` (default `Drop`), and `Stroke` gains `dash: Option<Dash { len: PerMille, gap: PerMille }>` and `visible: bool`. `TextStyle` gains `extra_strokes: SmallVec<[Stroke; 3]>` and `extra_shadows: SmallVec<[Shadow; 3]>` after TX-010's and TX-011's single entries. Older projects load with empty lists, so no migration is needed.

### TX-049 Readability and frame-fit checks — later
**What the user sees:** A small warning icon on text clips, caption segments and template layers whose text is hard to read or does not fit. Hovering lists the reasons, each with a one-click fix where one exists. Checks:
- Outside the canvas or EX-011's title-safe area. Fix: **Move inside**.
- Too small: rendered cap height below 1.5% of the canvas short edge. Fix: **Increase size**.
- Low contrast between the fill and what lies behind the text. Fix: **Add outline** or **Add box**.
- Very thin faces (weight 200 or lighter) at small rendered sizes.
- Busy image fills (TX-046) with high local contrast.
- Words broken across lines by a narrow box or caption width.
- Decorations or caption emoji (TX-051, TX-053) overlapping the text.
- Heavy styling: more than 8 textured or multi-layer text items visible at once may slow preview (information only).
**Inputs / controls:** In Preferences, each check can be turned off. Contrast threshold: 1.5:1–7:1, default 3:1 (the common large-text guideline). All values are FreeCut defaults.
**Behavior / output:**
- Contrast is measured against the item's own background box when it has one. Otherwise it is measured against the rendered frame beneath the text at the item's middle frame.
- Checks run off the UI thread when the timeline is paused and an item is selected or changed, never during playback.
- They never change anything by themselves. EX-036's preflight lists them as Info items once this feature ships.
**Edge cases:**
- Text over moving video: one sampled frame can mislead, so the low-contrast warning says which frame was checked.
- Fully transparent or hidden items are skipped.
- Fixes that conflict with keyframes (for example moving a keyframed position) add a keyframe at the playhead instead of moving every key.
- A warning the user dismisses stays dismissed for that item until the item changes.
**Undo:** Checks are n/a (view state). Each fix is the ordinary op of the property it changes, for example `SetTextStyle` or TR's transform op.
**Model notes:** Geometric checks (bounds, size, broken words) are pure functions over layout results in `freecut-render::text`. Contrast sampling uses the render path in `freecut-render`. Dismissals are view state in `freecut-app`.

### TX-050 Font collections — later
**What the user sees:** In the font picker the user can create named **Collections**, for example "Brand" or "Channel titles". Fonts are added from their context menu or by dragging them onto a collection, and a collection can be picked as a filter. A font can belong to several collections. Collections appear as sections under Favorites.
**Inputs / controls:** Collection name: 1–64 characters, unique regardless of case (FreeCut default). Members: font families.
**Behavior / output:**
- Collections are lists of family names stored per user in app preferences (UI-018), never in projects.
- Removing a font from a collection never removes the font itself. Deleting a collection deletes only the list, after a confirmation.
- Collections can be renamed, reordered, exported and imported with style packs (TX-020). A pack carries the list of family names, never the font files.
- Collections are FreeCut's local way to keep brand fonts, which OV-043's brand kit leaves to text.
**Edge cases:**
- Renaming to an existing name: refused, with an inline message. A failed rename never fails silently.
- A member font that is no longer installed: shown dimmed as "not installed" and kept in the collection, so reinstalling it restores it.
- Bundled, system and imported fonts can all be members.
- The reference editor's font folders cannot be copied, moved or deleted. FreeCut's collections can be edited freely, because they are only lists.
**Undo:** n/a. Collections are app preferences, not a project edit. Deletion asks for confirmation.
**Model notes:** `FontCollection { id, name, families: Vec<String> }` in `freecut-app` preferences. The picker's filter reads the `FontRegistry` snapshot.

### TX-051 Caption looks (animated caption presets) — v1
**What the user sees:** A **Caption looks** tab in the Text panel and a **Look** section in the caption track inspector. A look is a complete animated caption design: font and style, background, the word-state styles of TX-039, In and Out animations (TX-022, TX-023), a display mode (TX-052), keyword styling (TX-040) and optional decorations, which are sticker or shape elements from OV placed around the caption. Thumbnails play on hover.
- Clicking a look applies it to the selected caption track. A look can also be chosen in advance in the auto-captions dialog (TX-038), so new captions arrive styled.
- **Remove look** returns the track to the default caption style, after a confirmation.
- **Save as look…** saves the selected track's current design as a user look.
- FreeCut ships about 20 looks of its own design (FreeCut default) in generic groups: Clean, Bold pop, Karaoke highlight, Boxed words, Typewriter, Neon and Minimal lower line.
**Inputs / controls:**
- The look.
- **Variants** (advanced): a look has one main style plus up to 8 variants (FreeCut default). A variant is a style patch with a scope: whole segment, line, word, or keywords. Variants rotate across successive segments **In order** (default), **Random** (seeded) or in a **Custom sequence** of variant numbers typed as a comma-separated list, for example "1,1,2,3". Variants can be added, duplicated and deleted.
- **Keyword style linked** (default on): the keyword style follows changes to the main style, except for the properties the keyword style sets itself.
- Decorations: up to 4 elements anchored to the caption block (above, below, left, right or behind), each with its own animation.
**Behavior / output:**
- Applying a look writes resolved values into the track: style, layout, animations, word-state styles, display mode, emphasis style, variants and decorations. The project stores values, never only a look ID, so it never depends on the look library (the same rule as TX-019).
- Segment overrides (TX-028) and manual spans stay on top of the look.
- Variants are chosen per segment by a pure function of the segment's index and the seed. The result is deterministic and stable under undo.
- Auto-fit rules run when a look is applied and whenever segments are re-chunked: text stays inside EX-011's title-safe area, words are never broken across lines, the rendered size never drops below TX-049's minimum, and decorations move so they never cover text. When a rule cannot be met (for example one very long word), the segment gets a warning badge, and nothing is cut.
- Decorations render as part of the caption layer and follow each segment's timing. They are not separate timeline clips.
- Split, merge, cut, copy, paste and text edits stay available on segments with a look, and the look is re-applied to the result. The reference editor blocks these on its animated caption templates.
**Edge cases:**
- A look that needs word timings (karaoke, word by word) on segments without them: those segments fall back to whole-segment display, and the inspector explains why (TX-039).
- A look's font is missing: TX-007 applies.
- A look file from a newer schema: refused with an explanation, as for templates (TX-024).
- Deleting a variant is an ordinary undoable edit. The reference editor cannot undo deleting one of its sub-templates.
- **Save as look…** on a track with per-segment overrides: only the track-level design is saved. A look is saved from a track, not from one caption.
- A look's decorations use bundled FreeCut elements or the user's own stickers (OV-020). A user look that uses a sticker missing on another computer shows OV's missing-sticker placeholder.
- The reference editor does not relink keyword style to a base style with several outlines or shadows. FreeCut links property by property, so stacked effects (TX-048) are no obstacle.
**Undo:** `ApplyCaptionLook { track, before: CaptionTrackDesign, after }`, one op for apply, remove, and every variant or decoration edit. **Save as look…** writes a user file (n/a).
**Model notes:** `CaptionTrackDesign` is the part of `CaptionTrackData` that a look sets: style, layout, animations, word-state styles, emphasis style, `display` (TX-052), `variants: Vec<Variant { patch: TextStylePatch, scope: VariantScope::{Segment, Line, Word, Keywords} }>`, `rotation: Rotation::{InOrder, Random { seed }, Custom(Vec<u8>)}`, `keyword_linked: bool` and `decorations: Vec<Decoration { source: ElementSource, anchor, offset, animations: ClipAnimations }>`, where `ElementSource` is OV's. Looks are JSON in `resources/caption-looks/` plus a user folder, loaded by `freecut-app` and (de)serialized by `freecut-project`. `resolve_caption_style` (TX-028) gains the variant step right after the track style. Decorations are drawn by OV's element renderer inside the caption layer (FX-001 step 2).

### TX-052 Progressive caption display: word, line and screen — later
**What the user sees:** A **Display** setting on a caption track, also part of every caption look: **Whole caption** (default, the plain behavior), **Screen by screen**, **Line by line** or **Word by word**. In the progressive modes a long segment is shown in pieces that follow the speech instead of all at once. The Captions panel still shows and edits the whole segment.
**Inputs / controls:** All FreeCut defaults.
- Display mode, as above.
- Words per line: 1–12, or **No limit** (default), which folds lines by TX-037's maximum characters per line.
- Lines per screen: 1–3 (default 2), or **No limit**, which follows TX-037's maximum lines.
- Keyword placement: **Inline** (default), **Own line** or **Own screen**, with a keyword size of 100–200% (default 130%).
**Behavior / output:**
- The segment is cut into screens of up to the given lines and words.
- **Screen by screen:** each screen appears whole when its first word starts and is replaced by the next screen.
- **Line by line:** within a screen, each line appears when its first word starts. The screen clears when the next screen begins.
- **Word by word:** within a screen, each word appears when it starts.
- Timing comes from word timings (TX-039). Without them, the pieces share the segment's time in proportion to their character counts.
- Chunking happens at render time and never splits the stored segment. Subtitle files keep whole segments unless TX-032's **Export screens as separate cues** is on. Burn-in shows the pieces.
- Manual line breaks are kept as line boundaries. A manual break that would exceed lines per screen starts a new screen. The reference editor refuses such a break and asks the user to change lines per screen instead.
**Edge cases:**
- Text shorter than one line: shown as one piece.
- A single word longer than the line width: placed on its own line and never broken. A warning shows if it leaves the safe area (TX-051's auto-fit).
- A keyword on its own screen without word timings: it gets its share of the time by character count.
- Changing the layout re-chunks immediately. Very large tracks are re-chunked off the UI thread.
- Word-state styles (TX-039) and per-unit animations (TX-023) apply within each piece.
- Bilingual tracks (TX-036): the secondary line is shown whole for the segment's full duration, under every piece.
**Undo:** `SetCaptionTrackStyle` (the display setting is part of the track design), or `ApplyCaptionLook` when it changes through a look.
**Model notes:** `CaptionDisplay { mode: DisplayMode::{Whole, Screen, Line, Word}, words_per_line: Option<u8>, lines_per_screen: Option<u8>, keyword: KeywordPlacement::{Inline, OwnLine, OwnScreen}, keyword_size: PerMille }` on `CaptionTrackData`. `chunk_segment(&seg, &display, &rules) -> Vec<Chunk { text_range, time: TimeRange }>` is a pure function in `freecut-timeline`, tested with and without timings, with manual breaks and with keywords.

### TX-053 Automatic emoji on captions — later
**What the user sees:** **Add emoji** on a caption track turns on automatic emoji: each caption with a word that matches an emoji keyword gets one emoji shown with it. A list in the Captions panel shows each segment's emoji. Clicking one swaps it, through a search of emoji names or the OS emoji picker, or removes it. Track settings hold position, size and **Update automatically**.
**Inputs / controls:** All FreeCut defaults.
- Position: **Above** (default), **Below**, or **Free** (dragged in the preview; one offset per track).
- Size: Small, **Medium** or Large, which are 100%, 150% and 200% of the caption font size.
- At most one emoji per segment.
- **Update automatically** (default on): when a segment's text changes, its automatic emoji is recomputed. Emoji the user picked by hand are never replaced.
**Behavior / output:**
- Matching uses the open Unicode CLDR emoji annotation data (keywords and short names per language) for the track's language. Words are lowercased, simple plurals are folded where the language supports it, and the most specific match wins. Ties go to the earliest word.
- The emoji is stored on the segment and drawn with the bundled color emoji font (TX-006) as part of the caption layer.
- Everything runs locally, with no network.
**Edge cases:**
- Works on every caption track, whether typed, imported or recognized. The reference editor limits it to auto-generated captions in two languages.
- A track with no segments: the command is disabled, with the hint "Add captions first".
- A language with no CLDR annotation data: the command is disabled, with an explanation.
- No matches at all: "No emoji found", and no change.
- A segment whose text already contains an emoji gets no extra one. The reference editor blocks the whole feature when its caption style already contains emoji.
- Turning automatic emoji off removes every emoji on the track, hand-picked ones included, after a confirmation that states the count.
- Position Above on a track whose secondary line is above: the emoji goes above both lines.
- Subtitle files never contain these emoji (TX-032). Burn-in shows them.
**Undo:** `SetCaptionEmoji { track, before: Vec<(SegId, Option<CaptionEmoji>)>, after }`, one op for apply, swap, remove and turning off. Position, size and the automatic option are `SetCaptionTrackStyle`.
**Model notes:** `CaptionSegment.emoji: Option<CaptionEmoji { emoji: String, origin: EmojiOrigin::{Auto, Manual} }>` and `CaptionTrackData.emoji_settings`. The keyword table is generated at build time from CLDR annotation data into `resources/emoji-keywords/` and loaded by `freecut-app`. Matching is a pure function in `freecut-timeline` that takes the table as an argument, so it needs no I/O.

### TX-054 Text inspector: lens contents and card behaviour — v1
**What the user sees:** Selecting a text clip shows the five text lenses of the UI concept: **Content**, **Style**, **Box & bubble**, **Motion** and **Presets**. [ui-concept §7](../design/ui-concept.md#7-the-text-inspector) owns where each control sits and how it looks, and UI-008 owns which lenses exist. This feature fixes what each card holds on a text clip, which rows are essential (shown with the Detail switch on Essentials), and how the card-level controls behave:

| Lens | Card | Holds (essential rows first; the rest behind More) | Features |
|---|---|---|---|
| Content | Text | the content field | TX-002 |
| Content | Paragraph | alignment, box width mode and width; vertical anchor, direction | TX-005, TX-017 |
| Content | Speech (later) | voice, **Speak text**, state of the linked speech clip | TX-043 |
| Content | Convert | to captions; link to find and replace | TX-035, TX-034 |
| Style | Type | font and face, size with **−** / **+** steppers, emphasis (B, I, U, S), case | TX-003, TX-004, TX-008 |
| Style | Fill | colour or gradient; fill mode | TX-004, TX-014, TX-046, TX-047 |
| Style | Spacing | letters and lines, each with **−** / **+** steppers | TX-009 |
| Style | Outline, Glow, Shadow (optional layers) | each layer's colour and size; position, offset, direction and the other rows | TX-010, TX-012, TX-011, TX-048 |
| Box & bubble | Placement | position, scale, rotation, opacity; blend mode | TR-001, TR-015, OV-004 |
| Box & bubble | Background (optional) | colour, opacity, shape, padding, roundness; height, border | TX-013 |
| Box & bubble | Bubble (optional) | kind, tail on or off, colour; tail width and bend, padding, roundness, border | TX-055 |
| Box & bubble | Curve (optional) | curve amount | TX-016 |
| Motion | timing strip, preset grid, Pieces, Keyframes, Follow | see TX-022 | TX-021 to TX-023, TX-056, AI-008 |
| Presets | Looks, My looks, Templates, Default for new text | preset grids | TX-019, TX-020, TX-024 |

**Inputs / controls:** The controls of the features in the table, with their ranges and defaults. This feature adds only:
- Size steppers: ±1 ref px per click, ±10 with Shift, clamped to TX-004's range.
- Spacing steppers: ±1% per click, ±10% with Shift, clamped to TX-009's ranges.
- **Reset card** (in each card's `...` menu): the card's properties return to the default text style's values (TX-020) and the card's keyframes are removed. An optional layer keeps its on or off state.
- **Reset lens** (footer): every card of the lens, as one entry. On the Content lens it resets paragraph settings only and never the text itself. On the Motion lens it clears In, Loop and Out and every text keyframe of the clip. The Presets lens has no reset.
- **Save look…** (footer of the Style and Box & bubble lenses): TX-020's save dialog.
**Behavior / output:**
- Optional layers (Outline, Glow, Shadow, Background, Bubble, Curve) keep their values and keyframes while switched off (ui-concept §6.2, TX-021).
- **Background** and **Bubble** are exclusive: switching one on switches the other off in the same entry. Each keeps its own settings, so switching back restores them.
- Values follow the playhead for animated properties (UI-008), and editing them follows TR-028's auto-key rule.
- A layer that cannot apply right now stays editable and says why in its summary chip, for example "paused while curved" on Background and Bubble (TX-013), with a link to the Curve card.
- With several text clips selected, mixed values show a dash (UI-010), and a change applies to all of them in one entry.
- Every text setting is reachable from the command bar as "Go to setting: …" (ui-concept §6.2), through a stable property path such as `text.outline.width` or `text.bubble.tail.tip`. The assistant (AS-006) reaches the same properties through the text style and layout edit requests. Every property added for this feature's cards is a new variant of `TextStyleEdit` or `TextLayoutEdit`, so llm-editor.md's tool-coverage test catches any that lack an assistant tool.
- A caption track or segment selection shows the same lenses with TX-028's **Scope** switch (ui-concept §6.3), without the Speech and Follow cards, which have caption equivalents (TX-043's **Read all captions**).
**Edge cases:**
- Clip on a locked track: everything is read-only, with UI-008's banner.
- A layer from a newer FreeCut that this build cannot draw: its card is read-only with **Not supported in this version**, and its values are kept on save (TX-019).
- A file with both Background and Bubble switched on (hand-edited or from a newer build): the bubble draws, and the Background card shows as off until the user changes it.
- **Paste settings** on a card with nothing suitable on TL-034's attribute clipboard: disabled, with the tooltip "Copy the settings of a text card first".
- **Reset card** or **Reset lens** on properties that have keyframes: the keyframes go too, and a toast confirms it with **Undo**.
- A text layer inside a template group (`TextOrigin::Template`, TX-024): every card works; the header shows the grouped banner (UI-008).
**Undo:** The inspector itself is n/a. Each control is its owning feature's op, coalesced per gesture (UI-009). A layer switch is a `SetTextStyle` patch, which includes the exclusivity change. **Reset card** and **Reset lens** are one compound `ResetText { clips, before, after, removed_keys, cleared_slots }` each.
**Model notes:**
- Turning a layer off must keep its settings and keyframes. The layers of TX-010 to TX-013, TX-016 and TX-055 are therefore stored as `Layer<T> { on: bool, value: T }` instead of the `Option<T>` (or, for the curve, the bare `PerMille`) written in those features. Text counts as curved (TX-005, TX-013) only while the Curve layer is on and its amount is non-zero. None of them has shipped, so no migration is needed. The bubble is its own field, `bubble: Layer<TextBubble>` (TX-055), and a pure check keeps at most one of `background` and `bubble` on.
- `TextCard::{Text, Paragraph, Type, Fill, Spacing, Outline, Glow, Shadow, Background, Bubble, Curve}` and `card_props` (TX-021) live in `freecut-timeline`, so card reset and the card keyframe commands are pure and unit-tested. `TextPropPath` is a closed enum with a stable string form, also in `freecut-timeline`; the command bar and preset files use it.
- The lens and card renderers are in `freecut-app::ui::inspector`, built from the UI concept's card and row widgets.

### TX-055 Bubble and tag boxes — v1
**What the user sees:** The Box & bubble lens (TX-054) has a **Bubble** card, an optional layer next to Background. Its **Kind** choice offers eight shapes designed for FreeCut and drawn from parameters:
- **Rounded:** a rounded rectangle with a tail, for ordinary speech.
- **Oval:** an ellipse with a tail.
- **Thought:** a cloud of lobes with a trail of shrinking circles instead of a tail.
- **Shout:** a burst with spikes, for loud lines.
- **Square:** a sharp-cornered box with a tail, for captions and narration.
- **Tag:** a pill-shaped label, no tail.
- **Arrow tag:** a label with one pointed end, for pointing at something.
- **Ticket:** a rectangle with notched corners, no tail.

With a tailed bubble on, the Stage shows a round handle at the tail's tip. Dragging it moves the tip; the tail's base slides along the outline so it always faces the tip. Switching the Bubble card on switches Background off, and the reverse (TX-054).
**Inputs / controls:**
- Kind: as above. Default **Rounded** (FreeCut default).
- Fill colour and opacity, padding, roundness and border: the same controls and ranges as TX-013's box. Bubble defaults (FreeCut defaults): white fill at 100%, padding 45% horizontal and 30% vertical of the font size, roundness 40%, a dark border at 4%.
- Tail on or off: on by default for Rounded, Oval, Square and Shout; Thought uses its trail; off for Tag and Ticket. Arrow tag has a **Point** choice, left or right (default right), instead of a tail.
- Tail tip: a position in the text clip's local frame, in ref px from the box centre. Default: a quarter of the box width left of centre, 1.2 em below the bottom edge.
- Tail width at its base: 10–60% of the box's shorter side, default 24%.
- Tail bend: −100 to +100, default 20. 0 gives a straight wedge; other values curve it to one side.
- Shout only: spikes 8–32 (default 14), spike depth 5–40% of the shorter side (default 18%) and irregularity 0–100% (default 30%), with a stored seed and a dice button for a new one.
- Thought only: lobe size 10–40% of the shorter side (default 22%) and trail circles 2–4 (default 3).
**Behavior / output:**
- The body fits the laid-out text bounds plus padding, like TX-013's Block, with a minimum body of 2 em × 1.5 em. Oval and Thought bodies grow until the text box's corners are inside them.
- Body and tail are built as one closed outline, filled once and bordered once, so the joint never shows a seam. The bubble is drawn where the box is in TX-011's layer order, and TX-011's **Include background box** lets it cast the shadow.
- The tip lives in the clip's local frame: moving, scaling or rotating the clip carries the whole bubble. Tip, tail width and bend are keyframable (TX-021), so a tail can follow a speaker who moves. A motion-tracking binding (AI-008) moves the whole clip; tracking the tip on its own comes later.
- The bubble is not part of layout. Wrapping, alignment and anchors ignore it. Selection handles and snapping use the visual bounds, which include the bubble and tail.
- Block animations move the bubble with the text. Per-unit reveals never reveal it piece by piece (TX-023).
- Switching the bubble on when the text fill and the bubble fill would have less than 3:1 contrast (white text on the white default): the bubble takes the dark default `#1E1E24` instead. Turning a bubble on never changes the text colour.
- Kind thumbnails are rendered by FreeCut's renderer from the same parameters.
**Edge cases:**
- Tip inside the body: the tail is hidden; the handle stays draggable so the user can pull it out again (as OV-028).
- Tip further than 40 em from the box centre: the drag stops there.
- Empty text: no bubble, as TX-013 does for boxes. The dashed placeholder (TX-002) still shows while the clip is selected.
- Curved text: the bubble is paused like any box (TX-013, TX-016).
- The Bubble card has no Per line, Per character or Lower half options; those stay with Background.
- Fixed-width box (TX-005): the body width is the fixed width plus padding, and the text wraps inside it.
- Caption tracks (TX-028): each segment gets its own bubble. The tail tip is part of the track style; a segment override can move it for one caption.
- Very small text: the minimum body keeps the tail and spikes readable. Below 10 ref px font size, spikes are drawn without irregularity.
- A stand-alone bubble element (OV-028) linked to this clip and a TX-055 bubble at the same time: both draw, and the inspector suggests keeping one.
**Undo:** `SetTextStyle` patch. A tip drag coalesces into one entry. Keyframes are TR's ops.
**Model notes:** `TextStyle.bubble: Layer<TextBubble>` (TX-054's `Layer`), with `TextBubble { spec: BubbleSpec, fill: Rgba8, opacity: PerMille, pad_x: PerMille, pad_y: PerMille, roundness: PerMille, border: Option<(Rgba8, PerMille)> }` and `BubbleSpec { kind: BubbleKind::{Rounded, Oval, Thought, Shout, Square, Tag, ArrowTag { point: Side }, Ticket}, tail: Option<Tail { tip: (Fixed, Fixed), base: PerMille, bend: PerMille }>, spikes: u8, spike_depth: PerMille, jitter: PerMille, seed: u32, lobe: PerMille, trail: u8 }`, all in `freecut-timeline`. The geometry is one pure function, `bubble_path(&BubbleSpec, body: Rect, roundness) -> Path`, in `freecut-render::shape::bubble`, shared with OV-028. Its tests check that the outline is closed and free of self-intersection across the bend range, that the tail base lies on the body, and that a seed always gives the same burst.

### TX-056 Text animation preset catalogue — v1
**What the user sees:** The In, Loop and Out grids of the Motion lens (TX-022) list the presets below, each with a tile that plays "Abc" in the preset on hover. The **Pieces** control (block, line, word or letter) narrows the grid to presets that move those pieces. All presets are FreeCut's own motions with generic names; their behaviour is defined here in words and numbers.
**Inputs / controls:** Every preset uses the shared controls of TX-022 and TX-023: Duration or Pace, Easing, Direction where listed, Intensity where listed, and Unit (Pieces), Overlap and Order for per-unit presets. Conventions for the table:
- `em` is the clip's font size. Distances in em scale with the text, so a preset looks the same on a small caption and a big title.
- `k` = Intensity / 50: 0 gives no movement, 50 the default amplitude, 100 twice the default.
- Motions are given for In, from the start of the slot to rest. **Out** plays each unit's motion backwards in time, from rest to the start pose, with the easing mirrored (ease-out becomes ease-in), unless the table says otherwise. The order of units still follows the Order control, so with forward order the first unit leaves first (TX-023). Every In ends exactly at rest and every Out starts exactly at rest, with no jump at the slot boundary.
- Whole-text presets reuse FX-030's motions, with distances in em instead of canvas units.

*Whole text (In and Out):*

| Preset | Motion | Defaults |
|---|---|---|
| Fade | Opacity 0 → 1. | 0.5 s, ease-out |
| Rise | Starts 1·k em below and transparent, moves up to rest; opacity reaches 1 at 40% of the slot. | 0.5 s, ease-out cubic |
| Drop | Starts 1.5·k em above, falls to rest and settles with one small bounce (10% of the distance). | 0.6 s |
| Slide | Enters from the chosen side, starting just outside the canvas edge (FX-030's **From edge**), or 3·k em away with **Short**. | 0.6 s, ease-out cubic, from the left |
| Zoom in | Scale from 1 − 0.5·k (never below 0.05) to 1; opacity reaches 1 at 40%. | 0.5 s, ease-out cubic |
| Zoom out | Scale from 1 + 0.5·k to 1; opacity reaches 1 at 40%. | 0.5 s, ease-out cubic |
| Pop | Scale 0 → 1 + 0.15·k at 65% of the slot → 1. | 0.4 s, back-out |
| Bounce | Falls from 2·k em above with two decaying bounces. | 0.8 s, bounce-out |
| Blur in | Blur radius 0.4·k em → 0; opacity reaches 1 at 50%. | 0.5 s, ease-out |
| Focus pull | Blur 0.3·k em and scale 1 + 0.1·k → rest. | 0.6 s, ease-out |
| Spin in | Rotation −90·k° → 0 with scale 0.6 → 1. | 0.6 s, ease-out cubic |
| Flip open | Turns about the baseline from 90° (edge-on) to flat, with FX-030's 3D-turn perspective (eye distance 1.5 × the text block's width). | 0.5 s, ease-out |
| Wipe | Revealed by a straight edge moving from the chosen side, with a soft edge of 0.5 em. | 0.6 s, ease-in-out sine, from the left |

*Per unit (In and Out):*

| Preset | Units | Motion of each unit | Defaults |
|---|---|---|---|
| Typewriter | letters | Appears at once at the start of its window (a step, no easing). Option **Caret**: a bar in the fill colour, 0.08 em wide and one line tall, rides after the last revealed letter and blinks (0.5 s on, 0.5 s off) once typing ends, until the slot ends. | Pace 0.06 s per letter, Overlap 0%, forward, caret off |
| Letter fade | letters, words | Opacity 0 → 1. | 0.8 s, Overlap 70%, ease-out |
| Letter rise | letters, words | Starts 0.6·k em below and transparent, rises to rest. | 0.8 s, Overlap 60%, ease-out cubic |
| Letter drop | letters | Falls from 0.8·k em above and settles with a small bounce. | 0.8 s, Overlap 50% |
| Letter pop | letters, words | Scale 0 → 1 + 0.2·k → 1 about its own centre. | 0.7 s, Overlap 60%, back-out |
| Letter blur | letters, words | Blur 0.3·k em → 0 with a fade. | 0.8 s, Overlap 70% |
| Letter spin | letters | Rotation 180·k° → 0 about its own centre, with a fade. | 0.8 s, Overlap 50% |
| Scatter | letters | Starts at a seeded random offset up to 1.5·k em away, turned up to ±40·k°, transparent; gathers to rest. | 1.0 s, Overlap 80%, random order |
| Decode | letters | Shows a seeded sequence of other letters taken from the clip's own text, changing every 2 frames, and lands on its own letter at the end of its window. Each stand-in glyph is drawn centred in the advance of the letter it replaces, so the layout never changes (Animation never reflows, TX-022). Opacity stays at 1. | 1.0 s, Overlap 60%, forward |
| Spread | letters | Every letter starts pulled toward its line's centre, at (1 − 0.6·k) of its distance, transparent; all move out together. | 0.6 s, Overlap 100%, ease-out |
| Word rise | words, lines | Rises one line height from below its own line, clipped to the line's box, so it seems to come up through a slot. | 0.7 s, Overlap 50%, ease-out cubic |
| Word slide | words, lines | Slides in 1·k em from the chosen side, with a fade. | 0.6 s, Overlap 50%, from the left |
| Marker sweep | lines | Draws the clip's per-line box (TX-013) from left to right over the first half of the line's window, then fades the line's text in over the second half. Without a per-line box it plays as Letter fade on lines. | 0.9 s, Overlap 70% |

*Out only:*

| Preset | Units | Motion | Defaults |
|---|---|---|---|
| Erase | letters | The reverse of Typewriter: letters disappear one by one from the end. | Pace 0.04 s per letter, Overlap 0%, backward |
| Fall away | letters, words | Falls 2·k em with gravity (ease-in), turning up to ±30·k° (seeded), fading over the last 40% of its window. | 0.9 s, Overlap 60%, random order |

*Loop, whole text:*

| Preset | Motion | Defaults (cycle) |
|---|---|---|
| Pulse | Scale 1 → 1 + 0.06·k → 1, sine. | 1.0 s |
| Heartbeat | Two quick swells (1 + 0.08·k, then 1 + 0.05·k) in the first 40% of the cycle, then rest. | 1.0 s |
| Float | Moves up and down by ±0.15·k em, sine. | 2.4 s |
| Wobble | Rotation ±4·k° about the centre, sine. | 1.2 s |
| Swing | Pendulum about the top centre, ±8·k°. | 1.6 s |
| Shake | Smooth seeded jitter of ±0.06·k em, changing direction 12 times a second. | — (continuous) |
| Flicker | Opacity dips to 1 − 0.6·k for 2 frames at seeded moments, at most 2 dips per second. | 1.0 s |
| Hue cycle | Fill and outline hues turn once round the colour wheel per cycle; lightness is kept. | 4.0 s, no intensity |
| Breathing glow | Glow intensity swings between 30% and 30% + 70%·k of its set value. With Glow off, the loop uses a temporary glow in the fill colour at 25% radius and leaves the Glow card off. | 2.0 s |

*Loop, per unit:*

| Preset | Units | Motion | Defaults (cycle) |
|---|---|---|---|
| Wave | letters, words | Each unit moves up and down by ±0.25·k em, sine, with the per-unit phase delay of TX-023, so a wave runs across the text. | 1.2 s, Overlap 0% (one full wave across the text) |
| Hop | letters | Each unit hops 0.4·k em in turn, the rest stay still. | 1.6 s |
| Jelly | letters | Each unit squashes and stretches, width and height ±10·k% in opposite phase. | 0.8 s, Overlap 40% |
| Wiggle | letters | Each unit turns by a smooth seeded jitter of ±6·k°, about its own centre. | — (continuous) |
| Shimmer | letters | A band of brightness (fill lightened by 40·k%) travels across the units. | 1.5 s |
| Sparkle | letters | Each unit swells to 1 + 0.2·k for 0.2 s at seeded moments, about once per cycle. | 2.0 s |

**Behavior / output:**
- Each preset is a data file listing its channels (offset x/y, scale, scale x/y, rotation, 3D turn, opacity, blur, reveal edge, glyph substitution for Decode, style modulation for Hue cycle and Breathing glow), each with a normalized curve and an amplitude, using FX-030's manifest format. TX-022's evaluator and TX-023's unit windows drive them; nothing is special-cased in code except Decode's glyph choice and Typewriter's caret.
- Easing named in the table is the preset's built-in curve; the Easing control overrides it (FX-018).
- Seeded presets (Scatter, Decode, Shake, Flicker, Fall away, Wiggle, Sparkle) use TX-023's stored seed, so preview and export always match and every OS gives the same result.
- **Design rules for every FreeCut preset** (also the review checklist for new ones): the text is fully legible at rest; nothing flashes more than 3 times per second; no preset changes the text's colours except Hue cycle, Shimmer and Breathing glow, whose names say so; motion stays inside 3 em of the rest pose unless the preset is a slide from the edge; thumbnails show exactly what the preset does.
- There are no glitch presets in this catalogue. Colour-split and block-noise looks belong to FX-015's video effects applied to the text clip, so the text presets stay calm and readable. Shake and Wiggle are smooth, seeded movements, not frame jumps.
**Edge cases:**
- Decode on text with only one distinct letter: it cycles that letter's case variants when they exist, or plays as Letter fade.
- Decode on cursive-joining scripts (Arabic and similar): letter units degrade to words (TX-023), and swapping whole words would break joins, so it plays as Letter fade on words.
- Per-unit presets on a single-line, single-word text with Unit = line: the whole text moves as one unit (TX-023).
- Breathing glow with an Inside outline at full width: the glow still draws (TX-012's order).
- Hue cycle on a gradient fill: every stop turns by the same angle. On an image fill (TX-046) it has no effect, and the tile is disabled with a tooltip.
- Marker sweep on curved text: the box is paused (TX-016), so it plays as Letter fade.
- Typewriter caret on right-to-left text: the caret rides on the left of the last revealed letter.
- A preset file this build cannot read: FX-018's **Unavailable animation** handling.
**Undo:** As TX-022: `SetClipAnimation`. Picking a new seed is a change of `AnimParams` in the same op.
**Model notes:** Preset files live in `resources/text-animations/` with ids such as `fc.in.letter-rise`, `fc.out.erase` and `fc.loop.wave`, loaded by `freecut-app` and parsed by `freecut-project`. The channel model and curve evaluation are FX-030's (pure, in `freecut-timeline`); per-glyph application is `freecut-render::text::animate` (TX-022). Every preset has a golden test: its offsets sampled at 0%, 25%, 50%, 75% and 100% of the slot, plus a check that the 100% sample is the rest pose (0% for Out).
- *FreeCut decision (S2 catalogue, 2026-10-13):* every preset above is a file in `resources/text-animations/` (ids `fc.in.zoom-in`, `fc.in.letter-pop`, `fc.out.fall-away`, `fc.loop.breathing-glow`, …); Outs of In presets play them backwards (`fc.out.x`). Where the table leaves room: an unnamed easing is ease-out (quad); Drop and Letter drop fall as a parabola, land at 60 % and bounce back up 10 % of the distance; Letter pop peaks at 65 % like Pop; Letter rise, blur, spin and Word slide fade in with their motion's easing; Flip open turns about the bottom centre of the box; Pulse and Breathing glow follow a raised cosine (Breathing glow starts at its top, 30 % + 70 %·k, so at Intensity 50 the loop starts from the set glow); Heartbeat swells at 10 % and 30 % of the cycle; Hop takes the first quarter of each unit's cycle; Shimmer's band lights each unit over a fifth of the cycle; Decode shows stand-ins until the end of each unit's window; Marker sweep's line box follows the per-unit reveal. Scatter, Spread, Word rise, Fall away, Shake, Flicker, Wiggle and Sparkle use channels the S2 engine follow-up added (below) and ship playable; a build that lacked a channel would list the preset as **Unavailable animation** (`resources/README.md`). Shimmer's band is a per-unit lightness the plan passes to the renderer (`TextAnimPlan::style`, TX-022), drawn glyph by glyph.
- *FreeCut decision (review fixes, 2026-10-13):* where Decode can't substitute (cursive-joining text, which moves by words, or text with fewer than two distinct single-glyph letters), it plays as Letter fade: each unit's opacity follows its progress. Case-variant stand-ins are not used. Typewriter's caret rides after the last letter that is both revealed and visible, so it follows the bundled Typewriter, which types with an opacity step. Under motion, TX-013's per-line and block boxes cover only letters already visible, and per-character boxes fade with their letter.
- *FreeCut decision (S2 engine channels, 2026-10-13):* the channels the catalogue waited for, in `freecut_timeline::motion` (seeded numbers come from integer hashing of the stored seed, the unit's reading-order index and the channel, so every OS agrees and a unit's motion never depends on the others; Intensity scales each linearly):
  - `random.offset` (Scatter): a seeded direction and a seeded distance of 0.5–1 × the amplitude, so every letter visibly gathers; `random.rotate` (Scatter, Fall away): a seeded turn within ±amplitude.
  - `toward_line_center` (Spread): the unit moves the amplitude's share of the way from its centre to its line's centre (`UnitContext::to_line_center`, from the layout); a whole block never moves by it.
  - `line_rise` (Word rise): down by the amplitude in line heights (`UnitContext::line_height`; 1.2 em when the layout gives none), and the pose asks for the unit to be clipped to its line's box while it moves.
  - `jitter.translate_x`, `jitter.translate_y`, `jitter.rotate` (Shake, Wiggle, FX-030's Jitter shake): knots alternating in sign with seeded sizes of 0.4–1 × the amplitude, 12 per second (an even number per cycle, so a loop is seamless), joined by smoothstep (smooth, never past the amplitude); vertical knots fall half a knot after horizontal ones, so the path wanders in 2D.
  - `flicker.opacity` (Flicker): opacity dips to `1 − amplitude` for 2 frames (2 frames at 30 fps without a frame rate). Dips are at least 1 s apart, also across the cycle seam: a dip is two sharp changes, so this keeps every second within FX-030's three sharp changes (and within 2 dips per second). A cycle of `c` seconds holds `⌊c⌋` dips at seeded places; a cycle under 1 s does not dip.
  - `sparkle.scale` (Sparkle): one raised-cosine swell to `1 + amplitude` lasting 0.2 s per cycle, at a seeded moment per unit (the whole cycle when it is shorter).
  - The photosensitivity checker counts flashes and dips on opacity, white tint, fill lightening and glow, at Intensity 0, 50 and 100, at the default length and 0.1 s, and over several seeds and units when a seeded channel changes brightness; `PresetTable` caches each preset's result (`availability`, `is_safe`, `safe_manifests`, `precompute`) and can also list presets the loader left out (`with_unavailable`), so grids can show them as **Unavailable animation** instead of hiding them.
- *FreeCut decision (S2 app, 2026-10-13):* Typewriter's **Caret** is stored with the slot (`AnimParams::caret`, off by default; the project file writes `"caret": true` only when on, and a file without it reads as off, so no migration). The Motion lens shows a **Caret** switch for a text In or Out whose preset reveals its letters with a step (Typewriter; `freecut_render::motion::is_stepped`); switching it is one `SetClipAnimation` undo entry. The renderer draws the bar with TX-056's 1 s blink (`CARET_BLINK`) in In and Out only; a Loop never has one. Decode's step is not a control: its stand-ins change every 2 frames of the sequence's frame rate (`TextAnimPlan::frame`, `DecodeStep::Frames(2)`), and preview and export use the same default compositor options, so they match. The **Seed** row with its dice shows for any preset whose motion depends on the seed (`PresetManifest::uses_seed`: a random order, a seeded channel, or Decode's stand-ins), not only for Order Random.

### TX-057 Captions drawer — v1
**What the user sees:** The **Captions** tab of the Shelf (UI-004, in the Write group beside Text) opens a Drawer whose side list holds every way to put captions on the timeline and to restyle them. It is the one place to start captions; the context-menu and panel entries elsewhere (TX-025, TX-031, AI-010) open the same forms. Side-list rows, in order (FreeCut wording):
- **From speech** — the Auto captions form of AI-010 (built in captions slice CS2; see the [captions build](#captions--speech-build--scope)).
- **From a song** — the lyrics form of TX-044 (built in captions slice CS3).
- **Type or paste** — **Add caption** at the playhead (TX-025), **Captions from text…** (TX-030) and **Tap to sync** (TX-030).
- **Files** — **Import subtitles…** (TX-031) and **Export subtitles…** (TX-032), with the formats the build supports.
- **Looks** — caption looks (TX-051) and the text style presets that suit captions (TX-019), as a grid of live tiles (ui-concept §13.7).
- **My styles** — the user's saved looks (TX-020) and saved caption looks.
- **Caption list** — the caption list of TX-026 for the active caption track, shown in the Drawer's content area until UI-056 lets it float as its own panel.
- *Favorites* and *Recent* join under *Mine* once UI-006 exists. Rows whose feature is not built are hidden (UI-004).

The **From speech** form is one property card (ui-concept §6.2) with these rows, then **Generate** as the accent button across the card:
- **Speech language:** *Detect automatically* (default) or a language, recently used first. After a run, the detected language and, when unsure, a "not sure" note with **Run again in…** appear under the row.
- **Second line:** *None* (default), *Empty, to type in*, *English, from the speech*, and further engines as TX-041 and AI-013 make them available (*Translate with my assistant provider* only when the assistant is set up, AS-002). Choosing anything but None turns on the track's secondary line (TX-036).
- **Mark filler words:** a switch, off by default; when on, a two-way choice **Keep them, dimmed** (default) or **Leave them out**. Cutting them out of the video is AI-012's review sheet, linked from here, never automatic.
- **Replace earlier auto captions:** a switch. On by default when the target track already has unedited recognized captions; it never touches captions that were typed, imported, scripted or corrected (TX-038).
- **Quality:** Fast, Balanced (default) or Accurate, each with its download size and an *installed* tick (AI-009, AI-002).
- **Apply to:** whole timeline (default), selected clips, or in–out range (AI-010).
- **Style:** the receiving track's current style (default), a look or a saved style (TX-038's Caption style control).

The **From a song** form has the music clip (default: the selected clip whose audio is music, else the first music clip under the playhead), **Separate the vocals first** (on when AI-023's pack is installed), an optional **Lyrics to align** text box (empty: recognize; filled: align the pasted lines, AI-010's Align known text), Speech language, Replace earlier lyrics, Quality, Style, and **Generate**.

**Inputs / controls:** As above. Form values are remembered per user as last-used settings (preferences), not in the project.
**Behavior / output:**
- **Generate** starts AI-010's job. When the model for the chosen quality is missing, the button reads **Get speech model (<size>)…** and opens AI-002's download prompt; nothing is downloaded without that confirmation.
- While the job runs the form is disabled, a progress line names the media file and the percentage, and **Cancel** stops it; the rest of FreeCut stays usable, and the job also shows in the task list (PM-073). A second Generate while one runs is queued (AI-001).
- When done, a one-line result names the number of captions and the receiving track, with **Caption list** (opens the row above on that track) and **Undo**.
- Clicking a look tile applies it to the active caption track (TX-051's op); its hover `+` with no caption track creates one with a sample caption at the playhead (FreeCut default).
**Edge cases:**
- No clip with audio in the chosen scope: Generate is disabled with the reason.
- A build compiled without speech recognition (architecture C48): the speech and song rows show an empty state that says so, and every other row works.
- Target caption track locked: Generate is disabled with a hint, as TX-038 says.
- Offline with the model missing: the prompt offers **Choose file…** (AI-002).
- Unknown or unsure language detection: the result stays, and **Run again in…** re-runs with a chosen language through the same replace rule.
- Narrow asset panel (< 360 pt): the side list becomes a chip row (ui-concept §8.2) and the form keeps one column.
**Undo:** n/a for the drawer itself (form values are preferences). Each action is its owner's op: `InsertCaptionSegments` (TX-025, TX-030), `ImportSubtitles` (TX-031), the "Auto captions" compound entry of AI-010 (its `AttachTranscript` changes plus TX-038's `ReplaceCaptions`), TX-051's look op.
**Model notes:** The drawer, its forms and their last-used values are `freecut-app` (`ui::captions`). The form builds an `AutoCaptionRequest` (scope, `LangChoice`, second-line choice, `FillerMode`, replace flag, model tier, style choice) that the app hands to AI-010's job; nothing in `freecut-timeline` changes until the job's result is committed. Design and build plan: [speech-and-captions.md](../architecture/speech-and-captions.md).

## Text round 2 — next build scope

The next build after the mvp text slice takes text to "everything a user expects from a modern short-form editor's text tools", in FreeCut's own layout. It pulls these features forward from M7 and M8 (see the [roadmap](README.md#roadmap)); their priority stays v1.

**Builds in round 2:**
- Inspector and Drawer: TX-054 (the text lenses Content, Style, Box & bubble, Motion and Presets, with card reset and card keyframe commands; presentation from the UI concept's slices S1, S2, S3 and S7) and the Text Drawer contents (TX-001).
- Remaining mvp items of the basic panel that are not built yet: TX-003's face dropdown, sections and hover preview, and TX-005's justify, vertical anchor and side handles. The side handles and TX-055's tail handle are built together on the player's existing selection overlay (which already uses the renderer's laid-out boxes).
- Basic panel: TX-008 (underline, strikethrough, case control), TX-009 (letter and line spacing), TX-010 (outline), TX-011 (shadow), TX-012 (glow, with direction), TX-013 (background box), TX-014 (gradient fill), TX-016 (curve).
- Bubbles: TX-055.
- Looks: TX-019 (style presets, including the Statement group except the gated looks) and TX-020 (Save look, My looks, copy and paste style, default style).
- Keyframes: TX-021 (row and card keyframe pattern on every text property).
- Animation: TX-022 (In, Loop, Out, the Motion lens and its timing strip, Pace, composition order), TX-023 (per-unit), TX-056 (the catalogue), with FX-018 and FX-019's slot model and FX-030's motion definitions.
- Shared groundwork these need, built only as far as text uses it: TR-027, TR-028, TR-030 and TR-031 (keyframes, diamonds, navigation, easing presets); UI-006 and UI-007 (library grids, search, hover previews); UI-009's keyframe diamond; UI-064 (save-as-preset dialog); TL-034's attribute clipboard for the Text style and Animations categories.

**Demo.** A user can add a title, set its font, size, capitals, spacing, outline, glow and shadow, put it in a speech bubble whose tail points at the speaker, apply a Statement look and save it to My looks, keyframe its colour, and animate it with a letter-rise In, a wave Loop and an erase Out, previewed live and exported identically.

**Feasibility on the current stack (checked 2026-10-12; build order in the round-2 plan, README C46):**
- *CPU compositor (README C44).* Everything above runs on `freecut_render::cpu`, which stays the reference renderer for preview and export. It needs four additions, none of them blocking: (1) per-glyph rasters and placement (today a text block is one raster), needed by per-unit animation (TX-023), curve (TX-016), per-character boxes and Decode; (2) outline, glow and shadow computed from a distance transform of the glyph coverage mask (the CPU counterpart of the SDF passes in the text architecture), with blur radii capped and downsampled as TX-011 says; (3) a premultiplied text layer, because the canvas is straight-alpha RGBA8 today and TX-012's additive glow cannot be expressed in straight alpha; (4) a projective (3×3 homography) sampler for the 3D turns of Flip open (TX-056) and FX-030's 3D flip. Box, bubble and decoration shapes are filled and stroked with `zeno`, which swash already depends on.
- *Real time.* Static text is cached as a layer raster; animated text is recomputed every frame. Acceptance target: a 1080p preview of one title with outline, shadow, glow and a per-letter In plays at the sequence rate on the reference machine. When a frame is late, the preview's existing frame-drop policy applies; export is never degraded.
- *egui 0.36.* Every widget the lenses need is stock or a small custom painter (ui-concept §16): cards, segmented controls, steppers, the timing strip, the gradient stop bar and the keyframe navigator. Preset tiles show a poster rendered once per FreeCut version into the media cache (PM-074) by a render worker; only the hovered tile animates, at 160 × 120 px and 15 fps.
- *Not in round 2 because the groundwork is missing:* on-canvas edit mode (TX-002's caret on the Stage). Range-based commands (TX-008's **Change case** on a selection) work from the Content lens's text field instead. The radial gradient's on-canvas centre handle (TX-014) is left out to keep the canvas work to the two handles above; the centre is set with fields until then.

**Stays v1, after round 2:** on-canvas text edit mode (TX-002) and the gradient centre handle (TX-014), TX-007 (missing fonts), TX-015 (per-range styling), TX-018 (font import), TX-024 (text templates), the caption features TX-025 to TX-037, the Follow card (with AI-008) and FX-017 (paste animations onto other clip kinds).

**Stays later:** TX-017 (vertical text), TX-038 to TX-053 (speech, translation, lyrics, dictation, image fill, word colours, stacked and long shadows, readability checks, font collections, caption looks, progressive display, caption emoji), the Speech card with TX-043, the gated Statement looks that need TX-046 to TX-048, and OV-028's stand-alone bubble elements. The captions build later moved TX-038, TX-039, TX-044 and TX-051 to v1 (see [Captions & speech build](#captions--speech-build--scope)).

**FreeCut round-2 model (S1, 2026-10-12).** As built in `freecut-timeline::model::text`; where it differs from the model notes of TX-008 to TX-016, TX-054 and TX-055, this paragraph wins.
- *Units.* `PerMille` (TR) is unsigned, so the signed values use `SignedPerMille(i16)`: letter spacing (−200..=1000), curve amount (−1000..=1000, the UI's −100 to +100) and tail bend (−1000..=1000, same scaling). Angles are `DeciDegrees(u16)`, tenths of a degree in `0..3600`, wrapped on construction (−10° is 350°). Every style struct has `clamped()`, and a style is valid when clamping changes nothing; edits clamp, they never refuse an out-of-range value.
- *Style.* `TextStyle { font, size, fill: Fill, bold, italic, decoration: Decoration { underline, strike }, case: CaseTransform, letter_spacing, stroke: Layer<Stroke>, shadow: Layer<Shadow>, glow: Layer<Glow>, background: Layer<TextBox>, bubble: Layer<TextBubble>, curve: Layer<SignedPerMille> }` with the defaults of each feature; every layer starts off. Borders of boxes and bubbles are `Layer<Border { color, width }>` instead of `Option<(Rgba8, PerMille)>`, so a border switched off keeps its colour and width. A bubble's tail is `BubbleSpec::tail: Layer<Tail { tip: TailTip { x, y: RefPx }, base, bend }>`; the same switch drives Thought's trail. `BoxHeight::LowerHalf` with the Block shape is kept but drawn as Full. `TextStyle::is_curved()` is the Curve layer on with a non-zero amount.
- *Fill.* `Fill::{Solid(Rgba8), LinearGradient { angle, stops, extent }, RadialGradient { center, spread, stops, extent }}`. `GradientStops` can only be built with 2–8 stops at 0–1000, sorted by position (equal positions keep their order). Switching kinds (`Fill::with_kind`) keeps stops and extent between linear and radial; a solid colour becomes a gradient from that colour to the same colour at half brightness (FreeCut default); a gradient becomes solid in its first stop's colour. Edits of a gradient's part (angle, centre, spread, extent, stops, one stop's colour) leave fills without that part alone.
- *Layers and exclusivity.* `TextStyleEdit::SetLayer { layer: TextLayerKind, on }` switches one of Outline, Shadow, Glow, Background, Bubble, Curve; off keeps the settings. Background on switches Bubble off and the reverse, in the same entry. Switching the bubble on checks the text fill against the bubble fill with WCAG's contrast ratio in integer arithmetic (a lookup table, no floats); with a gradient every stop must reach 3:1, else the bubble fill becomes `#1E1E24`. `validate` reports `TextBoxAndBubble` for a clip with both on, separately from `TextRange`; `TextClipData::clamped()` (for loaders) lets the bubble win, as the edge case above says. Choosing a bubble kind also sets the tail switch to that kind's default (on for Rounded, Oval, Square, Shout and Thought) and keeps the tail's settings.
- *Tail tip default.* The model cannot know the laid-out box, so the stored default is the TX-055 rule applied to the factory text ("Your text" at 64 ref px in a default bubble): (−90, 135) ref px from the box centre. The app may send the rule applied to the real box (`BubbleTailTip`) in the same gesture as the switch. Stored tips are clamped to ±40,000 ref px per axis; the 40 em drag stop is the editor's.
- *Tail base (FreeCut default, renderer).* The base is centred on the point of the body outline nearest the tip, and its ends lie on the outline either side of it, equally far along the outline, with the straight distance between them being the tail width (24 % of the shorter side by default). So the width holds wherever the tip is, and a tip beyond one end of a long caption leaves from that end. A Shout's base ends move to the nearest valleys between spikes (at least the two around the base centre), unless those valleys are further apart than the widest base (60 %), as at the ends of a long burst; then the base keeps its own width on the spike.
- *Paragraph.* `TextLayout` gains `justify: bool` and `v_anchor: VAnchor::{Top, Middle, Bottom}`. Justify is stored beside the alignment: `TextLayoutEdit::Align` sets the alignment and ends justification (the four buttons are one choice), `TextLayoutEdit::Justify(true)` keeps the alignment for when it is switched off. The text layer's origin is the box's horizontal centre and the vertical anchor's edge (see `freecut_timeline::plan`).
- *Edits.* `TextStyleEdit` has one variant per property of the Style and Box & bubble lenses (outline, glow, shadow, background, bubble and fill parts, decoration, case, letter spacing, curve amount) plus `SetLayer`; editing a layer that is off keeps it off. `TextLayoutEdit` gains `Justify` and `VAnchor`.
- *Property paths and cards.* `TextPropPath` lists every text property with a stable string (`text.size`, `text.fill.angle`, `text.outline.width`, `text.background.border.on`, `text.bubble.tail.tip`, `text.curve.amount`, …; layer switches end in `.on`). `card_props(TextCard)` gives each card's paths in row order, `TextCard::lens()` and `TextLens::cards()` the lens grouping, `TextPropPath::copy(from, to)` copies one property (the `text.fill.kind` path carries the whole fill), and `is_animatable()` marks TX-021's keyable rows (gradient stop colours get stop-by-stop addressing with the keyframe engine).
- *Reset.* `EditRequest::ResetText { seq, clips, scope: ResetScope::{Card(TextCard), Lens(TextLens)} }` puts the scope's properties back to the factory default style (TX-020's user default replaces it in S3, keyframe removal comes in S2), keeps the content and every layer's on/off switch, and is one entry over all listed text clips. The Text card resets nothing; the Content lens resets the paragraph; the Box & bubble lens also resets its Placement card, the clip transform, to identity (opacity included). A bubble that stays on while its colour is reset follows TX-055's switch-on rule (FreeCut default): if the text fill would contrast less than 3:1 with the factory white, the bubble takes `#1E1E24`, so a reset never leaves white text on a white bubble. The tail tip resets to the model default above, which the model cannot measure; with the bubble on, the app is to send TX-055's rule for the measured box in the same entry (`BubbleTailTip` after `ResetText`), as it does on switch-on. `TextLens` has Content, Style and Box & bubble; Motion joins with the animation engine.
- *Case mapping.* `case::display(text, CaseTransform, Option<&LangTag>) -> Cow<str>` (the renderer maps before layout) and `case::change_range(text, byte range, case, lang)` use Unicode's full default mappings (`ß` → `SS`, `ﬁ` → `FI`), the Turkish/Azerbaijani i rules for `tr` and `az`, final sigma, and Unicode's titlecase mapping wherever it differs from the uppercase one (`ǆ` → `ǅ`, `ß` → `Ss`, `ﬁ` → `Fi`, `և` → `Եւ`, `ᾀ` → `ᾈ`; Georgian letters keep their form, since Mtavruli is only their all-caps form), so Title case applied twice changes nothing more. A word starts at a letter after anything but a letter, digit or combining mark, and an apostrophe between letters stays inside the word ("don't" → "Don't"); combining marks before the apostrophe are skipped, so decomposed text ("José's", `e` + U+0301) behaves like composed text; Title case lowercases the rest of each word. A sentence ends at `.`, `!`, `?`, `…`, `‽`, `。`, `！` or `？`, optionally followed by closing quotes or brackets, then whitespace; abbreviations count as ends. `edit::change_case(doc, seq, clip, selection, case, lang)` builds **Change case** as a `SetTextContent` (the range is widened to character boundaries; an empty range changes nothing). `LangTag` exists for the mapping; the per-clip language field of TX-006 is not in `TextStyle` yet, so display mapping uses no tag until it is.

**FreeCut round-2 UI (S1, 2026-10-12).** As built in `freecut-app` on the S0 card grammar (`ui::style_cards`, `ui::box_cards`, `ui::props`, `ui::stage`, `ui::inspector`); where it differs from TX-054's table or the UI concept, this paragraph wins until they are updated.
- *Content lens.* The Text card holds the content field and a **Change case** row (capitals, small letters, Title, Sentence) that rewrites the field's selected characters as one `SetTextContent` (`edit::change_case`); it is disabled without a selection. The Paragraph card has one four-way alignment choice (left, centre, right, justify: `Align` or `Justify(true)`), the box width switch and field (the field says "paused" while the text is curved), and behind More the vertical anchor. Line spacing stays in the Style lens's Spacing card, as `card_props` files it, so **Reset card** on Paragraph and Spacing reset exactly what each card shows. A disabled **Convert** card stands in for TX-035.
- *Style lens.* Type: font, size (logarithmic slider, typed 4–1000, **−**/**+** steppers of 1 ref px, 10 with Shift), emphasis **B I U S**, and the case as four segments (As typed, UPPER, lower, Title); Sentence case is a button behind More and lights no segment. The **Face** dropdown, also behind More, offers Regular, Italic, Bold and Bold italic: `FontRef` names only a family, so a face is the bold and italic switches, set together as one entry. Fill: Solid / Linear / Radial; a gradient shows the stop bar (click a marker to pick its stop, drag it to move it; a dragged stop stops at its neighbours, so stops never swap places), the picked stop's colour with add (the widest gap's midpoint, coloured halfway), remove (blocked at two) and reverse, the linear angle or the radial centre X/Y and spread fields, and behind More the extent (whole text or each line). The radial centre handle on the Stage stays out of round 2. Spacing: letters (−20 to +100 %) and lines (50–300 %), each with steppers of 1 %, 10 % with Shift.
- *Optional layers.* Outline, Shadow and Glow cards show once added (switched on, or switched off with changed settings, which the chip shows as "off · …"); the others are offered on an **Add layer** row, one entry each. Each card's `...` menu has **Reset card** (`ResetText` of the card; the switch is kept) and **Remove layer** (switched off and reset, one entry). Essentials are colour and width (Outline), colour and blur (Shadow), colour and radius (Glow); the other rows are behind More.
- *Box & bubble lens.* Placement, then Background, Bubble and Curve cards, always listed with their switches. Switching one of Background and Bubble on switches the other off in the same entry (the model's `SetLayer`), and the other card's chip says why it is off ("off · the bubble is on"); while the text is curved both chips read "paused while curved" and the cards say so above their still-editable rows. Bubble: kind dropdown (the eight TX-055 kinds by name; Arrow tag adds a Left/Right row), tail (Thought: trail) switch, colour, the kind's extras (Shout: spikes, depth, irregularity and a dice that rolls a new seed; Thought: lobes and circles), and behind More opacity, tail width and bend, padding, roundness and the border. Switching the bubble on while its tail tip is still the model default sends TX-055's rule for the measured text box (a quarter of the body width left of centre, 1.2 em below the body; body = text plus padding, at least 2 × 1.5 em) in the same entry. Kind thumbnails are not drawn yet; the dropdown lists names.
- *Footers.* Content, Style and Box & bubble end with **Reset lens** (`ResetText` of the lens, one entry). **Save look…** comes with S3.
- *Stage.* With one text clip selected (not on a locked track, not playing) the player draws white side handles mid-way up the box's left and right edges (hidden while curved) and, for a bubble whose kind has a tail and whose tail is on, a round handle at the tip. A side drag sets a fixed width (an auto-width box switches to fixed) with the opposite edge kept by moving the clip's position by half the change, or about the centre with Alt; width and position go into one entry. A tip drag sets `BubbleTailTip`, stopping 40 em from the box centre. Esc cancels either drag with no entry; a click on a handle keeps the selection. Handles use the preview thread's laid-out boxes, which now follow the vertical anchor.
- *Read-only.* A text clip on a locked track shows its lenses disabled under UI-008's banner.
- *Settled in review (2026-10-13).* **Several clips** (UI-010): the cards show the first clip's values; a row whose value differs across the selected text clips shows a dash (fields, colour hex, font and face) or lights no segment, shows its reset arrow, and whatever is picked or typed goes to every clip, the first clip's own value included, as one entry. An edit of one part of a compound value is made per clip from that clip's own value, one entry in all: a radial centre's X keeps each clip's Y, and moving, adding, removing or reversing stops works on each clip's own stops (stop *n* of each). **Stop bar:** a press picks the marker it lands on (even when the drag is reported a few points later), the marker keeps its offset from the pointer, and of stops that met, the drag's direction picks (the upper one rightward, the lower one leftward), so met stops can always be pulled apart. **Stage handles** keep their offset from the press, so a grab beside a handle never jumps. **Reset** of the Bubble card, or of the Box & bubble lens, with the bubble on sends TX-055's tail tip for the measured box in the same entry.

## Captions & speech build — scope

The captions build (requested 2026-10-13) brings captions to what users expect from a modern short-form editor: typed and imported captions, captions generated from speech on the user's own machine, filler-word marks, lyrics, caption looks and a second-language line. Like Text round 2 it pulls features forward from M8 and M9. Unlike round 2 it also changes priorities, because the user asked for these features now: TX-038, TX-039, TX-044 and TX-051 (with AI-002, AI-009 and AI-010) move from later to v1 and from M9 to M8; TX-041 is built only in part (C51's engines 1–4) and stays later. Nothing is gated: the reference editor reserves its second-language line and filler marks for subscribers; FreeCut does not. The design, the engine choice and the slice contracts are in [speech-and-captions.md](../architecture/speech-and-captions.md) (architecture README C47–C51).

| Slice | Builds | Demo |
|---|---|---|
| **CS1** Caption model, files, list, burn-in | TX-025, TX-026, TX-027, TX-028 (track scope), TX-031 and TX-032 (SRT, WebVTT), TX-033 (burn-in and sidecar), TX-037, TX-057 (Type or paste, Files, Caption list) | A user types captions or imports an SRT, fixes text and timing in the list, restyles the track and exports an MP4 with the captions burned in plus an SRT file. |
| **CS2** Local speech recognition and Auto captions | AI-001 (minimal pool), AI-002 (speech packs), AI-009, AI-010 (Recognize speech), TX-038, TX-039 (timings stored), TX-057 (From speech) | After one confirmed model download, a user generates captions for an interview in its own language, detected automatically, and works offline from then on. |
| **CS3** Filler words, lyrics, caption looks | AI-010 (Mark filler words, song lyrics, Align known text), AI-012 (lexicons), TX-044, TX-031/TX-032 (LRC), TX-039 (word styles drawn), TX-051 (six FreeCut looks), TX-019 caption presets, TX-057 (From a song, Looks, My styles) | Fillers show dimmed or are left out; a song gets line-by-line lyrics with a moving word highlight; a look restyles the track in one click. |
| **CS4** The second line | TX-036, TX-041, AI-013 (windows and split-back), TX-057 (Second line) | A user adds a typed second line, an English line from foreign-language speech, or a translation from their own assistant provider, and exports both lines burned in. |

**FreeCut decisions for this build (2026-10-13):**
- *Engine.* Speech is recognized by whisper.cpp through the `whisper-rs` binding in `freecut-analysis`, behind the cargo feature `ml-whisper` (on by default in the app as `stt`; the app also builds without it). Models are Whisper's MIT weights in ggml form, downloaded only after the user confirms (AI-002, AI-009).
- *Segments are clips* on `TrackKind::Caption` tracks (architecture C5); word timings are stored relative to the segment start, so moving a caption never rewrites its words.
- *Filler marks are word flags*, not rich-text spans: `WordFlags::FILLER` on a segment's word timings, drawn with the look's Filler word style (default 50 % opacity). This replaces TX-038's `SpanOrigin::Filler` until TX-015's spans exist; TX-040's emphasis spans are unaffected.
- *Second-line text* comes, in this order of availability, from typing, from an attached subtitle file, from Whisper's built-in translation into English, from the user's assistant provider (opt-in, text only, AS-004 disclosure each run), and later from local translation models (AI-013).

## Interactions with other areas

- **Timeline & core editing ([timeline-editing.md](timeline-editing.md)):**
  - TL owns time representation (TL-001), tracks and layering (TL-004, TL-006, TL-007), clip placement and `InsertClips`, selection, trim, split (TL-023), groups (TL-039), links (TL-011), ripple and undo semantics (TL-043).
  - Text clips are ordinary clips on `Text` tracks. Caption segments live on `TrackKind::Caption` tracks (TL-004), which accept only caption segments. TX-025 owns their payload.
  - TL-004 orders the visual bands Video, then Text/Sticker/Effect/Adjustment, then Caption. The Caption band stays on top under Flexible stacking (TL-007), and TL-006 never auto-removes an empty `Caption` track.
  - TL-011's links accept `LinkMember::CaptionSegment(TrackId, SegId)` as well as clips, so generated speech clips can link to caption segments (TX-043).
  - TL's split op applies FX's animation split rule, with the text Loop rule of TX-022.
  - TX registers the **Text style** category in TL-034's Paste attributes registry (TX-020). Template groups use TL-039, and speech regeneration uses TL-035's `ReplaceSource` (TX-043).
- **Transform, keyframes & speed ([clip-transform-and-speed.md](clip-transform-and-speed.md)):**
  - Text clip position, scale, rotation and opacity, the keyframe engine (TR-027's `Animatable<T: Interpolate>`) and easing all come from TR.
  - TX only registers its animatable properties (TX-021). Following the scalar rule in TR's shared conventions (only time must be rational, and each spec picks its own non-time scalar type), they use TX's fixed-point style types as `T`.
  - Using text as a mask over media belongs to TR masks.
  - Corner handles in the preview are TR-007's scale. Side handles of a text box are TX-005's width.
- **Effects, transitions & clip animations ([effects-transitions-animations.md](effects-transitions-animations.md)):**
  - FX owns per-frame render order (FX-001). Text and caption layers enter at L8–L10 (step 4), and step 2 composites the Caption band after all other visual tracks (TX-025).
  - FX owns the clip-animation slot model and its per-clip-kind policy (FX-018 Entrance/Exit, FX-019 Continuous), which covers video, image, text and element clips. Video, image and element clips use exclusive slots. On text clips Loop coexists with In and Out and runs in `[in, d − out)`, a window TX-022 defines. TX supplies text presets and per-unit parameters (TX-023).
  - Transitions between text clips use FX transitions.
  - FX-030 defines the motion and manifest format of every animation preset. TX-056's text catalogue reuses its whole-block motions (distances in em) and adds per-unit presets on the same channel model. FX-030's Intensity is the Intensity control of TX-022.
  - A blurred-text look is FX-015's blur effect on the text clip (TX-012). Animations travel in FX-017's Paste attributes category, not in TX's Text style category (TX-020).
- **Overlays, stickers & templates ([overlays-and-stickers.md](overlays-and-stickers.md)):**
  - Shapes (OV-024) and stickers come from OV, and text templates may include OV shape layers. Bubbles around a text clip are TX-055's, part of the text style. OV-028's stand-alone bubble elements reuse TX-055's `BubbleSpec` and `bubble_path`, so both draw the same shapes.
  - Caption-look decorations (TX-051) are OV elements (`ElementSource`) drawn inside the caption layer.
  - Whole-project templates (OV-030 to OV-035) belong to OV. Text templates (TX-024) are TX's.
  - OV-043's local brand kit leaves brand fonts to TX: font collections (TX-050) and style packs (TX-020) serve that role. Brand colors feed UI-009's shared color picker, which TX uses.
- **Audio ([audio.md](audio.md)):** Generated speech (AI-025, TX-043) is an ordinary audio clip that AU mixes. Its initial volume uses AU-005's range, and its pitch is AU-024's pitch shift. Ducking under voice-over and fades are AU features. Dictation (TX-045) uses AU-027's input device. Restyling a recorded voice is AU-022's voice effects, not a TX feature.
- **Smart / AI tools ([ai-tools.md](ai-tools.md)):**
  - AI owns the jobs, dialogs and models behind TX-038 to TX-045: transcription (AI-009, also used for dictation in TX-045), auto captions (AI-010), caption translation (AI-013), vocal isolation (AI-023) and text-to-speech (AI-025), plus the model manager (AI-002) and the `freecut-analysis` crate.
  - For speech, AI-025 owns the voices, voice preview, sentence chunking and synthesis. TX-043 owns the speech controls (source, speed, pitch, volume, Fit to caption, Keep in sync, merged or per-caption clips), the size limit and the `GenerateSpeech` op, which AI-025 refers to.
  - Motion tracking (AI-008) binds text clips like other overlays. Resizing a tracked text box keeps the binding (TX-005). Its controls are the Follow card of the Motion lens (TX-054), and animation offsets are added after the tracked position (TX-022's composition order).
  - TX owns what those jobs produce: caption tracks and segments, `CaptionRules` (TX-037), word timings and the active-word highlight (TX-039), emphasis and filler spans, the secondary line (TX-036), and the `ReplaceCaptions` op (TX-038).
  - AI also owns transcript-based editing (AI-011) and filler-word cleanup (AI-012), which consume the word timings and filler spans on caption segments.
  - Recognition is per media file: AI-009 transcribes each file once, and AI-010 maps the words through each clip onto the timeline. AI-010's results, its Re-sync captions and its Align known text mode (TX-030, TX-044) all land through `ReplaceCaptions` (TX-038).
  - AI-013 owns the translation context window and the rule that splits translations back onto segments. TX-041 receives one text per segment.
  - All inference runs in `freecut-analysis`. Line-rule defaults live only in TX-037, and AI-010 points to them.
- **Color ([color-and-filters.md](color-and-filters.md)):** Text and caption colors are sRGB and are converted into the working space by CL's color pipeline. Filters on adjustment layers above text affect text like any other layer.
- **Preview & export ([preview-and-export.md](preview-and-export.md)):**
  - EX hosts the export dialog's Captions section (TX-033, EX-045) and the font-substitution warning (TX-007).
  - Sidecar caption files from the export dialog are named after the video file stem (EX-045). Files from the Captions panel's **Export subtitles…** are named after the project (TX-032).
  - EX owns title-safe and action-safe overlays in the preview (EX-011), which TX-036, TX-049 and TX-051 use as their fit area, and provides the playback clock used by tap-to-sync (TX-030).
  - EX-036's preflight lists font substitution (TX-007) and, once TX-049 ships, readability findings as Info items.
- **Projects & media ([project-and-media.md](project-and-media.md)):** PM owns project settings (canvas size, which sets the ref-px scale, and frame rate). When the ratio or resolution changes, PM-011 rescales text by TX-001's rule (lengths × short edge / 1080). PM also owns the media cache for generated speech audio, the asset index of imported fonts (PM-042), and collect and package (PM-101), which copies fonts where their licenses allow (TX-007). Image fills (TX-046) use ordinary media items, so offline handling and relinking are PM's (PM-090, PM-091). Font imports report progress in PM-073's task list.
- **Workspace & shortcuts ([workspace-and-shortcuts.md](workspace-and-shortcuts.md)):** UI owns panel docking, the inspector frame (UI-008; TX-054 fills the text lenses, whose presentation is [ui-concept §7](../design/ui-concept.md#7-the-text-inspector)), and every keyboard shortcut mentioned here. Saving text looks uses UI-064's dialog, and every text setting is reachable from the command bar (UI-034) through TX-054's property paths. The assistant ([assistant.md](assistant.md), AS-006) edits text through the same style and layout requests. TX's panels use UI-006's library frame (search, favorites, recent) and UI-009's color field. Multi-selection styling follows UI-010. A saved personal color palette, if FreeCut adds one, belongs to UI-009, not to TX.

## Out of scope (and why)

- **Online font, template, style or lyrics libraries, and cloud sync of captions.** FreeCut has no cloud and no account. All content is bundled, made by the user, or imported.
- **Cloud speech-to-text, translation or text-to-speech.** These are replaced by the optional local tools of [ai-tools.md](ai-tools.md) (AI-009, AI-010, AI-013, AI-025). TX-038, TX-041 and TX-043 define only the caption side.
- **Voice cloning, custom-voice character quotas and paid re-voicing.** Cloning raises consent and misuse risks, and suitable open models with clear licenses are scarce. [ai-tools.md](ai-tools.md) excludes it too. Without accounts there are no quotas.
- **Paywalls, free-use counters and credits** on auto captions, lyric recognition, text-to-speech, export with speech, and premium fonts, styles or templates. FreeCut is free, every feature is available, and saved styles may contain anything.
- **Sign-in for font import, font favorites and template favorites.** These are local preferences in FreeCut (TX-003, TX-018, TX-024).
- **Downloaded glow presets, text effects, templates and fonts, with their download, retry and offline-failure states.** Everything is bundled, generated by FreeCut's renderer from parameters (TX-012, TX-019, TX-051), made by the user, or imported.
- **Rating recognition quality and other feedback forms.** They would send data to a server, and FreeCut has no telemetry.
- **Publishing text effects, text templates or caption looks to an online marketplace, and creator programs.** Local substitute: style packs (TX-020), user templates (TX-024) and user looks (TX-051) are plain files that users can share however they like.
- **A team brand kit synced through an account.** Local substitute: style packs (TX-020) and font collections (TX-050), next to OV-043's local brand kit.
- **Online commercial-use scanning of fonts.** Local substitute: the font info popover shows each font's license metadata and embedding flags (TX-018).
- **Cloud script writing and copywriting that adds its result as captions.** Users paste their own text into TX-030's long-text dialog.
- **One-click online packaging (captions, keyword highlights and sound effects added by a server).** Partial local substitute: auto captions (TX-038) with keyword emphasis (TX-040) and a caption look (TX-051).
- **A stand-alone voice-over window with its own download list, and a speech-to-speech voice changer.** Speech generation lives inside the editor (TX-043, AI-025), and restyling a recorded voice is AU-022's local voice effects.
- **Language restrictions tied to the reference editor's servers** (for example emoji only for two languages, or lyrics for fewer languages). FreeCut's limits come only from the local models and open data it uses (AI-009, CLDR for TX-053).
- **Automatic speaker detection (diarization).** [ai-tools.md](ai-tools.md) lists it as out of scope. Manual speaker labels and imported voice tags are covered by TX-042.
- **Generating text styles from a prompt with AI.** This is low value relative to its cost. Presets and saved styles cover the need.
- **3D extruded or bevelled text, and text on arbitrary paths.** These are large render features. Circular arcs (TX-016) cover the common case, and the long shadow of TX-048 gives a 2D block-3D look. The reference editor's own 3D and path text come with many restrictions on styles, effects and animations, and its 3D text does not even carry over between its platforms, which confirms the cost.
- **Per-character keyframing UI.** It is too complex for the everyday workflow. Per-unit presets (TX-023) cover it, including in caption looks (TX-051). The reference editor's caption templates let keyframes be set on the first word, line or screen and copied to the rest; FreeCut does not plan that.
- **Broadcast closed-caption standards (CEA-608/708, SCC, EBU-STL, IMSC/TTML).** These are niche for this audience. They could be added later via `freecut-project::subtitles`.
- **Built-in emoji keyboard.** The OS emoji picker is used.
- **An AI avatar that reads the text on screen, and the upsell tab that offers it.** It depends on a cloud service and paid credits. Local speech from text (TX-043) covers the voice.
- **Spell check.** Deferred. It could later use open dictionaries. Until then, the Captions panel only flags characters that no font can render (TX-026).

## Clean-room notes

- **Research basis:** this spec was written from public help-center pages, third-party tutorials and the documentation of open-source editors (listed below), plus a paraphrased inventory of the reference editor's English UI labels, taken from the bundled English localization file of a local install (CapCut desktop 9.5.0; see Provenance in the [README](README.md)). Nothing was decompiled, disassembled or traced, and no project or draft files, network traffic, APIs, assets or models were inspected or used. No label text, preset, effect, template, font or voice name was copied; every label in this spec is FreeCut's own. Facts recorded from these sources (for example, that the reference editor exports only automatically recognized captions, limits caption imports to 3 MB, or caps a caption at 2 lines) are observations of behavior, written in our own words. Where FreeCut chose differently, the feature says so in one line.
- **Subtitle files and user fonts:** FreeCut reads only subtitle and font files the user chooses. Imported fonts stay in the user's font folder and are never redistributed with FreeCut (TX-018).
- **Names:**
  - All preset, template and animation names in FreeCut are our own generic descriptions. No reference-editor preset, effect, template or font names are reused.
  - The placeholder text ("Your text") and every default value marked "(FreeCut default)" are our own choices.
- **Fonts (bundled), all redistributable with software:**
  - Noto Sans, Noto Serif and Noto Sans Mono (SIL OFL-1.1), with Noto CJK and other script families as an optional pack (OFL-1.1).
  - Noto Color Emoji (OFL-1.1).
  - Liberation Sans, Serif and Mono for metric-compatible substitution (OFL-1.1).
  - A small display set: Inter, Montserrat, Bebas Neue, Anton, Oswald, Playfair Display, Lobster, Pacifico and JetBrains Mono (all OFL-1.1), and Permanent Marker (Apache-2.0).
  - OFL Reserved Font Names must not be used for any modified versions, so fonts are shipped unmodified.
- **Style presets, text animations, text templates and caption looks:** designed in-house as JSON data. Thumbnails are generated by FreeCut's own renderer. No reference-editor assets, screenshots or artwork are used in them or shipped with FreeCut (screenshots served only as research observations; see Provenance in the README). The two-tone style pairs (TX-019) use FreeCut's own palette.
- **Bubble shapes (TX-055):** eight parametric outlines drawn by FreeCut's own geometry code. No bubble artwork or vector files from any other product are used.
- **Animation catalogue (TX-056, FX-030):** every motion is defined in this spec by its channels, amplitudes and timings, written for FreeCut. No preset was traced, sampled or measured from the reference editor; the names are generic descriptions of the motion.
- **Fill textures (TX-046):** generated procedurally by FreeCut's own build-time code. No third-party texture images are bundled.
- **Emoji keywords (TX-053):** derived from Unicode CLDR emoji annotation data (Unicode License v3, attribution in UI-051's license notices). Emoji glyphs come from Noto Color Emoji (OFL-1.1).
- **Readability thresholds (TX-049):** the contrast default follows the widely published large-text contrast guideline. All other thresholds are FreeCut defaults.
- **Text stack (verify licenses at integration):**
  - cosmic-text (MIT OR Apache-2.0) for layout and shaping.
  - rustybuzz (MIT) for HarfBuzz-compatible shaping.
  - fontdb (MIT) for system font discovery.
  - ttf-parser (MIT OR Apache-2.0).
  - swash (MIT OR Apache-2.0) for scaling and rasterizing.
  - unicode-segmentation and unicode-bidi (MIT OR Apache-2.0).
  - encoding_rs and chardetng for subtitle encodings (permissive dual licenses).
- **ML models (none bundled, none run by TX):** the models behind TX-038 to TX-044 are chosen, licensed and downloaded (only with explicit consent, AI-002) by [ai-tools.md](ai-tools.md). Its license table is authoritative. For reference, the open substitutes it names are:
  - Speech-to-text: Whisper weights (MIT) run via whisper.cpp (MIT), with Silero VAD (MIT). Shotcut and Kdenlive use the same kind of local recognition. The same model serves dictation (TX-045).
  - Translation: Argos Translate (MIT) with OPUS-MT-family models (CC-BY-4.0, attribution required in About), or Bergamot/Firefox Translations models (license checked per model).
  - Text-to-speech: Kokoro-82M (Apache-2.0 weights) and Piper (MIT for the archived original, GPL-3.0 for current development; voices are licensed one by one, and only CC0, CC-BY or permissive voices are offered).
  - Vocal isolation for lyrics: Demucs-family models (MIT code, weight license verified per model).
  - Known-text alignment (AI-010's Align known text mode, used by TX-030 and TX-044): the same Whisper weights (MIT) and their word timestamps. Non-commercial models (CC-BY-NC, for example the MMS alignment weights) are excluded, because they conflict with FreeCut's free distribution.
- **Subtitle formats:** all are open or de facto standards: SRT, WebVTT (W3C), ASS/SSA (community-documented), LRC and SBV. They are implemented from their public descriptions.

## Sources

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- Unicode CLDR, emoji annotations (data used by TX-053) — https://cldr.unicode.org/
- CapCut Help Center, How do I import subtitles — https://www.capcut.com/help/how-to-import-subtitles
- CapCut Help Center, Why do captions fail to export — https://www.capcut.com/help/captions-fail-to-export
- CapCut Help Center, How do I recognise subtitles — https://www.capcut.com/help/how-to-recognise-subtitles
- CapCut Help Center, Batch editing recognized subtitle/lyrics text — https://www.capcut.com/help/batch-edit-to-recognize-subtitle
- CapCut Help Center, Bilingual subtitles — https://www.capcut.com/help/bilingual-subtitles
- CapCut Help Center, Font units in CapCut — https://www.capcut.com/help/font-units-in-capcut
- CapCut Help Center, Auto lyrics entrance — https://www.capcut.com/help/entrance-to-auto-lyrics
- CapCut Resource, Video caption generator guide — https://www.capcut.com/resource/video-caption-generator
- CapCut Resource, Create curved text — https://capcut.com/resource/create-curved-text
- CapCut Resource, Create gradient text — https://www.capcut.com/resource/create-gradient-text
- Riverside blog, How to add captions in CapCut — https://riverside.com/blog/how-to-add-captions-in-capcut
- GoTranscript, Auto-generate captions in CapCut for Windows (video transcript) — https://gotranscript.com/public/how-to-auto-generate-captions-for-videos-in-capcut-for-windows
- GoTranscript, Captions on phone and laptop with CapCut (video transcript) — https://gotranscript.com/public/mastering-captions-on-phone-and-laptop-a-step-by-step-guide-with-capcut
- AutoAE blog, Text animation in CapCut on PC — https://autoae.online/blog/how-to-do-text-animation-in-capcut-pc
- Miracamp, How to add text in CapCut — https://www.miracamp.com/learn/capcut/how-to-add-text
- Shotkit, Add text in CapCut — https://shotkit.com/add-text-capcut/
- CAMB.AI blog, Text-to-speech in CapCut — https://www.camb.ai/blog-post/add-text-to-speech-capcut-pro-tools
- BlitzCut blog, Adding custom fonts to CapCut — https://blitzcutai.com/blog/how-to-add-fonts-to-capcut
- Kdenlive documentation, Subtitles — https://docs.kdenlive.org/en/effects_and_filters/subtitles.html
- Kdenlive documentation, Speech to Text — https://docs.kdenlive.org/en/effects_and_filters/speech_to_text.html
- Linuxiac, Kdenlive 24.12 subtitle features — https://linuxiac.com/kdenlive-24-12-video-editor-released/
- Linuxiac, Shotcut 24.10 speech-to-text — https://linuxiac.com/shotcut-24-10-introduces-ai-powered-speech-to-text-feature/
- Shotcut forum, Speech to text subtitles — https://forum.shotcut.org/t/speech-to-text-subtitles/46306
- OpenShot user guide, Text & Titles — https://en.openshot.org/files/user-guide/titles.html
- Uncle (tryuncle.com), DaVinci Resolve subtitles guide — https://tryuncle.com/learn/davinci-resolve/subtitles
- Motion Array, Subtitles in DaVinci Resolve (search summary only; the page itself could not be fetched) — https://motionarray.com/learn/davinci-resolve/davinci-resolve-subtitles/
- PromptQuorum, Piper TTS review (licensing) — https://www.promptquorum.com/power-local-llm/piper-tts-review
- Piper GitHub discussion on voice licensing — https://github.com/rhasspy/piper/discussions/271
