# CS2a review: 16 kHz mono PCM reader (AI-009 step 1)

Branch `worktree-cs2a`, commit `c8b49bd` on main `339c149`. Lane CS2a, brief
`docs/handoff/captions-side-lanes.md`. Design: `speech-and-captions.md` §5.1.

## Verdict: approve with changes

No blockers or majors. The core claims hold up when checked against the rubato
5.0.1 source and against throwaway probes:

- **Delay alignment.** `output_delay()` is `fft_size_out/2`, the sinc is
  centred under a periodic window, and `FixedSync::Both` gives exactly the
  planned sizes. The delay is a whole number of output samples, and the skip
  is exact.
- **Tick origins.** Each origin is `n * 44,100` ticks, with no accumulation.
  A 3 h range fits easily in i64.
- **Probes that passed:** joins at the 60 s block boundary, a mid-stream
  start that crosses a boundary, an off-grid 37,800 Hz source, a 6-channel
  source through the FFT path, an MKV whose audio starts late, the tail
  flush, and a 1 h AAC file (60 blocks, 5.7 s in release).

The gate is green in the worktree: fmt, clippy `-D warnings` (workspace, and
freecut-media with `--no-default-features`), `cargo test --workspace` (1931
passed, 0 failed, 8 ignored), and `cargo doc` with no warnings.

- **Licences:** rubato `=5.0.1` and all 13 transitive crates match what the
  agent claimed, and all are compatible with GPL-3.0.
- **Conventions:** no `unwrap`/`expect`, no `unsafe`, errors use thiserror,
  and time is in Ticks throughout.
- **Deviations:** both disclosed deviations (`DecodeError`, the named
  `PcmBlocks` iterator) are sound.

The changes below are small, and none needs edits to `AudioReader`, which the
lane rules forbid.

## Required changes

None blocking. Fix these before merge:

**CS2a-1 (minor): the stated cancel latency is wrong for forward-only streams.**
`crates/freecut-media/src/audio/pcm.rs:325-337`, `:268`, `:278`.
- Cancel is checked between `AudioReader::read` calls, but one read can decode
  far more than a second of audio. With MP3, ADTS AAC or Ogg and a range that
  starts mid-file, the first read decodes the whole prefix inside
  `produce()`. Opening the file can't be cancelled either.
- Measured: a 1 h MP3 with the range starting at 3000 s took 1.72 s to cancel
  in release. That cost grows with the start offset.
- Fix in this lane: say so in the module docs and in the `read_pcm_mono` docs
  ("cancel latency is about 1 s, except when opening the file and on the
  first read into forward-only streams"). Add a follow-up for the audio.rs
  owner: a crate-private `AudioReader` open that takes the caller's
  `CancelToken`, and checks in the forward-decode loop.

**CS2a-2 (minor): paths the agent claims to handle have no tests.**
`crates/freecut-media/tests/pcm.rs` (~:254).
Add these tests:
- **(a) Off-grid rate:** a 37,800 Hz WAV with an impulse on a grid sample.
  Check that the peak lands on sample 24,000 and that its neighbours are
  symmetric. This exercises `OFF_GRID_READ_RATE`.
- **(b) Six channels through the FFT path:** a 6-channel 48 kHz WAV checked
  against the expected ITU mix. Today's 6-channel test runs at 16 kHz and
  never resamples.
- **(c) The block join:** read [59 s, 61 s) and [30 s, 100 s) of a 130 s file
  and compare each with the matching slice of a whole read.
- **(d) Audio that starts late:** a stream whose audio starts after time 0
  (a muxed fixture). Skip it when the `ffmpeg` CLI is missing.

**CS2a-CONV-1 (minor): `known_end` reads private `AudioReader` state.**
`crates/freecut-media/src/audio/pcm.rs:360-368`.
- `known_end` reads `pipe`, `Pipe::done`, `Pipe::out_start` and `o_end()`.
  What those fields mean is defined only by how audio.rs uses them today, so
  the owner could change it without breaking the build.
- Fix in this lane: none to code. Name the dependency in the `known_end` doc,
  and record a follow-up for a documented
  `pub(crate) fn known_end(&self) -> Option<i64>` on `AudioReader`.

## Nits (optional)

- **CS2a-3, pcm.rs:314:** a returned block can carry up to 2x its length in
  spare capacity (up to about 3.8 MB). Call `shrink_to_fit()`, or reserve
  `pending` once.
- **CS2a-4, pcm.rs:516:** rubato errors are reported as
  `DecodeError::Ffmpeg { op: "resample audio", code: 0 }`. The crate already
  does this elsewhere (audio.rs:292), so it's acceptable. Ideally the op
  string would say "resample audio (rubato)".
- **CS2a-CONV-5, pcm.rs:268:** the source is opened twice to learn its native
  format. That is one extra open per call. Comment that it is deliberate.
- **For the orchestrator, not the agent:**
  - **CONV-2:** the lane goes over the §3 line budget: `lib.rs` changes 1
    line, `audio.rs` adds 2, and the crate's `Cargo.toml` changes. All of it
    is unavoidable and was disclosed, so accept it.
  - **CONV-3:** `pub mod audio` also exposes duplicate paths to
    `AudioReader` and `audio_peaks`, and makes the audio.rs `//!`
    implementation notes public docs. The contract path requires this, so
    accept it.
  - **CONV-7:** §5.1 and brief line 72 still say `16_000`, `MediaError` and
    `impl Iterator`. CS2b should update them to `rate: SampleRate` and
    `-> PcmBlocks` with items `Result<PcmBlock, DecodeError>`. Note that the
    `SampleRate` parameter is a third deviation the agent didn't list,
    though it's a better design.
  - **CONV-8:** add `audio::read_pcm_mono` to the crate doc list in `lib.rs`
    at merge.

## Message for the CS2a agent

> Review of c8b49bd: approve with changes. The delay alignment, tick origins,
> block contiguity, licences and gate all check out. Please fix the following
> in your lane, without touching AudioReader:
> 1. **Cancel docs** (pcm.rs module docs + `read_pcm_mono` docs): say that
>    cancel latency is about 1 s except when opening the file, and on the
>    first read of a range that starts mid-file in a forward-only stream
>    (MP3/ADTS AAC/Ogg). Those decode the whole prefix inside one
>    `AudioReader::read` (measured 1.72 s at a 3000 s offset). Add a
>    follow-up note asking for a cancellable `AudioReader` open/forward
>    decode.
> 2. **tests/pcm.rs:** add (a) a 37,800 Hz off-grid WAV impulse that lands
>    exactly on its 16 kHz grid sample with symmetric neighbours; (b) a
>    6-channel 48 kHz WAV checked against the ITU mix through the FFT path;
>    (c) ranges [59 s, 61 s) and [30 s, 100 s) of a 130 s file matching the
>    whole-read slice across the 960,000-sample join; (d) a stream whose
>    audio starts after 0, skipped when the ffmpeg CLI is missing.
> 3. **`known_end` doc** (pcm.rs:360): name the AudioReader internals it
>    depends on (`pipe.done`, `pipe.out_start`, `o_end()`) and add a
>    follow-up for a documented `pub(crate) known_end()` on AudioReader.
>
> Optional:
> - Call `shrink_to_fit()` on returned blocks, or reserve `pending` once
>   (pcm.rs:314).
> - Make the rubato error op string say "rubato" (pcm.rs:516).
> - Comment that opening the source twice is deliberate (pcm.rs:268).
>
> Re-run fmt, clippy `-D warnings` and `cargo test --workspace` in the
> worktree, then report.
