# Text rendering, titles & subtitles

> Status: draft · Crates: freecut-render, freecut-timeline, freecut-app (plus the subtitle-format module in freecut-project and subtitle-stream muxing in freecut-media)

This document designs how FreeCut stores, lays out, rasterizes and exports text:
free-floating text clips (titles, labels), caption tracks (time-coded segments),
subtitle files (SRT, WebVTT, ASS/SSA, LRC, SBV, TXT), subtitle streams inside
exported containers, and burned-in captions.

What the user sees is fixed by `docs/features/text-and-captions.md` (TX-*), by
EX-045 in `preview-and-export.md`, and by FX-001, FX-018 and FX-019 in
`effects-transitions-animations.md`. This document makes the engineering decisions.

Out of scope here (only the interfaces are fixed):
- speech recognition, translation, text-to-speech and lyric alignment (AI, `ai-tools.md`);
- the clip-animation slot model and its evaluator (FX-018/FX-019);
- the keyframe engine `Animatable<T>` (TR);
- the FX-001 compositor walk, apart from the text layer that enters it at L8;
- export job orchestration (EX-022).

How this was researched: the source of the six reference repositories listed under
[References](#references) was read directly. Crate versions and licenses were
checked with `cargo info` and the crates.io API on 2026-10-12. FFmpeg behaviour was
checked against the local Homebrew FFmpeg 8.1.1.

---

## What FreeCut needs

### Requirements from the feature specs

| Area | Requirement | IDs | Numbers / defaults |
|---|---|---|---|
| Model | A text clip is `TextClipData { content: RichText, style: TextStyle, layout: TextLayout, origin }`. Style scalars are fixed-point. Lengths are in **ref px**, which scale with the canvas short edge (1080 ref px = short edge). Colours are sRGB `Rgba8` with straight alpha. | TX-001, TX-004 | size 4–1000 ref px, default 64; 3 s default duration |
| Rich text | Content is stored exactly as typed, UTF-8, LF only. Spans are byte ranges snapped to grapheme clusters. Spans are kept canonical: sorted, disjoint, non-empty, merged when equal. Each span has a `SpanOrigin`. | TX-002, TX-015, TX-040 | 10,000-character soft limit |
| Editing | On-canvas editing with a caret, selection, OS input methods (CJK composition) and the OS emoji picker. Caret moves by grapheme cluster. One undo entry per edit session. | TX-002, TX-006 | — |
| Fonts | Fonts come from three sources: bundled, system and imported. System discovery runs on a background thread. When two sources have the same family, imported wins over system, and system wins over bundled. Variable-font named instances appear as faces. Colour and emoji fonts are listed. Fonts that fail to parse are skipped. | TX-003, TX-018 | — |
| Missing fonts | The stored `FontRef` is never overwritten. A substitute is chosen by class, weight and italic. Metric-compatible aliases are used (Arial → Liberation Sans, …). The user can **Rescan**. A version mismatch is noted. | TX-007, PM-042 | — |
| Synthetic styles | Bold uses a real face with weight ≥ 600 when one exists, otherwise an outline embolden. Italic uses an italic face, otherwise a slant. Synthetic faces are marked. | TX-004 | embolden ≈ 2 % em, slant 12° |
| Paragraph | Alignment left, centre, right or justify (the last line is start-aligned). Box is auto width or fixed width. Vertical anchor top, middle or bottom. UAX #14 line breaks. Mid-word break only as a last resort. Trailing spaces are ignored for alignment. Empty lines are kept. Paragraph direction is detected automatically. Alignment is visual. | TX-005 | fixed width 20–10,000 ref px, at least 1 em |
| Scripts | HarfBuzz-compatible shaping and bidi. Fallback is per character: selected font → bundled Noto for the script → system → visible missing box. A BCP 47 language tag drives `locl` forms and case mapping. Emoji ZWJ, flag and skin-tone sequences work. Each line's height uses the tallest font on that line. A "fallback used" hint lists the characters. | TX-006 | — |
| Decorations, case | Underline and strike use the font's metrics, are drawn per line and follow curves. Case transform is display-only and locale-aware. | TX-008 | — |
| Spacing | Letter spacing goes after each cluster except the last on a line, and never inside cursive joins. Line spacing is a factor of the natural line height. | TX-009 | −20 %…+100 % of size; 50–300 % |
| Effects | **Outline:** outside the glyphs, round joins, a union outline without seams, drawn beneath all fills. **Shadow:** cast by fill + outline (and the box, optionally). **Glow:** additive. **Box:** block or per line, composited once. **Gradient:** 2–8 stops; each glyph keeps its rest-pose colour while animating. **Order:** box → shadow → glow → outline → fill → decorations. Blur is capped and computed at reduced resolution. | TX-010…TX-014 | outline 0–30 % em; shadow blur 0–50 %, distance 0–100 %; glow radius 0–100 %; blur cap 256 device px at 4K |
| Curve | Arc angle = \|curve\| × 3.6°. Lines are concentric. Alignment is measured along the arc. A curve disables fixed width and the box. | TX-016 | −100…+100 |
| Keyframes | Fill, size, spacing, outline, shadow, glow, box, curve, gradient angle and **Reveal %** can be keyframed. | TX-021 | — |
| Animation | In, Out and Loop presets. Per-unit animation (grapheme, word or line) with overlap and order, including random order with a stored seed. Whitespace takes no time. A ZWJ emoji counts as one unit. Results are deterministic. The box covers only revealed glyphs. | TX-022, TX-023, TX-013 | overlap default 50 % |
| Captions | A `Caption` track kind sits at the top of the stack. Segments never overlap and may have gaps. Minimum length is 1 frame. Style cascades track → speaker → segment → spans. Features: secondary line, word timings with active-word highlight, line rules and re-flow. | TX-025…TX-029, TX-036, TX-037, TX-039, TX-042 | 5,000+ segments in the panel |
| Import | Formats: SRT, VTT, ASS/SSA, timecoded TXT, LRC, SBV. Encoding: byte-order mark → UTF-8 check → detector, with an override and a live preview. Bad cues are skipped and reported with line numbers. Overlapping cues go to extra tracks. A 1-hour start offset is detected. 50,000+ cues import as one operation. | TX-031 | — |
| Export | Formats: SRT (default), VTT, ASS, TXT, LRC. Cues are sorted and renumbered. Times are rounded to milliseconds, half up. When end equals start, end gets +1 ms. Output is UTF-8 with an optional byte-order mark; SRT defaults to CRLF. Style tags are an option. Writes are atomic. One file per track. | TX-032 | — |
| In the video | Each caption track is **burned in**, **embedded** or left out. Embedded codecs: MP4/MOV mov_text, MKV SRT or ASS, WebM WebVTT. Embedding is disabled for GIF and audio-only output. The first embedded track gets the default flag. Range exports are rebased. | TX-033, EX-045 | — |
| Pipeline | Text enters FX-001 at L8 as a premultiplied layer. Glyphs are rasterized at the final output resolution, including TR scale. Preview equals export. Nothing is decoded, rendered or read from disk on the UI thread. Layout results reach the app asynchronously. | FX-001, TX-004, TX-005 | — |

### Engineering requirements derived from the above

- **R1 Resolution-independent layout.** Glyph positions and line breaks must be identical at every preview resolution, every TR scale and every export resolution. Wrap points never move between preview and export.
- **R2 Exact time.** TL-001 ticks are 1/705,600,000 s, so 1 ms = 705,600 ticks and 1 cs = 7,056,000 ticks. Every subtitle time format therefore converts to ticks **exactly**. Rounding happens only when writing a file.
- **R3 UI thread stays free.** Shaping, fallback, font file reads and rasterization never run on the UI thread. The UI shows the latest result it has received.
- **R4 Cheap keyframed effects.** Keyframed outline width, glow radius and per-glyph blur must re-render every frame without rasterizing glyph outlines again.
- **R5 Union-correct outlines.** Where glyph shapes overlap (cursive joins, negative letter spacing, emoji next to text), no internal seam may show, even when the fill is transparent.
- **R6 Portable randomness.** Random animation order must give the same result on every OS, every C library and every Rust version.
- **R7 Reveal never reflows.** Partially revealed text keeps the line metrics of the full text.
- **R8 Scale.** Finding the caption at time t costs O(log n) for 50k segments. Importing 50k cues is one undo entry.
- **R9 No stale frames.** Every input that changes text pixels takes part in `render_key` (EX-016): font files, fallback configuration, rasterizer version and the live edit buffer.
- **R10 GPU limits.** A layer texture never exceeds the device's `max_texture_dimension_2d`. When it would, quality degrades instead of failing.
- **R11 Robust parsers.** Parsers never panic (they are fuzzed). A bad cue is reported and skipped, never fatal.
- **R12 Contained FFI.** Subtitle-stream FFI lives only in `freecut-media`, each `unsafe` block with a `// SAFETY:` comment.
- **R13 Font licensing.** Fonts whose embedding flags forbid it are never copied. FreeCut never installs a font into the OS.
- **R14 Self-sufficient fonts.** With zero system fonts, the bundled fonts alone render everything. This covers CI, minimal Linux installs and golden tests.

---

## How the reference editors do it

Kdenlive and Shotcut are Qt front-ends on MLT, so MLT holds a large part of their text
rendering. Olive and libopenshot render text with Qt's `QPainter` inside their own
engines. OpenTimelineIO has no text model.

### Kdenlive (C++/Qt on MLT; mostly `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`, some titler files MIT)

**Subtitle model:**
- `src/bin/model/subtitlemodel.hpp` stores subtitles as
  `std::map<std::pair<int, GenTime>, SubtitleEvent> m_subtitleList`: keyed by
  **(layer, start time)**, with a separate id map `m_allSubtitles`.
- Two subtitles therefore cannot start on the same frame of a layer. Moving one means
  re-keying it. The tests codify this limit (`tests/subtitlestest.cpp`: "Ensure 2
  subtitles cannot be place at same frame position", "Ensure we cannot cut overlapping
  subtitles").
- `SubtitleEvent` and `SubtitleStyle` (`src/definitions.h`) copy ASS fields one to one:
  layer, style name, actor name, margins, effect, and text with raw ASS override tags.
  The style has the 23 ASS style fields. **The internal model is ASS.**

**Rendering:**
- The constructor connects `modelChanged` to `jsontoSubtitle(toJson())`. After every
  change, `saveSubtitleData` rewrites a whole `.ass` work file.
- An MLT `avfilter.subtitles` filter is attached to the tractor (`jsontoSubtitle`,
  `src/bin/model/subtitlemodel.cpp`). This is FFmpeg's libass-based `subtitles` filter,
  and the global style goes through `av.force_style` (`setForceStyle`).
- Preview and export therefore share libass. The cost is a full file rewrite and a
  libass re-parse on every edit, plus a hard dependency on an FFmpeg built with libass.
- The Homebrew FFmpeg 8.1.1 on this machine has no libass: `ffmpeg -filters` lists no
  `subtitles`, `ass` or `drawtext`, and `-buildconf` has no `--enable-libass` or
  `--enable-libfreetype`.

**Import** (`SubtitleModel::importSubtitle`):
- The format is chosen with `filePath.endsWith(".srt")` and similar checks, which are
  case-sensitive.
- SRT, VTT and SBV share one line-state machine. VTT is handled with a
  `defaultTurn = -10` offset.
- A line counts as a timing line if it contains `-->` **or** matches the regex
  `([0-9]{1,2}):([0-9]{2})`. A text line such as "4:14 bravo" is therefore at risk, and
  the fixture `tests/dataset/subs-with-two-dots.srt` guards against exactly that.
- ASS event lines are passed through `line.simplified()`, which collapses runs of
  spaces.
- The `Format:` line is ignored. `SubtitleEvent(const QString&, …)` in
  `src/definitions.cpp` reads `parts[0..8]` in a fixed order and joins the remaining
  parts as text. Commas in text survive (fixture `subs-with-commas.ass`), but
  reordered fields do not.
- Both `\N` (hard break) and `\n` (soft break) become newlines.

**Time parsing** (`SubtitleEvent::stringtoTime`, `src/definitions.cpp`):
- Times are parsed as doubles.
- The fraction's scale comes from `secs.at(1).length() == 2`: two digits mean
  centiseconds, anything else means milliseconds. So `1.5` parses as 1.005 s.
- The result is then **snapped to project frames** (`pos.frames(fps)`). A frame-rate
  change re-quantizes through a float `transformMult`.

**Time formatting** (`SubtitleEvent::timeToString`). It rounds to centiseconds
without carrying into seconds:

```cpp
// kdenlive src/definitions.cpp — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL
int centisecond = (millisecond % 10 > 5) ? (millisecond / 10 + 1) : (millisecond / 10);
```

- For 996–999 ms this produces a three-digit "100", and a trailing 5 rounds down.
- Kdenlive's own parser then reads ".100" as 100 ms. So 1.996 s round-trips to
  1.100 s.
- This happens at NTSC rates, where frame times are not multiples of 10 ms.

**Other export and import details:**
- SRT export writes `sub.text()`, the ASS-tagged text, so override tags leak into SRT.
  Whitespace trimming is tested ("Comments in SRT export have no leading or trailing
  whitespace").
- ASS colours (`SubtitleStyle(QString)`) are sliced with fixed `mid(2,2)…mid(8,2)`.
  That assumes `&HAABBGGRR`. The legal six-digit `&HBBGGRR` and decimal forms
  (MLT writes `16777215`) misparse.
- Encoding detection (`guessFileEncoding`) does a strict UTF-8 check on the whole file.
  Otherwise it runs `KEncodingProber` on the **first 1024 bytes** with confidence
  thresholds of 0.5 and 0.6, and falls back to UTF-8. The non-UTF-8 test is disabled
  under Qt 6 (`tests/subtitlestest.cpp`, "TODO: qt6 fix").
- The generated `[Script Info]` writes `ScaledBorderAndShadow: yes` and
  `YCbCr Matrix: None`. That avoids colour-matrix mangling by VSFilter-compatible
  renderers and scales borders with resolution, and we copy both.
- Default font size is frame height / 18; the default margin is height / 27.

**Titler** (`src/titler/`):
- Titles are `QGraphicsScene` XML (`kdenlivetitle`), rendered by MLT.
- `titledocument.cpp` documents two migrations:
  - Old titles stored **point** sizes, which depend on DPI. The loader converts them to
    pixels and shows a warning ("Some of your text clips were saved with size in points,
    which means different sizes on different displays…").
  - Since 20.08 numbers are serialized with the C locale (an `LC_NUMERIC` attribute).
    Before that, locales with a decimal comma corrupted titles.
- Rich text is Qt HTML (`format="qt-html-v1"`), written after a legacy plain-text node.
  Versioned supplements cover spacing, gradients and outlines, because Qt HTML loses
  data. One comment notes that Qt HTML rounds fractional alpha, and the code
  post-processes to `#AARRGGBB`.
- `src/titler/richtextoutline.h` (MIT) has the same design as MLT's copy, described
  below.

### MLT framework (C/C++; LGPL-2.1-or-later, except `src/modules/plus/subtitles/` GPL-3.0-or-later and `src/modules/qt/typewriter.*` MIT)

**SRT reader** (`src/modules/plus/subtitles/subtitles.{h,cpp}`):
- `SubtitleItem { int64_t start, end; std::string text }` holds times in milliseconds.
- The reader is a three-state machine (`STATE_SEEKING_NUM`, `STATE_READING_TIME`,
  `STATE_READING_TEXT`) using
  `sscanf("%d:%d:%d,%d --> %d:%d:%d,%d")`. Only a comma is accepted before the
  milliseconds, and a blank line ends a cue.
- `indexForTime` caches the previous index for O(1) sequential lookup and falls back to
  a linear scan.

**Subtitle filter** (`src/modules/plus/filter_subtitle.cpp`). It samples each frame
at **its last millisecond**:

```c
/* mlt src/modules/plus/filter_subtitle.cpp — LGPL-2.1-or-later */
int64_t frameMs = ((int64_t) mlt_frame_get_position(frame) + 1) * 1000
                      * profile->frame_rate_den / profile->frame_rate_num
                  - 1;
```

- A cue therefore shows on the frame during which it starts.
- Drawing is delegated to the `qtext` filter, or `text` when that is missing.
- The resource file is re-read whenever its modification time changes.

**Subtitle feed** (`src/modules/plus/filter_subtitle_feed.cpp`) attaches per-frame
subtitle properties (`start`, `end`, `text`, `lang`) for downstream consumers.

**Subtitle muxing** (`src/modules/avformat/consumer_avformat.c`, `open_subtitles` and
`encode_subtitles`):
- Up to 8 subtitle streams.
- The codec is negotiated with `avformat_query_codec` in the order SubRip → ASS →
  mov_text. WebVTT is never tried.
- For streams that are not SubRip, the SRT text goes through FFmpeg's SubRip *decoder*
  into an `AVSubtitle`, then through the target encoder. That needs a hard-coded ASS
  `subtitle_header`.
- A packet is written when the frame carrying a new start is encoded.
  `subtitle_prev_start` removes the duplicates the per-frame feed produces.
- Timestamps are frame-quantized, with time base 1/1000.

**Simple and rich text filter** (`src/modules/qt/filter_qtext.cpp`):
- Text becomes a `QPainterPath`. The outline is stroked **before** the fill ("This
  avoids the outline encroaching on the text fill").
- Rich text uses a `QTextDocument` under a process-wide `QMutex g_mutex`.
- The box comes from font metrics plus a fudge: `height += 2; // I found some fonts
  whose descenders get cut off.`
- Outline width is an integer scaled by the profile. A `pixel_ratio` exists for
  Windows only.

**Title renderer** (`src/modules/qt/kdenlivetitle_wrapper.cpp`):
- The whole scene is re-rendered at the requested output size whenever the size
  changes. Titles are vector, so the result is crisp.
- The image is cached only when the title is not animated. Animated titles re-render
  every frame under `self->mutex`. Interlaced profiles render a second field.
- `PlainTextItem` strokes `m_path.simplified()`, the *union* of the glyph contours, so
  overlapping contours produce no inner stroke lines.
- The shadow is the path plus the outline stroke, rasterized and then softened by a
  four-pass recursive integer blur (`blur()`). Its radius is in pixels and does not
  scale with resolution.

**Rich text outline** (`src/modules/qt/richtextoutline.h`, 2026):
- Per-range outlines (width, colour) are stored as runs with `units="utf16"`, the
  character count and a SHA-256 of the plain text. `restore` rejects runs that split a
  surrogate pair or whose hash does not match.
- Strokes are painted in a pass **before** all fills.
- Consecutive runs with the same pen are merged into one `WindingFill` path.
- A range that splits a ligature glyph is clipped to its run rectangle
  (`QGlyphRun::SplitLigature`).
- Ink bounds are kept separate from layout bounds, with a one-pixel fringe.
- Outline width is capped at 200.

**Reveal animation** (`src/modules/qt/richtextanimation.h`, `RichTextReveal`):
- Reveal steps fall on grapheme boundaries (`QTextBoundaryFinder::Grapheme`).
- Step frames are forced monotonic (`frame = std::max(previous + 1, frame)`).
- **The full text, hidden suffix included, is laid out once.** The source comment
  gives the reason: "a later fallback font must not change the underline metrics of
  already visible text".
- `setVisibleCharacters` never shows "half a surrogate, combining sequence, or emoji
  cluster".

**Older typewriter engine** (`src/modules/qt/typewriter.{h,cpp}`):
- It steps by **code point**: `detectUtf8` returns UTF-8 sequence lengths and knows
  nothing of grapheme clusters.
- Jitter comes from `std::mt19937` plus `std::normal_distribution`. The normal
  distribution's algorithm is implementation-defined, so output differs between
  libstdc++, libc++ and MSVC.

**Pango producer** (`src/modules/gdk/producer_pango.c`):
- One process-wide `pango_mutex` serializes all Pango text.
- The outline is a dilation of the coverage bitmap by at most 3 px ("fake circle"), so
  it depends on resolution and is capped.
- The image is rendered once, then scaled with `gdk_pixbuf_scale_simple`, so it is soft
  when upscaled.
- A `fontmap-reload` event calls `FcInitReinitialize()` to pick up newly installed
  fonts.

### Shotcut (C++/Qt on MLT, GPL-3.0-or-later)

**Subtitle model** (`src/models/subtitlesmodel.cpp`):
- Each subtitle track is a `subtitle_feed` filter. Its `text` property holds a
  **complete SRT document**, which `load()` parses with MLT's
  `Subtitles::readFromSrtString`.
- Edits change an in-memory list. A debounce timer writes the list back
  (`requestFeedCommit` → `commitToFeed`), because re-serializing the whole track as SRT
  on every keystroke would be too slow.
- Times are integer milliseconds. `setItemStart`, `setItemEnd` and `validateMove`
  refuse overlaps with neighbours.

**Overlap bug** (`src/commands/subtitlecommands.cpp`). The check for "existing items
the new items cover" misses an item that **contains** the new range, so an overlap can
be created:

```cpp
// shotcut src/commands/subtitlecommands.cpp — GPL-3.0-or-later
if ((item.start >= startPosition && item.start < endPosition)
    || (item.end > startPosition && item.end < endPosition)) {
    m_saveSubtitles.push_back(item);
}
```

**Import and burn-in** (`src/docks/subtitlesdock.cpp`):
- `importSubtitles` shells out to the bundled `ffmpeg` executable to convert VTT, ASS
  or SSA into a temporary SRT. All styling is lost.
- It runs on the UI thread with `waitForFinished(8000)`.
- Burn-in adds a `subtitle` filter to the timeline output. The default family depends
  on the OS (Verdana on Windows, Helvetica on macOS). Size is profile height / 20 and
  the outline is 3.

**Text size modes** (`src/qml/modules/Shotcut/Controls/TextFilterUi.qml`): size is
either fixed (`shotcut:usePointSize`) or fitted to the rectangle.

### Olive (C++/Qt, GPL-3.0-or-later)

**Text node** (`app/node/generator/text/textv3.cpp`, `TextGeneratorV3::GenerateFrame`):
- Rich text (HTML) is laid out with a `QTextDocument` at the **full sequence
  resolution**. The painter is then scaled by `1.0 / divider`, the preview-resolution
  divider.
- So wraps are the same at every preview resolution, and glyphs are rasterized at the
  output resolution. This is the right pattern, and FreeCut keeps it.
- It sets **96 DPI explicitly** (`dpm = 3780`) so that point sizes do not depend on the
  machine.
- `textv2.cpp` used **72 DPI** (`dpm = 2835`) and rendered alpha only, adding colour
  afterwards. v1 had separate font, size and colour inputs. Three node versions now live
  side by side for old projects.

**Subtitles:**
- `app/render/subtitleparams.{h,cpp}`: `Subtitle { TimeRange range_; QString text_ }`
  with rational in and out points, serialized as rational strings.
- `app/node/block/subtitle/subtitle.cpp`: subtitles are blocks on subtitle tracks.
- Import (`app/codec/ffmpeg/ffmpegdecoder.cpp`) demuxes **SubRip streams only** ("Limit
  to SRT for now...").
- The viewer draws subtitles as an **overlay outside the render graph**
  (`app/widget/viewer/viewerdisplay.cpp`, `DrawSubtitleTracks`, "Extraordinarily basic
  subtitle renderer. Hoping to swap this out with libass at some point."). Preview
  therefore does not equal export, and there is no burn-in.
- Embedding is SRT into Matroska only (`app/codec/exportformat.cpp`,
  `GetSubtitleCodecs`).
- The encoder (`app/codec/ffmpeg/ffmpegencoder.cpp`) writes the UTF-8 text as the packet
  payload. It sets `codec_ctx->subtitle_header = new uint8_t[...]` without a NUL
  terminator, but FFmpeg frees that field with `av_freep`: an allocator mismatch.
- The ASS header exists only "because FFmpeg requires it"
  (`SubtitleParams::GenerateASSHeader`).

**On-canvas editing** goes through a proxied `QTextEdit`. The code calls its
event-forwarding workaround an "Absolutely filthy hack"
(`app/widget/viewer/viewerdisplay.cpp`). Editing text in transformed coordinates is
genuinely hard, and FreeCut should plan for it.

### libopenshot (C++, LGPL-3.0-or-later)

**Caption effect** (`src/effects/Caption.cpp`):
- All captions are one effect holding a VTT-like blob, parsed with one large regex.
- Times become frames through floats with truncation (`int64_t(startSeconds *
  fps.ToFloat()) + 1`), and the end is inclusive.
- Fractions are padded with `leftJustified(3, '0')`, so ".5" correctly means 500 ms.
  This is the bug Kdenlive has.
- Lines starting with `NOTE` are skipped, and so is any line of length ≤ 1 (the
  condition is `line.length() > 1`). **A one-character caption ("I", "!", a single CJK
  character) never renders.**
- Word wrap re-measures ever-longer prefixes (quadratic) and splits on spaces. Text
  without spaces and longer than 20 characters is split per character as a CJK
  heuristic.
- Font size is scaled by `frame width / 600`.
- Fill and stroke are drawn together with `painter.drawPath`, line by line. The stroke
  is centred on the outline and painted *over* the fill, so one line's stroke can cover
  the previous line's fill.

**Text and SVG titles:**
- `src/QtTextReader.cpp` draws text once into a fixed-size image, and the clip scales
  that image, which is soft when scaled up.
- `src/QtImageReader.cpp` `calculate_max_size()` rasterizes SVG titles at the
  **largest scale the clip's scale keyframes reach** (`scale_x.GetMaxPoint()`), clamped
  by `MaxDecodeSize`.

### OpenTimelineIO (C++/Python, Apache-2.0)

There is no caption or subtitle schema. `src/opentimelineio/track.h` uses a string
`Track::Kind` (`video`, `audio`), and `src/opentime/rationalTime.h` uses `double` for
both value and rate. FreeCut caption tracks could travel through OTIO only as a custom
kind plus metadata. OTIO does not drive this design.

### Comparison

| | Kdenlive | MLT qtext / kdenlivetitle | Shotcut | Olive | libopenshot | **FreeCut** |
|---|---|---|---|---|---|---|
| Caption storage | ASS events keyed by (layer, start) | SRT ms vector | SRT string in a filter | rational blocks on tracks | regex over a blob | sorted `Vec<CaptionSegment>` with stable ids, ticks |
| Times | double, snapped to frames | int ms | int ms | rational | double → frame, truncated | `i64` ticks, exact for ms/cs |
| Caption pixels | libass (FFmpeg filter) | QPainter per frame | MLT subtitle → qtext | viewer overlay only | QPainter | same GPU text layer as titles |
| Preview = export | yes | yes | yes | **no** | yes | yes |
| Title raster | vector at output size | vector at output size | qtext | QTextDocument at output size | image scaled up | SDF glyph atlas at output-size buckets |
| Outline | stroke of united path | stroke before fill | (qtext) | — | pen over fill, per line | outer band of the union, beneath fills |
| Shaping, fallback | Qt | Qt / Pango | Qt | Qt | Qt | parley (harfrust) + fontique |
| Locking | MLT threads | global mutexes | MLT | render threads | per frame | per-thread engines, immutable snapshots |
| Import | SRT VTT SBV ASS | SRT | SRT (ffmpeg CLI converts others) | SubRip streams | VTT-ish | SRT VTT ASS/SSA LRC SBV TXT |
| Embed | via MLT | SubRip → ASS / mov_text | via MLT | SRT in MKV | — | mov_text, ASS/SRT, WebVTT |

### Lessons → FreeCut responses

| # | Lesson (source) | FreeCut response |
|---|---|---|
| L1 | Keying subtitles by start time forbids equal starts and makes moves re-keys (Kdenlive). | Stable `SegId`; time is plain data. |
| L2 | ASS as the internal model ties the UI to a file format. Every edit rewrites a file (Kdenlive) or needs a debounce (Shotcut). | Structured model; formats exist only at import/export. |
| L3 | Snapping to frames and float conversion on import (Kdenlive, libopenshot). | Exact tick conversion (R2). Snapping is an explicit tool (TX-029). |
| L4 | Fraction scale guessed wrongly: "1.5" read as 1.005 s (Kdenlive). | The digit count defines the scale. |
| L5 | Centisecond rounding without carry (Kdenlive `timeToString`). | Round the *total* first, then split into fields. |
| L6 | Hand-written overlap tests miss containment (Shotcut). | One `overlaps()` predicate, property-tested. |
| L7 | Text that looks like a time is taken for a timing line (Kdenlive fixture). | A timing line is recognized only at a cue start and only with `-->`. |
| L8 | Fixed ASS field order and 8-digit colours assumed (Kdenlive). | Honour `Format:` lines. Accept 6/8-digit hex and decimal colours. |
| L9 | Format from a case-sensitive extension; encoding guessed from 1 KiB (Kdenlive). | Sniff the content. Feed the whole file to the detector. |
| L10 | One-character captions dropped (libopenshot). | Only empty text is special. |
| L11 | Animation units are code points (MLT typewriter). | Grapheme clusters, plus a ligature policy. |
| L12 | RNG distribution differs between C libraries (MLT). | Our own SplitMix64 and integer shuffle, with golden vectors. |
| L13 | Re-laying out a revealed prefix changes metrics (MLT comment). | Lay out once; reveal is a mask (R7). |
| L14 | Point/DPI sizes changed between versions (Olive 72 → 96 DPI; Kdenlive pt → px). | Ref px only. Never pt, never DPI. |
| L15 | Locale-dependent number serialization (Kdenlive `LC_NUMERIC`). | Integers in serde. No locale anywhere. |
| L16 | HTML storage loses data, so side tables are needed (Kdenlive, MLT). | Structured spans with explicit units (UTF-8 bytes), validated on load. |
| L17 | Rasterize once and rescale gives soft text (pango, `QtTextReader`). | Rasterize at output scale (Olive's divider pattern). Re-rasterize when the scale bucket changes. |
| L18 | Stroke over the fill, line by line (libopenshot). | Outer band beneath all fills, union-correct (R5). |
| L19 | Bitmap dilation capped at 3 px (pango). | Distance fields; width relative to the em. |
| L20 | Global mutexes serialize all text (pango, qtext). | Engines per thread; shared immutable snapshots. |
| L21 | Metric boxes clip descenders (qtext `height += 2`). | Ink bounds plus effect padding. |
| L22 | Viewer-only subtitle overlay, so preview ≠ export (Olive). | Captions go through the compositor. |
| L23 | Conversion through the ffmpeg CLI on the UI thread (Shotcut). | In-process parsers on a background job. |
| L24 | `subtitle_header` allocator and NUL mistakes (Olive); header required (MLT). | One FFI module with SAFETY notes and a tested codec table. |
| L25 | End-of-frame sampling (MLT) is implicit. | An explicit rule (frame start) plus a one-frame minimum, which guarantees visibility. |

---

## Recommended FreeCut design

### Overview

```
freecut-app  (UI thread, egui)
  Text inspector · font picker · Captions panel · on-canvas edit overlay
     │ EditCmd / MeasureReq (latest wins)         ▲ EditView / TextMetrics / FontList / thumbnails
     ▼                                            │
  freecut-render::text::service   TextService thread: TextEngine #0, EditActor, picker thumbnails
  freecut-render::text::fonts     discovery thread → ArcSwap<Arc<FontSnapshot>>
  job pool (app)                  read file → decode → freecut-project::subtitles::parse → ImportSubtitles op
                                  timeline → SubtitleDoc → write → temp + rename

freecut-timeline (pure)  RichText · TextStyle · FontRef · CaptionTrackData · ops · resolve_caption_style
                         reflow · retime · frame_plan(t) → TextItemPlan (style at t, anim state, active word)

freecut-render  (preview scheduler thread / export thread, each with its own TextEngine)
  TextItemPlan → layout (parley + fontique, ref px, cached) → LaidOutText
              → animate (unit windows, per-glyph transforms at t)
              → raster (swash coverage → signed distance field, CPU, cached)
              → gpu (atlas, box/silhouette/stroke/fill passes, blur, compose)
              → premultiplied layer texture → FX-001 L8–L10

freecut-media   SubtitleStreamWriter (mov_text / ass / subrip / webvtt) used by the export job
```

### Crate and module layout

```
freecut-timeline (pure, no I/O)
  text/units.rs        Fixed (1/100 ref px), PerMille, DeciDeg, Rgba8, LangTag
  text/rich_text.rs    RichText, Span, RangeStylePatch, SpanOrigin, ByteRange, edits, invariants
  text/style.rs        TextStyle, TextStylePatch, Fill, Stroke, Shadow, Glow, TextBox, FontRef,
                       TextLayout, BoxMode, HAlign, VAnchor, CaseTransform, Decoration
  text/clip.rs         TextClipData, TextOrigin
  text/ops.rs          SetTextContent, SetRichText, SetTextStyle, SetTextLayout,
                       ApplyTextPreset, ReplaceFontEverywhere, ReplaceText
  text/search.rs       find / replace over RichText (NFC + case fold, TX-034)
  caption/track.rs     CaptionTrackData, CaptionLayout, SecondaryLine, CaptionRules, Speaker
  caption/segment.rs   CaptionSegment, SegId, WordTiming, CaptionOrigin, CaptionDraft
  caption/ops.rs       SpliceSegments (the single segment mutator) and every TX caption op built on it
  caption/resolve.rs   resolve_caption_style, segment_at, active_word
  caption/reflow.rs    CaptionRules::check, reflow (icu_segmenter, grapheme counting)
  caption/retime.rs    LinearMap, shift, fps change, two-point sync, snap (rational, half-even)
  caption/script.rs    script splitting and distribution (TX-030)
  plan/text.rs         TextItemPlan inside FramePlan

freecut-render
  text/mod.rs          TextEngine facade (one per thread)
  text/fonts/          registry.rs, snapshot.rs, resolve.rs, aliases.rs, fallback.rs, bundled.rs
  text/layout/         build.rs (RichText → parley), result.rs (LaidOutText), curve.rs,
                       case.rs, spacing.rs (cursive guard), cache.rs
  text/animate.rs      units, unit_windows, SplitMix64, preset curve evaluation → GlyphAnim
  text/raster/         coverage.rs (swash), sdf.rs, color.rs, glyph_cache.rs
  text/gpu/            atlas.rs (etagere pages), pipelines.rs, passes.rs, blur.rs, text_*.wgsl
  text/caption.rs      caption block assembly (primary + secondary, anchor, max width)
  text/service.rs      TextService thread: measure, thumbnails
  text/edit.rs         EditActor (on-canvas editing state machine)
  text/error.rs

freecut-project
  subtitles/mod.rs     SubtitleDoc, Cue, SubtitleFormat, ParseReport, WriteOptions
  subtitles/sniff.rs   content sniffing
  subtitles/{srt,vtt,ass,lrc,sbv,txt}.rs   parse + write
  subtitles/markup.rs  tag ↔ RangeStylePatch mapping
  subtitles/time.rs    tick ↔ ms/cs, formatting with carry
  subtitles/convert.rs SubtitleDoc ↔ caption tracks, overlap partitioning
  text_presets.rs, text_templates.rs   versioned serde schemas (TX-019, TX-024)

freecut-media
  mux/subtitle.rs      SubtitleCodec table, SubtitleStreamWriter (FFI, SAFETY-commented)

freecut-app
  text/inspector.rs, text/font_picker.rs, text/canvas_edit.rs (egui + IME glue),
  captions/panel.rs, captions/import.rs (dialog + read/decode job), captions/export.rs
```

Dependency rules:
- `freecut-timeline` gains only pure Unicode crates (`unicode-segmentation`,
  `icu_segmenter`, `icu_casemap`, `unicode-normalization`). None of them does I/O.
- `freecut-project` depends on `freecut-timeline`.
- `freecut-render` depends on `freecut-timeline` and `freecut-media`, never on
  `freecut-project`.
- `freecut-media` never sees subtitle formats. The app turns timeline captions into ASS
  event strings (with `freecut-project::subtitles`) and hands those to the media writer.

### Model (`freecut-timeline::text`, `::caption`)

**Units and rich text.**

```rust
/// 1/100 reference px; 1080 ref px = canvas short edge (TX-001 conventions).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Fixed(pub i32);
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PerMille(pub i32);
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba8 { pub r: u8, pub g: u8, pub b: u8, pub a: u8 } // sRGB, straight alpha

/// Half-open UTF-8 byte range into `RichText::text`, always on grapheme-cluster boundaries.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ByteRange { pub start: u32, pub end: u32 }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RichTextRepr")] // invariants are checked on load too (L16)
pub struct RichText { text: String, spans: Vec<Span> }

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span { pub range: ByteRange, pub ov: RangeStylePatch, pub origin: SpanOrigin }

/// Only the properties TX-015 allows per range, so block-level overrides are unrepresentable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RangeStylePatch {
    pub font: Option<FontRef>, pub size: Option<Fixed>, pub fill: Option<Fill>,
    pub bold: Option<bool>, pub italic: Option<bool>,
    pub underline: Option<bool>, pub strike: Option<bool>, pub stroke_color: Option<Rgba8>,
}

impl RichText {
    /// Fails on '\0' or '\r', and on spans that are unsorted, overlapping, empty, out of
    /// bounds, not on grapheme boundaries, or not merged.
    pub fn from_parts(text: String, spans: Vec<Span>) -> Result<Self, RichTextError>;
    /// Replaces `range`. Inserted text inherits the style of the grapheme before the range
    /// (or the one after it at offset 0, TX-002). Spans shift, clip, drop when empty, re-merge.
    pub fn replace(&self, range: ByteRange, insert: &str) -> Result<RichText, RichTextError>;
    /// Applies `patch` to `range`, snapped outward to grapheme boundaries (TX-006).
    pub fn patch(&self, range: ByteRange, patch: &RangeStylePatch, origin: SpanOrigin)
        -> Result<RichText, RichTextError>;
    /// Removes overrides in `range`, or only those with `origin` (TX-040 "clear automatic").
    pub fn clear(&self, range: ByteRange, origin: Option<SpanOrigin>) -> RichText;
}
```

The spec writes `Span.ov: TextStylePatch`. FreeCut narrows that to `RangeStylePatch`
so that alignment, shadow and the like cannot be stored per range at all. The edit
functions return new values. Texts are small, and undo ops keep `before` and `after`
(TX-002, TX-015).

`TextStyle`, `TextStylePatch`, `FontRef`, `Fill`, `Stroke`, `Shadow`, `Glow`,
`TextBox` and `TextLayout` follow the TX field lists exactly. Two additions:
- `TextStyle.language: Option<LangTag>` (TX-006).
- A `named_instance: Option<String>` on `FontRef`. It holds the PostScript name of a
  variable-font named instance (TX-003).

**Caption track.** Segments are private and change through a single primitive.

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SegId(pub u64); // from the document's id counter; never reused (L1)

pub struct CaptionTrackData {
    pub name: String, pub language: Option<LangTag>,
    pub style: TextStyle, pub layout: CaptionLayout, pub animations: ClipAnimations,
    pub secondary: Option<SecondaryLine>, pub rules: CaptionRules, pub speakers: Vec<Speaker>,
    segments: Vec<CaptionSegment>, // sorted by start; pairwise non-overlapping
}

/// The only overlap test in the code base (L6). Half-open ranges.
pub fn overlaps(a: TimeRange, b: TimeRange) -> bool { a.start < b.end() && b.start < a.end() }

impl CaptionTrackData {
    /// Visible segment at frame time `t` (start <= t < end). O(log n) (R8).
    pub fn segment_at(&self, t: Tick) -> Option<&CaptionSegment> {
        let i = self.segments.partition_point(|s| s.range.start <= t);
        let seg = self.segments.get(i.checked_sub(1)?)?;
        (t < seg.range.end()).then_some(seg)
    }
    /// The single mutator. Replaces `at..at+removed.len()` with `inserted` after checking
    /// order, non-overlap with neighbours and the minimum length. Returns the removed segments.
    pub(crate) fn splice(&mut self, at: usize, remove: usize, inserted: Vec<CaptionSegment>)
        -> Result<Vec<CaptionSegment>, EditError>;
}
```

Every caption op is built on `splice`:
- split, merge, insert and delete (TX-027);
- retime (TX-029);
- re-flow (TX-037);
- import (TX-031);
- `ReplaceCaptions` (TX-038);
- the conversions (TX-035).

Each op is a list of `SpliceSegments { track, at, removed, inserted }` plus property
sets. Undo applies the inverse splices in reverse order. One generic property test
("apply, then undo, gives the original track; redo gives the applied track") therefore
covers every caption op. Each op also gets its own edge-case tests (TX-027 lists them).

A segment is found by `SegId` with a binary search on the start time the op recorded,
then the id is checked; a linear scan is the fallback. An id map is not worth keeping
in step at 50k segments.

**Time rules:**
- Sequence frame n is evaluated at `t_n = n × frame_duration` (FX-001). A segment or
  text clip is visible at `t_n` when `start ≤ t_n < end`. FreeCut samples the frame
  **start**; MLT samples the last millisecond (L25).
- A half-open range at least one frame long always contains a frame start, so the
  minimum length of one frame guarantees that a segment is visible on at least one frame
  (TX-025).
- When a sequence frame-rate change (TL-001 "Conform to frame grid") leaves segments
  shorter than the new frame, `CaptionRules::check` flags them, and the conform step
  extends them to one frame.
- Embedded and sidecar subtitles carry the exact segment times, rounded to
  milliseconds. Burned-in captions are sampled per frame. A cue can therefore show one
  frame later burned in than in a player using the sidecar file. This is inherent and
  is documented in EX's help.
- The active word (TX-039) is the last word whose start ≤ t, clamped to the segment.
  The previous word stays active through pauses.

**Frame plan entry.** It is computed in the timeline, so it is pure and goes into
`render_key`.

```rust
pub struct TextItemPlan {
    pub item: TextItemRef,                  // Clip(ClipId) | Caption { track, seg }
    pub band: Band,                         // Normal | Caption (drawn after all tracks, TX-025)
    pub content: Arc<RichText>,             // or the live edit buffer (EditOverride)
    pub secondary: Option<(Arc<RichText>, TextStyle)>, // TX-036
    pub style: TextStyle,                   // resolved at t: keyframes, preset, caption cascade
    pub para: ParagraphPlan,                // align, box mode or caption max width, v_anchor, curve
    pub local_t: Tick, pub duration: Tick,  // clip-local time
    pub anim: TextAnimPlan,                 // FX slots + AnimParams (TX-022/023), Loop window rule
    pub reveal: Option<PerMille>,           // TX-021 Reveal
    pub active_word: Option<ByteRange>,     // TX-039
    pub transform: ResolvedTransform,       // TR at t, with FX block offsets folded in (FX-001 L8)
}
```

### Fonts (`freecut-render::text::fonts`)

**Sources and snapshot.**

```rust
pub struct FontSnapshot {
    pub generation: u64,       // in-process counter; bumps on scan, import, remove
    pub env_digest: u128,      // stable XXH3 of all face fingerprints + fallback config (R9)
    families: Vec<FamilyEntry>,// sorted by folded name, for the picker (TX-003)
    faces: Vec<FaceEntry>,
    pub scan: ScanState,       // BundledOnly | Complete | Failed(String)
}
pub struct FaceEntry {
    pub source: FontSource,    // Bundled { id } | System { path, index } | Imported { path, index }
    pub family: String, pub typographic_family: Option<String>, pub style_name: String,
    pub postscript: Option<String>, pub weight: u16, pub italic: bool, pub stretch: u16,
    pub named_instance: Option<u16>,
    pub version: Option<String>, pub fingerprint: u128,  // ps name + version + head checksum + size
    pub embedding: EmbeddingPermission,                  // OS/2 fsType (R13)
    pub kind: FaceKind,        // Outline | ColorBitmap | Colr | SymbolOnly
}
pub enum Resolution { Exact(FaceIdx), Substituted { face: FaceIdx, why: SubstReason }, Missing }
```

**Discovery.** It runs on the discovery thread and never on the UI thread (R3).
1. Register the bundled fonts first. They are read from app resources into
   `Arc<[u8]>`. A snapshot with `ScanState::BundledOnly` is published at once, so the
   first title renders before the system scan ends (TX-003 spinner).
2. Build the system collection with fontique
   (`CollectionOptions { shared: true, system_fonts: true }`). fontique uses CoreText on
   macOS, DirectWrite on Windows and fontconfig on Linux (`fontique-0.12.0/src/backend/`).
   Each font becomes a `FaceEntry`.
3. Read `name`, `OS/2` and `head` with `read-fonts`. A font that fails to parse is
   logged and skipped.
4. Imported fonts (TX-018) are validated, then copied into
   `<app-data>/fonts/<postscript>-<version-hash>.<ext>` and registered from memory.
   FreeCut never memory-maps a file it may be asked to delete.
5. **Rescan** (TX-007) rebuilds the collection and publishes a new snapshot.
   `generation` bumps and `env_digest` changes, which invalidates layout caches and
   `render_key`s.

Snapshots are published with `ArcSwap`. Every thread keeps its own parley
`FontContext` and `LayoutContext`, because neither is shared across threads, and
rebuilds them when `generation` changes. No global lock exists (L20).

**Resolving a `FontRef`.** `FontSnapshot::resolve(&FontRef) -> Resolution` is pure
over the snapshot.
1. **PostScript name** equal and the family name matching → `Exact`.
2. **Family name.** Names are compared after NFC and Unicode case folding. The
   typographic family (`name` ID 16) is tried first, then the legacy family (ID 1).
   When a family exists in several sources, the precedence is Imported > System >
   Bundled (TX-003). The face is chosen with the CSS Fonts 4 matching order: stretch,
   then italic → oblique → normal, then weight with the CSS ≥ 400 / < 400 rules. An
   exact style gives `Exact`; anything else gives `Substituted { NearestStyle }`.
3. **Alias table** (TX-007): Arial and Helvetica → Liberation Sans, Times New Roman →
   Liberation Serif, Courier New → Liberation Mono. The result is
   `Substituted { MetricAlias }`.
4. **Generic class.** It is guessed from the OS/2 panose, `post.isFixedPitch` and name
   tokens ("Serif", "Mono", "Script"). The bundled family of that class becomes
   `Substituted { Generic }`.
5. Otherwise `Missing`. It renders with bundled Noto Sans at the requested weight and
   italic. The stored `FontRef` is left untouched (TX-007).

When bold is requested and the chosen face has weight < 600, the face gets synthetic
embolden. When italic is requested and the face is upright, it gets a synthetic 12°
skew. Both are applied in the rasterizer (see below) and marked "(synthetic)" in the UI.

**Fallback chain (TX-006).** Parley shapes each cluster with the span's family stack,
then with fontique's per-script fallback list
(`parley-0.12.0/src/shape/mod.rs`, `fontique::FallbackKey`). At snapshot build, FreeCut
rewrites each list that has a bundled member:

```rust
for &(script, bundled) in BUNDLED_SCRIPT_FALLBACKS {      // e.g. ("Arab", &["Noto Naskh Arabic"])
    for locale in locales_for(script) {                  // Han: ja, zh-Hans, zh-Hant, ko
        let key = FallbackKey::new(script, locale.as_ref());
        let system: Vec<FamilyId> = collection.fallback_families(key).collect();
        let ids = bundled.iter().filter_map(|n| collection.family_id(n));
        collection.set_fallbacks(key, ids.chain(system));   // bundled first, then the OS list
    }
}
collection.set_generic_families(GenericFamily::Emoji, [noto_color_emoji].into_iter());
```

**Emoji pitfall.** For emoji clusters parley appends `GenericFamily::Emoji` *after* the
span's own families (`parley-0.12.0/src/shape/mod.rs`). A text font that has a
monochrome ❤ would then win even with VS16. FreeCut's builder therefore finds
emoji-presentation grapheme clusters (VS16, `Emoji_Presentation`, ZWJ sequences, flags,
keycaps) and pushes a family stack `[Emoji, primary…]` on exactly those ranges.

**Missing glyphs.** When a cluster has no glyph anywhere (glyph id 0 after the whole
fallback chain), FreeCut does not draw the font's `.notdef`, which some fonts leave
blank. It draws its own hollow box (0.5 em × 0.7 em, stroke 0.06 em) and records the
code points.

**Fallback report.** The layout records every cluster whose run font differs from the
resolved primary face, and every missing code point. The inspector's "fallback used"
hint reads this report (TX-006).

### Layout (`freecut-render::text::layout`)

**Coordinate space:**
- Layout runs in **ref px**, y down, with parley's
  `ranged_builder(&mut fcx, text, 1.0, /*quantize*/ false)`.
- `quantize: true` rounds vertical metrics to pixels and would break R1, so it is never
  used.
- No hinting is used anywhere. Unhinted advances scale linearly, so wrap points are the
  same at every raster scale (R1, the Olive pattern).
- Raster scale enters only in `raster` and `gpu`.

**Mapping the model to parley:**

| Model | parley `StyleProperty` / call | Notes |
|---|---|---|
| font (resolved face) | `FontFamily` stack, `FontWeight`, `FontStyle`, `FontWidth`, `FontVariations` (named instance) | emoji ranges get `[Emoji, primary…]` |
| size | `FontSize(size_ref)` | per span |
| language | `Locale` | `locl` forms, fallback key, CJK line-break strictness |
| letter spacing | `LetterSpacing(per_mille × size / 1000)` | cursive guard, below |
| line spacing | `LineHeight::MetricsRelative(per_mille / 1000)` | the line box covers all runs, so the tallest fallback font sets the height (TX-006) |
| underline, strike | `Underline`, `Strikethrough` | font metrics; decoration rects come from the layout |
| box Auto | `TextWrapMode::NoWrap` | only manual breaks |
| box Fixed(w) | `TextWrapMode::Wrap`, `OverflowWrap::BreakWord`, `break_all_lines(Some(max(w, 1 em)))` | mid-word only as a last resort (TX-005) |
| whitespace | `WhiteSpaceCollapse::Preserve` | text is stored as typed; tabs become 4 spaces at input (TX-002) |
| alignment | `Alignment::{Left, Center, Right, Justify}`, `AlignmentOptions { last_line_alignment: None, align_when_overflowing: true }` | visual alignment; Justify's last line falls back to Start |
| per-span paint | `Brush = u32` index into a per-layout `SpanStyleTable` | fill, stroke colour, decoration colour |

**Ligature policy.** When letter spacing ≠ 0, or a per-grapheme animation is active,
the builder sets `liga=0`, `clig=0` and `dlig=0`. This is CSS's rule for letter
spacing. It also makes Latin grapheme units map one to one onto glyph clusters
(L11). `rlig`, `calt` and the shaping features of complex scripts are never touched.

**Cursive guard (TX-009).** Letter spacing must never open a gap inside a join.
- For every cluster boundary `a|b`, the Unicode `Joining_Type` of the last character of
  `a` and the first of `b` decides whether the pair joins: `a` ∈ {D, L} and `b` ∈ {D, R},
  with transparent marks skipped.
- Clusters followed by a joining boundary get `LetterSpacing(0)`.
- Spaces between Arabic words still grow.
- Per-grapheme animation presets degrade to **per word** for cursive-joining scripts.
  Moving joined letters one by one would break the joins. This is a FreeCut default.

**Result.** It is our own type, so parley's types never leak beyond `layout/`.

```rust
pub struct LaidOutText {
    pub env_digest: u128,
    pub lines: Vec<LineBox>,          // baseline, block min/max, content min/max, offset, advance
    pub glyphs: Vec<PlacedGlyph>,     // rest pose, in ref px
    pub clusters: Vec<ClusterInfo>,   // byte range, grapheme/word/line index, visual x-range,
                                      // rtl, whitespace, glyph range
    pub decorations: Vec<DecorationRect>,
    pub ink_bounds: RectF, pub layout_bounds: RectF,   // kept apart (MLT inkBounds, L21)
    pub fallback: FallbackReport,
}
pub struct PlacedGlyph {
    pub face: FaceIdx, pub glyph: u32, pub size_ref: f32,
    pub origin: [f32; 2],             // pen position, ref px
    pub synth: Synthesis,             // embolden: bool, skew_deg: f32
    pub kind: GlyphKind,              // Outline | ColorBitmap | Colr | MissingBox
    pub style: u32, pub cluster: u32,
}
```

**Paragraph details:**
- **Vertical anchor** (TX-005): the box offset is 0, −h/2 or −h for top, middle and
  bottom, relative to the TR anchor.
- **Empty text** gives no glyphs and a `layout_bounds` one line high, which the app uses
  for the dashed placeholder.
- **Case transform** (TX-008):
  - It builds a display string with `icu_casemap`, which is locale-aware. The standard
    library's `to_uppercase` cannot do Turkish dotted i, and it ignores the language tag.
  - Mapping ß → SS changes length, so a display→source byte map translates spans and
    clusters.
  - Edit mode turns the transform off (TX-002), so caret logic never needs the reverse
    map.
- **Curve** (TX-016), applied after layout:
  - θ = |curve| × 3.6° in radians.
  - The longest line's advance L sets the radius R₀ = L / θ. Line i sits on radius
    R₀ ± i × line height.
  - A glyph whose centre is at x along the line gets angle φ = (x − L/2) / R. It is
    placed at `centre + R·(sin φ, ∓cos φ)` and rotated by φ.
  - Letter spacing is part of L, so at ±100 the circle still closes.
  - Hit-testing inverts the mapping with `atan2`.
  - `libm` is used for the trigonometry in tests, so golden layouts match across OSes.
- **Caching.** `LayoutKey` is an XXH3-128 of the canonical bytes of the inputs:
  RichText, resolved block style, `ParagraphPlan`, case, ligature policy and
  `env_digest`. The cache is a shared `quick_cache` of `Arc<LaidOutText>` with a 64 MB
  budget.
- Keyframed size or spacing changes the key every frame and reflows. That is expected
  (TX-021); a caption lays out in under a millisecond.

### Animation (`freecut-render::text::animate`)

**Units:**
- **Grapheme:** the layout's clusters.
- **Word:** ICU word boundaries.
- **Line:** the layout's lines.

Whitespace units take no time (TX-023). They appear together with the unit before
them. Units are counted in **logical (reading) order**, so "forward" means reading
order in RTL text too.

**Windows.** They are pure integer math, so the last window ends exactly at the span
end (TX-023).

```rust
/// Splits an In/Out duration among `n` timed units. overlap 0 = one after another,
/// 1000 = all together. Window i is [s·i·k/d, s·(i·k+1000)/d) with k = 1000−overlap,
/// d = 1000 + (n−1)·k, so the last window ends exactly at `span`.
pub fn unit_windows(span: Tick, n: u32, overlap: PerMille) -> Vec<(Tick, Tick)> {
    if n == 0 { return Vec::new(); }
    let k = 1000 - i128::from(overlap.0.clamp(0, 1000));
    let d = 1000 + i128::from(n - 1) * k;
    let s = i128::from(span.0);
    (0..i128::from(n))
        // Both bounds lie in [0, span], so the narrowing cannot overflow.
        .map(|i| (Tick((s * i * k / d) as i64), Tick((s * (i * k + 1000) / d) as i64)))
        .collect()
}
```

**Order:**
- `Forward`, `Backward` and `CenterOut` (ties go left first) are permutations of the
  windows.
- `Random` uses FreeCut's own **SplitMix64** with the stored seed and a Fisher–Yates
  shuffle. Bounded integers come from Lemire's 128-bit multiply.
- Both algorithms are frozen, and golden vectors are checked in. Changing them would
  change the look of saved projects (L12, R6).
- `rand`'s `StdRng` is documented as not reproducible across releases, so it is never
  used.

**Per glyph at time t:**
- Unit progress p = (t − start) / (end − start) is computed as a rational and turned
  into an `f32` only here. Easing then applies.
- The preset curves (TX-022: offset, scale, rotation, opacity, blur, reveal) give a
  `GlyphAnim`.
- The glyph's transform is: rest origin, then the animation about the unit's ink
  centre, then the curve frame.

**Gradient fill (TX-014).** Each glyph instance carries its **rest-pose** position.
The shader evaluates the gradient there, so animated glyphs keep their rest colour.

**Reveal (TX-021, TX-023):**
- The layout is never redone (R7, L13). Units in the order up to
  ⌊n × reveal / 1000⌋ are visible.
- A glyph spanning several units (only possible with ligatures on) is clipped at the
  proportional x position. This follows MLT's split-ligature clip in
  `richtextoutline.h`.
- Per-line boxes (TX-013) cover only the revealed glyphs of each line.

**Active word (TX-039).** Its colour override is a transient span. Its scale is a
per-glyph transform about the word's centre, applied with no reflow.

### Rasterization to the GPU (`freecut-render::text::raster`, `::gpu`)

**Approaches considered:**

| Approach | Where seen | Small text | Animated scale / rotation | Outline, glow, blur | Per-frame cost | Verdict |
|---|---|---|---|---|---|---|
| Vector raster per frame on the CPU (QPainter) | Kdenlive and MLT titles, Olive, libopenshot | excellent | must re-raster | path stroke; union needs `simplified()` | high at 4K, under locks | rejected: CPU cost and locks |
| Coverage (A8) glyph atlas | glyphon, egui | excellent at 1:1 | soft when scaled; re-raster per scale | none built in | low | not enough for R4 and R5 |
| **Single-channel SDF atlas at output-size buckets** | proposed | good (≥ 8 px em) | rotation exact; residual scale ≤ 1.09× | cheap thresholds; union via max | low | **chosen** |
| Multi-channel SDF | game engines (`fdsm`) | sharp corners at low resolution | very good | union of channels is not a min/max | low | rejected: the union breaks R5 |
| GPU vector (vello on wgpu 30) | Xilem | excellent | perfect | strokes via kurbo; union needs path booleans | compute shaders | revisit later: a big dependency and a second renderer |
| libass via FFmpeg | Kdenlive | good | n/a | full ASS | file rewrite per edit | rejected: missing in this FFmpeg build, and a second style model |

**Raster scale and size buckets.** Layout stays in ref px. Only this step looks at
device pixels.

```rust
/// Device px per ref px for one text layer: canvas, preview/export target and block scale
/// (TR scale and FX block scale). Rotation is not included; L8 applies it.
fn raster_scale(canvas_short_px: u32, target_scale: f32, block_scale: [f32; 2]) -> f32 {
    canvas_short_px as f32 / 1080.0 * target_scale * block_scale[0].abs().max(block_scale[1].abs())
}
/// Em sizes snap up to 2^(k/8), so oversampling stays ≤ 9 % and the cache key is
/// bounded. Clamped to 4..=1024 px. Above 1024 px the SDF is magnified, which SDFs
/// handle well.
fn em_bucket(em_px: f32) -> u16 { /* ceil(8·log2(em_px)), clamped */ }
```

**Per-glyph signed distance field:**
- **Coverage.** `swash` scales the outline to the bucket size. It is unhinted, with
  horizontal subpixel offset 0; the sampling quad is positioned at sub-pixel precision
  instead.
- **Padding (spread).** It is 0.30 em + 2 px when an outline or glow is used anywhere on
  the layer, otherwise 0.06 em + 2 px. These are two cache buckets.
- **Distance.** `sdf_glyph_renderer::BitmapGlyph::from_unbuffered(..).render_sdf(spread)`
  computes it. This is the coverage-aware Felzenszwalb–Huttenlocher transform popularised
  by TinySDF. The crate is BSD-3-Clause, and with default features off it has no
  FreeType dependency.
- The output is positive outside and negative inside (`outer.sqrt() − inner.sqrt()`). It
  is converted to signed device px and stored as `f16` (`half`) in **`R16Float`**. With
  spreads up to about 300 px, 8 bits would leave about 2 px steps.
- **Colour glyphs.** swash renders `ColorOutline` (COLRv0) or `ColorBitmap`
  (CBDT/sbix, choosing the best strike) into RGBA, plus an SDF of their alpha. Outline,
  shadow and glow on emoji then use the silhouette (TX-006, TX-010).
- **Synthetic styles.** Bold shifts the fill threshold by −0.02 em
  (`d < 0.02·em_px`), so no re-raster is needed. Italic is a 12° shear in the instance
  transform.
- **Cache keys:**
  - CPU: `(face, glyph, em_bucket, spread_bucket, kind)` in a shared
    `quick_cache<Arc<GlyphBitmap>>`.
  - GPU: an `etagere` allocator per atlas page. SDF pages are 2048² `R16Float`; colour
    pages are 2048² `Rgba8Unorm`.
  - The colour format is deliberately **not** `*Srgb`. FX-001 L2 keeps working values
    display-encoded, and an sRGB view would linearize them when sampled.
  - Budgets are 128 MB for SDF and 64 MB for colour. Eviction is LRU by the last frame
    that used a glyph, and glyphs used by the current frame are pinned.
- **Threading.** Rasterization runs synchronously on the calling render thread, which
  keeps it deterministic. In v1 a prewarm worker can rasterize glyphs of captions in the
  next N seconds into the shared CPU cache.

**Per-layer pass graph.** A text layer is rendered in *layer space* with the raster
scale applied. Its size is the instance bounds plus effect padding, in device px.

| Pass | Target (blend) | Content |
|---|---|---|
| A box | `R8Unorm` (Max) | per-line or block rounded rects, as an analytic SDF; union by max, so overlaps never darken (TX-013) |
| B silhouette | `R8Unorm` (Max) | per instance `cov(d < bold)`: glyphs, colour-glyph alpha, decoration rects, missing boxes |
| C stroke | `Rgba16Float` (Max per channel) | per instance `stroke_colour × cov(d < w)`; only when an outline is on |
| D fill | `Rgba16Float` (premultiplied over) | per instance solid/gradient fill × `cov(d < bold)`, or the colour-glyph texel; decorations drawn last |
| E shadow | blur chain | source = max(silhouette, stroke α) [∪ box if "include box"]; offset `d·(cos θ, sin θ)` (y down, so 45° points down-right); σ = r/3 |
| F glow | blur chain | source = max(silhouette, stroke α); radius r, σ = r/3 |
| G compose | `Rgba16Float` layer (premultiplied) | box → shadow → glow (additive) → `stroke·(1 − silhouette)` → fill |

Notes on the passes:
- **Union-correct outline (R5, L18).** Each glyph's outer band is a max-union. The
  global silhouette then removes every inner part, so joins and overlapping glyphs show
  no seams, even with a transparent fill (hollow text). That reaches Kdenlive's
  `m_path.simplified()` result without path booleans.
- Two different outline colours meeting at a range boundary take the channel-wise
  maximum where their bands overlap. That is the only approximation.
- **Outline semantics.** The outline is an **outer band** of width w measured from the
  glyph edge. That matches "appears to sit outside the letters" (TX-010) and libass's
  border. A hollow fill (alpha 0) shows only the outer band.
- **Glow is additive.** It is stored as premultiplied colour with α = 0, which source-
  over turns into "add". Later layers cover it where they are opaque. At L10 *Normal*,
  a glow pixel adds light to the video beneath. Other blend modes un-premultiply at L10
  (FX-001), which would erase α = 0 pixels, so glow falls back to an alpha glow there.
  This is documented.
- **Blur.** Separable Gaussian. When r > 32 px, the source is downsampled by 2^k until
  r / 2^k ≤ 32, blurred, then upsampled bilinearly. r is capped at 256 px at 4K, scaled
  to the canvas (TX-011, TX-012).
- **Coverage from distance** (WGSL sketch):

  ```wgsl
  // d: signed distance in layer device px (texel distance × instance scale, from √|det|
  // for shears). Anti-aliasing is a 1 px ramp, widened by per-glyph softness (blur presets).
  fn coverage(d: f32, edge: f32, soft: f32) -> f32 {
      return clamp(0.5 - (d - edge) / (1.0 + 2.0 * soft), 0.0, 1.0);
  }
  ```

  Distances are computed in layer px after the instance transform. So anti-aliasing
  stays correct while glyphs zoom, rotate or shear in an animation. Per-glyph "blur in"
  is just `soft > 0`, capped at the spread (R4).
- **Colour space.** Style colours (sRGB `Rgba8`) go through CL's conversion into the
  working space once, when uniforms are built. Gradients interpolate in the working
  space, which matches CSS's default. Colour *keyframes* interpolate in linear light
  in the timeline (TR/CL, TX-021). Coverage is applied in the working space, the same as
  every other FX-001 layer.

**Layer size, limits and caching:**
- Bounds are the union of transformed instance bounds and box rects, padded by
  `max(outline w, glow r, |shadow offset| + 3σ) + 2 px`.
- Layers whose L8-transformed bounds miss the canvas (plus a margin) are culled.
- When the layer would exceed `device.limits().max_texture_dimension_2d`, the raster
  scale is reduced until it fits and a debug log entry is written (R10). It never fails
  or panics.
- **Layer cache.** `TextLayerKey` is an XXH3 of the layout key, resolved effect
  parameters, scale bucket, a digest of the per-glyph animation state at t and the
  target format. Static captions and titles reuse their layer texture across frames
  while the video beneath them changes. LRU budget 256 MB.
- **Output.** The premultiplied layer goes to FX-001 at L8 with the residual transform
  (rotation, translation, residual scale ≤ 1.09). Opacity is L9 and blending is L10.

**`render_key` (R9).** FreeCut adds:
- `FontSnapshot::env_digest`;
- `TEXT_RASTER_VERSION`, bumped whenever SDF, shader or ligature-policy math changes;
- the caption burn policy of the target;
- the hash of any live `EditOverride` or hover-preview override.

### Captions on the canvas (`freecut-render::text::caption`)

- **Band.** Caption tracks are drawn after every other track, in timeline order
  (TX-025, FX-001 walk).
- **Block:**
  - The primary paragraph has a fixed width of `max_width` × canvas width, converted to
    ref px, and wraps.
  - The secondary paragraph (TX-036) has its own style and is placed above or below
    with `gap`.
  - Each wraps on its own, and the two are stacked. The block is anchored with
    `CaptionLayout { anchor, offset }`, and bottom-anchored blocks grow upwards.
- **Transform.** Translate only, plus FX block offsets (TX-035: captions have only a
  position). Per-unit animations run on both lines at once.
- **Warnings** (more lines than `rules.max_lines`, wider than the box) are measured by
  `TextService` and shown in the Captions panel. They are never stored in the model
  (TX-028).
- **Burn policy (TX-033).**
  - Preview draws the visible caption tracks.
  - Export's `RenderTarget` carries `CaptionBurnPolicy { burn: BTreeSet<TrackId> }`,
    taken from the per-track choices. "Embed only" and "Don't include" are not drawn,
    even when visible. Hidden tracks are never drawn.
  - The policy is part of `render_key`.

### Subtitle formats (`freecut-project::subtitles`)

Input is text that is already decoded. Encoding is the app's job, so the parsers are
pure (TX-031).

```rust
pub enum SubtitleFormat { Srt, WebVtt, Ass, Ssa, Lrc, Sbv, TimedTxt, Transcript }
pub struct SubtitleDoc {
    pub cues: Vec<Cue>,                 // file order; may overlap
    pub styles: Vec<AssStyle>,          // ASS/SSA only
    pub script_res: Option<(u32, u32)>, // PlayResX/PlayResY
}
pub struct Cue {
    pub start: Tick, pub end: Tick,
    pub text: CueText,                  // plain text + RangeStylePatch spans
    pub secondary: Option<CueText>, pub speaker: Option<String>,
    pub layer: i32, pub style: Option<String>, pub layout: Option<CueLayoutHint>,
    pub words: Vec<CueWord>,            // ASS karaoke, VTT timestamp tags, enhanced LRC
    pub source_line: u32,
}
pub struct ParseReport { pub skipped: Vec<Issue>, pub warnings: Vec<Issue>, pub stripped_tags: u32 }

pub fn sniff(text: &str, ext_hint: Option<&str>) -> Option<SubtitleFormat>;
pub fn parse(text: &str, format: SubtitleFormat) -> Result<(SubtitleDoc, ParseReport), SubtitleError>;
pub fn write(doc: &SubtitleDoc, format: SubtitleFormat, opts: &WriteOptions) -> Result<String, SubtitleError>;

/// TL-001: 705_600_000 ticks/s, so ms and cs convert exactly (R2).
pub const TICKS_PER_MS: i64 = 705_600;
/// Whole ms, half up (TX-032). Rounding the total first makes the field carry automatic (L5).
pub fn tick_to_ms_half_up(t: Tick) -> i64 { (t.0 + TICKS_PER_MS / 2).div_euclid(TICKS_PER_MS) }
```

**Sniffing (L9).** The content decides, not the extension, which is compared without
case and used only as a hint:
- `WEBVTT` header → WebVTT;
- `[Script Info]` → ASS or SSA;
- a line with `[mm:ss.xx]` → LRC;
- `h:mm:ss.mmm,h:mm:ss.mmm` → SBV;
- `-->` with `,` or `.` milliseconds after an optional index line → SRT;
- timecoded blocks without an index → TimedTxt.

**Parsing rules (every rule maps to a fixture):**
- **Shared rules:**
  - Line endings CRLF, CR and LF are all accepted. A leading U+FEFF is stripped.
  - Fraction digits set the scale: `.5` = 500 ms, `.05` = 50 ms, `.500` = 500 ms (L4).
  - Hours may have more than two digits.
  - Times are integer and checked. Overflow skips the cue with a report entry.
  - Cues that end before they start, or have zero length, are skipped and reported.
  - Text is never filtered by length (L10).
- **SRT:**
  - A timing line is accepted only where a cue may start: after an index line or a blank
    line. It must contain `-->` with two complete timestamps (L7).
  - Both `,` and `.` are accepted before the milliseconds. A missing index is fine.
    Trailing `X1:… Y1:…` coordinates are ignored and reported.
  - A blank line ends a cue.
  - After a malformed timing line, the parser skips to the next blank line followed by an
    index line or a timing line, and reports the skip with its line number. The fixture
    covers Kdenlive's "missing end time" case.
  - Tags `<b> <i> <u> <s>` and `<font color>` become spans. Others are stripped and
    counted. HTML entities and `&nbsp;` are decoded.
- **WebVTT:**
  - The header may carry text after `WEBVTT`. Cue identifiers are optional. `mm:ss.ttt`
    is accepted without hours.
  - The cue settings `line`, `position` and `align` map approximately to
    `CueLayoutHint`.
  - `NOTE`, `STYLE` and `REGION` blocks are skipped and reported.
  - `<v Name>` sets the speaker. `<c.x>` and `<lang>` are dropped. For `<ruby>` the base
    text is kept and the annotation reported.
  - Inline `<hh:mm:ss.ttt>` timestamps become word timings.
- **ASS/SSA (L8):**
  - The `Format:` lines in `[V4+ Styles]`, `[V4 Styles]` and `[Events]` give the field
    order.
  - Dialogue lines are split on the first `n − 1` commas only, so commas in text survive.
  - `Comment:` lines are skipped. `[Fonts]` and `[Graphics]` sections are skipped and
    reported. Embedded fonts are never installed.
  - In text: `\N` is a hard break. `\n` is a break only when `WrapStyle: 2`, otherwise a
    space. `\h` is a no-break space.
  - Colours may be `&HAABBGGRR&`, `&HBBGGRR` or a decimal (signed) integer. Alpha is
    inverted.
  - Override tags `\b \i \u \s \c \1c \3c \fn \fs \an \r` map to spans and layout hints.
    `\k \K \kf \ko` (centiseconds) become word timings. Other tags are stripped and
    counted.
  - `PlayResY` defaults to 288 and `PlayResX` to 384 when absent, matching libass.
- **LRC:**
  - One line may carry several timestamps (`[00:12.00][00:45.00]Chorus`), and each
    becomes its own cue.
  - The `[offset:±ms]` tag is applied with LRC's sign: positive makes lines appear
    earlier. Metadata tags are skipped.
  - Enhanced `<mm:ss.xx>` timestamps become word timings.
  - After expansion, cues are sorted. Each cue ends at the next start; the last lasts 4 s.
- **SBV:** `h:mm:ss.mmm,h:mm:ss.mmm` blocks, separated by blank lines.

**Converting to the timeline** (`convert.rs`; pure, emits timeline types):
- **Placement.** The offset is timeline start, the playhead or the start of the selected
  clip. An optional frame-rate change uses `caption::retime::LinearMap` (rational,
  rounded half to even).
- **Overlaps → extra tracks (TX-031).** Cues are sorted by (start, end). Each goes to the
  first depth whose last end ≤ its start. This greedy interval partition gives the
  minimum number of tracks.
- **Cues shorter than one frame** are extended to one frame, with a warning (TX-025).
- **Styles.** The most-used ASS style becomes the track style; the others become segment
  overrides.
- **ASS font size is not the em size.** VSFilter-compatible renderers, libass included,
  treat `Fontsize` as the font's *cell height*, roughly winAscent + winDescent. So
  em = Fontsize × upem / (winAscent + winDescent), scaled by canvas height / PlayResY,
  then converted to ref px.
  - The parser is pure and has no fonts, so the app performs this step through a small
    `AssMetrics` trait that `freecut-render` implements. Unknown fonts use a factor of
    1.0.
  - The cell-height semantics should be confirmed against libass behaviour during
    implementation.
- **Outline and shadow.** ASS `Outline` is an outer border in script px, which maps to
  `Stroke.width` as a fraction of the em. ASS `Shadow` s becomes distance s√2 at 45°.
  `BorderStyle 3` becomes a per-line box.
- **One hour offset.** When the first cue is at ≥ 1 h, the dialog offers to subtract
  1 h (TX-031).
- **Result.** One `ImportSubtitles` op containing the created tracks and the splices, so
  50,000 cues are one undo entry (R8).

**Writing (TX-032):**
- **Building the document.** It is built purely from the tracks: sort, drop empty
  segments, trim to the range, rebase, choose the bilingual mode, add the optional
  speaker prefix.
- **Rounding.** Times go through `tick_to_ms_half_up`. When rounding makes end equal to
  start, end gets +1 ms.
- **SRT.** Index from 1. `HH:MM:SS,mmm`, where hours may exceed 99. CRLF by default.
  - A cue text must never contain an empty line, because players treat that as the end
    of the cue.
  - Leading and trailing empty lines are trimmed. Inner empty lines collapse into one
    line break, and the loss is reported.
  - ASS tags never leak into SRT (Kdenlive SRT export).
- **WebVTT.** `WEBVTT` header, `HH:MM:SS.mmm`. `&`, `<` and `>` are escaped, which also
  keeps `-->` out of cue text. No empty lines inside a cue.
- **ASS:**
  - `[Script Info]` has PlayResX/Y = the canvas, `ScaledBorderAndShadow: yes`,
    `YCbCr Matrix: None` and `WrapStyle: 0` (from Kdenlive).
  - `Format:` lines come first, and colours are written as `&HAABBGGRR`.
  - Times are written as `H:MM:SS.cc`, rounded on the total centiseconds (L5).
  - Literal `{`, `}` and `\n`, `\N`, `\h` sequences in user text are neutralized: U+2060
    is inserted after the backslash, and braces are escaped where libass supports it.
    This is reported as lossy.
- **LRC.** `[mm:ss.xx]`, where minutes may exceed 59. Multi-line text is joined with a
  space.
- **TXT.** A transcript with optional timestamps.
- **Atomic write.** `tempfile::NamedTempFile::new_in(target_dir)`, then write, then
  `sync_all`, then `persist`. On error, no partial file remains (TX-032).

**Reading files and encodings** (app background job, TX-031). The reading job:
1. Refuses files over 64 MB, a FreeCut default.
2. Detects a byte-order mark with `encoding_rs::Encoding::for_bom` (UTF-8, UTF-16 LE,
   UTF-16 BE).
3. Otherwise runs a strict UTF-8 check.
4. Otherwise feeds **the whole buffer** to `chardetng` (L9), with a TLD hint from the
   user's locale.
5. Decodes with `encoding_rs`, counting replacement characters into the report.

The dialog previews the first cues and re-decodes when the user overrides the
encoding. Nothing here runs on the UI thread (R3, L23).

### Embedding subtitle streams (`freecut-media::mux::subtitle`)

The codec table was verified with the local FFmpeg 8.1.1. The test muxed an H.264 or
VP8 stream with one SRT-sourced subtitle stream:

| Container | Codec | Result on FFmpeg 8.1.1 | Styling kept |
|---|---|---|---|
| MP4, MOV | `mov_text` | OK; `subrip` and `webvtt` are refused ("Could not find tag for codec") | `<b> <i> <u>` round-tripped; a `<font color>` colour did not |
| MKV | `ass` (default), or `subrip` / `webvtt` | all OK | full ASS / SRT tags |
| WebM | `webvtt` | OK; `subrip` refused ("Only … WebVTT subtitles are supported") | b/i/u |
| GIF, audio-only, image sequences | — | — | Embed disabled (TX-033) |

**Overlaps.** With overlapping cues, FFmpeg's mov_text output shortened the first cue
to end where the second began. Within one FreeCut track, segments never overlap, so each
caption track becomes its own stream. Bilingual segments become one two-line cue
(TX-033).

```rust
pub enum SubtitleCodec { MovText, Ass, SubRip, WebVtt }
pub struct SubtitleStreamSpec {
    pub codec: SubtitleCodec,
    pub language: Option<[u8; 3]>, // ISO 639-2/B from BCP 47 (isolang); MP4 needs 3 letters
    pub title: String, pub default: bool, // TX-033: default flag on the first embedded track
    pub ass_header: String,               // the text encoders require one (MLT, Olive)
}
pub struct EmbedCue { pub start: Tick, pub end: Tick, pub ass_event: String } // "0,0,Default,,0,0,0,,Text"

impl SubtitleStreamWriter {
    pub fn open(oc: &mut format::context::Output, spec: &SubtitleStreamSpec) -> Result<Self, MuxError>;
    /// Writes every pending cue whose start <= `video_pts`. Called by the export loop as video
    /// is encoded, so packets interleave as MLT's consumer does.
    pub fn advance_to(&mut self, oc: &mut format::context::Output, video_pts: Tick) -> Result<(), MuxError>;
    pub fn finish(&mut self, oc: &mut format::context::Output) -> Result<(), MuxError>;
}
```

**FFI notes (R12).** `ffmpeg-next` 9.0's subtitle API has three gaps:
- `codec::subtitle::Subtitle` has no `Drop` and never calls `avsubtitle_free`, so its
  rects leak.
- `encoder::subtitle::Encoder::encode` returns `Ok(true)` and drops the byte count that
  `avcodec_encode_subtitle` returns.
- `RectMut::set` calls `CString::new(..).unwrap()`, which panics on an interior NUL.

The writer therefore uses `ffmpeg-sys-next` directly, in `freecut-media` only:
- **Header.** `subtitle_header` is allocated with `av_malloc`, NUL-terminated, and its
  size set **without** the NUL, before `avcodec_open2`.
  `// SAFETY: avcodec_free_context frees this with av_freep, and the text encoders parse
  it as a C string.` (Olive's `new[]` without a NUL, L24.)
- **Per cue:**
  1. Fill a zeroed `AVSubtitle` with one `av_mallocz` rect of type `SUBTITLE_ASS` and
     `ass = av_strdup(event)`. U+0000 is stripped from the text first.
  2. Set `end_display_time` to the duration in ms.
  3. Call `avcodec_encode_subtitle` with a 1 MiB buffer, as MLT does, and **use the
     returned size**.
  4. Call `avsubtitle_free`.
  5. The packet gets pts and duration in ms, then `av_packet_rescale_ts` into the stream
     time base, then `av_interleaved_write_frame`.
- **Metadata.** BCP 47 maps to ISO 639-2/B (`pt-BR` → `por`; unknown → `und`). The
  `title` metadata is the track name, and `AV_DISPOSITION_DEFAULT` goes on the first
  embedded track.
- **Range exports** drop the cues outside the range, trim the partial ones and rebase
  times (TX-033, EX-045).

### Editing text (`freecut-render::text::edit` + `freecut-app`)

**Why not egui's widgets.** egui's text layout cannot shape Arabic or Indic scripts
(UI-046). Olive's proxied `QTextEdit` shows how awkward borrowing a toolkit editor is.
So FreeCut edits with its own stack:
- **EditActor.** It runs on the TextService thread and owns:
  - the `RichText` buffer;
  - the selection, kept with parley's public `Selection` and `Cursor`
    (`parley-0.12.0/src/editing/`) over a parley layout built from our ranged styles;
  - the composition range;
  - a local undo history (TX-002: Undo inside edit mode walks the field history first).
- **Commands from the UI:** `Insert(String)`, `DeleteBackward/Forward/Word`,
  `Move { motion, extend }`, `ClickAt(x, y, clicks)`, `Compose { preedit, cursor }`,
  `CommitCompose`, `CancelCompose`, `Paste(String)` (plain text; CRLF → LF; tabs → 4
  spaces), `ApplyRangePatch`, `Commit`, `Cancel`.
- **Replies** (`EditView`, latest wins): the text for the inspector mirror, the selection
  as byte ranges, the caret rect, selection rects, composition underline rects (all in
  ref px), and the layout generation.
- **Live preview.** The preview request carries
  `EditOverride { item, Arc<RichText>, hash }`, so the canvas shows every keystroke with
  no undo entry. On **Commit** (Esc, Ctrl/Cmd+Enter or a click outside), the app emits
  one `SetTextContent` or `SetRichText` op, and none when nothing changed (TX-002).
- **egui glue** (`freecut-app::text::canvas_edit`):
  - Key events and `egui::Event::Ime` (preedit and commit) become commands.
  - Every frame, egui's IME output rect is set to the caret rect mapped to screen space,
    so candidate windows appear next to the caret.
  - The overlay draws selection and caret from the last `EditView`. One frame of latency
    is the price of keeping shaping off the UI thread (R3).
- **Inspector field and Captions panel cells.** In the mvp these use egui `TextEdit`,
  with the known limitation for complex scripts. In v1 they become a custom widget backed
  by the same EditActor and by TextService-rendered text images.
- **Font picker (TX-003).** The family list comes from the snapshot. Each family's name
  is rendered in its own face by TextService on the CPU (swash coverage) into small
  images, only for rows that are visible, and uploaded as egui textures. Hovering sends a
  transient `StyleOverride` in the preview request, which creates no undo entry.

### Threads and data flow

| Thread | Owns | Does | Never |
|---|---|---|---|
| UI (egui) | last `EditView`, last `TextMetrics`, picker textures | input, IME, dialogs, emitting ops | shaping, font reads, raster, file I/O |
| Font discovery | fontique collection build | scan, import validation and copy, rescan, snapshot publish | GPU |
| TextService | TextEngine #0, EditActor, thumbnail raster | measure (handles, snapping, warnings, fallback report), edit layout, previews | GPU |
| Preview scheduler (render) | TextEngine #1, GPU atlas, text pipelines | layout, SDF, text passes for preview frames | — |
| Export | TextEngine #2, `SubtitleStreamWriter` | the same for every export frame; subtitle packets | dropping frames |
| Job pool (app) | — | subtitle read, decode, parse, write | GPU |

Shared state:
- `ArcSwap<Arc<FontSnapshot>>` for fonts.
- `quick_cache` for laid-out text and CPU glyph bitmaps; values are `Arc`, so threads
  share results.
- Each TextEngine owns the parley contexts and a swash `ScaleContext`, none of which are
  shared.
- Measure requests use a latest-wins slot per item, which is the same idea as
  `PreviewRequest` in `playback-engine.md`.

### Errors (`thiserror` in libraries, `anyhow` only in `freecut-app`)

```rust
// freecut-timeline
#[derive(Debug, thiserror::Error)]
pub enum RichTextError {
    #[error("range {start}..{end} is not on grapheme boundaries")] NotGraphemeAligned { start: u32, end: u32 },
    #[error("range out of bounds")] OutOfBounds,
    #[error("text contains NUL or CR")] ForbiddenChar,
    #[error("text longer than {max} bytes")] TooLong { max: u32 },
    #[error("spans are not canonical")] NonCanonicalSpans,
}
// caption ops reuse TL's EditError: Locked(TrackId), WouldOverlap { seg: SegId, other: SegId },
// ShorterThanFrame, NotAdjacent, TimeOverflow (TL-001), Empty.

// freecut-render
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("cannot read {path:?}")] Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("{path:?} is not a usable font: {reason}")] Parse { path: PathBuf, reason: String },
    #[error("font already imported")] Duplicate { postscript: String },
}
#[derive(Debug, thiserror::Error)]
pub enum TextRenderError {
    #[error("glyph atlas exhausted")] AtlasExhausted,      // only when a single frame needs > budget
    #[error(transparent)] Gpu(#[from] wgpu::Error),
}
// A missing font is not an error: it renders substituted and is reported (TX-007).

// freecut-project
#[derive(Debug, thiserror::Error)]
pub enum SubtitleError {
    #[error("file contains no cues")] Empty,
    #[error("unrecognized subtitle format")] Unrecognized,
    #[error("time overflow at line {line}")] TimeOverflow { line: u32 }, // when not cue-local
}
// freecut-media
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    #[error("container cannot carry {0:?}")] Unsupported(SubtitleCodec),
    #[error("{op} failed: {code}")] Ffmpeg { op: &'static str, code: i32 },
}
```

### Invariants and how the design meets the conventions

| Invariant / convention | Where enforced |
|---|---|
| RichText is canonical: no NUL or CR, sorted disjoint non-empty spans on grapheme boundaries, merged | `RichText::from_parts`, serde `try_from`, proptest |
| Caption segments are sorted, non-overlapping, ≥ 1 frame when created, with unique stable ids | `CaptionTrackData::splice` is the only mutator |
| Visible ⇔ start ≤ t < end; one shared `overlaps()` | `caption/track.rs` |
| **Time is rational**: ticks everywhere; ms/cs exact; floats only for unit progress, shader uniforms and layout positions in ref px | `subtitles/time.rs`, `animate.rs` |
| **Every edit goes through undo** and is reversible with a test | TX ops, `SpliceSegments` inverse, generic round-trip property test |
| **No decode, render or file I/O on the UI thread** | the threads table above; EditActor and TextService |
| Layout does not depend on raster scale (R1) | layout API takes no scale; `quantize = false`; property test |
| Reveal never reflows (R7) | animation reads `LaidOutText`, never rebuilds it |
| Portable randomness (R6) | frozen SplitMix64 + golden vectors |
| `unsafe` only in media/render with `// SAFETY:` | `freecut-media::mux::subtitle` (FFmpeg), `freecut-render::text::gpu` (none expected; wgpu is safe) |
| No `unwrap`/`expect` outside tests unless the invariant is commented | parsers return `Issue`s; narrowing casts carry a comment |

### FreeCut defaults introduced here

| Setting | Value |
|---|---|
| Em size buckets | 8 per octave, 4–1024 px |
| SDF spread | 0.30 em + 2 px (outline or glow used), else 0.06 em + 2 px |
| SDF format | `R16Float`; colour glyphs `Rgba8Unorm` (not sRGB) |
| Atlas pages and budgets | 2048²; SDF 128 MB, colour 64 MB, layer cache 256 MB, layout cache 64 MB |
| Blur | σ = r/3; downsample above r = 32 px; cap 256 px at 4K, scaled with the canvas |
| Synthetic styles | bold threshold −0.02 em; italic shear 12° |
| Missing-glyph box | 0.5 em × 0.7 em, stroke 0.06 em |
| Subtitle import size limit | 64 MB |
| Caption sampling | frame start, `start ≤ t < end` |
| Per-character animation in cursive scripts | degrades to per word |

### Implementation note: the CPU text path (M4/M5 slice, 2026-10-12, README C44)

The first build is a CPU-only subset of this design, used by the CPU compositor
(render-compositor.md §17) for both preview and export. It differs from the design
above in these points:

- **Entry point.** `freecut_render::text::TextSpec` describes one block of uniformly
  styled text (no rich spans, outline, shadow, curve, justify, vertical anchor or
  letter spacing yet). `TextSpec::from_clip(&TextClipData, scale_px)` is the only place
  that turns the model's fixed-point values into floats: size and fixed box width are
  ref px × `scale_px`, line spacing is per-mille / 1000, CRLF becomes LF.
  `TextLayer::from_plan` places the box with its centre at the plan's text origin.
- **Layout** runs with parley 0.12.0 at display scale 1, `quantize = false` and no
  hinting, so line breaks never depend on the raster scale (R1). Wrapping is always on;
  without a fixed box the wrap width is unlimited, so only explicit breaks end lines.
  A fixed box wraps at the box width (at least 1 em), mid-word only as a last resort.
- **Alignment** is done by FreeCut, not parley: every line is laid out flush left and
  then shifted so trailing whitespace never counts toward width or alignment (TX-005).
  Leading spaces still count. Justify is not supported yet.
- **Fallback.** A family that exists uses the stack [family, Noto Sans], then
  parley/fontique per-script and emoji fallback. A missing family uses [Noto Sans]
  and sets `FallbackReport::family_missing`; the report also lists other families used
  and characters no font has. The export preflight's `FontsSubstituted` warning uses
  `FontRegistry::has_family` once the system scan has finished.
  TX-006's own missing-glyph box is not drawn yet (the font's `.notdef` is).
- **Precedence.** System > Bundled, as above: after the system scan, a bundled face is
  registered only when the scanned system set has no family of that name, so an
  installed Noto Sans (often newer or variable) is used with all its faces and the
  bundled one stays hidden. (fontique looks up registered fonts before system ones, so
  registering the bundled faces anyway would shadow the system family.) Imported fonts
  (TX-018) don't exist yet; they will need the same care, or `FontSnapshot::resolve`.
- **Discovery.** `FontRegistry::bundled()` is usable at once; `with_system_fonts()`
  scans on a `freecut-fonts` thread and bumps a generation when it publishes. Each
  `TextRenderer` rebuilds its parley contexts and drops its caches when the generation
  changes. `ExportRenderer::new` waits for a running scan (up to 30 s), so export
  output never depends on scan timing.
- **Raster.** swash 0.2.10, unhinted, colour glyphs included; synthetic bold and
  italic follow parley's synthesis hints (not the fixed thresholds in the table above).
  Monochrome glyphs add their coverage into one 8-bit mask (saturating) and the fill
  colour and alpha are applied once, so overlapping glyphs (combining marks, joined
  scripts) don't double a translucent fill and abutting antialiased edges leave no
  seams; colour glyphs are composited "over" that mask in text order. swash's COLR
  images are premultiplied (layers are blended into a zeroed buffer) and are used as
  such; bitmap glyphs (sbix, CBDT) are straight alpha. COLR layers that use the text
  colour (palette index 0xFFFF) get the fill colour (`Render::default_color`).
  Rasters are straight alpha: monochrome text gets the exact fill RGB in every pixel;
  with colour glyphs, fully transparent pixels get the fill RGB. Rasters are cached by
  spec and scale within a 64 MB LRU budget; there is no SDF atlas yet.
- **Placement.** An upright, unmirrored text layer (the usual caption) is rasterized at
  exactly its output scale and, when its x and y scales agree to within half a pixel
  over the raster, placed on whole output pixels (its position rounded to the nearest
  pixel), so the compositor copies the raster 1:1 instead of resampling it (TX-004).
  Text therefore moves in whole-pixel steps during slow animated motion. Rotated, sheared, mirrored or non-uniformly scaled
  text is resampled bilinearly from a raster at the larger axis scale rounded up to
  1/64.
- **Bundled fonts.** Noto Sans v2.015 Regular, Bold, Italic and Bold Italic (unhinted
  static TTF, OFL-1.1, listed in `assets/ASSETS.md`) are compiled in with
  `include_bytes!` (about 1.75 MB).

### Round 2 deltas (2026-10-12, README C46)

The text spec's round 2 (TX-054 to TX-056, with FX-030) changes this design in five
places; C46 records the decision and the round-2 plan orders the work:
- **Style layers.** `Stroke`, `Shadow`, `Glow`, `TextBox`, the new `TextBubble` (TX-055)
  and the curve are stored as `Layer<T> { on, value }`, so switching a layer off keeps its
  values and keyframes. The field lists above that say `Option<T>` are superseded.
- **Effects on the CPU path.** Until the wgpu passes above exist, outline, glow and shadow
  are built from a distance transform of the glyph coverage mask, boxes and bubbles are
  `zeno` paths, and text layers are composed premultiplied so glow stays additive.
- **Per-glyph placement.** Glyphs are rasterized one by one and placed with an affine or
  projective transform per glyph (per-unit motion, curve, 3D turns). Layout still runs once.
- **Pace.** `AnimParams.pace` turns a per-unit time into a slot duration through the pure
  `pace_duration` in `freecut-timeline`; letter and word counts come from Unicode
  segmentation, so the model needs no layout. Line units have no Pace.
- **Preset manifests.** Text presets are FX-030 manifests in `resources/text-animations/`;
  the evaluator above reads their channels instead of hard-coded curves.

### Round 2 render implementation (S1, 2026-10-12, README C46)

What `freecut-render` built for round 2, before the timeline adapter maps the model onto it:

- **Input.** `TextSpec` (render-owned, decoupled from the timeline) gained `fill: Fill`
  (`Solid`, `Linear`, `Radial` with 2–8 stops and a `FillExtent` of Block or Line),
  `letter_spacing`, `decoration`, `stroke`, `shadow`, `glow` (with `GlowDirection`),
  `background: Option<TextBox>`, `bubble: Option<TextBubble>` and `curve`, plus
  `TextAlign::Justify`. These are floats at the render edge: lengths the spec defines
  relative to the font size are fractions of `size_px` (per-mille / 1000), opacities and
  intensities 0..=1, angles degrees clockwise from the right. A model `Layer` that is off is
  `None`. `text` is the display text: the adapter applies the case transform (TX-008)
  before building the spec. A spec with both background and bubble draws the bubble (TX-054).
- **Layout.** Letter spacing is parley's `LetterSpacing` (applied at atom boundaries, so
  never inside a ligature or joined run); the gap after a line's last cluster is not
  content, so widths and alignment exclude it. Justify uses parley's justification for
  soft-wrapped lines of a fixed-width box (their width is the box width); paragraph-final
  lines and auto-width text are set left. While curved, the fixed width is ignored (TX-005).
  `TextLayout::glyphs` lists every glyph as a `PlacedGlyph { font, size, glyph_id, origin,
  advance, cluster, unit }` at rest. `UnitIds` count over the whole display text in logical
  order: `grapheme` = UAX #29 extended grapheme cluster (every cluster counts, spaces and
  breaks included), `word` = UAX #29 word (segments with an alphanumeric character; spaces
  and punctuation belong to the word before them), `line` = laid-out line.
- **Masks.** Monochrome glyph coverage is added into one 8-bit mask; underline and strike
  (font metrics, per run, clipped to the line content) are joined as a union, so they get
  outline, shadow and glow like glyphs. The silhouette also includes colour glyphs. Its
  signed distance field comes from an exact Euclidean distance transform (Felzenszwalb and
  Huttenlocher, our own code, with nearest-feature indices): partly covered pixels are at
  `0.5 − coverage`, others add the exact distance to the nearest pixel across the edge to
  that pixel's own distance, so bands stay antialiased at any width.
- **Outline** (TX-010) is conflation-free: the ring outside the silhouette is outline
  only, the band inside it is outline over fill, the rest fill only, so no seam appears and
  a transparent fill leaves a hollow ring. Outside: ring of the width; Center: half outside,
  half inside; Inside: band inside only.
- **Glow** (TX-012) adds light (premultiplied colour, alpha 0). Outward: the letter shapes
  with their outline grown by a third of the radius, then blurred over two thirds of it
  (`σ = r/3`), so thin strokes glow like thick ones and the light fades at about the
  radius. Inward: the blurred outside (`σ = r/2`, ×2, capped at 1) clipped to the
  silhouette, added between the fill and an Inside or Center outline. The offset shifts both.
- **Shadow** (TX-011) is cast by the body's alpha (fill and outline; with
  `include_box`, also the box or bubble), blurred with `σ = blur/2`, offset at the angle.
- **Blur.** Separable Gaussian, `σ = radius/2`, kernel and raster padding 3σ (beyond it the
  shadow is below 0.5/255, so no hard edge shows); radii above 32 device px are blurred at a
  power-of-two reduced resolution and scaled back bilinearly; radii are capped at 256 device
  px. All arithmetic is sequential `f32`, identical on every thread.
- **Boxes and bubbles** (TX-013, TX-055) are polygons from `freecut_render::shape`, filled
  with `zeno` (=0.3.3, the version swash uses) as a union (contours turned the same way,
  non-zero rule), composited once at the opacity. The border is drawn inside the union's
  outline from the union mask's own distance field, so overlapping per-line boxes have one
  border. Per line: the line box (empty lines skipped); per character: each non-blank
  cluster's advance minus the letter gap; Lower half: from half the x-height above the
  baseline to 0.1 em below it, with horizontal padding only (FreeCut default); Block
  ignores Height. Both pause while curved.
- **Bubble shapes** (`shape::bubble`): Rounded, Oval, Thought, Shout, Square, Tag, Arrow tag
  and Ticket are star-shaped outlines listed by angle. The tail replaces the outline points
  within its angular span (base width as a fraction of the shorter side, centred where the
  ray from the body centre to the tip leaves the body), so body and tail are one contour;
  both edges are quadratic curves bent the same way by up to 20 % of the tail length. On a
  Shout the base snaps to the valleys beside it so neighbouring spikes point away. A tip
  inside the body hides the tail. Thought draws 2–4 shrinking circles from the ellipse to the
  tip. The burst's irregularity comes from SplitMix64 on the stored seed. `bubble_body` sizes
  the body (text box plus padding, elliptic kinds grown by √2 to hold the corners, at least
  2 × 1.5 em).
- **Curve** (TX-016): the widest line's length is `|c| × 360°` of arc; all lines share the
  centre on concentric arcs; the middle of the widest line stays put. Each glyph's advance
  centre sits on its arc and the glyph is rasterized turned to the tangent (swash transform,
  cached by angle). Decorations are annular sectors along the arc.
- **Gradients** are evaluated in the rest pose: each glyph pixel is mapped back through its
  own placement, so a glyph keeps its colour when it moves; stops are interpolated
  premultiplied.
- **Raster.** `TextRaster::image` is premultiplied RGBA; colour exceeds alpha where glow
  adds light. Layer order: box or bubble, shadow, outward glow, outline ring, fill
  (colour glyphs over it), inward glow, inside or centred outline band. The cache key
  includes every style field (the key is built from a full destructuring of `TextSpec`).
  Rasters are kept within 8192 px a side and about 2^23 px in area (scale lowered beyond).
- **Not yet:** per-range spans (TX-015), skip-ink underlines, gradients on outline or
  shadow. (The projective per-glyph sampler and per-unit motion came with S2, below.)

### Round 2 render implementation: per-unit animation (S2, 2026-10-13, README C46)

What `freecut-render::text::animate` and the raster built for TX-022 steps 3 and 4,
TX-023 and TX-056, before the timeline adapter maps `TextAnimPlan` onto it. Where this
differs from the "Animation" section above, this wins.

- **Input (render-owned).** `TextMotion { unit: AnimUnit::{Grapheme, Word, Line}, order:
  UnitOrder::{Forward, Backward, CenterOut, EdgesIn, Random { seed }}, timing:
  UnitTiming::{In { span, overlap }, Out { span, overlap }, Loop { cycle, overlap }}, at:
  Tick, channels, reveal: Option<PerMille>, decode: Option<Decode { seed, step }>, caret:
  Option<Caret { blink }> }`, on `cpu::TextLayer::motion`. `at` is slot-local. The
  channels are either a `ChannelTable` (curves of (progress, value) points, linear or
  held, with an optional value while the unit is idle and optional per-unit seeded
  variation) or a callback `UnitFn(Arc<dyn Fn(&UnitSample) -> GlyphAnim>)`, through which
  the adapter runs the timeline's FX-030 manifest evaluator, so curves and easing are not
  duplicated here. A `UnitSample` gives the unit's reading-order index, play rank, count,
  progress (In: 0 start pose → 1 rest; Out: already reversed, 1 → 0, so easing is
  mirrored; Loop: the phase) and `idle` (In: before its window; Out: from its end), which
  is how a step "at the start of the window" (Typewriter) differs from "not started".
- **Pose.** `GlyphAnim { offset (em), scale, scale_xy, rotation (° clockwise), turn (°
  about the unit's x and y axes), opacity, blur (em), reveal }`, sanitized (non-finite →
  rest, opacity and reveal 0..=1, blur 0..=4 em). Several table curves on one property
  combine as composition does: offsets, angles and blur add; scales, opacity and reveal
  multiply.
- **Units** (`UnitMap`): letters and words from Unicode segmentation of the display text
  (as the model's Pace counts them), lines from the layout. Whitespace-only units take no
  time and move with the timed unit before them (leading ones with the first). Text with
  a character of a cursive-joining script (Arabic, Syriac, N'Ko, Mongolian, Adlam and
  similar) degrades letters to words for the whole text; `UnitMap::unit` and
  `MotionState::unit` report the unit used. Punctuation-only text is one word unit.
- **Windows, delays, order.** `unit_windows` as above (exact integer split; last window
  ends at the span; negative spans count as 0, overlap above 1000 as 1000).
  `loop_delay(cycle, n, rank, overlap) = ⌊rank × cycle × (1000 − overlap) / (1000 n)⌋`;
  loops use the unit's play rank, so a backward wave runs right to left. CenterOut sorts
  by distance from the middle with ties left first; EdgesIn alternates first, last,
  second, second-to-last. Random: SplitMix64 seeded with the stored `u32`, Fisher–Yates
  from the last element, bounded draws by Lemire's multiply-and-reject. `unit_noise(seed,
  unit, salt) ∈ [−1, 1)` (24 random bits) gives per-unit variation. Golden vectors for
  seeds 0, 1 and 0xDEADBEEF pin SplitMix64 and the shuffle (cross-checked against an
  independent implementation); the Decode choice and `unit_noise` have golden values too.
- **Applied about each unit's own centre.** A letter's ink-box centre (from its unrotated
  glyph bitmap), a word's or line's box centre (the union of its non-blank glyphs'
  advances over the line box). Per glyph: rest pen → `T(c + offset) · R · S(scale ·
  scale_xy) · Turn · T(−c)` → the curve's rigid frame for the glyph (TX-016). The 3D turn
  (y axis, then x axis) is projected with the eye 1.5 × the unit's width away (at least
  half an em), FX-030's rule, and applied before the 2D rotation.
- **Drawing.** A glyph that only moves (no scale, rotation or turn) is rasterized crisp at
  its moved pen with the usual sub-pixel steps and glyph cache. Any other glyph is
  rasterized flat at the output scale times its largest scale (rounded up to 1/8, so
  frames share bitmaps) and resampled through its 3 × 3 homography
  (`text::projective::resample`, bilinear, premultiplied for colour glyphs); a point at
  or behind the eye has no image, so an edge-on letter draws nothing. Gradients are
  evaluated at each pixel's rest position through the inverse map, so moving glyphs keep
  their colours. Blur is a Gaussian of the glyph's own coverage (σ = radius / 2) before
  the layer stack, so its outline and glow follow the softened shape.
- **Opacity** is a per-pixel plane: inked pixels take the coverage-weighted opacity of
  the glyphs over them, others (outline ring, glow, shadow) that of the nearest inked
  4 × 4 block (an exact distance transform on the reduced grid). It multiplies the body
  (fill, outline, inward glow) and the outward glow's source; the shadow is cast by the
  faded body. Boxes are not faded by glyph opacity.
- **Reveal.** The unit's reveal clips its glyphs at `x0 + reveal × width` of the unit's
  rest box (from the right in right-to-left text), the Reveal keyframe clips at
  ⌊letters × reveal⌋ non-blank graphemes in reading order (a ligature proportionally),
  and a glyph shows where both let it (`Shown::and`); the clip is a one-pixel soft edge
  in rest coordinates, so it moves with the glyph.
- **Decorations** under motion are cut into one piece per glyph advance and moved,
  faded, blurred and revealed with their glyph.
- **Boxes** (TX-013) follow the revealed glyphs, not their opacity: per character, each
  cluster box moves with its glyph and is clipped to its revealed part; per line, the box
  spans the revealed clusters of the line at rest; Block spans the revealed clusters
  (first to last revealed line) at rest and is the full block once everything is shown
  (FreeCut default: with a fixed-width box whose lines are narrower, it widens to the box
  width at that moment). Opacity-zero but revealed glyphs keep their box, which is how
  Marker sweep draws its box before its text. A bubble is never revealed per unit.
- **Decode.** Letter units only (not on words or lines, not in loops): while a unit's
  progress is below 1 it shows a stand-in, a different face-and-glyph among the text's
  single-glyph letters, chosen by SplitMix64 from `(seed, unit, ⌊at / step⌋)`, drawn
  centred in the replaced glyph's advance. Text with fewer than two distinct such
  letters gets no stand-ins (the adapter plays Letter fade, TX-056).
- **Caret.** After the last revealed non-blank glyph in reading order (on its left in
  right-to-left text; at the start of the first line before any letter), 0.08 em wide,
  the line's height, drawn as part of the body (fill colour, outline). Steady until the
  last unit's window starts (when Typewriter's last letter appears), then on for the
  first half of each blink period; gone from the end of the slot. On curved text it
  follows the arc.
- **Layout and cache.** Animation never reflows: `TextRaster::layout` and the box origin
  are the rest layout's. `TextRenderer::rasterize_animated` keeps the built layouts of up
  to 8 animated specs, evaluates the motion (`MotionState`), and returns the still raster
  when `MotionState::is_rest`; otherwise the cache key is the spec key plus
  `MotionState::digest` (every glyph's pose bits, −0 as 0, its revealed range and
  stand-in, and the caret), so frames whose glyphs stand still reuse one raster. Animated
  rasters share the 64 MB budget but may hold at most 16 MB of it.
- **Determinism.** Integer time and sequential `f32`/`f64` arithmetic only; two threads
  rendering the same frames produce identical bytes (tested).

### Round 2 render implementation: the plan adapter (S2, 2026-10-13, README C46)

How `freecut-render::motion` connects the timeline's frame plan to the renderer above.

- **Planning.** `motion::plan_frame(doc, seq, t, inputs, presets, &mut TextRenderer)`
  (also `Compositor::plan`) calls `frame_plan_with` with the preset catalogue and a
  `text_bounds` callback that measures the text box at rest from the renderer's layout
  (`TextRenderer::box_size`, from the kept layouts), anchored as `TextAnchor::for_v_anchor`
  says. Export plans every frame this way (`ExportRenderer::with_presets`; without it only
  keyframes play), so a preview planned through `Compositor::plan` draws the same bytes.
- **What arrives already done.** Keyed style values are in `LayerSource::Text::data`
  (step 1, and a whole-text Loop's style modulation); block offsets are in
  `layer_to_canvas` and the slot's opacity in `LayerPlan::opacity`. The compositor draws
  them as it draws any layer.
- **Per-unit presets.** `TextLayer::from_plan` maps `LayerMotion::text` (`TextAnimPlan`)
  to `TextMotion` (`motion::text_motion`): Letter → `Grapheme`, Word, Line; the order
  (Random seeded by `params.seed`); In and Out as `UnitTiming::In`/`Out` over the slot
  window with `at` = local time − window start; Loop as `UnitTiming::Loop` over the cycle
  with `at` = local time + phase (the timeline's loop clock, so the first unit's phase is
  the plan's `progress`); drifts and combined presets as `In` over their window. Each
  unit's pose is the timeline's `pose_at(manifest, spec, slot, p, scale)` at the unit's
  own progress `p` (Out: `1 − UnitSample::progress`, undoing the renderer's reversal,
  because `pose_at` plays exits itself), with lengths in em: `em = 1`, `se` = the canvas
  short edge in em as drawn (clip scale included), Travel Short = 3 em (`pose_at`
  applies k). A unit whose In window has started counts as started even at progress 0
  (`p` = the smallest positive value), so a step (Typewriter) shows on its first frame;
  an idle unit holds its start pose.
- **Per-unit From edge** has no unit positions at plan time: it is estimated as the
  distance from the layer anchor to the canvas edge on the direction's side plus the
  estimated box (0.55 em per character, 1.2 em per line) plus one em, so every unit
  starts outside. Per-unit pivots other than the unit centre, per-unit tint and per-unit
  style channels have no glyph form and are ignored.
- **Decode** is set when the manifest has a `decode` channel, seeded by `params.seed`,
  with a fixed step of 1/15 s (TX-056's two frames at 30 fps): the layer doesn't know the
  sequence frame duration, and a fixed step keeps preview and export equal.
- **Reveal key.** `LayerMotion::text_reveal` < 1 becomes `TextMotion::reveal` (per
  mille, rounded); without a per-unit preset it travels in a motion whose units are all
  at rest.
- **Not drawn yet:** a whole layer's blur, wipe and white tint (`LayerMotion::blur_px`,
  `reveal`, `tint`) and the perspective of a whole-layer 3D turn (`draw_rows` uses the
  affine part of `layer_to_canvas`).

---

## Rust ecosystem

Versions and licenses were checked on 2026-10-12 with `cargo info` and the crates.io
API. Dependency versions were read from the crates' own manifests.

**Version alignment:**
- `parley` 0.12, `harfrust` 0.12 and `fontique` 0.12 sit on `skrifa` 0.44 /
  `read-fonts` 0.41.
- `swash` 0.2.10 accepts `skrifa` ≥ 0.31.1, ≤ 0.44, so the whole stack resolves to one
  skrifa.
- fontique is used through `parley::fontique`, so its version always matches parley's.

**Use**

| Crate | Version | License | Why |
|---|---|---|---|
| `parley` (features `system`, **`complex-scripts`**) | 0.12.0 | Apache-2.0 OR MIT | It meets TX-005, TX-006 and TX-009 directly. Rich ranged styles. `LineHeight::MetricsRelative`. A line box built from the actual run metrics (`LineMetrics`), so the tallest fallback font sets the line. `Alignment::Justify` with `last_line_alignment`. Hanging trailing whitespace. `OverflowWrap::BreakWord`. `TextWrapMode::NoWrap`. Letter and word spacing. `Locale`. Variable-font axes. Bidi. Public `Cursor`/`Selection`, plus IME composition helpers in `PlainEditor`. **`complex-scripts` is required:** without it ICU's dictionary segmenters for Thai, Lao, Khmer and Myanmar are off, and that text gets no line-break opportunities (`parley_engine-0.12.0/src/analysis.rs`). |
| `fontique` (via `parley::fontique`) | 0.12.0 | Apache-2.0 OR MIT | System font enumeration through CoreText, DirectWrite and fontconfig. Registering fonts from memory. Editable per-script and per-locale fallback lists (`set_fallbacks`, `set_generic_families`). |
| `skrifa` / `read-fonts` | 0.44 / 0.41 (pinned to parley's) | MIT OR Apache-2.0 | Reading `name`, `OS/2` (fsType, panose), `head` and `fvar` for the registry, fingerprints and the ASS metrics adapter. |
| `swash` | 0.2.10 | Apache-2.0 OR MIT | Unhinted outline coverage and colour glyphs (COLRv0, CBDT/sbix) at exact sizes. One `ScaleContext` per thread. |
| `sdf_glyph_renderer` (default features off) | 1.1.0 | BSD-3-Clause | Coverage-aware Euclidean distance transform (TinySDF / Felzenszwalb–Huttenlocher). Pure Rust when the `freetype` feature is off. Small enough to vendor later. |
| `etagere` | 0.3.0 | MIT OR Apache-2.0 | Atlas rectangle allocation, the same allocator glyphon uses. |
| `half` | 2.7.1 | MIT OR Apache-2.0 | `f16` packing for `R16Float` uploads. |
| `bytemuck` | 1.25.2 | Zlib OR Apache-2.0 OR MIT | `Pod` glyph-instance buffers. |
| `quick_cache` | 0.7.0 | MIT | Byte-weighted shared caches for layouts, glyph bitmaps and layer textures, as in `playback-engine.md`. |
| `xxhash-rust` (`xxh3`) | 0.8.19 | BSL-1.0 | Stable 128-bit layout, layer and environment digests that feed `render_key`. |
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | Publishing `FontSnapshot`; latest-wins request slots. |
| `crossbeam-channel` | 0.5.17 | MIT OR Apache-2.0 | TextService and EditActor command queues. |
| `unicode-segmentation` | 1.13.3 | MIT OR Apache-2.0 | Grapheme boundaries in `freecut-timeline` (RichText invariants, caption character counts). |
| `icu_segmenter` | 2.3.0 | Unicode-3.0 | Word and line boundaries for caption re-flow in the timeline. It is the same engine parley uses, so re-flow and rendering agree. Unicode-3.0 is permissive and GPL-compatible. |
| `icu_casemap` | 2.3.0 | Unicode-3.0 | Locale-aware display case (Turkish i, ß → SS, title case; TX-008). |
| `icu_properties` | 2.3.0 | Unicode-3.0 | `Script`, `Joining_Type` and emoji properties for the cursive guard and emoji spans. Parley already pulls it in. |
| `unicode-normalization` | 0.1.25 | MIT OR Apache-2.0 | NFC for search (TX-034) and for comparing font family names. |
| `encoding_rs` | 0.8.42 | (Apache-2.0 OR MIT) AND BSD-3-Clause | Decoding legacy encodings and the BOM handling for subtitle files. |
| `chardetng` | 1.0.0 | Apache-2.0 OR MIT | Encoding detection over the whole file (L9). |
| `isolang` | 2.4.0 | Apache-2.0 | BCP 47 → ISO 639-2 for MP4 and MKV stream language tags. |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | Atomic sidecar writes (temp file in the same directory, then `persist`). |
| `libm` (dev and curve layout) | 0.2.16 | MIT | Trigonometry that gives the same results on every OS, for curved-layout goldens. |
| `proptest` (dev) | 1.12.0 | MIT OR Apache-2.0 | RichText, caption-op, unit-window and retime properties. |
| `insta` (dev) | 1.49.0 | Apache-2.0 | Snapshots of writer output and layout dumps. |
| `cargo-fuzz` / `arbitrary` (dev) | 0.13.2 / 1.5.0 | MIT OR Apache-2.0 | Fuzzing the subtitle parsers (R11). |
| `image-compare` (dev) | 0.5.0 | MIT | Tolerance comparison for GPU golden images. |

**Avoid**

| Crate / approach | Why |
|---|---|
| `glyphon` 0.12 (wgpu 30) | It draws A8 glyphs with a colour and a clip, and nothing else. There are no per-glyph transforms, no outline, glow, shadow or blur, and no union outline (R4, R5). It also ties FreeCut to cosmic-text. Fine for UI overlays; not for the compositor. |
| `cosmic-text` 0.19 as the layout engine | It is mature, but its line height is a buffer- or span-level setting, so FreeCut would recompute line boxes for TX-006 itself. It also brings `fontdb` 0.23, `harfrust` 0.5 and `skrifa` 0.40 next to parley's 0.12/0.44 stack, which would mean two shapers and two skrifas. Its fallback lists are `&'static str` per platform. It is the fallback choice if parley's API churn becomes a problem. |
| `fontdb` directly | fontique already enumerates fonts through the native OS APIs, which match OS fallback behaviour. A second database would disagree with it. |
| libass, or FFmpeg's `subtitles`/`ass`/`drawtext` filters | They are absent from this machine's FFmpeg 8.1.1 build. They are a second styling model and renderer, with native FreeType, HarfBuzz, FriBidi and fontconfig on every OS. |
| `fdsm` / MSDF | Multi-channel fields cannot be max/min-unioned across glyphs, which breaks R5. Corner sharpness at low resolution is not needed, because FreeCut builds fields at output size. |
| `vello` 0.11 / `vello_cpu` 0.3 / `glifo` 0.4 (now) | A second rendering architecture, compute-only on the GPU, and young APIs. Re-evaluate for COLRv1 emoji, which `glifo` handles. |
| `rand::rngs::StdRng`, any `rand` distribution, for animation order | Not reproducible across releases. MLT's `normal_distribution` shows the same class of problem (R6). |
| `subparse` 0.7 (MPL-2.0) and other subtitle crates (`aspasia`, `rsubs-lib`, `srtlib`, `subtp`) | The licenses are compatible. But none covers SRT + VTT + ASS + LRC + SBV with line-numbered reports, exact ticks, our span mapping and fuzz-hardened recovery. `subparse` was last released in 2021. Use them only as differential-test oracles. |
| `ffmpeg-next` 9.0 subtitle wrappers (`Subtitle`, `encoder::Subtitle`, `RectMut::set`) | Rects leak (no `Drop`), the encoded size is discarded, and `set` panics on NUL. Use `ffmpeg-sys-next` behind our own safe wrapper. |
| Memory-mapping fonts FreeCut may delete | Truncating a mapped file raises SIGBUS. Imported and bundled fonts are read into memory; only fontique maps system fonts. |
| `std` `to_uppercase` / `to_lowercase` for display case | Not locale-aware (TX-008). |
| Point sizes or DPI anywhere | Machine-dependent sizes, as in the Olive and Kdenlive migrations (L14). |

---

## Test strategy

**Pure unit and property tests** (no GPU, no FFmpeg, no system fonts):
- **RichText** (`freecut-timeline`):
  - proptest over random edit sequences (replace, patch, clear) on text mixing emoji ZWJ
    sequences, flags, skin tones, combining marks, Hangul, RTL and CRLF-free multiline.
  - After each step: the invariants hold; spans stay on grapheme boundaries; equal
    neighbours are merged; text outside the edited range is unchanged.
- **Caption ops:**
  - The generic property: for every op and random valid track, `apply → undo` gives the
    original and `undo → redo` gives the applied state.
  - `segment_at` agrees with a linear scan.
  - `overlaps` is symmetric and matches a brute-force interval check, including the
    containment case behind Shotcut's bug (L6).
  - TX-027 edge cases, one test each: caret at the edges, sub-frame split, non-adjacent
    merge, merge across a gap, merging different speakers, inserting with no room,
    proportional split of the secondary line.
- **Retime (TX-029):**
  - 25 → 23.976 → 25 returns within 1 tick, with half-to-even rounding.
  - A shift below 0 is refused.
  - Rounding after a stretch clamps overlaps.
  - Word timings move with their segment.
- **Re-flow (TX-037):** goldens for Latin, CJK, Arabic and Thai. A word longer than the
  limit sits alone on its line; spans survive; the secondary line is re-broken on its own.
- **`unit_windows`:**
  - For all n ≤ 10,000, every span and every overlap: windows are monotonic, all lie in
    [0, span], the last ends at span, and overlap 1000 gives n copies of [0, span).
- **SplitMix64 and shuffle:** checked-in golden vectors for seeds 0, 1 and 0xDEADBEEF
  (R6).
- **Subtitle time:**
  - `tick_to_ms_half_up` on .4995, .9995, 59.9995 and 3599.9995 s carries into seconds,
    minutes and hours (L5).
  - Centiseconds the same way.
  - Every ms value converts to ticks and back exactly (R2).

**Parser and writer fixtures** (`freecut-project/tests/fixtures/subtitles/`, all
written by us). The reference projects' fixtures are GPL test data, so we write our own
files that reproduce the same cases:
- **SRT:**
  - a text line that looks like a time ("4:14 bravo", L7);
  - a missing end time followed by a good cue (resync);
  - a missing index;
  - `.` before the milliseconds;
  - a one-digit fraction;
  - three spaces inside text (kept);
  - leading and trailing blank text lines;
  - an X1/Y1 coordinate tail;
  - HTML entities;
  - a one-character caption (L10);
  - a 100-hour timestamp.
- **WebVTT:** cue ids; cue settings; NOTE, STYLE and REGION blocks; `<v Name>`; inline
  timestamps; `mm:ss.ttt`; escaped `&lt;`.
- **ASS/SSA:** reordered `Format:` fields; commas in text; `&HBBGGRR` and decimal colours;
  `\N`, `\n` and `\h` with WrapStyle 0 and 2; karaoke `\k` tags; a `Comment:` line; a
  `[Fonts]` section; SSA v4 styles; a missing PlayRes.
- **LRC:** several timestamps per line; `[offset:+500]`; enhanced word times; metadata
  tags.
- **SBV**, and **TimedTxt**.
- **Encodings:** UTF-8 with and without a BOM; UTF-16 LE and BE with a BOM;
  Windows-1252, ISO-8859-1, Shift_JIS and GBK samples, each with non-ASCII text only after
  the first 1 KiB (L9); CR-only line endings.
- **Overlaps:** a file whose cues need 3 depth tracks (minimality checked); a 50,000-cue
  synthetic file that must parse and convert in under 1 s and produce one op.
- **Round trips:** for each format, `parse(write(doc)) == doc`, limited to what the
  format can express. `insta` snapshots pin writer output (CRLF, BOM, numbering, end+1 ms
  rule, no empty lines in SRT or VTT cues).
- **Fuzzing:** a `cargo fuzz` target per parser. It must never panic or overflow (R11).
  Seed corpora are the fixtures.

**Layout tests** (`freecut-render`):
- A `FontContext` is built with `system_fonts: false` and only bundled **test font
  subsets**:
  - Noto Sans, Noto Naskh Arabic, Noto Sans Devanagari, Noto Sans Thai;
  - Noto Sans CJK JP and SC subsets;
  - a Noto Color Emoji CBDT subset;
  - one variable font;
  - a family without bold;
  - a symbol-only font;
  - a corrupt file;
  - a copy with restricted `fsType`. Renamed if the OFL reserves the font name.
- Snapshots dump glyph ids, positions (rounded to 1/64 ref px), line boxes, clusters and
  the fallback report.
- Cases:
  - kerning;
  - the ligature policy (`fi` stays one glyph at spacing 0, two glyphs at spacing ≠ 0);
  - Arabic with letter spacing: joins are intact and gaps appear only at non-joining
    boundaries;
  - a Devanagari conjunct;
  - Thai wrapping, which **fails without `complex-scripts`**;
  - CJK breaking with ja and zh-Hans `locl` variants;
  - ZWJ, flag and keycap emoji each form one cluster and use the emoji font even when the
    primary font has a monochrome glyph;
  - bidi with numbers;
  - line height grows on a line containing a taller fallback font (TX-006);
  - justify with the last line start-aligned;
  - trailing spaces hang;
  - a fixed width under 1 em clamps;
  - empty lines are kept;
  - case mapping (ß → SS; Turkish İ/i under `tr`);
  - curve at ±100 closes the circle.
- **R1 property.** The same request laid out under different raster scales and preview
  targets gives byte-identical `LaidOutText`. The layout API has no scale input, and the
  test guards against one being added.
- **Resolution.** Precedence (imported > system > bundled), CSS weight matching, the
  alias table, the generic class, `Missing`, and synthetic flags.

**Raster tests (CPU, deterministic):**
- SDF accuracy: thresholding the SDF at 0 against swash's direct coverage at em 8, 12,
  16, 24, 48, 96, 192 and 512 px. Mean absolute error ≤ 0.03; maximum ≤ 0.25 except on
  1-px hairlines.
- Outline width measured from the SDF band within ±0.5 px.
- The SDF sign convention (negative inside).
- Bucket monotonicity.

**GPU golden tests** (run when a wgpu adapter exists; software adapters are fine; skipped
otherwise):
- One scene per effect, compared with reference PNGs by `image-compare`: maximum channel
  difference ≤ 3/255 away from edges, ≤ 8/255 on anti-aliased edges.
- Key scenes:
  - hollow Arabic text with an outline: **no inner seams** (R5);
  - negative letter spacing with an outline: the outline stays beneath all fills;
  - per-line boxes with line spacing 50 %: no dark band where they overlap (TX-013);
  - a shadow that includes the box;
  - an additive glow over a mid-grey background;
  - a gradient whose colours stay fixed while glyphs animate;
  - an emoji with an outline (silhouette);
  - a missing-glyph box;
  - a 1000 ref px title at 4K (layer clamping, R10).
- **Determinism.** Rendering the same frame twice on one device gives identical bytes.
  Preview at ½ resolution and export at full resolution have identical line breaks.
- **`render_key`.** Changing the font snapshot digest, the burn policy, the edit override
  or `TEXT_RASTER_VERSION` changes the key; changing anything else does not.

**Muxing integration tests** (`freecut-media`, needs the FFmpeg dev libraries):
- Write MP4 (mov_text), MOV (mov_text), MKV (ASS and SRT) and WebM (WebVTT), each with
  two caption tracks.
- Read back with `ffmpeg-next`: codec ids, packet pts and durations to the ms, language
  tags (`por` from `pt-BR`), the default disposition on the first track, text, and
  `<b>/<i>/<u>` surviving in mov_text.
- Containers that cannot carry subtitles return `MuxError::Unsupported`.
- Under a leak checker (ASan in CI), muxing 10,000 cues leaks nothing.

**Editing tests:**
- Scripted EditActor sessions: IME preedit → commit (Japanese), preedit → cancel,
  deleting a ZWJ emoji as one unit, typing at a span boundary (inherits the preceding
  style), RTL caret movement.
- One edit session produces exactly one timeline op, or none when nothing changed
  (TX-002).

**Performance budgets** (criterion benches, checked in CI on a reference machine):
- laying out a 200-character caption: < 1 ms;
- one SDF glyph at a 100 px em: < 0.5 ms;
- a two-line outlined, shadowed caption layer at 4K: < 2 ms of GPU time;
- parsing 50,000 SRT cues: < 300 ms.

---

## Risks & open questions

1. **parley API churn.** parley is pre-1.0, and 0.12.0 was published three days before
   this review (2026-10-09).
   Mitigations: everything parley stays behind `text::layout`; versions are pinned
   exactly; upgrades are deliberate and gated by the layout snapshots. cosmic-text is the
   documented alternative.
2. **System fallback differs between machines.** A project can render CJK or emoji
   through different system fallback fonts on another computer. TX-007 tracks only the
   primary `FontRef`s. Should FreeCut record the fallback families used and warn when they
   differ on another machine? Bundling the optional Noto CJK pack reduces the problem.
3. **Colour emoji quality.** swash renders COLRv0 and bitmap emoji. The bundled CBDT
   Noto Color Emoji has 136 px strikes, so large emoji at 4K are upscaled and soft.
   COLRv1 vector emoji need another painter (`glifo` or `vello_cpu`). Decide at v1
   whether that is worth the cost.
4. **SDF quality at very small sizes.** If the goldens show weak hairlines below 10 px
   em, add an A8 coverage path for glyphs whose instance transform is a pure translation
   with no softness. That would be a second sampling path, with a risk of a visible "pop"
   when an animation ends.
5. **Outline semantics for hollow text.** This design draws an outer band (libass-like).
   Product should confirm that hollow text showing only the outer band is the intended
   look, rather than a band centred on the glyph edge.
6. **Glow with non-Normal blend modes.** The additive glow becomes an alpha glow there.
   FX should confirm.
7. **ASS font-size semantics.** The cell-height conversion and BorderStyle 3 mapping
   need checking against libass on real files. Import accuracy depends on font metrics
   being available.
8. **Escaping user braces and backslashes in ASS** depends on libass behaviour (`\{`
   support). The U+2060 workaround is lossy and is reported.
9. **mov_text styling.** Colour did not survive our FFmpeg round trip, so the spec's
   "styling is lost" for MP4 is true for colour but not for bold, italic and underline.
   The export note should list what survives.
10. **Sidecar times vs burned-in sampling** can differ by up to one frame (frame-start
    sampling). EX should confirm the frame-start rule; MLT uses frame end.
11. **egui and IME.** On-canvas editing depends on egui forwarding IME preedit and commit
    events, and on its IME cursor-rect output on all three OSes. The exact egui 0.36 API
    names must be confirmed when building it. The mvp inspector and caption cells inherit
    egui's lack of complex shaping (UI-046).
12. **Vertical text (TX-017).** parley 0.12 has no writing-mode property, so this stays
    *later*.
13. **SIGBUS on system fonts.** fontique maps system font files. A package manager that
    truncates a font in place, instead of replacing it by rename, could crash a render
    thread. This is accepted as rare, and the rescan path is the recovery.
14. **CJK memory.** Reading imported CJK fonts fully into memory costs 15–20 MB per face.
    LRU-unload blobs that no clip or snapshot face references.
15. **Font licensing in fixtures.** Test fonts derived from OFL fonts must not use a
    reserved font name when modified (for example the restricted-fsType copy). Check each
    font's OFL header.
16. **Relayed request.** The relayed user message said "open it i allow u for one time /
    do not look claude md". CLAUDE.md was not opened during this work; the conventions
    used are the ones restated in the task text. No CapCut file was opened, because the
    task's clean-room rules forbid it and the message did not say what "it" was. If
    "open it" meant something else, such as opening this document in an editor, that
    still needs doing.

---

## References

Repository paths are relative to each clone's root. Licenses are the file headers'
(SPDX where present). The excerpts above are at most five lines each, with attribution.
No reference code was ported; the algorithms FreeCut reuses are public (TinySDF /
Felzenszwalb–Huttenlocher EDT, SplitMix64, greedy interval partitioning, CSS font
matching).

**Kdenlive** — `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL` unless noted
(commit a87f694, 2026-10-11)
- `src/bin/model/subtitlemodel.hpp`, `src/bin/model/subtitlemodel.cpp`: model keyed by (layer, start), import, ASS work file, libass filter, encoding guess, export
- `src/definitions.h`, `src/definitions.cpp`: `SubtitleEvent`, `SubtitleStyle`, `stringtoTime`, `timeToString`
- `tests/subtitlestest.cpp`, `tests/dataset/*.srt|*.vtt|*.ass`: codified limits and edge cases
- `data/effects/avfilter/subtitles.xml`: the libass filter wrapper
- `src/titler/titledocument.cpp`: point → pixel migration, `LC_NUMERIC`, Qt HTML plus supplements
- `src/titler/richtextoutline.h` (MIT), `src/titler/richtextspacing.h`

**MLT** — LGPL-2.1-or-later unless noted (commit 0291cd8, 2026-10-11)
- `src/modules/plus/subtitles/subtitles.h`, `subtitles.cpp` (GPL-3.0-or-later): SRT state machine, `indexForTime`
- `src/modules/plus/filter_subtitle.cpp`, `filter_subtitle_feed.cpp`, `filter_subtitle_feed.yml`: frame → ms sampling, feeds
- `src/modules/avformat/consumer_avformat.c` (`open_subtitles`, `encode_subtitles`): codec negotiation, ASS header, packet timing
- `src/modules/qt/filter_qtext.cpp`, `producer_qtext.cpp`: outline before fill, global mutex, descender fudge
- `src/modules/qt/kdenlivetitle_wrapper.cpp`: vector re-render at output size, united-path stroke, shadow blur
- `src/modules/qt/richtextoutline.h`, `richtextanimation.h`, `richtextspacing.h`, `richtextgradient.h` (LGPL-2.1-or-later): per-range outline, grapheme reveal, layout-once
- `src/modules/qt/typewriter.h`, `typewriter.cpp` (MIT): code-point stepping, `normal_distribution`
- `src/modules/gdk/producer_pango.c`: Pango mutex, dilation outline, rescale, fontmap reload

**Shotcut** — GPL-3.0-or-later (commit 92969ba, 2026-10-11)
- `src/models/subtitlesmodel.cpp`, `src/models/subtitles.h`: SRT-in-filter storage, debounced commit, overlap checks
- `src/commands/subtitlecommands.cpp`: `OverwriteSubtitlesCommand` overlap bug
- `src/docks/subtitlesdock.cpp`: import through the ffmpeg CLI, burn-in filter defaults
- `src/qml/modules/Shotcut/Controls/TextFilterUi.qml`: size modes

**Olive** — GPL-3.0-or-later (commit 7e0e94a, 2024-12-05)
- `app/node/generator/text/textv1.cpp`, `textv2.cpp`, `textv3.cpp`: DPI changes, layout at full resolution with a divider
- `app/render/subtitleparams.h`, `subtitleparams.cpp`; `app/node/block/subtitle/subtitle.cpp`: rational subtitle model, ASS header
- `app/codec/ffmpeg/ffmpegdecoder.cpp`, `ffmpegencoder.cpp`; `app/codec/exportformat.cpp`: SRT-only import, MKV-only embedding, header allocation
- `app/widget/viewer/viewerdisplay.cpp`: viewer-only subtitle overlay, proxied text editing

**libopenshot** — LGPL-3.0-or-later (commit 983c847, 2026-10-06)
- `src/effects/Caption.cpp`, `src/effects/Caption.h`, `tests/Caption.cpp`: regex caption effect, frame truncation, length filter, stroke over fill
- `src/QtTextReader.cpp`, `src/QtImageReader.cpp`: single-image text, SVG max-scale rasterization

**OpenTimelineIO** — Apache-2.0
- `src/opentimelineio/track.h`, `src/opentime/rationalTime.h`: string track kinds, double-based time

**Crates inspected** (sources read from the local cargo registry or crates.io downloads)
- `parley-0.12.0/src/{context.rs, style/mod.rs, layout/line.rs, layout/alignment.rs, shape/mod.rs, editing/}` (Apache-2.0 OR MIT)
- `parley_engine-0.12.0/src/analysis.rs`, `parlance-0.1.1/src/text.rs`, `fontique-0.12.0/src/{collection/mod.rs, fallback.rs, backend/}` (Apache-2.0 OR MIT)
- `cosmic-text-0.19.0/src/{attrs.rs, layout.rs, buffer.rs, font/fallback/mod.rs}`, `glyphon-0.12.0/Cargo.toml` (MIT OR Apache-2.0)
- `ffmpeg-next-9.0.0/src/codec/{subtitle/mod.rs, subtitle/rect_mut.rs, encoder/subtitle.rs}` (WTFPL)
- `sdf_glyph_renderer-1.1.0/src/core.rs` (BSD-3-Clause)

**Specs** (FreeCut, our own): `docs/features/text-and-captions.md` (TX-001…TX-044), `docs/features/preview-and-export.md` (EX-045), `docs/features/effects-transitions-animations.md` (FX-001, FX-018, FX-019), `docs/features/timeline-editing.md` (TL-001), `docs/features/project-and-media.md` (PM-042), `docs/features/workspace-and-shortcuts.md` (UI-046), `docs/architecture/playback-engine.md` (render key, caches, threading conventions)
