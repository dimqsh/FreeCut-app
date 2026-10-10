# Speech-to-text and captions

> Status: draft · 2026-10-13 · Scope: the Captions drawer, caption tracks, subtitle files,
> burn-in, local speech recognition, model downloads, filler words, lyrics and bilingual
> captions. Decisions are recorded in [README §6](README.md#6-conflicts-between-the-studies-and-how-they-are-resolved)
> rows C47–C51; this page is the research and the build plan behind them. Feature IDs refer to
> [text-and-captions.md](../features/text-and-captions.md) (TX) and
> [ai-tools.md](../features/ai-tools.md) (AI).

## 1. What the user asked for, mapped to the specs

The request names three groups of caption features. Everything below runs on the user's
machine and nothing is gated: the reference editor puts a subscription in front of its
second-language line and its filler-word marking; FreeCut does not.

| Request | Spec entries | Slice |
|---|---|---|
| A Captions place in the asset panel with ways to start | TX-057 (new: Captions drawer), UI-004's Captions tab | CS1 (manual and file rows), CS2 (speech row) |
| Manually added captions | TX-025, TX-026, TX-027, TX-028, TX-037 | CS1 |
| Subtitle files in and out, captions in the exported video | TX-031, TX-032 (SRT and WebVTT first), TX-033 (burn-in and sidecar first) | CS1 |
| Automatic captions from speech, with a speech-language picker and a Generate button | AI-009, AI-010, TX-038, TX-039 (stored word timings) | CS2 |
| Remove earlier captions before generating | AI-010 output mode, TX-038 replace rule | CS2 |
| Mark filler words | AI-010 *Mark filler words*, AI-012 lexicons, TX-038 | CS3 |
| Lyrics for songs | TX-044, AI-010 *Recognize song lyrics* and *Align known text*, TX-031 LRC | CS3 |
| Caption style templates | TX-019/TX-020 styles applied to caption tracks (TX-028), TX-051 caption looks, TX-039 active word | CS3 |
| A second-language line | TX-036, TX-041, AI-013 | CS4 |

The slices are called **CS1–CS4** here so they are not confused with the C-numbered decision
rows of the README. They are the task's C1–C4.

## 2. Choosing the local speech recognizer

### 2.1 Requirements

- **Licences:** code, Rust binding and model weights must all be GPL-3.0-or-later compatible
  and allow commercial use. Non-commercial weights are out (ai-tools.md *Clean-room notes*).
- **Build:** the same `cargo build --workspace` on macOS, Windows and Linux, with no network
  access during the build and no prebuilt binaries fetched by a build script.
- **Speed:** a GPU path on Apple silicon (Metal) and a usable CPU path everywhere else.
- **Word-level timestamps** for the active-word highlight (TX-039), filler marking, transcript
  editing (AI-011) and lyric alignment (AI-010).
- **Language detection** and about 100 languages, so one model pack serves every user.
- **Voice-activity detection** in front of the recognizer, so music, room tone and silence do
  not turn into invented text.
- **Safety rules:** no `unsafe` in FreeCut's own crates outside media and render
  ([CLAUDE.md](../../CLAUDE.md)); FFI stays inside the binding crate.

### 2.2 Candidates

Checked on 2026-10-13 with `cargo info` (crate metadata from crates.io) and by reading the
downloaded crate sources in the local cargo registry; model files and their licences were read
from the Hugging Face API (see §11).

| | whisper.cpp through `whisper-rs` | sherpa-onnx (`sherpa-onnx` crate, or `sherpa-rs`) | Whisper in `candle` | Whisper ONNX through `ort` | Vosk (`vosk` crate) |
|---|---|---|---|---|---|
| Code licence | whisper.cpp MIT; `whisper-rs` 0.16.0 and `whisper-rs-sys` 0.15.0 **Unlicense** | sherpa-onnx Apache-2.0 (official crate 1.13.8); `sherpa-rs` 0.6.8 MIT | MIT OR Apache-2.0 (`candle-transformers` 0.11.0) | `ort` 2.0.0-rc.14 MIT OR Apache-2.0; ONNX Runtime MIT | `vosk` 0.3.1 MIT; Vosk/Kaldi Apache-2.0 |
| Weights | OpenAI Whisper, MIT; ggml conversions in the `ggerganov/whisper.cpp` model repo, MIT | per model family: Whisper exports MIT; Zipformer models mostly Apache-2.0; NVIDIA Parakeet CC-BY-4.0; SenseVoice under its own model licence (needs review) | Whisper safetensors (MIT; some Hub cards say Apache-2.0, both fine) | Whisper ONNX exports (MIT) | per-language models, mixed licences |
| Build on macOS, Windows, Linux | compiles the bundled whisper.cpp 1.8.3 sources with CMake and a C/C++ compiler; bindgen needs libclang unless `WHISPER_DONT_GENERATE_BINDINGS` is set (the crate then uses its bundled bindings). No downloads during the build | the build script **downloads prebuilt archives from GitHub releases** unless `SHERPA_ONNX_LIB_DIR` points at a local build; brings ONNX Runtime (tens of MB per platform) | pure Rust; heavy compile; Metal and CUDA through candle features | `ort` downloads ONNX Runtime binaries by default (`download-binaries`); a pinned local build is possible | needs the prebuilt `libvosk` shared library next to the app |
| GPU | Metal (feature `metal`), Vulkan (`vulkan`, needs the Vulkan SDK at build time), CUDA, HIP, SYCL; CPU with NEON/AVX | CPU first; CoreML, DirectML and CUDA execution providers | Metal, CUDA | CoreML, DirectML, CUDA execution providers | CPU only |
| Word timestamps | token timestamps, plus DTW alignment with presets for every Whisper size up to large-v3-turbo | token and segment timestamps (model dependent; transducer and CTC models are precise) | not built in: FreeCut would write the DTW alignment | not built in | yes, per word |
| Language detection | built in (Whisper), 99 languages | Whisper models: yes; SenseVoice: a few languages; a separate language-ID model exists | built in (Whisper) | built in (Whisper) | no; one model per language |
| VAD | built in since 1.7.x: Silero VAD converted to ggml (`ggml-org/whisper-vad`, MIT, 0.9 MB) | Silero VAD, built in | none | none | none |
| Cancel and progress | abort and progress callbacks with safe closures (`set_abort_callback_safe`, `set_progress_callback_safe`) | partial | own loop | own loop | own loop |
| Extras | — | speaker diarization, punctuation, spoken-language ID, keyword spotting, TTS in one toolkit | — | shares the runtime AI-002 plans for other models | streaming, small models |

Model families inside sherpa-onnx were also considered on their own: NVIDIA Parakeet (fast,
English plus a set of European languages, CC-BY-4.0 attribution), Moonshine (English-first)
and SenseVoice (few languages, licence to review). None covers ~100 languages with language
detection the way Whisper does.

### 2.3 Decision (README C47)

**whisper.cpp through `whisper-rs`, pinned exactly (`=0.16.0`, `whisper-rs-sys =0.15.0`,
which bundles whisper.cpp 1.8.3), inside `freecut-analysis` behind the cargo feature
`ml-whisper`.** Reasons:

1. Everything is permissive: MIT runtime, Unlicense binding, MIT weights, MIT VAD.
2. It builds from sources that ship inside the crate, so the build is offline and
   reproducible; the only new build tools are CMake and a C/C++ compiler (FFmpeg already needs
   native tooling). The workspace sets `WHISPER_DONT_GENERATE_BINDINGS=1` in
   `.cargo/config.toml` `[env]`, so no libclang is needed; one CI job builds without it and
   fails if the generated bindings differ from the bundled ones.
3. Metal on Apple silicon is a first-class backend, and the CPU path is the most tuned of the
   candidates. Vulkan covers Windows and Linux GPUs when the release build can provide the
   Vulkan SDK (open question in §11).
4. Word timestamps (token timestamps, DTW alignment heads), language detection, VAD,
   cancellation and progress all exist with safe Rust APIs, so `freecut-analysis` needs no
   `unsafe`.
5. It is what AI-002, AI-009 and TX-044's lyric alignment already assumed, and what other open
   editors (Kdenlive, Shotcut) ship.

sherpa-onnx stays the **planned second engine** for things Whisper does not do: speaker
diarization (TX-042, still out of scope until a model's licence is checked) and fast
English-only recognition. It would come in through `ort` once AI-002's ONNX runtime lands,
with ONNX Runtime built or vendored by FreeCut's release pipeline, never downloaded by a build
script. `candle` is the fallback if whisper.cpp ever stops building on a platform; Vosk is not
adopted (per-language models, no detection, weaker accuracy).

## 3. Speech model packs

Model files come from the upstream project's own model host, pinned to a repository revision,
so the bytes behind a manifest entry can never change. Sizes and SHA-256 values were read from
the Hugging Face API on 2026-10-13 (the LFS object id is the file's SHA-256). The implementation
re-checks them when the manifest is written.

| Pack (tier) | File | Bytes (shown as) | SHA-256 | Licence |
|---|---|---|---|---|
| Fast | `ggml-base-q5_1.bin` | 59,707,625 (60 MB) | `422f1ae452ade6f30a004d7e5c6a43195e4433bc370bf23fac9cc591f01a8898` | MIT |
| **Balanced** (default) | `ggml-small-q5_1.bin` | 190,085,487 (190 MB) | `ae85e4a935d7a567bd102fe55afc16bb595bdb618e11b2fc7591bc08120411bb` | MIT |
| Accurate | `ggml-large-v3-turbo-q5_0.bin` | 574,041,195 (574 MB) | `394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2` | MIT |
| Voice activity (installed with any tier) | `ggml-silero-v6.2.0.bin` | 885,098 (0.9 MB) | `2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987` | MIT |

- Whisper files: `https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/<file>`
  (model card licence: MIT).
- VAD file: `https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/<file>`
  (MIT). If the pinned whisper.cpp release does not load the v6.2.0 file, the manifest uses
  `ggml-silero-v5.1.2.bin` (885,098 bytes, SHA-256
  `29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf`) from the same revision.
- Multilingual models only: the English-only `.en` files would save little and would need a
  second language path.
- The large-v3-turbo model was trained for transcription, not translation. When *Translate to
  English* (AI-009) or the English second line (CS4) is asked for with the Accurate tier, the
  job uses the Balanced model for that pass and the dialog says so.
- CoreML encoder files (`*-encoder.mlmodelc.zip`, up to 1.2 GB) are not offered: Metal is
  enough, and they would double the download.

## 4. The `freecut-analysis` crate (README C48)

### 4.1 Rules

- **New workspace member** `crates/freecut-analysis`, the seventh crate. It owns every
  analysis algorithm and all local inference (AI-001). For this page: voice activity, speech
  recognition, transcript assembly, caption drafting, filler lexicons, lyric alignment, the
  model manifest and file verification, and the pure translation helpers (AI-013).
- **Dependencies:** `freecut-timeline` only (time, `LangTag`, `CaptionDraft`, `Transcript`,
  `CaptionRules` and `reflow`). It does not depend on `freecut-media`: the app decodes audio
  with media and hands the analysis crate plain `f32` buffers. Only `freecut-app` depends on
  `freecut-analysis`. The README §1.2 edge table gains `freecut-analysis | timeline`, and the
  dependency guard forbids every other internal edge.
- **No I/O except model loading.** The manifest and lexicons are compiled in
  (`include_str!`); verifying a file's SHA-256 reads a path the app gives it; the recognizer
  opens the model file. No networking: downloads are the app's (AI-002).
- **No `unsafe`** (`#![forbid(unsafe_code)]`); FFI stays in `whisper-rs`.
- Errors with `thiserror` (`SttError`, `ManifestError`, `LexiconError`); no `unwrap` outside
  tests.

### 4.2 Cargo features

```toml
# crates/freecut-analysis/Cargo.toml (sketch)
[features]
default = []
ml-whisper = ["dep:whisper-rs"]          # the recognizer; needs CMake + a C/C++ compiler
ml-whisper-vulkan = ["ml-whisper", "whisper-rs/vulkan"]   # release builds on Windows/Linux, if the SDK is available
ml-whisper-cuda = ["ml-whisper", "whisper-rs/cuda"]       # never in official builds

[dependencies]
freecut-timeline = { workspace = true }
serde = { workspace = true }             # manifest and lexicon data
serde_json = { workspace = true }
sha2 = "0.11.1"                          # MIT OR Apache-2.0, file verification
thiserror = { workspace = true }
tracing = { workspace = true }

[target.'cfg(target_os = "macos")'.dependencies]
whisper-rs = { version = "=0.16.0", optional = true, features = ["metal", "tracing_backend"] }
[target.'cfg(not(target_os = "macos"))'.dependencies]
whisper-rs = { version = "=0.16.0", optional = true, features = ["tracing_backend"] }
```

- `freecut-app` has a feature **`stt`** = `["freecut-analysis/ml-whisper"]`, **on by
  default**, so official builds and `cargo run -p freecut-app` include speech recognition.
- `cargo build -p freecut-app --no-default-features` (plus the app's other features) builds
  without whisper.cpp and without CMake. The Captions drawer then shows the speech and lyrics
  rows with an empty state that says this build has no speech recognition; manual captions,
  files, looks, burn-in and everything else work. CI builds this configuration on one OS.
- `cargo test -p freecut-analysis` (no features) tests every pure part; the recognizer has a
  fake (§5.6).

### 4.3 Module layout

```
freecut-analysis
  lib.rs            #![forbid(unsafe_code)]; re-exports
  models/           manifest.rs (ModelManifest, ModelEntry, ModelId, Tier), verify.rs (sha256 of a file, chunked, cancellable)
  models/manifest.json   compiled-in list of §3 (and later packs)
  audio.rs          Pcm16k (borrowed mono f32 at 16 kHz + source-time origin), sample↔Tick (exact: 44,100 ticks per sample)
  vad.rs            SpeechRegions, VoiceDetector trait; WhisperVad (ml-whisper); region merging and padding (pure)
  stt/mod.rs        SttRequest, SttProfile, LangChoice, SpeechRecognizer trait, RawWord, LangGuess, SttError
  stt/chunks.rs     plan_chunks: VAD regions → ≤ 30 s windows grouped into ≤ 5 min chunks, cut in silences (pure)
  stt/words.rs      assemble_words: tokens → words (UTF-8 safe byte joining, leading-space rule, ICU word breaks for scripts without spaces) (pure)
  stt/clean.rs      drop_repetition_loops, drop_low_speech_hallucinations (pure)
  stt/whisper.rs    WhisperRecognizer (ml-whisper only)
  stt/fake.rs       FakeRecognizer (scripted words; used by tests in this crate and the app)
  transcript.rs     build_transcript: chunks → freecut_timeline::Transcript (pure)
  captions.rs       words_for_clip, draft_captions (uses timeline's reflow), align_known_text
  fillers/          lexicon.rs (Lexicon, parse, validate), find.rs (find_fillers), data/<lang>.json
  translate.rs      plan_windows, split_back (AI-013, pure; CS4)
```

## 5. Transcription pipeline (AI-009)

### 5.1 Steps

1. **Decode** (app job, `freecut-media`): the chosen stream and channel of the media file,
   resampled to 16 kHz mono `f32` (`freecut_media::audio::read_pcm_mono(media, stream,
   channel, 16_000, range, cancel)`), in blocks of 60 s so memory stays flat on long files.
2. **Voice activity** (`vad.rs`): Silero VAD probabilities per 512-sample window; regions with
   probability ≥ 0.5 (speech profile) or ≥ 0.35 (lyrics profile), at least 250 ms long, merged
   across gaps < 300 ms, padded by 200 ms (FreeCut defaults).
3. **Chunks** (`stt/chunks.rs`): regions are grouped into chunks of at most 5 minutes, cut only
   in silences, each fed to whisper as one call. FreeCut cuts the audio itself and does **not**
   use whisper.cpp's internal VAD switch, so the mapping from model time to source time is one
   addition in our own tested code (`chunk.origin + t`).
4. **Language** (Auto-detect): `lang_detect` over the first 30 s of speech; the result and its
   probability are stored. Below 0.5 the drawer shows "detected: X (unsure)" with a picker to
   re-run in another language (TX-038).
5. **Recognize** (`stt/whisper.rs`): greedy decoding with temperature fallback, `suppress_nst`
   on, `no_speech_thold` 0.6, token timestamps on, DTW alignment heads for the model's preset
   (`WHISPER_AHEADS_BASE`, `_SMALL`, `_LARGE_V3_TURBO`); the tail of the previous chunk's text
   (≤ 200 characters) is the next chunk's prompt, so sentences continue across chunks. Word
   hints (AI-009) and the filler prompt (§7.1) are prepended. Threads: AI-002's limit.
6. **Words** (`stt/words.rs`): token bytes are joined before decoding, so a character split
   across two tokens is never broken; a token that starts with a space starts a new word; for
   scripts written without spaces (Chinese, Japanese, Thai) ICU word breaks group the
   characters. Each word gets start and end in **source ticks** (whisper's 10 ms units ×
   7,056,000, plus the chunk origin), a probability, and a segment-end flag.
7. **Clean** (`stt/clean.rs`): identical n-grams repeated more than 3 times in a row are cut
   (AI-009); a segment whose VAD speech coverage is below 30 % and whose text is in the
   language's short list of known silence phrases is dropped (FreeCut heuristic, listed in the
   lexicon file).
8. **Transcript** (`transcript.rs`): words grouped into sentence segments; AI-001 key
   (fingerprint, stream, range, kind `Transcribe`, model id and version, params hash including
   the backend name). The raw chunk results go to the media cache (PM-074) as soon as each
   chunk finishes, so a cancelled job resumes at the next chunk and another project reuses the
   result.

### 5.2 Determinism

Results stay stable for one key: temperature fallback uses whisper.cpp's fixed seed, and the
params hash includes the compute backend (`metal`, `vulkan`, `cpu`), because GPU and CPU
arithmetic can pick different tokens. The transcript itself is document data (§6.3), so export
never depends on re-running recognition.

### 5.3 Threads, memory, cancellation

- One recognizer job at a time on AI-001's GPU slot; CPU-only machines use the same slot.
- The loaded `WhisperContext` is kept by the worker for 60 s after its last job, then dropped.
- Cancel: the job's cancel flag is read by whisper.cpp's abort callback and between chunks.
  Finished chunks stay in the cache.
- Progress: chunk index plus whisper's per-chunk percentage, reported to PM-073's task row and
  the drawer's progress line.

### 5.4 Speed targets (measured in CS2, not promised here)

CS2 records seconds of audio per second of compute for each tier on three reference machines
(an Apple-silicon laptop with Metal, a 4-core x86 laptop CPU, an x86 desktop with a Vulkan GPU
if CS2 ships Vulkan). The drawer uses the measured numbers for its ETA (AI-001). If the chosen
tier would run slower than real time, the drawer suggests the next faster tier before starting.

### 5.5 Music and lyrics

Whisper is trained on speech; on songs it skips sung words and can invent lines over
instruments. FreeCut's lyrics profile (`SttProfile::Lyrics`, CS3):

- **Isolate vocals first** when AI-023's separation pack is installed (default on); without
  it, the drawer offers to install it or to *try without*, with a note that accuracy drops.
- Lower VAD threshold (0.35), shorter minimum silence (300 ms); no text prompt; segment ends
  at pauses of 300 ms or more become line ends, so each caption is one sung line.
- **No vocals:** if voice activity covers less than 3 % of the clip, the job ends with
  "No vocals detected" and creates nothing (TX-044).
- Lines whose mean word probability is below 0.5 are flagged *needs review* in the caption
  list.
- When the user pastes the lyrics, AI-010's *Align known text* replaces recognition output with
  the user's words and keeps only the timings (§7.2).

### 5.6 Recognizer trait

```rust
pub trait SpeechRecognizer: Send {
    fn model(&self) -> &ModelRef;
    fn detect_language(&mut self, pcm: Pcm16k<'_>, cancel: &dyn Fn() -> bool)
        -> Result<LangGuess, SttError>;
    fn transcribe(&mut self, pcm: Pcm16k<'_>, req: &SttRequest, prompt: Option<&str>,
                  cancel: &dyn Fn() -> bool, progress: &mut dyn FnMut(PerMille))
        -> Result<Vec<RawToken>, SttError>;
}
pub struct SttRequest { pub language: LangChoice, pub task: SttTask, pub profile: SttProfile,
                        pub hints: Option<String>, pub verbatim: Option<LangTag> }
pub enum LangChoice { Auto, Fixed(LangTag) }
pub enum SttTask { Transcribe, TranslateToEnglish }
pub enum SttProfile { Speech, Lyrics }
```

`FakeRecognizer` returns scripted tokens with timings, so the whole job (decode → VAD regions →
chunks → transcript → drafts → `ReplaceCaptions`) is tested without a model. The real
recognizer has one `#[ignore]` test that runs when `FREECUT_TEST_WHISPER_MODEL` points at a
model file.

## 6. Captions data (README C50)

### 6.1 Segments are clips

C5 decided that caption segments are clips on a `TrackKind::Caption` track, so move, trim,
split, selection, snapping and undo come from the common machinery. The TX-025 field list is
kept and placed as follows:

```rust
// freecut-timeline::model::caption (pure data)
pub enum TrackKind { Video, Text, Caption, Sticker, Effect, Adjustment, Audio } // Caption added (band: Caption, top)
pub enum TrackPayload { None, Caption(Arc<CaptionTrackData>) }                  // on Track; Caption iff kind == Caption (validate)
pub enum ClipContent { /* … */ Caption(Arc<CaptionSegment>) }                   // only on Caption tracks (validate)

pub struct CaptionTrackData {
    pub name: String, pub language: Option<LangTag>,
    pub style: TextStyle, pub layout: CaptionLayout, pub rules: CaptionRules,
    pub secondary: SecondaryLine,                 // CS4 UI; stored from CS1 with defaults
    pub look: Option<CaptionLookRef>,             // CS3 (TX-051)
    pub word_styles: WordStateStyles,             // CS3 (TX-039), filler style included (§7.1)
}
pub struct CaptionLayout { pub anchor: VAnchor, pub offset_x: SignedPerMille, pub offset_y: SignedPerMille,
                           pub max_width: PerMille, pub align: HAlign, pub wrap: bool }
pub struct CaptionSegment {
    pub text: RichText, pub secondary_text: Option<RichText>, pub secondary_check: bool,
    pub style_override: Option<TextStylePatch>,   // None until TX-028's per-segment scope lands
    pub words: Option<Arc<[WordTiming]>>,         // segment-local times
    pub speaker: Option<SpeakerId>,
    pub origin: CaptionOrigin, pub edited: bool,
}
pub struct WordTiming { pub start: Tick, pub end: Tick, pub text: ByteRange, pub flags: WordFlags }
pub struct WordFlags(u8);                          // FILLER (CS3), LOW_CONFIDENCE (CS2)
pub enum CaptionOrigin { Manual, Imported, Recognized { model: ModelTag, lang: LangTag },
                         Lyrics { model: Option<ModelTag> }, Script, Translated { engine: EngineTag } }
pub struct CaptionDraft { pub range: TimeRange, pub text: String, pub words: Vec<DraftWord>,
                          pub lang: Option<LangTag>, pub origin: CaptionOrigin }
pub struct DraftWord { pub range: TimeRange, pub text: ByteRange, pub flags: WordFlags }
```

- **Word times are segment-local** (offset from the segment start). Moving or rippling a
  caption clip changes nothing inside it; only caption ops that change the segment's own
  start (trim head, split, merge, retime) rebase the words. A pure
  `WordTiming::rebase(delta)` and the clamp to the segment length are unit-tested.
- **Every caption op** is a planner built on `caption::ops::splice_plan` (C5) and emits
  ordinary changes; each gets an `EditRequest` variant so the assistant's coverage test
  (llm-editor.md) sees it.
- The project format gains `Caption` tracks, caption clips and transcripts in **additive**
  bumps (CS1 and CS2), so `write_compat` (project-format) keeps older readers safe.

### 6.2 Generated results land in one op

`ReplaceCaptions { track, created_track, removed, inserted }` (TX-038) is the only op for
generated captions: speech, lyrics, alignment, re-sync and Whisper translation. It replaces
only segments in scope whose origin is generated and `edited == false`; manual, imported,
scripted and corrected captions are never touched. The drawer's **Replace earlier auto
captions** switch (TX-057) chooses between this replace mode and adding a new track.

### 6.3 Transcripts are document data with labelled commits

AI-009 says a transcript is saved in the project because the user can correct it, and also
that transcription "is not on the undo stack". README §5.11 forbids unlabelled document
changes after open. Resolution: attaching a transcript is a labelled, undoable change
`AttachTranscript { media, stream, before, after }`. An Auto captions run commits **one**
compound entry, "Auto captions", made of its `AttachTranscript` changes and its
`ReplaceCaptions`. Undo removes both; the raw recognizer output stays in the media cache, so
redo or a re-run costs nothing. A plain **Transcribe…** (AI-009) commits its own
"Transcribe" entry.

```rust
// freecut-timeline::model::library::transcript (pure data)
pub struct Transcript { pub media: MediaId, pub stream: u32, pub key: AnalysisKeyStamp,
                        pub language: LangTag, pub lang_prob: PerMille, pub model: ModelTag,
                        pub words: Arc<[TranscriptWord]>, pub segments: Arc<[Range<u32>]> }
pub struct TranscriptWord { pub text: Box<str>, pub start: Tick, pub end: Tick,  // source time
                            pub confidence: PerMille, pub edited: bool }
```

`AnalysisKeyStamp` is the plain-data copy of AI-001's key (fingerprint, stream, range, kind,
algorithm and version, params hash) that staleness is checked against.

## 7. Filler words, lyric alignment and the second line

### 7.1 Filler words (AI-010, AI-012; README C50)

- **Lexicons** are FreeCut data files, one per language, `freecut-analysis/src/fillers/data/<bcp47>.json`:

  ```json
  { "version": 1, "language": "en",
    "verbatim_prompt": "Um, so, uh, I was, like, thinking we could, hmm, try it.",
    "entries": [ { "text": "um", "kind": "hesitation", "checked": true },
                 { "text": "you know", "kind": "discourse", "checked": false } ],
    "silence_phrases": [ "thank you for watching" ] }
  ```

  The prompt is FreeCut's own sentence; it nudges the recognizer to write hesitations down
  instead of smoothing them away (AI-012). English ships in CS3; other languages land as
  reviewed data files (a validation test checks every file: normalized text, no duplicates,
  known kinds).
- **Matching** (`find_fillers`): words normalized (NFKC, case-folded, punctuation stripped),
  longest phrase first, over the transcript's word sequence; scripts without spaces match on
  the token sequence. Ambiguous entries are found but unchecked.
- **Where marks live:** on words, as `WordFlags::FILLER` on `DraftWord` and `WordTiming`, not
  as rich-text spans. TX-015's spans are not built yet, word flags survive TX-039's
  re-alignment after text edits, and AI-012 reads the same flags. The look's word styles carry
  a **Filler** style (default: 50 % opacity, FreeCut default), drawn by the same per-word path
  as the active word.
- **Drawer choice** (TX-057): *Mark filler words* on/off; when on, *Keep them, dimmed*
  (default) or *Leave them out of the captions*. Leaving them out removes the words from the
  drafts before packing; a draft left empty is skipped (TX-038). Cutting fillers out of the
  **video** is never automatic: it is AI-012's review sheet.

### 7.2 Lyric alignment (AI-010 *Align known text*)

`align_known_text(known: &[KnownLine], recognized: &[TranscriptWord]) -> AlignResult` is a
global alignment (Needleman–Wunsch with a band, unit costs) over normalized word tokens;
matched words take recognized times, unmatched words share the time between their anchors in
proportion to their characters, and each pasted line becomes one draft. Long texts are aligned
in windows anchored on runs of 4 confident matches. Lines that cannot be placed follow TX-044
(listed, placed after the last aligned line at 2 s each, flagged).

### 7.3 The second line and translation (README C51)

The second line (TX-036) is data from CS1 and gets its UI in CS4. Where its text comes from:

1. **Typed by hand** — always available, no model (CS4).
2. **Attached from a subtitle file** — TX-031's "attach as secondary" with TX-036's overlap
   rule (CS4).
3. **English from the speech itself** — a second recognizer pass with `SttTask::
   TranslateToEnglish` on the same audio; its segments are attached to the existing captions
   by the TX-036 overlap rule. Needs no extra model (CS4).
4. **The user's assistant provider** (opt-in) — when the assistant (AS-002) is configured,
   *Second line* lists "Translate with my assistant provider". The caption text (never audio
   or video) is sent in AI-013's context windows with numbered boundary tokens, after AS-004's
   disclosure for that run; the answer is split back with AI-013's rule and lands through
   `TranslateCaptions`. Off by default, hidden when no provider is configured (CS4, after
   M-AI1).
5. **Local translation models** — AI-013's OPUS-MT-family packs through the ONNX runtime,
   once AI-002's `ml-onnx` exists. This becomes the default engine when present (later).

FreeCut's own default stays local: options 1–3 never leave the machine, option 4 is the
assistant's existing single exception to *Local only*, used only when the user picks it.

## 8. Captions in the UI

- **Shelf tab *Captions*** (UI-004, ui-concept §8): its Drawer is TX-057. Side list: *From
  speech*, *From a song*, *Type or paste*, *Files*, then *Looks* and *My styles*, then the
  *Caption list* row (TX-026's table shown in the drawer area until UI-056 docking
  lets it float). Rows whose feature is not built are hidden; in a build without `stt` the
  speech and song rows show an explanation instead of a form.
- **Form grammar:** the *From speech* form is a stack of §6.2 property rows inside one card:
  Speech language (Select), Second line (Select), Mark filler words (toggle, with a segmented
  choice when on), Replace earlier auto captions (toggle), Quality (Select listing size and
  installed state), Apply to (Select: whole timeline, selected clips, in–out range), Style
  (look picker), then **Generate** as the accent button across the card. Without a model the
  button reads *Get speech model (190 MB)…* and opens AI-002's prompt.
- **While running:** the form stays visible but disabled, a progress line names the file and
  the percentage, with **Cancel**; the rest of the app stays usable. When done, a one-line
  result ("Added 128 captions to *English (auto)*") with *Caption list* and *Undo*.
- **Inspector:** a caption selection shows the *Caption item* lenses of ui-concept §6.3.
- **Timeline:** caption tracks use the `clip.caption` token and the all-strip clip drawing;
  word ticks (TX-039) are later polish.

## 9. Files and burn-in

Subtitle parsers and writers follow text-and-subtitles.md (`freecut-project::subtitles`,
pure, fixture-tested). CS1 builds **SRT and WebVTT** read and write; LRC comes with lyrics in
CS3; ASS, SBV and timed TXT follow inside M8. Burn-in uses the one renderer of C44: caption
tracks are planned into `FramePlan` as text items in the Caption band and drawn after every
other track by `freecut-render::text::caption`, in preview and export alike. The export dialog
gets, per caption track, **Burn into video** (default on for visible tracks) and **Also save a
subtitle file** (SRT or VTT next to the video, EX-045 naming). Embedding a soft subtitle stream
(TX-033 *Embed*) follows in M8 with C21's writer.

## 10. Build slices

Every slice ends with `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`
and `cargo test --workspace` green on macOS, Windows and Linux, every new edit op with an
apply → undo → redo test, and nothing decoded, recognized, downloaded or written on the UI
thread.

### CS1 — Caption model, SRT/VTT, caption list, burn-in (task's C1)

- **Features:** TX-025, TX-026 (table, inline edit, keyboard split and merge, filter), TX-027,
  TX-028 (track scope only), TX-031 and TX-032 (SRT, WebVTT), TX-033 (burn-in and sidecar),
  TX-037 (rules, warnings, re-flow), TX-057 (*Type or paste*, *Files*, *Caption list* rows).
- **Crates:** `timeline` (model, ops, reflow, frame-plan items), `project` (DTOs, additive
  format bump, `subtitles::{srt, vtt}`), `render` (`text::caption` block layout on the CPU
  compositor), `app` (`ui::captions` drawer and list, import/export jobs with `encoding_rs` +
  `chardetng`, export-dialog options).
- **Contracts:** §6.1 types; ops `AddCaptionTrack`, `RemoveCaptionTrack`,
  `InsertCaptionSegments`, `SetCaptionText`, `SetCaptionRange`, `SplitCaption`,
  `MergeCaptions`, `DeleteCaptions { ripple }`, `SetCaptionTrackStyle`, `SetCaptionRules`,
  `ReflowCaptions`, `ImportSubtitles`, `ReplaceCaptions`; `CaptionRules::check(&seg) ->
  Vec<Warning>`; `reflow(&[CaptionSegment], &CaptionRules) -> Vec<CaptionSegment>`;
  `subtitles::{parse(&str, Format) -> Result<(SubtitleDoc, ParseReport), ParseError>,
  write(&SubtitleDoc, Format, &WriteOptions) -> String}`; `TextItemRef::Caption { track, clip }`
  in the frame plan with `Band::Caption`.
- **Tests:** one unit test per TX-025/TX-026/TX-027 edge case; a property test that every
  caption op round-trips apply → undo → redo and keeps segments sorted and non-overlapping;
  reflow goldens for Latin, CJK and Arabic; project round trip and golden file; SRT/VTT parser
  fixtures (byte-order mark, CRLF, missing index numbers, malformed timing reported by line,
  overlapping cues split into extra tracks, 50,000 cues under 1 s); write → parse round trip
  at millisecond precision; CPU render golden of a two-line caption with outline at the bottom
  anchor; preview frame equals export frame for the same time; app snapshot tests of the drawer
  rows and the caption list.
- **Acceptance:** a user adds captions by hand and from an SRT file, edits text and times in
  the list, splits and merges, restyles the track, exports an MP4 with the captions burned in
  plus an SRT next to it, reopens the project and finds everything; every step undoes.

### CS2 — Analysis crate, speech recognizer, model manager, Auto captions (task's C2)

- **Features:** AI-001 (minimal pool: one GPU slot, PM-073 rows, cache of chunk results),
  AI-002 (speech packs only: manifest, first-use prompt, Preferences rows, download, verify,
  resume, cancel, import from file, remove), AI-009, AI-010 (*Recognize speech*, scopes,
  *Use existing transcripts*, replace mode, *Re-sync captions*), TX-038, TX-039 (word timings
  stored; highlight drawing waits for CS3), TX-057 (*From speech* row).
- **Crates:** new `analysis` (§4), `timeline` (`Transcript`, `AttachTranscript`,
  `CaptionDraft` → segments), `project` (transcript DTO, additive bump), `media`
  (`read_pcm_mono` at 16 kHz), `app` (`stt` feature, `jobs::analysis`, `jobs::model_download`
  with `ureq` + rustls and `sha2`, Preferences → Smart tools → Models, drawer form).
- **Contracts:** §5.6 trait; `plan_chunks(&[TimeRange], ChunkLimits) -> Vec<Chunk>`;
  `assemble_words(&[RawToken], &LangTag) -> Vec<RawWord>`; `build_transcript(..) ->
  Transcript`; `words_for_clip(&Transcript, used: TimeRange, map: &TimeMap) -> Vec<TimedWord>`;
  `draft_captions(&[TimedWord], &CaptionRules, &DraftOptions) -> Vec<CaptionDraft>`;
  `ModelManifest::entries_for(Task) -> &[ModelEntry]`; app downloader
  `download(entry, dir, cancel, progress) -> Result<PathBuf, DownloadError>` (§ model download
  policy in AI-002).
- **Tests:** pure tests for chunk planning (cuts only in silence, 5 min cap), word assembly
  (a CJK character split across two tokens, leading-space rule, Thai without spaces),
  repetition-loop removal, the 50 % word-inside rule and speed-changed clips in
  `words_for_clip`, packing against the rules; the whole job with `FakeRecognizer` from a WAV
  fixture to a caption track, then one undo removing track and transcript; manifest test (every
  entry has HTTPS URL pinned to a revision, SHA-256, size, licence id and URL); downloader
  against a local test server (fresh, resume with 206, server ignoring `Range` restarts, 416,
  hash mismatch deletes and reports "damaged", cancel keeps the partial file, low disk space
  refuses before starting); `cargo build -p freecut-app --no-default-features` in CI; the
  dependency guard checks `freecut-analysis → timeline` only and that `whisper-rs-sys` is absent
  without `stt`; one `#[ignore]` real-model test on a CC0 speech clip (expected words, word
  starts within 200 ms).
- **Acceptance:** after one confirmed 190 MB download, a user generates captions for a
  10-minute interview in any language Whisper knows, with auto-detection; the captions follow
  the track's line rules, can be edited, and replace only untouched auto captions on a re-run;
  cancelling and restarting continues where it stopped; with the network off afterwards,
  everything still works. Speed per tier is measured on the reference machines and recorded
  in this page.

### CS3 — Filler words, lyrics, caption looks (task's C3)

- **Features:** AI-010 *Mark filler words*, AI-012 lexicons (English), TX-044 (from the music,
  align pasted lyrics, LRC), AI-010 *Recognize song lyrics* and *Align known text*, TX-039
  (active, spoken, upcoming and filler word styles drawn), TX-051 (a first set of six
  FreeCut-made caption looks), TX-019 caption-suitable style presets in the drawer, TX-057
  (*From a song*, *Looks*, *My styles* rows, Style picker in the forms).
- **Crates:** `analysis` (`fillers`, `SttProfile::Lyrics`, `align_known_text`), `timeline`
  (`WordFlags`, `WordStateStyles`, `ApplyCaptionLook`, word-state evaluation), `project`
  (look manifest schema, `subtitles::lrc`), `render` (per-word style in `text::caption`, on the
  per-glyph path of C46), `app` (forms, look grid with live thumbnails, §13.7).
- **Contracts:** `Lexicon::parse(&str) -> Result<Lexicon, LexiconError>`;
  `find_fillers(&[TranscriptWord], &Lexicon) -> Vec<FillerHit>`; `DraftOptions { fillers:
  FillerMode::{Off, Dim, LeaveOut}, .. }`; `align_known_text` (§7.2); `word_state_at(&seg,
  t) -> impl Iterator<Item = WordState>`; caption look files as FreeCut data in
  `resources/caption-looks/*.json` (base style patch, word styles, display mode).
- **Tests:** lexicon validation for every shipped file; filler matching (multi-word phrases,
  punctuation, case, unchecked ambiguous entries); *Leave out* removes words and skips empty
  drafts; lyric line splitting and the no-vocals threshold from fixture word lists; alignment
  fixtures (skipped verse, repeated chorus, misheard words, wrong song); LRC parse and write,
  including word tags; render goldens of a caption at three times showing upcoming, active and
  spoken words; look apply → undo; look files parse against the schema.
- **Acceptance:** fillers in an English talk appear dimmed, or not at all, as chosen; a song
  gets line-by-line lyric captions with a moving word highlight, or the user's pasted lyrics
  timed to it; six looks restyle a caption track in one click and export exactly as previewed.

### CS4 — The second line (task's C4)

- **Features:** TX-036 (second column, inspector *Second line* card, placement, gap, display,
  swap, *check translation* mark), TX-031 attach mode, TX-032 both-lines export, TX-041 with
  the engines of §7.3 options 1–4, AI-013's `plan_windows` and `split_back`, TX-057's
  *Second line* select.
- **Crates:** `timeline` (`SetCaptionText { field }`, `SetSecondaryStyle`, `SwapCaptionLines`,
  `TranslateCaptions`, attach-by-overlap), `analysis` (`translate.rs`, `SttTask::
  TranslateToEnglish` path), `render` (stacked two-line block, each line with its own
  direction), `project` (both-lines writing), `app` (UI; assistant engine wiring only when
  `freecut-assistant` exists).
- **Tests:** overlap attach fixtures (most overlap wins, nearest midpoint within 0.5 s,
  unmatched reported); `split_back` with tokens kept, tokens lost, CJK; the *check translation*
  mark set by primary edits and splits and cleared by updates; swap applied twice is identity;
  render goldens for below and above, and RTL under LTR; SRT export of both lines; assistant
  engine replayed from a recorded reply with no network, refusing to send before the
  disclosure is accepted.
- **Acceptance:** a user types a second line by hand, or gets an English second line from
  Spanish speech, or (with an assistant provider configured and confirmed) a translated
  second line, and exports a video with both lines burned in.

## 11. Risks and open questions

1. **Vulkan in official Windows and Linux builds.** It needs the Vulkan SDK on the release
   machines and a Vulkan loader on users' machines; without it those builds are CPU-only. To
   be decided with the release pipeline (README §8).
2. **CMake as a build requirement** for the default build. Contributors who cannot install it
   build with `--no-default-features`; the README's build notes must say so.
3. **CLAUDE.md lists six crates.** The maintainer updates its crate list and dependency
   sentence when CS2 starts (README C48); this page does not edit it.
4. **Silero VAD file version** must match the pinned whisper.cpp (§3).
5. **Vocal isolation pack** (AI-023): its weight licence is still marked "verify"; lyrics work
   without it, less accurately.
6. **Assistant translation** (§7.3 option 4) extends where the assistant's provider is used;
   the AS and AI owners confirm before CS4 builds it.
7. **Transcript size in the project file:** about 30 bytes per word as compact arrays; an hour
   of speech (~9,000 words) adds ~270 KB. Acceptable; revisit if projects with many hours
   appear.

## 12. Provenance and sources

- Crate metadata (version, licence, features, MSRV) from `cargo info` against crates.io on
  2026-10-13 for `whisper-rs`, `whisper-rs-sys`, `sherpa-onnx`, `sherpa-onnx-sys`,
  `sherpa-rs`, `ort`, `candle-transformers`, `vosk`, `ureq`, `sha2`, `cmake` and `bindgen`.
  The sources of `whisper-rs` 0.16.0, `whisper-rs-sys` 0.15.0 (`build.rs`,
  `whisper.cpp/CMakeLists.txt`, `whisper.cpp/include/whisper.h`, `whisper.cpp/LICENSE`) and
  `sherpa-onnx` / `sherpa-onnx-sys` 1.13.8 (`build.rs`, `src/offline_asr.rs`) were read in the
  local cargo registry. No crate was added to the workspace.
- Model file names, sizes, SHA-256 values, repository revisions and licence tags from the
  Hugging Face model API for `ggerganov/whisper.cpp`, `ggml-org/whisper-vad`,
  `openai/whisper-large-v3-turbo` and `openai/whisper-small` on 2026-10-13. No model was
  downloaded.
- The reference editor's English UI-label inventory (see [features README](../features/README.md)
  Provenance) was used only to confirm which caption features exist; every label here is
  FreeCut's own wording.
- whisper.cpp — https://github.com/ggml-org/whisper.cpp
- whisper-rs — https://codeberg.org/tazz4843/whisper-rs
- sherpa-onnx — https://github.com/k2-fsa/sherpa-onnx
- OpenAI Whisper (code and weights, MIT) — https://github.com/openai/whisper
- Silero VAD — https://github.com/snakers4/silero-vad
- ggml Whisper models — https://huggingface.co/ggerganov/whisper.cpp
- ggml VAD models — https://huggingface.co/ggml-org/whisper-vad
