# CS1a review, round 2: subtitle read/write (SRT, WebVTT, LRC)

Branch `worktree-cs1a`, HEAD `d0933ff` on main `339c149`. Round-1 commit
`38d0003`, round-1 review `docs/handoff/cs1a-review.md`. Lane CS1a, brief
`docs/handoff/captions-side-lanes.md` §3-§5.

## Verdict: approve with changes

All seven round-1 items and the nits are fixed, one commit each. The
fixes were checked by reading the code, by release and debug probes
against the built library, and by mutation runs that put each CR-1 fix
back. Every parse and write path is now linear: 2 MB hostile lines take
4-13 ms in release. A differential fuzz of 900k parses found no change in
output from the CR-1 caching.

One new major blocks the merge: `&#13;` decodes to a carriage return.
That breaks the `CueText` invariant, panics debug builds and loses cue
text in release. The fix is one line plus tests. Two cheap minors should
go in the same pass. After that the orchestrator can merge without
another full round.

## Round-1 items

| Item | Status | Commit | Notes |
|---|---|---|---|
| CR-1 quadratic scans | Fixed | `94b6b28` | `NextOf` cache, O(1) font colour, 32-byte LRC stamp cap, linear `CueText::keep`. Linear at 1-8 MB in every probe. Mutants with each fix removed go 4x per doubling. |
| CR-2 LRC memory blow-up | Fixed | `1444eb6` | 256 stamps per line, 16 MiB repeat budget, one `RepeatsDropped` per line, last copy moved. No bypass found. Repeats now add at most 1.4x the cues of a plain file. See R2-4 for the general cue-count cost. |
| CR-3 `[mm:ss:xx]` | Fixed, with a deviation | `b6a54c8` | `a:b:cc` reads as centiseconds with one `ColonCentiseconds` warning per line. Overflow is reported, never wraps. Deviation accepted below. |
| CR-4 `-->` in SRT text | Fixed | `e20091f` | Works even when a style span splits the arrow. The round-trip generator now makes timing and index lines. The pre-fix output splits the cue, so the property test would catch a revert. 25,600 random cases: 0 failures. |
| CR-5 speaker prefix | Fixed | `17433a7` | SRT escapes the prefix. LRC adds U+2060 after `[` or `<` before a digit. Note: the joiner stays in the imported text. That is a fair choice, but the module docs don't say so. |
| CR-6 LRC metadata | Fixed | `0646536` | Only the ten known keys are metadata. Keys match without case. `[Chorus: Ann] la la` is kept. No text is dropped without a report. |
| CR-7 LRC text limit | Documented | `d0933ff` | Module docs, mod.rs:77-80. See R2-5 for a cheap way to remove the limit. |
| CR-8 adversarial tests | Fixed | `94b6b28`, `1444eb6` | Every case asked for is in `tests/subtitles_perf.rs`, with time and size bounds. See R2-3: the time bound only fires after the call returns. |
| NITS | Fixed | `d0933ff` | CONV-2 `source_line: Option<u32>` (`None` from `Cue::new`); CONV-4 derived tick units with a `const` assert, same values as before; CR-9 overlaps docs. CONV-5 (architecture docs) is still the orchestrator's job at merge. |

## CR-3 deviation: accepted

The review asked to read `a:b:cc` as minutes only when `a < 60`. The agent
applies it to any `a`. That is the better rule. LRC minutes often run past
59, FreeCut's own writer writes them that way, and `parse_stamp` already
accepts them. With the `a < 60` guard, `[75:00:00]` would land at 75
hours, which is never right for lyrics. The guard also never removed the
ambiguity for small `a`. The warning is what tells the user, and it fires
in both cases. A stamp with a fraction, such as `[01:00:00.00]`, still
reads as hours. Keep the code as it is. The fixture already pins
`[75:00:00]`.

## Remaining issues

### Required before merge (major)

**R2-1. `&#13;` puts a carriage return into cue text.**
Location: `crates/freecut-project/src/subtitles/markup.rs:304`
(`entity_at`).
- `entity_at` rejects only `value == 0`. So `&#13;`, `&#xD;`, `&#xd;` and
  `&#013;` decode to `'\r'`. `Reader::char` pushes it into the text.
- Debug builds (`cargo test`, dev runs) panic at the
  `debug_assert!(!text.contains('\r'))` in `CueText::from_sorted`
  (text.rs:130). Input: `1\n00:00:01,000 --> 00:00:02,000\na&#13;b\n` as
  SRT, or the WebVTT version. This breaks "parsers never panic" (R11).
- Release builds keep the `\r`. On read-back `split_lines` treats it as a
  line end, so a WebVTT cue of `&#13;12` is written and then vanishes.
  `<v A&#13;B>` gives the speaker `A\rB` through `decode_entities`.
- `&#13;` is a real input: XML-based subtitle formats use it for CR, and
  converted files carry it into SRT and WebVTT. `subtitle_soup` cannot
  build it, which is why the property test missed it.
- Fix: reject `0x0D` next to the `value == 0` check, so it stays literal
  text (the SRT writer already escapes it as `&amp;#13;`). This covers
  both `Reader::char` and `decode_entities`. Add `&#13;` and `&#xD;` to
  `subtitle_soup` in `tests/subtitles_roundtrip.rs`. Add a unit test that
  parse leaves no `\r` in text or speaker, for SRT and WebVTT.

### Fix in the same pass (minor)

**R2-2. A WebVTT speaker name ending in `--` loses the cue text.**
Location: `crates/freecut-project/src/subtitles/markup.rs:538`
(`push_voice`).
- The speaker `Ann--` is written as `<v Ann-->hello`. That text line
  contains `-->`, so the reader ends the cue there. The cue comes back
  empty with no speaker, and the line is skipped as `MalformedTiming`.
  Browsers also end the cue at that line. The name `--` fails the same way.
- The reader trims the voice name and the writer does not, so ` Ann ` and
  `\u{A0}Ann` are not a fixed point.
- The VTT speaker strategy in `tests/subtitles_roundtrip.rs` has no `-`,
  so the property test can't find this.
- Fix: write `name.trim()`. When the name ends with `-`, write a space
  before the closing `>` (the reader trims it), or write `-` as `&#45;`.
  Allow `-` at the end of names in the VTT speaker strategy and add a case
  for `Ann--`.

**R2-3. A CR-1 regression would hang the test run, not fail it.**
Location: `crates/freecut-project/tests/subtitles_perf.rs:114`
(`one_long_cue`), also :173, :196 and :225.
- Each test checks `elapsed < budget()` only after `parse` or `write`
  returns. `freecut-project` is opt-level 0 in `profile.dev`. With the
  `NextOf` cache removed, one 2 MB sub-case takes about 4.7 hours
  unoptimized, and `unclosed_tags_on_one_long_line` has three. So
  `cargo test --workspace` would stall for hours instead of failing in
  20 s. The module doc at lines 7-8 says the debug bound "still catches a
  parser that went quadratic", which is not true for this case.
- Only the `<font face=x>` sub-case guards the font-stack fix. The
  `color=red` sub-case can't, because the old walk stopped at the first
  colour. That is fine, but worth a comment.
- Fix: run the call on a `std::thread::spawn` worker, send the result over
  an `mpsc` channel, and wait with `recv_timeout(budget())`. Or add a
  doubling check at small sizes, for example time(512 KB) < 4 × time(256
  KB) + slack. Either fails within seconds.

### Record as an open question (minor, not for this lane)

**R2-4. Hostile files can still need gigabytes and millions of tracks.**
Locations: `crates/freecut-project/src/subtitles/lrc.rs:87` (`cost`),
`vtt.rs:71` (`SettingDropped`), `mod.rs:214` (`tracks_needed`).
- Every path is linear, but nothing caps the number of cues or issues.
  Measured peak memory per input byte on degenerate files: 26-45x for
  tiny SRT, WebVTT and LRC cues, 176x for 1-byte untimed LRC lines. At
  the 64 MB TX-031 limit that is about 2.3-2.8 GB, and about 11 GB for the
  untimed LRC case.
- The CR-2 `cost` counts text and 16 B per word stamp. It leaves out the
  ~200 B `Cue` and ~64 B `Pending` per copy, so the 16 MiB budget never
  binds for short lines. Charging that overhead would barely change the
  worst case, because plain lines already cost about 42x.
- WebVTT pushes one `SettingDropped` issue per bad setting token, with no
  limit per cue.
- Cues that all start at once each get their own track:
  `tracks_needed()` can reach about 13.4M for one LRC file, or 6.7M for
  `0:0-->0:1` lines.
- Fix: the real fix belongs to TX-031 and CS1: a per-file cue cap and a cap
  on extra caption tracks before CS1 creates them. Optional here: charge
  `size_of::<Cue>()` in `cost`, and fold `SettingDropped` into one issue per
  cue with a count. At minimum, list this in the report's open questions.

### Nits (optional)

- **R2-5**, `lrc.rs:410` `push_text`: cue text still goes out raw, so
  `[1:00]x` reads back as two cues and `<00:01.00>` as text makes the cue
  disappear. This is the documented CR-7 limit, but the U+2060 guard
  already exists. Apply it to the cue text too, keeping word-stamp offsets
  in line, and update the CR-7 note.
- **R2-6**, `markup.rs:180`: WebVTT reads `<s>` as strike and does not
  count it as stripped. The docs say WebVTT keeps only `<b> <i> <u>`. The
  VTT writer drops strike and colour but does not merge the spans that
  then look the same, so a second write differs. Fix: `"s" if !vtt`, and
  merge equal spans in the VTT writer once strike and colour are masked.
- **R2-7**, `lrc.rs:374`: LRC with speakers and word times reads the
  prefix back as a zero-length word `Ann:`, and a second write adds a
  redundant stamp. No text or timing is lost. Add a doc note, or write the
  first word stamp before the prefix.
- **R2-8**, `tests/subtitles_lrc.rs:39`: the CR-3 edge cases all behave
  correctly but nothing pins them. Add a table test for `[00:75:00]`
  (Malformed), `[00:01:99]` (1.99 s), `[00:01:5]` (65 s as h:m:s),
  `[00:01:500]` (Malformed), `[217861211:00:00]` (TimeOverflow) and
  `[217861206:27:99]` (the `checked_add` branch).
- **R2-9**, `lrc.rs:329` `MAX_STAMP_LEN`: a zero-padded word stamp longer
  than 32 bytes is now text, while WebVTT has no such cap. FreeCut writes at
  most 15 bytes, so round trips are safe. Say it in the LRC module docs.
- **R2-10**, `markup.rs:244` `NextOf`: the cache is only correct if every
  call passes the same slice, and the docs don't say so. Both callers are
  fine today. Store `&'a [u8]` in the struct, or document it.
- **R2-11**, report: the round-2 report still has no open-questions
  section (CONV-5, brief §5). Its points are there inline, but one is
  missing: the public API (`source_line: Option<u32>`,
  `LRC_MAX_LINE_STAMPS`, `LRC_MAX_REPEAT_BYTES`, `RepeatsDropped`,
  `ColonCentiseconds`, `TextAfterTag`, `UnknownTag`) now differs from the
  API section of `docs/architecture/text-and-subtitles.md`.
- **R2-12**, orchestrator gate prompt (not the branch): `cargo test -p
  freecut-project --release subtitles_perf -- --include-ignored` runs 0
  tests. The filter matches test names, and the crate has no `#[ignore]`
  tests. Use `cargo test -p freecut-project --release --test
  subtitles_perf -- --nocapture` and check that it reports `10 passed`.
- **R2-13**, resolved: a review lane's temporary probe
  `tests/zz_review_probe.rs` was untracked in the worktree for a while. Its
  lane deleted it with its build artifacts. `git status --short` is now
  empty. No action needed. Never `git add -A` without checking.

## Gate results

Run in `.claude/worktrees/cs1a` at `d0933ff`. Working tree clean.

- `cargo fmt --all --check`: clean.
- `cargo clippy -p freecut-project --all-targets -- -D warnings`: clean.
- `cargo test -p freecut-project` (debug): 353 passed, 0 failed,
  0 ignored. Subtitle binaries: srt 11, vtt 5, lrc 9, roundtrip 7,
  perf 10.
- `cargo test -p freecut-project --release`: 353 passed, 0 failed.
- `cargo test -p freecut-project --release --test subtitles_perf`:
  10 passed. 50,000 cues: SRT 28 ms, WebVTT 34 ms, LRC 21 ms. 2 MB lines:
  3.8-12.8 ms. LRC with 100,000 stamps: 3.9 ms. Byte budget: 276 cues,
  17 MB, 2.7 ms. 100,000-line write: 3.1 ms.
- Round-trip property tests run 100 times each (25,600 cases per test):
  0 failures.
- `cargo test --workspace` was not re-run, because the crate gate is green.
  The agent reports 1,970 passed, 0 failed, 8 ignored (not verified here).
- Scope: 22 files changed since `339c149`, 13 since `38d0003`, all inside
  the §3 list. `lib.rs` has only `pub mod subtitles;`. No Cargo.toml or
  Cargo.lock changes, so no new dependencies. No floats, no `unsafe`,
  `unwrap` only in test modules, and errors use `thiserror`. The one
  `unwrap_or_default()` (mod.rs:399) has its invariant commented.
- Main is still at `339c149`, the merge base.

## Message for the agent

> CS1a round 2: approve with changes. All seven review items and the nits
> are fixed. The CR-3 deviation (any `a` counts as minutes) is accepted. Keep
> it as it is.
> One new major must be fixed before merge:
> (1) R2-1: markup.rs:304 `entity_at` decodes `&#13;` / `&#xD;` to `\r`.
> Debug builds panic at the `debug_assert` in `CueText::from_sorted`
> (text.rs:130). Release builds lose cue text on round trip. Reject `0x0D`
> next to the `value == 0` check. Add `&#13;` and `&#xD;` to
> `subtitle_soup`, and add a unit test that parse leaves no `\r` in text or
> speaker for SRT and WebVTT.
> Fix these two in the same pass:
> (2) R2-2: markup.rs:538 `push_voice` writes `<v Ann-->`, which ends the
> WebVTT cue. Write `name.trim()`, and put a space before `>` when the name
> ends with `-` (or write `-` as `&#45;`). Allow a trailing `-` in the VTT
> speaker strategy and add a case for `Ann--`.
> (3) R2-3: tests/subtitles_perf.rs only checks the budget after the call
> returns, so a quadratic regression hangs for hours in debug. Run each
> call on a worker thread and wait with `recv_timeout(budget())`, or add a
> doubling-ratio check. Fix the module doc at lines 7-8.
> Optional nits if cheap: R2-6 (`"s" if !vtt` and merge equal spans in the
> VTT writer), R2-8 (CR-3 edge-case table test), R2-9 (`MAX_STAMP_LEN` doc
> line), R2-10 (`NextOf` same-slice doc), R2-7 (doc note for LRC speakers
> with word times), R2-5 (U+2060 guard for cue text).
> Add an open-questions section to your report (brief §5). List R2-4: peak
> memory on hostile files is 26-176x the input, and `tracks_needed` can
> reach millions, so TX-031 or CS1 needs a cue cap and a track cap. Also
> list the CR-7 LRC limit, the U+2060 joiner kept in imported text, and the
> API names that differ from docs/architecture/text-and-subtitles.md.
> One commit per fix on worktree-cs1a. Run fmt, clippy and
> `cargo test -p freecut-project` before reporting. Do not push, and do not
> merge. The orchestrator merges.
> Full review: docs/handoff/cs1a-review-round2.md.
