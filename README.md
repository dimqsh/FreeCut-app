# FreeCut

A free, open-source desktop video editor written in Rust — aiming for the
everyday editing workflow people know from CapCut: a multi-track timeline,
quick trims and splits, text, transitions, filters, speed control, and fast
export.

> **Status:** early desktop prototype. Build instructions, preview packages
> and current platform limits are documented in [docs/packaging.md](docs/packaging.md).

> FreeCut is an independent project. It is **not** affiliated with, endorsed by,
> or derived from ByteDance or CapCut. "CapCut" is a trademark of its owner and
> is used here only to describe the kind of editor FreeCut aims to be.

## Download preview

| Platform | Download | Requirements |
|---|---|---|
| macOS | [DMG for Apple Silicon](https://github.com/dimqsh/FreeCut-app/releases/tag/v0.1.0-macos-preview) | macOS 15+, M-series Mac |
| Windows | [Installer and portable ZIP](https://github.com/dimqsh/FreeCut-app/releases/tag/v0.1.0-windows-preview) | Windows 10 22H2 / 11, x64 |

On Windows without a GPU, use **FreeCut (CPU)** or **FreeCut-CPU.cmd**.
The macOS preview uses Metal for the window. These are unsigned / ad-hoc signed
preview packages; see each release for installation steps, checksums and tested limits.

## Goals

- **Free forever** — no watermark, no paywalled features, no account required.
- **Fast** — CPU video compositing, bounded caches and a lightweight desktop UI.
- **Simple** — the common edit (cut, arrange, caption, export) takes minutes.
- **Clean-room** — every feature is designed and written from scratch based on
  publicly observable behavior. No proprietary code, assets, or models.

## Planned features

| Area        | Features                                                         |
|-------------|------------------------------------------------------------------|
| Timeline    | Multi-track video/audio/text, trim, split, ripple delete, snapping, undo/redo |
| Media       | Import video/audio/images, proxy generation, thumbnails, waveforms |
| Editing     | Speed (constant + curves), reverse, crop, rotate, keyframes      |
| Visuals     | Transitions, filters/LUTs, color adjustment, blend modes         |
| Text        | Titles, captions, styles, animations                             |
| Audio       | Volume, fades, mixing                                            |
| Export      | H.264/H.265/AV1, presets for common aspect ratios (9:16, 16:9, 1:1) |
| Later       | Auto-captions (open speech models), background removal (open models) |

## Tech stack

- **Language:** Rust (stable)
- **UI:** egui / eframe
- **Rendering & effects:** CPU compositor; wgpu for the desktop window
- **Decode / encode:** FFmpeg via `ffmpeg-next`
- **Audio output:** cpal
- **Project files:** serde (JSON)

## Repository layout

```
crates/
  freecut-app/       # desktop binary: windows, panels, timeline UI
  freecut-timeline/  # edit model: tracks, clips, edit operations, undo/redo
  freecut-media/     # FFmpeg decode/encode, probing, thumbnails, proxies
  freecut-render/    # CPU compositor, effects, transitions
  freecut-audio/     # mixing and playback
  freecut-project/   # project file format (save/load)
docs/
  features/          # feature specs written from observed behavior
```

## Building

Requires Rust (stable) and FFmpeg development libraries.

```bash
# macOS
brew install ffmpeg pkg-config

cargo build --release
cargo run -p freecut-app
```

Use the release build when measuring playback or export performance:
`cargo run -p freecut-app --release`. Resource costs, reproducible benchmarks,
and the current older-hardware/OS limitations are documented in
[docs/performance.md](docs/performance.md).

Build a local macOS `.app` and `.dmg` with bundled FFmpeg libraries using
`python3 scripts/package_macos.py` (Python 3.11+). Output sizes, supported
architecture, signing status and Windows packaging notes are documented in
[docs/packaging.md](docs/packaging.md).

## Contributing

Contributions are welcome. Before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Clean-room rule:** do not contribute code, assets (effects, LUTs, stickers,
fonts, music, templates, icons), or ML models taken from CapCut or any other
proprietary editor, and do not submit work based on decompiling them. Describe
features in your own words in `docs/features/` and implement them from scratch.

## License

GPL-3.0-or-later (see `LICENSE`).
