# Timeline & core editing

> Status: draft · Owns: time representation, sequences, tracks (kinds, layering bands, flags, colors), clips (names, links), selection, undo/redo semantics, batch-edit scopes and skip reporting (TL-053), per-clip operation availability (TL-054), markers, groups, compound clips and clip snippets (TL-055), clipboard and attribute clipboard (Paste attributes) · Crates: freecut-timeline, freecut-app, freecut-media (thumbnails/waveforms only), freecut-project (persistence of the model)

## Summary

The timeline is where a FreeCut user assembles a video. Media goes onto a stack of
tracks: one magnetic main track for the primary footage, overlay tracks for picture-in-picture,
text, stickers, effects and adjustments, caption tracks on top, and audio tracks for music and
voice. Clips can be
moved, trimmed (one or several at once), split and joined again, deleted with or without
closing gaps, duplicated, frozen, grouped, nested and saved as reusable snippets, and every one
of those edits can be undone. This spec also defines the shared vocabulary
other areas build on: how time is stored, what a track and a clip are, how selection works and
how the undo stack behaves.

Conventions used below:

- "Frame" means one frame at the **sequence frame rate** (project setting, owned by
  [project-and-media.md](project-and-media.md)).
- Key bindings: the default keymap in UI-031
  ([workspace-and-shortcuts.md](workspace-and-shortcuts.md)) is canonical. Keys named here
  either repeat UI-031's binding or are proposals for commands UI-031 has not bound yet. Where
  an earlier proposal of this spec differed from UI-031, the entry now says "see UI-031"
  instead of naming a key. `Mod` = Ctrl on Windows/Linux, Cmd on macOS; `Alt` = Option on
  macOS.
- "(FreeCut default)" marks a value we chose because public information was missing or
  because we deliberately differ.
- When a feature refers to behavior from a feature with a later priority (for example an mvp
  edit mentioning attached items, TL-029, or locked tracks, TL-009), that behavior applies only
  once the later feature ships. Until then the mvp op simply has nothing to do for it.

## Features

### TL-001 Rational time & frame grid — mvp

**What the user sees:** Nothing directly. Edits always land exactly on frame boundaries, audio
stays in sync after hours of edits, and positions never drift through rounding.

**Inputs / controls:** None. The sequence frame rate comes from project settings (PM). A
sequence frame rate is a positive rational `num/den` whose frame duration is a whole number of
ticks (see below). All common rates qualify: 23.976 (24000/1001), 24, 25, 29.97 (30000/1001),
30, 48, 50, 59.94 (60000/1001), 60, 120. Other rates are refused by the project-settings
validator (FreeCut default). Source media may have any rate.

**Behavior / output:**
- All model time values are integer **ticks** in a fixed sequence timebase of
  **1/705,600,000 s** (FreeCut default). This tick rate divides evenly into every common video
  frame duration (including the NTSC 1001-denominator rates) and every common audio sample rate
  (8 k, 11.025 k, 16 k, 22.05 k, 32 k, 44.1 k, 48 k, 88.2 k, 96 k), so frame and sample positions
  are exact integers. An `i64` of these ticks covers roughly 400 years, which is far more than any timeline needs.
- Every user-facing edit point (move, trim, split, marker, playhead) is **quantized to the
  sequence frame grid**: `round_half_down(t / frame) * frame` (FreeCut default rounding:
  nearest frame, ties go to the earlier frame). The only planned exception is TL-050 (later):
  edit points of audio-only clips may land on the audio sample grid instead.
- Source media timestamps arrive in arbitrary container timebases (for example 1/90000 or
  1/15360). They are converted to ticks by exact rational multiplication, rounding to the
  nearest tick (ties to even). The error is under one nanosecond, and source frame selection
  then rounds to the source's own frame grid in `freecut-media`.
- Floats (`f32`/`f64`) appear only at the UI edge: pixel positions, zoom factors, and display of
  seconds.

**Edge cases:**
- Sequence frame rate changed after editing (PM operation): clip positions keep their absolute
  tick values. The change offers a **Conform to frame grid** step that rounds every clip start
  and end to the new grid in one undoable operation. Clips that would shrink to 0 frames are
  kept at 1 frame.
- Clip whose source frame rate differs from the sequence (for example a 25 fps clip in a 30 fps
  sequence): the timeline length is still measured in sequence frames, and each sequence frame
  shows the source frame at or before that instant. Frame mapping is owned by `freecut-media`
  and `freecut-render`.
- Variable-frame-rate sources: their duration comes from the container (PM). Trim limits use
  that duration.
- Arithmetic overflow: all tick math uses checked operations. An overflow returns an
  `EditError::TimeOverflow` and leaves the document unchanged; it is never a panic.
- Negative times are invalid in the sequence. Anything that would start before 0 is clamped to
  0 or refused, as each operation below specifies.
- A project file declaring a sequence rate whose frame duration is not a whole number of ticks
  (hand-edited or corrupt): the loader refuses it with a clear error rather than guessing a
  rate (PM).

**Undo:** n/a (representation). Conform to frame grid is one undoable edit.

**Model notes:** `freecut-timeline::time`: `Tick(i64)`, `TimeBase` (const 705_600_000/s),
`FrameRate { num: u32, den: u32 }`, `TimeRange { start: Tick, duration: Tick }` (duration > 0),
`fn quantize(t, rate) -> Tick`, and checked add/sub. Conversion helpers
`from_stream_ts(ts, num, den)` live in the timeline crate (pure math). `freecut-media` calls them,
so the timeline never depends on media.

### TL-002 Timecode display & ruler — mvp

**What the user sees:** A time ruler along the top of the timeline, with labels that adapt to
the zoom level (hours, then seconds, then individual frames). The current playhead position and
total duration appear near the preview as `HH:MM:SS:FF`.

**Inputs / controls:**
- Time display format: `HH:MM:SS:FF` (default, and the only format in mvp). `HH:MM:SS.mmm`
  and frame count are v1 options in preferences (UI).
- Drop-frame display for 29.97 and 59.94 sequences (v1): off by default; can be switched on in
  preferences (FreeCut default).
- Ruler label spacing: at least 80 px between major labels (FreeCut default). Minor ticks
  subdivide the major interval into 2, 5 or 10 parts.

**Behavior / output:** Labels are computed from ticks through `f64` only for layout. Clicking
or dragging in the ruler seeks the playhead (TL-013). The ruler also shows timeline markers
(TL-037) and the in/out range (TL-038).

**Edge cases:**
- At extreme zoom-out (24 h visible), labels show whole hours only.
- At extreme zoom-in, every frame gets a tick and labels show `SS:FF`.
- Sequences longer than 99 h display a 3-digit hour field rather than wrapping.
- Changing the sequence frame rate changes the meaning of `FF`. Labels re-render and positions
  stay the same.

**Undo:** n/a (view state).

**Model notes:** Formatting and ruler layout live in `freecut-app`. `freecut-timeline` exposes
`Tick -> (h, m, s, frame)` helpers so the format is testable without UI.

### TL-003 Sequence & timeline duration — mvp

**What the user sees:** One timeline per project. Its length is the end of the last clip on any
track. An empty timeline shows a drop hint ("Drag media here") and has a duration of 0.

**Inputs / controls:** None directly. The sequence inherits frame rate, canvas size and audio
sample rate from project settings (PM).

**Behavior / output:**
- `duration = max(item.end)` over all clips on all tracks, plus caption segments on caption
  tracks (TL-004). This includes hidden, muted and disabled items, because they still occupy
  time (FreeCut default).
- Preview and export cover `[0, duration)` unless an in/out range limits export (TL-038, EX).
- Compound clips (TL-040) own nested sequences of the same type.

**Edge cases:**
- Empty sequence: duration 0. Export and playback are disabled, and the playhead is pinned at 0.
- Trailing empty space on overlay tracks after the last clip is not part of the duration.
- Leading gaps (an overlay clip starting at 10 s with an empty main track) still count: the
  duration is that clip's end, and frames before 10 s render the canvas background.
- The model has no hard length limit beyond i64. The UI warns once a sequence exceeds 12 h
  (FreeCut default).
- A sequence whose last item ends between two frames (only possible with TL-050's sub-frame
  audio edits, later): `duration` stays exact. Frame-based consumers (playhead clamp, ruler
  end, export length) round it up to the next frame. The trailing partial frame shows the last
  picture or the background, and audio after the true end is silent.

**Undo:** n/a (derived value). Every change to a `SequenceFlags` bit is an undoable history
entry (`SetSequenceFlag { flag, before, after, content_changes }`), so the dirty flag and the
saved file always agree. Flags that change content when toggled (magnet compaction TL-005,
attachment rebuild TL-029 including its per-kind set, stacking normalization TL-007) carry
those changes in the same entry; the others (ripple all tracks, markers follow ripple) carry
none. Master audio settings
in `Sequence.audio` change only through AU's undoable ops (`SetMasterGain`, and AU-010's other
master ops).

**Model notes:** `Sequence { id, settings_ref, tracks: Vec<Track>, audio_tracks: Vec<Track>,
markers, groups, links: BTreeMap<LinkId, Link> (TL-011), audio: MasterAudio, flags:
SequenceFlags { main_magnet, follow_main, follow_kinds: FollowKinds (TL-029),
flexible_stacking, ripple_all_tracks, markers_follow_ripple } }`. `audio` is a typed property-bag slot whose type `MasterAudio` is
defined by AU ([audio.md](audio.md), AU-010). TL stores it by value and never interprets it.
Only the root sequence's slot drives the master bus; AU decides whether nested sequences use
theirs. Range marks are view state (TL-038), not part of `Sequence`. `Sequence::duration()` is
derived and never stored. `freecut-timeline` defines the container `Timeline { root: SeqId,
sequences: BTreeMap<SeqId, Sequence> }` holding the root sequence plus nested sequences from
compound clips; `freecut-project` only persists it.

### TL-004 Track model: main track, overlay tracks, kinds & layer order — mvp

**What the user sees:** Horizontal lanes stacked vertically.
- The **main track** holds the primary footage. It has a slightly taller, highlighted header.
- Video overlay tracks sit above the main track. Text, sticker, effect and adjustment tracks sit
  above those, and caption tracks sit at the very top.
- Audio tracks sit below the main track.
- Higher visual tracks draw on top of lower ones.

**Inputs / controls:** Track header per track: kind icon, name, a color stripe (TL-049), and
toggles for mute and hide (TL-008, mvp) and lock (TL-009, v1). Track creation is mostly
automatic (TL-006).

**Behavior / output:**
- Track kinds and the items they accept (FreeCut default):

  | Kind | Accepts | Owned content spec |
  |---|---|---|
  | `Video` | video clips, still images, freeze frames, compound clips | this spec, TR |
  | `Text` | text/title items | TX |
  | `Sticker` | stickers, shapes, animated graphics | OV |
  | `Effect` | effect items that affect layers below | FX |
  | `Adjustment` | filter and adjustment items that affect layers below | CL |
  | `Caption` | caption segments only (TX-025); no clips | TX |
  | `Audio` | audio clips (music, voice, detached audio, sound effects) | AU |

- There is **exactly one main track**. It is a `Video` track and is always the lowest visual
  layer. It cannot be deleted, renamed away, or moved, though it can be muted, hidden or locked.
- **Visual bands**, bottom to top:
  1. **Video band:** `Video` tracks, with the main track lowest.
  2. **Upper band:** `Text`, `Sticker`, `Effect` and `Adjustment` tracks.
  3. **Caption band:** `Caption` tracks.

  Within a band, tracks can be in any order.
- **Strict layering** (default): the bands are kept in the order above. Flexible stacking
  (TL-007) merges the video and upper bands, so any non-main video or upper-band track can sit
  at any height above the main track. The **Caption band stays on top even under Flexible
  stacking**, so captions are never covered by overlays (TX-025). A user who wants a caption
  under other layers converts it to a text clip (TX-035).
- Visual **render order** is TL's validated bottom-to-top track order. FX-001 composites in that
  order and therefore draws the Caption band after all other visual tracks. Effect and
  adjustment items apply to the composite of everything below them at that time. Compositing
  details are owned by FX.
- Audio tracks are mixed regardless of order. Their order is only presentational (AU).
- A track is a single lane: **the stored ranges of items on one track never overlap**,
  including at transitions. A transition (FX-002) is an object on the cut point between two
  touching clips A|B (`A.end == B.start`). Its placement in terms of stored ranges:
  - **Add (duration `D`):** B's **source in-point** advances by `D`. B's timeline start stays
    at the cut, B becomes `D` shorter, and every later item on the track ripples −`D` (other
    tracks follow the usual ripple rules, TL-021; attached items follow their owners, TL-029).
  - **Render:** during `[cut − D, cut)`, which is A's last `D` frames, the track shows A
    against B's source `[in − D, in)` (with `in` = B's new in-point). The head frames B gave up
    are therefore shown inside the transition, not lost.
  - **Remove:** exactly the reverse. B's in-point moves back by `D`, B grows by `D`, and later
    items ripple +`D`.
  - FX-002 owns the transition object, its duration limits and its rendering. FX-008's
    length-keeping mode keeps stored ranges disjoint too: it takes the `D` frames from unused
    source before B's in-point instead of from B's visible head, so nothing moves.
- Limit: 100 tracks per sequence, caption tracks included (FreeCut default). Beyond that,
  creating a track fails with a toast that names the limit ("Track limit reached (100)"). Every
  drop, move, paste or command that would need a new track is then refused as a whole, never
  half-applied. This bounds UI and compositor cost. (The reference app, as observed, caps only
  the number of video tracks, with an undisclosed number; FreeCut caps all kinds together.)

**Edge cases:**
- Empty main track with content on overlays: the frames render the canvas background (TR/FX).
- No content prerequisite on the main track: text, stickers, effects, adjustments and audio can
  be added and edited while the main track is empty (FreeCut default). The reference app, as
  observed, refuses adding text and some effects or tools until the main track holds a video or
  photo; FreeCut has no such gate, because overlays over the canvas background are a valid
  composition.
- Dropping an item on an incompatible track redirects it to the nearest compatible track, or
  creates one (TL-006). It is never converted. For example, a text item dropped on a caption
  track goes to the nearest `Text` track; turning text into captions or back is TX's explicit
  command (TX-035).
- A project file whose main track is missing or not lowest (corrupt or hand-edited): the loader
  (`freecut-project`) repairs it by promoting the lowest `Video` track, or by creating an empty
  one, and logs a warning.
- A project file with a caption track below another visual track (hand-edited): the loader
  moves it to the top of the caption band, keeping the relative order of caption tracks, and
  logs a warning.
- Locked and hidden states on the main track behave like any other track.
- Caption tracks have a hide toggle and a lock, but no mute (they carry no sound).

**Undo:** Track structure changes are undoable (TL-006, TL-007). Track flags, color and track
audio change through `SetTrackFlags` (TL-008), `SetTrackColor` (TL-049) and AU's
`SetTrackGain` / `SetTrackPan` (AU-013). Transition placement is part of FX-002's
`AddTransition` / `RemoveTransition` change sets. The table above is static.

**Model notes:** `Track { id: TrackId, kind: TrackKind, name: Option<String>, color:
Option<PaletteColor> (TL-049), flags: TrackFlags { muted, hidden, locked }, audio: TrackAudio,
items: TrackItems }`, where `TrackItems::{Clips(Vec<Clip>) /* sorted by start */,
Captions { data: CaptionTrackData, segments: Vec<CaptionSegment> } }`. `Captions` is used if
and only if `kind == Caption`. TX-025 defines `CaptionTrackData` and `CaptionSegment`; TL
applies its generic rules to segments (sorted, no overlap, at least 1 frame long, frame-grid
edit points, shifted by ripple edits, Lift/Extract and Conform to frame grid). Caption-specific
editing (split, merge, text) is TX's (TX-026, TX-027).
`audio` is a typed property-bag slot whose type `TrackAudio` (gain, pan) is defined by AU
(AU-013). Every track carries it with AU's defaults, and only audio-bearing kinds (`Video`,
`Audio`) use it; AU's ops refuse other kinds. Track height is view state kept outside the edit
model (TL-010).
`Sequence.tracks` is ordered bottom-to-top for visual kinds, with audio stored in a separate
`audio_tracks` vec ordered top-to-bottom. Invariants checked by `Sequence::validate()` and
property tests:
1. Exactly one main track, and it is the lowest video track.
2. No overlaps between stored item ranges within a track.
3. Item kind matches track kind (`Caption` tracks hold only segments; other kinds only clips).
4. Band order holds: strict layering unless `flexible_stacking` is set, and the Caption band
   is on top in both modes.
5. Main track contiguous from 0 while `main_magnet` is on.
6. Every transition sits on a cut between two touching clips of its own track, within FX-002's
   limits.

### TL-005 Magnetic main track — mvp

**What the user sees:** With the magnet on (default), the main track behaves like a string of
beads:
- Clips always butt up against each other starting at 0:00.
- Dropping a clip inserts it between two clips and pushes the rest later.
- Deleting or shortening a clip pulls the rest earlier.

A magnet toggle sits in the timeline toolbar. Turning it off lets main-track clips sit anywhere
and leave gaps, like any other track.

**Inputs / controls:** Toggle button in the timeline toolbar (UI-011) and the Timeline menu
(UI-012); key binding: see UI-031. Stored per sequence. The default for new projects comes from
preferences (default on). Each toggle shows a short confirmation toast with the new state
("Magnetic main track on" / "off"; UI-040), because the key toggles it silently otherwise.

**Behavior / output (magnet on):**
- Insert position for a drop or move onto the main track: the cut point nearest the pointer.
  If the pointer is over a clip, its first half means "before" and its second half means
  "after". Past the last clip means "append". An insertion bar shows the target.
- Inserting shifts all later main-track clips right by the inserted duration. Removing (delete,
  move away, trim) shifts them left.
- Attached items on other tracks follow the main-track clips they belong to (TL-029).
- Trimming a main-track clip is always a ripple trim (TL-020).

**Behavior / output (magnet off):** Main track = ordinary single-lane track (non-ripple moves,
gaps allowed, collisions refused) that stays the lowest layer.

**Switching on with gaps present:** The main track is compacted. All gaps are closed and the
first clip moves to 0, in order. Attached items shift with their clips. A toast reports
"Closed N gaps". This keeps the invariant "magnet on ⇒ contiguous from 0".

**Edge cases:**
- Magnet on and main track empty: the first dropped clip lands at 0 regardless of pointer.
- Main track locked: magnetic inserts and ripples are refused (toast "Main track is locked").
  Overlay edits are unaffected.
- A main-track clip moved onto an overlay track: the main track closes the hole. The clip lands
  at the pointer time on the overlay.
- An overlay clip moved onto the main track: it is inserted, so its time snaps to a cut point,
  not the pointer.
- Turning the magnet on when compaction would move clips on locked tracks: attached items on
  locked tracks stay put and are flagged out-of-sync (TL-029). Compaction itself still runs,
  unless the main track is locked, in which case the toggle is refused.
- Main track locked, other cases (FreeCut default): switching the magnet **off** is always
  allowed, because it moves nothing; switching it **on** is allowed when the main track is
  already contiguous from 0 (no compaction needed). The reference app, as observed, blocks the
  toggle in both directions while the main track is locked.
- Deleting the last main-track clip is allowed and leaves an empty main track (TL-025). The
  reference app, as observed, never lets the main track become empty; FreeCut lets it
  be empty, since an empty main track is already valid in TL-003 and TL-004.
- Transcript-based editing (AI-011, later): its word deletions are always ripple range-deletes
  on the edited clips, whatever the magnet state, and the flag itself is not changed (FreeCut
  default; the reference app, as observed, forces the magnet on while text-based editing is in
  use).

**Undo:** Toggling is an undoable `SetSequenceFlag` (TL-003). When switching on, it carries
the compaction change set. Undo restores the previous positions and the flag.

**Model notes:** The magnet is a `SequenceFlags` bit. Magnetic behavior is implemented as a
policy inside the edit operations (insert, move, trim, delete), not as a post-pass. That keeps
each op's change set exact and invertible.

### TL-006 Add, auto-create & remove tracks — mvp

**What the user sees:**
- Tracks appear when needed: dropping a clip above the top track, or onto an occupied span,
  creates a new track.
- Tracks disappear when emptied.
- A track header context menu offers **Add track above/below** and **Delete track**.

**Inputs / controls:**
- Drop zones: the empty area above the top visual track and below the last audio track.
- Context menu items: Add Video / Text / Sticker / Effect / Adjustment / Caption / Audio
  track.
- Delete track. It asks for confirmation if the track holds clips or caption segments; the
  dialog states how many items go with it ("Delete track and its 7 clips?"). An empty track is
  deleted without asking. With several tracks selected (TL-015), one confirmation covers all
  of them and the deletion is one history entry.

**Behavior / output:**
- New tracks get default names ("Video 2", "Text 1", "Captions 1", "Audio 3"), default height
  (TL-010) and the *Default* color, except overflow tracks, which take the color of the track
  the item was heading for (TL-049). Under strict layering, the new track is placed in
  the correct band at the position nearest the drop. A new caption track always goes into the
  Caption band (TL-004), in either stacking mode.
- Caption tracks are also created by TX-025's *Add caption* when none exists.
- **Auto-remove (FreeCut default):** at the end of any edit transaction, every non-main track
  that is empty **and unlocked** is removed as part of the same undo step. Locked empty tracks
  persist. **Empty `Caption` tracks are exempt** and persist, because the track itself carries
  the language, style and rules that new segments use (TX-025). *Delete track* still removes
  them.
- Deleting a track with clips deletes those clips. Items attached to them are handled per
  TL-029, and groups lose those members. Deleting a caption track deletes its segments, and
  speech clips linked to those segments go with them (TL-011, TX-043).

**Edge cases:**
- Main track: Delete is disabled.
- Track limit (100) reached: auto-create fails and the drop is refused with a toast.
- An edit that would leave the sequence with no audio tracks is fine: zero audio tracks is valid.
- Removing a track between two others leaves order otherwise unchanged.
- Undo of an edit that auto-removed a track restores the track with its original id, name,
  flags and position.

**Undo:** `AddTrack`, `RemoveTrack { track snapshot incl. clips }`. Auto-removal is folded into
the triggering op's change set.

**Model notes:** Track ids are stable `u64`s allocated from a per-sequence counter, which is
persisted so ids never get reused after undo or redo.

### TL-007 Reorder tracks & flexible stacking — v1

**What the user sees:** Track headers can be dragged up and down to change stacking order.
A sequence setting, **Flexible stacking**, allows video overlay, text, sticker, effect and
adjustment tracks at any height above the main track. For example, a title can be tucked
beneath a video overlay, which then hides it where they overlap. Caption tracks always stay on
top (TL-004).

**Inputs / controls:**
- Drag a track header (4 px threshold).
- Context menu: Move track up/down.
- Project settings panel: *Flexible stacking* checkbox (default off), stored in the root
  sequence's flags; nested sequences follow the root's value. A global preference sets the
  default for new projects.

**Behavior / output:**
- Strict layering: drag is constrained within the track's band (video band, upper band, or
  Caption band). The main track never moves. Audio tracks reorder among audio tracks only.
- Flexible stacking: any non-main track of the video or upper band can move to any position
  above the main track and below the Caption band. Caption tracks reorder only among caption
  tracks, in both modes. Effect and adjustment items then affect only tracks below them (FX).
- Render order is always TL's validated bottom-to-top order (TL-004). FX-001 reads it and adds
  no stacking rules of its own.
- Turning Flexible stacking **off** is allowed only if the current order already satisfies
  strict layering. Otherwise FreeCut offers **Normalize order**, which moves upper-band tracks
  above all video tracks while keeping their relative order, as one undoable step. The Caption
  band is already on top and is untouched. (The reference app, by public accounts and by its
  observed UI, does not let the equivalent setting be switched back off once the project is
  saved; FreeCut deliberately allows it.)
- Turning Flexible stacking **on** changes no track flags: locked tracks stay locked and hidden
  tracks stay hidden (FreeCut default). The reference app, as observed, unlocks and unhides every
  track when its equivalent mode is switched on; FreeCut leaves that to the user, since lock and
  hide are deliberate choices (TL-008, TL-009).

**Edge cases:**
- Reordering a locked track is allowed, because locking protects the clips, not the stack
  position (FreeCut default).
- Reordering tracks never removes a transition, because a transition's two clips always sit on
  the same track (TL-004) and move with it. Moving one clip of a transition pair to another
  track is a clip move, which removes the transition (TL-018, FX-002); OV-007's layer-order
  commands disable that case with a hint.
- A project saved with Flexible stacking on and opened by a FreeCut build that predates TL-007:
  the project's format version (PM-021) is newer than that build understands, so it opens
  read-only instead of silently flattening the stack order.
- Groups and attachments are unaffected by track reorder.
- A drag dropped onto its own position is a no-op with no history entry.

**Undo:** `MoveTrack { from, to }`. Toggling is `SetSequenceFlag` (TL-003) carrying the
Normalize order moves when switching off.

**Model notes:** Layering validation lives in `freecut-timeline` (`TrackKind::band()` plus
the `flexible_stacking` flag). The compositor reads the final order only (FX).

### TL-008 Track mute & hide — mvp

**What the user sees:** An eye icon on visual tracks (caption tracks included) and a speaker
icon on audio tracks and on video tracks (for the clips' embedded audio), in every track
header from the mvp on (TL-004, UI-035). Hidden tracks are dimmed and excluded from preview
and export. Muted tracks are silent.

**Inputs / controls:** Click the icons; the same toggles are in the track header context menu
(UI-035) and the inspector's Track tab (UI-008). Alt-click a mute icon to mute every other
audio-bearing track ("solo" is owned by AU, see [audio.md](audio.md), AU-012).

**Behavior / output:**
- Exactly two flags per track, `hidden` and `muted`. Both are saved in the project and both
  are undoable.
- Hidden: skipped by the compositor in preview and export (EX-002). Clips stay editable and
  selectable. A hidden caption track is neither shown nor burned in (TX-025).
- Muted: skipped by the mixer in preview and export (AU-011, EX-002).
- On a `Video` track the speaker silences the embedded sound of every clip on it. On the main
  track this is the one-click way to drop all original camera sound under a music bed; each
  clip's own mute (AU-007) is untouched, so unmuting the track brings back exactly the previous
  per-clip states.
- Text, sticker, effect, adjustment and caption tracks have no mute toggle (no sound).
- With several track headers selected (TL-015), clicking one toggle sets that flag on all of
  them in one entry. If their states differ, the toggle shows a mixed state (UI-010) and the
  click turns the flag on for all.

**Edge cases:**
- Hiding the main track shows the canvas background beneath the overlays.
- Hidden Effect or Adjustment tracks: their effects are not applied.
- Hiding every track is valid and exports a background-only video. Export warns first (EX).
- Hidden or muted clips still count toward sequence duration.
- Toggling during playback takes effect at the next rendered frame and mixed block (AU-011
  ramps the audio change).
- Mute and hide on a locked track: the toggles stay usable, because they don't edit clips
  (FreeCut default; TL-009 ships later and keeps this rule).
- Deleting a muted or hidden track and undoing restores the flags with the track (TL-006).
- Analysis and processing tools (AI-001's jobs, such as transcription AI-009 or cutout AI-014)
  still run on clips of hidden or muted tracks (FreeCut default): hide and mute are output
  flags, not data locks. The reference app, as observed, refuses some tools on hidden clips.

**Undo:** `SetTrackFlags { track, before, after }`, one entry per click (an Alt-click or a
multi-track click is one entry covering every changed track). These are undoable because they change the output
(FreeCut default).

**Model notes:** `TrackFlags { muted, hidden, locked }`; `locked` belongs to TL-009 (v1) and
stays `false` until then. Render and audio crates consume the flags through a read-only
`RenderView` and `MixView` of the sequence.

### TL-009 Track lock — v1

**What the user sees:** A padlock icon per track. Locked tracks show a hatched overlay. Their
clips can't be selected, moved, trimmed, split or deleted, and nothing can be dropped onto them.

**Inputs / controls:** Click the padlock. Shift-click locks or unlocks every track of the same
kind (FreeCut default).

**Behavior / output:**
- Locked clips are excluded from: click and marquee selection, select-all, split-all,
  select-forward, ripple shifts, compaction, and paste targets.
- Edits elsewhere that would normally shift a locked clip (main-track ripple moving an attached
  item, or ripple across all tracks) leave it in place. The item is marked **out of sync**: a
  red badge shows the offset in frames, with a "Resync" action available once the track is
  unlocked.

**Edge cases:**
- Main track locked with magnet on: any edit needing a main-track ripple is refused with a toast.
- A drop onto a locked track is redirected to a new track above it.
- A locked empty track is not auto-removed (TL-006).
- Mute, hide, rename and track color stay usable on a locked track, because they don't touch
  its clips (TL-008, TL-010, TL-049).
- Undo and redo of edits made before locking still work. Undo restores the document exactly,
  regardless of the current lock state.

**Undo:** Lock state is part of `SetTrackFlags` (undoable). "Resync" is an undoable move.

**Model notes:** The edit operations check `locked` during validation. Out-of-sync state is
derived: an attachment whose expected position differs from the actual one.

### TL-010 Track height, collapse & naming — v1

**What the user sees:** Tracks can be made taller (more thumbnail and waveform detail) or
shorter. The name label is editable. (Track color is TL-049.)

**Inputs / controls:**
- Drag the divider under a track header. Height range is 24–240 px.
- Defaults (FreeCut default): main 64 px, video overlay 56 px, audio 48 px, other kinds 32 px.
- Shift-drag resizes all tracks of that kind.
- Context menu presets: Small (0.75×), Medium (1×), Large (1.5×). The same three presets are
  offered for all tracks at once from the timeline's `⋯` menu (UI-011).
- Double-click the divider to reset.
- Double-click the name to rename (max 64 chars). An empty name reverts to the default.
- **Waveform zoom** (FreeCut default): 1×, 2×, 4× or 8× (default 1×), per track, in the
  context menu of audio tracks and of video tracks whose clips carry sound. It magnifies the
  drawn waveform vertically so quiet recordings become readable; peaks taller than the band are
  clipped at its edge.

**Behavior / output:** Height affects only the UI. Below 32 px, thumbnails collapse to a single
color bar and waveforms to a thin line. Waveform zoom affects only drawing: volume, meters and
AU-009's gain overlay line keep their true scale.

**Edge cases:**
- A name collision is allowed, because names aren't identifiers.
- Names are trimmed, and control characters are stripped.
- Waveform zoom on a track whose clips have no sound: the item is disabled.

**Undo:** Rename is undoable (`RenameTrack`). Height and waveform zoom are view state (n/a),
but they are persisted in the project's UI section.

**Model notes:** The name is in `Track` (timeline). Height and waveform zoom are in a
view-state map keyed by `TrackId` (`freecut-project` UI section, not part of the edit model).

### TL-011 Clip model — mvp

**What the user sees:** Rectangles on tracks. Each clip shows its name and a thumbnail or
waveform. It can be selected, trimmed and moved, and it carries properties edited in the
inspector (transform, color, audio, speed and so on, owned by other specs).

**Inputs / controls:** n/a (data model).

**Behavior / output:**
- A clip has:
  - id
  - track
  - **timeline range** (start, duration)
  - **content**:
    - Media clips (video, audio, image): `MediaRef` (PM asset id plus stream selection) and a
      **source range** (in point, duration) in source time.
    - Generated clips (text, sticker, shape, effect, adjustment, freeze-frame still): no
      source range, only duration.
    - Compound clips: a `NestedSequenceId` and a source range into it.
  - enabled flag
  - optional name override (changed by `RenameClip`, TL-048)
  - markers, group id, attachment, link id
  - opaque property bags owned by other specs (transform, keyframes, filters, effects, audio
    settings, retime)
- Video clips with an audio stream carry **embedded audio** (one clip, one waveform band). There
  is no separate audio clip until the user detaches it (TL-030).
- Default durations on insert (FreeCut default; owning specs may override): still image 5 s,
  text 3 s, sticker 3 s, effect/adjustment 3 s. Media clips use their full source length.
- Duration ≥ 1 frame. A media clip's source range must lie within the source duration, adjusted
  for speed (the mapping comes from TR,
  [clip-transform-and-speed.md](clip-transform-and-speed.md)). For constant speed `s`, a
  rational owned by TR (never a float): `source_duration = timeline_duration × s`, rounded
  down to a tick.
- **Source limits** come from a `SourceLimits` provider fed with plain data from the project
  (asset durations, playback modes); it does no I/O. It returns, per clip, either the bounded
  source span or **unbounded**:
  - Stills, freeze frames and generated items: unbounded (they have no source range).
  - Animated media whose OV-013 playback mode loops (*Loop* or *Ping-pong*): unbounded. This
    covers animated images (OV-012) and image sequences (OV-017). Their source range is
    measured in *unwrapped* animation time; OV-013 owns the mapping that wraps it into the
    file's natural length, so the in-point may even be negative for these clips only.
  - All other media: bounded by the source duration (after the time map).
- **Links** join items that belong together without grouping them. A link has a kind and two
  or more members, each a `LinkMember::{Clip(ClipId), CaptionSegment(TrackId, SegId)}`:
  - `DetachedAudio`: a video clip and the audio clips detached from it (TL-030).
  - `Speech`: a generated speech clip and the text clip or caption segment it was read from
    (TX-043, AI-025).

  Deleting the text clip or caption segment of a `Speech` link deletes its linked speech clip
  in the same op (TX-043); undo restores both. Deleting only the speech clip leaves the text in
  place. A link left with fewer than 2 members dissolves. Splitting a member (TL-023, TX-027)
  keeps every piece in the link.

**Edge cases:**
- **Missing media** (file moved or deleted): the clip stays, drawn with an "offline" stripe
  pattern and the file name. It remains fully editable. Trims are limited by the last known
  source duration (stored in the asset record). It renders as a "media offline" slate (EX).
  Relinking is PM.
- Source duration unknown (probe still running): a clip can't be created until the probe
  completes (TL-012).
- Zero-length results (for example a trim to nothing) are rejected. The minimum is 1 frame.
- Very short sources (shorter than 1 sequence frame, such as a 1-frame 60 fps clip in a 24 fps
  sequence): the clip is 1 sequence frame long and shows that source frame.
- Stills, generated items and looping animated media have no source limits, so they can be
  extended freely.
- A looping animated clip extended past its natural length, then switched to a non-looping
  OV-013 mode (*Play once, hold last frame* or *Play once, then hide*): OV-013 keeps the clip
  length, and `validate()` accepts the over-length range. What shows past the natural length
  is OV-013's rule. Further lengthening trims are refused until a looping mode is chosen again.
- A link member that disappears without going through a TL op (for example a nested sequence
  removed on load as corrupt): the loader drops it from the link and dissolves the link if
  fewer than 2 members remain.
- FreeCut default: a clip carrying a `DetachedAudio` link that can't be a member of it (anything
  but the video whose sound it holds and audio clips, possible only in a hand-edited file)
  loses the link on load, with a repair notice, so restoring that sound never deletes it.

**Undo:** n/a (structure). Every mutation goes through ops below. Link creation and removal
are part of the ops that create them (`DetachAudio`, TX-043's speech generation) and of the
deletes that dissolve them.

**Model notes:** `Clip { id: ClipId, range: TimeRange, content: ClipContent, enabled: bool,
name: Option<String>, markers: Vec<Marker>, group: Option<GroupId>, attach: Option<Attachment>,
link: Option<LinkId>, props: PropBags }`. `ClipContent::{Media{asset, streams, source: TimeRange},
Generated{kind}, Compound{seq, source: TimeRange}, Still{from: StillSource}}`. Property bags
are typed structs from other specs, stored by value but defined in `freecut-timeline` modules
or behind a trait so the timeline crate stays I/O-free.
`Sequence.links: BTreeMap<LinkId, Link { kind: LinkKind::{DetachedAudio, Speech}, members:
BTreeSet<LinkMember> }>` is the single source of truth for links. `Clip.link` is a cached
back-reference that `validate()` checks against it. Caption segments are found through the
map, so TX-025's segment type needs no link field. Links never cross sequences.
`SourceLimits::for_clip(&Clip) -> Limits { start: Bound, end: Bound }` with
`Bound::{At(Tick), Unbounded}`; the looping case reads the clip's OV-013 `ElementPlayback`.

### TL-012 Add media to the timeline — mvp

**What the user sees:** Media is dragged from the library (or the OS file manager) onto the
timeline, or added with a "+" button on a library item. A ghost of the clip follows the pointer
and shows where it will land.

**Inputs / controls:**
- Drag and drop onto a track or a new-track zone.
- "+" button / `Enter` on a selected library item: **Add to main track**.
- `Shift+Enter`: **Add as overlay at playhead** (FreeCut default).
- Hold `Alt` while dropping on an overlay track: **overwrite** instead of creating a new track
  (v1).

**Behavior / output:**
- **Drop on the main track (magnet on):** inserted at the nearest cut point (TL-005).
- **Drop on an overlay track:** placed at the pointer time (snapped, TL-028). If that span is
  occupied, a new track of the same kind is created directly above and the clip goes there.
  Clips are never silently overwritten.
- **Overwrite drop (Alt, v1):** the span is cleared on that track. Clips fully covered are
  removed, and partially covered ones are trimmed or split. Then the clip is placed.
- **Add to main track:** inserts at the main-track cut point nearest the playhead (ties go to
  the later cut). The playhead then moves to the new clip's end.
- **Add as overlay:** placed at the playhead on the lowest overlay track of the right kind
  with free space, or on a new track.
- Media kinds:
  - Video, with or without audio: one `Video` clip.
  - Audio-only: `Audio` track (first audio track free at that span, else a new one).
  - Image: `Video` track clip with the default still duration.
- Multiple library items are added in library order, back to back.
- Text, stickers, effects and adjustments from their panels follow the overlay rules (placed at
  the playhead on a compatible track with free space).

**Edge cases:**
- Drop before 0: clamped to 0.
- Empty timeline: a video or picture dropped anywhere on the timeline (on a row, the new-track
  zone or the empty space below the tracks) lands on the main track (FreeCut default). Without
  this, a first drop that misses the main row would put the clip on an overlay above an empty
  main track. Audio-only items still go to an audio track.
- Drop on a locked track: redirected to a new track above.
- Item still being probed: the ghost shows a spinner and the clip appears once the probe
  finishes (off the UI thread). If the probe fails, the drop is cancelled with an error toast
  and nothing changes.
- Unsupported or corrupt file: the import is refused (PM). No timeline change.
- Media whose frame rate or sample rate differs from the sequence: accepted without conversion
  (TL-001). Audio resampling happens in `freecut-audio`.
- Dropping onto a track inside an open compound clip edits that nested sequence.
- The track limit is reached and a new track is needed: the drop is refused.
- A drop with no valid landing spot (over the ruler, a track header or the playhead handle):
  the ghost turns red and releasing does nothing.
- A folder dragged from the OS onto the timeline: its supported files are imported into the
  library as a bin through PM-032, nothing is placed, and a toast says so (FreeCut default). The
  user then adds items deliberately, so a large folder never floods the timeline in an
  arbitrary order. The reference app, as observed, simply refuses folder drops on the timeline.
- Several library items added with "+" or `Enter`: one busy indicator covers the whole insert,
  and it is still one history entry.

**Undo:** `InsertClips { clips, ripple_changes, created_tracks }` is one step per drop, even
for multi-item drops.

**Model notes:** Placement policy is a pure function, `plan_insert(seq, request) ->
Result<ChangeSet, EditError>`, so the ghost preview and the real drop use the same code. The
app calls it on every pointer move (cheap) and applies it on release.

**FreeCut defaults (model, mvp):**
- Media without a picture always becomes an audio clip on an audio track, whatever track or
  kind the drop asked for. A video clip of a file without sound carries no sound settings.
- "Add as overlay" and the audio panel's "+" use one placement, *first free*: the first
  unlocked track of the right kind that is free over the whole span, searched from the main
  track outward (overlay tracks bottom-up, never the main track itself; audio tracks
  top-down). If none is free, a new track goes on top of the kind's band, or below the last
  audio track.

### TL-013 Playhead & navigation — mvp

**What the user sees:** A vertical line across all tracks marking the frame shown in the
preview. It can be clicked, dragged and stepped with the keyboard, and it moves during
playback.

**Inputs / controls:**
- Click the ruler or empty timeline area to seek. Drag the playhead head to scrub.
- `←`/`→`: one frame. `Shift+←`/`Shift+→`: 10 frames (FreeCut default).
- `↑`/`↓`: previous/next cut point (any clip start or end on visible, unlocked tracks, plus 0
  and the sequence end).
- `Home`/`End`: start/end.
- Previous/next keyframe of the selected clip: TR-030's keyframe navigation (keys in UI-031).
- Space, J/K/L and the transport itself are owned by EX
  ([preview-and-export.md](preview-and-export.md)).

**Behavior / output:**
- The playhead is always on the frame grid and is clamped to `[0, duration]`. At `duration`
  the preview shows the last frame.
- During playback the view auto-scrolls by **page** (default) or **continuous** (playhead
  centered), set in preferences.
- Scrubbing requests frames asynchronously, so the UI thread never decodes. Audio scrub is AU.

**Edge cases:**
- Empty sequence: the playhead stays at 0 and keys do nothing.
- Edits that shorten the sequence below the playhead clamp it to the new end.
- Next/previous cut point with no cut points in that direction: jump to 0 or the end.
- Inside an open compound clip, the playhead is in nested-sequence time. Leaving maps it back
  to parent time when the compound instance contains it, else it returns to its previous parent
  position.

**Undo:** n/a (view state). The playhead is stored in the project UI section.

**Model notes:** The playhead lives in `freecut-app` view state. The timeline crate offers
`cut_points(seq, filter) -> Vec<Tick>` and `next_cut(t)` and `prev_cut(t)`.

### TL-014 Hover skim preview — v1

**What the user sees:** With *Hover skim* on, moving the pointer over the timeline shows the
frame under the pointer in the preview without moving the playhead. When the pointer leaves the
timeline, the preview returns to the playhead frame.

**Inputs / controls:**
- Toolbar toggle plus key `S` (as in UI-031). The toggle has a small dropdown with three modes
  (FreeCut default): **Off** (default), **Picture only**, **Picture and sound**. `S` switches
  between Off and the last used on-mode.
- *Picture and sound* plays a short snippet of the mixed audio at the skim position, using
  AU-016's scrub audio path. Audio while dragging the playhead is AU-016's own preference and
  is independent of this mode.

**Behavior / output:** Frame requests are throttled to the display refresh rate. If decoding
can't keep up, stale requests are dropped. A thin secondary line marks the skim position. With
the mode Off, hovering never changes the preview.

**Edge cases:** Skimming is disabled during playback and while dragging clips. Over offline
media it shows the offline slate. In an empty area past the end, it shows the last frame. The
blade tool (TL-024) shows the frame under its cut line on its own, so it works the same with
skim Off (the reference app, as observed, instead refuses to turn its skim line off while the
blade is active).

**Toolbar control (toolbar round, 2026-10-13):** the reference editor's timeline toolbar has an
on/off switch for a pointer-following preview line, with a small menu; FreeCut's equivalent is
this feature, so the toolbar shows it as a **split button** (UI-011): the left part is the
toggle (icon: a vertical line with a small play glyph; *checked* while a mode is on), the
chevron opens a menu with three radio items, **Off**, **Picture only** and **Picture and
sound**, then a separator and the checkable **Scrub sound while dragging the playhead**
(AU-016's preference, `Shift+S`, shown here because both decide what is heard while the
pointer moves). The toggle's tooltip names the current mode and key ("Skim: picture only (S)";
FreeCut text). Clicking the toggle does what `S` does: Off ↔ the last used on-mode, which
starts as *Picture only*. The mode is a per-user preference (UI-020) and survives restarts.
While the blade (TL-024) is the active tool the toggle stays clickable; the blade's own cut-line
preview works whatever the mode, so nothing is refused.

**Undo:** n/a (view state).

**Model notes:** App-only. It uses the same async frame request path as scrubbing (EX/render).
`SkimMode::{Off, Picture, PictureAndSound}` plus `last_on: SkimMode` live in the app's
preferences; the skim position is `Option<Tick>` view state that the player reads before the
playhead (`preview_time = skim.unwrap_or(playhead)`), and it is `None` while playing,
dragging a clip or with the pointer outside the lanes and ruler.

Build notes (app, T5, 2026-10-14):
- `freecut_app::skim` holds the pure parts: `Skim { mode, last_on }` (`toggle`
  for `S` and the toggle part, `set_mode` for the menu), `skim_time(skim,
  SkimInput { playing, dragging, pointer, blade })`, and `Throttle`, which sends at
  most one frame request per display refresh (egui's predicted frame time) and
  drops a waiting time that a newer one overtakes. The preview's own request slot
  is latest-wins as well, so nothing queues.
- The timeline reports the frame under the pointer over the ruler and lanes
  (quantized; past the end, the sequence's end, which shows the last frame), the
  blade's cut line, and whether a gesture runs. The blade's cut line drives the
  preview whatever the mode, which finishes TL-024's frame-under-the-cut-line rule.
- The skim line is 1 pt `text.tertiary` over the ruler and lanes.
- *Picture and sound* and *Scrub sound while dragging the playhead* stay hidden
  until AU-016 exists; a settings file that names *Picture and sound* reads as
  *Picture only*. The mode is saved as `timeline.skim` and `timeline.skim_last_on`
  in the user settings (UI-018).
- When space is short, the folded split button is a `⋯` submenu: the toggle
  (with its key), then the modes.

### TL-015 Clip selection — mvp

**What the user sees:** A clicked clip gets a bright outline and its properties appear in the
inspector. Several clips can be selected at once and edited together (move, delete, copy,
group).

**Inputs / controls:**
- Click: select only this clip (plus its group, TL-039).
- `Mod`+click: add or remove from the selection.
- `Shift`+click: select the time/track rectangle from the anchor clip to the clicked clip
  (every clip whose start lies in the time span, on tracks between the two).
- `Alt`+click: select a single member of a group.
- `Mod+A`: select all clips on unlocked tracks. `Mod+Alt+A`: select all on the clicked track
  (FreeCut default); the track header context menu offers the same as *Select all clips on this
  track*.
- *Select clips with this color* (TL-045, later) and *Select forward / backward* (TL-017).
- `Esc`, or click on empty space or on the preview canvas outside every item: clear selection.
  The inspector then shows the Project tab (UI-008).
- **Track selection (v1):** clicking a track header selects that track, not its clips, and
  `Mod`+click adds or removes tracks. Selecting tracks clears the clip selection and the other
  way round. Track commands (lock, hide, mute, color, delete, close all gaps) then act on every
  selected track as one history entry, and the inspector's Track tab reads "3 tracks selected"
  (FreeCut text), following UI-010's mixed-value rules.

**Behavior / output:**
- Selection can contain clips, caption segments (TX-025; kept in sync with the Captions panel,
  TX-026), markers, gaps (TL-027) and transitions (FX), but is homogeneous for clip-only
  commands. Commands ignore non-applicable members, and the inspector shows shared
  properties when multiple clips are selected. A selection may mix kinds and span any number
  of video, overlay and audio tracks.
- Multi-selection editing in the inspector is UI-010's: only shared properties are shown, a
  value that differs across the selection shows "—", typing sets it on every item, a boolean on
  for only some items shows a mixed state, and text content is hidden (text styling stays
  editable). Every such edit is one history entry (TL-053).
- **Empty-selection guards:** a command that needs a target and has none is disabled, and its
  tooltip says what to select ("Select a clip first", "Select a track first"; FreeCut text,
  UI-011). Invoked from a key, it shows the same reason in the status bar and changes nothing.
- Selection survives edits when its clips survive. Clips removed by an edit leave the selection.
  After split, the right-hand part is selected (TL-023).

**Edge cases:**
- Clips on locked tracks can't be selected. Locking a track removes its clips from the
  selection.
- Hidden-track clips are selectable.
- A selection spanning the main and overlay tracks is valid. Each command defines how it treats
  mixed selections.
- A selection inside an open compound clip is separate from the parent selection.

**Undo:** Selection changes are **not** history entries. Each edit op records the selection
before it, and undo restores that selection (FreeCut default).

**Model notes:** `Selection { clips: BTreeSet<ClipId>, segments: BTreeSet<(TrackId, SegId)>,
markers, gaps: Vec<GapRef>, tracks: BTreeSet<TrackId> /* v1; exclusive with the rest */,
anchor: Option<ClipId> }` lives in `freecut-app`, but
selection-resolution helpers (expand to groups,
rectangle selection, filter locked tracks) are pure functions in `freecut-timeline` with tests.
History entries store the pre-op selection as a plain id snapshot (`SelectionSnapshot`, a
timeline type made of `ClipId`/`MarkerId` sets), so the history never depends on app types.

### TL-016 Marquee selection — v1

**What the user sees:** Dragging on empty timeline space draws a translucent rectangle. Every
clip it touches becomes selected.

**Inputs / controls:** Drag from empty space (4 px threshold). `Mod`+drag adds to the existing
selection. `Alt`+drag removes from it (FreeCut default).

**Behavior / output:**
- Any clip intersecting the rectangle is selected, together with its group members.
- The view auto-scrolls when the pointer nears the timeline edges (speed proportional to
  distance, up to 2 000 px/s).

**Edge cases:** A drag that starts on a clip is a move, not a marquee. Locked tracks are
skipped. A zero-area rectangle (click) clears the selection.

**Undo:** n/a (selection).

**Model notes:** `clips_in_rect(seq, time_range, track_span) -> Vec<ClipId>` in the timeline
crate.

### TL-017 Select forward / backward from playhead — v1

**What the user sees:** One command selects everything after the playhead (or before it), so a
whole tail of the edit can be pushed later or earlier at once.

**Inputs / controls:**
- Commands *Select forward* and *Select backward* (all unlocked tracks), plus *on current
  track only* variants. Proposed keys `]` / `[` for the all-track commands; UI-031 decides and
  has not bound them yet. The current-track variants are unbound by default, because UI-031
  uses `Alt+[` / `Alt+]` for keyframe navigation.
- Toolbar tool **Select forward**: clicking a clip selects it and every clip starting at or
  after its start on all unlocked tracks. `Alt`+click limits this to that track.

**Behavior / output:** Forward = clips with `end > playhead`. Backward = clips with
`start < playhead`. Clips crossing the playhead are included in both.

**Edge cases:** Empty result clears the selection. Group expansion applies. Locked tracks are
skipped.

**Undo:** n/a (selection).

**Model notes:** Pure helper `clips_after(seq, t, scope)` and `clips_before(seq, t, scope)`.

### TL-018 Move & drag clips — mvp

**What the user sees:** Selected clips can be dragged along the track and to other tracks. A
ghost shows the landing spot, snap lines appear (TL-028), and invalid positions show a red
ghost.

**Inputs / controls:**
- Drag a selected clip (4 px threshold). `Esc` cancels.
- `Alt` on release: overwrite on overlay tracks (v1, same rules as TL-012).
- Hold `Mod` while dragging to suspend snapping.

**Behavior / output:**
- **Horizontal on an overlay track (or main with magnet off):** non-ripple. The clip moves to
  the pointer time. If the span collides with another clip on the target track, the move goes
  to a new track directly above (same rule as drop), or is refused if the move is multi-clip
  (see below).
- **Within the main track (magnet on):** reorders. The clip is removed from its position (rest
  closes up) and inserted at the target cut point.
- **Between tracks:** only to compatible kinds. Dragging above the top track or into the
  new-track zone creates a track. Main → overlay leaves no hole when the magnet is on. Overlay →
  main inserts.
- **Multi-clip move:** relative time offsets and relative track offsets are preserved. If any
  member would collide, start before 0, hit an incompatible kind or a locked track, the whole
  move is refused (red ghost). Partial moves never happen.
- Attached items move with their main-track clip (TL-029). Moving an attached item itself
  re-attaches it based on its new position.
- Transitions on the cuts of a moved clip are removed when its neighbors stop touching
  (FX-002). The removal is FX-002's `RemoveTransition` folded into the same change set, so the
  surviving neighbor gets its `D` head frames back (TL-004) before the move is applied.
- The preview updates on release, not during the drag (performance). The skim line shows the
  ghost's start frame (FreeCut default).

**Edge cases:**
- Dropping exactly at the original place is a no-op with no history entry.
- Moving a clip past the sequence end extends the duration.
- A multi-clip selection containing main-track clips (magnet on): the main-track members must be
  contiguous. Otherwise the move is refused with a toast ("Move non-adjacent main-track clips
  one at a time").
- A drag that reaches the timeline edge auto-scrolls (TL-016 speeds).
- A move that would push the auto-created track count beyond the limit is refused.
- FreeCut never asks where moved items should go: the pointer's track and the collision rule
  above always decide, and the ghost shows the result before release (FreeCut default). The
  reference app, as observed, sometimes asks the user to pick a destination when it is
  ambiguous.
- Pieces of a split clip are ordinary clips, so they can be reordered by dragging like any
  other clip (on the magnetic main track this is a reorder, above).

**Undo:** `MoveClips { moves, ripple_changes, created/removed tracks }`, one step per drag.

**Model notes:** `plan_move(seq, selection, delta_time, delta_track, mode) -> Result<ChangeSet>`.
Pure and shared by ghost preview and commit. Property tests cover: apply + invert = identity,
and no overlaps after any successful plan.

### TL-019 Keyboard nudge & reorder — v1

**What the user sees:** Selected clips move one frame at a time with the keyboard, for precise
sync.

**Inputs / controls:** `,` / `.` moves 1 frame earlier/later. `Shift+,` / `Shift+.` moves 10
frames (FreeCut default). On the main track with magnet on, `Alt+,` / `Alt+.` swaps the clip
with its previous/next neighbor.

**Behavior / output:** The same validation as TL-018 (non-ripple; refused on collision or
before 0). Main-track clips with magnet on can't be nudged in time (there are no gaps), so they
use swap instead.

**Edge cases:** Nudge with nothing selected does nothing. With a mixed main/overlay selection
and magnet on, only overlay members would move, so the nudge is refused with a toast.

**Undo:** `MoveClips`. Repeated nudges of the same selection within 1 s coalesce into one
history entry (FreeCut default).

**Model notes:** Reuses `plan_move`. Coalescing is a History feature (TL-043).

### TL-020 Trim clip edges — mvp

**What the user sees:** Hovering a clip's left or right edge shows a trim cursor. Dragging
shortens or lengthens the clip, with a tooltip showing the change (`-00:00:01:12`) and the new
duration. On the main track (magnet on), the rest of the edit follows. On overlay tracks,
nothing else moves.

**Inputs / controls:** Drag an edge. Snapping applies, and `Mod` suspends it. The numeric
duration field in the inspector issues the same op (UI).

**Behavior / output:**
- **Out point (right edge):** changes duration. Media clips can't extend past the end of their
  source (adjusted for speed). Stills, generated items and looping animated media (TL-011's
  `SourceLimits` returns unbounded) have no limit.
- **In point (left edge):** moves the source in-point. On overlay tracks the clip's start moves
  with the edge. Media clips can't extend before source 0. Looping animated media are trimmed
  like stills on this edge too: the in-point moves into unwrapped animation time and OV-013's
  mapping keeps the animation continuous.
- **Overlay tracks:** non-ripple. Extending stops at the neighboring clip's edge, with no
  overlap.
- **Main track, magnet on:** always ripple.
  - Right edge: later clips shift by the change.
  - Left edge: the clip's timeline start stays put, its content shifts, and later clips shift
    by the change.
  - Attached items follow per TL-029.
- **Main track, magnet off:** behaves like an overlay track.
- Minimum duration: 1 frame. Edit points land on the frame grid (TL-001; sub-frame audio trims
  are TL-050, later).
- Properties anchored in source time (keyframes, clip markers) keep their source positions.
  Keyframes outside the new range are kept, and TR-027 defines how they still shape the value
  at the clip edge.
- Transitions on a trimmed edge: under TL-004's placement model, a transition at cut A|B needs
  A to keep at least `D` frames and B to keep `D` frames of source before its in-point. A trim
  that breaks this shortens or removes the transition in the same change set (FX-002 owns the
  clamp).

**Edge cases:**
- Trimming a linked or grouped clip trims only the dragged clip (FreeCut default). Detached
  audio is independent (TL-030). With several clips selected beyond a single group, one drag
  trims them all (TL-051, v1).
- Trimming offline media: limited by the last known source duration.
- Trimming a clip on a speed curve: limits come from TR's mapping (source end reached).
- A trim that ends with the same values: no history entry.
- A trim on the main track that would need to shift a clip on a locked track: refused if the
  main track itself is locked. Otherwise locked attached items stay and get flagged (TL-009).

**Undo:** `TrimClip { clip, edge, before, after, ripple_changes }`.

**Model notes:** `plan_trim(seq, clip, edge, new_time, mode) -> Result<ChangeSet>`. The
timeline crate needs a `SourceLimits` provider: asset durations come in as plain data from the
project, not via I/O.

### TL-021 Ripple trim & ripple across tracks — v1

**What the user sees:** On any track, holding a modifier while trimming makes later clips on
that track move with the edit. An optional *Ripple all tracks* mode makes ripple edits shift
the other tracks too.

**Inputs / controls:** Hold `Shift` while dragging an edge (FreeCut default). *Ripple all
tracks* toggle in the timeline toolbar's overflow menu (UI-011) and the Timeline menu (UI-012);
default off; per sequence. Key binding: see UI-031.

**Behavior / output:**
- Ripple trim on an overlay track: clips after the edited edge on that track shift by the
  change (left-edge trims keep the clip's start, matching the main-track rule).
- With *Ripple all tracks* on, every ripple edit (ripple trim, ripple delete, gap close, freeze
  insert, adding or removing a transition, magnetic main-track edits) also shifts clips and
  caption segments on other unlocked tracks that start at or after the edit point.
- A track is **exempt** from ripple-all when it has a clip spanning the edit point, or a clip
  inside the removed span. Shifting such a track would cut through or overlap content. Exempt
  tracks are listed in a toast, and their attached items are flagged.

**Edge cases:**
- Shrinking ripples can't create overlaps, because content after the edit point moves as one
  block.
- Locked tracks are always exempt.
- Timeline markers move only if *Markers follow ripple edits* is on (TL-037).

**Undo:** Ripple shifts are part of the triggering op's change set. Toggling *Ripple all
tracks* is an undoable `SetSequenceFlag` with no content changes (TL-003).

**Model notes:** A single `RipplePlan { at: Tick, delta: Tick, tracks: TrackScope }` helper
used by all rippling ops.

### TL-022 Slip & roll edits — later

**What the user sees:**
- **Slip:** drag a clip's content left or right while its position and length stay fixed (it
  shows a different part of the source).
- **Roll:** drag a cut point between two adjacent clips so one gets longer and the other
  shorter, with the total unchanged.

**Inputs / controls:** A **Slip tool** (`Y`, proposal; not yet in UI-031): dragging inside a clip slips it. A
plain `Alt` drag inside a clip is not used for slip, because `Alt` at release already means
overwrite for moves (TL-018). `Alt`+drag on a cut point between two touching clips = roll
(FreeCut default). Both snap to frames; the slip/roll amount is limited as below. A two-up
preview shows both edge frames while dragging.

**Behavior / output:**
- Slip changes only the source in-point. It is limited by the source range.
- Roll changes one clip's out-point and the neighbor's in-point by the same amount. It is
  limited by both clips' handles. Nothing else moves.

**Edge cases:**
- Stills, freeze frames and generated items can't be slipped (no source range).
- A clip that already uses its whole source has no handles, so slip does nothing and makes no
  history entry.
- Roll is limited to adjacent clips with no gap. It works the same on the main track with the
  magnet on, because the total is unchanged.
- A transition on the rolled cut point limits the roll: A must keep at least `D` frames and B
  at least `D` frames of source before its in-point (TL-004, FX-002).
- Offline media: limits come from the last known source duration (TL-011).
- Either clip on a locked track: slip/roll is refused.

**Undo:** `SlipClip`, `RollEdit`.

**Model notes:** Pure ops. They reuse `SourceLimits`.

### TL-023 Split at playhead — mvp

**What the user sees:** Splitting cuts the selected clip into two at the playhead. *Split all*
cuts every clip under the playhead on all unlocked tracks.

**Inputs / controls:** `Mod+B` = split the selection (or, with nothing selected, the main-track
clip under the playhead). `Mod+Shift+B` = split all. A toolbar scissors button does the same.

**Behavior / output:**
- Each affected clip becomes two clips with adjacent timeline and source ranges. Both keep all
  properties (transform, filters, effects, audio settings, speed).
- Keyframes follow TR-027, the owner of the keyframe model: keyframes are anchored in source
  time, **both parts keep all keyframes**, and **no boundary keyframe is inserted** at the
  split point, so the output is unchanged. This applies to every keyframed property, including
  effect parameters (FX-010) and volume (AU-008), which defer to TR-027.
- Fade-in stays on the left part and fade-out on the right part (AU). Incoming and outgoing
  transitions stay on their respective outer edges (FX).
- Clip markers (beat markers included) and attached items go to whichever part contains their
  anchor.
- Split all also splits caption segments under the playhead on unlocked caption tracks, using
  TX-027's text-split rule.
- Links: both parts stay members of the clip's link (TL-011).
- Group membership: both parts stay in the group.
- A compound clip splits into two compound clips with adjacent source ranges. The right-hand
  part gets a deep copy of the nested sequence, so every nested sequence keeps exactly one
  referencing clip and the halves edit independently (TL-040).
- After a split, the right-hand part becomes selected.

**Edge cases:**
- Playhead on a clip's first frame, or at its end: that clip is skipped, since no 0-length
  parts are created. If nothing could be split, nothing happens and no history entry is made.
- Playhead within a transition region: under TL-004's placement model that region is
  `[cut − D, cut)`, the last `D` frames of A's stored range. Splitting A there is refused
  (toast). Remove or shorten the transition first (FX-002). A playhead exactly on the cut is
  B's first frame, so nothing is split.
- Text split: both parts carry the same text content (TX).
- Locked tracks are skipped by split-all.
- Split on offline media works (metadata only).
- Nothing selected and no main-track clip under the playhead: nothing is split, and the status
  bar says "No clip at the playhead" (FreeCut text). With clips selected, Split works on them
  whatever the main track holds at that time (FreeCut default; the reference app, as observed,
  refuses a split when the main track has no content at the playhead).
- Mixed selection where some clips can't be split (playhead outside or on an edge, inside a
  transition region, or a clip kind whose owner refuses splitting, TL-054): the other clips are
  split, and one summary toast names how many weren't and why ("2 clips weren't split: playhead
  outside the clip"; TL-053). Still one history entry.
- Data bound to a clip and stored in source time, such as AI-008's motion-tracking samples,
  stays valid for both parts; how each owner divides its bound data is its `SplitAt` rule
  (FreeCut default: nothing is dropped). The reference app, as observed, keeps tracking data
  on the first piece only.

**Undo:** `SplitClips { originals, results }`. Undo restores the original single clips with
original ids.

**Model notes:** `plan_split(seq, targets, at) -> Result<ChangeSet>`. New clip ids come from the
sequence id counter. Property bags implement a `SplitAt` trait so each owning spec defines how
its data divides. Keyframe tracks inside those bags use TR-027's helper, which copies all keys
to both parts unchanged.

### TL-024 Blade tool — v1

**What the user sees:** A blade tool mode. Hovering a clip shows a cut line and timecode, and
clicking cuts it there. The pointer becomes a blade.

**Inputs / controls:** `B` = blade tool, `A` = select tool (as in UI-031). Click = cut that clip.
`Shift`+click = cut all unlocked tracks at that time. Snapping applies.

**Behavior / output:** Identical to TL-023 at the clicked (snapped, frame-quantized) time. The
tool stays active until switched back. While hovering a clip, the preview shows the frame under
the cut line, whatever TL-014's hover-skim mode is.

**Edge cases:** Clicks on empty space or locked tracks do nothing. A click on a clip edge does
nothing. A click during playback cuts at the clicked time and playback continues (FreeCut
default; the clicked position is fixed on the timeline even while the playhead moves). The
reference app, as observed, refuses click-to-split while playing.

**Undo:** `SplitClips` per click.

**Tool menu and the Hand tool (toolbar round, 2026-10-13):**
- The toolbar shows the active tool as one **tool button with a chevron** (UI-011): its icon is
  the active tool's (arrow, blade, open hand) and the chevron opens a menu of the three tools
  with their keys: **Select** (`A`), **Blade** (`B`), **Hand** (`H`, as in UI-031).
  Clicking the button itself switches between Select and the last non-select tool used
  (FreeCut default), so one click turns the blade on and off.
- **Hand** (FreeCut default): dragging anywhere in the lanes pans the view horizontally and
  vertically, like the middle-button drag of TL-041; clicks neither select nor seek, and the
  ruler still scrubs. The pointer is an open hand, closed while dragging.
- **Pointers:** Select uses the normal arrow (and TL-020's trim cursors at clip edges); Blade
  shows a crosshair with a small blade glyph beside it and, over a clip, TL-024's cut line with
  its timecode; Hand shows the hand. Over a locked track the blade and hand show the "not
  allowed" pointer while over a clip (FreeCut default; the hand still pans there, only clicks
  are inert).
- The active tool is view state, not saved: each project opens with Select (FreeCut default).
  `Esc` cancels a gesture in progress but does not change the tool.
- Keyboard commands keep working whatever the tool (split at playhead, trim to playhead,
  delete); only pointer behavior in the lanes changes.

**Model notes:** App-only tool state, `TimelineTool::{Select, Blade, Hand}` plus
`last_other: TimelineTool` in the timeline's view state. A blade click builds
`EditRequest::Split(SplitReq { at, targets: SplitTargets::Clips(vec![clip]) })`, or
`SplitTargets::AllTracks` with `Shift`, where `at` is the pointer time snapped by the gesture's
`SnapIndex` (TL-028) and quantized by the op. It reuses `plan_split`. Hit-testing, the snapped
cut time and "would this click split anything" are pure helpers in the app's timeline layout
module, unit-tested without a window.

Build notes (app, 2026-10-14): the helpers live in `ui::timeline_tools` (`ToolState`,
`blade_time`). The blade's line and timecode show while it hovers a clip; the preview does not
follow the blade yet (that comes with TL-014's skim, T5). A blade click outside a clip, on an
edge or on a locked track does nothing and does not move the playhead.

### TL-025 Delete & ripple delete — mvp

**What the user sees:**
- **Delete** removes selected clips. On the main track (magnet on), the rest closes up. On
  overlay tracks, a gap is left.
- **Ripple delete** removes clips and closes the gap on any track.

**Inputs / controls:** `Delete`/`Backspace` = delete. `Shift+Delete` = ripple delete. Toolbar
trash button and context menu.

**Behavior / output:**
- Delete: main track with magnet on ripples. Other tracks lift, leaving a gap.
- Ripple delete: each affected track closes the removed span. With *Ripple all tracks* (TL-021),
  other tracks shift too, subject to the exemption rule.
- Items attached to a deleted main-track clip are **deleted with it** while *Follow main track*
  (TL-029) is on, with a toast "Also removed N attached items". Undo brings them back. With it
  off they stay in place.
- Deleting one member of a group deletes the whole group unless it was Alt-selected. A group
  left with fewer than 2 members dissolves.
- Selected markers or gaps in the selection are deleted or closed as well (TL-027, TL-037).
  Selected caption segments are deleted by TX-027's rules.
- Transitions on the cuts of a deleted clip are removed first by FX-002's `RemoveTransition`
  in the same change set, so a surviving neighbor gets its `D` head frames back (TL-004).
- Links (TL-011): deleting the text clip or caption segment of a `Speech` link also deletes the
  linked speech clip (TX-043). Deleting a speech clip or a detached audio clip only dissolves
  or shrinks its link.
- Tracks emptied by the delete are auto-removed (TL-006), except caption tracks.

**Edge cases:**
- Multiple non-adjacent clips on the same track with ripple delete: gaps close in one pass,
  computed from the latest to the earliest so offsets stay correct.
- Deleting every main-track clip leaves an empty main track. Overlays attached to them were
  deleted with them; unattached overlays remain at their times.
- Deleting detached audio does not restore the video's embedded audio (TL-030).
- Delete with nothing selected does nothing.
- Deleting clips never asks for confirmation, because one undo brings everything back (FreeCut
  default; some delete paths in the reference app, as observed, ask first). Deleting a whole
  track still confirms (TL-006), and so does Restore audio when it would remove edited clips
  (TL-030).

**Undo:** `DeleteClips { removed (full snapshots), removed_transitions, removed_link_members,
ripple_changes, removed_tracks }`. Undo restores clips, transitions and links exactly.

**Model notes:** `plan_delete(seq, selection, ripple: bool)`.

### TL-026 Trim to playhead (cut away the head or tail at the playhead) — v1

**What the user sees:** One key removes the part of a clip before the playhead, and another
removes the part after it. This is the fastest way to tighten a talking-head edit.

**Inputs / controls:** `Q` = trim start to playhead. `W` = trim end to playhead. They act on
the selected clips, or with nothing selected, on the main-track clip under the playhead.

**Behavior / output:**
- Main track (magnet on), ripple:
  - Q removes the left part. The clip's start stays where it was, and it and all later clips
    shift left.
  - W removes the right part. Later clips shift left.
  - The playhead moves to the new cut point, which is the clip's start for Q (FreeCut default).
- Overlay tracks (non-ripple): Q moves the in-point to the playhead, so the clip now starts at
  the playhead. W moves the out-point to the playhead. Nothing else moves. With `Shift` they
  ripple that track (TL-021).

**Edge cases:** Playhead not strictly inside the clip (outside it, or exactly on its first
frame or its end): no-op for that clip. With a single target, the command is disabled and its
tooltip says "Move the playhead inside the clip" (FreeCut text); with several targets, the
ineligible ones are skipped and counted in the summary (TL-053). A result shorter than 1 frame
is impossible, because the playhead is strictly inside.

**Toolbar buttons (toolbar round, 2026-10-13):** two buttons sit right after **Split** in
UI-011's edit group. Their icons show a clip cut at a line with one side greyed out, and their
tooltips say what goes and what stays (FreeCut text): **"Cut away the part before the playhead
(Q)"**, which keeps the right-hand part, and **"Cut away the part after the playhead (W)"**,
which keeps the left-hand part. They enable from the same availability check as the keys; when
disabled, the tooltip gives the reason: "Select a clip or put the playhead over the main
track" (nothing to act on) or "Move the playhead inside the clip". The reference editor offers
the same pair, named by the side that goes; FreeCut names them by what is cut away relative
to the playhead, because a side name reads ambiguously next to Split.

**More behavior (FreeCut defaults):**
- During playback the buttons and keys act at the frame being shown, like Split (UI-011), and
  playback continues.
- With the magnet on and several selected main-track clips, each is cut in turn from the latest
  to the earliest, so the ripple of one never moves the playhead out of the next (the playhead
  is fixed while the command plans; every target is judged against it).
- Linked partners (TL-057 on): a clip's linked audio is cut at the same time when it covers the
  playhead, so sync is kept.
- Q on a main-track clip with the magnet on moves the playhead to the clip's start (above); W
  never moves the playhead. On other tracks the playhead stays for both.
- Items attached to the clip (TL-029) follow the ripple like any other main-track trim.

**Undo:** `TrimClip` (one step even for multiple clips).

**Model notes:** Reuses `plan_trim` with edge = playhead. Contract for the build:
`EditRequest::TrimToPlayhead { seq, targets: TrimTargets, at: Tick, cut: Cut, ripple_others:
bool }` with `TrimTargets::{Clips(Vec<ClipId>), MainUnderPlayhead}` and `Cut::{Head, Tail}`
(`Head` = Q, `Tail` = W); `ripple_others` is the `Shift` variant for non-main tracks. It
resolves to one `TrimReq` per eligible clip inside one transaction, labelled
`OpKind::TrimToPlayhead` (history label "Cut away before playhead" / "… after playhead",
TL-043), skips ineligible clips with TL-053's reasons and is refused with
`EditError::NothingToTrim` when none is left. A head cut on the magnetic main track adds
`Notice::PlayheadTo(clip_start)` so the app moves the playhead without guessing.
`fn trim_to_playhead_available(doc, seq, selection, at) -> Result<(), Unavailable>` is the
shared availability check for the toolbar, menus and keys.

Build notes (model, 2026-10-13):
- The label carries the side: `OpKind::TrimToPlayhead(Cut)`, so the history can say "before"
  or "after".
- `Unavailable` starts with `NoTarget`, `LockedTrack` and `PlayheadOutside`; TL-054's other
  reasons join it as their ops land. Skipped targets come back as
  `Notice::Skipped(Vec<(ClipId, Unavailable)>)`. The check is
  `trim_to_playhead_available(doc, seq, &TrimTargets, at) -> Result<(), Unavailable>`: `Ok`
  when at least one target can be cut, else the first target's reason.
- The model cuts the targets it is given; the app adds linked partners with TL-057's
  `expand_selection`. A partner linked to a main-track clip that ripples loses the same
  content in place (it keeps its start), so the pair stays in sync. With `ripple_others`, a
  head cut off the main track is also made in place and the track closes up.
- There is no transition model yet, so FX's rule for a clip under a transition has no test
  yet.

### TL-027 Gaps: select, close, close all — v1

**What the user sees:** Empty space between clips on an overlay track (or on the main track
with magnet off) highlights on hover. Clicking it selects the gap, and Delete closes it.
A track menu offers **Close all gaps**.

**Inputs / controls:**
- Click a gap to select it. `Delete` closes it.
- Clip context menu: *Close gap before this clip* closes the gap directly before that clip on
  its track (disabled when the clip already touches its neighbor or starts at 0).
- Track header context menu: *Close all gaps on this track* (acts on every selected track when
  several are selected, TL-015).
- Sequence menu: *Close all gaps* (all unlocked tracks, main track first).

**Behavior / output:**
- Closing a gap shifts every later clip on that track left by the gap length. With *Ripple all
  tracks* on, other tracks shift too (TL-021 exemptions apply).
- The space before the first clip on a track (from 0) counts as a gap. Space after the last
  clip doesn't.

**Edge cases:**
- Main track with magnet on never has gaps, so nothing is selectable there.
- A gap shorter than 1 frame can't exist, because positions are frame-quantized.
- Closing a gap that moves items attached to main-track clips: impossible, since attached items
  only move with their main clip. Closing gaps on an overlay shifts its clips, and they are
  re-attached according to their new positions.

**Undo:** `CloseGaps { track, gaps, shifts }`.

**Model notes:** `gaps(track) -> Vec<TimeRange>` and `plan_close_gaps`.

### TL-028 Snapping — mvp

**What the user sees:** While dragging clips, edges, the playhead or markers, they jump to
nearby meaningful points. A vertical guide line flashes where they snap.

**Inputs / controls:** Toolbar toggle *Snapping* (its own icon, distinct from the main-track
magnet of TL-005) plus key `N` (as in UI-031). Default on (preference, per user). Hold `Mod`
during a drag to suspend snapping. Toggling shows a short confirmation toast with the new
state (UI-040), like the magnet.

**Behavior / output:**
- Snap targets:
  - playhead
  - clip starts and ends on all tracks (excluding the dragged clips), and caption segment
    edges
  - timeline markers and clip markers
  - in/out range marks
  - 0 and the sequence end
- Snap distance: 8 screen pixels (FreeCut default), converted to time at the current zoom.
- For moves, both edges of the dragged block are tested and the closest match wins. Ties go to
  the playhead, then markers, then clip edges.
- Snapping applies to: move, trim, playhead drag, blade position, marker drag, range-mark drag
  and drop placement.
- Frame quantization (TL-001) always applies, even with snapping off.

**Edge cases:**
- Two targets within the threshold: the nearest wins. A snapped edge shows one guide even if
  several targets coincide.
- At extreme zoom-out, many targets fall within 8 px. The nearest still wins, so there is no
  jitter.
- Hidden tracks' clips remain snap targets. Locked tracks' clips also remain targets (FreeCut
  default), because they are fixed references.

**Undo:** n/a (behavior preference; not a document change).

**Model notes:** `snap_targets(seq, exclude) -> Vec<(Tick, SnapKind)>` (timeline crate, sorted,
with binary search). The pixel threshold is converted to ticks by the app.

### TL-029 Attached items (following main-track clips) — v1

**What the user sees:** Text, stickers, overlays, effects and audio placed over a main-track clip
belong to it. When that clip is moved, trimmed or rippled, they travel with it and stay in
sync. A small tick at the start of each attached item points down to its owner. A *Follow main
track* toggle turns this off so items stay fixed in time.

**Inputs / controls:**
- Toolbar toggle *Follow main track*, beside the magnetic main track (UI-011), also in the
  Timeline menu (UI-012). Key binding: see UI-031. Default on, per sequence; its state is an
  undoable sequence flag (TL-003).
- **Which kinds follow** (FreeCut default list): a small popover from the toggle's dropdown arrow
  and from the Timeline menu, *Follow main track ▸ Item kinds…*, with one checkbox per kind
  plus *All* and *None*. Default: all checked. Kinds:

  | Kind | Items it covers |
  |---|---|
  | Video overlays | clips and images on non-main `Video` tracks, compound clips there |
  | Text | items on `Text` tracks |
  | Stickers & shapes | items on `Sticker` tracks |
  | Effects | items on `Effect` tracks |
  | Filters & adjustments | items on `Adjustment` tracks |
  | Music | audio clips whose AU-001 origin is music |
  | Sound effects | audio clips whose origin is sound effect |
  | Voice & speech | voiceover recordings (AU-027) and generated speech (TX-043, AI-025) |
  | Other audio | detached (TL-030), extracted and other-origin audio clips |

- The toggle's tooltip summarizes the set: "All kinds", "None" or a short list ("Text,
  Stickers & shapes"). The default set for new projects is a preference (UI-020).
- **Toolbar presentation (toolbar round, 2026-10-13):** the reference editor has an
  equivalent on/off control with a per-kind settings dialog; FreeCut keeps its own name,
  *Follow main track*, and draws it as a split button (UI-011): the left part toggles the flag (`Alt+N`), the
  chevron opens a menu with *Item kinds…* (the popover above), a separator, and the checkable
  *Select linked clips together* (TL-057), because both decide what travels with a clip. The
  toggle's icon is two chain links over a bar; the button shows *checked* while the flag is on
  and at least one kind is checked, and a small dot when only some kinds are checked.
- **First use (FreeCut default):** the first time a project with this flag on has an overlay
  item follow a main-track move, a one-time hint (UI-058) points at the toggle: "Items above
  follow the main-track clip under their start. Change which kinds in this menu." (FreeCut
  text), with "Don't show again".

**Behavior / output:**
- An item is attached to the main-track clip that lies under the item's **start**. The anchor
  is stored as that clip's id plus a **source-time offset**, meaning the specific frame of the
  owner's content the item starts on (FreeCut default; content-anchored). So trimming the
  owner's head moves the item along with the content it was placed over.
- Attachment is recomputed whenever the item itself is placed, moved or trimmed by the user.
  Items starting where no main-track clip exists (past its end, or in a main-track gap with the
  magnet off) are unattached.
- When owners move (reorder, ripple, compaction, speed change), attached items move by the same
  amount, unless their track is locked, in which case they're flagged out-of-sync (TL-009). If
  the shifted item would collide on its track, it moves to a new track directly above (a
  collision would otherwise lose content).
- Owner deleted: the attached items are deleted with it (TL-025).
- Owner split: items re-anchor to the part containing their anchor frame.
- Owner trimmed so the anchor frame is removed: the item re-anchors to the nearest remaining
  frame of the owner, so it stays adjacent to it.
- Turning *Follow main track* **on** attaches items based on current positions without moving
  anything. Turning it **off** clears all attachments.
- Only items of a checked kind attach. Items of an unchecked kind behave as if the toggle were
  off: they stay fixed in time when owners move and are not deleted with their owner.
  Checking a kind attaches its items by current position; unchecking it clears their
  attachments. Neither moves anything.

**Edge cases:**
- Items on the main track can't be attached (they are owners).
- Toggle on with every kind unchecked: nothing attaches, and the tooltip says "None", which is
  equivalent to the toggle being off (FreeCut default; the two settings stay independent, so
  checking a kind again restores the user's set).
- An audio clip whose origin changes (AU-001) re-evaluates its kind on its next placement; an
  existing attachment is kept until then.
- Caption segments never attach: they move only through ripple rules (TL-021), because a
  caption track is timed to speech across clip boundaries.
- Items inside compound clips attach within their own nested sequence.
- An attached item longer than its owner is fine. Only the start is anchored.
- Two items stacked on the same track attached to different owners can swap order after a
  reorder. Collisions are resolved by the new-track rule above.
- Detached audio follows the same rules (it attaches to the main clip under its start).

**Undo:** Toggling *Follow main track* is an undoable `SetSequenceFlag` (TL-003) whose change
set holds every attachment it created or cleared; undo restores the previous attachments and
the flag exactly. No clip moves either way. Changing the kind set is the same kind of entry
(`SetSequenceFlag` on `follow_kinds`, carrying the attachments created or cleared). Attachment
shifts are part of each triggering op's change set.

**Model notes:** `Attachment { owner: ClipId, owner_source_offset: Tick }`.
`FollowKinds` is a bitset over `FollowKind::{VideoOverlay, Text, Sticker, Effect, Adjustment,
Music, SoundEffect, Speech, OtherAudio}` stored in `SequenceFlags` (TL-003); `fn
follow_kind(&Clip, &Track) -> FollowKind` is pure and reads AU-001's `origin` as plain data.
`fn follow_owners(seq, owner_changes) -> ChangeSet` is applied inside every op that moves or
retimes main-track clips, and skips items whose kind is unchecked. Speed changes call it via
TR's mapping.

Build contract (toolbar round, slice T4): `Clip.attach: Option<Attachment>` (the
architecture's `Attachment { owner, owner_source_time }`, timeline-model.md §9, is this
entry's `owner_source_offset`; the reverse `attached` index is derived, never stored) and
`SequenceFlags { follow_main: bool, follow_kinds: FollowKinds }` (defaults on and all kinds;
the format's records default them the same way for older files). Requests:
`SetFollowMain { seq, on }` and `SetFollowKinds { seq, kinds }`, each one entry carrying the
attachments it created or cleared. `fn attach_all(seq) -> Vec<(ClipId, Option<Attachment>)>`
recomputes attachments from positions (pure); the `Tx` helper `reattach(clips)` runs it for
clips the user placed, moved or trimmed; `follow_owners` runs inside move, trim (ripple),
delete, split, `SetMagnet` compaction and `TrimToPlayhead`. Tests: the property test of
TL-018 gains "after any op, every attached item starts at its owner's anchor frame or is
flagged out of sync", plus one apply → undo → redo test per op that moves owners.

Build notes (model, 2026-10-13):
- `follow_owners` runs once at the end of every transaction (`Tx::finish`), so every op that
  moves, trims, splits or deletes main-track clips is covered, including insert, delete and
  remove-media. It has two passes:
  - Items the op didn't move themselves follow their owner's anchor frame. The anchor goes to
    the piece that shows it after a split, and to the nearest remaining frame after a trim.
  - Items the op placed, moved or trimmed are attached again by position.
  Conform attaches every item on an unlocked track again where it lands, instead of letting it
  follow sub-frame shifts.
- When a video is deleted, its own detached sound is not deleted with it, because TL-030 says
  the sound remains. Only its attachment is cleared.
- `follow_kind(&Clip, &Track, &Library)` reads the media item's PM-041 origin. Until AU-001's
  music and sound-effect origins exist, audio is either *Voice & speech* (a voiceover or TTS
  origin) or *Other audio*.
- Out of sync stays derived: `expected_start(seq, attachment)` against the item's start. An
  item that was already out of sync keeps its offset when its owner moves.
- An item moved in the same move as its owner is attached again where it lands.

### TL-030 Embedded audio: detach & restore — v1

**What the user sees:** A video clip with sound shows a waveform strip along its bottom. **Detach
audio** pulls the sound out to its own audio clip on an audio track, so it can be trimmed,
moved or deleted separately. **Restore audio** puts it back.

**Inputs / controls:** Context menu *Detach audio* / *Restore audio*, and one key command that
toggles depending on state (binding: see UI-031).

**Behavior / output:**
- **Detach:**
  - An audio clip is created on the first audio track free over the same span (or a new audio
    track at the top of the audio band), with the same timeline range and source range.
  - The video clip's embedded audio becomes `Detached` (silent, no waveform band).
  - Audio settings (volume, fades, audio effects; AU) move to the new clip.
  - Both clips become members of one `DetachedAudio` link (TL-011).
  - The new clip inherits the video clip's name override, if any (TL-048).
  - Sources with several audio streams produce one audio clip per enabled stream, on separate
    tracks.
- **Out-of-sync badge:** if a linked audio clip's source offset relative to its video clip
  changes (one was moved or trimmed without the other), both show the offset in frames (for
  example "+12f"). Context action *Resync* moves the audio back.
- **Restore:** re-enables the embedded audio and deletes every audio clip still carrying that
  `LinkId`. A confirmation lists them if there is more than one or if any was edited after
  detaching. Audio settings from the first linked clip move back to the video clip.

**Edge cases:**
- A video without an audio stream, or an image: Detach is disabled.
- Linked audio split into several pieces: Restore removes all pieces (after confirmation).
- Linked audio deleted entirely: Restore simply re-enables the embedded audio.
- A video clip deleted while its audio is detached: the audio clip remains. Its `LinkId`
  becomes dangling and is cleared, and its badge disappears. FreeCut default: this holds
  however many pieces the audio was split into; the link is cleared from all of them.
- Replace clip media on a video with detached audio: the link is cleared and the user is warned
  (TL-035).
- Selecting a linked clip selects its partners too while *Select linked clips together*
  (TL-057) is on, which is the default; with it off, selecting a linked clip doesn't
  auto-select its partner, and moving them together requires selecting both or grouping them.
  `Alt`+click always selects one member alone.

**Undo:** `DetachAudio { video, created_audio_clips, moved_props }` and `RestoreAudio`.

**Model notes:** `EmbeddedAudio::{Active(props), Detached(LinkId), None}` lives on video clips.
The mixer (AU) reads only `Active` embedded audio plus audio-track clips.

**FreeCut defaults (model):**
- After *Detach*, the new audio clips are selected (one per detached video clip). The new clip
  also copies the video clip's enabled state, so nothing changes audibly. Locked audio tracks
  are passed over when looking for a free one, and a new audio track is announced like any
  other created track. Clips without active sound in a mixed selection are skipped; the
  command is refused only when no selected clip has any.
- A detached video keeps knowing its sound was detached after its link dissolves (for example
  when the audio clip is deleted), so *Restore* still works and gives back default settings.
- *Restore* is a command on the video clip. A selected linked audio clip is not a target of
  its own, so a selection with no detached video is refused.
- *Restore* takes the settings of the first linked clip (earliest start, then track order),
  except the fade-out, which comes from the linked clip that ends last. A detached clip that
  was split therefore gives back both of its fades. Each video piece also takes back the
  stream selection (AU-038) of the first linked clip that reads its own file, so it plays the
  stream its detached sound played; with no such clip left it keeps its own selection.
- *Detach* on a video that already has a link (streams added as clips, AU-038) detaches under
  that link instead of a new one, so those clips stay linked to the video: their out-of-sync
  badge, *Resync*, *Restore* (which removes them) and the "already playing" check still see
  them. A video without a link gets a new one.
- A video split while detached keeps every piece in the link. *Restore* on any piece gives
  every piece its sound back, with no fade on the inner edges (as a split of a clip with sound
  would have). Afterward the restored video clips are selected.
- *Out of sync* (v1): `freecut_timeline::edit::sync_offset(doc, seq, clip)` gives
  `SyncOffset { frames, ticks, partner }` for a linked media clip, or `None` when it has no
  partner in the link or is exactly in sync. The offset is the audio clip's (start − source
  start) minus the video clip's, so positive means the sound is late; `frames` rounds it to the
  nearest sequence frame with exact halves toward zero, and is 0 for a slip of less than half
  a frame. An audio clip is measured against the video piece of its link closest to sync (a
  video split while detached); a video reports its largest offset among the audio clips of
  its link (detached sound and streams added as clips, AU-038), ties to the earlier one.
  `sync_offsets(doc, seq)` computes every badge in one pass. Trims and splits keep sync; only
  moving one side breaks it.
- *Resync* (v1): `Resync { seq, clips }` moves every listed out-of-sync audio clip by minus its
  offset, and for a listed video every audio clip of its link that is out of sync with it. A
  clip stays on its track when the target span is free there, else it goes to the first free
  unlocked audio track (or a new one at the top of the audio band, TL-006). It is one undo
  entry, and is refused when nothing listed is out of sync, when a clip would start before 0,
  or when a moving clip's track is locked.

**FreeCut defaults (app):**
- The clip context menu shows one item, *Detach audio* when any selected clip can detach,
  else *Restore audio* when any is detached; otherwise *Detach audio* stays visible, disabled,
  with the reason ("This clip has no sound"). The inspector offers the same command, and a
  detached video's Audio section says "Audio is detached" with a *Restore audio* button.
- After *Detach*, the status line names the track the sound went to ("Sound detached to
  Audio 1", with "(new track)" when one was created). Unnamed tracks are numbered within their
  band: "Video 1" is the first track above Main, "Audio 1" the top audio track.
- *Restore* asks first when a video has more than one linked clip, or when a linked clip no
  longer covers the same timeline and source range as its video (it was moved, trimmed or
  split). The question lists the clips by name and start. Sound settings edited on the
  detached clip move back with the restore, so they alone don't trigger the question.
- A detached video shows a small link badge and no sound strip.
- An out-of-sync clip (and its partner) shows a red badge at its top-right corner with
  `sync_offsets`' offset: "+12f" / "-3f" for whole frames, one decimal for a slip that isn't
  ("+0.4f"), and "+<0.1f" under a tenth of a frame. The badge follows a move live while it is
  dragged. The clip menu offers *Resync* while any selected clip is out of sync (`Resync` on the
  selection, one undo entry).

### TL-031 Freeze frame — v1

**What the user sees:** With a video clip selected and the playhead over it, **Freeze** inserts
a still of the current frame (3 s by default) right at the playhead. The clip is split around
it, and the rest of the edit moves later.

**Inputs / controls:**
- Toolbar button (enabled only when a video clip is selected and the playhead is inside it) and
  `Shift+F` (as in UI-031).
- Default freeze duration: 3 s. Preference range: 1 frame to 60 s.
- The freeze clip's edges can be trimmed afterwards like any still.

**Behavior / output:**
1. The clip is split at the playhead (TL-023).
2. A `Still` clip showing the source frame at the playhead is inserted between the two halves.
   It inherits transform, filters, color, effects and adjustments, but not speed or audio, so
   it is silent.
3. Later content ripples right by the freeze duration: the main track with magnet on; overlay
   tracks within that track only; plus *Ripple all tracks* if enabled. Attached items follow
   their owners.

**Edge cases:**
- Playhead on the clip's first frame: no split, and the still is inserted before the clip.
  Playhead on the clip's last frame: inserted after the clip.
- Clip with variable speed or reverse: the frozen frame is the frame actually displayed at the
  playhead, from TR's mapping.
- Offline media: the freeze clip is created, but it renders the offline slate until relinked.
- Images, text and stickers: Freeze is disabled. Compound clips: v1 freezes the composited
  frame of the nested sequence at that time (the still source is a nested-sequence reference).
- Effects with time-based animation on the still: their animation still runs over the still's
  duration (FX).
- Clip length: never a reason to refuse. Any clip with the playhead inside it can be frozen,
  even a 1-frame clip (the still goes before or after it, above). The reference app, as
  observed, refuses freezing clips below a minimum length.
- The still is independent of its source clip: later edits to the original clip don't reach
  it, because TR-044 bakes the carried-over settings at creation (FreeCut default). The
  reference app, as observed, keeps freeze segments linked to their source clip and copies
  later changes to them.
- Freezing writes no file, so there is no progress bar; the still shows TL-042's shimmer for
  the moment its frame takes to decode.
- **Change frozen frame… (later):** a context-menu command opens a frame picker over the
  original media and moves the still's `StillSource` time to the chosen frame, keeping its
  length and settings. Until then, TR-044's rule applies: to freeze another frame, undo and
  freeze again. Offered only for `MediaFrame` stills whose media is online.

**Undo:** `FreezeFrame { split, inserted_still, ripple_changes }` is one step. *Change frozen
frame* (later) is `SetStillSource { clip, before, after }`.

**Model notes:** `StillSource::{MediaFrame{asset, source_time}, SequenceFrame{seq, time}}`. No
image file is written. The renderer decodes or renders the frame on demand and caches it
(`freecut-media`/`freecut-render`).

### TL-032 Copy, cut & paste — v1

**What the user sees:** Standard copy, cut and paste for clips, keeping their relative timing
and stacking. Pasted clips land at the playhead.

**Inputs / controls:** `Mod+C`, `Mod+X`, `Mod+V`. Edit menu and context menu.

**Behavior / output:**
- Copy stores the selected clips with all properties, relative time offsets, and their tracks'
  kinds and relative stacking. Clip markers come along. Attachments and links are not copied.
  Copy and Cut also fill the attribute clipboard used by *Paste attributes…* (TL-034).
- Cut = copy + Delete (same ripple rules as TL-025), as one undo step.
- Paste puts the earliest copied clip at the playhead.
  - Main-track clips in the clipboard (magnet on) are inserted at the cut point nearest the
    playhead, in their copied order, with ripple.
  - Other clips go to their original tracks if those still exist and the spans are free.
    Otherwise new tracks are created directly above, one per clipboard track.
  - Pasted clips become the selection.
- Paste works across projects in the same app session. Media references are resolved by asset
  path and added to the target project's library if missing (PM).

**Edge cases:**
- Paste with an empty clipboard does nothing.
- Pasting into an open compound clip pastes into the nested sequence. Pasting a compound clip
  into itself (directly or indirectly) is refused (cycle).
- The copied source asset was deleted from the library: pasted clips reference it as offline
  (TL-011).
- Paste would exceed the track limit: refused.
- The system clipboard is not used in v1 (internal only). Interop is later.
- Copy confirms with a short toast ("Copied 3 clips"; UI-040). Paste never asks where items
  should go: the rules above always decide (FreeCut default; the reference app, as observed,
  asks when several tracks or nothing is selected).

**Undo:** Copy: n/a. Cut: `DeleteClips`. Paste: `InsertClips`.

**Model notes:** `Clipboard { clips: Vec<ClipSnapshot>, track_layout, origin_time }` lives in
`freecut-app`. The serialization for cross-project paste reuses `freecut-project` clip
serialization.

### TL-033 Duplicate — v1

**What the user sees:** **Duplicate** places a copy of the selection right after it.

**Inputs / controls:** `Mod+D` and the context menu.

**Behavior / output:**
- The copy starts at the selection's end (latest end among the selected clips), on the same
  tracks.
- Main-track members (magnet on) are inserted after the last selected main-track clip, with
  ripple.
- If an overlay span is occupied, that clip goes to a new track above.
- The duplicate becomes the selection, so repeated `Mod+D` creates a run.
- Compound clips are deep-copied: each copy gets its own nested sequence (FreeCut default; no
  shared instances).

**Edge cases:**
- A duplicate of a grouped selection forms a new group; the original group is unchanged.
- Duplicating at the sequence end extends the duration.
- New tracks needed beyond the track limit: the whole duplicate is refused (no partial copy).
- Attachments and audio links are not duplicated; attached copies re-attach by position
  (TL-029).
- Nothing selected: does nothing and makes no history entry.

**Undo:** `InsertClips`.

**Model notes:** Implemented as clipboard-free copy + paste-at-time.

### TL-034 Paste attributes — v1

**What the user sees:** After copying a clip, **Paste attributes…** on one or more selected
clips opens a checklist dialog. It names the copied clip and lists the property groups it
carries (for example transform, crop, color, effects, animations, audio settings, speed, text
or element style). The checked groups are applied to every selected clip in one step, and a
short summary lists anything that was skipped. Copying a look, an effect stack or a motion from
one clip to others is everyday work, so this ships in v1: FX-017 and CL-025 (both v1) build on
it.

**Inputs / controls:**
- Edit menu, clip context menu, and the key binding from UI-031 (`Ctrl+Alt+V` / `⌥⌘V`).
- One checkbox per category registered through `PropBag::category()`. Each owning spec defines
  its categories and their default check state (TR-034, CL-025, FX-017, OV-044; AU and TX
  register theirs the same way). This spec defines none of its own.
- Keyframe timing: *Rescale to target length* (default) or *Keep offsets from clip start*.
  The reference app, as observed, offers the same two choices.
- There is no separate *Keyframes* checkbox: keyframes travel with the category of the
  property they animate (FreeCut default). The reference app, as observed, lists keyframes as a
  group of their own beside color, animation, text style, text effects, audio and speed; in
  FreeCut those groups are the owners' registered categories (CL-025, FX-017, TX, AU, TR-034).
- *All* / *None* buttons. The check state is remembered per category for the session (FreeCut
  default).
- Category-specific options appear inline when the owning spec defines them (for example
  FX-017's Replace / Append mode for effects).

**Behavior / output:**
- **Attribute clipboard:** every Copy or Cut (TL-032) also takes an attribute snapshot of one
  clip: the copied clip if exactly one was copied, otherwise the earliest one (lowest track on
  ties). The snapshot holds that clip's property bags by value, its kind, its timeline duration
  and its time map. It survives deletion of the source clip and lasts until the next Copy or
  Cut, or until the app quits. It is not saved in the project.
- Each checked category replaces the target's value for that category, unless its owning spec
  defines another mode.
- Keyframes are placed using the timing option, measured in clip-local timeline time and
  converted to source time through the target's time map (TR-034). They stay source-anchored
  per TR-027.
- Several targets are processed in timeline order. All changes form one history entry.

**Edge cases:**
- Empty attribute clipboard: the command is disabled with the hint "Copy a clip first".
- A category that doesn't apply to a target's kind (for example audio settings onto an image,
  or speed onto a still) is skipped for that target and listed in the summary.
- A speed paste changes the target's duration. On the main track with the magnet on it
  ripples. On other tracks, a target that would grow into its neighbor is skipped and listed,
  and the other targets still apply.
- Locked-track targets can't be selected, so they are never targets (TL-009).
- The source clip itself among the targets: allowed. Unchanged categories make no change, and
  a paste that changes nothing makes no history entry.
- Snapshot taken in another open project of the same session: allowed. Values that reference
  files (a LUT, a mask image) resolve the same way as TL-032's cross-project paste.
- Compound clip target: only the compound clip's own bags change, never the clips inside it.
  A compound clip can also be the source (its own bags). The reference app, as observed,
  refuses compound clips as source or target.
- Several clips were copied: the snapshot comes from the earliest one, and the dialog title
  names it, so the user sees which clip is used (FreeCut default). The reference app, as
  observed, disables attribute paste after a multi-clip copy.
- Kind matching is each category's `applies_to`: owners may restrict a category to the same
  kind (text styles only onto text, TX; an effect item's parameters only onto effect items of
  the same effect, FX-017). Text with several formats in one clip is handled by TX's
  categories. A category that refuses a target is skipped and listed, never forced.
- *Keep offsets* onto a shorter target: keyframes landing past the end are kept but inactive
  until the clip is extended (TR-033's rule), and the summary says how many (FreeCut default;
  the reference app, as observed, drops them with a warning).
- Fades and in/out animations longer than a shorter target: the copied values are stored as
  they are, and their owners clamp the effective length at render time (AU-006's clamp, FX's
  slot policy), so lengthening the clip later restores them. The reference app, as observed,
  shrinks them while pasting.
- The dialog previews what will be skipped before *Apply* ("Speed: 2 images will be skipped").
  There is no separate "paste anyway" step: Apply applies what applies, and the summary
  afterwards lists the rest (TL-053).
- Transitions are not attributes: copying a transition onto cuts is FX-017's separate command.

**Undo:** One compound `PasteAttributes { targets, changes }` built from each category's own
set-op (TR-034's list, CL-025's, FX-017's, OV-044's) plus any ripple. Undo restores every
target and every shifted clip.

**Model notes:** `freecut-timeline` owns the registry and the pure op builder:
`AttrCategory { id: &'static str /* namespaced, e.g. "tr.transform" */, label: LabelId,
applies_to: fn(&ClipContent) -> bool, default_checked: bool, extract, apply }`, collected
through `PropBag::category()`. `AttrClipboard { source: ClipId, kind, duration, time_map,
values: BTreeMap<AttrCategoryId, AttrValue> }` is a timeline type held by `freecut-app`. No
other spec defines a separate attributes struct: there is no `ClipAttributes` type, and every
spec registers through this registry. Tests: per category, paste then undo restores the target
exactly; skipped categories leave the target untouched.

### TL-035 Replace clip media — v1

**What the user sees:** **Replace…** on a clip swaps in different media while keeping the
clip's place, length and every edit applied to it (position, filters, effects, animation,
volume).

**Inputs / controls:**
- Context menu *Replace…* opens the library picker (or a file dialog, which imports first via
  PM). The picker lists only compatible kinds: video and images for a video or image clip,
  audio for an audio clip.
- Exactly one clip at a time: with several clips selected, *Replace…* is disabled with the
  tooltip "Select one clip" (FreeCut text).
- Dragging a library item onto a clip while holding `Mod+Alt` replaces it (FreeCut default).
- A range picker lets the user choose which part of the new media fills the clip. It defaults
  to the start.

**Behavior / output:**
- The clip's content switches to the new `MediaRef`, and the source in-point is set from the
  picker (default 0).
- Timeline range and all property bags are unchanged. Clip markers are cleared, because they
  referenced the old source.
- Compatibility:
  - video ↔ video
  - video ↔ image (images fill the duration)
  - audio ↔ audio
  - Video can't replace an audio clip, and audio can't replace a video clip.

**Edge cases:**
- New media is shorter than the clip at the clip's speed: the clip is shortened to the new
  media's length. On the main track with magnet on, this ripples. A toast explains. (The
  reference app, as observed, refuses media shorter than the clip; FreeCut shortens, because
  the user explicitly chose that media and undo restores the old state.)
- Different aspect ratio: transform is kept as-is, and the fit mode is re-evaluated by TR.
- New media has no audio but the old one had embedded audio settings (for example a video
  replaced by a still image): the settings are kept but inactive, and a toast warns that the
  clip is now silent.
- The clip had detached audio: the link is cleared, with a warning.
- The clip uses settings backed by analysis of its old media, such as stabilization (TR-026),
  automatic cutout (AI-014), motion tracking (AI-008) or voice isolation (AU-021): the picker's
  confirm step warns that these will be analyzed again for the new media. After the replace,
  their jobs are queued on AI-001's scheduler, and each owner shows its pending state until the
  result arrives. The replace itself does not wait.
- The new media fails to probe or decode: nothing is replaced, the clip is unchanged and an
  error toast names the file. A replace never ends half-done.
- Replacing on a locked track is impossible (the clip isn't selectable).

**Undo:** `ReplaceSource { clip, before: content, after: content, duration_change }`.

**Model notes:** `plan_replace(seq, clip, new_media, in_point, limits)`.

### TL-036 Enable / disable clip — v1

**What the user sees:** A disabled clip stays on the timeline, drawn hatched and faded with an
"Off" badge (FreeCut text), but it isn't shown or heard in preview or export. It is a quick way
to try an edit without a clip.

**Inputs / controls:** The context menu and Clip menu *Enable/Disable clip*; key binding: see
UI-031. Applies to the selection.

**Behavior / output:** The renderer and mixer skip disabled clips. On the main track, a disabled
clip's span shows the canvas background. Effect and adjustment items that are disabled don't
apply.

**Edge cases:** Disabled clips still occupy time, snap and count toward duration. Disabling one
group member doesn't disable the others. Disabled clips are still edited by split, trim and
move. A mixed selection (some on, some off) toggles every clip to the same new state: if any
is enabled, all become disabled (the same rule as AU-007's mute).

The reference app, as observed, also has a per-clip *hide* that removes only the picture. FreeCut
covers the same needs with Disable (picture and sound off), clip mute (AU-007, sound off) and
track hide (TL-008, every clip's picture on a track); a separate picture-only switch per clip is
not offered (FreeCut default), so there is one fewer flag for every renderer and op to honor.

**Undo:** `SetClipEnabled { clips, before, after }`.

**Model notes:** `Clip.enabled`.

### TL-037 Markers — v1

**What the user sees:** Small colored flags. **Timeline markers** sit on the ruler at fixed
times. **Clip markers** sit on a clip and move with it. Hovering shows the name. Double-clicking
opens an editor for name, note, color and duration. Markers can be point markers or ranges.

**Inputs / controls:**
- *Add marker* (`M`, as in UI-031) adds a marker at the playhead. It becomes a clip marker if
  exactly one clip is selected and the playhead is inside it; otherwise a timeline marker.
- *Add marker with next color* adds one with the next color in the palette. Binding: see
  UI-031.
- *Go to next / previous marker*. Bindings: see UI-031.
- Drag to move (snaps). Drag a marker's right side to give it a duration.
- `Delete` with a marker selected removes it. The marker context menu offers *Delete all
  markers of this color*. The clip context menu offers *Delete markers on this clip*, and the
  ruler's context menu offers *Delete all timeline markers* (FreeCut names). Each bulk delete
  is one history entry and needs no confirmation (undo restores them).
- New markers are named automatically "Marker N" (FreeCut text), where N is one more than the
  highest number used by markers of the same owner (the sequence, or that clip); deleted
  numbers are not reused until the owner has none left.
- Editor fields:
  - Name: 0–120 chars.
  - Note: 0–2 000 chars.
  - Color: 8-color palette of our own, shared with track colors (TL-049) and clip labels
    (TL-045), default blue (FreeCut default).
  - Duration: ≥ 0 frames, where 0 means a point marker.
- **Toolbar (toolbar round, 2026-10-13):** a marker split button after the edit group in
  UI-011. The flag part adds a marker at the playhead (same as `M`, same clip-or-timeline rule).
  The chevron opens: *Add marker with next colour* (`Alt+M`), *Previous marker*
  (`Ctrl+Shift+M`), *Next marker* (`Shift+M`), a separator, *Delete markers on this clip*
  (enabled with one clip selected that has markers) and *Delete all timeline markers* (enabled
  when the sequence has any). Tooltip: "Add a marker at the playhead (M)" (FreeCut text).
- **Where they are drawn:** timeline markers are 9 pt flags standing on the ruler floor, in
  their colour, with a 1 pt line of the same colour down through the lanes while hovered or
  selected; a range marker draws a 4 pt bar along the ruler floor for its duration. Clip
  markers are 7 pt flags on the clip's name strip (on the top edge for all-strip kinds), drawn
  only while the clip is at least 24 pt wide. Selected markers get a 1 pt `accent` ring.
- **Navigation:** *Next* / *Previous marker* move the playhead to the nearest marker start
  strictly after / before it, among timeline markers and the clip markers of every clip on a
  visible track (FreeCut default). There is no wrap-around: past the last marker the command
  does nothing and the status line says "No more markers" (FreeCut text).
- **Editor:** double-clicking a marker, or *Edit marker…* in its context menu, opens a small
  popover anchored to the flag with Name, Note, the 8 colour swatches and Duration (a timecode
  field). Changes apply on close or `Enter` as one `EditMarker`; `Esc` discards them.

**Behavior / output:**
- Timeline markers are stored in sequence time. They stay put during ripple edits unless the
  per-sequence option *Markers follow ripple edits* is on (default off).
- Clip markers are stored in source time, so they follow moves and trims and divide on split.
  Markers trimmed out of view are kept and hidden, and they reappear if the clip is extended.
- Markers are snap targets (TL-028).
- **Beat markers** are the only storage for beats. Beat detection, its controls, the tempo
  readout and the tap key are AU-025's (tap latency per AU-026). Its results are stored here as
  **clip markers** with `origin = Beat`, in source time, each flagged *auto* (from detection)
  or *manual* (tapped or placed by hand). There is no separate beat list in AU's `ClipAudio`
  and no other beat-marker type. AI-005 rewrites only the auto beat markers when the beat grid
  is refined; AI-006 reads beat markers from here. Beat markers have a distinct style and can
  be removed in bulk (all, or auto only).

**Edge cases:**
- Several markers at the same time: drawn stacked, and the tooltip lists all of them.
- A range marker extending past the sequence end is allowed.
- Range markers on clips are clipped to the visible range when drawn.
- Copy and paste of clips carries their clip markers. Timeline markers are not copied.
- Deleting a clip deletes its clip markers.

**Undo:** `AddMarker`, `EditMarker { before, after }`, `MoveMarker`, `DeleteMarkers` are each
undoable. Toggling *Markers follow ripple edits* is a `SetSequenceFlag` (TL-003).

**Model notes:** `Marker { id, time: Tick (sequence or source time, by owner), duration: Tick,
name, note, color: PaletteColor, origin: MarkerOrigin }` with `MarkerOrigin::{User,
Beat { auto: bool }, Scene, …}`; `Beat` is valid only on clip markers (`validate()` checks
this). Sequence-level markers go in `Sequence.markers` and clip markers in `Clip.markers`.
AU-025's `SetAutoBeatMarkers` op replaces a clip's auto beat markers in one change set and
never touches manual ones. Chapter export from markers is EX (later).

Build contract (toolbar round, slice T2). The type is the architecture's
([timeline-model.md §8](../architecture/timeline-model.md)): `Marker { id: MarkerId, at:
MarkRange, name, note, color, origin: MarkerOrigin }`, where `at.start` and `at.duration` are
this entry's time and duration. The colour type is `PaletteColor`, the one palette shared
with TL-049 and TL-045; timeline-model.md §8 uses the same name (architecture C53). Clip markers
of generated clips (text, stickers, effects) are in clip-local time, as §8 says.
- `MarkerId` comes from the sequence id counter. `PaletteColor` is an 8-value enum (`Blue`
  first, the default); its RGB values are theme tokens in the app, so the model stores only
  the name.
- `Sequence.markers: Vec<Marker>` and `Clip.markers: Vec<Marker>` are kept sorted by
  `(at.start, id)`; `validate()` checks the order, `at.duration ≥ 0`, name and note lengths,
  and that `Beat` appears only on clip markers.
- `MarkerRef::{Timeline(MarkerId), Clip(ClipId, MarkerId)}` addresses one marker.
- Requests: `AddMarker { seq, at: Tick, owner: MarkerOwner::{Auto, Timeline, Clip(ClipId)},
  color: Option<PaletteColor> }` (`Auto` applies the `M` rule against the given selection and
  `color: None` takes Blue; *next colour* passes the colour after the last-used one),
  `EditMarker { seq, marker: MarkerRef, patch: MarkerPatch { name, note, color, duration } }`
  (each field `Option`), `MoveMarker { seq, marker, to: Tick }` (frame-quantized; a clip
  marker is clamped to the clip's source range, or its clip-local range for generated clips), `DeleteMarkers { seq, which:
  MarkerSelector::{Refs(Vec<MarkerRef>), Color(PaletteColor), OnClip(ClipId), AllTimeline} }`.
  `AddMarker` selects the new marker. Every request is one entry with its own `OpKind`.
- Split (TL-023) gives each clip marker to the part whose source range contains it; markers
  hidden by a trim go with the part nearest to them. Delete, copy and duplicate carry clip
  markers as TL-032/TL-033 say.
- Queries: `fn next_marker(seq, t, scope) -> Option<Tick>` and `prev_marker`, where `scope`
  says which clip markers count (visible tracks), and `SnapIndex::build` adds every marker
  start and range end as `SnapKind::Marker`.
- Persistence: the timeline model stays serde-free (architecture README C4); the project
  format's own records in `freecut-project` gain both marker lists in the next format version
  (PM-021), defaulting to empty, so older files load with no markers and the migration step
  only raises the version number.

Build notes (model, 2026-10-13):
- `MarkerOwner::Auto(Vec<ClipId>)` carries the selection that the `M` rule reads. `AddMarker`
  reports `Notice::AddedMarker(MarkerRef)`, and the app selects that marker.
- Clip markers are stored in the clip's content time: source time for media, and for generated
  clips the clip-local content time of TR-027's key origin. A head trim therefore keeps their
  sequence position. `Clip::marker_start(&Marker)` gives the sequence time of a shown marker
  and `None` for a hidden one.
- "Marker N" counts up from the highest N among the owner's current names. Gaps are not
  refilled, but a deleted top number can come back.
- `MarkerSelector::Color` covers timeline and clip markers. Clip markers on locked tracks are
  left alone, and `OnClip` on a locked clip is refused (TL-009).
- Queries: `next_marker` / `prev_marker(seq, t, MarkerScope)` with
  `MarkerScope::{TimelineOnly, VisibleTracks, AllTracks}` (`VisibleTracks` is the navigation
  default). `SnapIndex::build` adds the shown markers of every clip it doesn't exclude.
- `PaletteColor` slots, in order: Blue, Teal, Green, Yellow, Orange, Red, Pink, Purple. Each
  has a stable id (`id()` / `from_id()`) for the project format.
- *Markers follow ripple edits* is not built yet: timeline markers always stay put.

Build notes (app, 2026-10-14):
- The app keeps the selected markers beside the clip selection (`Project.markers`). Clicking a
  flag selects its marker and clears the clip selection; `Delete` removes the selected markers
  and clips as one entry.
- Dragging a timeline range's end changes its duration; clip markers move by their flag only,
  and their duration is set in the editor.
- The marker split button's menu also lists every marker by time; a row jumps the playhead
  there and selects the marker.

### TL-038 In/out range marks — v1

**What the user sees:** A highlighted region on the ruler between an in mark and an out mark. It
is used to export just that part (EX), to loop playback (EX), or to **Extract** or **Lift** that
range from the timeline.

**Inputs / controls:**
- `I` / `O` set in/out at the playhead. `Alt+X` clears both. Drag the marks on the ruler (they
  snap).
- Sequence menu: *Lift range* removes content inside the range on unlocked tracks and leaves
  gaps. *Extract range* removes it and closes the gap across all unlocked tracks.
- *Delete range from selected clips* (FreeCut name; proposal `Alt+Delete`, not yet bound in
  UI-031): with clips selected and a range set, removes only the part of those clips that lies
  inside the range. On the main track with the magnet on, the removed span ripples. On other
  tracks a gap is left, and with `Shift` it ripples that track (TL-021).

**Behavior / output:**
- Range marks are frame-quantized and need at least 1 frame between them. Setting In after the
  current Out (or Out before In) clears the other mark.
- Lift and Extract split clips at the range edges first, then remove the inside pieces.
  Extract then shifts everything after the out mark left by the range length on every unlocked
  track. With the magnet on, the main track stays contiguous automatically.
- Caption segments on unlocked caption tracks: segments fully inside the range are removed.
  Segments crossing a range edge are trimmed to the outside part with their text unchanged,
  rather than having their text split (FreeCut default).

**Edge cases:**
- Only In set: the range runs to the sequence end. Only Out set: from 0.
- Setting In and Out on the same frame is refused (status bar "A range needs at least 1
  frame"; FreeCut text), and the earlier mark stays.
- Extract with a locked track that has content in the range: that track is left unchanged and
  flagged as out of sync. A toast names it.
- A range entirely past the sequence end: Lift and Extract do nothing.
- *Delete range from selected clips* with selected clips that don't overlap the range: those
  clips are skipped and counted (TL-053); if none overlaps, the command is disabled.

**Undo:** Setting marks: n/a (view/transport state, persisted in the project). Lift, Extract
and *Delete range from selected clips* are undoable (`ExtractRange`, the last one with a clip
filter).

**Model notes:** `RangeMarks { in_: Option<Tick>, out: Option<Tick> }` is in the project UI
section. `plan_extract(seq, range, mode, filter: Option<&BTreeSet<ClipId>>)` is in the timeline
crate; AI-011's transcript deletions use the same op with a clip filter.

### TL-039 Group / ungroup — v1

**What the user sees:** Several clips, possibly on different tracks, can be grouped so they
select and move as one unit (for example a lower-third made of a shape, two text items and a
sound effect). A grouped clip shows a small chain badge.

**Inputs / controls:** `Mod+G` groups the selection (≥ 2 clips). `Mod+Shift+G` ungroups.
`Alt`+click selects a single member.

**Behavior / output:**
- Clicking any member selects the whole group. Double-clicking a member that isn't a compound
  clip selects just that member, like `Alt`+click, so its own properties can be edited in the
  inspector (FreeCut default). With the whole group selected, the inspector shows the shared
  properties (UI-010). The reference app, as observed, makes members editable only after
  "entering" the group; FreeCut needs no separate mode.
- Move, delete, copy, cut, duplicate and enable/disable act on all members.
- Trim acts only on the dragged member's edge when the group is selected on its own. A wider
  selection (more than this one group) trims every selected clip (TL-051).
- Ungroup confirms with a short toast ("Ungrouped 4 clips").
- Split splits the members under the playhead, and both halves remain in the group.
- Groups do not nest (FreeCut default): grouping a selection that includes existing groups
  merges them into one new group.
- A clip belongs to at most one group.

**Edge cases:**
- A group containing main-track clips (magnet on): a move inserts the main-track members at the
  target cut point and shifts the others by the same delta. If the main-track members aren't
  contiguous, or any member's move is invalid, the move is refused.
- A group whose members go below 2 (deletion) dissolves automatically.
- Attachments still apply to group members on overlay tracks. Moving the owner shifts them, and
  the group's relative layout is preserved when all members are attached to the same owner.
- A group can't include clips from different (nested) sequences.

**Undo:** `Group { clips, group_id }`, `Ungroup { group_id, members }`.

**Model notes:** `GroupId`. `Sequence.groups: BTreeMap<GroupId, BTreeSet<ClipId>>` plus
`Clip.group` (kept consistent; validated).

### TL-040 Compound clips — v1

**What the user sees:** Several clips (across tracks) collapse into one **compound clip**, a
single bar that behaves like a video clip: it can be moved, trimmed, split, given effects or a
speed change. Double-clicking opens it to edit its contents, with a breadcrumb ("Timeline ›
Compound 1") to go back up. *Break apart* restores the original clips.

**Inputs / controls:**
- `Alt+G` = create compound clip from the selection. `Alt+Shift+G` = break apart (as in UI-031).
- Double-click (or `Enter` with it selected) = open. The breadcrumb goes up. `Esc` first
  clears any selection (TL-015); with nothing selected it goes up one level.
- Context menu: *Rename* (TL-048's `RenameClip`; the breadcrumb shows the new name),
  *Pre-render* (cache for smooth playback, owned by EX-016), *Save as snippet…* (TL-055,
  later).
- Inside an open compound clip, the timeline area carries a thin accent-colored frame and the
  breadcrumb stays visible, so the user always knows they are editing nested content.

**Behavior / output:**
- **Create:** a nested sequence is made with the selected clips at their relative times and
  relative track layout.
  - The lowest selected video-track clips become its main track. If the selection includes
    main-track clips, those form the nested main track.
  - The compound clip spans `[earliest start, latest end]` of the selection and is placed on:
    - the main track, if any selected clip was on it (magnet rules apply), or
    - otherwise the lowest selected visual track, if that span is free after removing the
      selected clips, or
    - otherwise a new track above.
  - An all-audio selection creates an audio compound clip on an audio track.
  - Interior gaps render transparent or silent.
- **Edit inside:** all ordinary editing works. Changes update this compound clip's duration (it
  is the nested sequence's duration). If the nested content gets shorter than the compound's
  current source range, the compound clip shrinks, rippling on the parent main track.
- **Break apart:** the compound clip is replaced by its contents at parent time, offset by the
  compound's start minus its source in-point. Content outside the compound's visible source
  range is trimmed away, after confirmation if anything would be lost. Properties applied to
  the compound clip itself (effects, transform, speed) can't be distributed; a warning lists
  them and they are dropped.
- Compound clips render by compositing the nested sequence at the parent's canvas size and
  frame rate (render/FX). Their audio is the nested mix.
- Settings applied to a compound clip (effects, color, speed, audio processing) act on its
  composited output, never on the clips inside it (FreeCut default). The reference app, as
  observed, passes some processing through to the inner clips; FreeCut keeps one rule so the
  result doesn't depend on which tool was used.
- **Pre-render** is EX-016's cache: its states (queued, percent done, done, failed) show on the
  compound clip and in PM-073's task list. It is not a document change, so it is not in history
  (the reference app, as observed, lists it as undoable); *Clear render cache* removes it.
  Editing inside invalidates only the cached segments the edit affects.
- A compound clip can't be exported on its own: Export always exports the main timeline (EX),
  even from inside a compound clip. To export just that part, set range marks around it
  (TL-038) and export the range.

**Edge cases:**
- On the main track (magnet on), the selected main-track clips must be contiguous. Otherwise
  creation is refused with a toast.
- Break apart that would need more tracks than the parent's limit (TL-004): refused as a whole,
  nothing changes, and the toast names the limit. Large compound clips (hundreds of inner clips)
  break apart in one step; there is no need to unpack them piece by piece.
- Deleting all content of an inner track while inside: those clips go, the track is
  auto-removed (TL-006), and the compound clip's length follows its nested content as above.
- Some tools refuse compound clips (for example per-clip analysis tools that need a single
  media source); each owner states this through TL-054, so the reason is shown.
- Nesting depth is limited to 8 (FreeCut default). A compound clip can never contain itself
  (cycle check on paste and create).
- Duplicating, copying or splitting a compound clip deep-copies the nested sequence. Each
  instance edits independently (FreeCut default): a nested sequence is referenced by exactly
  one compound clip, which `Timeline::validate()` checks. The reference app, as observed, keeps
  every instance of a nested sub-project linked (editing one changes all, and deleting the
  sub-project from its media-panel list removes every instance), and has to explain that to
  surprised users. FreeCut keeps instances independent; reuse across projects is TL-055 and
  merging projects is TL-056, both by copy.
- An empty nested sequence (everything deleted inside): the compound clip keeps a 1-frame
  duration and renders transparent. The user is prompted to delete it on leaving.
- Undo history is one stack across parent and nested sequences. Undoing an edit made inside a
  compound clip while viewing the parent applies it and briefly highlights the compound clip.
- Speed change on a compound clip (TR) applies to its whole output.

**Undo:** `CreateCompound { removed clips, new seq, new clip }`,
`BreakApart { … }`. Inner edits are ordinary ops tagged with the sequence id.

**Model notes:** `Timeline { root, sequences: BTreeMap<SeqId, Sequence> }` in
`freecut-timeline` (TL-003); `freecut-project` persists it. `ClipContent::Compound
{ seq, source }`. Cycle detection is a DFS over compound references. The ops above take a
`SeqId` so every op works at any nesting level.

### TL-041 Zoom & scroll — mvp

**What the user sees:** The timeline can be zoomed out to see the whole edit or zoomed in to
individual frames, and scrolled horizontally and vertically.

**Inputs / controls:**
- Zoom slider in the timeline toolbar. `Mod+=` / `Mod+-` zoom in and out by a factor of 1.25
  (FreeCut default). The anchor is the playhead if visible, else the view center.
- `Mod`+wheel or trackpad pinch: zoom around the pointer. A plain wheel never zooms, so
  scrolling can't change the zoom by accident.
- `Shift+Z`: fit the whole sequence in view, with 5 % padding.
- Wheel: vertical scroll of tracks. `Shift`+wheel: horizontal scroll. Trackpad two-finger
  horizontal swipe scrolls horizontally.
- Middle-button drag: pan (FreeCut default). The Hand tool (TL-024) pans with a plain drag.
- **Toolbar group** (UI-011, right end): *Zoom out*, a 96 pt slider, *Zoom in*, *Zoom to fit*.
  The slider is logarithmic: its position is `ln(px_per_sec)` mapped linearly between the
  minimum and maximum zoom, so each step of the buttons (×1.25) moves the thumb by the same
  distance at every zoom level. Dragging the slider anchors on the playhead when it is visible,
  like the keys. Double-clicking the slider fits the sequence (FreeCut default). The buttons
  disable at the limits, with the tooltip "Zoomed in all the way" / "Zoomed out all the way"
  (FreeCut text).

**Behavior / output:**
- Zoom range, in pixels per second: the minimum lets 24 h fit in the view width, and the
  maximum makes one frame 120 px wide at the sequence frame rate.
- The ruler header and track headers stay pinned while content scrolls.
- Zoom and scroll are saved per sequence in the project UI section.

**Edge cases:**
- Empty sequence: zoom-to-fit shows 0–30 s.
- Very long sequences: clip drawing is culled to the visible range, so frame time doesn't
  depend on clip count.
- Zoom during playback keeps the playhead's screen position stable.

**Undo:** n/a (view state).

**Model notes:** `TimelineView { px_per_sec: f64, scroll_x_px: f64, scroll_y_px: f64 }` is
float, which is allowed because it is UI-only. Visible-range queries,
`clips_in_range(track, TimeRange)`, use binary search over the sorted clips.

### TL-042 Timeline thumbnails & waveforms — mvp

**What the user sees:**
- Video clips show a filmstrip of frames. Audio clips, and the audio band of video clips, show
  waveforms.
- Text items show their text, stickers a small preview, effects and adjustments their name and
  icon.
- Each clip shows its display name (TL-048) and duration when there's room.

**Inputs / controls:** Preferences (v1): filmstrip mode (*Filmstrip* default, *First frame
only*, *First & last*, *None*); *Show waveforms* (default on).

**Behavior / output:**
- Thumbnail tiles are as wide as track height × source aspect. One tile is drawn per tile-width
  of clip, sampled at the tile's center time.
- Thumbnails and waveform peaks are generated by `freecut-media` on background workers and
  cached on disk (cache location owned by PM). A shimmer placeholder shows until they're ready.
- Waveforms are drawn from cached peak data at the resolution matching the zoom level (a
  multi-resolution peak pyramid).
- Waveforms show the source signal. The clip's volume and fades are drawn as an overlay line
  (AU).

**Edge cases:**
- Offline media: striped "offline" pattern, no thumbnails.
- Very short clips (narrower than one tile): one cropped tile.
- Rapid zooming cancels outstanding thumbnail requests for tiles no longer visible.
- Huge projects: the thumbnail cache is LRU-bounded at 2 GB on disk by default (preference) and
  256 MB in memory (FreeCut default).
- Images: a single repeated tile. Compound clips: a thumbnail of the nested composite, rendered
  at low resolution.
- Rotated or flipped clips show the source orientation (FreeCut default), so thumbnails don't
  need re-rendering on every transform edit.

**Undo:** n/a (derived display).

**Model notes:** `freecut-media`: `ThumbnailRequest { asset, time, height }`,
`PeakRequest { asset, stream, level }`. `freecut-app` owns the request queue and texture cache.
The timeline crate is uninvolved beyond providing clip ranges.

**FreeCut defaults (app, mvp waveforms):**
- Peaks are scanned once per library item and audio stream (the item's first stream, the one
  it plays), on a background worker of their own so long files don't delay imports, and are
  kept in memory for the open project (the disk cache comes later). The finest level holds
  min/max per 1/100 s of media time; each coarser level merges four buckets. Each pixel column
  maps through the clip's source range and reads its exact span, coarse levels in the middle
  and fine ones at the edges, so short peaks are never skipped or smeared into a neighbour.
- An extracted audio item that still reads its parent's file shares the parent's peaks.
- Audio clips draw the waveform under their name; a video clip with its own sound draws a
  12 px strip along its bottom. Muted clips draw it grey. Offline media draws none. Until the
  peaks arrive a flat centre line shows; if the stream can't be read, the band says "Waveform
  unavailable" and why.
- Audio clips are green; extracted audio is a bluer green, so it reads apart from imported
  files (AU-001's origin colours, first two categories).

### TL-043 Undo / redo — mvp

**What the user sees:** Undo reverses the last edit and Redo reapplies it, for every change to
the project. The Edit menu names the action ("Undo Split clip").

**Inputs / controls:** `Mod+Z` = undo. `Mod+Shift+Z` (and `Ctrl+Y` on Windows/Linux) = redo.
Edit menu items and toolbar arrows.

**Behavior / output:**
- **One linear history per open project**, covering every document change: timeline edits
  (this spec) and property edits from all other specs (transform, text, color, audio, effects,
  library organization). Other specs register their edits as commands on this stack.
- Each history entry = one user action, applied atomically as a `ChangeSet` of primitive
  changes (insert, remove or update of a track, clip, marker, group, sequence, or property bag)
  with before and after values. Undo applies the inverse change set. Redo reapplies it.
- A new edit after an undo discards the redo branch.
- **Transactions:** composite actions are one entry. Examples: a drag with ripple, attachment
  shifts and auto-created or removed tracks; freeze frame; replace with shortening.
- **Coalescing:** continuous interactions produce one entry from pointer-down to pointer-up.
  Examples: dragging an inspector slider, dragging a clip. Keyboard repeats of the same command
  on the same target within 1 s also coalesce (nudge, volume steps).
- **Validation first:** every op is planned as a pure function returning
  `Result<ChangeSet, EditError>`. If planning fails, the document is untouched, no history entry
  is created, and the user sees a toast with the reason.
- Depth: 200 entries (FreeCut default; preference 20–1 000). The oldest entries drop first.
  There is also a memory budget of 512 MB of change data (FreeCut default). Above it, the
  oldest entries are dropped until history fits. The newest entry is always kept, even if it
  alone exceeds the budget.
- Undo restores the selection captured before the op. Playhead and view do not change, except
  that the playhead is clamped if the sequence got shorter.
- **Batch edits** (apply-to-all, multi-selection inspector edits, multi-clip paste, split
  all) are one entry each, labelled with the command and a count ("Apply color to 12
  clips"); TL-053 defines how they skip and report items.
- **Not undoable, therefore announced:** an action that changes something outside the
  document and can't be undone (writing a template, OV-030; saving a snippet, TL-055; clearing
  caches; deleting files from disk) says so in its confirmation before it runs, and adds no
  history entry. Removing media from the project is not one of them: it stays undoable
  (PM-058). The reference app, as observed, treats deleting media from its bin as not undoable.
- **In history although they are toggles or cosmetic:** every sequence flag (main-track
  magnet, follow main track and its kind set, flexible stacking, ripple all tracks, markers
  follow ripple; TL-003), track mute/hide/lock flags (TL-008, TL-009), clip names (TL-048) and track colors
  (TL-049), because they are saved in the document. AU's master and track audio ops
  (`SetMasterGain`, `SetTrackGain`, `SetTrackPan`) are registered on this stack too.
- **Not in history:** view state (zoom, scroll, track heights, playhead, panel layout, hover
  skim), preferences (snapping, sub-frame audio editing TL-050), transient selection, range
  marks, the clipboards (TL-032, TL-034), and background work
  (thumbnails, waveforms, proxies). Background tasks that change the document,
  such as applying auto-generated captions, add one entry when applied.
- The dirty flag compares the current history position with the saved position. Undoing back
  to the saved point clears it.

**Edge cases:**
- Undo or redo during a drag: ignored until the drag ends. During playback: allowed, and
  playback continues on the new state.
- Undo of an edit referencing media that has since gone missing: it restores fine, and the clip
  is offline.
- Undo after a partial internal failure: impossible by construction, because a change set is
  either applied fully or not at all. In debug builds, primitive application is asserted, and
  the applier rolls back already-applied primitives if one fails. That case is a bug and is
  logged.
- History is in memory only. Saving doesn't clear it. Reopening a project starts with an empty
  history (FreeCut default). Autosave/crash recovery is PM.
- Undo of edits made inside a compound clip while viewing another sequence: applied, with a
  highlight (TL-040).
- Undo of a track lock or unlock toggle: restores the flag only.

**Undo:** This is the mechanism itself.

**Model notes:** `freecut-timeline::history`: `ChangeSet { label: &'static str/LabelId,
changes: Vec<Change> }`, `Change::{Insert(Entity), Remove(Entity), Update{ id, before, after }}`,
`History { undo: VecDeque<Entry>, redo: Vec<Entry>, saved_at: Option<usize>, coalesce_key }`.
Test pattern required by CLAUDE.md, for every op:
1. Plan and apply, then assert invariants.
2. Undo, then assert the state equals the original (structural equality).
3. Redo, then assert it equals the post-apply state.

Property tests (`proptest`) run random op sequences against `Sequence::validate()`.

### TL-044 Edit history panel — later

**What the user sees:** A panel listing past actions ("Move 3 clips", "Split clip", "Change
opacity"), newest at the top. Clicking an entry jumps back to that point, and the later entries
grey out until a new edit discards them.

**Inputs / controls:** View menu → *History*. Click an entry to undo or redo to it.

**Behavior / output:** Equivalent to repeated undo or redo. Labels come from `ChangeSet.label`
plus counts.

**Edge cases:** Jumping across a save point updates the dirty flag. Very long histories are
virtualized in the list.

**Undo:** It is navigation of the history itself.

**Model notes:** Needs only the labels, which are required from MVP onwards.

### TL-045 Clip color labels — later

**What the user sees:** Clips can be tinted with one of 8 label colors to organize the
timeline, and selected by color.

**Inputs / controls:** Context menu *Label color* (8 colors from our own palette, shared with
markers and track colors, plus *None*; default *None*) and *Select clips with this color*.
Applies to the whole
selection.

**Behavior / output:** The label tints the clip body, overriding the track color (TL-049). It
doesn't affect output.

**Edge cases:** The label is preserved through split, copy and duplicate. Compound clips keep
their own label, independent of the labels of clips inside them. *Select clips with this color*
skips locked tracks and finds nothing inside unopened compound clips. Setting the color a clip
already has makes no history entry.

**Undo:** `SetClipLabel`.

**Model notes:** `Clip.label: Option<PaletteColor>` (the palette type shared with TL-037 and
TL-049).

### TL-046 Reveal source & match frame — later

**What the user sees:** From a clip, **Reveal in library** highlights its media item. **Match
frame** opens the library preview at the same source frame as the playhead.

**Inputs / controls:** Context menu; key binding left to UI-031 (proposal `F` in timeline
scope, where UI-031's asset-panel `F` does not apply).

**Behavior / output:** Uses `MediaRef` and the playhead-to-source mapping (TR).

**Edge cases:** Generated items: disabled. Offline media: reveals the library item marked
offline.

**Undo:** n/a.

**Model notes:** Query helper `source_time_at(seq, clip, t)`.

### TL-047 Insert & overwrite edits from the source preview — later

**What the user sees:** In the library preview, the user marks an in/out range on a source and
then presses **Insert** (pushes later content right) or **Overwrite** (replaces what is there)
to place that range at the playhead on a chosen target track.

**Inputs / controls:** `I` / `O` while the source preview has focus set source in/out
(frame-quantized to the source's own frame grid). *Insert* and *Overwrite* buttons (their key
bindings are left to UI). A target-track selector (defaults to the main
track; lists only tracks compatible with the source's kind).

**Behavior / output:**
- Insert = split at the playhead plus ripple-insert on the target track (and on all tracks with
  *Ripple all tracks*).
- Overwrite = clear the span on the target track, then place the clip.
- On the main track with the magnet on, Insert lands at the playhead only if it is on a cut
  point; otherwise the clip under the playhead is split there first. Overwrite on the magnetic
  main track replaces the covered span and keeps the track contiguous (no gap is created).

**Edge cases:**
- No in/out marks means the whole source. Only In: to the source end. Only Out: from source 0.
- Target track locked: the action is refused with a toast.
- Audio-only source with a video target track (or the reverse): the selector won't offer it;
  if the remembered target is incompatible, the nearest compatible track is used.
- Overwrite past the sequence end extends the duration. Overwrite partly covering clips trims
  or splits them as in TL-012's overwrite drop.
- Source still being probed or offline: buttons are disabled.

**Undo:** `InsertClips` / `OverwriteClips`.

**Model notes:** Reuses `plan_insert` with explicit source ranges.

### TL-048 Rename clip — v1

**What the user sees:** Every clip has a label: by default its media's name (or, for generated
items, a name derived from their content). **Rename** replaces that label with the user's own,
for example "Interview A – take 3". The custom name shows on the clip bar, in the inspector
header (UI-008), in tooltips, in the compound-clip breadcrumb (TL-040) and in the undo label
("Undo Rename clip"). Clearing the name brings the default back.

**Inputs / controls:**
- *Rename* in the clip context menu (UI-035) and the Clip menu (UI-012), and *Rename focused
  item* (`F2`, UI-031) with a clip focused. Double-clicking the name in the inspector header
  also starts it (FreeCut default).
- Inline text field on the clip bar (or in the inspector header when the bar is too narrow).
  `Enter` commits, `Esc` cancels, clicking elsewhere commits.
- Length 0–120 characters (FreeCut default, same as marker names). Leading and trailing spaces
  are trimmed and control characters are stripped. An empty result clears the override.
- Enabled with exactly one clip selected (FreeCut default). Batch renaming is not offered.

**Behavior / output:**
- The name is a display label only. It never changes output, the media file, or the library
  item's name (renaming media in the library is PM's).
- Default display names (FreeCut default): media clips use the library item's current name, so
  a library rename shows up on clips that have no override; text items use their first line
  (TX); stickers, shapes, effects and adjustments use their library item's name; compound clips
  use "Compound N"; freeze frames use the source name plus "(freeze)".
- The override survives split (both parts keep it), copy, paste, duplicate, replace media
  (TL-035) and detach audio (the new audio clip inherits it, TL-030).
- Names aren't identifiers: several clips may share a name.

**Edge cases:**
- Committing the current name unchanged, or an empty field on a clip that has no override: no
  history entry.
- Clip on a locked track: it can't be selected (TL-009), so Rename is unavailable.
- Offline media: the default name is the last known file name (TL-011); an override still
  shows.
- Very long names: ellipsized on the bar, shown in full in the tooltip and inspector.
- Mixed scripts, right-to-left text and emoji are stored as typed; rendering is UI's (UI-046).
- Renaming inside an open compound clip renames only that inner clip; the compound clip keeps
  its own name.
- A rename started during a drag or during playback: allowed during playback, refused during a
  drag (the field opens after the drag ends).

**Undo:** `RenameClip { clip, before: Option<String>, after: Option<String> }`, one entry per
commit.

**Model notes:** `Clip.name: Option<String>` (TL-011). A pure helper `display_name(&Clip,
&NameSource) -> Cow<str>` in `freecut-timeline` takes library names as plain data (no I/O),
so the default-name rules are unit-tested. `freecut-project` persists the override only.

### TL-049 Track color — v1

**What the user sees:** Each track header shows a thin color stripe. *Track color ▸* in the
track header context menu (UI-035), and the color field in the inspector's Track tab (UI-008),
let the user pick one of 8 colors from FreeCut's own palette or *Default*. Clips on a colored
track are tinted with that color instead of their per-type default color, so, for example,
all B-roll tracks can be green and all voice tracks orange. Label text on the clips switches
between light and dark to stay readable (UI-013, UI-043).

**Inputs / controls:**
- Palette: the 8-slot palette shared with markers (TL-037) and clip labels (TL-045), plus
  *Default* (no color). Default for new tracks: *Default* (FreeCut default).
- Shift-click a swatch to apply it to every track of the same kind (FreeCut default, like
  TL-009's Shift-click).

**Behavior / output:**
- Color priority on a clip body: clip label (TL-045, later) > track color > the per-type
  default from UI-013 (and, on audio tracks, AU-001's origin colors).
- Color is cosmetic: it never affects preview or export.
- The model stores a palette slot, not RGB, so each theme (dark, light, high contrast; UI-013,
  UI-014) maps the slot to a shade that meets its contrast rules.

**Edge cases:**
- The main track and caption tracks can be colored like any other track. On a caption track
  the color tints its segment blocks (TX-025's default caption color applies otherwise).
- Locked track: the color can still be changed, because it doesn't touch clips (TL-009).
- Hidden track: keeps its color, drawn dimmed.
- A track auto-created because its target was occupied (TL-006, TL-012, TL-018) takes the
  color of the track the item was heading for, so an overflow of B-roll stays green (FreeCut
  default). Other new tracks start at *Default*.
- Undo of a track removal (manual or automatic) restores its color with the track snapshot.
- Choosing the color a track already has: no history entry.
- Tracks inside a compound clip's nested sequence have their own colors.
- A project file with an unknown palette slot (written by a newer version): loaded as
  *Default*, with a log entry.

**Undo:** `SetTrackColor { tracks, before, after }`, one entry per pick (a Shift-click is one
entry for all affected tracks).

**Model notes:** `Track.color: Option<PaletteColor>` (TL-004). `PaletteColor` is one 8-slot
enum used by markers, clip labels and track colors. Its RGB values live in UI's theme tokens
(`freecut-app`), never in the timeline crate.

### TL-050 Sub-frame edit points for audio-only clips — later

**What the user sees:** With **Sub-frame audio editing** turned on, trimming, moving or blading
an audio clip on an audio track no longer jumps from frame to frame. It lands on individual
audio samples, so a breath, click or plosive can be cut out exactly, and a separately recorded
track can be lined up more tightly than one frame. Video clips, including their embedded
audio, still edit on the frame grid. This answers the open request in AU-001 and AU-035.

**Inputs / controls:**
- Preference *Sub-frame audio editing* (default off; per user, like snapping), in the Timeline
  menu and preferences. Key binding: UI-031 decides; unbound by default.
- With it on, sub-frame placement happens when the timeline is zoomed in far enough that one
  frame is wider than 24 px (FreeCut default). At lower zoom, edits still snap to frames,
  because sub-frame precision can't be seen there.
- The ruler shows sub-frame ticks over audio tracks at that zoom, and trim tooltips show the
  change as frames plus milliseconds (for example "+2f 7.3 ms").

**Behavior / output:**
- Applies only to media clips on `Audio` tracks: imported audio, detached audio (TL-030),
  recordings and generated speech. Their start, in-point, out-point and blade position are
  quantized to the **project sample grid** (one sample at the project sample rate, PM) instead
  of the frame grid. TL-001's tick rate divides every common sample rate exactly, so these
  positions are exact integers in ticks.
- Not affected: video, image, text, sticker, effect, adjustment and caption items; audio
  compound clips; the playhead, markers and range marks, which stay on the frame grid.
  Splitting at the playhead therefore stays frame-aligned. Only the blade tool (TL-024) and
  edge drags reach sub-frame positions.
- Snapping (TL-028) runs first. Snapping to a frame-aligned target gives an exactly
  frame-aligned result.
- Ripple: a sub-frame change ripples later clips on the same audio track by the same sub-frame
  amount. It never moves non-audio tracks off the frame grid: with *Ripple all tracks* on, the
  edit falls back to the frame grid and a one-time toast explains why.
- Turning the preference off moves nothing. Existing sub-frame positions are kept, and the
  next frame-grid edit of an edge puts that edge back on the frame grid.
- AU-035 (later) sync may place an audio clip at sample precision through an ordinary move op
  when this feature is available; without it, its result is rounded to the frame grid.

**Edge cases:**
- Minimum clip duration stays 1 frame, so a clip is never too short to see or select.
- A sequence whose last item ends between frames: see TL-003; export and playhead use the
  rounded-up frame length.
- Project sample rate changed later (PM): positions keep their tick values, and the mixer maps
  ticks to samples exactly as AU-010 defines. No conform step is needed, and positions may
  then sit between samples of the new rate, which is harmless.
- Conform to frame grid (TL-001) after a frame-rate change leaves sub-frame audio edges alone;
  it only re-quantizes frame-grid edges.
- Linked audio (TL-030) moved by a sub-frame amount: the out-of-sync badge shows the offset
  with one decimal of a frame (for example "+0.4f") (FreeCut default).
- Attachments (TL-029) anchor at sub-frame source offsets without change.
- Audio crossfades (AU-029) at a sub-frame cut: allowed. Their length rules are AU-029's.
- Keyboard nudge (TL-019) still moves by whole frames.

**Undo:** No new ops. Trim, move and split ops already store exact tick values, so undo
restores sub-frame positions exactly. The preference itself is not a document change (n/a).

**Model notes:** `enum EditGrid { Frame(FrameRate), Sample(SampleRate) }` and
`quantize(t, grid)` in `freecut-timeline::time`. The plan functions (`plan_trim`, `plan_move`,
`plan_split`) take the grid chosen by the app per edit (sample grid only for eligible clips
when the preference and zoom allow it). `validate()` doesn't require frame alignment for any
clip (positions can already be off-grid after a frame-rate change until conformed), so no
invariant changes. Property tests add random sub-frame audio edits and check that no non-audio
item ever leaves the frame grid.

### TL-051 Trim several clips at once — v1

**What the user sees:** With several clips selected, dragging the left or right edge of any one
of them trims that same edge on every selected clip by the same amount. The tooltip shows the
change and the count ("−00:00:00:12 · 4 clips"). It is the quick way to shave the same head or
tail off a run of B-roll clips, or to shorten a stack of overlay items together.

**Inputs / controls:**
- Drag a left or right edge of a selected clip while the selection holds more than one clip
  and is not just the dragged clip's own group (TL-039). Snapping applies to the dragged edge
  only; `Mod` suspends it.
- `Esc` during the drag cancels.

**Behavior / output:**
- Every selected clip gets the same delta on the same edge (a relative trim), taken from the
  dragged edge. Each clip follows TL-020's rules for its track: ripple on the main track with
  the magnet on, non-ripple elsewhere (with `Shift`, ripple per track, TL-021).
- The delta is clamped to the tightest limit among all selected clips: source limits (TL-011),
  a neighbor's edge on non-ripple tracks, and the 1-frame minimum. Every clip therefore changes
  by exactly the same amount (FreeCut default), and the tooltip says "limited by <clip name>"
  while a clamp applies. Clamping each clip on its own while the others keep going is not used,
  because a drag that silently stops for only some clips is hard to read.
- Several main-track clips with the magnet on: their ripples are applied in one pass from the
  latest clip to the earliest, so the main track stays contiguous; attached items follow
  (TL-029).
- Caption segments in the selection are trimmed by TX's segment rules. Markers and gaps in the
  selection are ignored.

**Edge cases:**
- Selection is exactly one group: only the dragged member is trimmed (TL-039).
- A selected clip whose edge can't move in the drag direction at all (a media clip already at
  its source end, or a non-ripple clip touching its neighbor) clamps the delta to 0 in that
  direction, and the tooltip names it.
- Two selected clips touching on the same non-ripple track: extending the left one's right edge
  is blocked by the right one, which only moves its own right edge.
- Transitions on trimmed edges: FX-002's clamp per clip, as in TL-020.
- Mixed kinds (video, text, audio): allowed; each uses its own source limits.
- A drag that ends where it started (delta 0): no history entry.

**Undo:** One `TrimClips { edits: Vec<(ClipId, Edge, before, after)>, ripple_changes }` entry;
undo restores every clip and every shift.

**Model notes:** `plan_trim_many(seq, clips, edge, delta, mode) -> Result<ChangeSet>` in
`freecut-timeline`, built on `plan_trim`. It computes the common clamp first, then plans each
clip. Property test: for random selections, apply then invert is the identity and no overlaps
appear.

### TL-052 Join clips (heal a through edit) — later

**What the user sees:** Two neighboring pieces of the same clip, such as the halves left by a
split, can be joined back into one clip. A cut that can be joined is drawn as a dotted line
instead of a solid one (a "through edit"), so the user sees where nothing actually changes.
*Join clips* in the clip context menu and the Clip menu joins the selected pieces.

**Inputs / controls:**
- Select two or more clips on one track, or right-click a dotted cut, and choose *Join clips*.
  Key binding: UI-031 decides; unbound by default.
- Preference *Show through edits* (default on).

**Behavior / output:**
- Two clips A|B on the same track can be joined when all of these hold:
  1. They touch (`A.end == B.start`) and no transition sits on the cut.
  2. Same content: the same `MediaRef` and streams, with B's source in-point equal to A's
     source out-point (continuous); or, for items without a source range, the same kind and
     equal content (for example two text items with identical text, or two clips of the same
     image).
  3. Equal property bags, compared by value, including keyframes and the time map. A split
     gives both parts all keyframes (TR-027), so untouched split halves compare equal. Speed
     curves and reverse can't be joined.
  4. Same enabled state, group and link membership.
- The joined clip keeps A's id, name override and attachment, and spans both ranges. B's clip
  markers (already in source time) merge into A's. Items attached to B re-anchor to A at the
  same source frames, so nothing moves. Links that held both keep one member.
- With more than two clips selected, every joinable neighbor pair is joined in timeline order.
  Pairs that can't join stay as they are and are listed with the reason ("different speed",
  "not continuous in the source", "transition on the cut"; TL-053, TL-054).
- Compound clips can't be joined: after a split their halves own separate nested sequences
  (TL-040).

**Edge cases:**
- B's name override differs from A's: A's is kept, and the summary says so.
- A and B differ in one setting (for example a fade-out on A): not joinable, and the summary
  names the setting so the user can make them equal first. Differing settings are never merged
  silently (FreeCut default).
- Pieces from two library items that are copies of the same file (PM-038 duplicates): not
  joinable, because TL compares `MediaRef`s, not files.
- Offline media: joinable (metadata only).
- The reference app, as observed, has a merge command that can fail with a generic error;
  FreeCut states the exact reason for each cut instead.

**Undo:** `JoinClips { kept: ClipId, before: ClipSnapshot, removed: Vec<ClipSnapshot>,
reanchored }`; undo restores the original pieces with their ids.

**Model notes:** `fn joinable(seq, a, b) -> Result<(), JoinRefusal>` and `plan_join(seq,
clips) -> Result<ChangeSet>` in `freecut-timeline`; bag equality goes through each `PropBag`'s
`PartialEq`. AI-011 (transcript-based editing, later) uses `plan_join` to rejoin pieces when a
deleted range is restored. Through-edit drawing is app-side and calls `joinable`.

### TL-053 Batch edits: scopes, skipped items & one history entry — v1

**What the user sees:** Commands that apply one change to many items behave the same way
everywhere: the *Apply to all* buttons (TR-014, CL-025, FX-006), the inspector with several
clips selected (UI-010), Paste attributes onto several clips (TL-034), Split all (TL-023), and
enable/disable or mute on a selection (TL-036, AU-007). They change everything they can in one
step, skip what they can't, and say what was skipped: "Applied to 12 clips · 3 skipped: locked
track (2), not supported (1)" (FreeCut text). *Show skipped* in the toast selects the skipped
items.

**Inputs / controls:**
- Where a command offers a scope choice, it uses FreeCut's shared scope words (FreeCut
  default). Each owning spec picks which scopes it offers and which is the default:
  - **Selected items:** the current selection after group expansion (TL-015).
  - **This track:** every item on the source item's track.
  - **All of this kind:** every item of the source's kind in the sequence being edited (inside
    an open compound clip, its nested sequence).
  - **Inside the range:** items overlapping the in/out range (TL-038), offered only while range
    marks are set.
- An owner may add an *Include clips inside compound clips* checkbox (default off) where its
  setting makes sense there.
- The summary toast (UI-040) with *Show skipped*.

**Behavior / output:**
- The scope resolves to items in timeline order (start time, then track bottom to top), so
  results and tests are deterministic.
- Items on locked tracks are skipped and counted. Then items that the owner's eligibility check
  refuses are skipped and counted per reason (TL-054 for timeline ops, the category's
  `applies_to` for property categories).
- Each remaining item is planned, and all changes commit as **one** history entry with a
  counted label (TL-043).
- **Partial success is normal** (FreeCut default): skipped items never roll back the others.
  Only edits whose layout would be scrambled if half-applied stay all-or-nothing, and their
  specs say so: multi-clip moves (TL-018), duplicate (TL-033), compound creation (TL-040).
- Every item skipped: nothing changes, no history entry is made, and the toast lists the
  reasons.
- Duration changes across items (for example a speed paste) are applied in one ripple pass
  from the latest item to the earliest, as in TL-025.
- No scope reaches beyond the project's single timeline. FreeCut has one top-level sequence per
  project (see Out of scope), so there is no "every timeline" scope like the one the reference
  app, as observed, offers.

**Edge cases:**
- Items on hidden tracks and disabled items are included (hidden is not locked).
- A selection that mixes kinds: each item uses its own kind's eligibility. A command named for
  one kind ("Apply color") leaves other kinds out without counting them; any other command
  counts them as skipped.
- 1,000 or more items: still one entry. Planning is model-only and targets UI-010's 100 ms; if
  it takes longer, the UI shows a busy cursor. Nothing decodes or touches files on the UI
  thread.
- Undo restores every item, every ripple shift and the previous selection. Redo reapplies the
  same item set, even if the scope would resolve differently now.

**Undo:** One compound entry `Batch { label, count, parts: Vec<ChangeSet> }`, flattened into a
single `ChangeSet` (TL-043).

**Model notes:** In `freecut-timeline`: `ApplyScope::{Selection(SelectionSnapshot),
Track(TrackId), AllOfKind(KindFilter), Range(TimeRange)}`, `fn resolve_scope(&Timeline, seq:
SeqId, scope, source) -> Vec<ItemRef>`, and `BatchReport { applied: usize, skipped:
Vec<(ItemRef, SkipReason)> }`. Owners (TR-014, CL-025, FX-006, TX-028's caption scope switch,
UI-010, TL-034) keep their own dialogs and eligibility rules and build their ops on these
helpers. Tests: resolution order per scope, partial skips, an all-skipped run makes no entry,
and undo equals the original.

### TL-054 Operation availability per clip — v1

**What the user sees:** When an edit can't act on a clip, FreeCut says why instead of doing
nothing. A disabled button or menu item has a tooltip with the reason (UI-011), a key press
shows it in the status bar, and a command on a mixed selection acts on the clips it can and
reports the rest (TL-053). Background jobs never block editing.

**Inputs / controls:** None of its own.

**Behavior / output:**
- Every timeline op asks one pure question before planning: may this op act on this clip now?
  The answer is *yes* or an **unavailable reason**. TL defines these reasons (FreeCut wording,
  kept in the string catalog, UI-044):

  | Reason | Typical ops |
  |---|---|
  | On a locked track | every clip edit (TL-009) |
  | Playhead not inside the clip | split, trim to playhead, freeze |
  | Inside a transition | split (TL-023) |
  | No source to slip or reveal | slip, match frame (stills, generated items) |
  | No sound | detach audio, audio settings |
  | Not joinable | join (TL-052), with the specific cause |
  | Would exceed the track or nesting limit | drop, paste, compound create, break apart, snippet insert |
  | Would contain itself | paste or create compound (cycle) |
  | Owner's reason | anything another spec registers (for example TR's reverse on a still, or an analysis tool that refuses compound clips) |

- Which of TL's own ops each clip kind supports (FreeCut default; owners add theirs):

  | Op | Video | Image / freeze still | Audio | Text / sticker / shape | Effect / adjustment | Compound |
  |---|---|---|---|---|---|---|
  | Move, trim, split, delete | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
  | Freeze frame (TL-031) | ✓ | — | — | — | — | ✓ |
  | Detach audio (TL-030) | with sound | — | — | — | — | — |
  | Replace media (TL-035) | ✓ | ✓ | ✓ | OV-010's command | — | — |
  | Slip (TL-022) | ✓ | — | ✓ | — | — | ✓ |
  | Join (TL-052) | ✓ | ✓ | ✓ | ✓ | ✓ | — |
  | Paste attributes target (TL-034) | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ (own bags) |

- **Background jobs never lock a clip** (FreeCut default). Analysis and processing jobs
  (AI-001, PM-073) read media, not the edit model, so a clip with a running job can still be
  moved, trimmed, split or deleted. Their results are in source time, so they still map onto
  the clip after such edits. A job's apply step works on the clip as it is when it runs,
  through ordinary undoable TL ops. A part of the clip outside the processed source range is
  shown by the owner (for example AI-019's partially-enhanced badge with an offer to extend). A
  clip deleted before its job finishes simply receives nothing; the cached result stays for
  undo. The reference app, as observed, locks a clip while a job processes it (no split, trim,
  length change or compound) and asks for a rerun after its length changes. FreeCut needs no
  lock, because no local job rewrites a clip in place.

**Edge cases:**
- A command whose every target is unavailable is disabled; its tooltip shows the first
  target's reason, plus "and 3 more" when several reasons apply.
- Reasons are evaluated at plan time against the current document and never cached across
  edits, so an undo or another edit can make a command available again at once.
- Offline media: TL ops stay available (metadata only, TL-011). Ops that need pixels or samples
  report their owner's "media offline" reason.

**Undo:** n/a (queries; the ops they guard have their own entries).

**Model notes:** `enum Unavailable { LockedTrack, PlayheadOutside, InTransition, NoSource,
NoSound, NotJoinable(JoinRefusal), TrackLimit, DepthLimit, Cycle, Owner(ReasonId) }` and `fn
availability(op: OpKind, seq, clip, ctx) -> Result<(), Unavailable>` in `freecut-timeline`.
Owners register extra checks through the same trait as their ops. A table-driven test checks
every (op, clip kind) pair against the table above.

### TL-055 Clip snippets: save and reuse compound clips — later

**What the user sees:** A compound clip, for example a lower third made of a shape, two text
items and a sound effect, or a short intro, can be saved as a named **snippet** and dropped into
any project later. Snippets appear in a *Snippets* section of the asset panel (placement is
UI-004's) with a thumbnail, name and duration. Dropping one onto the timeline inserts a compound
clip holding a fresh copy of its content. Editing that copy never changes the saved snippet.

**Inputs / controls:**
- *Save as snippet…* in a compound clip's context menu. With a selection that isn't a compound
  clip, the same command packs the selection as TL-040 would, but only into the snippet: the
  timeline stays unchanged (FreeCut default).
- Dialog: name (required, 1–80 characters, trimmed like clip names, TL-048), cover frame
  (default: the middle frame), and *Copy media into the snippet* (default on, so the snippet
  still works on another machine or after the original files move).
- Library actions: rename, delete, duplicate, *Show in folder*, drag to the timeline or the "+"
  button (TL-012 placement rules).
- Preference: snippet folder location (default: inside FreeCut's app-data folder), with
  *Move…* to move existing snippets.

**Behavior / output:**
- A snippet is a small bundle in the snippet folder: the nested sequence in the project format
  (PM-021, versioned, with migrations), a manifest (name, duration, the canvas size and frame
  rate it was made with, cover image, app version, and the media list with PM-038
  fingerprints), and the media files when copying is on.
- Inserting a snippet imports its media into the project's library (PM-030) in a bin named
  after the snippet, reusing items already there with the same fingerprint (PM-038). It then
  inserts a compound clip whose nested sequence is a deep copy with fresh ids, so TL-040's
  one-reference rule holds.
- An inserted snippet breaks apart like any compound clip (TL-040).
- Deleting or renaming a snippet never touches projects that used it, because they hold copies
  (FreeCut default). The reference app, as observed, shows contradictory signs here (deleting a
  saved preset removing it from every project, yet copies that edit independently); FreeCut
  uses copies everywhere.
- Name clash on save: *Replace* or *Keep both* ("Lower third (2)"), as OV-030 does for
  templates.
- No fixed cap on the number of snippets (FreeCut default; the reference app, as observed,
  caps its equivalent library at 99). The list is virtualized.

**Edge cases:**
- Snippet media missing at insert (copying was off and the files moved): the clips come in
  offline and can be relinked (PM-091).
- Snippet saved by a newer FreeCut: inserting it is refused with PM-021's "made with a newer
  FreeCut" message.
- Inserting would exceed nesting depth 8 (TL-040), for example a snippet that holds compound
  clips dropped inside a deeply nested compound: refused with the reason.
- Canvas or frame rate differs from the project: inserted anyway. The nested sequence takes
  the project's settings (TL-040), positions are conformed to the frame grid (TL-001), and a
  toast notes the difference.
- Fonts used by text items are not copied (font licences; TX). A missing font shows TX's
  substitution warning.
- Snippet folder missing or unreadable: the section shows "Snippet folder not found" with
  *Choose folder…*, and nothing is deleted. A move that fails halfway keeps the old folder in
  use and lists the snippets that weren't moved, so no reference breaks.
- Saving fails (disk full): the half-written bundle is removed, so no broken snippet appears.
- Saving and moving run on worker threads; the user can keep editing.

**Undo:** Saving, renaming, deleting and moving snippets are n/a (outside any project document;
per TL-043 an irreversible delete says so first). Inserting a snippet is one `InsertClips` entry
that also holds the created nested sequence and PM's library additions.

**Model notes:** Bundle layout and manifest in `freecut-project` (same serializer and migrations
as projects). The library list and file work run in `freecut-app` worker tasks.
`freecut-timeline` provides `import_sequence(into: &mut Timeline, from: Sequence, media_map) ->
ChangeSet`, shared with TL-056, which re-keys every id. Snippets complement OV-030's templates (a
whole project to start from) and PM-062's personal library (single media files).

### TL-056 Insert another project as a compound clip — later

**What the user sees:** *File ▸ Insert project…*, or dragging a `.freecut` file from the OS onto
the timeline, brings the whole timeline of another FreeCut project into the open one as a
compound clip at the playhead. This is how several projects are combined into one: each
inserted project becomes one compound clip that can be opened, edited or broken apart. The
other project's file is never changed and stays independent.

**Inputs / controls:**
- File picker for one or more `.freecut` project files, or a drop from the OS file manager.
- Placement follows TL-012: on the main track at the nearest cut point (magnet on), otherwise
  at the playhead. Several projects go back to back in the chosen order.
- Progress "Inserting 2 of 5…" with *Cancel* while the projects load on a worker thread.

**Behavior / output:**
- Each source project is read through PM's loader for reading only, without taking its
  single-writer lock (PM-013). Migrations run in memory; the source file isn't rewritten.
- Its root sequence and nested sequences are deep-copied into this project's `Timeline` with
  fresh ids (TL-055's `import_sequence`). The root becomes the nested sequence of one new
  compound clip named after the source project (TL-048).
- Its media references are added to this project's library in a bin named after the source
  project, reusing items with the same fingerprint (PM-038). Missing source files come in
  offline with their last known paths (PM-091).
- Project settings: the nested sequence takes this project's canvas and frame rate (TL-040).
  If the frame rate differs, positions inside the copy are conformed to the frame grid
  (TL-001), and a note lists the differences ("Made at 1080×1920, 30 fps").
- The source's timeline markers become markers of the nested sequence. Its view state, range
  marks, export settings and history are not brought over.
- Success, partial success and failure are reported per project in TL-053's summary style.

**Edge cases:**
- A project made by a newer FreeCut: that project is refused with PM-021's message; the others
  still insert.
- An empty project: refused with "Nothing to insert" (FreeCut text).
- The open project itself chosen: allowed; it inserts a copy of its last saved state, and the
  toast says "Inserted the saved version" (FreeCut default).
- The nesting depth would exceed 8 (TL-040): refused for that project, with the reason.
- Several projects with some failing: the successful ones are inserted in one history entry,
  and the failed ones are listed.
- *Cancel* during loading: nothing is inserted.
- A very large source (thousands of clips): loading runs on the worker; the insert itself is
  one model op.

**Undo:** One `InsertClips` entry including the created nested sequences, with PM's library
additions in the same compound step. Undo removes the compound clips, their sequences and any
newly added library items.

**Model notes:** Loading and migration live in `freecut-project`. Id re-keying and media
remapping are pure functions in `freecut-timeline` (`import_sequence`, shared with TL-055) that
take a `MediaMap` from PM as plain data. No link to the source file is stored.

### TL-057 Linked selection — v1

**What the user sees:** A video clip and the sound detached from it (TL-030), or the extra
audio streams added from it as clips (AU-038), behave as one item while *Select linked clips
together* is on: clicking either selects both, dragging moves both, and deleting removes both.
`Alt`+click picks one member alone, so the sound can still be slid or trimmed by itself. A
linked clip shows TL-030's link badge; while this option is on, a selected clip's partners get
the selection outline too.

**Inputs / controls:**
- Checkable command *Select linked clips together* in the *Follow main track* split button's
  menu (TL-029, UI-011) and in the Timeline menu (UI-012). Key: unbound by default (assignable,
  UI-031). Toggling shows a short toast with the new state (UI-040), like snapping.
- Default **on** (FreeCut default), a per-user preference (UI-020), not a document flag, so it
  has no undo entry and doesn't travel with the project.
- `Alt`+click, `Alt`+marquee-add and double-click (as for groups, TL-039) select a single
  member whatever the setting.

**Behavior / output:**
- **Which links count:** `DetachedAudio` links only (which AU-038's added stream clips share).
  `Speech` links (TX-043) never expand a selection (FreeCut default), because a text clip's
  inspector would otherwise turn into a mixed selection every time; their delete rule stays
  TX-043's.
- **Expansion:** every way of selecting a clip (click, `Mod`+click, `Shift`+click rectangle,
  marquee, select forward/backward, select-all-on-track, *Select clip under playhead*) adds the
  clip's link partners on unlocked tracks, and then their group members (TL-039), repeated
  until nothing new is added. `Mod`+click on a selected linked clip removes the whole linked
  set.
- **Commands on the expanded selection:** move, nudge, move to another track (only the clicked
  member changes track; partners keep theirs and shift in time by the same amount), delete and
  ripple delete, copy, cut, duplicate, enable/disable and split act on every member, as they do
  for any multi-selection, so the members stay in sync.
- **Trim:** dragging an edge of one member also trims the partners whose same edge lies at the
  same time (in sync and aligned); partners whose edge is elsewhere are left alone. This is
  TL-051's multi-clip trim with the partners added, one entry.
- **Off:** TL-030's old rule: selecting a linked clip selects only that clip.

**Edge cases:**
- A partner on a locked track is not selected (TL-015) and doesn't move; the others move and
  both show TL-030's out-of-sync badge. The toast says "1 linked clip is on a locked track and
  stayed" (FreeCut text).
- A partner on a hidden track is selected and moves (hidden clips are selectable, TL-015).
- A move that is refused for any member (collision on a full track, start before 0) is refused
  as a whole, as TL-018 says for multi-clip moves; the red ghost shows every member.
- With the magnet on, moving a main-track video with linked audio reorders the video on the
  main track and shifts the audio by the video's resulting time change, not by the pointer
  delta.
- Restore audio (TL-030) on an expanded selection: the audio members are not targets of their
  own, as TL-030 already says, so nothing changes in its rules.
- Out-of-sync linked clips still expand together; *Resync* is how they line up again.
- Inside an open compound clip, links of the nested sequence expand there; links never cross
  sequences (TL-011).

**Undo:** n/a for the setting (preference). Every command on the expanded selection is one
entry, as for any multi-selection; undo restores the pre-op selection, which includes the
partners.

**Model notes:** Pure helper in `freecut-timeline` (`select` module):
`fn expand_selection(seq: &Sequence, picked: &BTreeSet<ClipId>, rules: Expand) ->
BTreeSet<ClipId>` with `Expand { groups: bool, links: bool }`, skipping clips on locked
tracks; and `fn linked_edges(seq, clip, edge: Edge) -> Vec<ClipId>` for the trim rule. The app
calls `expand_selection` wherever it builds a selection and passes the result to the existing
ops, so no edit op changes. The preference lives in the app's settings beside snapping.

Build notes (model, 2026-10-13):
- The functions are `select::expand_selection(seq, &BTreeSet<ClipId>, Expand)` and
  `select::linked_edges(seq, clip, Edge) -> Vec<ClipId>`. The second returns the partners on
  unlocked tracks whose same edge is at the same time and that are in sync.
- Every link in the model today is a detached-sound link. Speech links (TX-043) must stay out
  of the expansion once they exist.
- The magnet edge case above is the one change to an existing op. In `Move`, a partner of a
  clip that lands on the magnetic main track shifts by that clip's resulting change, not by
  the pointer delta.

## Toolbar round — build slices

The user asked for the reference editor's timeline toolbar functions (add, select/blade tool
menu, undo/redo, split, cut away before/after the playhead, delete, add marker; on the right
voiceover recording, magnetic main track, snapping, a switch that ties other items to main-track
clips (with a menu), a pointer-preview switch with a menu, and zoom). Most of them already have
specs; this round maps each one to its owner, fills the gaps in FreeCut's own words, and
orders the work. Nothing is gated or online: every function is free and local.

| Toolbar function | Owner | Status before this round |
|---|---|---|
| Add (`+` menu: import, add track, title, record) | UI-011, PM-030, PM-032, TL-006, TX-001, AU-027 | add-track button built (R5); menu new |
| Select / Blade / Hand tool menu | TL-024 (tools), UI-011 | specified (blade); Hand and the menu new |
| Undo / Redo | TL-043 | built |
| Split | TL-023 | built (`Mod+B`) |
| Cut away before / after the playhead | TL-026 | specified; toolbar buttons and the request contract new |
| Delete | TL-025 | built |
| Add marker (with menu) | TL-037 | specified; toolbar, drawing, navigation details and contract new |
| Record voiceover | AU-027 | specified; target track, pre-roll, punch range, monitoring, meter and cancel new |
| Magnetic main track | TL-005 | built |
| Snapping | TL-028 | built |
| Tie-items-to-main-track switch with menu | TL-029 *Follow main track* + new TL-057 *linked selection* | TL-029 specified, not built; TL-057 new |
| Pointer-preview switch with menu | TL-014 hover skim | specified; split button and menu new |
| Zoom out / slider / in (/ fit) | TL-041 | built; slider details recorded |

The slices below are tracer bullets: each leaves FreeCut buildable, tested and usable, with
`cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and
`cargo test --workspace` green, every new edit op covered by an apply → undo → redo test, and
nothing decoded, rendered or written to disk on the UI thread. Two slices add project-format
fields (T2 markers, T4 attachments); each takes the next free `CURRENT_FORMAT_VERSION` at merge
time (6 today; other rounds may land first) with a migration test from the previous version.

**T1 — Toolbar layout, tool menu, blade and hand, cut away before/after the playhead.**
- *Crates:* `timeline` (request and availability check), `app` (toolbar, tools, keys).
- *Contracts:* `EditRequest::TrimToPlayhead { seq, targets: TrimTargets, at, cut: Cut,
  ripple_others }`, `OpKind::TrimToPlayhead`, `EditError::NothingToTrim`,
  `Notice::PlayheadTo(Tick)` and `trim_to_playhead_available` (TL-026); app-side
  `TimelineTool::{Select, Blade, Hand}` with `last_other` (TL-024); toolbar actions
  `ToolAction::{CutBefore, CutAfter, SetTool(TimelineTool), Import, ImportFolder, AddTrack(kind),
  AddText}`; UI-011's new group order and fold order.
- *Keys:* `A`, `B`, `H` (tools), `Q`, `W` (cut away), as UI-031. The app's extra plain `S`
  binding for Split is removed, because UI-031 gives `S` to hover skim (T5); the Split tooltip
  names `Mod+B` only.
- *Tests (timeline):* Q and W on a main-track clip with the magnet on (ripple, later clips
  shift, `PlayheadTo` on Q only); on an overlay clip (no ripple; with `ripple_others`, that
  track ripples); playhead on the first frame, at the end and outside → skipped with reasons,
  refused when nothing is left; several main-track clips in one request; a clip under a
  transition region keeps FX's rule; apply → undo → redo restores ids, ranges and selection.
- *Tests (app):* `fold_toolbar` keeps UI-011's order and fold sequence at shrinking widths; the
  blade's hit-test helper returns the snapped, quantized cut time and "nothing" on edges, gaps
  and locked tracks; the tool button toggles Select ↔ last other; the keymap test finds no
  plain-`S` split.
- *Acceptance:* with the blade chosen from the tool menu (or `B`), clicking a clip cuts it at the
  snapped frame and `Shift`+click cuts every unlocked track; the hand drags the view; the two
  new buttons and `Q`/`W` tighten a talking-head clip on the magnetic main track with one undo
  step each; the `+` menu imports media, adds a track of any kind and adds a title.

**T2 — Markers.**
- *Crates:* `timeline` (model, ops, snapping, queries), `project` (format records, migration),
  `app` (ruler and clip drawing, toolbar split button, editor popover, keys, selection).
- *Contracts:* TL-037's build contract (`Marker`, `MarkerId`, `PaletteColor`, `MarkerRef`,
  `AddMarker`, `EditMarker`, `MoveMarker`, `DeleteMarkers`, `next_marker` / `prev_marker`,
  `SnapKind::Marker` targets); the app's `Selection` gains `markers: BTreeSet<MarkerRef>`.
- *Tests:* add with `Auto` owner (timeline vs clip rule), naming "Marker N" per owner, next
  colour; edit, move (clamped on clips, quantized), each delete selector; split hands clip
  markers to the right part; ripple edits leave timeline markers in place; snapping reaches
  marker starts and range ends; navigation with no wrap-around; save → load round trip and the
  migration test; apply → undo → redo for every request.
- *Acceptance:* `M` and the toolbar flag drop markers on the ruler or on the selected clip,
  `Shift+M` / `Ctrl+Shift+M` jump between them, double-click edits name, note, colour and
  duration, dragged clips snap to markers, and the markers survive save and reopen.

**T3 — Linked selection (TL-057).**
- *Crates:* `timeline` (`expand_selection`, `linked_edges`), `app` (selection paths, trim
  gesture, preference, the checkable command in the Timeline menu; the *Follow main track*
  split button that also carries it arrives with T4).
- *Tests:* expansion through links and groups to a fixed point; locked partners skipped;
  `Speech` links ignored; a move of a detached video and its sound keeps `sync_offset` at 0;
  the edge-aligned trim rule trims partners only when aligned; `Alt`+click picks one; with the
  preference off, behavior equals TL-030's old rule.
- *Acceptance:* after *Detach audio*, clicking the video selects its sound too; dragging
  either moves both in sync; `Alt`+click slides the sound alone and the out-of-sync badge
  appears.

**T4 — Follow main track (the tie-to-main-track switch, TL-029).**
- *Crates:* `timeline` (attachments, flags, `follow_owners` inside every owner-moving op),
  `project` (format records), `app` (split button, kinds popover, attachment ticks, hint).
- *Contracts:* TL-029's build contract (`Attachment`, `FollowKinds`, `SetFollowMain`,
  `SetFollowKinds`, `attach_all`, `reattach`, `follow_owners`).
- *Tests:* per op (move/reorder, ripple trim, delete with the "also removed N attached items"
  notice, split re-anchoring, magnet compaction, `TrimToPlayhead`): attached items keep their
  anchor frame; unchecked kinds stay put; locked tracks flag out of sync; collisions go to a
  new track above; toggling on/off moves nothing and undo restores attachments exactly; the
  TL-018 property test with attachments; format round trip and migration.
- *Acceptance:* a title and a sticker over a main-track clip travel with it through reorder,
  trims and deletes; unchecking *Text* in *Item kinds…* leaves titles fixed while stickers
  still follow; `Alt+N` turns following off with one undo step.

**T5 — Hover skim (the pointer-preview switch, TL-014).**
- *Crates:* `app` only (preferences, view state, player request path); `render`/`media` are
  reused through the existing async frame requests.
- *Contracts:* `SkimMode`, `last_on`, `preview_time = skim.unwrap_or(playhead)`; requests
  throttled to the display refresh and stale ones dropped. *Picture and sound* needs AU-016's
  scrub grains: until those exist the radio item and the menu's scrub-sound item are not
  shown (UI-011's rule that buttons appear when their commands exist), and `S` toggles Off ↔
  Picture only.
- *Tests:* the mode state machine (`S`, the toggle, the menu, `last_on`); skim time is `None`
  during playback and drags and outside the lanes; the throttle drops stale requests (pure
  helper with an injected clock).
- *Acceptance:* with skim on, moving the pointer across the timeline shows each frame in the
  player without moving the playhead; leaving the timeline returns to the playhead frame.

**T6 — Voiceover recording (AU-027).** Three sub-slices, in order:
- *T6a capture core (`audio`):* `freecut_audio::record` with input-device listing, a
  `Recorder` driven by a `CaptureSource` trait (cpal input in production, a synthetic sine or
  silence source for tests), the lock-free ring (`rtrb`) to a writer thread, FreeCut's own
  WAV writer with the RF64 switch and the periodic header refresh, atomic level meters and the
  recorder state machine; AU-027's build contract and audio-engine.md §12 give the types.
  Tests run without a microphone.
- *T6b placement (`timeline`, `project`):* `EditRequest::InsertRecordedClip` (one entry with
  the library item, the clip and any new track), the free-audio-track search, the redo-take
  replacement, latency-compensated placement as a pure function, the Recordings folder path
  and crash-recovery scan in `freecut-project`. Tests: apply → undo → redo; placement on the
  first free track, on a new track, past the sequence end; redo take as one entry; recovery
  of a file left with its `.recording` marker.
- *T6c panel (`app`):* the microphone button, the floating recorder panel (device, level,
  meter, channels, target track, countdown, pre-roll, punch range, monitoring, cleanup toggles,
  mute timeline), the growing red clip with its live waveform, the Takes list, permissions,
  errors, `Esc` cancel. Acceptance: a user picks a microphone, sees the meter move, records a
  take after a 3-second countdown while the timeline plays, stops, and finds the take on a free
  audio track at the start position, lined up with the picture; a second take with *Use In/Out
  range* stops by itself at the Out mark.

**Feasibility notes.** All slices fit the current stack: the toolbar is egui 0.36 icon buttons
and popovers already used by R5; markers and attachments are plain model data; hover skim
reuses the player's frame requests; recording uses `cpal` 0.18 (already a workspace
dependency for output) and `rtrb` 0.4 (already present), and needs no FFmpeg. The voiceover
panel's cleanup toggles (AU-018, AU-019, AU-037, AU-020) and voice effect (AU-022) appear only
once those features exist; until then the panel records clean takes.

## Interactions with other areas

- **Projects & media ([project-and-media.md](project-and-media.md)):**
  - PM owns the sequence settings (frame rate, canvas, sample rate) that TL-001 and TL-050
    consume, and the media library and `MediaRef`s.
  - PM owns offline/relink and import/probe, which TL-012 waits on.
  - PM owns autosave, and the cover/thumbnail-frame choice for the project.
  - PM owns renaming media in the library; clip default names follow it (TL-048).
  - The frame-rate change flow calls TL-001's conform op.
  - A folder dropped on the timeline is imported through PM-032 and not placed (TL-012).
  - Snippets (TL-055) and inserted projects (TL-056) bring their media in through PM's import
    (PM-030), reusing items by PM-038's fingerprint and relinking through PM-091. Their bundles
    and the read-only loading of another project use PM-021's format versions and migrations
    and respect PM-013's lock.
  - Removing media from the project stays undoable (PM-058), as TL-043 requires.
- **Transform, speed & keyframes
  ([clip-transform-and-speed.md](clip-transform-and-speed.md)):**
  - TR owns the property bags for transform, crop, masks and keyframes, and the retime mapping
    `timeline → source`.
  - TL uses that mapping for trim limits, split, freeze frame, attachments and match frame.
  - TR-027 owns keyframe behavior on split and trim: keyframes are source-anchored, both split
    parts keep all keys, and no boundary keys are inserted (TL-023, TL-020 defer to it).
  - TR-034 (v1, with TL-034) registers TR's Paste attributes categories, and TR-033's
    "keys past the end are kept" rule is the one TL-034 follows.
  - TR-044 owns which frame a freeze still shows and bakes its settings, which is why TL-031's
    still is independent of its source clip; TL-031's later *Change frozen frame* builds on it.
  - TR-030 owns keyframe navigation, which TL-013 lists among the playhead moves.
  - TR-014's *Apply background to all clips* builds on TL-053's batch rules.
  - Reverse playback and speed curves are TR features. Their duration changes go through TL ops
    so they ripple and undo correctly.
- **Text & captions ([text-and-captions.md](text-and-captions.md)):**
  - Text items live on `Text` tracks. Captions live on dedicated `Caption` tracks (TL-004's
    `TrackKind::Caption`, top band, kept on top under Flexible stacking, exempt from
    auto-removal, TL-006). TX-025 owns the caption payload (`CaptionTrackData`,
    `CaptionSegment`) and caption editing (TX-026, TX-027); TL applies its generic track rules.
  - TX-043 links generated speech clips to text clips and caption segments through TL-011's
    links (`LinkMember::CaptionSegment`). Deleting the text side deletes the speech clip in the
    same op.
  - TX defines the split behavior of text and the text-to-caption conversion (TX-035).
  - TX-028's caption scope switch (all captions on the track / selected captions) uses
    TL-053's scope words. TX's Paste attributes categories decide how text with several formats
    is handled (TL-034).
- **Audio ([audio.md](audio.md)):**
  - AU owns the mixer, track solo, volume and fades, audio effects, audio scrubbing and the
    volume overlay on waveforms.
  - AU owns `MasterAudio` and `TrackAudio`, stored in TL's typed slots `Sequence.audio`
    (TL-003) and `Track.audio` (TL-004) and changed through AU's undoable ops
    (`SetMasterGain`, `SetTrackGain`, `SetTrackPan`).
  - AU-011 (mvp) applies TL-008's mute flag, which is mvp for that reason.
  - **Beat detection is AU-025's:** the onset/tempo/tracker job (run on AI-001's scheduler),
    density and sensitivity controls, the tempo readout and the tap key (latency per AU-026).
    Beats are stored only as TL-037 clip markers (`origin = Beat`, source time, auto/manual
    flag). There is no `ClipAudio.beats` list and no `BeatMarker` type.
  - AU-008's volume keyframes follow TR-027 on split (no boundary keys).
  - AU-001's and AU-035's open request for sub-frame audio edit points is answered by TL-050
    (later). AU-035's waveform sync (later) emits ordinary TL move ops.
  - TL owns embedded/detached audio structure (TL-030) and track mute flags (TL-008). The
    speaker of a `Video` track silences its clips' embedded sound without touching AU-007's
    per-clip mute.
  - TL-029's per-kind follow settings classify audio clips by AU-001's `origin` (music, sound
    effect, voiceover, extracted, other), read as plain data.
  - TL-014's *Picture and sound* skim mode plays its snippets through AU-016's scrub audio
    path; audio while dragging the playhead stays AU-016's own preference.
  - AU-027's voiceover takes are placed by TL's `InsertRecordedClip` op with TL's free-track
    search (TL-006 creates a track when none is free); a take is an ordinary audio clip
    afterwards. *Use In/Out range* reads TL-038's range marks.
  - TL-057's linked selection expands only `DetachedAudio` links, the ones TL-030 and AU-038
    create.
- **Color & filters ([color-and-filters.md](color-and-filters.md)):** Filter and adjustment
  items live on `Adjustment` tracks and apply to layers below them; where those tracks may sit
  is TL-004's strict layering or TL-007's Flexible stacking (the term CL-006 uses). CL owns
  their content and the per-clip color bags. CL-025 registers its copy/paste-color categories
  through TL-034's `PropBag::category()` registry, and its *Apply to all* scopes map onto
  TL-053's scope words (all visual clips = *All of this kind*, this track, selected clips).
- **Effects, transitions & animations
  ([effects-transitions-animations.md](effects-transitions-animations.md)):**
  - TL-004 and TL-007 own layering. FX-001's render order is TL's validated bottom-to-top track
    order (strict layering by default, Flexible stacking per TL-007), and step 2 composites the
    Caption band after all other visual tracks. FX owns per-frame compositing itself.
  - FX owns the transition object (FX-002), defined on TL-004's no-overlap placement: adding a
    transition advances B's source in-point by `D` and ripples −`D`; removing it reverses
    that. FX-008's length-keeping mode keeps stored ranges disjoint as well.
  - FX-010's effect keyframes follow TR-027 on split. FX-017 registers its effect and
    animation categories through TL-034's registry.
  - FX owns effect items on `Effect` tracks and clip in/out animations, and clamps animation
    lengths on short clips at render time (TL-034 pastes them unchanged).
  - FX-006's *Apply to every cut* follows TL-053's skip-and-report rules.
- **Overlays & stickers ([overlays-and-stickers.md](overlays-and-stickers.md)):**
  - Stickers and shapes live on `Sticker` tracks. OV-031's template placeholder slots are
    filled by OV-032 through TL-035 replace.
  - OV-012 and OV-017 looping media: TL-011's `SourceLimits` returns unbounded for media whose
    OV-013 playback mode loops, and TL-020 trims them like stills. OV-013 owns the time mapping.
  - OV-044 registers its style categories through TL-034's registry.
  - OV-007 owns the layer-order commands (bring forward, send to back and so on), built on
    TL's moves and TL-004/TL-007's layering rules; it disables moving one clip of a transition
    pair to another track (TL-007).
  - OV-010 replaces an element's source through TL-035's `plan_replace` with element content;
    TL-035's own *Replace…* command covers media clips (TL-054's table).
  - OV-030's project templates and TL-055's snippets are separate: a template starts a whole
    project, a snippet inserts one compound clip. Both are copied on use.
- **AI tools ([ai-tools.md](ai-tools.md)):**
  - Automatic edits (silence removal, auto-cut, scene detection, auto captions) must emit
    ordinary TL ops (split, delete, insert), so they are undoable as one entry each.
  - AI-005 owns only beat-grid refinement (`BeatGridParams` on the beat-analysis record); it
    rewrites auto beat markers through AU-025's op. AI-006 reads TL-037 beat markers.
  - AI-025's speech clips are linked through TL-011 links (`Speech` kind).
  - Scene markers use TL-037.
  - AI-011's transcript deletions are TL-038's range delete with a clip filter, always
    rippled whatever the magnet state (TL-005), and its *Restore* rejoins pieces with TL-052's
    join.
  - AI-001's jobs never lock clips (TL-054): results are in source time, and apply steps run
    through ordinary TL ops on the clip as it is then. Analysis also runs on clips of hidden or
    muted tracks (TL-008). AI-008's tracking data stays valid for both parts of a split
    (TL-023).
- **Preview & export ([preview-and-export.md](preview-and-export.md)):**
  - EX owns transport (play/pause, J/K/L, loop), the preview frame requests used by scrub and
    hover skim, export range (from TL-038), compound pre-render caching (EX-016; not in TL's
    history, TL-040), and the offline slate. Export always exports the main timeline, even
    from inside a compound clip.
  - EX-002 (mvp) reads the hidden and muted flags (TL-008, mvp) and the disabled flag (TL-036).
  - Preview mute is EX's; its binding is UI-031's.
- **Workspace & shortcuts ([workspace-and-shortcuts.md](workspace-and-shortcuts.md)):**
  - UI-031 is the canonical keymap. It has explicit entries (bound, or unbound by default) for
    Follow main track (TL-029), Ripple all tracks (TL-021) and Add marker with next color
    (TL-037), and its collision test covers them. Paste attributes… (`Ctrl+Alt+V`) is v1
    with TL-034.
  - UI-011's toolbar holds Follow main track (v1, beside the magnet) and Ripple all tracks (v1,
    in the overflow menu); UI-012's Timeline menu lists both. Their state stays undoable TL
    sequence flags (TL-003).
  - UI-012 and UI-035 offer clip *Rename* (TL-048) and track *Track color ▸* (TL-049); UI-008's
    Track tab shows name, color, lock, hide and mute; UI-013 and UI-043 keep labels readable
    on user track colors.
  - UI owns the timeline toolbar layout, the inspector (numeric trim/duration fields), context
    menus, toasts and preferences storage.
  - UI-010's multi-selection editing produces TL-053 batch entries; UI-011's disabled-button
    tooltips show TL-054's reasons; UI-040 shows TL-053's summary toasts and the toggle
    confirmations of TL-005 and TL-028.
  - New commands without a default key yet: *Join clips* (TL-052, later), *Delete range from
    selected clips* (TL-038, proposal `Alt+Delete`) and *Change frozen frame* (TL-031, later).
    UI-031 decides.
  - UI-004 decides where the *Snippets* section (TL-055) sits in the asset panel. UI-020 holds
    the default *Follow main track* kind set (TL-029).

## Out of scope (and why)

- **Multiple independent timelines/sequences per project (tabs):** deferred by decision (see the
  README's *Deferred by decision*). The reference app, as observed, lets a project hold several
  timelines, creates a new one from selected media, shows several at once (stacked, as tabs or
  side by side) and can send generated output to a separate timeline so the current one stays
  untouched. FreeCut keeps one top-level sequence per project because separate sequences add
  project-model complexity with little benefit for the everyday short-form workflow. What users
  get from it is covered locally: compound clips for nesting (TL-040), snippets for reuse
  (TL-055), and *Insert project* (TL-056) for combining projects. If revisited, the "add
  output to a separate timeline without touching the current one" pattern is worth keeping for
  local generators and importers. Revisit later.
- **Linked nested sub-projects (one nested sequence shared by several instances, listed in the
  media panel):** the reference app, as observed, works this way and has to teach users how to
  make an independent copy. FreeCut deep-copies instead (TL-040), so editing one compound clip
  never changes another.
- **Multicam angle editing (angle viewer, switching between synced camera angles):** deferred by
  decision (README). The reference app, as observed, builds a multicam clip from several angle
  clips with a 4- or 9-angle grid, syncs them by audio waveform, timecode, first marker or
  automatically, can match colors across angles, switches the active angle and each angle's
  sound, and can flatten the result to a plain clip (dropping some features) and back. FreeCut
  treats it as niche for this class of editor. If it is ever revisited, everything it needs can
  be local: AU-035's waveform cross-correlation on decoded audio, timecode read from container
  metadata by `freecut-media`, markers as a fallback anchor, and CL-020's color match for the
  angles. Syncing separate audio recordings by waveform is not excluded today: it is AU-035
  (later), which emits TL move ops.
- **Cloud-backed clip jobs and their clip locks (translation, dubbing, speech correction,
  generated avatars):** these need an account and a server, and FreeCut is local only. The
  reference app's rule of locking a clip while such a job runs is replaced by TL-054's rule
  that local jobs never lock clips.
- **Script-to-video and other generator-made projects:** the reference app, as observed, starts
  such projects with item following switched off. FreeCut has no cloud script or project
  generator, so every new sequence uses the user's TL-029 default.
- **Saved presets that wait for downloads or include premium assets:** FreeCut snippets
  (TL-055) reference or copy local files only. A missing file becomes an offline clip with
  relink (PM-091), never a download wait, and nothing in FreeCut is paid.
- **Team-shared snippet or preset libraries:** no team spaces or shared cloud libraries. A
  snippet folder can be copied or put in any sync tool the user chooses (TL-055).
- **Template-authoring behaviors tied to the online template ecosystem** (for example freeze
  segments that regenerate when a template's video is swapped, or groups that contain a
  template and therefore can't be edited): FreeCut's freeze stills are independent (TL-031),
  groups are always editable (TL-039), and local templates are OV's (OV-030 to OV-034).
- **Mobile-companion compatibility flags:** there is no FreeCut mobile app. Projects that use a
  feature an older FreeCut build doesn't know are handled by PM-021's format versions
  (read-only in older builds).
- **Cloud drafts, collaborative or team editing, shared timelines, comments on the timeline:**
  FreeCut is local-only and single-user.
- **Online template authoring and publishing:** the reference app ties templates with swappable
  placeholder clips to its online template ecosystem. FreeCut's local templates and
  placeholder slots are OV's (OV-030 to OV-032) and reuse TL-035's replace.
- **Per-clip locking:** whole-track lock (TL-009) covers the need. Per-clip lock adds state to
  every op for little gain.
- **Track-level effects and track-level transforms:** effects belong on `Effect`/`Adjustment`
  tracks or on clips (FX/CL). Applying an effect to a whole lane is a different model and isn't
  needed for v1. (Track gain and pan are AU-013's, not track effects.)
- **Batch renaming of clips:** single-clip rename (TL-048) covers the everyday need.
- **Arbitrary RGB track colors:** an 8-slot palette (TL-049) keeps colors readable in every
  theme.
- **System clipboard interop with other editors (EDL/XML paste):** later. Interchange formats
  are owned by PM/EX if ever added.
- **Storyboard/script-based editing views:** possible later AI feature. Out of this spec.
- **Mobile-style simplified timeline and touch-first tablet gestures** (long-press to move,
  long-press menus, an explicit multi-select mode, one-finger scroll): FreeCut is desktop-only.
  Trackpad pinch-zoom and two-finger scrolling are covered by TL-041 and UI-038.
- **A per-clip "hide picture" switch separate from Disable:** covered by Disable (TL-036), clip
  mute (AU-007) and track hide (TL-008); see TL-036.
- **A manual beat-stepping or match-cut mode next to beat detection:** the reference app, as
  observed, has a label for such a mode, but its behavior couldn't be confirmed. Manual beats
  are already TL-037 clip markers (tapped or placed by hand, AU-025). Revisit only after
  hands-on use shows what it does.

## Clean-room notes

- No assets are required by this spec beyond UI iconography: magnet, chain/link, lock, eye,
  speaker, scissors, marker flag and similar. These must be our own drawings or come from an
  open-licensed icon set (for example Lucide, ISC license, or Phosphor, MIT license). Icon
  choice is owned by UI. No icons, cursors or artwork from the reference app.
- The 8-slot color palette shared by markers, clip labels and track colors (TL-037, TL-045,
  TL-049), the default clip names (TL-048), the sub-frame audio rules (TL-050), default
  durations, thresholds and limits above are FreeCut's own choices. Public pages we could reach
  describe no specifics of the reference app's clip renaming, track colors or sub-frame audio
  trimming, so those features are designed from general editing practice and open-source
  editors' manuals (for example Kdenlive's configurable clip and marker colors, Shotcut's clip
  rename), not from the reference app. Where they coincide with observed behavior (3 s freeze frame,
  magnet-on by default, split-at-playhead/split-all, select-left/right, group/compound commands),
  that is a recorded observation of behavior, not copied expression.
- Key bindings are functional choices made for familiarity with common editors (several match
  bindings that many editors, including the reference app, use). UI-031 presents them as
  FreeCut's own default keymap, never labeled with another product's name; this spec defers to
  it.
- Feature, toggle and command names in this spec are FreeCut's own wording or generic industry
  terms: Attached items, Follow main track, Flexible stacking, Trim to playhead, Break apart,
  Hover skim, Magnetic main track, Compound clip, Freeze frame, Paste attributes, Track color,
  Sub-frame audio editing, Join clips, Through edit, Snippet, Insert project, Delete range from
  selected clips, Waveform zoom. The caption band, link kinds, follow kinds, batch scopes and
  unavailable reasons are FreeCut model terms. Names that closely echoed the reference app's
  toolbar labels were replaced during review. Toasts and menu strings quoted in this spec are
  FreeCut's own proposed UI text.
- The behaviors marked "the reference app, as observed" come from a paraphrased inventory of
  the reference app's English UI labels, taken from the bundled English localization file of
  the user's local install (desktop 9.5.0; see Provenance in the [README](README.md)). Nothing
  was decompiled, disassembled or traced, no assets were copied, and no label text or internal identifiers appear in this spec. Each observation
  was restated in our own words, and where FreeCut chose differently the entry says so in one
  line. Numeric facts taken from it (for example the 99-item cap on saved presets, the 4- and
  9-angle multicam grids) are used only to explain FreeCut's own choice.
- Other research used public web pages: help-center articles, third-party tutorials, shortcut
  cheat sheets, and manuals of open-source editors. No project/draft files, network traffic or
  APIs of the reference app were inspected.

## Sources

- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- Primal Video — CapCut for PC & Mac: Desktop Tutorial for Beginners (2026) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- TutorialTactic — 50 CapCut Keyboard Shortcuts — https://tutorialtactic.com/blog/capcut-shortcuts/
- FastShortcuts — CapCut keyboard shortcut cheat sheet — https://fastshortcuts.com/shortcuts/capcut/
- Skillademia — CapCut Keyboard Shortcuts — https://www.skillademia.com/shortcuts/capcut-shortcuts/
- TourBox — What Is a Compound Clip & How to Use it in CapCut — https://tourboxtech.com/en/news/capcut-what-is-a-compound-clip.html
- TourBox — How to Freeze a Frame in CapCut PC — https://www.tourboxtech.com/en/news/how-to-freeze-a-frame-in-capcut-pc.html
- TourBox — What Is the Free Layer in CapCut — https://www.tourboxtech.com/en/news/what-is-free-layer-in-capcut.html
- Wondershare Filmora — CapCut timeline guide — https://filmora.wondershare.com/advanced-video-editing/capcut-timeline.html
- Wondershare Filmora — Does CapCut auto-align images to timeline markers — https://filmora.wondershare.com/answers/is-capcut-auto-align-images-timeline-markers.html
- CapCut — How to Use Markers and Labels to Organize Your Video Editing Timeline — https://www.capcut.com/create/timeline-organization-markers-labels
- CapCut Help — Why Is My Text Not Appearing in CapCut — https://www.capcut.com/help/text-not-appearing
- CapCut Help — How to Set Replaceable Material Clips in CapCut — https://www.capcut.com/help/how-to-set-replaceable-material-clips
- CapCut Help — Why Can't I Split the Subtitles in CapCut — https://www.capcut.com/help/split-the-subtiltes
- CapCut Resource — How to Extract Audio from Video — https://www.capcut.com/resource/extract-audio-from-video
- CapCut Resource — How to Make Thumbnails for YouTube (cover frame) — https://www.capcut.com/resource/how-to-make-thumbnails-for-youtube
- Cloudspress — CapCut Desktop icons beginner guide — https://www.cloudspress.com/?p=629105
- VideoWizardTools — How to select multiple clips in CapCut — https://videowizardtools.com/how-to-select-multiple-clips-in-capcut/
- CFRC — CapCut how-to (PDF) — https://cfrc.ca/downloads/capcut-how-to.pdf
- Oboe — Professional Video Editing with CapCut — https://oboe.com/learn/professional-video-editing-with-capcut-a4vma5
- LitCommerce — CapCut Tool Cheat Sheet (2026, PDF) — https://litcommerce.com/wp-content/uploads/2026/03/CapCut-ToolCheat-Sheet.pdf
- Kdenlive Manual — Timeline — https://docs.kdenlive.org/en/user_interface/timeline.html
- Kdenlive Manual — Grouping — https://docs.kdenlive.org/en/cutting_and_assembling/grouping.html
- Shotcut — Keyboard Shortcuts — https://www.shotcut.org/howtos/keyboard-shortcuts/
- Shotcut Forum — About the Timeline — https://forum.shotcut.org/t/about-the-timeline/12952
- Shotcut Forum — Ripple all tracks button — https://forum.shotcut.org/t/ripple-all-tracks-button/10929
- OpenShot User Guide — Timeline — https://www.openshot.org/static/files/user-guide/timeline.html
- Apple — Final Cut Pro User Guide (replace edits) — https://support.apple.com/en-jo/guide/final-cut-pro/ver4e2fcf0/mac
- Kdenlive Manual — Configure colors and markers — https://docs.kdenlive.org/en/getting_started/configure_kdenlive/configuration_colors+markers.html
