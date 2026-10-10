# FreeCut

A free, open-source desktop video editor written in Rust — aiming for the
everyday editing workflow people know from CapCut: a multi-track timeline,
quick trims and splits, text, transitions, filters, speed control, and fast
export.

> **Status:** early planning. Nothing is usable yet.

> FreeCut is an independent project. It is **not** affiliated with, endorsed by,
> or derived from ByteDance or CapCut. "CapCut" is a trademark of its owner and
> is used here only to describe the kind of editor FreeCut aims to be.

## Goals

- **Free forever** — no watermark, no paywalled features, no account required.
- **Fast** — GPU-accelerated preview and export.
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
- **Rendering & effects:** wgpu (GPU compositing, shaders)
- **Decode / encode:** FFmpeg via `ffmpeg-next`
- **Audio output:** cpal
- **Project files:** serde (JSON)

## Repository layout (planned)

```
crates/
  freecut-app/       # desktop binary: windows, panels, timeline UI
  freecut-timeline/  # edit model: tracks, clips, edit operations, undo/redo
  freecut-media/     # FFmpeg decode/encode, probing, thumbnails, proxies
  freecut-render/    # wgpu compositor, effects, transitions
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
