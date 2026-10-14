# CS1a review: subtitle read/write (SRT, WebVTT, LRC)

Branch `worktree-cs1a`, commit `38d0003` on main `339c149`. Lane CS1a, brief
`docs/handoff/captions-side-lanes.md`.

## Verdict: changes required

The module is solid. Tick maths, overflow checks, half-up rounding with carry,
BOM and line-ending handling, WebVTT header rules and settings validation, and
the minimum track count in `track_depths` were all checked and are correct.
The gate is green: fmt, clippy `-D warnings`, and `cargo test --workspace`
with 1,958 passed, 0 failed and 8 ignored. Ownership matches §3: outside the
new files, only `pub mod subtitles;` in `lib.rs` changed. No new
dependencies, no floats, no `unwrap`/`expect`, and errors use `thiserror`.

Two problems block the merge. A hostile file that fits inside the 64 MB
TX-031 import limit can hang the import job, which has no way to cancel, or
abort the whole editor.

## Required changes (majors)

**CR-1. One long line makes markup reading quadratic.**
Locations: `crates/freecut-project/src/subtitles/markup.rs:224` (`tag_end`),
`:232` (`override_block_end`), `lrc.rs:241` (`word_tags`), and
`Reader::style` (markup.rs ~127).
- Each `<` or `{\` rescans to the end of the line when nothing closes it.
  `Reader::style` walks the whole `<font>` stack on every tag.
- Measured in release on one SRT cue line of `<a` repeated: 349 ms at 40 KB,
  1.38 s at 80 KB, 5.1 s at 160 KB (4x per doubling). A 2 MB line would take
  about 13 minutes.
- Fix: keep the scans linear. Cache the position of the next `>`/`}`/newline
  and reuse it until the cursor passes it. Or, once a line is known to have
  no closing `>`, treat every later `<` on it as literal. Keep the current
  colour, or a count of open coloured fonts, instead of walking the stack.

**CR-2. LRC stamps multiply memory.**
Location: `crates/freecut-project/src/subtitles/lrc.rs:76`.
- `text.clone()` runs once per leading stamp, and nothing caps it. One
  70 KB line produced 100 MB of cue text. Within 64 MB of input this reaches
  terabytes, so allocation fails and the process aborts instead of returning
  `ParseError`.
- Fix: cap stamps per line and/or the total bytes of cue text produced, and
  report what goes over the cap as an `Issue`. Optionally share the text, for
  example with `Arc<str>`, until the output is built.

**CR-8. Add tests that cover CR-1 and CR-2.**
Location: `tests/subtitles_perf.rs`. Add adversarial cases with time and
output-size bounds:
- one multi-MB line each of `<a`, `{\` and nested `<font>`;
- an LRC line with 100k stamps.

## Minor items (fix in this lane if cheap, otherwise note them as open questions)

- **CR-4**, `markup.rs:471`: the SRT writer does not escape a cue-text line
  that is itself a valid timing line, so writing and reading back splits the
  cue in two. Fix: write the `>` of `-->` as `&gt;`. Also add full timing
  lines and index-plus-timing pairs to the TRICKY pieces in
  `tests/subtitles_roundtrip.rs`.
- **CR-5 / CONV-1**, `srt.rs:173` `push_speaker_prefix`: the speaker name is
  written without escaping, so `<i>` or `&amp;` in a name reads back as
  markup. Fix: pass the name through `markup::escape(Dialect::Srt)`. For LRC,
  break up `<digit` and a leading `[digit`. Add a round-trip case.
- **CR-6**, `lrc.rs:196` `leading_tag`: any `[word: …]` is treated as
  metadata, so `[Chorus: Ann] la la` is dropped without a report. Fix: treat
  a line as metadata only for known keys (ar, ti, al, au, by, re, ve, length,
  offset, #) or when nothing follows `]`. Otherwise handle it as an untimed
  line, and warn about unknown metadata.
- **CR-3**, `time.rs:45`: the LRC stamp `[mm:ss:xx]` is read as h:m:s with no
  warning, so every cue lands minutes late. Fix: in the LRC path, read
  `a:b:cc` (two-digit `cc`, `a < 60`) as mm:ss:cc and report a warning. Add a
  fixture.
- **CONV-5**, docs: `docs/architecture/text-and-subtitles.md:1058-1082` and
  the TX-031/TX-032 model notes still describe the old API. The orchestrator
  (or whichever lane owns the docs) should update them at merge time. CS1a's
  report should also include the §5 open-questions list it is missing.

## Nits (optional)

- **CR-7**, `lrc.rs:295`: LRC text that looks like a stamp does not survive a
  round trip. This is a limit of the format. Document it, or warn when
  writing.
- **CR-9**, `mod.rs:195`: in the `Overlap` / `ParseReport::overlaps` docs,
  say "cues placed on an extra track (depth > 0)". A cue can overlap an
  earlier cue and still land on track 0.
- **CONV-2**, `mod.rs:156`: rename `Cue.line` (and `Overlap.line`) to
  `source_line`, as in the architecture doc, so it is not confused with
  `cue.settings.line`. Consider `Option<u32>` instead of 0 for "no line".
  This is cheap to do before CS1 starts using the API.
- **CONV-4**, `time.rs:14,17`: derive `TICKS_PER_MS` and `TICKS_PER_CS` from
  `TICKS_PER_SECOND` and add a `const` assert.

## Message for the CS1a agent

> CS1a review: changes required. The module is solid and the gate is green.
> There are two DoS issues to fix before merge:
> (1) CR-1: markup scanning is quadratic on one long line. This affects
> markup.rs:224 `tag_end`, :232 `override_block_end`, lrc.rs:241
> `word_tags`, and the `<font>` stack walk in `Reader::style`. A 160 KB line
> of `<a` takes 5 s, and it grows 4x per doubling. Make the scans linear by
> caching the next `>`/`}`/newline or marking the rest of the line literal,
> and keep the current colour instead of walking the stack.
> (2) CR-2: lrc.rs:76 clones the text once per stamp with no cap, so memory
> can blow up and abort the process. Cap stamps per line and/or the total
> output bytes, and report the excess as an Issue.
> Add adversarial perf tests for both: multi-MB lines of `<a`, `{\` and
> nested `<font>`, and an LRC line with 100k stamps, with time and size
> bounds.
> Fix these too if cheap: CR-4 (escape `-->` as `--&gt;` in SRT text and add
> timing-line pieces to the round-trip strategy), CR-5 (escape the speaker
> prefix in srt.rs:173), CR-6 (only known LRC keys or a bare `[key:…]` count
> as metadata, and never drop text silently), CR-3 (read LRC `[mm:ss:xx]` as
> centiseconds, with a warning).
> Nits: rename `Cue.line` to `source_line`, derive `TICKS_PER_MS`/`CS` from
> `TICKS_PER_SECOND`, and reword the `overlaps` doc.
> Add an open-questions section to your report (brief §5). Full review:
> docs/handoff/cs1a-review.md.
