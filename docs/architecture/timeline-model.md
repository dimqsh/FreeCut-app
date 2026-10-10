# Timeline data model & rational time

> Status: draft · Crates: freecut-timeline

This document fixes how `freecut-timeline` represents time, sequences, tracks, clips,
transitions, markers, groups, links, attachments and nested sequences, and how every edit
becomes an exact, invertible change set. It turns the behavior in
`docs/features/timeline-editing.md` (TL-xxx) and the timeline-facing parts of
`effects-transitions-animations.md` (FX), `clip-transform-and-speed.md` (TR), `audio.md` (AU),
`text-and-captions.md` (TX) and `project-and-media.md` (PM) into Rust types and invariants.
Where this design deviates from a spec's "Model notes", it says so and lists the point under
[Risks & open questions](#risks--open-questions) so the spec can be updated.

---

## What FreeCut needs

Distilled from the specs (feature IDs in brackets) and the paraphrased behavior inventory.

### Time

| # | Requirement | Source |
|---|---|---|
| R1 | One integer time unit for every model value: `Tick` = 1/705 600 000 s in `i64`. Every common frame duration and audio sample period is a whole number of ticks. | TL-001, PM-007, AU (sample positions) |
| R2 | Checked arithmetic. Overflow is an `EditError`, never a panic or a silent wrap. | TL-001, PM-021 |
| R3 | Edit points snap to the sequence frame grid: nearest frame, ties go to the earlier frame. | TL-001 |
| R4 | A frame rate is accepted only if its frame duration is a whole number of ticks. A custom rate that fails (11 fps, for example) is refused, and the nearest valid rates are suggested. | TL-001, PM-005 |
| R5 | Container timestamps (1/90000, 1/15360, …) and caption cue times convert to ticks by exact rational multiplication, rounding to the nearest tick with ties to even. | TL-001, TX (subtitle import) |
| R6 | Ratios such as speed are exact rationals, multiplied with `i128` intermediates. | TR-036, TR-037 |
| R7 | `HH:MM:SS:FF` helpers live in the timeline crate. Hours never wrap. Drop-frame display arrives in v1. | TL-002 |
| R8 | Changing the frame rate keeps absolute tick positions. An optional, undoable **Conform to frame grid** step rounds them to the new grid. | TL-001, PM-011 |
| R9 | No floats in stored time. Floats exist only at the UI edge. | PM-021, TL-001 |

### Structure

| # | Requirement | Source |
|---|---|---|
| R10 | One root sequence per project. Further sequences exist only as the contents of compound clips. | TL-003, TL-040 |
| R11 | Track kinds `Video`, `Text`, `Sticker`, `Effect`, `Adjustment`, `Audio`, plus `Caption` (requested by TX). Visual tracks are ordered bottom-to-top and audio tracks are kept separately. There is exactly one **main** track, a `Video` track that is always the lowest visual track. Strict layering bands apply unless flexible stacking is on. At most 100 tracks per sequence. | TL-004, TL-007, TX-025 |
| R12 | A track is a single lane: clips on it never overlap. Sequence duration is the latest clip end, including hidden, muted and disabled clips. | TL-003, TL-004 |
| R13 | Track flags (mute, hide, lock) and names are undoable. Empty, unlocked, non-main tracks are removed automatically in the same undo step; caption tracks are exempt. Undo restores a removed track with its original id and position. | TL-006, TL-008, TL-009, TX-025 |

### Clips

| # | Requirement | Source |
|---|---|---|
| R14 | A clip has a timeline placement and content. Content is one of: media with a source range, generated content (no source), a still, or a compound clip with a source range into its nested sequence. A clip also carries an enabled flag, an optional name, markers, a group, an attachment, a link and typed property bags. | TL-011 |
| R15 | Timeline length and source length are related by the TR time map. A speed change alters the timeline length, never the source length. Clips are at least 1 frame long. Offline media keeps its last known duration. | TL-011, TR-036, TR-037 |
| R16 | Embedded audio state (`Active` / `Detached(LinkId)` / `None`). Freeze-frame still sources. Replace media. Enable/disable. | TL-030, TL-031, TL-035, TL-036 |

### Edits

| # | Requirement | Source |
|---|---|---|
| R17 | Magnetic main track: while on, it is contiguous from 0. Overlay tracks are non-ripple. Ripple-all has exemptions. Attached items follow their owners. Locked tracks never move, and their items are flagged out of sync instead. | TL-005, TL-009, TL-021, TL-029 |
| R18 | Every edit is planned by a pure function returning `Result<ChangeSet, EditError>`. The ghost preview and the commit run the same code. Planning failure leaves the document untouched. | TL-012, TL-018, TL-043 |
| R19 | Gaps are derived and selectable. The space before the first clip counts as a gap; the space after the last clip does not. | TL-027 |
| R20 | Splits create no zero-length parts and are refused inside a transition. Compound clips deep-copy on split and duplicate. | TL-023, TL-033, FX-002 |

### Transitions

| # | Requirement | Source |
|---|---|---|
| R21 | At most one transition per cut, only between butted clips on the same track. Default **overlap placement**: the incoming clip moves `D` earlier and the track shortens by `D`, with no extra footage needed. Removing the transition ripples back by `D`. | FX-002 |
| R22 | Later: **handle placement**, where nothing moves and the transition uses media beyond the cut, aligned centered, ending at the cut, or starting at the cut. Audio crossfades on audio tracks use the same geometry. | FX-008, AU-029 |
| R23 | When a neighbour changes, the transition is clamped to its cap, or removed if the cap falls below the minimum. Either result is recorded in the same undo step. Embedded audio crossfades over the overlap. | FX-002, FX-007 |

### Markers, groups, nesting, history, interop

| # | Requirement | Source |
|---|---|---|
| R24 | Timeline markers live in sequence time. Clip markers live in source time, so they follow trims and divide on split. Markers can be points or ranges, and carry a palette color and an origin (user or beat). | TL-037 |
| R25 | Groups are flat, a clip belongs to at most one, and a group with fewer than 2 members dissolves. | TL-039 |
| R26 | Each nested sequence is referenced by exactly one compound clip. Nesting depth is at most 8. Cycles are impossible. | TL-040 |
| R27 | One linear history of primitive before/after changes. Apply is atomic, every change has an inverse, entries can be coalesced, and history has a 200-entry / 512 MB budget. Ids are never reused after undo. | TL-006, TL-043 |
| R28 | The model maps cleanly to OpenTimelineIO tracks, clips, gaps, transitions and markers for later import and export. | PM-103 |

**Inventory notes that matter here.** The reference editor caps video track count and nesting
depth, and refuses splits shorter than a minimum length. It also blocks edits to clips that a
background job is processing (inventory `timeline-editing.md` §1, §9, §13, §18). FreeCut covers the
first three with explicit limits. Background-job locks are transient app state: they are not
part of the model and are not stored. The timeline-editing spec rules out per-clip locking, so
the app checks job locks before it calls a planner.

---

## How the reference editors do it

All paths are relative to each project's repository root.

### OpenTimelineIO (Apache-2.0)

**Key files:** `src/opentime/rationalTime.h`, `src/opentime/rationalTime.cpp`,
`src/opentime/timeRange.h`, `src/opentimelineio/{item,composition,track,stack,transition,clip,gap,marker,timeline,typeRegistry}.cpp`,
`src/opentimelineio/algo/editAlgorithm.{h,cpp}`, `docs/tutorials/time-ranges.md`.

**Time representation.**
- `RationalTime` holds `double _value, _rate` (`src/opentime/rationalTime.h:408`).
- Addition rescales to the larger of the two rates.
- Ordering divides `value / rate` (`rationalTime.h:350-357`).
- Equality compares rescaled doubles exactly (`rationalTime.h:384`).
- `from_frames` and `to_frames` both **truncate** through `int(...)` (`rationalTime.h:188`, `:229`). For negative values that truncates toward zero, not down.
- NTSC rates are stored as the doubles `24000.0/1001.0` etc. (`rationalTime.cpp:16-37`), so they are not exact.
- `TimeRange` is `{start, duration}` with an exclusive end. Its relation predicates (`before`, `meets`, `overlaps`, `contains` …) compare seconds against an epsilon of `1/(2·192000)` s (`src/opentime/timeRange.h:13-19`, `:159-300`).
- Timecode formatting snaps any rate within 0.1 of an SMPTE rate to that rate, and wraps at 24 h (`rationalTime.cpp:455-470`, `:519-530`). TL-002 forbids wrapping.

**Schema.**
- Structure: `Timeline → Stack → Track(kind "Video"/"Audio") → [Clip | Gap | Transition | Stack]` (`src/opentimelineio/track.h:18-22`).
- Positions are **derived**. `Track::range_of_child_at_index` sums the durations of the preceding non-overlapping children on every call (`track.cpp:52-92`). A `Stack` puts all of its children at 0.
- Gaps are explicit items.
- `Composition::child_at_time` bisects, but first rebuilds `range_of_all_children()` on each call (`composition.cpp:497-510`). Derived positions are O(n) per query unless they are cached.
- A clip has four ranges (`docs/tutorials/time-ranges.md`):
  - `available_range` comes from its media reference.
  - `source_range` is the trim.
  - `trimmed_range` is the source range, or the available range if none is set.
  - `visible_range` is the trimmed range plus the neighbouring transitions' offsets (`item.cpp:61-85`, `track.cpp:151-165`).
- Since schema `Clip.2`, a clip holds several media references keyed by name, plus an active key (`typeRegistry.cpp:159-178`).
- **Transitions** do not consume track time. `overlapping()` returns true (`transition.cpp:27-30`). The duration is `in_offset + out_offset` (`transition.cpp:53-55`). The window is `[cut − in_offset, cut + out_offset)`, and both clips borrow media beyond their trims (handles) to fill it.
- Markers can attach to any item, in that item's own time frame. Duration 0 means a point (`time-ranges.md`). Marker color changed from string names to a `Color` type in a schema upgrade (`typeRegistry.cpp:98-120`).
- `Composition` refuses a child that already has a parent (`CHILD_ALREADY_PARENTED`, `composition.cpp:65`, `:91`). That is single ownership, as Rust gives us for free.
- Speed is a separate `LinearTimeWarp` effect whose `time_scalar` "does not affect the duration" of the item (`docs/tutorials/otio-serialized-schema.md:360-363`). `Item::transformed_time` ignores time effects (`item.cpp:120-169`). The result is two independent sources of truth for clip length.
- Each schema type has its own version number, with registered upgrade **and downgrade** functions (`typeRegistry.h:101-135`, `CORE_VERSION_MAP.cpp`).

**Edit algorithms.** `algo/editAlgorithm.h` documents overwrite, insert, trim, slice, slip,
slide, ripple, roll, fill and remove with ASCII diagrams. Overwrite and insert can remove
transitions that intersect the edit, and slice refuses or removes transitions at the cut
(`editAlgorithm.cpp:106-130`, `:486-540`). Its Apache-2.0 license is GPL-3-compatible, so the
semantics (not the code) are a good checklist for our `plan_*` functions.

**Pitfalls to avoid:**
- f64 time (inexact NTSC rates, epsilon comparisons, truncating frame conversion).
- Wrap-around timecode.
- O(n) derived positions.
- Speed stored independently of duration.

**Worth keeping:**
- Half-open ranges.
- The four named clip ranges.
- Cut-anchored transitions with `in_offset`/`out_offset`.
- Single ownership of children.

### Kdenlive (GPL-3.0-only OR KDE-accepted GPL-3)

**Key files:** `src/timeline2/model/{timelinemodel,trackmodel,clipmodel,groupsmodel,snapmodel}.{hpp,cpp}`,
`src/undohelper.hpp`, `src/utils/gentime.{h,cpp}`, `src/definitions.h`,
`src/project/projectmanager.cpp`, `src/doc/kdenlivedoc.{h,cpp}`.

**Time.**
- Positions are integer frames, because the model sits on MLT.
- The legacy `GenTime` is a `double` in seconds whose comparisons use `s_delta = 0.00001` (`src/utils/gentime.cpp:9`). Such comparisons are not transitive.
- `GenTime::frames` rounds with `floor(x·fps + 0.5)` (`gentime.cpp:38`).
- When the project fps changes, every stored frame position (guides, for example) is rewritten through a `double` ratio and `qRound` (`src/project/projectmanager.cpp:1961-1991`). Storing positions in frames forces that rewrite.

**Tracks and same-track transitions ("mixes").**
- Each Kdenlive track is an MLT tractor with **two playlists**, so that clips can overlap for same-track transitions (`trackmodel.hpp:40-44`, `:366-371`).
- A mix is `MixInfo {firstClipId, secondClipId, firstClipInOut, secondClipInOut, mixOffset}` (`trackmodel.hpp:26-38`). Here `mixOffset` is the cut position inside the overlap.
- `requestClipMix` extends both clips into their handles, clamps the mix against neighbouring mixes, and when a chain of mixes needs it, moves clips between the two playlists (`trackmodel.cpp:1877-1960`).
- Missing handles are a user-facing failure: "Not enough frames at clip start/end to apply the mix" (`timelinemodel.cpp:1305`).
- Clips are stored in `std::map<int, shared_ptr<ClipModel>>` keyed by id. Position queries go through the MLT playlist (`trackmodel.hpp:373-376`).

**Ids and groups.**
- A single static `int` counter hands out ids for clips, tracks, compositions and groups (`src/doc/kdenlivedoc.cpp:73`, `timelinemodel.cpp:5886-5889`). The ids exist only at runtime.
- Groups therefore cannot be saved by id. They are serialized as `"track:position:subLayer"` strings and re-resolved on load (`groupsmodel.cpp:749-770`, `:817-835`), which is fragile under any position change.
- Groups form a tree of four types: `Normal`, `Selection`, `AVSplit` and `Leaf` (`src/definitions.h:47-52`). Selection and linked A/V are both modeled as kinds of group.

**Undo.**
- Each operation composes undo and redo closures (`Fun = std::function<bool()>`) with `PUSH_LAMBDA` (`src/undohelper.hpp`).
- If a step fails part-way, the operation calls `local_undo()` and asserts it succeeded (about 30 sites in `timelinemodel.cpp`).
- `FunctionalUndoCommand` needs a flag so that Qt does not run `redo()` a second time (`undohelper.hpp`).
- Closures are opaque: they can't be inspected, measured for memory, or compared in tests.

**Drag preview mutates the live model.** `suggestClipMove` calls `requestClipMove` on the real
model while the user drags (`timelinemodel.cpp:1587-1750`, for example `:1719`, `:1739`).
`requestClipMove` takes eight parameters, five of them boolean flags (`timelinemodel.cpp:1400`).

**Speed.** `double m_speed` (`clipmodel.hpp:291`). Every speed change remaps in/out points with
`qRound64` (`clipmodel.cpp:876-887`, `:1075-1102`), so repeated changes drift by up to a frame
each time.

**Good ideas:**
- `TimelineModel::checkConsistency` and `TrackModel::checkConsistency` (`timelinemodel.cpp:6732`, `trackmodel.cpp:997`) check blank/clip agreement and mix registrations, and the test suite calls them after edits.
- The snap model is a reference-counted `std::map<int,int>` searched with `lower_bound` (`snapmodel.cpp:13-52`).

**Threading.** The model is guarded by a `QReadWriteLock` (`trackmodel.hpp` `m_lock`; `QWriteLocker`
in `requestClipMove`), because MLT render threads read the same structures.

The license is GPL-3-compatible. We take ideas only, no code.

### Shotcut (GPL-3.0-or-later) on MLT (LGPL-2.1-or-later)

**Key files:** Shotcut `src/models/multitrackmodel.cpp`, `src/commands/timelinecommands.h`,
`src/commands/undohelper.h`. MLT `src/framework/mlt_types.h`, `mlt_producer.c`,
`mlt_playlist.c`, `mlt_property.c`, `mlt_profile.h`.

**MLT time.**
- `mlt_position` is an `int32_t` frame count (`mlt_types.h:252-256`), and the profile stores the rate as `int` num/den (`mlt_profile.h:38-39`).
- **Out points are inclusive**: `playtime = out − in + 1` (`mlt_producer.c:576-579`).
- `mlt_producer_set_in_and_out` silently clamps out-of-range values and swaps reversed in/out (`mlt_producer.c:488-523`). Bugs upstream are hidden instead of reported.
- Time-string parsing mixes `floor`, `ceil` and `lrint` for different fields (`mlt_property.c:452`, `:496`).

**Playlists.**
- A track is a playlist of entries in which blanks are explicit. MLT provides `mlt_playlist_consolidate_blanks` to merge adjacent blanks (`mlt_playlist.c:1699`). Shotcut calls its `consolidateBlanks` helper after edits (16 call sites in `multitrackmodel.cpp`).
- Blank lengths are passed as an inclusive out point, `mlt_playlist_insert_blank(self, clip, out)` (`mlt_playlist.c:1790`). Shotcut therefore writes `insert_blank(index, length - 1)` throughout (`multitrackmodel.cpp:1093`, `:1447`, `:1931`).
- Same-track transitions use `mlt_playlist_mix` (`mlt_playlist.c:1145-1240`). It cuts A's tail and B's head into a two-track tractor and inserts that between them as a playlist item. The track shortens by the mix length, which is the same user-visible result as FreeCut's overlap placement.
- The mix length is clamped against the *longer* of the two clips (`mlt_playlist.c:1162-1164`). For the shorter clip it relies on the producer's silent clamping.
- Shotcut attaches both a video dissolve and an audio crossfade to each mix (`multitrackmodel.cpp:2474-2490`), and detects a transition by a property on the producer (`multitrackmodel.cpp:4006-4013`).

**Edit API.** Every edit has a separate validity check (`trimClipInValid` / `trimClipIn`,
`addTransitionValid` / `addTransition`). The check and the edit can disagree because they are
separate code.

**Undo.**
- Commands address clips by index plus `QUuid` (`timelinecommands.h`).
- `UndoHelper` records before/after state as XML and diffs it.
- Its own header warns that the fine-grained path cannot detect an in-place content change with the same uuid, index and in/out. A command that mutates content without a structural change must use the `RestoreTracks` hint "or its undo will silently do nothing" (`undohelper.h:40-50`).

**Lessons.**
- Inclusive out points cause off-by-one bugs.
- Silent clamping hides errors.
- Separate valid/do code paths diverge.
- Diff-based undo cannot see everything. An explicit change log that records every mutation as it happens is safer.

### libopenshot (LGPL-3.0-or-later)

**Key files:** `src/ClipBase.h`, `src/Clip.cpp`, `src/Timeline.cpp`, `src/Fraction.h`.

**Time.**
- A clip's `position`, `start` and `end` are `float` seconds, alongside an `int layer` (`src/ClipBase.h:35-38`).
- Clips on the same layer may overlap; there is no single-lane invariant.
- Frame numbers are 1-based.
- `Fraction` is an `int` num/den pair (`Fraction.h`).

**Rounding is inconsistent within one file.**
- `floor(t·fps)+1` in `GetMinFrame` (`Timeline.cpp:500`).
- `llround(position·fps)+1` for the start of a clip, with an end frame computed without the `+1` in one place (`Timeline.cpp:1077-1078`) and with it in another (`:1189-1190`).
- Plain truncating casts on the cache-invalidation paths (`Timeline.cpp:1519-1520`, `:1554-1580`, `:1638-1639`).
- Cache removal pads by ±8 frames to cover the disagreement (`Timeline.cpp:1521`).

**Edits and retiming.**
- Edits arrive as JSON diff messages (`Timeline::ApplyJsonDiff`, `Timeline.cpp:1403`; insert/update/delete at `:1531-1574`).
- Ids are strings (`ClipBase.h:34`).
- Retime is a `time` keyframe curve that maps frames to frames (`Clip.cpp:595-610`).

**Lesson.** Float time plus several rounding conventions forces fudge factors. FreeCut needs one
conversion function per direction, used everywhere.

### Olive (GPL-3.0-or-later)

**Key files:** `app/node/output/track/track.{h,cpp}`, `app/node/block/block.h`,
`app/node/block/clip/clip.{h,cpp}`, `app/node/block/transition/transition.cpp`,
`app/timeline/timelineundopointer.cpp`, `app/timeline/timelineundosplit.cpp`,
`app/widget/timelinewidget/tool/transition.cpp`. Olive's `rational` and `Timecode` types live
in the `olive-core` submodule (`ext/core`, `.gitmodules`), which is absent from our clone, so
their internals are not reviewed here.

**Tracks.**
- A track is an ordered vector of blocks (clip, gap, transition, subtitle). Each block stores only its length.
- Starts and ends are recomputed from the edited index onward (`track.cpp:566-586`), and lookups binary-search them (`track.cpp:603-626`).
- Track types are `kVideo`, `kAudio` and `kSubtitle` (`track.h:37-43`).
- Gaps are explicit `GapBlock`s.

**Transitions.**
- A transition is a block that **occupies time**. `TrackPlaceBlockCommand` ripple-removes the area, then inserts the block (`timelineundopointer.cpp:316-361`). The adjacent clips are trimmed, and the transition renders their media beyond the new edit points.
- `in_offset` and `out_offset` are derived from the length and a "center offset" input (`transition.cpp:66-104`).
- One-sided transitions are allowed (`tool/transition.cpp:123-158`).
- Progress is computed in `double` (`transition.cpp:117-140`).

**Split.** `BlockSplitCommand::redo` asserts `in < point < out` and moves the outgoing transition
onto the new right-hand block (`timelineundosplit.cpp`). A `FIXME` notes that multi-point
splits assume the times are sorted (`timelineundosplit.cpp:119`).

**Speed and reverse.**
- Speed is a `double` (`clip.h:81`).
- `ClipBlock::SequenceToMediaTime` turns a rational into a double and back with `rational::fromDouble` to apply speed (`clip.cpp`), so exactness is lost.
- Reverse is `length() − t`, which at `t = 0` lands on the exclusive end of the source range, one frame past the last valid one.

**Other notes.**
- `RATIONAL_MIN`/`RATIONAL_MAX` serve as "none" sentinels (`app/widget/timelinewidget/timelinewidget.cpp:187`, `clip.cpp`). Every consumer must special-case them.
- Linked A/V is a many-to-many `block_links_` (`clip.h:126`, `:241`).
- Undo commands create sub-commands lazily on their first `redo()` (`timelineundopointer.cpp:318-358`). The command's effect therefore depends on the state at first redo.

### Comparison

| Aspect | OTIO | Kdenlive | Shotcut/MLT | libopenshot | Olive | **FreeCut** |
|---|---|---|---|---|---|---|
| Time unit | f64 value + f64 rate | int frames (+ f64 `GenTime`) | int32 frames, inclusive out | f32 seconds | rational (+ f64 speed) | **i64 ticks, 1/705 600 000 s** |
| Clip position | derived from durations | explicit | derived (playlist) | explicit float | derived (blocks) | **explicit start, sorted `Vec`** |
| Gaps | explicit `Gap` | MLT blanks | explicit blanks + consolidation | implicit | explicit `GapBlock` | **derived** |
| Same-track overlap | never | yes, for mixes (2 playlists) | never (mix item) | allowed | never | **never** |
| Transition | cut-anchored, in/out offsets, handles | stored overlap + handles | mix item consumes time | free-floating effect | block consumes time + handles | **cut-anchored in/out offsets; placement `Overlap` or `Handles`** |
| Ids | object identity | runtime int | `QUuid` | string | pointers | **persisted `NonZeroU64` newtypes** |
| Undo | n/a | closures | XML diff | JSON diff | command objects | **data `ChangeSet` recorded by a transaction** |
| Speed vs length | independent | f64 + re-round | n/a | keyframe curve | f64 | **exact `Rational`; source length authoritative** |

### Distilled lessons

1. **Exact integers or nothing.** Every float-based design (OTIO, `GenTime`, libopenshot, Olive's
   speed) ends up with epsilons or fudge padding. Every frame-integer design (MLT, Kdenlive)
   must rewrite positions when the rate changes. A fixed sub-frame tick avoids both.
2. **One rounding rule per conversion, named at the call site.** libopenshot and MLT mix floor,
   round and truncate for the same mapping.
3. **Half-open ranges only.** MLT's inclusive out points are a constant source of `±1`.
4. **Never mutate the document to preview.** Kdenlive's drag preview edits the live model. FreeCut
   plans purely (TL-018).
5. **Record changes as they happen, not by diffing.** Shotcut's diff-based undo can miss
   in-place changes.
6. **Stable, persisted ids.** Kdenlive's runtime ids force position-keyed group serialization.
7. **Don't let the render engine's layout leak into the model.** Kdenlive's two playlists per
   track and MLT's mix tractors exist only because of how MLT renders.
8. **Keep validity checks and mutations in one code path.** Shotcut's `*Valid()` functions are
   written separately from the mutations they guard.
9. **Speed must not make lengths drift.** Kdenlive re-rounds in/out on every speed change.
10. **Consistency checkers pay off** (Kdenlive's `checkConsistency`). FreeCut makes
    `validate()` the oracle of its property tests.

---

## Recommended FreeCut design

### 1. Principles

- `freecut-timeline` is pure data and pure functions:
  - `#![forbid(unsafe_code)]`.
  - No I/O, no FFmpeg, no GPU, no threads.
  - Media facts (durations) come in as plain data through a trait.
- **One time type**, `Tick`, used for every instant and every span in sequence or source time.
  `Rational` exists only for dimensionless ratios such as speed. Container timestamps are converted
  at the boundary by functions in this crate, which `freecut-media` calls.
- **Explicit positions, derived gaps.** A track is a `Vec<Clip>` sorted by start with no
  overlaps. Ripple is "shift every clip after `t` by `δ`". Gaps, cut points and duration are
  computed.
- **Transitions are cut-anchored joins**, using the OTIO geometry. They never make clips overlap,
  so the single-lane invariant (TL-004) holds literally. FX-002's overlap placement is an
  editing *policy* that creates handles by trimming and rippling (§6).
- **All mutation goes through a recording transaction (`Txn`).** Planning applies changes to a
  copy-on-write working copy and records each primitive `Change` as it happens. The resulting
  `ChangeSet` is exactly what was done, and its inverse is mechanical.
- **`validate()` is the oracle.** It is run on load, after every plan in debug builds (on the
  touched scope in release), and after every step of the property tests.

### 2. Module layout

```text
crates/freecut-timeline/src/
  lib.rs                 #![forbid(unsafe_code)]; re-exports
  time/
    mod.rs               Tick, TICKS_PER_SECOND, Rounding, div_round, TimeError
    range.rs             TimeRange (duration > 0), MarkRange (duration >= 0)
    rate.rs              FrameRate, SampleRate, PRESET_FRAME_RATES
    rational.rs          Rational (speed etc.), gcd
    stream.rs            StreamTimeBase, ticks_from_stream, stream_ts_at_or_before
    timecode.rs          Timecode, TcStyle, formatting/parsing, drop-frame (v1)
  id.rs                  entity id newtypes, IdAlloc
  model/
    timeline.rs          Timeline { root, sequences, ids }
    sequence.rs          Sequence, SequenceFlags, Band
    track.rs             Track, TrackKind, TrackFlags, TrackPayload
    clip.rs              Clip, ClipContent, SourceRange, EmbeddedAudio, StillSource
    transition.rs        Transition, Placement, Alignment, TransitionPayload
    marker.rs            Marker, MarkerOrigin (colour: PaletteColor, README C53)
    relation.rs          Attachment, GroupId/LinkId helpers, sync offsets
    props/               typed property bags defined with TR/FX/AU/CL/TX (data only)
  retime.rs              Retime, source_time_at, timeline_len, source_len (TR-036 functions)
  limits.rs              SourceLimits trait, Limits { max_tracks, max_depth, ... }
  index.rs               TimelineIndex: derived id -> location lookups
  query.rs               clips_at, clips_in_range, coverage_at, gaps, cut_points,
                         snap_targets, duration, next/prev cut
  validate.rs            validate(), validate_scope(), InvariantError
  edit/
    mod.rs               EditCtx, EditError, Txn
    ripple.rs            RipplePlan (TL-021), shift helpers
    insert.rs move_.rs trim.rs split.rs delete.rs gaps.rs freeze.rs
    transition.rs group.rs link.rs attach.rs compound.rs markers.rs tracks.rs conform.rs
  history.rs             Change, ChangeSet, apply, History, SelectionSnapshot
  settings.rs            ProjectSettings validation helpers (PM-005/PM-006 tables)
```

### 3. Time

#### 3.1 `Tick` and rounding

`TICKS_PER_SECOND = 705_600_000 = 2⁹·3²·5⁵·7²`. This is the same constant as the publicly
documented "flicks" unit. A reduced rate `num/den` lies on the grid iff `num` divides
705 600 000.

| Rate | Ticks per frame | | Rate | Ticks per sample |
|---|---|---|---|---|
| 24000/1001 | 29 429 400 | | 8 kHz | 88 200 |
| 24 | 29 400 000 | | 16 kHz | 44 100 |
| 25 | 28 224 000 | | 22.05 kHz | 32 000 |
| 30000/1001 | 23 543 520 | | 32 kHz | 22 050 |
| 30 | 23 520 000 | | 44.1 kHz | 16 000 |
| 48 | 14 700 000 | | 48 kHz | 14 700 |
| 50 | 14 112 000 | | 88.2 kHz | 8 000 |
| 60000/1001 | 11 771 760 | | 96 kHz | 7 350 |
| 60 | 11 760 000 | | 176.4 kHz | 4 000 |
| 120000/1001 | 5 885 880 | | 192 kHz | 3 675 |
| 120 | 5 880 000 | | 384 kHz | **3 675/2, not exact** |

- **Range.** `i64::MAX` ticks ≈ 414 years.
- **JSON.** 2⁵³ ticks ≈ 147.7 days. A tool that parses JSON numbers as f64 stays exact below that, which is far beyond any edit.
- **Stream time bases that are not exact:**
  - `1/15360` (common in MP4): 45 937.5 ticks per unit.
  - `AV_TIME_BASE` `1/1000000`: 705.6 ticks per unit.
  - Values in these bases round to the nearest tick, so the error is under 1.5 ns. §3.5 explains how to keep frame lookup consistent despite that.
- **Exact stream time bases:** `1/90000`, `1/48000`, `1/12800`, `1/1000`, `1/600`, `1001/30000` and `1/30000`.

```rust
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// An instant or a span in ticks. No `Add`/`Sub` impls on purpose: every
/// arithmetic step is checked, because a wrap in release mode would silently
/// corrupt a project.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct Tick(i64);

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Rounding { Floor, Ceil, NearestTiesDown, NearestTiesEven }

/// `n / d` rounded as requested. Euclidean division, so negative values round
/// down instead of toward zero (the `int(x)` truncation bug).
pub(crate) fn div_round(n: i128, d: i128, mode: Rounding) -> i128 {
    debug_assert!(d > 0, "callers pass validated positive denominators");
    let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
    match mode {
        Rounding::Floor => q,
        Rounding::Ceil => q + i128::from(r != 0),
        Rounding::NearestTiesDown => q + i128::from(2 * r > d),
        Rounding::NearestTiesEven => match (2 * r).cmp(&d) {
            core::cmp::Ordering::Less => q,
            core::cmp::Ordering::Greater => q + 1,
            core::cmp::Ordering::Equal => q + (q & 1), // round to even
        },
    }
}

impl Tick {
    pub const ZERO: Tick = Tick(0);
    pub const fn from_raw(v: i64) -> Tick { Tick(v) }
    pub const fn raw(self) -> i64 { self.0 }
    pub fn checked_add(self, d: Tick) -> Result<Tick, TimeError> {
        self.0.checked_add(d.0).map(Tick).ok_or(TimeError::Overflow)
    }
    pub fn checked_sub(self, d: Tick) -> Result<Tick, TimeError> {
        self.0.checked_sub(d.0).map(Tick).ok_or(TimeError::Overflow)
    }
    /// `self * r`, exact in i128, rounded once at the end.
    pub fn mul_rational(self, r: Rational, mode: Rounding) -> Result<Tick, TimeError> {
        let v = div_round(i128::from(self.0) * i128::from(r.num()), i128::from(r.den()), mode);
        i64::try_from(v).map(Tick).map_err(|_| TimeError::Overflow)
    }
    /// UI edge only. Never feed the result back into the model.
    pub fn as_secs_f64(self) -> f64 { self.0 as f64 / TICKS_PER_SECOND as f64 }
}
```

Rounding rules. There is exactly one function per direction, and the rule is fixed per
conversion:

| Conversion | Rule | Why |
|---|---|---|
| Edit point → frame grid (`FrameRate::quantize`) | `NearestTiesDown` | TL-001 |
| Stream timestamp → tick (`ticks_from_stream`) | `NearestTiesEven` | TL-001; unbiased |
| Caption cue ms → tick | `NearestTiesEven` | TX import |
| Tick → frame index (timecode, "frame containing t") | `Floor` | a frame is displayed during `[k·f, (k+1)·f)` |
| Tick → audio sample index | `Floor` (exact on grid) | AU |
| Timeline length → source length on trim (`T·r`) | `Floor` | TL-011: never read past the source |
| Source length → timeline length on speed change (`D/r`) | nearest frame, min 1 frame | TR-036 |
| Pixel → tick (app) | `NearestTiesDown`, then quantize | TL-001 |

#### 3.2 `FrameRate`, `SampleRate`

```rust
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct FrameRate { num: u32, den: u32, frame: i64 } // reduced; `frame` = ticks per frame

impl FrameRate {
    pub fn new(num: u32, den: u32) -> Result<Self, TimeError> {
        if num == 0 || den == 0 { return Err(TimeError::InvalidRate { num, den }); }
        let g = gcd_u32(num, den);
        let (num, den) = (num / g, den / g);
        let total = i128::from(TICKS_PER_SECOND) * i128::from(den);
        if total % i128::from(num) != 0 {
            return Err(TimeError::OffGridRate { num, den });
        }
        let frame = i64::try_from(total / i128::from(num)).map_err(|_| TimeError::Overflow)?;
        Ok(Self { num, den, frame })
    }
    pub const fn frame_ticks(self) -> Tick { Tick(self.frame) }
    /// Nearest frame boundary, ties to the earlier frame (TL-001).
    pub fn quantize(self, t: Tick) -> Result<Tick, TimeError> {
        let k = div_round(t.0.into(), self.frame.into(), Rounding::NearestTiesDown);
        i64::try_from(k * i128::from(self.frame)).map(Tick).map_err(|_| TimeError::Overflow)
    }
    /// Index of the frame displayed at `t`.
    pub fn frame_index(self, t: Tick) -> i64 { t.0.div_euclid(self.frame) }
    pub fn frame_start(self, k: i64) -> Result<Tick, TimeError> {
        k.checked_mul(self.frame).map(Tick).ok_or(TimeError::Overflow)
    }
    pub fn is_on_grid(self, t: Tick) -> bool { t.0.rem_euclid(self.frame) == 0 }
    // fn nearest_valid(num, den) -> Vec<FrameRate>: PM-005's "suggest the nearest valid
    // rates"; compares candidates by cross-multiplication, never via f64.
}
```

`SampleRate(u32)` uses the same validation, `TICKS_PER_SECOND % sr == 0`. 384 kHz fails, which
is acceptable: the mixer resamples, and source sample positions are not model time.

Serde: `FrameRate` serializes as `{num, den}` and deserializes through `TryFrom<RawRate>`, which
calls `new`. A hand-edited or corrupt file with an off-grid rate is refused (TL-001), not
rounded.

#### 3.3 `TimeRange`, `MarkRange`

```rust
/// Half-open [start, start + duration). duration > 0, and the end is guaranteed
/// to fit in i64, so `end()` cannot fail.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct TimeRange { start: Tick, duration: Tick }

impl TimeRange {
    pub fn new(start: Tick, duration: Tick) -> Result<Self, TimeError> {
        if duration.raw() <= 0 { return Err(TimeError::NonPositiveDuration); }
        start.checked_add(duration)?; // establishes the end() invariant
        Ok(Self { start, duration })
    }
    pub fn from_start_end(start: Tick, end: Tick) -> Result<Self, TimeError> {
        Self::new(start, end.checked_sub(start)?)
    }
    pub const fn start(self) -> Tick { self.start }
    pub const fn duration(self) -> Tick { self.duration }
    pub fn end(self) -> Tick {
        // Cannot overflow: checked in `new`, and the fields are private.
        Tick::from_raw(self.start.raw() + self.duration.raw())
    }
    pub fn contains(self, t: Tick) -> bool { self.start <= t && t < self.end() }
    pub fn overlaps(self, o: TimeRange) -> bool { self.start < o.end() && o.start < self.end() }
    pub fn intersection(self, o: TimeRange) -> Option<TimeRange> {
        let (s, e) = (self.start.max(o.start), self.end().min(o.end()));
        TimeRange::from_start_end(s, e).ok()
    }
    pub fn shifted(self, delta: Tick) -> Result<TimeRange, TimeError> {
        TimeRange::new(self.start.checked_add(delta)?, self.duration)
    }
}
```

`MarkRange` is the same shape with `duration >= 0`, for point markers. There are no epsilon
predicates. With integers, `meets` is just `a.end() == b.start()`.

#### 3.4 `Rational`

`Rational { num: i64, den: i64 }` is reduced, with `den > 0` and its own `gcd`. It provides
`checked_mul`, `checked_div`, `recip` and `cmp` (by cross-multiplying in `i128`). It is used for
speed (TR-037: 1.37× is stored as 137/100), curve coefficients (TR-036) and aspect ratios in
settings. Constructing it from a decimal string is exact (`"29.97"` → `2997/100`), and the PM-005
preset table maps `29.97` to `30000/1001` explicitly. It never has a constructor from `f64`.
Olive's `fromDouble` round trip is the anti-pattern.

#### 3.5 Stream time bases (the `freecut-media` boundary)

```rust
/// Seconds per unit = num/den, like FFmpeg's AVRational. Both > 0.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StreamTimeBase { num: i32, den: i32 }

/// The canonical (and only) stream -> tick conversion.
pub fn ticks_from_stream(ts: i64, tb: StreamTimeBase) -> Result<Tick, TimeError> {
    let n = i128::from(ts) * i128::from(tb.num) * i128::from(TICKS_PER_SECOND);
    let v = div_round(n, i128::from(tb.den), Rounding::NearestTiesEven);
    i64::try_from(v).map(Tick::from_raw).map_err(|_| TimeError::Overflow)
}

/// Largest `ts` with `ticks_from_stream(ts) <= t`: the exact inverse for
/// "the frame displayed at t". Because `ticks_from_stream` is monotonic, this
/// is well defined. The fix-up loops run at most ceil(tick/unit) + 1 times.
pub fn stream_ts_at_or_before(t: Tick, tb: StreamTimeBase) -> Result<i64, TimeError> {
    let per_unit = i128::from(tb.num) * i128::from(TICKS_PER_SECOND);
    let guess = div_round(i128::from(t.raw()) * i128::from(tb.den), per_unit, Rounding::Floor);
    let mut ts = i64::try_from(guess).map_err(|_| TimeError::Overflow)?;
    while ticks_from_stream(ts.checked_add(1).ok_or(TimeError::Overflow)?, tb)? <= t { ts += 1; }
    while ticks_from_stream(ts, tb)? > t { ts = ts.checked_sub(1).ok_or(TimeError::Overflow)?; }
    Ok(ts)
}
```

Rules for `freecut-media`:
- Convert every packet or frame PTS into ticks with `ticks_from_stream` and compare in tick
  space.
- If it must go the other way (a seek target), use `stream_ts_at_or_before`. Never use an
  independently rounded `t·den/num`.

With `1/15360`, frame `p` lands on `45 937.5·p` ticks rounded. A floor-based inverse could pick
`p−1` exactly at the boundary, so the preview would show the previous frame on the first tick of
each frame. The paired inverse rules that out. This is the same class of bug as libopenshot's
±8-frame padding, solved by construction.

#### 3.6 Timecode

`Timecode { hours: u32, minutes: u8, seconds: u8, frames: u32 }`, plus `TcStyle::{NonDrop, Drop}`.
- **Non-drop:**
  - Compute `n = rate.frame_index(t)`, then split `n` with the nominal rate `ceil(num/den)` (30 for 29.97).
  - Hours are unbounded (TL-002: three digits past 99 h, no 24 h wrap as in OTIO).
- **Drop (v1, 30000/1001 and 60000/1001 only):** integer-only, re-derived from SMPTE ST 12-1:
  - `drop = 2|4`, `per10 = 17 982|35 964` and `per_min = 1 798|3 596`.
  - `n += 9·drop·(n / per10) + drop·((n % per10 − drop) / per_min)` when `n % per10 ≥ drop`. Otherwise `n += 9·drop·(n / per10)`.
  - Then split with the nominal rate.
- The test vectors are known facts. At 29.97 DF: 1 799 → `00:00:59;29`, 1 800 → `00:01:00;02`, 17 982 → `00:10:00;00`.
- OTIO's `rationalTime.cpp:495-560` implements the same rule in f64. Our implementation is integer-only.

> **Display caveat (open question).** NTSC non-drop timecode drifts 3.6 s per hour from wall
> clock. TL-002 computes ruler labels "from ticks through f64" (wall clock) but shows the
> playhead as `HH:MM:SS:FF` (frame count). At 29.97 non-drop, the two disagree after a few
> minutes. Both should come from the same `Timecode` function.

### 4. Identity and lookup

```rust
macro_rules! entity_id {($($name:ident),* $(,)?) => {$(
    #[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
    pub struct $name(core::num::NonZeroU64);
)*}}
entity_id!(SeqId, TrackId, ClipId, TransitionId, MarkerId, GroupId, LinkId);

/// One allocator per *timeline* (not per sequence), persisted as `next_id`.
/// Monotonic: undo never lowers it, so ids are never reused (TL-006).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdAlloc { next: core::num::NonZeroU64 }

impl IdAlloc {
    pub fn alloc<I: From<core::num::NonZeroU64>>(&mut self) -> Result<I, EditError> {
        let id = self.next;
        self.next = id.checked_add(1).ok_or(EditError::IdsExhausted)?;
        Ok(I::from(id))
    }
    pub fn high_water(&self) -> u64 { self.next.get() }
    pub fn raise_to(&mut self, hw: u64) { /* next = max(next, hw); used by apply */ }
}
```

- **One id space per timeline.** Compound create and break-apart move clips between sequences
  while keeping their ids. Selection snapshots and the clipboard are plain id sets, and
  history entries can span sequences. The spec says "per-sequence counter" (TL-006, TL-023);
  see the open questions.
- **`NonZeroU64`** makes `Option<ClipId>` 8 bytes.
- **Ids are deterministic counters**, not random UUIDs, so tests and golden files are stable.
  `AssetId` (PM) may differ; the timeline treats it as an opaque `Copy + Ord` key.
- **`IdAlloc` lives in `Timeline` but outside history.** `ChangeSet` records the high-water mark
  it needs. `apply` calls `raise_to(hw)`. Undo and redo re-insert the recorded ids, and new edits
  after an undo continue above the mark.
- **Why not `slotmap`?** Its keys cannot be re-inserted at a chosen key, so undoing a delete
  would return a *different* key. That breaks "undo restores the original id" (TL-006, TL-023).

**Derived index.** It is not serialized, is rebuilt in O(N) after `apply` in v1, and can be made
incremental later if profiling asks for it:

```rust
pub struct TimelineIndex {
    clip: HashMap<ClipId, ClipLoc>,          // ClipLoc { seq, band, track_idx }
    track: HashMap<TrackId, TrackLoc>,       // TrackLoc { seq, band, idx }
    group_members: HashMap<GroupId, Vec<ClipId>>,
    link_members: HashMap<LinkId, Vec<ClipId>>,
    attached: HashMap<ClipId, Vec<ClipId>>,  // owner -> attached items
    seq_owner: HashMap<SeqId, ClipId>,       // nested sequence -> its compound clip
    transitions_of: HashMap<ClipId, [Option<TransitionId>; 2]>, // [incoming, outgoing]
}
```

The position inside a track is never cached: `clips.binary_search_by_key(&start, …)` finds it.
That removes a whole class of stale-index bugs; Shotcut and Olive store indices in commands. The
index is the only place reverse relations live. The model stores each fact once (§9).

### 5. Document model

```rust
pub struct Timeline {
    root: SeqId,
    sequences: BTreeMap<SeqId, Arc<Sequence>>, // BTreeMap: deterministic save order (PM-021)
    ids: IdAlloc,
}

pub struct Sequence {
    id: SeqId,
    visual: Vec<Arc<Track>>,   // bottom -> top; visual[0] is the main track (by position)
    audio: Vec<Arc<Track>>,    // top -> bottom (presentational)
    markers: Vec<Marker>,      // sequence time, sorted by (start, id)
    flags: SequenceFlags,      // main_magnet, follow_main, flexible_stacking,
                               // ripple_all_tracks, markers_follow_ripple
}

pub struct Track {
    id: TrackId,
    kind: TrackKind,
    name: Option<String>,
    flags: TrackFlags,               // muted, hidden, locked
    payload: TrackPayload,           // None | Caption(CaptionTrackData) (TX-025)
    clips: Vec<Clip>,                // sorted by start, non-overlapping
    transitions: Vec<Transition>,    // sorted by cut time; at most one per cut
}

#[non_exhaustive]
pub enum TrackKind { Video, Text, Caption, Sticker, Effect, Adjustment, Audio }

pub struct Clip {
    id: ClipId,
    range: TimeRange,                // placement in sequence time
    content: ClipContent,
    enabled: bool,
    name: Option<String>,
    label: Option<LabelColor>,
    markers: Vec<Marker>,            // source time (media/compound) or clip-local time
    group: Option<GroupId>,
    attach: Option<Attachment>,
    link: Option<LinkId>,
    props: PropBags,                 // typed bags owned by TR/FX/AU/CL/TX
}

pub enum ClipContent {
    Media { asset: AssetId, streams: StreamSel, source: SourceRange, retime: Retime,
            embedded_audio: EmbeddedAudio },          // EmbeddedAudio::{Active(ClipAudio), Detached(LinkId), None}
    Still { from: StillSource },                       // MediaFrame{asset, source_time} | SequenceFrame{seq, time}
    Generated { kind: GeneratedKind },                 // text, sticker, shape, effect, adjustment
    Caption(CaptionSegment),                           // TX-025 segment payload
    Compound { seq: SeqId, source: SourceRange, retime: Retime },
}

/// Range in the content's own time: media time (0 = first presentable frame,
/// PM-035) or nested-sequence time.
pub struct SourceRange { start: Tick, duration: Tick } // duration > 0, start >= 0
```

Notes:
- **Main track by position.** `visual[0]` is the main track. Nothing else marks it. "Exactly one
  main track, lowest, Video" is then one check (`visual[0].kind == Video`), and the main track
  cannot be moved by construction. The loader repair in TL-004 is a pure helper
  `repair_main_track(&mut Sequence) -> Option<RepairNote>`, called by `freecut-project`.
- **Caption segments are clips.** They get move, trim, split, select, snap and undo for free.
  The track-level style lives in `TrackPayload::Caption` (TX-025). TX asks TL to add the
  `Caption` kind, and this design does.
- **No frame rate in `Sequence`.** The rate belongs to `ProjectSettings` (PM-005) and is passed
  in through `EditCtx`, so it is defined in one place. Nested sequences render at the parent's
  rate (TL-040). `validate()` does not require positions to be on the grid, because after a
  rate change without conform they legitimately aren't (TL-001).
- **`Arc` per sequence and per track.** A commit clones only the touched tracks
  (`Arc::make_mut`). The same sharing makes snapshots for render and audio threads cheap (§14).
- **Property bags are concrete data types** (`Clone + PartialEq + Debug`, serde), defined in
  `model/props/*` together with the owning spec. There are no `Box<dyn Any>` bags: those would
  break equality-based undo tests and serialization. Large bags (keyframe lists, caption word
  timings) are held in `Arc` so that `UpdateClip` snapshots share them.

**Four clip ranges**, named after OTIO's (`docs/tutorials/time-ranges.md`):

| FreeCut name | Domain | Meaning |
|---|---|---|
| `clip.range` | sequence | where the clip sits (placement) |
| `source` | content | the part of the content shown, excluding transition handles |
| `visible_source(track)` | content | `source` extended by the adjacent transitions' handles (derived) |
| `available` | content | `SourceLimits::media_duration(asset)`; `None` while probing; last-known when offline |

**Placement length and source length.** This settles a spec inconsistency between TL-011 and
TR-036.
- Store **both** `range.duration` (T) and `source.duration` (D), plus the exact `Retime`.
- **Speed edits keep D and recompute T** as `max(1 frame, nearest_frame(D / r))` (TR-036). Toggling 1× → 2× → 1× therefore restores T exactly. Kdenlive's re-rounding drift cannot happen.
- **Trims set T** (grid-quantized) **and recompute D** as `floor_tick(T · r)`, clamped to `available` (TL-011).
- Curves use TR's exact integral.
- Undo restores both values from the `ChangeSet` and recomputes nothing.
- The reads past `out` that come from rounding T are clamped by the readers (TR-036 edge case).
- `retime.rs` holds the only implementations of `source_time_at`, `timeline_len` and `source_len`. `source_time_at` accepts clip-local `t` slightly outside `[0, T)`, so that transition handles can be evaluated: TR-036 says "extending past the authored range continues at the edge speed".

> **Reverse pitfall (to TR).** TR-036 defines reverse as `s(t) = out − t·r`. At `t = 0` that is
> the exclusive end of the source, one frame past the last valid one (Olive's `length() − t` has
> the same flaw). Frame selection for reversed clips should use `out − (t + frame)·r` (the start
> of the interval displayed in frame `t`), or "the frame strictly before `s(t)`". This needs a
> decision in TR before implementation.

### 6. Transitions: cut-anchored joins

```rust
pub struct Transition {
    id: TransitionId,
    from: ClipId,            // outgoing clip; ends exactly at the cut
    to: ClipId,              // incoming clip; starts exactly at the cut
    in_offset: Tick,         // window part before the cut (filled from `to`'s head handle)
    out_offset: Tick,        // window part after the cut (filled from `from`'s tail handle)
    placement: Placement,    // how it was created, which decides how removal undoes it
    payload: TransitionPayload,
}
pub enum Placement { Overlap, Handles(Alignment) }      // FX-002 default / FX-008, AU-029
pub enum Alignment { Centered, EndAtCut, StartAtCut }
pub enum TransitionPayload {
    Video { kind: TransitionKindId, params: ParamSet, easing: Easing, audio_crossfade: bool }, // FX-002/007
    AudioCrossfade { curve: CrossfadeCurve },                                                  // AU-029
}
```

**Geometry.** This is OTIO's model: `src/opentimelineio/transition.cpp`, `track.cpp:52-92` and
`:151-165`.
- The cut is at `c = from.range.end() == to.range.start()`. The window is
  `[c − in_offset, c + out_offset)`, with `D = in_offset + out_offset`.
- In `[c − in_offset, c)`, `from` plays normally and `to` plays head-handle media (clip-local
  time `−in_offset … 0`).
- In `[c, c + out_offset)`, `to` plays normally and `from` plays tail-handle media.
- Clips never overlap. The single-lane invariant (TL-004) holds, the magnetic main track stays
  contiguous, and `gaps()`, `cut_points()`, `duration()` and attachment lookup need no special
  cases.
- Kdenlive needs two playlists because it stores the overlap (`trackmodel.hpp:366-371`). We
  never store it.

**Overlap placement (FX-002 default) is an editing policy, not a different geometry.**
`plan_add_transition` with `Placement::Overlap` and duration `D` on a butt cut at `c` does three
things:
1. It shortens `from` at its tail by `o`, and `to` at its head by `i` (advancing `to`'s source
   start by `i` through its time map), where `o = floor_frames(D/2)` and `i = D − o`.
2. It closes the resulting space, so that `to` starts at `from`'s new end:
   - `to` moves by `−o`;
   - every later clip on the track moves by `−D`;
   - other tracks and attached items follow the TL-021 ripple rules and TL-029.
3. It inserts the transition with `in_offset = i` and `out_offset = o`.
   - The end result, with `c` the original cut and `c−o` the new one: `from = [a₀, c−o)`,
     `to = [c−o, c+len(to)−D)`, window `[c−D, c)`.

The handles are exactly the frames that were trimmed away, so they always exist. That is
FX-002's "no extra source footage". The rendered result is identical to "B moved `D` earlier and
overlaps A": A's last frame still shows at `c − 1 frame`, and B's first frame at `c − D`. This
is the result MLT achieves with a mix item (`mlt_playlist.c:1145-1240`), without inventing a
third item.

**Removal** reverses by `placement`:
- `Overlap`: extend `from` by `out_offset` and `to` by `in_offset`, then ripple `+D`. This is
  FX-002's "restores the butt cut".
- `Handles`: delete the transition only.

**Handle placement** (FX-008, AU-029) moves nothing. It requires `to` to have `in_offset` of
media before its `source.start`, and `from` to have `out_offset` after its `source.end`, both
mapped through retime. Reversed clips take handles from the opposite side (TR-036).

**Policy hook.** Every op that changes a clip next to a transition calls
`fit_transitions(txn, clip)` before finishing. It clamps the offsets to:
- the 50 % cap of FX-002,
- the window-disjointness invariant below,
- the available handles.

If `D` falls below 1 frame, the transition is removed, and the clamp or removal is recorded in
the same `Txn` (FX-002, AU-029). Splitting inside a window is refused (TL-023). A split elsewhere
keeps the outgoing transition on the right-hand part, as Olive does in
`timelineundosplit.cpp`.

**Render query.** The renderer asks the track for the coverage at a time:

```rust
pub enum Coverage<'a> {
    Empty,
    Clip { clip: &'a Clip, local: Tick },
    Transition { tr: &'a Transition, from: &'a Clip, from_local: Tick,
                 to: &'a Clip, to_local: Tick,
                 elapsed: Tick, duration: Tick }, // FX-002 progress = (elapsed + frame/2) / duration, computed by render
}
impl Track { pub fn coverage_at(&self, t: Tick) -> Coverage<'_> { /* binary search + ≤2 neighbour checks */ } }
```

Progress is handed over as two integers. The f32 division happens in `freecut-render` at the
edge. Olive computes progress in double inside the model (`transition.cpp:117-140`).

**Interop.** `in_offset` and `out_offset` map 1:1 to OTIO `Transition` (PM-103).

### 7. Main track, overlays, gaps

- **Magnet on** (TL-005). The invariant: `visual[0]` is contiguous from 0. Each op enforces it
  through its policy, not through a post-pass (the TL-005 model notes): insert at a cut point,
  remove with closing, trims ripple. Turning the magnet on runs `plan_compact_main`, recorded in
  the same `SetSequenceFlag` entry.
- **Overlay tracks.** Moves and trims never ripple implicitly. A collision on a move or drop
  redirects to a new track directly above (TL-012, TL-018), or refuses the whole multi-clip move.
  There are never partial moves.
- **Ripple.** One helper, `RipplePlan { at: Tick, delta: Tick, scope: TrackScope }` (TL-021),
  emits `Change::ShiftClips` per track. When ripple-all is on, it skips exempt tracks and records
  the out-of-sync items that result.
- **Gaps are derived:** `gaps(track) -> impl Iterator<Item = (GapRef, TimeRange)>`, where
  `GapRef { track: TrackId, before: ClipId }`. Every selectable gap is "the empty space right
  before clip X", including the leading gap (TL-027). The trailing space is never a gap, so this
  reference is stable and needs no gap ids. Explicit gap items (OTIO `Gap`, MLT blanks,
  Olive's `GapBlock`) would need merge/split bookkeeping; MLT's
  `mlt_playlist_consolidate_blanks` and Olive's `SetAllowSplittingGaps`
  (`timelineundopointer.cpp:355`) exist because of exactly that.
- **Lookups** use `partition_point` over `clips` by start. Since clips don't overlap,
  `clips_at(t)` is O(log n). `clips_in_range` is O(log n + k).
  `Sequence::duration() = max(last clip end)` over tracks, which is O(tracks) because the last
  clip of a non-overlapping track has the maximum end.

### 8. Markers

```rust
pub struct Marker {
    id: MarkerId,
    at: MarkRange,            // duration 0 = point marker
    name: String,             // ≤ 120 chars (TL-037)
    note: String,             // ≤ 2000 chars
    color: PaletteColor,      // FreeCut palette enum shared with tracks and labels (TL-049, README C53), serialized as a stable string id
    origin: MarkerOrigin,     // User | Beat | Scene …
}
```

- **Timeline markers** are `Sequence.markers`, in sequence time, sorted for binary search. They
  move on ripple only if `markers_follow_ripple` is set (TL-037).
- **Clip markers** are `Clip.markers`, in **source time** for media and compound clips. They
  follow moves and head trims for free and divide on split by source time. They stay hidden, not
  deleted, when trimmed out of view.
- For generated and caption clips, which have no source time, markers are in clip-local time. A
  left-edge trim shifts them by the trim delta so they keep their sequence position.
- On screen, a marker's position comes from the retime inverse, so markers stay on the beat at
  any speed (AU beat ticks).
- The color is an enum of our own palette, so a palette change is a schema migration rather than
  a string remap. OTIO had to remap color names in its Marker schema upgrade
  (`typeRegistry.cpp:98-120`).

### 9. Groups, links, attachments (each fact stored once)

| Relation | Stored | Derived (index) | Invariant |
|---|---|---|---|
| Group | `Clip.group: Option<GroupId>` | `group_members` | ≥ 2 members, all in one sequence; flat (TL-039) |
| A/V link | `Clip.link: Option<LinkId>` on detached audio + `EmbeddedAudio::Detached(LinkId)` on the video | `link_members` | the video side exists, or the link is cleared by the op that deleted it (TL-030) |
| Attachment | `Clip.attach: Option<Attachment { owner: ClipId, owner_source_time: Tick }>` | `attached` | owner is on the main track of the same sequence; the attached clip is not |

- The spec keeps groups in both `Sequence.groups` and `Clip.group` "kept consistent". This
  design stores them once and derives the reverse map, so the two copies can't disagree. Kdenlive
  needs consistency checks for its up/down link maps (`groupsmodel.cpp:1020-1025`).
- Selection is **not** a group. It lives in the app. Kdenlive's `GroupType::Selection`
  (`definitions.h:49`) shows how putting selection into the document couples UI state to undo.
- The attachment anchor is an absolute **source time** of the owner's content (TL-029's
  "specific frame of the owner's content"). The expected start is
  `owner.range.start + timeline_offset_of(owner, owner_source_time)`.
- **Out-of-sync** for attachments (TL-009) and links (TL-030) is *derived*:
  `actual_start − expected_start`. For a link, take the video's first source time
  `s₀ = video.source.start` and compute `(a.start + toff(a, s₀)) − (v.start + toff(v, s₀))`.
  Here `toff` is the retime inverse; with no retime it is `s₀ − source.start`. It is shown in
  frames, and never stored.

### 10. Compound clips and nested sequences

- **References.** `ClipContent::Compound { seq, source, retime }` points into
  `Timeline.sequences`. The index records `seq_owner`.
- **Invariants:**
  - every non-root sequence has exactly one owner clip;
  - walking owners from any sequence reaches the root within 8 steps;
  - the owner chain never repeats.
  Because every sequence has exactly one owner, the structure is a tree. Cycles can only be
  introduced by paste or create, and those check the ancestor chain of the target (TL-032,
  TL-040).
- **Copies.** Split, duplicate and paste of a compound clip deep-copy its nested sequence with
  freshly allocated ids, recursively, using one id remap table. The deep copy is ordinary
  `InsertSequence` changes.
- **Cross-sequence ops.** One `ChangeSet` may touch several sequences, because every `Change`
  names its `SeqId`. An inner edit that shortens the nested sequence clamps the owner's `source`
  (and may ripple the parent main track) in the same entry (TL-040).
- **Break apart** maps child times by `parent_t = owner.range.start + (child_t − owner.source.start)`.
  With retime it uses `retime`'s inverse. Content outside `source` is trimmed after confirmation.
- **Empty nested sequence.** The owner keeps a 1-frame `source` over `[0, 1 frame)`, which renders
  transparent (TL-040).

### 11. Invariants (`validate.rs`)

`validate(&Timeline, Option<&dyn SourceLimits>) -> Result<(), Vec<InvariantError>>` reports
*all* violations, each with its location. `validate_scope` checks only the touched sequences and
tracks.

| Id | Invariant |
|---|---|
| T1 | `root` exists. Every other sequence has exactly one compound owner. Owner chains are acyclic, with depth ≤ `limits.max_depth` (8). |
| T2 | All entity ids are unique across the timeline and `< ids.high_water()`. |
| T3 | Groups have ≥ 2 members in one sequence. Link ids pair a `Detached(link)` video with ≥ 1 audio clip. A dangling link is an error (ops must clear it). |
| S1 | `visual[0].kind == Video`. `audio[*].kind == Audio`, and no visual track is `Audio`. |
| S2 | Strict layering bands (Video < Text/Sticker/Effect/Adjustment < Caption) unless `flexible_stacking`. |
| S3 | `visual.len() + audio.len() ≤ limits.max_tracks` (100). |
| S4 | `main_magnet ⇒` `visual[0]` is contiguous from 0: the first start is 0 and each start equals the previous end. |
| S5 | Timeline markers have `start ≥ 0`. |
| K1 | `clips` are sorted by start and `clips[i].end ≤ clips[i+1].start`. All `start ≥ 0` and `duration > 0`. |
| K2 | `kind.accepts(clip.content)`: Media video, Still and Compound on Video; Media audio on Audio; Generated by sub-kind; Caption on Caption. |
| K3 | Transitions: `from`/`to` are consecutive clips of this track with `from.end == to.start`. `in_offset, out_offset ≥ 0`, `D > 0`. At most one per cut. The payload kind matches the track (Video payload on Video tracks, crossfade on Audio). |
| K4 | Window disjointness: for every clip, `incoming.out_offset + outgoing.in_offset ≤ clip.duration`. |
| K5 | (with limits) Handles are available for `Handles` placement. `Overlap` handles are re-checked too, after speed changes. |
| C1 | Media: `source.start ≥ 0`, `source.duration > 0`, `source.end ≤ available` when known. |
| C2 | Attachment owner exists on the main track of the same sequence; the attached clip is not on the main track. |
| C3 | Compound: `source` lies within the nested sequence's duration (or the 1-frame empty case). |

Not invariants. These are enforced by ops and checked by property tests:
- positions on the current frame grid (may break after a rate change),
- `T == timeline_len(D)` (TR rounding),
- minimum 1 frame,
- the 50 % transition cap.

### 12. Editing pipeline

```rust
pub struct EditCtx<'a> {
    pub rate: FrameRate,
    pub sources: &'a dyn SourceLimits,   // plain data from the project/media library
    pub limits: Limits,                  // max_tracks = 100, max_depth = 8, ...
}

pub trait SourceLimits {
    /// Media length in source ticks: None while probing (TL-011: no clip can be
    /// created then); the last known value when offline.
    fn media_duration(&self, asset: AssetId) -> Option<Tick>;
}

/// Recording transaction over a copy-on-write working copy. The only way to
/// mutate a timeline, so a planner cannot forget to record a change (unlike
/// Shotcut's diff-based undo, which misses in-place changes).
pub struct Txn<'a> {
    base: &'a Timeline,
    work: Timeline,                 // clone of Arcs: O(sequences + tracks)
    changes: Vec<Change>,
    touched: BTreeSet<(SeqId, TrackId)>,
}

impl<'a> Txn<'a> {
    pub fn new(base: &'a Timeline) -> Self;
    pub fn alloc<I: From<NonZeroU64>>(&mut self) -> Result<I, EditError>;
    pub fn insert_clip(&mut self, seq: SeqId, track: TrackId, clip: Clip) -> Result<(), EditError>;
    pub fn remove_clip(&mut self, id: ClipId) -> Result<Clip, EditError>;
    pub fn update_clip(&mut self, id: ClipId, f: impl FnOnce(&mut Clip)) -> Result<(), EditError>;
    pub fn shift_clips(&mut self, track: TrackId, from: Tick, delta: Tick) -> Result<(), EditError>;
    pub fn insert_track(&mut self, seq: SeqId, band: Band, idx: usize, t: Track) -> Result<(), EditError>;
    // … remove_track, move_track, set_track_meta, transitions, markers, sequences, flags
    pub fn view(&self) -> &Timeline;  // planners read intermediate state
    /// Validates the touched scope, auto-removes empty unlocked tracks (TL-006),
    /// and returns None when nothing changed (no history entry: TL-018, TL-020, ...).
    pub fn finish(self, label: Label) -> Result<Option<ChangeSet>, EditError>;
}

pub fn plan_move(tl: &Timeline, ctx: &EditCtx<'_>, req: &MoveRequest) -> Result<Option<ChangeSet>, EditError>;
pub fn plan_trim(tl: &Timeline, ctx: &EditCtx<'_>, req: &TrimRequest) -> Result<Option<ChangeSet>, EditError>;
// plan_insert, plan_split, plan_delete, plan_close_gaps, plan_add_transition, plan_freeze,
// plan_group, plan_create_compound, plan_conform, … all with the same shape
```

**Primitive changes and history** (TL-043):

```rust
pub enum Change {
    InsertClip { seq: SeqId, track: TrackId, clip: Clip },
    RemoveClip { seq: SeqId, track: TrackId, clip: Clip },               // full snapshot
    UpdateClip { seq: SeqId, track: TrackId, before: Clip, after: Clip }, // same id, same track
    ShiftClips { seq: SeqId, track: TrackId, ids: Vec<ClipId>, delta: Tick }, // compact ripple
    InsertTrack { seq: SeqId, band: Band, idx: usize, track: Track },
    RemoveTrack { seq: SeqId, band: Band, idx: usize, track: Track },    // keeps id, name, flags, clips
    MoveTrack { seq: SeqId, band: Band, from: usize, to: usize },
    UpdateTrackMeta { seq: SeqId, track: TrackId, before: TrackMeta, after: TrackMeta },
    InsertTransition { seq: SeqId, track: TrackId, tr: Transition },
    RemoveTransition { seq: SeqId, track: TrackId, tr: Transition },
    UpdateTransition { seq: SeqId, track: TrackId, before: Transition, after: Transition },
    InsertMarker { owner: MarkerOwner, m: Marker },
    RemoveMarker { owner: MarkerOwner, m: Marker },
    UpdateMarker { owner: MarkerOwner, before: Marker, after: Marker },
    InsertSequence { seq: Sequence },
    RemoveSequence { seq: Sequence },
    SetFlags { seq: SeqId, before: SequenceFlags, after: SequenceFlags },
}

pub struct ChangeSet {
    label: Label,
    changes: Vec<Change>,
    ids_high_water: u64,
    selection_before: SelectionSnapshot,   // plain id sets (TL-015)
    coalesce: Option<CoalesceKey>,
}
```

- **`Change::inverse`** swaps insert with remove, swaps update before/after, and negates shift
  deltas. `ChangeSet::inverse` reverses the order.
- **Moving a clip to another track** is `RemoveClip` + `InsertClip` with the same id.
- **No indices inside a track.** Clips are placed by `start`. Only tracks carry an index, because
  track order is the data itself.
- **`apply(&mut Timeline, &ChangeSet) -> Result<(), ApplyError>`** checks each primitive's
  precondition:
  - in release: the target exists, or doesn't, as required;
  - in debug: also `before == current`.
  If a step fails, apply rolls back the already-applied prefix by applying its inverses. Then it
  raises `ids` and rebuilds the index. A failure is a bug, and it is logged (TL-043).
- **Memory.** `ShiftClips` keeps ripple entries small: thousands of shifted clips don't snapshot
  thousands of `Clip`s. `Arc` property bags share their payload between `before` and `after`.
  `ChangeSet::approx_bytes()` feeds the 512 MB budget.
- **Coalescing** (slider drags, nudges within 1 s) merges consecutive `Update*` changes on the
  same target into `(first.before, last.after)`. Anything else is concatenated.
- **Preview and commit run the same function.** The app calls `plan_*` on every pointer move to
  draw the ghost, and calls it again on release (or reuses the last result if the inputs are
  equal), then `apply`. The live model is never touched during a drag. Contrast Kdenlive,
  `timelinemodel.cpp:1587-1750`.

**Reading Kdenlive's lambda style the Rust way.** Kdenlive composes closures because C++ makes
the alternative painful. Data changes give us more: they compare with `==`, measure memory, print
a readable test diff, run in any order in property tests, and survive later serialization
(crash-recovery journals, PM-015).

### 13. Queries for render, audio and UI

All of these are pure, live in `query.rs`, and take `&Sequence` or `&Track`:

| Query | Used by | Cost |
|---|---|---|
| `Sequence::duration()` | transport, export (TL-003) | O(tracks) |
| `Track::coverage_at(t)` | render, per track per frame (FX-001) | O(log n) |
| `Track::clips_in_range(r)` | timeline drawing, audio block mixing | O(log n + k) |
| `gaps(track)`, `cut_points(seq, filter)`, `next_cut`, `prev_cut` | TL-013, TL-027 | O(n) / O(log n) |
| `snap_targets(seq, exclude) -> Vec<(Tick, SnapKind)>`, sorted | TL-028; built once per drag | O(n log n) build, O(log n) query |
| `source_time_at(clip, t)`, `timeline_offset_of(clip, s)` | render, audio, match frame (TL-046) | O(1) for Normal, O(log k) for curves |
| `audio_overlaps(track)` | FX-007 crossfades | O(transitions) |
| `clips_in_rect`, `clips_after`, `clips_before` | TL-016, TL-017 | O(log n + k) per track |

`RenderView`/`MixView` (TL-008) are thin borrowed adapters over a snapshot. They filter hidden,
muted and disabled content and expose coverage. They live in this crate, so that render and
audio never re-implement layering or the time map (TR-036 model notes).

### 14. Ownership and threading

- The document's `Timeline` is owned by the app's edit loop. Planning and `apply` are
  sub-millisecond, pure, and do no I/O, so they may run on the UI thread under the CLAUDE.md
  conventions. Decoding and rendering never touch the model.
- After every commit, the app publishes an immutable snapshot `Arc<Timeline>`. A natural home is
  an `arc_swap::ArcSwap` in `freecut-app`; the timeline crate does not depend on it. Render,
  audio and export threads load the current snapshot without locks.
- The audio callback must never block, which rules out Kdenlive's `QReadWriteLock` around the
  model and libopenshot's recursive mutexes (`Clip.cpp:597`).
- Export takes its own snapshot at start (PM-011: "export works on its own snapshot anyway").
- `Arc<Sequence>`/`Arc<Track>` structural sharing makes a snapshot O(1) and a commit
  O(touched tracks). The persistent collection crates (`im`, `imbl`) are not needed at this
  granularity.
- `Timeline: Clone + Send + Sync`. Nothing in the crate uses interior mutability.

### 15. Errors (`thiserror`)

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimeError {
    #[error("time arithmetic overflowed")]                         Overflow,
    #[error("duration must be positive")]                          NonPositiveDuration,
    #[error("invalid rate {num}/{den}")]                           InvalidRate { num: u32, den: u32 },
    #[error("rate {num}/{den} has no whole-tick frame duration")]  OffGridRate { num: u32, den: u32 },
    #[error("invalid stream time base {num}/{den}")]               InvalidTimeBase { num: i32, den: i32 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditError {
    #[error(transparent)] Time(#[from] TimeError),     // TL-001's "TimeOverflow" is Time(Overflow)
    #[error("track is locked")]                       TrackLocked(TrackId),
    #[error("the main track is locked")]              MainTrackLocked,
    #[error("space is occupied")]                     Collision { track: TrackId, at: TimeRange },
    #[error("this item can't go on that track")]      IncompatibleTrack { track: TrackId },
    #[error("too short")]                             BelowMinimum,
    #[error("not enough media")]                      SourceExhausted { clip: ClipId },
    #[error("media is still being analysed")]         SourceUnknown,
    #[error("track limit reached")]                   TrackLimit,
    #[error("nesting too deep")]                      NestingTooDeep,
    #[error("a compound clip can't contain itself")]  Cycle,
    #[error("inside a transition")]                   InsideTransition { tr: TransitionId },
    #[error("main-track clips must be adjacent")]     MainNotContiguous,
    #[error("unknown item")]                          NotFound(EntityRef),
    #[error("id space exhausted")]                    IdsExhausted,
    #[error("internal invariant violated: {0}")]      Invariant(InvariantError), // a bug; logged
}
```

The `Display` strings are developer-facing. Toast wording is mapped in `freecut-app` (UI owns
the text). `ApplyError` is a separate type, because an apply failure is always a bug.

### 16. Persistence boundary (with `freecut-project`)

- The timeline crate provides `serde` derives behind a `serde` feature:
  - transparent for `Tick` and the ids;
  - `try_from` raw mirrors for `FrameRate`, `TimeRange` and the structural types, so that
    deserializing cannot build an invalid value.
- `freecut-project` owns:
  - the envelope (`format_version`);
  - migrations on `serde_json::Value`;
  - unknown-field preservation;
  - deterministic output (PM-021);
  - the decision to call `repair_main_track` and `validate()` on load.
- The id allocator's `next_id` is persisted with the timeline (TL-006).
- No float ever appears in the file (PM-021).

### 17. How this meets the project conventions

- **Rational time.** `Tick` is an `i64` with a fixed timebase, and `Rational` holds ratios; there is no `f32`/`f64` time. Floats appear only in `as_secs_f64` and the app's pixel mapping.
- **No I/O in the timeline crate.** Media facts arrive via `SourceLimits` as plain data, and `freecut-media` calls our pure conversions.
- **Undo for every edit.** Every edit is a `ChangeSet` built by `Txn`. It is reversible by construction and tested with the apply → undo → redo pattern (§Test strategy).
- **Errors.** `thiserror` only, no `anyhow`. There is no `unwrap`/`expect` outside tests; the two places that rely on an invariant (`TimeRange::end`, the index lookups after `validate`) carry a comment.
- **No `unsafe`.** The crate is `#![forbid(unsafe_code)]`.
- **Testable in isolation.** No crate dependency other than `serde` (optional) and `thiserror`.

---

## Rust ecosystem

Versions below were checked with `cargo info` on 2026-10-11.

**Use:**

| Crate | Version | License | Where | Why |
|---|---|---|---|---|
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | timeline | error enums (project convention) |
| `serde` (derive) | 1.0.229 | MIT OR Apache-2.0 | timeline (optional feature) | transparent/`try_from` derives for ticks, ids, rates; validated deserialization |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | dev-dep (golden tests); `freecut-project` | exact `i64` round trip of ticks |
| `proptest` | 1.12.0 (MSRV 1.88) | MIT OR Apache-2.0 | dev-dep | property and state-machine tests over random op sequences |
| `insta` | 1.49.0 | Apache-2.0 | dev-dep | golden snapshots of ASCII-rendered timelines and change sets |
| `pretty_assertions` | 1.4.1 | MIT OR Apache-2.0 | dev-dep | readable structural diffs for undo == original |
| `arc-swap` | 1.9.2 | MIT OR Apache-2.0 | `freecut-app` (not timeline) | lock-free snapshot publication to render and audio threads |

**Optional / later:**
- `smallvec` 1.16.3 (MIT OR Apache-2.0; avoid the 2.0 betas), for `[Option<TransitionId>; 2]`-style small lists if profiling shows allocation pressure.
- `rustc-hash` 2.1.3 (Apache-2.0 OR MIT), for a faster hasher in `TimelineIndex`. Never iterate a hash map for output (determinism).
- `test-strategy` 0.4.5 (MIT OR Apache-2.0), for proptest derive ergonomics.

**Avoid:**

| Crate | Why not |
|---|---|
| `num-rational` 0.4.2 (MIT OR Apache-2.0) | Its `+ − × ÷` operators use the underlying integer ops, so they panic on overflow in debug and wrap silently in release unless every call site uses the `Checked*` traits. Our needs (speed ratios, `i128` intermediates, a 10-line gcd) are small enough to own. Revisit if TR's curve math needs general rationals. |
| `slotmap` 1.1.1 (Zlib) | Keys can't be re-inserted at a chosen key, so undo of a delete would change ids (TL-006, TL-023). Its serialized keys are also not the stable integers we want in the file. |
| `im` 15.1.0 / `imbl` 7.0.2 (MPL-2.0+) | `Arc` per track already gives structural sharing. `im` has had no release since 15.1.0, and the maintained fork `imbl` would be another dependency for no measured gain. |
| `vtc` 0.1.14, `timecode` 0.7.5, `video-timecode` 0.6.1 (MIT) | Small single-purpose crates with their own time representations. We need timecode computed from `Tick` + `FrameRate` with no hour wrap and integer drop-frame. That is about 100 lines, tested against the vectors in §3.6. |
| `opentimelineio` 0.1.0 | Placeholder crate with no bindings. For PM-103 we write our own mapper, and may use the C++ library only in tests. |
| `uuid` 1.28.0 / `ulid` 3.0.0 (for entity ids) | Random ids make tests and golden files nondeterministic, and cost 16 bytes per reference. Counters suffice for a single-user document. PM may still use UUIDs for project or asset identity. |
| Any `f64`-seconds time crate | Violates R1 and R9, the lesson of OTIO, `GenTime` and libopenshot. |

---

## Test strategy

**1. Time unit tests** (`time/`). Table-driven:
- Every PM-005 preset and every common sample rate has an exact whole-tick period, and all
  `Tick ↔ frame index ↔ frame start` round trips are identities.
- Rates such as 11 fps and 384 kHz are rejected with the right `TimeError`.
- Rounding tables for `div_round` in all modes, including negative numerators (catches
  truncation toward zero) and exact ties. For example, quantizing `frame/2` at 30 fps goes to 0
  (ties down), and `3·frame/2` goes to `frame`.
- Overflow boundaries: `TimeRange::new(Tick(i64::MAX − 1), Tick(2))` is `Err(Overflow)`;
  `quantize` near `i64::MAX` is `Err`, not a panic.
- Stream conversions for `1/90000`, `1/15360`, `1/1000000`, `1/600`, `1001/30000`, `1/1000000000`.
- Timecode golden vectors: non-drop at 23.976, 25 and 30; 100 h without wrap; drop-frame vectors
  from §3.6. OTIO's `tests/test_opentime.py` (Apache-2.0) is a second source of timecode facts to
  cross-check, without copying code.

**2. Time property tests** (`proptest`):
- `quantize` is idempotent, monotonic and within half a frame.
- `ticks_from_stream` is monotonic.
- For all `ts`: `stream_ts_at_or_before(ticks_from_stream(ts)) == ts`. For all `t`:
  `ticks_from_stream(stream_ts_at_or_before(t)) <= t < ticks_from_stream(that + 1)`.
- `TimeRange::intersection` is commutative, contained in both ranges, and empty iff not
  `overlaps`.

**3. Model invariant tests.**
- A fixture builder DSL in `tests/support/`:

  ```rust
  seq().main(["A:2s", "B:3s"]).overlay(Text, ["@1s T:1s"]).audio(["@0 M:10s"])
  ```

  It builds valid timelines in one line.
- One hand-built *invalid* fixture per invariant (T1…C3), each asserting the exact
  `InvariantError` variant and location. The same fixtures as JSON in `tests/fixtures/invalid/`
  are reused by `freecut-project` load tests.

**4. Op tests: the required triple** (TL-043), for every `plan_*`:
1. plan, then apply, then `validate()`;
2. apply the inverse and `assert_eq!(timeline, original)`, including ids and selection snapshot;
3. apply again and `assert_eq!(timeline, after)`.

Each op also gets its spec edge cases:
- the no-op cases that must return `None`, with no history entry;
- locked tracks;
- refusal of a whole multi-clip move;
- the magnet on and off;
- ripple-all exemptions;
- transitions clamped or removed and restored by undo.

**5. State-machine property test** (`proptest`): random sequences of up to 50 ops
(insert, move, trim, split, delete, ripple delete, close gaps, add/remove transition, group,
compound create/break, track add/remove/move, flag toggles, frame-rate conform). It generates
op requests from the current state with a fixed `SourceLimits` table. After each step:
- `validate()` passes;
- no clip is shorter than 1 frame;
- the main track is contiguous when the magnet is on;
- every transition's handles exist;
- `T == timeline_len(D)` for unretimed clips.

At the end, undo everything (must equal the initial state) and redo everything (must equal the
final state). Shrinking gives minimal failing scripts. Kdenlive's `checkConsistency` played the
same role (`timelinemodel.cpp:6732`).

**6. Differential "frame-array" oracle.** For small timelines (≤ 6 clips, 30 fps, ≤ 300
frames), a naive reference model stores each track as `Vec<Option<(ClipId, source_frame)>>` per
frame and implements insert, overwrite, ripple delete, trim, split and overlap-transition in the
obvious way. Proptest compares `coverage_at` of the real model with the oracle at every frame.
This targets exactly the `±1` bugs seen in MLT's inclusive out points and libopenshot's mixed
rounding.

**7. Render-invariance properties:**
- Split anywhere (outside transitions) never changes `coverage_at` for any frame, including
  source times. This is TR-036's "split never changes any rendered frame".
- Adding then removing an `Overlap` transition is the identity.
- Adding one changes only frames in `[c − D, c)` and shifts later frames by `−D`.

**8. Golden snapshots** (`insta`). An ASCII renderer
(`|A·····|B···|  |T··|`) for scenario scripts gives reviewers readable diffs of what each
op did, plus snapshots of `ChangeSet` debug output for key ops.

**9. Interop fixtures (later, PM-103).** OTIO's `tests/sample_data/transition_test.otio`,
`nested_example.otio` and `simple_cut.otio` are Apache-2.0 and may be vendored with their notice.
Import then export must preserve cut positions, and transition in/out offsets must map 1:1.

**10. Performance guards** (plain `#[test]` with generous bounds, or `cargo bench` later):
`plan_move` on 2 000 clips across 20 tracks stays under 1 ms in release, because it runs on every
pointer move. `coverage_at` must not allocate.

---

## Risks & open questions

1. **Spec reconciliation: transitions** (TL-004 vs FX-002, AU-029).
   - TL-004 says clips never overlap. FX-002 says "the two clips now overlap", and its model notes
     derive "clip positions from clip lengths minus overlaps", with transitions keyed by left clip.
   - This design keeps TL-004 literally and implements FX-002's overlap as trim + ripple + handles
     (§6). The result looks the same to the user.
   - FX-002 and AU-029 model notes should be updated to:
     `Transition { id, from, to, in_offset, out_offset, placement, payload }` on the track;
     `RationalTime` → `Tick`; one shared geometry for video transitions and audio crossfades.
2. **Spec reconciliation: length vs speed** (TL-011 vs TR-036). TL-011 derives D from T
   (`floor(T·s)`); TR-036 derives T from D (`round(D/r)`). Proposal: speed edits keep D and
   recompute T; trims set T and recompute D (§5). Both spec texts should say so.
3. **Reverse mapping off-by-one** (TR-036). `s(t) = out − t·r` hits the exclusive end at `t = 0`
   (§5). TR must choose interval-start sampling for reversed clips.
4. **Id allocator scope** (TL-006, TL-023 say "per-sequence counter"). This design recommends one
   allocator per timeline (§4). Needs sign-off.
5. **Group storage** (TL-039 stores groups twice). This design stores `Clip.group` only (§9).
6. **`RationalTime` in other specs.** FX, TR and TX templates use `RationalTime`. Proposal: it
   means `Tick` everywhere, and there is no second time type.
7. **Conform to frame grid can collide** (TL-001, PM-011). Rounding each start and end with the
   same monotonic function keeps butted clips butted and never creates overlaps. But "clips that
   would shrink to 0 frames are kept at 1 frame" can push into the next clip. Options:
   - steal the frame from the following gap if there is one;
   - otherwise ripple that track by one frame (the main track always ripples anyway).
   Transitions, markers and keyframes are conformed in the same op. Needs a decision.
8. **Ruler labels vs timecode** (TL-002). At NTSC rates, wall-clock labels and the `HH:MM:SS:FF`
   frame count disagree (§3.6). Suggestion: both come from `Timecode`.
9. **Inexact stream bases** (`1/15360`, `AV_TIME_BASE`). Exactness is lost below 1.5 ns. That is
   safe only if `freecut-media` follows §3.5 (convert into ticks, compare in tick space, invert
   with `stream_ts_at_or_before`). Add a media-crate test that steps through every frame of a
   `1/15360` fixture at exact tick boundaries.
10. **Caption track kind** (TX-025). Accepted here as `TrackKind::Caption`, a band above text,
    exempt from auto-removal. TL-004's table must be updated. Caption segments are modeled as
    clips with `ClipContent::Caption`, and TX should confirm.
11. **Index rebuild cost.** A full O(N) rebuild after every apply is fine for thousands of clips.
    It may need to become incremental for very large projects. Measure before optimizing.
12. **`ChangeSet` memory.** Mitigated by `ShiftClips` and `Arc` bags. Still, an op that rewrites
    many large property bags (Paste attributes, TL-034) needs a size estimate and must respect
    the 512 MB budget.
13. **Background-job clip locks** (inventory §18). These are kept out of the model. The app must
    guard planners, and a test should prove that a running job can't be edited out from under
    itself.
14. **Kdenlive license nuance.** Kdenlive is "GPL-3.0-only OR KDE-accepted later versions". It
    is fine to study and compatible with GPL-3.0-or-later as a combination, but we copy no code
    from it (nor from Olive or Shotcut). Only OTIO (Apache-2.0) semantics and its sample files
    are candidates for direct reuse, with attribution.
15. **Disclosure.** During research, one `crates.io` metadata request (for the `vtc` crate) was
    sent with a User-Agent string that contained part of the user's email handle. It was
    unnecessary and has not been repeated. No other user data left the machine.

---

## References

**FreeCut specs:**
- `docs/features/timeline-editing.md`: TL-001…TL-047.
- `docs/features/effects-transitions-animations.md`: FX-001, FX-002, FX-007, FX-008, FX-009.
- `docs/features/clip-transform-and-speed.md`: shared conventions, TR-036, TR-037, TR-043, TR-044.
- `docs/features/audio.md`: time conventions, AU-029.
- `docs/features/text-and-captions.md`: TX-025.
- `docs/features/project-and-media.md`: PM-005, PM-006, PM-007, PM-011, PM-021, PM-035, PM-040, PM-103.
- Behavior inventory (paraphrased): `inventory/timeline-editing.md` §1–§2, §9, §13, §18, and "Notable limits".

**OpenTimelineIO** (Apache-2.0):
- `src/opentime/rationalTime.h`
- `src/opentime/rationalTime.cpp`
- `src/opentime/timeRange.h`
- `src/opentimelineio/item.cpp`
- `src/opentimelineio/composition.cpp`
- `src/opentimelineio/track.h`
- `src/opentimelineio/track.cpp`
- `src/opentimelineio/stack.cpp`
- `src/opentimelineio/transition.h`
- `src/opentimelineio/transition.cpp`
- `src/opentimelineio/marker.h`
- `src/opentimelineio/timeline.h`
- `src/opentimelineio/typeRegistry.h`
- `src/opentimelineio/typeRegistry.cpp`
- `src/opentimelineio/CORE_VERSION_MAP.cpp`
- `src/opentimelineio/algo/editAlgorithm.h`
- `src/opentimelineio/algo/editAlgorithm.cpp`
- `docs/tutorials/time-ranges.md`
- `docs/tutorials/otio-serialized-schema.md`
- `tests/sample_data/*.otio`
- `tests/test_opentime.py`

**Kdenlive** (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL):
- `src/timeline2/model/timelinemodel.cpp`
- `src/timeline2/model/timelinemodel.hpp`
- `src/timeline2/model/trackmodel.cpp`
- `src/timeline2/model/trackmodel.hpp`
- `src/timeline2/model/clipmodel.cpp`
- `src/timeline2/model/clipmodel.hpp`
- `src/timeline2/model/groupsmodel.cpp`
- `src/timeline2/model/groupsmodel.hpp`
- `src/timeline2/model/snapmodel.cpp`
- `src/undohelper.hpp`
- `src/utils/gentime.h`
- `src/utils/gentime.cpp`
- `src/definitions.h`
- `src/project/projectmanager.cpp`
- `src/doc/kdenlivedoc.h`
- `src/doc/kdenlivedoc.cpp`

**Shotcut** (GPL-3.0-or-later):
- `src/models/multitrackmodel.cpp`
- `src/commands/timelinecommands.h`
- `src/commands/undohelper.h`

**MLT** (LGPL-2.1-or-later):
- `src/framework/mlt_types.h`
- `src/framework/mlt_producer.c`
- `src/framework/mlt_playlist.c`
- `src/framework/mlt_property.c`
- `src/framework/mlt_profile.h`

**libopenshot** (LGPL-3.0-or-later):
- `src/ClipBase.h`
- `src/Clip.cpp`
- `src/Timeline.cpp`
- `src/Fraction.h`

**Olive** (GPL-3.0-or-later):
- `app/node/output/track/track.h`
- `app/node/output/track/track.cpp`
- `app/node/block/block.h`
- `app/node/block/clip/clip.h`
- `app/node/block/clip/clip.cpp`
- `app/node/block/transition/transition.h`
- `app/node/block/transition/transition.cpp`
- `app/timeline/timelineundopointer.cpp`
- `app/timeline/timelineundosplit.cpp`
- `app/widget/timelinewidget/tool/transition.cpp`
- `app/widget/timelinewidget/timelinewidget.cpp`
- `.gitmodules` (`ext/core` = olive-core, not present in the clone)

**Public background:** the "flicks" time unit (705 600 000 per second), and SMPTE ST 12-1
drop-frame counting rules (the facts only; our implementation is integer-based and our own).
