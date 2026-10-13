# Bundled resources

Data files compiled into FreeCut. Every file here is FreeCut's own work.

- `animations/`: clip animation presets (FX-030), one `<id>.json` each.
- `text-animations/`: text animation presets (TX-056), one `<id>.json` each.

Both use the preset manifest format described in
`crates/freecut-project/src/presets.rs` and FX-030
(`docs/features/effects-transitions-animations.md`). A new file must also be
listed in `freecut_project::presets::BUNDLED`; a test fails until it is.

## Ids

- Text presets: `fc.in.<name>`, `fc.out.<name>` (Out-only presets) and
  `fc.loop.<name>`, as TX-056 names them.
- Clip presets: `fc.clip.in.<name>`, `fc.clip.loop.<name>`,
  `fc.clip.drift.<name>` and `fc.clip.combined.<name>`, so a clip grid and a
  text grid never share an id.
- Exits have no files of their own unless the spec defines one (Erase, Fall
  away): `fc.out.x` and `fc.clip.out.x` play `fc.in.x` and `fc.clip.in.x`
  backwards (FX-030).

## Seeded and line-relative channels

Most channels move a unit the same way every time. A few depend on the
clip's stored seed (TX-023's dice, so preview, export and every OS agree)
or on where the unit sits in its line. The engine
(`freecut_timeline::motion`, TX-056 and FX-030 give the exact rules)
provides all of them:

| Channel | Meaning | Presets |
|---|---|---|
| `random.offset` | Each unit starts at a seeded direction and distance, 0.5–1 × the amplitude away, times the curve. | Scatter |
| `random.rotate` | Each unit turns by a seeded angle within ±amplitude, times the curve. | Scatter, Fall away |
| `toward_line_center` | Each unit moves this share of the way from its centre to its line's centre. A whole block never moves. | Spread |
| `line_rise` | The unit moves down by this many of its line heights (1.2 em without a layout), clipped to its line's box while it moves. | Word rise |
| `jitter.translate_x`, `jitter.translate_y`, `jitter.rotate` | Smooth seeded noise within ±amplitude, changing direction 12 times a second, and seamless over a cycle. The curve is its envelope. | Shake, Wiggle, Jitter shake (clip) |
| `flicker.opacity` | Opacity dips to 1 − amplitude for 2 frames at seeded moments, at least 1 s apart (also across the cycle seam). A cycle under 1 s does not dip. | Flicker |
| `sparkle.scale` | Each unit swells to 1 + amplitude for 0.2 s, once per cycle, at a seeded moment. | Sparkle |
| `turn_from_side` | A 3D turn whose axis follows the direction. From the left it turns +amplitude about the vertical axis, from the right −amplitude. From the top it turns +amplitude about the horizontal axis, from the bottom −amplitude. | 3D flip in (clip) |

The renderer gives each unit its reading-order index, the vector to its
line's centre, and its line height (`freecut_timeline::motion::UnitContext`).

## Tests

`crates/freecut-project/tests/catalogue.rs` checks every preset against:

- **The spec's numbers.** Seeded and line-relative presets are checked
  against bounds over many seeds and units: Scatter's distance and turn,
  Spread's share, Word rise's line height, jitter's amplitude and its 12
  turns a second, Flicker's one 2-frame dip a second, Sparkle's 0.2 s swell,
  and the 3D flip's axis per side.
- **Golden samples** at 0/25/50/75/100 % of its slot
  (`tests/golden/presets.txt`, re-blessed with `FREECUT_BLESS_GOLDEN=1`).
  Seeded and line-relative presets add two more units, and Flicker and
  Sparkle add when in the cycle each of their moments falls.
- **The rest rules.** An In ends at rest and an Out starts at rest. An Out
  without a file of its own is its In backwards, from the opposite side:
  slides, rotations and 3D turns mirror. A Loop repeats seamlessly. These
  hold for every seed and unit tried.
- **Intensity scaling.**
- **The photosensitivity rule**, checked at Intensity 0, 50 and 100 and, for
  seeded presets, over several seeds and units. It allows at most 3 sharp
  changes of brightness or opacity in any second, and at most 2 dips per
  second in a loop. The engine's own survey
  (`freecut_timeline::motion::photosensitivity`, which the grids use) has to
  agree.

`crates/freecut-project/src/presets.rs` checks that every bundled file
loads.

## Presets this build can't play

A preset file that names a channel this build doesn't know, or that was
written for a newer schema, is skipped. Its id stays in the table as
**Unavailable animation** with the reason (FX-018, FX-030,
`PresetTable::availability`), so the grids can show it instead of hiding it.
A clip that chose it plays no animation. None of the bundled presets is in
this state today.
