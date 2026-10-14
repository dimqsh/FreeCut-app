# Handoff: captions side lanes CS1a, CS2a, CS2b (parallel, disjoint files)

These three slices prepare FreeCut's captions and local speech-to-text. Each one owns its own
files, so three agents can run them at the same time without stepping on each other or on the main
line. The main line meanwhile builds colour K4 → K5 → K6 in `freecut-timeline`, `freecut-render`,
`freecut-project` (format DTOs) and `freecut-app`. The orchestrator reviews each branch and merges it.

## 1. Read first

1. `CLAUDE.md`: Tick time, `thiserror` in libraries, no `unwrap`/`expect` outside tests unless the
   invariant is commented, no I/O on the UI thread.
2. `docs/architecture/speech-and-captions.md`: the study, decision rows C47–C51, the crate layout
   (§4) and the caption data model.
3. `docs/features/text-and-captions.md`: TX-025..TX-044 and TX-057, plus the "Captions & speech
   build — scope" section. `docs/features/ai-tools.md`: AI-001, AI-002, AI-009, AI-010, AI-012.
4. The round-3 build plan, `/private/tmp/claude-501/-Users-dimash-PycharmProjects-FreeCut-app/f0cd0716-b9da-41be-93c5-0e00e0cdbfb5/scratchpad/round3-plan.md`,
   sections §2, §3 and §5 (CS1a, CS2a, CS2b). If that path is not readable, use §4 below.

**Owner decisions, already made; do not reopen:**
- The speech engine is whisper.cpp via `whisper-rs`, with Metal on macOS.
- The **default model tier is large-v3-turbo** (q5_0, about 574 MB, fast and accurate). **base**
  (about 60 MB) is the quick option. The docs that still name small-q5_1 as the default must be
  updated to this.
- The new crate `freecut-analysis` is **approved**, including its `CLAUDE.md` lines (plan §2).
- Regenerating auto captions replaces only untouched auto captions; typed, imported or corrected
  captions are never touched.

## 2. How to run (each lane is its own isolated session)

Start each agent **with Claude Code's built-in worktree isolation**, from the repo:

```
cd /Users/dimash/PycharmProjects/FreeCut-app && claude --worktree cs1a
```

Use `cs2a` and `cs2b` for the other two. In the desktop app, create the session with the
**worktree** option on. This gives each lane its own checkout and branch, and Claude Code blocks
edits to the main checkout.

Rules for every lane:
- Start from `main`'s latest commit. It must contain "Colour K2–K3" or later; if it doesn't, stop and say so.
- Touch **only** the files your lane owns (§3). If you think you need a file outside it, stop and ask.
- Do not change `CURRENT_FORMAT_VERSION`, the golden project files, or any colour code.
- Build in your worktree's own `target/`. Iterate with `cargo test -p <crate>`. Delete any extra
  build directories you create.
- Commit on your branch with clear messages. Never push or merge; the orchestrator merges.

## 3. File ownership

| Lane | Owns | Must not touch |
|---|---|---|
| CS1a | `crates/freecut-project/src/subtitles/**` (new), one `pub mod subtitles;` line in `crates/freecut-project/src/lib.rs`, `crates/freecut-project/tests/subtitles*.rs` and `tests/fixtures/subtitles/**` (new) | everything else in `freecut-project` (format DTOs, `migrate.rs`, goldens), other crates |
| CS2a | `crates/freecut-media/src/audio/pcm.rs` or a new `pcm` module (new), at most one export line in `crates/freecut-media/src/lib.rs` or `audio.rs`, `crates/freecut-media/tests/pcm*.rs` (new), and `rubato` in `[workspace.dependencies]` if needed | the LUT module, decoders' existing behaviour, other crates |
| CS2b | `crates/freecut-analysis/**` (new), `crates/freecut-app/tests/deps_guard.rs` (add the analysis rows only), `CLAUDE.md` (the approved lines only), `docs/architecture/README.md` §1.1/§1.2 rows, `docs/architecture/speech-and-captions.md` and `docs/features/ai-tools.md` (default-tier update) | every other crate, `Cargo.toml` beyond adding pinned deps to `[workspace.dependencies]` (the workspace already globs `crates/*`) |

## 4. The slices

### CS1a: subtitle files (`freecut-project` only)
- `freecut_project::subtitles::{SubtitleDoc, Cue, Format::{Srt, Vtt, Lrc}, parse(&str, Format) -> Result<(SubtitleDoc, ParseReport), ParseError>, write(&SubtitleDoc, Format, &WriteOptions) -> String}`.
- Times are Ticks. There are no timeline caption types yet (CS1 maps `SubtitleDoc` to segments later).
- Tests:
  - a byte-order mark, CRLF line endings and missing index numbers;
  - malformed timing reported with its line number;
  - overlapping cues reported, for splitting onto extra tracks;
  - 50,000 cues parsed in under 1 s (release-only test, or `#[ignore]` with a note);
  - write → parse round trip at millisecond precision;
  - LRC with word tags;
  - VTT styling and position settings preserved as data or reported as dropped.
- Acceptance: all fixtures pass; no timeline change; `cargo test -p freecut-project` green.

### CS2a: 16 kHz PCM reader (`freecut-media` only)
- `freecut_media::audio::read_pcm_mono(media, stream, channel, 16_000, range, cancel) -> impl Iterator<Item = Result<PcmBlock, MediaError>>`:
  60 s blocks resampled with `rubato` (pin an exact version, record its licence), each with an exact
  tick origin.
- One 16 kHz sample is exactly 44,100 ticks, because 705,600,000 ticks per second ÷ 16,000 = 44,100.
- Tests:
  - a generated WAV fixture (sine and impulse) gives sample-exact output within the resampler's tolerance;
  - block origins add up exactly;
  - cancel stops between blocks;
  - mono and stereo sources with a channel choice;
  - a range that starts mid-stream.
- Acceptance: `cargo test -p freecut-media` green (skip gracefully without the ffmpeg CLI if fixtures need it).

### CS2b: `freecut-analysis` pure core (new crate)
- First apply the approved `CLAUDE.md` lines exactly as written in the plan's §2: the crate bullet,
  the dependency sentence, and the `--no-default-features` build command. Also apply the matching
  `deps_guard.rs` rows: `analysis → timeline` only, only `app` may depend on it, and
  `whisper-rs-sys` only under `freecut-analysis` with its feature.
- The crate is `#![forbid(unsafe_code)]`, with features `ml-whisper`, `ml-whisper-vulkan` and
  `ml-whisper-cuda`, none on by default. Do **not** add `whisper-rs` yet; that is CS2. Keep the
  feature names declared.
- Contracts (speech-and-captions.md §4):
  - `models/manifest.json` (compiled in) with `ModelManifest::entries_for(Task)`. Each entry has an
    HTTPS URL pinned to a revision, SHA-256, size and licence. The default tier is
    **large-v3-turbo** and the quick tier is **base**; add the Silero voice-detection file.
  - `verify::sha256_file(path, cancel)`.
  - `audio::Pcm16k`, with sample ↔ Tick conversion exactly 44,100.
  - `vad::{SpeechRegions, VoiceDetector}`, with pure merging and padding.
  - `stt::{SttRequest, SttProfile, LangChoice, SpeechRecognizer, RawWord, LangGuess, SttError}`,
    `stt::chunks::plan_chunks` (cuts only in silence, 5-minute cap), `stt::words::assemble_words`,
    `stt::clean::{drop_repetition_loops, drop_low_speech_hallucinations}`, and `stt::fake::FakeRecognizer`.
  - `fillers::lexicon::{Lexicon, parse, validate}`; the lexicon data files ship in CS3.
- Tests:
  - chunk planning;
  - word assembly: a CJK character split across tokens, the leading-space rule, Thai without spaces;
  - repetition-loop removal;
  - manifest entries;
  - the dependency-guard rules.
- Update the docs that name small-q5_1 as the default to the owner's decision (large-v3-turbo
  default, base quick), and note "turbo cannot translate: Translate to English uses a non-turbo model".
- Acceptance: `cargo test -p freecut-analysis` green with no C++ toolchain; `deps_guard` green.

## 5. Definition of done (each lane)

- In your worktree, these pass for your crate:
  - `cargo fmt --all --check`
  - `cargo clippy -p <crate> --all-targets -- -D warnings`
  - `cargo test -p <crate>`
- `cargo test --workspace` too, if it builds. If it fails in code you did not touch, report it and
  don't fix it.
- Commit, then report:
  - what is built, with the spec IDs;
  - the public API;
  - every file touched outside your new module and why;
  - new dependencies with versions and licences;
  - the tests added;
  - open questions.
