# Handoff: colour slices K4, K5, K6 (three parallel branches)

> **Superseded (2026-10-14):** K4 → K5 → K6 are built **sequentially on the main line** by the orchestrator (three agents on the same files is an anti-pattern). Do not start these slices on separate branches. Parallel work goes to disjoint lanes instead: `docs/handoff/captions-side-lanes.md` and the Higgsfield branch.

Three agents build the rest of FreeCut's colour round at the same time, each on its own branch
and in its own worktree. The orchestrator reviews every branch and merges them **in order K4 → K5 → K6**,
each one rebased on the previous. These slices normally form a chain, so the rules in §2 exist to keep
the rebases small.

## 1. Read first

1. `CLAUDE.md`: Tick time, an undo test for every edit, no I/O on the UI thread, `thiserror` in
   libraries, no `unwrap`/`expect` outside tests unless the invariant is commented, `unsafe` only in
   media/render with a `// SAFETY:` comment.
2. The build plan, round 3, at `/private/tmp/claude-501/-Users-dimash-PycharmProjects-FreeCut-app/f0cd0716-b9da-41be-93c5-0e00e0cdbfb5/scratchpad/round3-plan.md`.
   Read §3 ("Rules for every slice") and your slice in §5 (K4, K5 or K6). If that path is not
   readable, use the summaries in §4 below.
3. `docs/features/color-and-filters.md`: CL-001 (stack and order, with the "K1 as built" note),
   CL-006, CL-019, CL-020, CL-026, CL-028, CL-031, CL-043..CL-045.
4. `docs/architecture/README.md` C44, C46 and C52 (the CPU renderer is the reference, and preview
   must equal export byte for byte), plus `docs/architecture/render-compositor.md` §17.
5. `docs/design/ui-concept.md`: the Look lens (pages Basic / HSL / Curves / Wheels / Masks) and
   the Adjustment asset tab.
6. The code that already exists on `main`:
   - `freecut-timeline::model::color` (`ColorStack`, `ColorEdit`, `ColorParamId`, `Section`) and
     `edit/ops/color.rs`;
   - `freecut-render::color` (`eval`, `bake`, `lut`, `skin`);
   - `freecut-project` format 10 and later;
   - `freecut-app/src/ui/look*.rs`.
   K1 (stack and Basic), K2 (HSL, Curves, Wheels), K3a (LUT parsers) and K3 (LUT section, library,
   bundled looks, skin protection) are done.

## 2. Rules that keep three parallel branches mergeable

- **Setup:** one worktree per slice, created from the `main` commit the orchestrator names
  (it contains K1–K3):
  ```
  git -C /Users/dimash/PycharmProjects/FreeCut-app worktree add ../FreeCut-k4 -b feature/colour-k4
  git -C /Users/dimash/PycharmProjects/FreeCut-app worktree add ../FreeCut-k5 -b feature/colour-k5
  git -C /Users/dimash/PycharmProjects/FreeCut-app worktree add ../FreeCut-k6 -b feature/colour-k6
  ```
  Work only in your worktree. Never switch branches or edit files in `/Users/dimash/PycharmProjects/FreeCut-app`.
- **Additive edits only** in the shared hot spots: add enum variants at the **end**
  (`ColorEdit`, `ColorParamId`, `PropPath`, `EditRequest`, `OpKind`, `PlanNode`), struct fields at the
  **end** (`ColorStack`), and match arms right after the last existing arm. Never reorder, rename or
  reformat existing code in these files:
  - `freecut-timeline/src/model/color.rs`, `model/keys.rs`, `edit/mod.rs`, `edit/ops/color.rs`, `plan.rs`;
  - `freecut-render/src/color/mod.rs`, `cpu/mod.rs`;
  - `freecut-project/src/migrate.rs` and the format DTOs;
  - `freecut-app/src/app.rs`, `ui/look.rs`, `ui/inspector/mod.rs`, `snapshots.rs`.
- **Put new work in new files** wherever you can:
  - `freecut-render/src/color/stats.rs` (K5) and `freecut-render/src/shape/mask.rs` (K6);
  - `freecut-timeline/src/model/mask.rs` (K6);
  - `freecut-project/src/look.rs` (K4);
  - `freecut-app/src/ui/look_auto.rs`, `look_match.rs`, `look_masks.rs`, `ui/adjust_tab.rs`, `jobs/color.rs`.
- **Format version:** **do not change `CURRENT_FORMAT_VERSION`**. New persisted fields get serde
  defaults so older files load unchanged. Add the DTO fields and a round-trip test, plus a
  `// TODO(merge): format version` next to the place the bump goes. The orchestrator assigns the
  next free number while merging (K5 and K6 each need one; K4 needs none).
- **Golden files** (`tests/golden/*`, the schema-hash lock): change them only if your slice must.
  If you re-bless, say so in your report; the orchestrator re-blesses again after the rebase.
- **Dependencies:** no new crates unless the slice truly needs one. If you add one, pin an existing
  exact version and record its licence.
- **Disk:** each worktree builds in its own `target/`. Iterate with `cargo test -p <crate> <name>`.
  Report the size of your `target/`. Don't create other target directories.
- **Labels** are FreeCut's own wording; bundled looks and LUTs are made by our own code.
- **Never** push, merge or touch other branches.

## 3. Definition of done for each branch

- Spec first: if behaviour is missing from a spec, add a short "build note" to the owning CL entry.
- Every new edit request has an apply → undo → redo test (`Fixture::apply` in
  `crates/freecut-timeline/tests/common`) and is added to the proptest (`tests/random_edits.rs`,
  `PROPTEST_CASES=2000` green).
- Preview equals export, byte for byte, for every new colour feature (test through
  `freecut_render::export::ExportRenderer`).
- Nothing analyses, decodes, renders or does file I/O on the UI thread (jobs only).
- Snapshot scenes for your UI. Render them with
  `FREECUT_SNAPSHOT_DIR=<dir> cargo test -p freecut-app main_window_snapshots`, look at them, and
  list them in the report.
- In your worktree, all of these pass:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build -p freecut-app --no-default-features`
- Commit with clear messages, then report:
  - what is built, with CL IDs;
  - the new public API;
  - every hot-spot file you touched and why;
  - the persisted fields added (for the merge-time version bump);
  - tests added;
  - snapshots;
  - open questions.

## 4. The slices

### K4: adjustment clips, saved looks, Adjustment tab (CL-006, CL-026, CL-044). No format change.
- `EditRequest::AddAdjustmentClip { seq, span: AdjustSpan::{Selection(Vec<ClipId>), AtPlayhead(Tick)}, color: Option<Arc<ColorStack>> }`,
  using CL-006's placement rule. Adjustment clips are attachable, so follow-main-track (TL-029,
  `follow_owners`) applies to them.
- `ColorEdit::ApplySections(Arc<SectionSnapshot>)`: apply chosen sections of a saved look to clips.
- `PlanNode::Adjust(AdjustPlan { clip, track, color, frame_index })`:
  - transform and opacity are ignored, and today's placeholder drawing for adjustment clips is removed;
  - the compositor flushes the layers below, then grades in **canvas space** at output resolution,
    blended by Mix.
- `freecut_project::look::{LookFile, read_look, write_look}` (`.fclook` v1, a library in app data).
  It never stores Auto, Match, Masks, cleanup or Amount.
- App:
  - the **Adjustment** asset tab: *New* (an "Add adjustment" tile creates an adjustment clip at the
    playhead), *Saved* (your looks, with hover preview, drag and `+`), and *LUTs* (the K3 library,
    applied on click to the selection);
  - **Save look…** in the Look lens, with a checklist of sections;
  - the Presets select.
- Tests:
  - only tracks below are affected; a hidden adjustment track does nothing; Mix 0 is identity;
  - the background is graded;
  - stacked adjustments apply bottom-up;
  - a blank adjustment clip renders nothing;
  - placement on an existing vs a new track;
  - the look round trip and version gating;
  - `ApplySections` apply → undo → redo.
- Acceptance: an adjustment clip with a saved look over three clips previews and exports identically.

### K5: Auto adjust and Color match (CL-019 revised, CL-020). Format: new fields `auto`, `match`.
- `AutoCorrection { offsets: AutoOffsets, style, source, strength }` stored as **offsets under the
  sliders**; the user's sliders add on top. `MatchTransform`.
- `ColorParamId::{AutoStrength, MatchIntensity}` and `ColorEdit::{SetAuto, ApplyAutoToSliders, SetMatch}`.
- `ColorStack::effective_basic()`.
- `freecut_render::color::stats::{frame_stats, auto_color, oklab_stats, match_stats}`: pure, `f64`,
  fixed summation order, returning `ColorStatsError`. Also `color::stack_input`.
- `freecut-app/src/jobs/color.rs`: samples 5 exact frames at ≤ 512 px on a worker, with a staleness
  check at commit (a clip edited during the job is skipped).
- UI in the Basic page: Auto adjust (on/off, strength, Re-run, Apply to sliders) and Color match
  (pick a reference clip or the current frame, intensity), with reset and keyframes per CL-028.
- Tests:
  - synthetic colour casts are corrected; a uniform frame gives "Nothing to correct"; re-runs are identical;
  - strength scales;
  - Apply to sliders sums and clamps;
  - self-match is identity;
  - gain limits hold;
  - results are deterministic across thread counts;
  - an edited clip is skipped.
- Acceptance: Auto and Match give visible, undoable, keyframable results that export identically.
- If you need an exact-frame sampling helper in `freecut-media`, keep it to one new function and
  report it.

### K6: colour masks (CL-031, shared core with TR-017/018/019). Format: new field `masks`.
- The shared `freecut_timeline::model::mask::{Mask { shape: MaskShape::{Linear, Band{width}, Ellipse, Rectangle{roundness}}, center, size, linked, rotation, feather, invert }}`,
  using TR's names and fixed point. It is reused later by clip masks (TR-017).
- `ColorMask { id, mask, expand, mode: MaskCombine }`, with `ColorStack.masks` holding at most 8.
- `PropPath::ColorMask { id, param }` for keyframes, and `ColorEdit::{AddMask, RemoveMask, ReorderMask, SetMask, SetMaskMode}`.
- `freecut_render::shape::mask::coverage` and the final `lerp(base, graded, amount × coverage)`.
- App:
  - the **Masks** page: add, remove, select, reorder; a shape picker with our own glyphs; feather,
    invert, expand and combine mode;
  - on-canvas handles on the Stage (`ui::stage`) for position, size, rotation and feather, with one
    undo entry per drag and `Esc` to cancel.
- Tests:
  - coverage goldens per shape; feather is monotonic; invert is the complement; the combine modes;
  - a zero-size mask hides the grade;
  - alpha is never changed;
  - clip masks follow the clip transform; adjustment-clip masks stay in canvas space;
  - "Reset all" names the mask count.
- Acceptance: a grade limited to a keyframed ellipse plays and exports identically.
- K6 and K5 both add `ColorStack` fields and `ColorEdit` variants. Keep yours together at the end of
  each list, under a `// K6` comment, so the rebase is mechanical.
