# CS2b review: freecut-analysis pure core (AI-001/002/009/012, C47–C49)

Branch `worktree-cs2b`, commit `14189ca` on main `339c149`. Lane CS2b, brief
`docs/handoff/captions-side-lanes.md`. Design: `speech-and-captions.md` §4–§5,
`ai-tools.md` AI-001/002/009/012.

## Verdict: approve with changes

There are no blockers. One major issue must be fixed before merge: word
times overlap when a character is split across tokens (CS2b-C1). Everything
else is minor or a nit.

What was checked and holds up:

- **Gate.** All green in the worktree: `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, and
  `cargo test --workspace` (1,981 passed, 0 failed, 8 ignored), which matches
  the report. `cargo test -p freecut-analysis` also passes on its own, and
  `deps_guard` passes 7 of 7.
- **Core maths.** `Pcm16k` uses exactly 44,100 ticks per sample, checks
  origin+len overflow when it is built, and rounds indices in i128. VAD
  merging, padding and clamping are correct.
- **Manifest.** HTTPS URLs pinned to 40-hex revisions. Sizes, SHA-256 and
  licences match spec §3. Tiers follow the owner's decision: the default is
  large-v3-turbo, the quick tier is base, and turbo's translation uses small.
  `sha256_file` streams in 1 MiB reads and checks cancel before each one.
- **Governance.** The CLAUDE.md edit matches round-3 plan §2 word for word,
  and leaving out the optional fourth line is fine. Every touched file is
  inside CS2b's ownership (handoff §3). The crate has
  `#![forbid(unsafe_code)]`, no FFmpeg and no networking. The one `expect` has
  its invariant commented, and errors use thiserror. The guard adds rules and
  weakens none.
- **Licences.** The sha2 family is MIT OR Apache-2.0. The icu_* crates are
  Unicode-3.0, already listed as compatible with GPL-3.0-or-later in README
  §4.3. They add nothing new to the lock file.

## Required changes

**CS2b-C1 (major): adjacent words overlap in time when a character is split
across tokens.** `crates/freecut-analysis/src/stt/words.rs:226-247`
(`CharTimes::new`). Start times are placed using `starting[token]`, the
characters whose first byte is in the token. End times use `ending[token]`,
the characters whose last byte is in the token. When a character straddles
two tokens, those counts differ and the slices no longer line up.

- **Probe.** Tokens `"我们今"`+first byte of 天 @0–40 and the rest of 天+`"去公园"`
  @40–80 give 我们 0..26, 今天 20..50, 去 40..60 and 公园 53..80. Each word
  starts before the previous one ends.
- **Why it matters.** It breaks the module doc (words.rs:15-17,
  "consecutive slices") and §5.1 step 6 ("never going backwards"). Whisper's
  byte-level BPE does this on real CJK output, and the overlap reaches caption
  packing.
- **Fix.** Count each non-space character once, in the token where it starts.
  Give it one slice `[start + len*k/n, start + len*(k+1)/n)`. A straddling
  character ends where the next character's slice starts. After assembly,
  also clamp so that `words[i].end <= words[i+1].start`; this covers
  out-of-order tokens too (line 47 keeps only starts monotonic).
- **Test.** Split a character at an ICU word boundary and assert
  non-overlap across all words.

## Minor (fix in the same round if cheap; otherwise track)

- **CS2b-GOV-1:** `stt/mod.rs:203-224`. **Recommended before CS2 builds on
  the trait.** `SpeechRecognizer` takes borrowed, non-`'static`
  `cancel: &dyn Fn() -> bool` and `progress: &mut dyn FnMut(PerMille)`.
  whisper-rs 0.16's safe callbacks need `'static` closures
  (`set_abort_callback_safe`, `set_progress_callback_safe`), and the raw
  versions are `unsafe`, which this crate forbids. **Fix:** take a cloneable
  `Send + Sync + 'static` handle, for example a `Cancel(Arc<AtomicBool>)` and
  an `Arc<dyn Fn(PerMille) + Send + Sync>`, or an atomic. Change
  `VoiceDetector::speech_probabilities` (vad.rs:232) the same way, and update
  `FakeRecognizer` and §5.6 of speech-and-captions.md.
- **CS2b-C2:** `stt/chunks.rs:90-94`. Cutting an overlong region at a fixed
  5 minutes can leave a tail of 1 tick, which becomes its own recognizer call.
  A tail under 1 s loses its speech; a longer short tail can make whisper
  invent text that the coverage filter cannot catch. **Fix:** split the region
  into `ceil(len/max)` equal pieces, or fold a remainder under about 10 s into
  the previous piece. Add a property-test case with length just over `k*max`.
- **CS2b-GOV-2:** `crates/freecut-app/tests/deps_guard.rs:204-229`.
  - **Gap:** the whisper rule reads direct dependencies only
    (`cargo metadata --no-deps`). The required "`whisper-rs-sys` is absent
    without the feature" check (plan §5 CS2b, speech-and-captions.md §8) is
    not implemented. The default-feature check is also a substring match on
    direct entries, so `default = ["gpu"]` with `gpu = ["ml-whisper"]` passes.
  - **Fix:** add a resolved-graph check that `whisper-rs-sys` is absent for
    `-p freecut-analysis` with default features. Expand `default` through the
    features map before testing for whisper features.
- **CS2b-GOV-3:** docs outside the lane still name small/Balanced as the
  default. **Merge-time follow-up for the orchestrator.**
  - **Where:** `docs/features/text-and-captions.md:1501` ("Balanced
    (default)") and the C47 row in `docs/architecture/README.md:1480`
    ("`ggml-small-q5_1` (190 MB, default)"). Both now contradict ai-tools.md
    and speech-and-captions.md on this branch.
  - **Report gap:** the report should have flagged these.
  - **Fix:** change text-and-captions.md to "Accurate (default)". Add a dated
    amendment to C47: the default is large-v3-turbo, the quick tier is base,
    and turbo cannot translate.

## Nits

- **CS2b-C5:** `stt/clean.rs:40`. One greedy pass misses a loop that appears
  only after an inner loop is collapsed (`a a a a b` x4 → `a b` x4). Repeat
  the pass until nothing changes.
- **CS2b-C6:** `models/manifest.rs:267-277`, 365. A duplicate id inside one
  `requires` list is counted twice by `with_requirements`/`install_size`.
  Reject it in `check_links` and de-duplicate while pushing.
- **CS2b-C7:** `models/manifest.rs:345`. The validator accepts a speech entry
  with `translates == false` and no `translate_with`, and then
  `speech_model_for(_, TranslateToEnglish)` silently returns None. Require
  one of the two.
- **CS2b-C8:** `stt/words.rs:128-137`. French spaced punctuation (`" ?"`)
  becomes its own timed word. Merge a piece made only of punctuation into
  the previous word, and add a French test.
- **CS2b-GOV-4:** README §4.2 (around line 955) has no `icu_normalizer 2.3.0`
  row, and the icu rows don't list the analysis crate as a user. Fix at
  merge, and add these crates to the UI-051 licence list when it exists.
- **CS2b-GOV-5:** `deps_guard.rs`. Nothing enforces the analysis crate's
  no-networking rule. Add an external allowlist, or a denylist of HTTP/TLS
  crates such as ureq, reqwest and hyper.
- **CS2b-GOV-6:** `stt/mod.rs:209`. The docs don't say what happens with
  `LangChoice::Auto` in `transcribe`. State that the job calls
  `detect_language` once and passes `Fixed(lang)` for every chunk, here and
  in §5.6.
- **CS2b-GOV-8:** `models/manifest.json:14,28,42,56`. The licence URLs point
  to `blob/main` and `blob/master`. Pin them to a commit or tag.

Rejected after verification: C3 (the chunk-start flag is a design choice for
CS2), C4 (loop removal follows the spec; exceptions for lyrics belong to CS3),
GOV-7 (`speech_model_for` + `with_requirements` already size translation
correctly).

## Message for the CS2b agent

```
CS2b review: approve with changes. The gate is green and the manifest, tiers,
Pcm16k maths, CLAUDE.md lines and ownership all check out. Please fix the
following on worktree-cs2b, then rerun fmt, clippy -D warnings and
cargo test --workspace:

1. (major) stt/words.rs:226 CharTimes::new. When a character is split
   across tokens, word times overlap: "我们今"+E5 @0-40 and the rest of 天+
   "去公园" @40-80 give 今天 20..50, 去 40..60, 公园 53..80. Count each
   character once, in the token where it starts, and give it one slice. Then
   clamp so words[i].end <= words[i+1].start, which also covers out-of-order
   tokens. Add a test that splits a character at a word boundary and asserts
   no overlap.
2. (minor, do now before CS2 depends on it) stt/mod.rs:203-224 and
   vad.rs:232. Replace the borrowed cancel and progress closures with
   'static Send+Sync handles (for example Cancel(Arc<AtomicBool>) and
   Arc<dyn Fn(PerMille)+Send+Sync>). whisper-rs 0.16's safe callbacks
   require 'static, and the crate forbids unsafe. Update FakeRecognizer and
   speech-and-captions.md §5.6.
3. (minor) stt/chunks.rs:90. Don't leave tiny tail chunks: split an overlong
   region into ceil(len/max) equal pieces. Add a proptest for lengths just
   over k*max.
4. (minor) deps_guard.rs. Add a resolved-graph check that whisper-rs-sys is
   absent from freecut-analysis with default features, and expand `default`
   through the features map before checking for whisper features.
5. (nits, optional) Rerun loop cleanup until nothing changes (clean.rs:40).
   Reject duplicate `requires` ids (manifest.rs:267/365). Require translates
   or translate_with on speech entries (manifest.rs:345). Attach spaced
   punctuation to the previous word (words.rs:129). Document that Auto is
   resolved once by detect_language (stt/mod.rs:209).

Merge-time follow-ups outside your lane, which the orchestrator will handle:
text-and-captions.md:1501 and README C47 still name small as the default,
and README §4.2 needs an icu_normalizer row.
```
