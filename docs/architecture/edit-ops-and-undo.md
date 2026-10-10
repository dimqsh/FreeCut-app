# Edit operations & undo/redo

> Status: draft · Crates: freecut-timeline (consumers: freecut-app drives gestures, selection and toasts; freecut-render and freecut-audio read published snapshots; freecut-project persists the document and the id counter)

This document designs how every document edit in FreeCut is **planned, validated, applied,
recorded, merged, undone and redone**, plus the snapping algorithm used while editing. It covers
the full timeline op set (insert, overwrite, move, trim with ripple/roll/slip/slide, split,
delete, ripple delete, close gaps, replace, freeze frame, duplicate, paste, group/ungroup,
link/unlink, track add/remove/reorder/lock/mute/hide, speed change) and the shared machinery other
areas (TR, AU, CL, FX, TX, OV, PM, AI) use to put their own edits on the same history.

It does not redefine behaviour owned by the feature specs. Retime math is TR-036, transitions
are FX, persistence is PM. This document says how those behaviours become reversible,
atomic edits.

---

## What FreeCut needs

Distilled from `docs/features/*.md` (feature IDs in brackets) and the paraphrased inventory
(`inventory/timeline-editing.md`, `inventory/workspace-and-shortcuts.md`).

### History semantics

- **N1 One linear history per open project** covering every document change: timeline edits and
  property edits from every other area. Examples: AU's `SetClipGain`, CL's multi-select
  `SetFilter` transactions, TX-038's `ReplaceCaptions`, PM's `ImportMedia`, `SetProjectSettings`,
  `RemoveMedia` and "Use copied media" [TL-043, PM-030, PM-039, PM-058, AU-005, CL-004,
  TX-038].
- **N2 Entry = one user action**, applied atomically as a change set of primitive changes with
  before/after values. Undo applies the inverse; redo reapplies it. A new edit after an undo
  discards the redo branch [TL-043].
- **N3 Transactions and compound ops are one entry.** Examples:
  - a drag with ripple, attachment shifts and auto-created/removed tracks [TL-018, TL-006, TL-029]
  - freeze frame (split + insert + ripple) [TL-031]
  - replace with shortening [TL-035]
  - PM's "Add media" (import plus placement, PM-031), and the first-clip auto-match of
    project settings (PM-006)
  - AI-006 "Sync N clips to beats"
  - every "apply to all" action (inventory §6: "each apply-to-all must be a single undo step")
- **N4 Coalescing.**
  - A continuous gesture (pointer down to pointer up) is one entry: clip drags, inspector
    slider drags, curve-point drags [TL-043, TR shared conventions, AU, CL, TX].
  - Keyboard repeats of the same command on the same target within 1 s also merge: nudge,
    volume steps [TL-019, TL-043].
- **N5 Validation first.** Every op is planned as a pure function returning
  `Result<ChangeSet, EditError>`. If planning fails, the document is untouched, no entry is made,
  and the user gets a toast with the reason. A plan with no effect makes no entry: a drop at the
  original place, a trim to the same values, or a split where nothing can be split
  [TL-043, TL-018, TL-020, TL-023].
- **N6 Limits.** Depth defaults to 200 (preference 20–1 000). A 512 MB memory budget for change
  data drops the oldest entries first, and the newest entry is always kept [TL-043]. Olive uses
  the same depth of 200 (`kMaxUndoCommands`).
- **N7 Selection.** Selection changes are not entries. Undo restores the pre-op selection and
  redo restores the post-op one. The playhead is only clamped [TL-015, TL-043].
- **N8 What is and isn't history.**
  - In history: every sequence flag and every track mute/hide/lock flag [TL-003, TL-008, TL-009].
  - Not in history: view state, preferences (including snapping), range marks, transient
    selection, and background derived data [TL-043, TL-038].
  - Background work that changes the document adds exactly one entry when applied [TL-043,
    PM "Use copied media"].
- **N9 Dirty flag.** Dirty means the current history position differs from the saved position.
  Saving does not clear history, and history is never persisted [TL-043, PM].
- **N10 Edge cases.**
  - Undo or redo during a drag is ignored. During playback it is allowed.
  - Undo must work on offline media.
  - Partial application is impossible by construction.
  - One stack covers nested sequences. Undoing a nested edit highlights the compound clip.
  - Undo restores a document exactly, regardless of the current lock state [TL-043, TL-040, TL-009].
- **N11 Labels.** Every op needs a human-readable name for the Edit menu ("Undo Split clip")
  and for the history panel (TL-044: click an entry to jump there). Inventory §23 confirms every
  operation has a display name.

### Time and invariants

- **N12 Time.**
  - All model time is integer ticks of 1/705,600,000 s [TL-001].
  - Every user-facing edit point is quantized to the sequence frame grid with
    round-half-down.
  - Tick math is checked. Overflow gives `EditError::TimeOverflow` and never panics.
  - Negative times are invalid.
  - Speed is an exact rational, never a float [TR-036, TR-037].
- **N13 Invariants checked after every op** [TL-004, TL-039, TL-040, TL-006]:
  1. Exactly one main track, and it is the lowest video track.
  2. No overlaps within a track.
  3. Clip kind matches track kind.
  4. Strict layering, unless flexible stacking is on.
  5. With the magnet on, the main track is contiguous from 0.

  Also:
  - group and link consistency
  - each nested sequence is referenced by exactly one compound clip
  - no compound cycles, nesting depth ≤ 8
  - ids are stable and never reused, even after undo
  - duration ≥ 1 frame
  - media source ranges lie within the last-known source duration [TL-011]

### Op catalog

| Op (history label) | Spec | Priority |
|---|---|---|
| Insert clips (drop, add, paste, duplicate, source insert) | TL-012, TL-032, TL-033, TL-047 | mvp / v1 / later |
| Overwrite (Alt-drop, source overwrite) | TL-012, TL-018, TL-047 | v1 / later |
| Move clips, nudge, swap | TL-018, TL-019 | mvp / v1 |
| Trim edge (plain), trim to playhead | TL-020, TL-026 | mvp / v1 |
| Ripple trim, ripple across tracks | TL-021 | v1 |
| Roll, slip | TL-022 | later |
| Slide | not specced (asked for by this study); designed as "later" | later |
| Split, split all, blade | TL-023, TL-024 | mvp / v1 |
| Delete, ripple delete | TL-025 | mvp |
| Lift / extract range | TL-038 | v1 |
| Close gap(s) | TL-027 | v1 |
| Replace clip media | TL-035 | v1 |
| Freeze frame | TL-031, TR-044 | v1 |
| Group / ungroup | TL-039 | v1 |
| Detach / restore audio (link/unlink), resync | TL-030 | v1 |
| Add / remove / reorder tracks, rename | TL-006, TL-007, TL-010 | mvp / v1 |
| Track lock / mute / hide | TL-008, TL-009 | v1 |
| Sequence flags (magnet, follow main, flexible stacking, ripple all, markers follow) | TL-003, TL-005, TL-007, TL-021, TL-029, TL-037 | mvp / v1 |
| Speed change (constant, curve, reverse) | TR-037, TR-039, TR-040, TR-043 | v1 |
| Enable/disable, label, markers | TL-036, TL-037, TL-045 | v1 / later |
| Compound create / break apart | TL-040 | v1 |
| Conform to frame grid | TL-001 | mvp |
| Property edits from other areas (transform, color, audio, text…) | TR, CL, AU, FX, TX, OV | mvp+ |

### Cross-cutting policies the ops must apply consistently

- **Magnetic main track** [TL-005].
  - Inserts land at the cut point nearest the pointer: first half of a clip means "before",
    second half means "after", and ties go to the later cut [TL-012].
  - Removing or shortening a main-track clip closes up; lengthening pushes later clips.
  - Trims on the main track are always ripple.
  - Switching the magnet on compacts the track.
- **Auto-created and auto-removed tracks** [TL-006].
  - A collision on an overlay track goes to a new track directly above.
  - At the end of every transaction, every empty, unlocked, non-main track is removed in the
    same step.
  - Limit: 100 tracks.
- **Locks** [TL-009]. Locked tracks refuse edits and stay put during ripples. Attached items on
  locked tracks are flagged out of sync, which is derived state.
- **Ripple across tracks** [TL-021]. Every ripple edit can shift other unlocked tracks. A track
  is exempt if a clip spans the edit point or lies inside the removed span, and exempt tracks
  are reported in a toast.
- **Attachments** [TL-029]. An item is anchored to the main-track owner under its start by a
  source-time offset. It follows the owner on every move, ripple or retime, and is deleted with
  the owner. Collisions go to a new track above.
- **Links and groups** [TL-030, TL-039].
  - Groups don't nest, and a group with fewer than 2 members dissolves.
  - Links are cleared when one side is deleted.
  - Both halves of a split stay in the group and the link.
- **Source limits** [TL-011, TR-036]. Trims are limited by the media's last-known duration
  (offline media included), as mapped through the clip's time map. Stills and generated items
  are unlimited.
- **Snapping** [TL-028].
  - Targets: playhead, clip edges on all tracks except the dragged clips, timeline and clip
    markers, in/out marks, 0 and the sequence end.
  - Threshold 8 px, converted to ticks by the app. The nearest target wins. Ties go to the
    playhead, then markers, then clip edges.
  - Moves test both edges of the dragged block.
  - Snapping is used by move, trim, playhead, blade, markers, range marks and drop placement.
  - Quantization always applies. Hidden and locked clips stay targets. A snap shows one guide
    even if several targets coincide.
- **One planner for ghost and commit** [TL-012, TL-018]. `plan_insert`/`plan_move` are pure and
  called on every pointer move. That is cheap enough only if planning is O(touched data).
- **From the inventory, as open questions.**
  - Clips locked while a background job runs (§18).
  - Trimming several selected clips with one drag (§8).
  - The reference editor never lets its main track become empty (§2). FreeCut's TL-025 deliberately allows an
    empty main track.

---

## How the reference editors do it

All paths are relative to each clone's root. Code was read, not built or run.

### Kdenlive (Qt on MLT) — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL

**Mechanism: undo as composed closures** (`src/undohelper.hpp`, `src/undohelper.cpp`,
`src/macros.hpp`, `src/doc/docundostack.cpp`).
- `Fun` is a `std::function<bool()>`.
- Every model operation has a variant `requestX(..., Fun &undo, Fun &redo)`. It performs the
  change immediately, then appends an *(operation, reverse)* closure pair to the caller's
  accumulated `undo`/`redo`.
- Macros compose the pairs:
  - `UPDATE_UNDO_REDO` makes new undo = reverse-then-old-undo and new redo = old-redo-then-operation.
  - `PUSH_LAMBDA` and `PUSH_FRONT_LAMBDA` append or prepend.
  - `LOCK_IN_LAMBDA` wraps each closure in the model's recursive `QReadWriteLock`, which the
    header describes as protection against concurrent access.
- `PUSH_UNDO` wraps the pair in a `FunctionalUndoCommand`. `QUndoStack::push` calls `redo()`,
  but the work was already done while the closures were built, so that class ignores redo until
  the command has been undone once (the `m_undone` flag).

**Validation is "do it, then roll back".**
- `TimelineModel::requestItemResizeInfo` (`src/timeline2/model/timelinemodel.cpp`) performs a
  trial resize into throw-away lambdas, then runs their undo to learn whether the resize would
  succeed.
- `TimelineModel::requestClipMove` builds `local_undo`/`local_redo` and calls `local_undo()` on
  every failure branch.

**Ops worth noting.**
- **Move** = delete from the old track + insert into the new one (`requestClipMove`).
- **Group move** (`requestGroupMove`) works in two stages: remove every member, then re-insert
  them at their new positions, so members can't collide with each other. Members are sorted in
  the direction of travel. For same-track group moves it first proposes a smaller delta by
  clamping to the nearest free space, instead of failing.
- **Split** (`TimelineFunctions::processClipCut`, `src/timeline2/model/timelinefunctions.cpp`)
  works like this:
  1. Clone the clip.
  2. Shorten the original, so the left half keeps the original id.
  3. Shorten the clone from the left and move it to the cut.
  4. Strip fades on both sides (`cleanFadeEffects`).
  5. Reassign same-track mixes.

  `requestClipCut` sorts the clips to cut by `(position, id)`. The comment there says this is
  for determinism, because the input is an unordered set.
- **Ripple trim** (`requestItemRippleResize`) reuses the spacer tool. When lengthening it first
  pushes the later clips and then resizes. When shortening it resizes first and then pulls.
- **Slip** (`ClipModel::requestSlip`, `src/timeline2/model/clipmodel.cpp`) clamps the offset to
  the available media rather than refusing.
- **Speed** (`requestClipTimeWarp`) removes the clip, swaps in a time-warp producer and
  re-inserts it. Speed is a `double`, compared with `qFuzzyCompare`. The change fails if the
  longer clip no longer fits.
- **Insert/extract zones** (`insertZone`, `extractZoneWithUndo`, `liftZone`, `removeSpace`,
  `requestInsertSpace`): to ripple, they select everything after a point, turn the selection
  into a temporary group, and do a group move.

**Groups** (`src/timeline2/model/groupsmodel.hpp`, `src/definitions.h`).
- Groups form a tree.
- `GroupType` is one of `Normal`, `Selection`, `AVSplit` (the audio/video link) or `Leaf`.
- The current selection is itself a temporary group. That is why `requestClipCut` must clear
  the selection before splitting: its comment warns the group split would otherwise be
  corrupted, since selection groups have no undo support.

**Snapping** (`src/timeline2/model/snapmodel.cpp`, `TimelineModel::getBestSnapPos`, `suggestSnapPoint`).
- The index is a `std::map<int,int>` multiset of positions with reference counts.
- `ignore()`/`unIgnore()` temporarily remove the moving items' own edges (`getBoundaries`).
- `getClosestPoint` uses `lower_bound`. On a tie it returns the later point.
- Move snapping tests every edge of the moving group, but stops at the first candidate within
  1 frame, so it isn't globally nearest.
- Threshold inclusivity is inconsistent: `suggestSnapPoint` uses `<`, `proposeSize` uses `<=`.
- Resize snapping uses MLT's inclusive out points (`in + size - 1`).

**Testing.**
- `TimelineModel::checkConsistency` checks track, clip and group structure. It also checks that
  the snap multiset equals the actual clip edges, i.e. that derived data matches the model.
- `fuzzer/fuzzing.cpp` drives random API calls interleaved with undo/redo and asserts
  consistency. This file is GPL-2.0-only OR GPL-3.0-only OR KDE-Accepted.
- Its findings live on as `tests/regressions.cpp` "FuzzBug1…11", which suggests the
  closure-based undo needed fuzzing to become trustworthy.

**Pitfalls observed in the code.**
1. **Applied but unrecorded change on failure.** In `requestItemRippleResize`, the shrink path
   resizes the clip and then calls the spacer move. If the spacer move fails, the function
   returns `false` without running `local_undo`. The resize stays applied, but it is in no undo
   entry.
2. **Inconsistent rollback ownership.** On failure, `requestClipDeletion` runs the *caller's*
   accumulated `undo()`. `requestClipMove` rolls back only its own local lambdas. A caller that
   also rolls back on failure therefore risks undoing twice.
3. **Lost error results.**
   - `extractZoneWithUndo` overwrites the result of its ungroup step with the result of
     `breakAffectedGroups`.
   - `liftZone` returns `true` whatever its cuts and deletions did.
   - `extractZone` pushes the undo entry even when the operation reported failure.
4. **Opaque history.**
   - Closures hold `shared_ptr`s to deleted clips: `requestClipDeletion`'s reverse
     re-registers the captured clip. That memory is invisible.
   - Entries can't be inspected, measured against a budget, serialized for a repro, or merged.
5. **Preview state lives inside the model.**
   - Drags use separate "fake" code paths (`requestFakeClipMove`, `requestFakeGroupMove`).
   - `requestSpacerEndOperation` first moves the group back to its start position, then
     performs the real move.
   - That function also uses static spacer state, and re-groups items with lambdas that are
     never pushed to the stack.
6. **Policy checks inside primitives.** `ClipModel::requestResize`/`requestSlip` refuse on a
   locked track. Replaying history can therefore depend on policy state as it is now.

**Take for FreeCut:** two-stage multi-clip moves; sorting for determinism; the consistency
checker; fuzzing undo/redo against invariants. **Avoid:** do-then-rollback validation, closures
as history, a selection modelled as a group, preview state in the model, float speed, and
inclusive out points.

### Shotcut (Qt on MLT) — GPL-3.0-or-later

**Mechanism: classic `QUndoCommand` subclasses** (`src/commands/timelinecommands.h/.cpp`).
- There is one command class per action: `Append`, `Insert`, `Overwrite`, `Lift`, `Remove`,
  `Group`/`Ungroup`, `Name`/`Mute`/`Hide`/`Composite`/`LockTrack`, `MoveClip`,
  `TrimClipIn`/`Out`, `Split`, fades, the transition trims, `Add`/`Insert`/`Remove`/`MoveTrack`,
  `DetachAudio`, `Replace`, `AlignClips`, and gain changes.
- **Merging** uses `QUndoCommand::id()`/`mergeWith` with an `UndoId` enum.
  - `TrimClipInCommand::mergeWith` adds the deltas together if the clip and the ripple flags
    match.
  - `MoveClipCommand::mergeWith` refuses to merge moves between tracks. Its comment explains
    why: moving a clip to another track and back would leave an entry with nothing to undo.
    It also refuses when overwrite snapshots exist.
- Many commands are built with `redo = false`. The model was already changed live during the
  drag, so the first `redo()` only records the after-state.

**Undo through `UndoHelper`** (`src/commands/undohelper.h/.cpp`) has two strategies:
- **RestoreTracks:** snapshot the affected tracks as MLT XML and rebuild them verbatim on undo.
- **Per-clip diff by uuid:** a surgical diff of moved, resized, added and removed clips.

Details and limits:
- Its header documents that the diff path can't detect an in-place content change (same uuid,
  index and in/out). Such an undo silently does nothing unless the command opted into
  RestoreTracks.
- If the surviving clips are no longer in their original order, it falls back to the snapshot.
- Clip uuids are private, non-serialized MLT properties. They are promoted to a temporary
  property around XML round trips. `MoveClipCommand::redo` must re-stamp the uuid after
  `moveClip` round-trips a producer through XML.

**Model** (`src/models/multitrackmodel.cpp`).
- Tracks are MLT playlists with blank entries, and positions are implicit.
- Validation is a separate function per op (`trimClipInValid`).
- `trimClipIn` clamps the delta to the blank next to the clip.
- **Ripple all tracks is destructive:**
  - When shortening, other tracks get `removeRegion`, which splits and deletes whatever
    content lies in the region.
  - When lengthening, `insertOrAdjustBlankAt` splits clips that span the point and inserts a
    blank.

**Snapping** (`src/qml/views/timeline/Track.js` `snapClip`/`snapTrimIn`, `Timeline.js`).
- It works in pixel space, with 10 px for moves and 4 px for trims.
- The first match wins, in a fixed order:
  1. origin
  2. other clips on the same track
  3. markers
  4. playhead
- It is not "nearest", and edges on other tracks are ignored.

**Pitfalls observed.**
1. **Index addressing.** Commands store `(trackIndex, clipIndex)`, which is only valid when
   replaying exactly the same state. Uuid lookups were bolted on later.
2. **Non-deterministic redo.** `GroupCommand::redo` allocates a fresh group number on *every*
   redo.
3. **Missing early return.** `MultitrackModel::splitClip` logs an invalid track or clip index
   and then dereferences it anyway.
4. **Partial state on mid-redo returns.** `MoveClipCommand::redo` lifts the clips one by one,
   then places them. If a clip can't be found part-way through lifting (non-ripple), or a
   producer is missing while placing, it logs an error and returns. The clips already lifted
   stay off the timeline.
5. **Silent no-op undo** for in-place content edits on the diff path, as described above.
6. **Destructive ripple-all**, which differs from FreeCut's exemption rule (TL-021).

### Olive — GPL-3.0-or-later

**Mechanism: command objects with explicit inverses** (`app/undo/undocommand.h/.cpp`,
`app/undo/undostack.h/.cpp`).
- `UndoCommand` has `prepare()`, `redo()` and `undo()`. `prepare()` runs lazily on the first
  redo, so the "before" state is captured at execution time.
- A `done_` flag makes redo and undo idempotent.
- `MultiUndoCommand` redoes its children in order and undoes them in reverse.
- `UndoStack`:
  - keeps done and undone lists
  - drops an empty `MultiUndoCommand` on push
  - caps history at 200 (`kMaxUndoCommands`)
  - has `jump(index)` for the history panel
  - saves the project-modified flag per command

**Model.** Each track is a linked list of `Block`s, including explicit `GapBlock`s. Time is a
`rational`; the type comes from the separate olive-core library, which isn't in this shallow
clone.

**Commands** (`app/timeline/timelineundo*.h/.cpp`).
- `TrackRippleRemoveAreaCommand::prepare` classifies the blocks a range touches as:
  - trim-out of the first block
  - whole removals
  - trim-in of the last block
  - a *splice* (split, then trim) when one block contains the whole range

  Gaps are never split unless `SetAllowSplittingGaps` is set.
- `BlockTrimCommand` creates or removes an adjacent `GapBlock`, so trimming never leaves a hole
  in the linked list. A neighbour trimmed to zero length is removed.
- `TrackSlideCommand` grows or shrinks the blocks on either side, creating or removing gaps.
- Removed blocks stay alive through a per-command `memory_manager_` `QObject`.

**Tools** (`app/widget/timelinewidget/tool/*.cpp`).
- Tools assemble `MultiUndoCommand`s: `slip.cpp` adds one `BlockSetMediaInCommand` per ghost,
  and `ripple.cpp` adds one `TrackListRippleToolCommand` per track type plus a
  `SetSelectionsCommand`, so the selection change is part of the entry.
- `PointerTool::ProcessDrag` (`pointer.cpp`):
  1. Validate the movement.
  2. Snap.
  3. Validate again, because snapping can push the movement out of range.
- `ValidateInTrimming`/`ValidateOutTrimming` clamp the movement across *all* ghosts and keep at
  least one timebase unit. Only the first ghost's movement is quantized to the timebase.

**Snapping** (`app/widget/timebased/timebasedwidget.cpp` `AttemptSnap`/`SnapPoint`).
- For every moving reference time (a multi-reference block), it collects candidates within a
  hard-coded 10 px screen range. The source marks that value with a FIXME.
- Candidate sources: playhead, clip in/out, clip markers, sequence markers, work area and
  keyframes.
- The winner minimises |snapped movement − requested movement|. On ties the first collected
  wins, which is the playhead.
- Every reference that lands on a target at the winning movement is reported as a guide.

**Pitfalls.**
- Snapping and ghost validation are linear scans.
- The implicit-gap model forces every trim and slide to create or remove neighbouring gaps.
- Commands mutate in place and rely on exact replay order. Because `prepare()` is lazy, a redo
  against a diverged state is undefined.
- There is no merging.

### OpenTimelineIO — Apache-2.0

**Role.** OTIO is not an editor. `src/opentimelineio/algo/editAlgorithm.h/.cpp` is nonetheless
a useful vocabulary for edit semantics, with diagrams: overwrite, insert, trim, slice, slip,
slide, ripple, roll, fill (3/4-point) and remove.

**Model and time.**
- Tracks are ordered children. Positions are implicit, and gaps are explicit `Gap` items.
- `trim` fills the freed time with a gap.
- `ripple` changes an item's source range, and later items shift implicitly.
- `RationalTime` stores `double` value and rate (`src/opentime/rationalTime.h`). Equality goes
  through rescaling.
- `editAlgorithm.cpp` compares doubles with a fixed absolute epsilon (`isEqual`,
  `double_epsilon`).
- There are no transactions or undo. Functions mutate in place and report an `ErrorStatus`.

**Pitfalls** (checked against `tests/test_editAlgorithm.cpp`).
1. **Partial mutation before an error.** `overwrite` removes the transitions in the range
   first. Only then does it find no item and return `NOT_AN_ITEM`.
2. **Roll moves the downstream edit.** On the out side, `roll` shifts the next item's source
   start but keeps its duration, so the track end moves. `test_edit_roll_3` and `_4` expect
   track lengths of 80 and 65 frames, starting from 70. That contradicts the header's promise:
   "End time in parent of Item after input item will never change".
3. **Roll in-side clamp.** When the previous item is shorter than the requested roll, the code
   subtracts the shortened duration from the offset instead of replacing the offset. That can
   produce a negative duration. The tests only roll ±10 frames against 20–30-frame items.
4. **Ripple in-delta clamp.** It compares `delta_in` with `start_time` instead of checking that
   `start_time + delta_in` stays at or after the available start. With a source start of 20 and
   a delta of +5, the source start jumps to 0. The tests use a start of 5 and deltas of ±10, so
   this is never hit.
5. **Slide adjusts only the previous item.** On an implicit-position track this shifts
   everything after the slid item. It is not the NLE slide, where the next item's head
   compensates.

**Value.** Vocabulary, diagrams and test vectors for insert, overwrite, trim, slice, slip and
fill. Do **not** port `roll`, `ripple` or `slide` as they are.

### MLT — LGPL-2.1-or-later

`src/framework/mlt_playlist.c` provides the playlist primitives Shotcut and Kdenlive sit on:
`mlt_playlist_insert_at` (with an overwrite-blank mode), `split`, `join`, `mix`,
`remove_region`, `resize_clip`, `insert_blank` and blank consolidation.

- Out points are **inclusive**, so every caller writes `length - 1`.
- `insert_at`: when the position falls inside a clip, it inserts *after* the clip if the
  position is past `start + frame_count/2`. This is the same midpoint rule TL-005 uses for drops.
- Playlists with blanks force every non-ripple edit to maintain blank entries, as seen in
  Shotcut's `consolidateBlanks` and `insertOrAdjustBlankAt`.

### libopenshot — LGPL-3.0-or-later

**Mechanism.** The library has no undo stack. Hosts mutate the timeline through
`Timeline::ApplyJsonDiff` (`src/Timeline.cpp`), which takes a list of
`{type: insert|update|delete, key: [path], value}` changes. This is the only reference where
edits arrive as data, so it is the closest in spirit to a change set. The host application's
own undo is not part of this clone.

**Model.**
- Clips carry `float` seconds for position, start and end (`src/ClipBase.h`).
- Overlapping clips on one layer are allowed.
- Clips are sorted by `(layer, position)` (`CompareClips` in `src/Timeline.h`). A comment
  insists on strict-weak ordering, which suggests an earlier sort bug.

**Pitfalls.**
1. **Non-atomic apply.** A list is applied change by change. An exception part-way leaves the
   earlier changes applied and is reported as `InvalidJSON`.
2. **Silent no-op on a diverged state.** `update`/`delete` for an unknown id are silently
   ignored (`apply_json_to_clips`).
3. **Float time leaks into invalidation.** Cache invalidation pads ±8 frames, and the code mixes
   `llround` (`InvalidateCacheForClip`) with truncation (`apply_json_to_clips`) when converting
   seconds to frames.

### Comparison

| Aspect | Kdenlive | Shotcut | Olive | OTIO | libopenshot | **FreeCut (proposed)** |
|---|---|---|---|---|---|---|
| History record | closure pairs built while executing | command + XML track snapshot or uuid diff | command objects, inverse coded per class | none | JSON diff list (app-side) | **data change set from a pure planner** |
| Validation | execute, then roll back | `*Valid()` + clamps | tool clamps ghosts | in place + status | none | **plan on an Arc-shared working copy** |
| Failure atomicity | `local_undo`, with leaks | partial on early `return` | assumes valid | partial | partial | **by construction (swap in result)** |
| Merging | none (fake moves until release) | `id()`/`mergeWith`, delta sums | none | — | — | **gesture/repeat keys + change compaction** |
| Identity | int ids; objects kept alive by closures | indexes + private uuid | object pointers | object pointers | string ids | **persisted u64 ids, monotonic, allocated at plan time** |
| Time | int frames, double speed | int frames, inclusive out | rational | double value/rate | float seconds | **i64 ticks (TL-001), rational speed** |
| Positions | explicit over MLT playlist | implicit + blanks | implicit + gap blocks | implicit + Gap | explicit float | **explicit, sorted, no gap objects** |
| Snapping | refcounted map, early exit | px, first match, same track | px, nearest movement, multi-reference | — | — | **sorted Vec + binary search in ticks, nearest, priority ties, multi-reference** |
| Preview vs commit | separate fake paths | live model mutation | ghost items | — | — | **same planner; `plan.preview()` drawn as ghost** |

### Lessons carried into the design

- **L1** Plan, don't do-then-undo. Kdenlive's trial-and-rollback and both editors' failure
  leaks all come from validation that mutates state. A pure planner over immutable input can't
  leak.
- **L2** History entries must be **data**. Data can be inspected, measured against the
  512 MB budget, merged, logged for fuzz repro, and checked for divergence. Closures (Kdenlive)
  and in-place command objects (Olive) can't.
- **L3** Ids are allocated **once, at planning**, and replayed verbatim. Shotcut's group number
  allocated on each redo is the counter-example. Ids are never reused after undo.
- **L4** Apply must be all-or-nothing. libopenshot's mid-list exception, OTIO's overwrite,
  Shotcut's mid-redo returns and Kdenlive's ripple-resize path all leave partial state.
- **L5** Policy (locks, limits, the magnet) belongs to planning. Apply is structural only, so
  undo is exact whatever the current policy state (TL-009).
- **L6** Multi-clip moves are two-stage: remove all moving clips, then place them. Order and
  sort deterministically; never iterate a hash set.
- **L7** Validate → snap → re-validate (Olive). Clamping happens after snapping.
- **L8** Explicit positions avoid gap bookkeeping (Shotcut blanks, Olive gap blocks, OTIO Gap
  items). Ripple becomes an explicit shift primitive.
- **L9** Half-open ranges everywhere. MLT's inclusive out points spread `-1` through Kdenlive
  and Shotcut.
- **L10** Never use floats for time or speed. OTIO's epsilon, libopenshot's ±8-frame padding
  and Kdenlive's `qFuzzyCompare` speed all show the cost.
- **L11** A consistency checker plus random op sequences with undo/redo finds what unit tests
  miss (Kdenlive's FuzzBug1…11).
- **L12** Keep selection out of the model, but record it on the entry, as Olive's
  `SetSelectionsCommand` does.

---

## Recommended FreeCut design

### Decision: planned change sets over a structurally shared document

Three undo patterns were considered.

| Pattern | Seen in | Verdict for FreeCut |
|---|---|---|
| **Command objects** with coded `do`/`undo` (or closures) | Kdenlive, Shotcut, Olive | Rejected. Every op needs two hand-written code paths that must stay mirror images. Failure handling is ad hoc (L1, L4), and entries are opaque (L2). |
| **Snapshot** (whole state per entry) | Shotcut's RestoreTracks, as a fallback | Viable *only* with structural sharing, and then attractive: undo is a pointer swap. Kept as a **test oracle**. |
| **Inverse by data:** a change set of primitive before/after values, inverted mechanically | libopenshot diffs; TL-043's prescribed model | **Chosen.** One apply routine. Inversion is generic. Memory is proportional to the edit. It can be merged and inspected. |

The chosen design combines inverse-by-data with structural sharing:

1. **Shared model.** The document is `Arc`-structured: sequences, tracks, clips and property
   bags all sit behind `Arc`. Cloning a `Document` is a handful of pointer copies, and
   `Arc::make_mut` copies only the path being modified.
2. **Pure planners.** A planner takes `&Document` and builds a **transaction** (`Tx`) on a
   cheap working clone. Each primitive `Change` is applied to the working copy as it is
   recorded, so later steps of a compound op see earlier ones.
3. **Plan.** The planner returns a `Plan` holding the `ChangeSet`, the post-state `Document`,
   notices, the selection after, and a label. The ghost is drawn from `plan.preview()`.
4. **Commit = swap.** `Editor::commit(plan)` installs the post-state, which is O(1) and can't
   fail, and records the `ChangeSet` in `History`.
5. **Undo/redo apply to a clone, then swap.** Undo applies `changes.inverse()` to a clone of the
   live document and swaps it in only on success. A divergence (a bug) leaves the live document
   untouched; this replaces TL-043's "roll back already-applied primitives".
6. **Lock-free reads.** After every commit, undo or redo, the app publishes the new
   `Arc<Document>` to render and audio threads. Readers never block the UI and never see a
   half-applied edit.

### Module layout (`crates/freecut-timeline/src/`)

```
ids.rs            SeqId, TrackId, ClipId, GroupId, LinkId, MarkerId; IdAlloc, IdCursor
time.rs           Tick, FrameRate, TimeRange, quantize, checked ops        (TL-001, exists in spec)
model/            Document, Sequence, Track, Clip, PropBags               (TL-003/004/011 shapes)
  validate.rs     Document::validate(), validate_touched(&ChangeSet)       (N13)
  locate.rs       per-plan id → location maps (derived, never stored)
edit/
  mod.rs          EditRequest (dispatch enum), plan(), Plan, Label/OpKind, Notice
  change.rs       Change, ChangeSet, inverse, apply, keys, heap_bytes, compact
  tx.rs           Tx: working copy + recorded changes + end-of-tx passes
  ctx.rs          EditCtx (prefs, busy clips, playhead, id base)
  error.rs        EditError, ApplyError, InvariantViolation
  limits.rs       drag_limits(): legal delta ranges for gestures (clamp before planning)
  policy/
    magnet.rs     main-track sequence ops: cut_index, relayout
    ripple.rs     RipplePlan + exemption rule (TL-021)
    attach.rs     follow_owners, reanchor (TL-029)
    place.rs      collision → new track above, track limit (TL-006)
    finish.rs     dissolve groups < 2, clear dangling links, auto-remove empty tracks
  ops/
    insert.rs overwrite.rs move_.rs trim.rs (plain/ripple/roll/slip/slide)
    split.rs delete.rs (delete/ripple/lift/extract/close gaps) replace.rs freeze.rs
    clipboard.rs (paste/duplicate) group.rs link.rs track.rs retime.rs flags.rs
    compound.rs conform.rs markers.rs props.rs (generic property-bag edits for other areas)
history.rs        History, Entry, CoalesceKey, Gesture, saved mark, budget
editor.rs         Editor: the only owner of a mutable Document
snap.rs           SnapIndex, Snap
query.rs          gaps(), cut_points(), clips_in_range(), clips_after()   (read-only helpers)
testkit/          (feature "testkit", dev only) ASCII timeline DSL, proptest strategies, frame oracle
```

`freecut-timeline` depends on nothing internal. It contains no I/O, no threads and no
interior mutability. `Document` is `Send + Sync` because it is all `Arc` and plain data.

### Core types (Rust sketches, ours)

**Ids.** One document-wide, monotonic allocator. Undo never rolls it back, so an id is never
reused (TL-006).

```rust
// ids.rs
macro_rules! id {
    ($($n:ident),* $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $n(u64);
    )*};
}
id!(SeqId, TrackId, ClipId, GroupId, LinkId, MarkerId);

/// Persisted next to the document by freecut-project; lives in `Editor`, not `Document`,
/// so structural equality of documents ignores it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdAlloc { next: u64 }

/// Planning-time cursor. Ghost previews allocate from a cursor and burn nothing;
/// `Editor::commit` advances `IdAlloc` to the plan's high-water mark.
#[derive(Clone, Debug)]
pub struct IdCursor { next: u64 }

impl IdCursor {
    pub fn clip(&mut self) -> Result<ClipId, EditError> { self.bump().map(ClipId) }
    pub fn track(&mut self) -> Result<TrackId, EditError> { self.bump().map(TrackId) }
    fn bump(&mut self) -> Result<u64, EditError> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or(EditError::IdSpaceExhausted)?;
        Ok(id)
    }
}
```

On load, `freecut-project` sets `next = max(stored next, max id in document + 1)`. A
hand-edited file can therefore never cause a reuse. Never derive the next id from the current
maximum at runtime: after an undo the maximum drops, and that leads straight to reuse.

**Shared model shape.** The fields are owned by TL-003/004/011; only the sharing is
prescribed here.

```rust
// model/mod.rs
#[derive(Clone, Debug)]
pub struct Document {
    settings: Arc<ProjectSettings>,
    library: Arc<Library>,                       // PM's media items and bins (freecut-timeline::library)
    sequences: BTreeMap<SeqId, Arc<Sequence>>,
    root: SeqId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub id: SeqId,
    pub flags: SequenceFlags,
    pub visual: Vec<Arc<Track>>,                 // bottom → top; visual[0] is the main track
    pub audio: Vec<Arc<Track>>,                  // top → bottom
    pub markers: Arc<[Marker]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub kind: TrackKind,
    pub header: TrackHeader,                     // name + TrackFlags { muted, hidden, locked }
    clips: Vec<Arc<Clip>>,                       // sorted by (range.start, id); no overlaps
}
// Clip as in TL-011, with every heavy property bag behind Arc so that
// `Clip::clone()` is shallow and ReplaceClip before/after values are cheap.
```

`PartialEq for Document` compares semantic content only. It is the oracle for "undo restores
the original".

**Primitive changes.** These refine TL-043's `Insert`/`Remove`/`Update`. `Replace` is the
update of one entity. `ShiftClips` is a batched update, so a ripple over 10 000 clips is one
small primitive and not 10 000 clip copies.

```rust
// edit/change.rs
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    InsertClip  { seq: SeqId, track: TrackId, clip: Arc<Clip> },
    RemoveClip  { seq: SeqId, track: TrackId, clip: Arc<Clip> },          // full snapshot
    ReplaceClip { seq: SeqId, track: TrackId, before: Arc<Clip>, after: Arc<Clip> }, // same id
    ShiftClips  { seq: SeqId, track: TrackId, ids: Arc<[ClipId]>, delta: Tick },
    InsertTrack { seq: SeqId, slot: TrackSlot, track: Arc<Track> },       // may carry clips
    RemoveTrack { seq: SeqId, slot: TrackSlot, track: Arc<Track> },       // snapshot incl. clips
    MoveTrack   { seq: SeqId, from: TrackSlot, to: TrackSlot },
    SetTrackHeader { seq: SeqId, track: TrackId, before: TrackHeader, after: TrackHeader },
    SetSeqFlags { seq: SeqId, before: SequenceFlags, after: SequenceFlags },
    UpsertMarker { seq: SeqId, before: Option<Marker>, after: Option<Marker> },
    InsertSequence { seq: Arc<Sequence> },
    RemoveSequence { seq: Arc<Sequence> },
    SetSettings { before: Arc<ProjectSettings>, after: Arc<ProjectSettings> },
    Library(LibraryChange),                                               // PM's primitives, same shape
}

impl Change {
    /// Mechanical inverse: no per-op undo code exists anywhere.
    pub fn inverse(&self) -> Change {
        use Change::*;
        match self.clone() {
            InsertClip { seq, track, clip } => RemoveClip { seq, track, clip },
            RemoveClip { seq, track, clip } => InsertClip { seq, track, clip },
            ReplaceClip { seq, track, before, after } => ReplaceClip { seq, track, before: after, after: before },
            ShiftClips { seq, track, ids, delta } => ShiftClips { seq, track, ids, delta: -delta },
            MoveTrack { seq, from, to } => MoveTrack { seq, from: to, to: from },
            // … every other variant swaps before/after or Insert/Remove the same way
            other => other.inverse_rest(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet(Vec<Change>);

impl ChangeSet {
    pub fn inverse(&self) -> impl Iterator<Item = Change> + '_ {
        self.0.iter().rev().map(Change::inverse)
    }
}
```

Writing `delta: -delta` requires `Tick: Neg`. It is `checked_neg` internally, and planners
never emit `i64::MIN`.

**Rules for `apply`.**
- Apply checks **structural preconditions only**:
  - the entity exists or doesn't, as required
  - `before` matches the current value: `Arc::ptr_eq` fast path, then `==`
  - `ShiftClips` targets are present on that track
  - shifted ticks don't overflow
- It never consults locks, the magnet, limits or busy state (L5).
- It doesn't check overlaps per primitive. A compound op may pass through transient overlaps,
  for example shifting before a trim; invariants are checked once, at the end of the
  transaction. Kdenlive has to sort moves by direction to avoid exactly this self-collision.
- Clips stay sorted by `(start, id)`. Ties are broken by id, so the order is deterministic
  even through transient states.

```rust
/// Atomic by construction: work on an Arc-shared clone, swap only on success.
pub(crate) fn apply_all<I: IntoIterator<Item = Change>>(doc: &Document, changes: I)
    -> Result<Document, ApplyError>
{
    let mut next = doc.clone();                  // O(#sequences) pointer copies
    for c in changes { next.apply(&c)?; }        // Arc::make_mut copies only touched paths
    Ok(next)                                     // on Err, `next` is dropped; `doc` untouched
}
```

**Transaction.** Every planner, simple or compound, goes through `Tx`.

```rust
// edit/tx.rs
pub struct Tx<'a> {
    base: &'a Document,
    ctx: &'a EditCtx<'a>,
    work: Document,                    // Arc-shared clone of base; always the "current" state
    changes: Vec<Change>,
    ids: IdCursor,
    notices: Vec<Notice>,              // "Closed 3 gaps", exempt tracks, "Also removed N attached items"
    selection_after: Option<SelectionSnapshot>,
}

impl<'a> Tx<'a> {
    pub fn new(base: &'a Document, ctx: &'a EditCtx<'a>) -> Self {
        Self { base, ctx, work: base.clone(), changes: Vec::new(),
               ids: ctx.id_cursor(), notices: Vec::new(), selection_after: None }
    }
    pub fn work(&self) -> &Document { &self.work }

    /// Records a primitive and applies it to the working copy, so later steps of a
    /// compound op plan against the updated state. Failure here is a planner bug.
    pub fn push(&mut self, c: Change) -> Result<(), EditError> {
        self.work.apply(&c).map_err(EditError::Internal)?;
        self.changes.push(c);
        Ok(())
    }

    pub fn finish(mut self, label: Label) -> Result<Plan, EditError> {
        policy::finish::dissolve_small_groups(&mut self)?;      // TL-039
        policy::finish::clear_dangling_links(&mut self)?;       // TL-030
        policy::finish::auto_remove_empty_tracks(&mut self)?;   // TL-006 (unlocked, non-main)
        self.work.validate_touched(&self.changes).map_err(EditError::Invariant)?; // N13
        Ok(Plan::new(label, ChangeSet(self.changes), self.work, self.ids, self.selection_after, self.notices))
    }
}
```

`validate_touched` costs O(touched tracks), so it runs in **release builds too**. A planner
bug then becomes a refused edit (`EditError::Invariant`, logged) instead of a corrupt
document. Kdenlive's `checkConsistency` only runs in tests.

**Plan, context and errors.**

```rust
// edit/mod.rs
pub struct Plan {
    pub label: Label,                         // OpKind + count; the app localizes ("Split 3 clips")
    pub changes: ChangeSet,
    pub notices: Vec<Notice>,
    pub selection_after: Option<SelectionSnapshot>,
    pub primary_seq: SeqId,                   // undo highlight for nested edits (TL-040)
    result: Document,                         // private: only Editor::commit can install it
    base_revision: u64,                       // stamped by Editor::plan
    id_high_water: u64,
}
impl Plan {
    pub fn is_noop(&self) -> bool { self.changes.is_empty() }
    pub fn preview(&self) -> &Document { &self.result }     // ghost rendering (TL-012/018)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label { pub op: OpKind, pub count: u32 }

pub struct EditCtx<'a> {
    pub prefs: &'a EditPrefs,                 // freeze length, still length, track limit, nesting limit
    pub busy: &'a BTreeSet<ClipId>,           // clips locked by background jobs (inventory §18), app state
    pub playhead: Option<Tick>,               // for split / freeze / paste / trim-to-playhead
    next_id: u64,                             // filled by Editor from IdAlloc
}

// edit/error.rs — thiserror (library crate); the app maps these to toasts via anyhow.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EditError {
    #[error("track is locked")]                     TrackLocked { track: TrackId },
    #[error("main track is locked")]                MainTrackLocked,
    #[error("clip is being processed")]             ClipBusy { clip: ClipId },
    #[error("would overlap another clip")]          Collision { track: TrackId, other: ClipId },
    #[error("would start before 0")]                BeforeZero,
    #[error("clip can't go on this kind of track")] KindMismatch { clip: ClipId, track: TrackId },
    #[error("source media ends here")]              SourceLimit { clip: ClipId },
    #[error("a clip must be at least one frame")]   TooShort { clip: ClipId },
    #[error("track limit reached")]                 TrackLimit { max: u16 },
    #[error("main-track clips must be adjacent")]   NonContiguousMain,
    #[error("a compound clip can't contain itself")] CompoundCycle,
    #[error("nesting too deep")]                    NestingTooDeep { max: u8 },
    #[error("split point is inside a transition")]  InsideTransition { clip: ClipId },
    #[error("time value out of range")]            TimeOverflow,
    #[error("item no longer exists")]               Stale(EntityRef),
    #[error("no more ids")]                         IdSpaceExhausted,
    #[error("internal edit error")]                 Internal(ApplyError),
    #[error("internal invariant violated")]         Invariant(InvariantViolation),
}
```

A no-op is **not** an error. Planners return `Ok(plan)` with `plan.is_noop()`, and commit makes
no entry (N5).

**Requests.** The dispatch enum is serde-serializable, for fuzz repro logs and debug
recordings. There is one typed planner per variant.

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EditRequest {
    Insert(InsertReq), Overwrite(OverwriteReq), Move(MoveReq), Trim(TrimReq),
    Split(SplitReq), Delete(DeleteReq), Range(RangeReq /* lift | extract */), CloseGaps(GapReq),
    Replace(ReplaceReq), Freeze(FreezeReq), Paste(PasteReq), Duplicate(DuplicateReq),
    Group(GroupReq), Ungroup(UngroupReq), DetachAudio(ClipRef), RestoreAudio(ClipRef), Resync(ClipRef),
    AddTrack(AddTrackReq), RemoveTrack(TrackRef), MoveTrack(MoveTrackReq), SetTrackHeader(TrackHeaderReq),
    SetSeqFlag(SeqFlagReq), SetRetime(RetimeReq), SetEnabled(EnableReq),
    CreateCompound(CompoundReq), BreakApart(ClipRef), Conform(SeqId), Props(PropsReq),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Edge { In, Out }
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrimMode { Plain, Ripple, Roll, Slip, Slide }

pub fn plan(doc: &Document, ctx: &EditCtx, req: &EditRequest) -> Result<Plan, EditError> { /* dispatch */ }
```

### Editor and History

`Editor` is the **only** way to mutate a `Document`. `Document` has no public mutators, and
`Plan::result` is private. That makes "every edit goes through the undo stack" a compile-time
property rather than a code-review rule.

```rust
// editor.rs
pub struct Editor {
    doc: Document,
    ids: IdAlloc,
    history: History,
    revision: u64,                 // bumped by commit/undo/redo; background jobs compare it
}

pub enum Coalesce { Never, Gesture(GestureId), Repeat { key: CoalesceKey, now_ms: u64 } }

pub struct CommitMeta {
    pub selection_before: SelectionSnapshot,  // from the app's current selection (TL-015)
    pub coalesce: Coalesce,
}

impl Editor {
    pub fn plan(&self, req: &EditRequest, ctx: EditCtxArgs<'_>) -> Result<Plan, EditError>; // stamps revision + id base

    pub fn commit(&mut self, plan: Plan, meta: CommitMeta) -> Result<Committed, CommitError> {
        if plan.base_revision != self.revision { return Err(CommitError::Stale); } // re-plan
        if plan.is_noop() { return Ok(Committed::NoOp); }
        self.history.admit(&meta.coalesce)?;            // refuses foreign commits while a gesture is open
        let dirty = DirtyRegions::of(&plan.changes);    // time ranges per (seq, track) for caches/UI
        self.ids.advance_to(plan.id_high_water);
        self.doc = plan.result;                         // O(1) swap; cannot fail
        self.revision += 1;
        self.history.record(Entry::new(plan.label, plan.changes, meta.selection_before,
                                       plan.selection_after, plan.primary_seq), meta.coalesce);
        Ok(Committed::Applied { dirty, notices: plan.notices })
    }

    pub fn undo(&mut self) -> Result<Applied, HistoryError> {
        let top = self.history.peek_undo()?;            // Err(Empty | GestureOpen) (N10)
        let next = apply_all(&self.doc, top.changes.inverse()).map_err(HistoryError::Diverged)?;
        self.doc = next;
        self.revision += 1;
        let e = self.history.shift_to_redo();
        Ok(Applied { selection: e.selection_before.clone(), dirty: DirtyRegions::of(&e.changes),
                     highlight: e.primary_seq })
    }
    // redo() mirrors undo() with the forward change set and selection_after.
}
```

A divergence (`HistoryError::Diverged`) is a bug. It is logged, the live document is
untouched, and the history from that point down is cleared. It never panics.

```rust
// history.rs
pub struct History {
    done: VecDeque<Entry>,
    undone: Vec<Entry>,
    base_state: StateId,          // state below done[0]: initial, or after the last evicted entry
    saved: Option<StateId>,       // None = never saved or unreachable
    limits: HistoryLimits,        // max_entries (200), max_bytes (512 MiB)
    bytes: usize,
    gesture: Option<GestureId>,
    last: Option<LastCommit>,     // (entry id, key, time); cleared by undo/redo/save
}

impl History {
    fn current_state(&self) -> StateId { self.done.back().map_or(self.base_state, |e| e.state_after) }
    pub fn is_dirty(&self) -> bool { self.saved != Some(self.current_state()) }
}
```

**Edge rules (each has a test).**
- **New commit.** Clears `undone`. If `saved` pointed into the discarded branch, it becomes
  `None`.
- **Eviction from the bottom** (depth or bytes): `base_state` takes the evicted entry's
  `state_after`. A save point exactly at that state stays reachable by undoing everything. A
  save point below it becomes unreachable, which is correct because it can't be reached any
  more. Without `base_state`, a stack that undoes down to its new bottom would report the wrong
  dirty state. The newest entry is never evicted (N6).
- **`mark_saved()`** sets `saved = current_state()` and **seals** the top entry, so a later
  repeat can't merge into the saved state.
- **Undo/redo while a gesture is open** returns `Err(GestureOpen)`, which the app ignores
  (N10). A commit from another source during a gesture, for example a background job, gets
  `CommitError::GestureOpen`. The app re-plans after the gesture ends; the job's own staleness
  rules (AI-001/AI-010) decide whether to retry.
- **Byte accounting.** `Entry::heap_bytes()` sums `Change::heap_bytes()`. Shared `Arc`s are
  counted once per entry through a pointer set. Over-counting is acceptable because the budget
  is conservative.

### From gesture to history entry

```
pointer/keys ──► app: raw delta (f64 px → Tick at UI edge)
                 │  quantize to frame grid (TL-001)
                 │  snap (SnapIndex built at gesture start; Mod suspends)        ── snap.rs
                 │  clamp to DragLimits computed at gesture start                ── edit/limits.rs
                 ▼
           Editor::plan(req) ──► Tx on Arc-clone ──► Plan { changes, preview, notices }
                 │                   (Err → red ghost + reason; no state touched)
                 ▼
     pointer move: draw plan.preview() as ghost      pointer up: Editor::commit(plan, meta)
                                                                │
                                         History::record (gesture/repeat coalescing)
                                                                │
                                      app publishes Arc<Document> to render/audio
```

**Clamp after snap (L7).** Snapping can move a value outside its legal range, so the clamp
comes last. If the clamp overrides a snap, no guide is drawn.

**Inside-rounding.** Limits that aren't on the frame grid, such as a source end at an
arbitrary tick, are rounded *inward*: floor for an upper limit, ceil for a lower one. An
in-range value can then never quantize out of range. Olive quantizes only the first ghost.

**Drags plan from scratch.** During a drag the planner is called with the *total* delta since
the gesture started, always against the pre-gesture document, and nothing is committed until
release. The drag is therefore one entry with no merging (TL-018: preview updates on release).

**Live gestures merge.** Inspector sliders change the rendered output while dragging, so they
commit each step with `Coalesce::Gesture(id)`, and the steps merge as described under
*Coalescing*.

### The op set

Every op below is a planner `fn plan_x(doc, ctx, req) -> Result<Plan, EditError>` built on `Tx`.
Every refusal maps to an `EditError` variant. The entry label (`OpKind`) matches the op names
the specs use, such as `MoveClips`, `TrimClip` and `SplitClips`. Those spec names are labels;
their fields are realized by the primitives above.

#### Shared policies and the order passes run in

Ops that move main-track clips run their passes in this fixed order. The order prevents
double shifts and makes tests deterministic.

1. **Primary edit** on the target track(s).
2. **Magnet relayout** of the main track if the magnet is on (`policy/magnet.rs`). It returns
   `OwnerMove { owner, old_start, new_start, old_src_in, new_src_in }` per changed owner.
3. **Ripple across tracks**, if the flag is on (`policy/ripple.rs`):
   - It uses `RipplePlan { seq, at, delta, scope }`.
   - It **skips** items attached to a main-track owner, because step 4 moves those. Shifting
     them here as well would double-shift any item attached to a head-trimmed owner.
   - It also skips clips already moved in step 1.
   - **Exemptions (TL-021):**
     - For `delta < 0`, a track is exempt if any clip intersects the removed span
       `[at+delta, at)` or strictly contains `at`.
     - For `delta > 0`, a track is exempt if a clip strictly contains `at`.
     - Locked tracks are always exempt.
     - Clips starting exactly at `at` shift; clips ending exactly at `at` stay.
     - Exempt tracks become a `Notice`.
   - This deliberately differs from Shotcut, whose ripple-all deletes or splits content on the
     other tracks.
4. **Follow owners** (`policy/attach.rs`, TL-029).
   - An attached item's new start is the owner's new start plus the owner's time map applied
     to the anchor's source offset.
   - If the anchor frame was trimmed away, the item re-anchors to the nearest remaining frame.
   - On a collision the item goes to a new track directly above.
   - On a locked track the item stays put; out-of-sync is derived state.
5. **Markers follow ripple**, if that flag is on (TL-037).
6. **`Tx::finish`:** dissolve groups with fewer than 2 members, clear dangling links,
   auto-remove empty unlocked non-main tracks, validate the touched sequences.

**Collision → new track** (`policy/place.rs`):
- Creates a track of the same kind directly above the target.
- Under strict layering it stays inside the right band.
- It refuses with `TrackLimit` at 100 tracks.
- New tracks get ids from the cursor, so redo recreates the same ids.

**Main-track insertion index** (`magnet::cut_index`):
- Inside clip `c`, the pointer means "after" if `2·(t − c.start) ≥ c.duration`, else "before".
  The comparison is done in integers.
- A tie goes "after", matching TL-012's "ties go to the later cut".
- MLT's `insert_at` uses the same midpoint rule but sends the exact midpoint "before".
- Past the end means append. On an empty main track the clip goes at 0 (TL-005).

#### Placement ops

- **Insert** (`InsertClips`; TL-012/032/033/047): drop, add, paste, duplicate, source insert.
  - **Main track with magnet on:**
    1. Find the cut index.
    2. Insert the clips back to back.
    3. Relayout: everything after shifts by the inserted length.
    4. Owners move, and attached items follow.
  - **Overlay track:** place at the snapped, quantized time. If the span is occupied, use
    collision → new track; clips are never silently overwritten.
  - **Source insert (TL-047):**
    1. Split at the playhead on the target track.
    2. Ripple-insert, with ripple-all if enabled.
  - **Multi-item drop:** items go in library order, back to back, as one entry.
  - Refusals: locked target (except drops, which are redirected to a new track above),
    `KindMismatch`, `TrackLimit`, probe not finished (handled by the app, which never plans).
- **Overwrite** (`OverwriteClips`; Alt-drop, TL-047). Clear `[a, b)` on the target track, then
  place the new clip. For each clip `c` that intersects the span:
  - inside the span → `RemoveClip`
  - overlapping the left edge → trim its out to `a`
  - overlapping the right edge → trim its in to `b`; the source in-point moves through the
    time map
  - spanning the whole range → split into a left part, which keeps the id with out = `a`, and
    a right part with a **new id** and in = `b`

  Transitions inside the range are removed through FX's hook, as OTIO does by default. On the
  magnetic main track, the covered span is replaced and the track stays contiguous; an
  overwrite past the end grows it. One more rule: the clear-then-place sequence must be
  planned in one `Tx` so no partial result can escape. OTIO's `overwrite` mutates first and
  errors later.
- **Paste / duplicate** (`InsertClips`; TL-032, TL-033).
  - The clipboard holds `Arc<Clip>` snapshots with relative offsets and the track layout.
  - **Every paste remaps every id**, using an `IdRemap` built from the cursor: clips, groups
    (new `GroupId`s; a pasted group is new, the original is unchanged) and marker ids.
    Attachments and links are not copied (TL-032). Nested sequences are deep-copied with fresh
    `SeqId`s and fresh inner ids.
  - Pasting a compound clip into itself or its descendants → `CompoundCycle`.
  - Duplicate = paste at the selection end on the same tracks.
  - Pitfall: reusing the clipboard's ids on a second paste breaks id uniqueness.

#### Move ops

- **Move** (`MoveClips`; TL-018). This is the two-stage algorithm (L6):
  1. Expand the selection to whole groups.
  2. Compute every target `(track', start')` with relative time and track offsets preserved.
     Track offsets only go to compatible kinds; dragging above the top creates a track.
  3. Check collisions on each target track **with all moving clips removed**, and check the
     moving clips against each other in case two map onto one track.
  4. Emit:
     - same track → `ReplaceClip` with the new range
     - track change → `RemoveClip` + `InsertClip` with the **same id**; all removes first,
       then all inserts

  Multi-clip collisions → `Collision` (red ghost). A single clip uses collision → new track
  (TL-018).
  - **Main track with magnet on:** the selected main members must be contiguous, otherwise
    `NonContiguousMain`. Remove them as a block, find the cut index in the remaining sequence,
    re-insert, relayout. Non-main members shift by the same time delta. Owners move, and
    attached items follow.
  - Dropping at the same cut index is a no-op.
  - Kdenlive clamps a same-track group move to the nearest free space. FreeCut does that
    clamping in `drag_limits`, *before* planning, so the planner itself stays strict.
- **Nudge** (TL-019): `plan_move` with ±1 or ±10 frames, committed with
  `Coalesce::Repeat { key: (Nudge, selection hash) }`. On the magnetic main track,
  `Alt+,/.` is **swap**: a move to the neighbour's cut index.

#### The trim family

`TrimReq { clip, edge, delta, mode }` uses `trim.rs`. Every source-time step goes through TR's
pure functions: `source_time_at`, and the retime trim/split helpers. Speed curves are cut with
Hermite subdivision as TR-036 requires. Trim never maps time itself.

| Mode | Changes | Limits (inclusive, inside-rounded) | Other clips |
|---|---|---|---|
| **Plain** (overlay, or main with magnet off) | In: `start += d`, source in moves by map(d), duration −= d. Out: duration += d | previous neighbour's end / next neighbour's start (or 0); ≥ 1 frame; source `[0, last-known length]` mapped through speed; stills unlimited | none |
| **Ripple** (always on main with magnet on; Shift on overlays) | In: **start stays**, content shifts (source in += map(d)), duration −= d. Out: duration += d | ≥ 1 frame; source limits | everything after the old out shifts by the duration change (`RipplePlan` at the old out); with the magnet it's the relayout; attached items follow |
| **Roll** (TL-022) | A.out += d, B.in += d, B.start += d for an adjacent pair (`A.end == B.start`) | both ≥ 1 frame; A's source end; B's source in ≥ 0; FX transition handles | none, total unchanged; allowed with the magnet on |
| **Slip** (TL-022) | source in and out += s, where `s` is in **source** ticks (the app derives it from the drag through the time map) | `[−src_in, src_len − src_out]`; stills, freezes and generated items → no-op | none |
| **Slide** (not in spec; later) | B.start += d; A.out += d; C.in += d, C.start += d | A ≥ 1 frame and within A's source; C ≥ 1 frame and C's source in ≥ 0; a gap neighbour only needs ≥ 0 length and t ≥ 0 | A and C only, total unchanged. Unlike OTIO's `slide`, which only adjusts A |

Notes:
- **Trim to playhead (TL-026)** is `TrimClip` with `d` derived from the playhead. Several clips
  are one entry, and a clip whose edge is not strictly past the playhead is a no-op.
- **Ripple extending on the left edge** keeps the start: the content reaches earlier into the
  source and later clips move right. This matches TL-020/021.
- **Grouped or linked clips:** only the dragged clip is trimmed (TL-020). Multi-trim
  (inventory §8) is an open question.
- **Equal values** make an empty plan, so no entry (TL-020).

#### Split

**Split / split all / blade** (`SplitClips`; TL-023/024). For each target clip with
`start < t < end`:
- The **left part keeps the id.** The start doesn't move, so attachments anchored before the
  cut, links and selection references stay valid. Kdenlive also keeps the original as the left
  part. Shotcut keeps its uuid on the right part.
- The **right part** gets a new id.
- Source ranges divide through TR's map, so output is unchanged. A property test checks this
  (TR-036).
- Property bags divide through the `SplitAt` trait owned by each area:
  - Fade-in stays on the left and fade-out on the right. Both Shotcut (`splitClip` detaching
    fade filters) and Kdenlive (`cleanFadeEffects`) handle fades this way.
  - Clip markers divide by anchor.
- Both parts keep the group and the `LinkId`. TL-030's "restore removes all pieces" relies on
  the shared link.
- Attachments re-anchor to the part that contains their anchor.
- A compound clip's right part gets a deep copy of the nested sequence (TL-040).
- A split point inside a transition → `InsideTransition` for that clip (TL-023).
- Edge positions are skipped. If nothing splits, the plan is a no-op.
- Targets are sorted by `(track order, start, id)` (L6).

Split all skips locked tracks; an explicit selection on a locked track can't exist (TL-015).

```rust
pub fn plan_split(doc: &Document, ctx: &EditCtx, req: &SplitReq) -> Result<Plan, EditError> {
    let mut tx = Tx::new(doc, ctx);
    let at = quantize(req.at, doc.seq(req.seq)?.rate());
    let mut rights = Vec::new();
    for loc in resolve_split_targets(tx.work(), ctx, req)? {      // groups expanded, sorted, locked skipped
        let clip = tx.work().clip(loc)?.clone();                  // Arc clone
        if !(clip.range.start < at && at < clip.range.end()?) { continue; }  // TL-023 edge rule
        fx::check_split_point(tx.work(), loc, at)?;
        let right_id = tx.ids.clip()?;
        let (left, right) = clip.split_at(at, right_id, &mut tx.ids)?;      // retime + SplitAt bags
        tx.push(Change::ReplaceClip { seq: loc.seq, track: loc.track, before: clip, after: Arc::new(left) })?;
        tx.push(Change::InsertClip { seq: loc.seq, track: loc.track, clip: Arc::new(right) })?;
        attach::reanchor_after_split(&mut tx, loc, right_id, at)?;
        rights.push(right_id);
    }
    tx.select(rights.iter().copied());                            // TL-023: right parts selected
    tx.finish(Label { op: OpKind::SplitClips, count: rights.len() as u32 })
}
```

#### Removal ops

- **Delete / ripple delete** (`DeleteClips`; TL-025).
  - Expand the selection to whole groups, unless a single member was Alt-selected.
  - Add attached items of deleted main-track owners while follow-main is on, with a notice.
  - Snapshot each removed clip in `RemoveClip`.
  - **Ripple closing.** Per track, merge the removed intervals. Each survivor's new start is
    its old start minus the total removed length before it. Emit one `ShiftClips` per segment
    between removed intervals. All arithmetic uses the *original* positions, so there is no
    right-to-left bookkeeping (TL-025 note).
  - **Delete** ripples only the magnetic main track. **Ripple delete** ripples every affected
    track, plus ripple-all with exemptions.
  - Linked partners of deleted clips get their `LinkId` cleared in `finish`. Groups with fewer
    than 2 members dissolve, and emptied tracks are auto-removed.
- **Lift / extract range** (`ExtractRange`; TL-038):
  1. On unlocked tracks, split at the range edges.
  2. Remove the pieces inside.
  3. Extract only: shift everything after the out mark left by the range length on every
     unlocked track.

  Locked tracks with content in the range stay unchanged and are named in a notice. A range
  entirely past the end is a no-op. This is Olive's `TrackRippleRemoveAreaCommand`
  classification (trim-out, removals, trim-in, splice) restated for explicit positions.
- **Close gaps** (`CloseGaps`; TL-027): `query::gaps(track)` returns the gaps, including the
  one from 0 and excluding the tail. Emit `ShiftClips` per segment, plus ripple-all if enabled.
  Switching the magnet on runs the same routine on the main track as part of
  `SetSequenceFlag`.

#### Content ops

- **Replace** (`ReplaceSource`; TL-035).
  - `ReplaceClip` switches the content to the new `MediaRef` with the picker's in-point, clears
    the clip markers, keeps the property bags and clears the link with a notice.
  - If the new media is shorter at the clip's speed, the clip is shortened. That ripples on the
    magnetic main track, with a notice.
  - Kind compatibility per TL-035: video ↔ video and image, audio ↔ audio.
- **Freeze frame** (`FreezeFrame`; TL-031, TR-044), all in one `Tx`:
  1. Split at the playhead, unless the playhead is on the first or last frame, in which case
     the still goes before or after the clip.
  2. Insert a `Still` clip of the freeze length. Its source time `s₀` and baked settings come
     from TR's `freeze_settings_at`. It has no speed or audio.
  3. Ripple by the freeze length: on the main track with the magnet, or on that overlay track,
     plus ripple-all.
  4. Owners move, and attached items follow.
- **Speed change** (`SetRetimeMode`; TR-037/039/040/043).
  - `ReplaceClip` with the new `Retime`. The new duration is `T' = D/r`, rounded to frames as
    TR-036 says.
  - A delta of `T' − T` ripples the clip's own track and the track of its detached linked audio
    (TR-037).
  - On a non-ripple overlay, a longer clip pushes the following clips right by
    `max(0, new_end − next.start)`. It never overwrites them. A shorter clip leaves a gap.
  - Curve-point drags commit with `Coalesce::Gesture` and merge.
  - This replaces Kdenlive's remove/replace/re-insert dance, which fails when the clip no
    longer fits.
- **Enable/disable, label, markers** (TL-036, TL-037, TL-045). `ReplaceClip` or `UpsertMarker`.
  Setting a value the clip already has is a no-op.
- **Property edits from other areas** (`props.rs`). These are generic `ReplaceClip` changes
  with one bag swapped. Each area exposes typed request builders, for example
  `PropsReq::transform(clip, |t| ...)`. Multi-select applies are one `Tx`, and so one entry
  (CL-004, CL-019, inventory §6). Parameter drags (AU-005 volume, TR transform, CL intensity)
  commit with `Coalesce::Gesture`.

#### Group and link ops

- **Group / ungroup** (`Group`, `Ungroup`; TL-039).
  - Membership has a **single source of truth**: `Clip.group`. The map from `GroupId` to
    members is computed per plan, never stored. See the open questions.
  - Grouping takes the union of any existing groups in the selection and creates one new
    `GroupId`, because groups don't nest. It emits a `ReplaceClip` per member.
  - Ungroup clears the field.
  - This avoids Kdenlive's tree of `Selection`/`AVSplit`/`Normal` groups and its consistency
    burden.
- **Detach / restore audio, resync** (`DetachAudio`, `RestoreAudio`; TL-030).
  - **Detach:**
    1. Insert one audio clip per enabled stream with a new shared `LinkId`. Each goes on the
       first free audio track, or a new one.
    2. `ReplaceClip` the video so embedded audio becomes `Detached(link)`.
    3. Move the audio bags across.
  - **Restore** removes every clip that carries the link, after the app's confirmation, and
    reactivates the embedded audio.
  - **Resync** is a move.
  - **Unlink** clears the `LinkId` on both sides.

#### Track and structure ops

- **Tracks** (`AddTrack`, `RemoveTrack`, `MoveTrack`, `SetTrackFlags`, `RenameTrack`;
  TL-006 to TL-010):
  - `InsertTrack` / `RemoveTrack` carry the track snapshot, including its clips. Removing a
    track deletes the attached items of owners on it; groups and links are fixed up by
    `finish`.
  - `MoveTrack` is constrained to its band under strict layering. The main track never moves,
    and dropping a track at its own position is a no-op.
  - `SetTrackHeader` covers lock, mute, hide and rename. Rename trims the name and strips
    control characters.
  - Undoing an auto-removal restores the original id, name, flags and position: the snapshot
    holds all of them.
- **Sequence flags** (`SetSequenceFlag`; TL-003). `SetSeqFlags` plus the content changes in the
  same `Tx`: magnet compaction, attachment build or clear, Normalize order. Toggling the magnet
  while the main track is locked → `MainTrackLocked`.
- **Compound create / break apart** (`CreateCompound`, `BreakApart`; TL-040).
  - Create: `InsertSequence` plus removals plus `InsertClip` of the compound clip.
  - Selected main-track members must be contiguous.
  - The cycle check is a DFS over compound references, and depth must stay ≤ 8.
  - Break apart is the reverse. Content outside the visible source range is trimmed, and
    compound-level bags are dropped with a notice.
  - An edit inside a nested sequence is an ordinary op on that `SeqId`. `primary_seq` drives
    the undo highlight.
- **Conform to frame grid** (TL-001): one `Tx` of `ReplaceClip` that rounds every start and
  end, keeping at least 1 frame.

### Coalescing (op merging)

Merging works at the change-set level, so it is the same for every area. Shotcut instead
needs a hand-written `mergeWith` per command.

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CoalesceKey { pub op: OpKind, pub target: u64 /* stable hash of target ids / property path */ }

impl Entry {
    fn can_absorb(&self, how: &Coalesce, last: Option<&LastCommit>) -> bool {
        !self.sealed && match (how, last) {
            (Coalesce::Gesture(g), _) => self.gesture == Some(*g),
            (Coalesce::Repeat { key, now_ms }, Some(l)) =>
                l.entry == self.id && l.key == *key && now_ms.saturating_sub(l.at_ms) <= 1_000,
            _ => false,
        }
    }
    fn absorb(&mut self, next: Entry) {
        self.changes.extend(next.changes);
        self.changes.compact();
        self.selection_after = next.selection_after;     // keep the original selection_before
    }
}
```

**`ChangeSet::compact()` folding rules** (one O(n) pass, keyed by `EntityKey`):
- `ReplaceClip(x: a→b)` followed by `ReplaceClip(x: b→c)` → `ReplaceClip(x: a→c)`, provided
  no primitive in between touches `x`.
- `ShiftClips(t, ids, d1)` followed by `ShiftClips(t, same ids, d2)` → `ShiftClips(d1 + d2)`,
  on the same condition.
- `InsertClip(x)` … `RemoveClip(x)` with nothing in between touching `x` → both dropped.
- Drop any `Replace` where `before == after`, and any `Shift` with a delta of 0.
- A "barrier" is any primitive that touches the key. `ShiftClips` touches each of its ids.
  Folding across a barrier is never allowed. Collapsing `Replace(x)`, `Shift(x)`, `Replace(x)`
  into a single `Replace` would be wrong.

**When merging is allowed.**
- Only into the **top** of the undo stack.
- Only when that entry was the last thing committed: no undo, redo or save in between
  (`last` is cleared by those).
- Only when the redo stack is empty.
- If the merge results in no changes at all, the entry is **removed**: nudge right then left
  makes no entry. This is the case Shotcut's move-merge refuses to handle; here it is resolved
  by construction.
- Merging into the saved entry makes `saved` unreachable. `mark_saved` seals the top entry to
  avoid exactly that.

**Gestures.**
- `begin_gesture(key)` returns a `GestureId`. Every commit carrying it merges, and
  `end_gesture` seals.
- Pointer-capture loss or `Esc` calls `cancel_gesture`, which applies the inverse of the open
  entry and discards it. A cancelled slider drag leaves no trace.

### Transactions and compound ops across areas

- **Compound planners call sub-planners on the same `Tx`.**
  - `Tx::nest(|tx| plan_split_in(tx, …))` composes; there is no separate transaction stack.
  - Freeze composes split, insert and ripple. Overwrite composes clear and insert. PM's "Add
    media" composes `Library::import` changes and `plan_insert`. AI-006 composes trims and
    moves. Silence removal (AI) composes hundreds of splits and deletes.
  - Each step sees the earlier steps' results through `tx.work()`.
- **Cost.** A compound op over k clips costs O(k · log n + touched-track copies).
- **Off-thread planning for bulk AI ops.**
  - A bulk planner can run on a worker with a snapshot clone. `Document` is `Send + Sync`.
  - The `Plan` comes back stamped with `base_revision`.
  - At commit, a stale revision → `CommitError::Stale`, and AI-001/AI-010 decide: re-plan,
    apply anyway after re-planning, or discard. That gives TX-038's "timeline changed while the
    job ran" a mechanical check.
- **Background results that change the document** (PM-039 "Use copied media", TX-038 auto
  captions) go through `plan` + `commit` like any other edit, so they are exactly one entry (N8).

### Validation and failure atomicity

| Layer | Checks | On failure |
|---|---|---|
| `drag_limits` (gesture start) | legal delta range given neighbours, source limits, ≥ 1 frame, before-zero, track bounds | the app clamps; the user sees the edge stop |
| Planner | policy: locks (`TrackLocked`, `MainTrackLocked`), busy clips, kind compatibility, collisions, contiguity, track/nesting limits, cycles, transition constraints, checked tick math | `Err(EditError)`: red ghost or toast; **nothing changed** |
| `Tx::push` → `Document::apply` | structural preconditions | `EditError::Internal` (planner bug, logged); nothing changed |
| `Tx::finish` → `validate_touched` | N13 invariants on touched sequences | `EditError::Invariant` (bug, logged); nothing changed |
| `Editor::commit` | revision staleness, gesture ownership | `CommitError`; nothing changed |
| `Editor::undo`/`redo` | structural preconditions against the live doc | `HistoryError::Diverged` (bug); live doc unchanged; history below cleared |

- **Overflow.** All tick arithmetic is `checked_*` and returns `TimeOverflow`, never panics. A
  test drives positions near `i64::MAX`.
- **Stale requests.** A request can name an id the user just deleted, for example a queued
  keyboard repeat. That is `EditError::Stale`, a silent refusal with no toast.

### Snapping

`snap.rs` is a pure index, built **once per gesture** and queried on every pointer move.

```rust
/// Declaration order is the tie-break priority (TL-028).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SnapKind { Playhead, Marker, RangeMark, ClipEdge, Bound }

pub struct SnapIndex { pts: Vec<(Tick, SnapKind)> }   // sorted by tick; one entry per tick (best kind kept)

pub struct Snap { pub delta: Tick, pub guides: Vec<Tick> }

impl SnapIndex {
    /// `exclude`: the dragged clips plus everything that moves with the drag
    /// (ripple followers, attached items of dragged owners, both edges of a trimmed clip).
    pub fn build(seq: &Sequence, extra: &[(Tick, SnapKind)], exclude: &BTreeSet<ClipId>) -> Self { /* … */ }

    /// `edges`: reference times of the moving block (block start and end for moves, the edge for
    /// trims, a single time for playhead/blade/markers). `threshold` is in ticks (app converts 8 px).
    pub fn snap(&self, edges: &[Tick], delta: Tick, threshold: u64) -> Option<Snap> {
        let mut best: Option<(u64, SnapKind, usize, Tick)> = None;     // (distance, kind, edge idx, target)
        for (ei, &e) in edges.iter().enumerate() {
            let want = e.checked_add(delta)?;
            let i = self.pts.partition_point(|&(t, _)| t < want);
            for &(t, kind) in self.pts[i.saturating_sub(1)..self.pts.len().min(i + 1)].iter() {
                let dist = t.abs_diff(want);
                if dist <= threshold {
                    let cand = (dist, kind, ei, t);
                    if best.map_or(true, |b| cand < b) { best = Some(cand); }
                }
            }
        }
        let (_, _, ei, target) = best?;
        let delta = target.checked_sub(edges[ei])?;
        let mut guides: Vec<Tick> = edges.iter().filter_map(|&e| e.checked_add(delta))
            .filter(|t| self.contains(*t)).collect();
        guides.sort();
        guides.dedup();                                                   // one guide per coincident time
        Some(Snap { delta, guides })
    }
}
```

Properties, each pinned by a test:
- **Nearest wins.** Ties go by kind priority (playhead > marker > range mark > clip edge >
  bound), then to the earlier reference edge (block start before end), so results are
  deterministic. Kdenlive's early exit within 1 frame and Shotcut's first match aren't nearest.
- **Inclusive threshold, everywhere.** Kdenlive uses `<` for `suggestSnapPoint` and `<=` for
  `proposeSize`.
- **Speed.** Querying is O(edges · log n). The index is rebuilt only at gesture start, or when
  the playhead moves during a drag (it is held fixed). Indexes aren't maintained incrementally,
  so they can't drift from the model; Kdenlive has to verify its incremental snap multiset in
  `checkConsistency`.
- **Hidden and locked clips stay targets** (TL-028). Disabled clips are targets too, since they
  occupy time (TL-036).
- **Quantization.** Every stored edge is on the frame grid, so a snapped delta is
  grid-aligned. Unsnapped deltas are quantized before snapping (pipeline above).
- **Zoom-out.** When many targets fall within the threshold, nearest still wins with a
  deterministic tie-break, so there is no jitter.

### Threading and ownership

- **The UI thread owns `Editor`.** Planning is pure CPU work over in-memory data. It does no
  decode, render or I/O, so it is allowed there.
- **Budget.** Planning a single-clip op on a 10 000-clip sequence should take under 2 ms,
  measured by a benchmark (risk R12).
- **Snapshot publication.** After each commit, undo or redo, `freecut-app` stores
  `Arc::new(editor.doc().clone())` into an `arc_swap::ArcSwap<Document>`.
  - Render and audio threads `load()` the current snapshot per frame or block.
  - Old versions are freed when the last reader drops them.
  - There are no locks in `freecut-timeline`, unlike Kdenlive's `LOCK_IN_LAMBDA` and
    `READ_LOCK` or libopenshot's `getFrameMutex`.
  - An undo during playback takes effect at the next snapshot load (N10).
- **Cache invalidation is computed from data.** `DirtyRegions::of(&ChangeSet)` returns
  `(seq, track, TimeRange)` spans from each primitive's before and after ranges. Kdenlive
  instead emits invalidation signals from inside its lambdas.
- **Bulk planners may run on a worker.** They take an owned snapshot clone and return a `Plan`;
  the commit is revision-checked.

### How this meets the project conventions

- **Rational time.** Ticks are `i64` with checked math, TR's speed is rational, and floats
  appear only in the app's px↔tick conversion.
- **Every edit goes through the undo stack.** `Editor` is the only mutable owner, and
  `Plan::result` is private. Every op is reversible by mechanical inversion and has the
  roundtrip test below.
- **No I/O, decode or render on the UI thread.** `freecut-timeline` does none at all.
- **Errors.** `thiserror` for `EditError`, `ApplyError`, `HistoryError` and `CommitError`;
  `anyhow` only in `freecut-app`. Nothing panics on user input.
- **No `unsafe`** in this crate.

---

## Rust ecosystem

| Crate | Version | License | Use |
|---|---|---|---|
| `thiserror` | 2 (already in workspace) | MIT OR Apache-2.0 | `EditError`, `ApplyError`, `HistoryError`, `CommitError` |
| `serde` | 1 (already) | MIT OR Apache-2.0 | derive on ids, `EditRequest` (fuzz repro logs, debug recordings), clipboard snapshots via freecut-project |
| `proptest` | 1.12.0 (MSRV 1.88) | MIT OR Apache-2.0 | dev-dependency: random documents + op sequences, undo/redo interleavings, shrinking to minimal repros; regressions checked in |
| `insta` | 1.49.0 | Apache-2.0 | dev-dependency: golden snapshots of ASCII-rendered timelines and change sets |
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | in **freecut-app**, not freecut-timeline: lock-free publication of `Arc<Document>` snapshots to render/audio threads |

Conditional, only if profiling shows a need:
- **`imbl` 7.0.2** (MPL-2.0+, GPL-3-compatible). Persistent vectors and maps with O(log n)
  update, for tracks with tens of thousands of clips where `Vec<Arc<Clip>>` copy-on-write
  becomes measurable.
- **`arbitrary` 1.5.0** (MIT OR Apache-2.0) with `cargo-fuzz`, for coverage-guided fuzzing of
  the `EditRequest` interpreter once proptest coverage plateaus.

Avoid:
- **`undo` 0.52.0** (MIT OR Apache-2.0; last release 2025-03). It is a command-pattern library:
  edits implement apply/undo on a target they mutate in place. That gives up plan-as-data,
  shared ghost and commit planning, atomic swap and change-set compaction, the very things the
  references lacked. It adds nothing that `History` (about 300 lines) doesn't cover.
- **`slotmap` 1.1.1** (Zlib). Versioned keys reuse slots and aren't stable across save/load.
  FreeCut needs persisted, never-reused `u64` ids (TL-006).
- **`im`.** Superseded by its maintained fork `imbl`.
- **Float-based time or duration types** (`f64` seconds, `std::time::Duration` for media
  time). Ticks are the only model time (TL-001).
- **`num-rational` for `Tick`.** Ticks are a fixed-timebase integer, not a general rational.
  TR may still use `num-rational` 0.4.2 (MIT OR Apache-2.0) for speed factors, but its
  operators aren't the checked arithmetic FreeCut needs (it offers `Checked*` traits). A small
  in-house `Rational` with `i128` intermediates is TR's call.

---

## Test strategy

All tests live in `freecut-timeline`, with no media and no GPU. Run them with
`cargo test -p freecut-timeline`.

### 1. The roundtrip contract, for every op (TL-043)

A helper that every planner test calls:

```rust
pub fn assert_roundtrip(doc: &Document, ctx: &EditCtx, req: &EditRequest) -> Document {
    let before = doc.clone();
    let plan = plan(doc, ctx, req).expect("plan");          // test code: expect is fine
    assert_eq!(doc, &before, "planning must not mutate its input");
    plan.preview().validate().expect("invariants after apply");
    let after = plan.preview().clone();
    let undone = apply_all(&after, plan.changes.inverse()).expect("undo applies");
    assert_eq!(undone, before, "undo restores the original exactly");
    let redone = apply_all(&undone, plan.changes.0.iter().cloned()).expect("redo applies");
    assert_eq!(redone, after, "redo reproduces the post-state, ids included");
    after
}
```

### 2. Readable fixtures: an ASCII timeline DSL

`testkit::dsl` parses and prints a compact text form. Inputs and expected outputs are both
written in it, and golden comparisons use `insta`. It is far easier to review than OTIO-style
range lists.

```text
rate 30 · magnet on · follow-main on
main : [A 0..30 @0] [B 30..90 @10] [C 90..120 @0]
v2   :          [T 40..70 ^B+10]
a1   : [M 0..200 @0]
# after: RippleDelete { B }
main : [A 0..30 @0] [C 30..60 @0]
v2   :                                   # T removed with its owner (TL-025, TL-029)
a1   : [M 0..200 @0]                     # unattached, not rippled (ripple-all off)
```

`@n` is the source in-point in frames, `^B+10` is an attachment, and `{L}` marks locked
tracks. One fixture file per op family lives under `crates/freecut-timeline/tests/fixtures/`.
At minimum each family covers:
- magnet on and off
- locked tracks, including a locked attached item
- ripple-all with and without an exempt track
- groups, and links (detached audio)
- offline media
- retimed clips: 2× and a speed curve
- a nested sequence

### 3. Property tests (`proptest`)

- **Generators.** Valid random documents: 1–6 tracks, ≤ 12 clips each, mixed kinds and locks,
  random flags, a small library.
- **Requests as selectors.** Requests are built from *selectors* (track index %, clip index %,
  frame offsets) that are resolved against the current document. Most requests are then
  meaningful, as in Kdenlive's fuzzer.
- **Properties:**
  1. Any successful plan leaves `validate()` clean.
  2. A failed plan leaves the document untouched (structural equality).
  3. Undo after any op equals the pre-op document; redo equals the post-op document.
  4. Random interleavings of commit, undo and redo match a **snapshot oracle**: a test-only
     `Vec<Document>` stack that records full snapshots. This differential test checks the
     inverse machinery against the snapshot pattern rejected for production.
  5. A coalesced entry undone in one step equals undoing the parts one by one.
     `compact()` never changes the state an entry produces.
  6. Ids are never reused across any sequence of commits and undos.
  7. Planning is deterministic: the same input gives the same `ChangeSet`.
  8. `ShiftClips` never produces negative starts.
  9. Split never changes rendered output. This uses the frame oracle below and TR's map, as
     TR-036 requires.
- **Regressions.** Shrunk failures go into `proptest-regressions/`, which is checked in. Each
  one also becomes a named regression test, as with Kdenlive's `tests/regressions.cpp`.

### 4. The frame-array oracle for off-by-one bugs

`testkit::oracle` renders a track to `Vec<Option<(ClipId, src_frame)>>` with one cell per
sequence frame. Each op has an obviously-correct frame-level spec. Random small tracks
(≤ 200 frames) are compared between the planner and the oracle.

| Op | Frame-level spec |
|---|---|
| Ripple delete `[a,b)` | `frames.drain(a..b)` |
| Insert at `k` | `frames.splice(k..k, new)` |
| Overwrite `[a,b)` | replace those cells |
| Split | same cells; the right part's ids are renamed |
| Roll | only cells between the old and new cut change, and the length is the same |
| Slip | same ids and positions, source frames offset by `s` |
| Slide | B's cells move; A grows or shrinks at its tail; C shrinks or grows at its head; the length is the same |

This catches the half-open versus inclusive mistakes seen in MLT-based code (L9) and OTIO's
roll and ripple clamps. Several OTIO `editAlgorithm` vectors (Apache-2.0, cited) can be
re-expressed for insert, overwrite, trim, slice, slip and fill. The roll, ripple and slide
vectors can't, because they encode the behaviour shown above.

### 5. History unit tests

- Depth limit, and the memory budget dropping the oldest entries while keeping the newest.
- The saved mark across:
  - eviction (`base_state`)
  - redo-branch discard
  - merging into the saved entry
  - an entry merged to empty
- Undo and redo refused while a gesture is open.
- A foreign commit during a gesture → `GestureOpen`.
- A stale plan → `Stale`.
- Cancelling a gesture leaves no entry.
- Keyboard repeat: 1 000 ms merges; 1 001 ms, or a different target, doesn't.

### 6. Snapping table tests

- Exactly at the threshold, which is inclusive.
- Equidistant targets: priority order, then the earlier edge.
- Coincident targets give one guide.
- Exclusion of dragged clips and ripple followers.
- A snap overridden by a clamp gives no guide.
- Extreme zoom-out: 1 000 targets within the threshold still give the nearest.
- Overflow near `i64::MAX` → `None`.

### 7. Edge-case and overflow tests

- Positions near `i64::MAX` → `TimeOverflow`, no panic.
- 1-frame clips: split skipped, trim refused at minimum length.
- Offline media trims limited to the last-known duration.
- A compound cycle refused.
- Track limit 100.
- Nesting limit 8.
- Removing the main track refused.

### 8. Benchmarks

Dev-only, not a test gate. Planning a move, trim and split on a 10 000-clip, 50-track sequence,
and `SnapIndex::build` and `snap` at that size.

---

## Risks & open questions

1. **Keyframes at a split: the specs conflict.**
   - TL-023 inserts an interpolated boundary keyframe on both sides.
   - `clip-transform-and-speed.md` (Interactions) says a split "keeps all keyframes on both
     halves rather than inserting boundary keyframes (TR-027)".
   - The `SplitAt` trait can do either. TL and TR owners must pick one.
2. **Group membership is stored twice.** TL-039 and TL-011 store both
   `Sequence.groups` and `Clip.group`, "kept consistent". This design keeps only `Clip.group`
   and computes the map, so the two can never disagree; Kdenlive needs a group consistency
   checker for this. The spec text needs that change.
3. **The id counter's scope.** TL-006 and TL-023 say "per-sequence counter". Compound
   create and break-apart move clips between sequences while keeping their ids, so this design
   uses **one document-wide** monotonic counter, kept in `Editor` and persisted by PM.
4. **Which split half keeps the id.** This design keeps it on the left, for stable attachment,
   link and selection references; Kdenlive does the same. TL-023 only requires undo to restore
   the originals' ids. Please confirm.
5. **Track storage.** `Vec<Arc<Clip>>` and `Arc` property bags instead of TL-004's
   `Vec<Clip>`. This is required for cheap ghosts, atomic swap and lock-free snapshots. It is a
   model-notes change for TL-004/011.
6. **Multi-clip trim.** Inventory §8 says several selected clips can be trimmed with one drag.
   TL-020 and TL-039 trim only the dragged clip. If multi-trim is wanted, `TrimReq` takes a list
   and `drag_limits` takes the intersection of the limits, as Olive's
   `ValidateInTrimming` does across ghosts.
7. **Slide isn't specced.** TL-022 has only slip and roll. Slide is designed above as "later";
   its UX (tool, modifier) needs a spec entry.
8. **Busy clips.** Inventory §18 describes clips locked during background jobs. There is no TL
   feature yet. `EditCtx.busy` and `EditError::ClipBusy` are the hook; which ops it blocks
   (split, trim, speed, compound) needs a spec.
9. **Background commits during a gesture.** This design refuses them and the app re-plans
   after the gesture. The alternative, queueing, is subtler with stale revisions. Confirm with
   the AI and PM owners.
10. **Memory accounting.** With shared `Arc`s the 512 MB budget is an estimate that
    over-counts shared data. That is acceptable, but the preference UI shouldn't promise
    precision.
11. **Ripple-all semantics.** FreeCut exempts tracks (TL-021), while Shotcut cuts through other
    tracks' content. User testing may push towards an "also split" option. Keep it out of v1.
12. **Planning on every pointer move.** Very large timelines (AI-generated cuts with more
    than 10 000 clips) could exceed the 2 ms target. Mitigations: cache per-gesture `locate`
    maps and limits, and if needed switch tracks to `imbl::Vector`.
13. **Transitions (FX) on cut points** constrain roll, split, trim, overwrite and extract. The
    planners call FX hooks (`fx::check_split_point`, handle limits). Until FX defines them,
    these are stubs, and FX tests must join the roundtrip suite.
14. **An empty main track.** Inventory §2 says the reference app requires at least one clip on
    the main track. FreeCut's TL-025 deliberately allows an empty main track. That is kept here.
    The note is only so the discrepancy isn't "fixed" by accident.
15. **Licensing of reuse.**
    - Kdenlive's timeline code is GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL. Copying it would
      tie FreeCut's distribution to GPLv3, or to later versions only with KDE e.V.'s
      acceptance. **Reimplement ideas only; port no code.**
    - Shotcut and Olive are GPL-3.0-or-later and MLT and libopenshot are LGPL. All are
      compatible, but nothing there is worth a literal port.
    - OTIO (Apache-2.0) test vectors may be adapted with attribution, except roll, ripple and
      slide.

---

## References

All paths are relative to each reference repository root. Licenses are as declared in each
file or repository.

**Kdenlive** — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL; `fuzzer/` is GPL-2.0-only OR
GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL.
- `src/undohelper.hpp`, `src/undohelper.cpp`: `Fun`, `PUSH_LAMBDA`, `FunctionalUndoCommand`.
- `src/macros.hpp`: `UPDATE_UNDO_REDO`, `LOCK_IN_LAMBDA`, `PUSH_UNDO`, `READ_LOCK`.
- `src/doc/docundostack.hpp`, `src/doc/docundostack.cpp`.
- `src/definitions.h`: `GroupType`.
- `src/timeline2/model/timelinemodel.cpp`: `requestClipMove`, `requestGroupMove`,
  `requestItemResize`, `requestItemResizeInfo`, `requestItemRippleResize`, `requestClipSlip`,
  `requestClipDeletion`, `requestClipTimeWarp`, `getBestSnapPos`, `suggestSnapPoint`,
  `getBoundaries`, `deregisterClip_lambda`, `checkConsistency`.
- `src/timeline2/model/timelinefunctions.cpp`: `processClipCut`, `requestClipCut`,
  `requestClipCutAll`, `extractZone`, `extractZoneWithUndo`, `insertZone`, `liftZone`,
  `removeSpace`, `requestInsertSpace`, `requestSpacerEndOperation`.
- `src/timeline2/model/clipmodel.cpp`: `requestResize`, `requestSlip`.
- `src/timeline2/model/snapmodel.hpp`, `src/timeline2/model/snapmodel.cpp`.
- `src/timeline2/model/groupsmodel.hpp`.
- `tests/regressions.cpp`, `fuzzer/fuzzing.cpp`.

**Shotcut** — GPL-3.0-or-later.
- `src/commands/undohelper.h`, `src/commands/undohelper.cpp`.
- `src/commands/timelinecommands.h`, `src/commands/timelinecommands.cpp`: `MoveClipCommand`
  (`redo`, `undo`, `mergeWith`), `TrimClipInCommand`, `GroupCommand`, `SplitCommand`.
- `src/models/multitrackmodel.cpp`: `trimClipInValid`, `trimClipIn`, `splitClip`,
  `removeRegion`, `insertOrAdjustBlankAt`, `moveClip`.
- `src/qml/views/timeline/Track.js` (`snapClip`, `snapTrimIn`),
  `src/qml/views/timeline/Timeline.js`.
- `src/shotcut_mlt_properties.h`: `kShotcutGroupProperty`, `kUuidProperty`.

**Olive** — GPL-3.0-or-later.
- `app/undo/undocommand.h`, `app/undo/undocommand.cpp`, `app/undo/undostack.h`,
  `app/undo/undostack.cpp`.
- `app/timeline/timelineundoripple.h`, `app/timeline/timelineundoripple.cpp`.
- `app/timeline/timelineundopointer.h`, `app/timeline/timelineundopointer.cpp`.
- `app/timeline/timelineundosplit.h`, `app/timeline/timelineundogeneral.h`,
  `app/timeline/timelineundotrack.h`.
- `app/widget/timelinewidget/tool/pointer.cpp`: `ProcessDrag`, `ValidateInTrimming`,
  `ValidateOutTrimming`.
- `app/widget/timelinewidget/tool/ripple.cpp`, `rolling.cpp`, `slide.cpp`, `slip.cpp`.
- `app/widget/timebased/timebasedwidget.cpp`, `app/widget/timebased/timebasedwidget.h`:
  `AttemptSnap`, `SnapPoint`.
- `tests/timeline/timeline-tests.cpp`.

**OpenTimelineIO** — Apache-2.0.
- `src/opentimelineio/algo/editAlgorithm.h`, `src/opentimelineio/algo/editAlgorithm.cpp`:
  `overwrite`, `insert`, `trim`, `slice`, `slip`, `slide`, `ripple`, `roll`, `fill`, `remove`.
- `tests/test_editAlgorithm.cpp`: `test_edit_ripple_*`, `test_edit_roll_*`, `test_edit_trim_3`.
- `src/opentime/rationalTime.h`.

**MLT** — LGPL-2.1-or-later.
- `src/framework/mlt_playlist.c`: `mlt_playlist_insert_at`, `mlt_playlist_split`,
  `mlt_playlist_remove_region`, `mlt_playlist_resize_clip`.

**libopenshot** — LGPL-3.0-or-later.
- `src/Timeline.cpp`: `ApplyJsonDiff`, `apply_json_to_clips`, `InvalidateCacheForClip`.
- `src/Timeline.h`: `CompareClips`.
- `src/ClipBase.h`.

**FreeCut specs used.**
- `docs/features/timeline-editing.md`: TL-001…TL-047, especially TL-043.
- `docs/features/clip-transform-and-speed.md`: TR-036, TR-037, TR-040, TR-044, and the shared
  undo conventions.
- `docs/features/project-and-media.md`: PM compound ops, saved position, history not persisted.
- `docs/features/ai-tools.md`: AI-006 compound op, `freecut-timeline::ops`.
- `docs/features/text-and-captions.md`: TX-038.
- `docs/features/audio.md`, `docs/features/color-and-filters.md`: coalesced parameter drags,
  multi-select transactions.
- Inventory notes: `inventory/timeline-editing.md` §2, §6, §8, §18, §23.
