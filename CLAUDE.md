# FreeCut

Free, open-source desktop video editor in Rust, covering the everyday workflow
of editors like CapCut. Clean-room project: we reimplement *behavior*, never
proprietary code or assets.

## Commands

- Build: `cargo build --workspace`
- Run: `cargo run -p freecut-app`
- Test all: `cargo test --workspace`
- Test one crate: `cargo test -p freecut-timeline`
- Test one test: `cargo test -p <crate> <test_name>`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all`

Run fmt, clippy, and tests before saying a change is done.

## Architecture

Cargo workspace under `crates/`:

- `freecut-timeline` — pure edit model (tracks, clips, ops, undo). **No I/O, no
  FFmpeg, no GPU.** Must stay unit-testable in isolation.
- `freecut-media` — the only crate that talks to FFmpeg (`ffmpeg-next`).
- `freecut-render` — wgpu compositor, effects, transitions (WGSL shaders).
- `freecut-audio` — mixing and playback (cpal).
- `freecut-project` — serde project format; versioned, with migrations.
- `freecut-app` — egui/eframe UI; wires the other crates together.
- `freecut-cloud` — opt-in generation through the user's own account on a
  third-party service (Higgsfield first): MCP client, OAuth sign-in, model
  catalogue, cloud job ledger. Off by default; networking only behind its
  `http` feature. No FreeCut server, no telemetry.

Dependency direction: `app` → everything; `render`/`audio` → `media` +
`timeline`; `cloud` → `timeline` + `media` (without FFmpeg, for the cancel
token only); `timeline` depends on nothing internal. Nothing but `app`
depends on `cloud`.

## Conventions

- Time is a rational (`i64` ticks + timebase), never `f32`/`f64`. Convert to
  float only at the UI/display edge.
- Never decode, render, or do file I/O on the UI thread.
- Every edit operation goes through the undo stack; it must be reversible and
  have a test.
- Errors: `thiserror` in library crates, `anyhow` only in `freecut-app`.
- No `unwrap()`/`expect()` outside tests unless the invariant is commented.
- `unsafe` only inside `freecut-media`/`freecut-render`, each block with a
  `// SAFETY:` comment.
- Paid cloud calls are never retried or resent automatically; every one
  needs the user's confirmation after a cost preflight
  (docs/features/cloud-generation.md, architecture §4.5–§4.7).

## Skills

- `rust-skills` — Rust idioms, ownership, async, error handling. Default for
  any Rust code.
- `a-philosophy-of-software-design` — designing crate/module boundaries and
  public APIs.
- `clean-code`, `code-complete` — everyday code quality.
- `refactoring` — restructuring existing code without changing behavior.
- `the-pragmatic-programmer` — slicing a feature into small end-to-end steps.

Don't apply the DDD skills, `patterns-of-enterprise-application-architecture`,
or `designing-data-intensive-applications` here: FreeCut is a single-user
desktop app with no business domain, database, or distributed system.

## Feature workflow

1. Write or update a spec in `docs/features/<feature>.md`: what the user sees,
   inputs, outputs, edge cases — in our own words.
2. Implement from the spec, starting with the model in `freecut-timeline`.
3. Add tests, then the UI.

## Clean-room rules (non-negotiable)

Allowed:
- Using CapCut as an ordinary user and describing its features and UX in our
  own words in `docs/features/`.
- Studying public docs, tutorials, and open standards (codecs, containers, LUT
  formats).
- Documenting the format of project files *we created ourselves*, only to build
  an importer for interoperability.

If a task would require any of the above, stop and say so instead.
