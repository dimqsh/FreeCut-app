# LUT test fixtures

FreeCut's own test data for `freecut_media::lut` (CL-016), written for these
tests. None of it comes from another product. The larger tables were
generated from the formulas below with a short script and are plain text.

## Valid

| File | What it is |
|---|---|
| `identity_2.cube` | 2³ identity with a `TITLE` and comments |
| `invert_17.cube` | 17³, `out = 1 − in`, 6 decimals |
| `ramp_1d_1024.cube` | 1D, 1024 entries: red `x`, green `x²`, blue `1 − x` |
| `shaper_3d.cube` | 3-entry 1D shaper (0, 0.25, 1), then a 2³ invert |
| `domain_0_2.cube` | 2³ identity with `DOMAIN_MIN 0 0 0` / `DOMAIN_MAX 2 2 2` |
| `input_range.cube` | the same domain as `LUT_3D_INPUT_RANGE 0 2` |
| `crlf_bom.cube` | 2³ identity with a UTF-8 BOM, CRLF, tabs, trailing spaces and blank lines |
| `comments_title.cube` | comments everywhere, `#` inside the quoted title, two unknown keywords |
| `latin1_title.cube` | a Latin-1 (not UTF-8) title |
| `out_of_range.cube` | outputs below 0 and above 1, which are kept |
| `rotate_17_10bit.3dl` | mesh `0 64 … 960 1023`, 10-bit, `out = (g, b, r)` |
| `identity_3_12bit.3dl` | mesh `0 2048 4095`, 12-bit identity |
| `invert_2_float.3dl` | float outputs, `out = 1 − in` |
| `uneven_mesh.3dl` | mesh `0 256 1023` (not even), identity at the mesh positions |

## Rejected (`bad/`)

Each file breaks exactly one validation rule. Its first line says which, and
`tests/lut.rs` (`rejected()`) lists the expected line and reason. A test fails
if a file here is not listed.
