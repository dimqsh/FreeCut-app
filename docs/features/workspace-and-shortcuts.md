# Workspace layout, inspector & shortcuts

> Status: draft · Owns: application shell & window layout, workspace/layout state, docking of auxiliary panels, the shared library-panel framework (category tabs, search field, Favorites/Recent categories, tile grid, hover-preview plumbing), inspector framework (which tabs appear for which selection, shared property-control conventions), command registry & keymap (all default keyboard shortcuts; other specs point to UI-031 instead of proposing keys), context-menu / drag-and-drop / tooltip conventions, application settings (Preferences) storage and window, save-status and unsaved-changes presentation, crash reports & safe mode, toasts and the status-line task indicator, hints & the "don't ask again" registry, the system check, diagnostic bundles, the shared save-as-preset dialog, theme tokens, UI localization & accessibility baseline. Project lifecycle (start-screen content, save, autosave, recovery, backups, recent projects), the media cache and proxies are owned by `project-and-media.md`; this spec only places their controls. · Crates: freecut-app (primary), freecut-project, freecut-media, freecut-render, freecut-audio, freecut-timeline

## Summary

This area is the frame around every other FreeCut feature. It defines the
editor window (asset panel, player, inspector, timeline) and how its panels are
sized, collapsed and saved; the asset panel's category tabs; the inspector that
shows the properties of whatever is selected; the timeline toolbar and the top
bar; the frame around the Home screen; the Preferences window, which hosts
this area's settings and those other areas define; a single command system
with a complete default keymap, a shortcut editor, a cheat sheet and a command
palette; context menus, drag-and-drop, tooltips and notifications; the
unsaved-changes prompt, crash reports and safe mode; and where the project
safety nets owned by `project-and-media.md` (autosave, backups, crash recovery)
surface in the UI. It also covers the help around the editor (hints, a
guided tour, release notes, a system check, memory warnings and a local
diagnostic bundle for issue reports), several open projects in their own
windows, and FreeCut's accessibility and localization baseline. Everything is
local: no account, no sign-in, no cloud space, no telemetry.

### Shared conventions used by every feature below

- **Three kinds of state.**
  1. *Project state* — the edit model (`timeline-editing.md`) and project
     settings (`project-and-media.md`). Saved in the project file; changed only
     through the undo stack.
  2. *Workspace state* — per user and per machine: panel sizes, collapsed
     panels, active tabs, collapsed inspector sections, window geometry,
     toolbar toggles, library favorites and recents (OV-038/OV-039, FX-016).
     Stored in the config folder, never in the project, never undoable.
  3. *Settings* — Preferences (`settings.json`) and keymap overrides
     (`keymap.json`). Never undoable; applied immediately.
  Per-project *view* state (playhead position, timeline zoom/scroll, target
  track, media-panel sort and filters) lives in the per-project view file in
  app data, `view/<project-id>.json`, whose format and location are owned by
  `project-and-media.md` (PM-021). It is written in the background, is not
  part of undo and never marks the project dirty.
- **File locations (FreeCut defaults),** resolved with the `directories` crate
  (MIT/Apache-2.0). Rows marked PM repeat `project-and-media.md`'s table for
  convenience; that spec is authoritative for them.

  | What | macOS | Windows | Linux |
  |---|---|---|---|
  | Config (this area): `settings.json`, `keymap.json`, `workspace.json`, `favorites.json`, `recents.json`, `colors.json` (UI-009), `hints.json` (UI-058) | `~/Library/Application Support/FreeCut/` | `%APPDATA%\FreeCut\` | `$XDG_CONFIG_HOME/freecut/` (`~/.config/freecut/`) |
  | App data (PM): recent-projects registry, `recovery/`, `backups/`, `view/`, presets — plus this area's `crash/` reports and `session-<pid>.lock` markers | `~/Library/Application Support/FreeCut/` | `%APPDATA%\FreeCut\` | `$XDG_DATA_HOME/freecut/` (`~/.local/share/freecut/`) |
  | Logs (this area) | `~/Library/Logs/FreeCut/` | `%LOCALAPPDATA%\FreeCut\logs\` | `$XDG_STATE_HOME/freecut/logs/` |
  | Media cache (PM-074): thumbnails, filmstrips, waveform peaks, render cache, proxies | `~/Library/Caches/FreeCut/` | `%LOCALAPPDATA%\FreeCut\Cache\` | `$XDG_CACHE_HOME/freecut/` |
  | Default project folder (PM) | `~/Movies/FreeCut/` | `%USERPROFILE%\Videos\FreeCut\` | `$XDG_VIDEOS_DIR/FreeCut/` (`~/Videos/FreeCut/`) |

- **No I/O on the UI thread.** Reading/writing settings, workspace, recent
  list, autosave, snapshots, backups, cache measurement and device enumeration
  all run on the app's background I/O executor. The UI thread hands over
  immutable snapshots and receives results through channels. All writes are
  atomic: write a temp file in the same directory → flush/fsync → rename.
- **Modifier notation.** Shortcuts are written in Windows/Linux form. On macOS
  `Ctrl` → `⌘ Cmd`, `Alt` → `⌥ Option`, `Shift` → `⇧`. FreeCut defaults never
  use the macOS `⌃ Control` key except where macOS itself sets the convention
  (window full screen `⌃⌘F`, tab cycling `⌃⇥`). `Del` means both the forward
  Delete key and Backspace (on Mac keyboards, the key labelled *delete*).
- **Commands.** Every user action reachable from a button, menu, context menu,
  shortcut or the command palette is a *command* in one registry (UI-030).
  Surfaces dispatch command ids; they never call model code directly. That is
  how every tooltip, menu and the cheat sheet always show the current shortcut
  and the same "why is this disabled" reason.
- **Time at the UI edge.** Times are rationals (`i64` ticks + timebase, see
  `timeline-editing.md`, TL-001); this area only formats and parses them.
  The default display is TL-002's timecode format; other formats are
  UI-019's. Parsed input is snapped to the project frame grid (TL-001).
- **Undo.** Settings and workspace changes are never on the project undo
  stack. Edits made through the inspector or panels are model operations owned
  by their area specs; a continuous gesture (scrub, slider drag) becomes one
  undo entry on release, a typed value becomes one entry on commit (coalescing
  rules in `timeline-editing.md`, TL-043).
- **Placement and edit rules are never restated here.** Where a UI surface
  adds, moves or removes something, the rule (which track, which cut, what
  happens on a locked track) is the owning spec's; this spec only supplies the
  inputs (playhead, selection, pointer target) and shows the planner's result
  or refusal reason.

## Features

### UI-001 Main editor window layout — mvp

**What the user sees:** With a project open, one dark window divided into:

1. **Top bar** (full width, 36 pt, on the window's ground colour): menus,
   project name with save status, the command bar in the centre (UI-034), and
   on the right the assistant toggle, a keyboard-shortcuts button, a
   Preferences gear and the primary **Export** button (UI-012).
2. **Asset panel** (left): a strip of category tabs across its top (the
   Shelf, icon over label) and, below it, the Drawer: a side list of
   sub-categories with item counts, a search field and a grid of items
   (UI-004..UI-007).
3. **Player** (upper centre, the Stage): a header with the output size and
   frame rate, the program viewer, and a transport bar with timecode (owned by
   `preview-and-export.md`), plus a floating context strip for the selection.
4. **Inspector** (upper right): a lens strip over property sections for the
   current selection (UI-008); the same panel also hosts the assistant face.
5. **Timeline** (bottom, full width): sequence tabs and the timeline toolbar
   (UI-011) above track headers and tracks (`timeline-editing.md`).
6. **Status line** (bottom, 22 pt): background-task progress and short hints
   (UI-040).

**Design concept:** how these regions look and behave visually (floating
panels and gutters, the slim top bar, Shelf tabs and Drawer side list, Stage
header and transport, lens strip and property sections, timeline tabs,
toolbar and clips, command bar, the assistant face, tokens, densities and
breakpoints) is defined in [`../design/ui-concept.md`](../design/ui-concept.md)
§3–§5 and §13. Since the concept's revision 2 (2026-10-13) the regions below
the top bar are rounded panels floating on a darker ground, separated by 8 pt
gutters (4 pt in Compact) instead of divider lines
([ui-concept §3.4](../design/ui-concept.md#34-panel-shell-new-in-rev-2)).
This feature keeps the region list, sizes, minimums and behaviour; the concept
wins on presentation.

Reference: CapCut's desktop editor uses the same broad arrangement (library left,
viewer centre, properties right, timeline below), so CapCut-class users find
things where they expect them; FreeCut's lens-based inspector, command bar
and assistant face are deliberately its own. Its observed UI labels also show a toggle that
enlarges one region and a command that shows or hides the media panel;
FreeCut's equivalents are UI-048 and the panel toggles of UI-002 (keys in
UI-031).

**Inputs / controls:** none of its own; sizes are changed per UI-002, whole
arrangements per UI-003.

Default geometry (logical points, FreeCut default):

| Region | Default | Minimum |
|---|---|---|
| Window (first launch) | 85 % of the primary monitor's work area, centred | 1024 × 640 |
| Upper area height | 58 % of the space between top bar and status line | 240 |
| Timeline height | 42 % | 160 |
| Asset panel width (Shelf tabs over the Drawer) | 26 % of window width (≈ 366 pt at 1440 pt) | 240 (collapsed: a 44 pt rail of Shelf icons) |
| Inspector width | 24 % of window width | 280 (collapsed rail: 44) |
| Player width | the remainder | 320 |

**Behavior / output:**
- Region positions are fixed (left / centre / right / bottom); only sizes
  change. Free docking is UI-054 (later).
- Exactly one region has keyboard focus, shown by a 2 pt accent line along
  the straight part of its panel's top edge. Clicking anywhere in a region focuses it. Focus decides which
  scoped shortcuts apply (UI-030).
- Regions render independently. Thumbnails, waveforms, decoded frames and
  library previews stream in from background workers; until they arrive the UI
  shows neutral placeholders. Nothing blocks the frame loop.
- Window title: `<project name> — FreeCut`. When there are unsaved changes,
  Windows/Linux prefix a `•`; macOS uses the native "document edited" dot.
- Below 1280 pt window width (the concept's *Compact* breakpoint) the Shelf
  tabs and the lens strip switch to icon-only with tooltips, gutters shrink to
  4 pt, and a collapsed asset panel opens as a peeking overlay unless pinned; below 1024 pt (*Tight*) the inspector becomes a
  rail whose lenses open as an overlay sheet. At 1680 pt and wider the
  assistant may open as its own column. Breakpoints:
  [ui-concept §5.1](../design/ui-concept.md#51-breakpoints).
- Auxiliary panels that other areas define (scopes, mixer, captions,
  transcript, jobs, history) open in fixed slots described in UI-056 (v1);
  the mvp has none.

**Edge cases:**
- Portrait (9:16) project: the player region keeps its size and the frame is
  letterboxed inside it (`preview-and-export.md`); no automatic layout change
  (the opt-in "Vertical video" layout is UI-003).
- Small laptop screen (e.g. 1366 × 768 at 125 % scaling ≈ 1093 × 614 pt): the
  OS minimum window size applies; once the player would drop below 320 pt
  wide, the inspector and then the asset panel collapse to rails.
- Ultra-wide screens: panels keep their fractional widths, the player region
  grows and the frame stays centred.
- No project open: the Home screen (UI-016) is shown instead of the editor.
- Window restored onto a monitor that no longer exists: see UI-002.

**Undo:** n/a (view state).

**Model notes:** `freecut-app::shell::{Shell, RegionId}` with stable ids
`assets`, `player`, `inspector`, `timeline`. Built from egui 0.36's
`Panel::top` / `bottom` / `left` / `right` (which replaced `TopBottomPanel` and
`SidePanel`) and `CentralPanel` using stored sizes; `Panel::show_switched`
covers the rail ↔ expanded switch (egui mapping: ui-concept §16). No other
crate involved.

---

### UI-002 Panel resizing, collapsing & window state — mvp

**What the user sees:** Thin dividers between the asset panel, player and
inspector, and between the upper area and the timeline. Hovering a divider
shows a resize cursor and tints it with the accent colour. Side panels have a
collapse chevron in their header; a collapsed panel becomes a 44 pt rail of
tab icons.

**Inputs / controls:**
- Drag a divider to resize (hit area 6 pt, visible line 1 pt, 3 pt on hover).
- Double-click a divider: reset that split to its default fraction.
- Click the chevron (or the rail) to collapse / expand. View menu: *Show asset
  panel*, *Show inspector* toggles.
- Drag a side panel narrower than 60 % of its minimum and release: it snaps to
  the collapsed rail. Drag a rail outward: it expands.

**Behavior / output:**
- Live resize at display rate; the player rescales its frame continuously.
- Sizes are stored as fractions of the available width/height, so layouts
  survive window resizes and monitor changes.
- Workspace state (splits, collapsed panels, window size, position,
  maximized/full-screen flag, monitor name) is written to `workspace.json`
  1 s after the last change (background, atomic) and restored at launch.
- Resizing the timeline region never changes timeline zoom; the track area
  scrolls instead.
- A collapsed inspector stays collapsed when the selection changes; its rail
  shows a dot when the selection has properties. Double-clicking a clip in the
  timeline expands the inspector (FreeCut default).

**Edge cases:**
- Window too small for all minimums: collapse the inspector, then the asset
  panel, then shrink the timeline to 160 pt; the player keeps ≥ 320 pt; the OS
  minimum window size stops further shrinking.
- Saved position puts less than 64 × 64 pt of the title bar on any monitor
  (monitor unplugged, resolution changed): re-centre on the primary monitor,
  size clamped to its work area.
- Moving between monitors with different DPI: sizes are logical points and
  fractions, so nothing jumps.
- `workspace.json` unreadable or from an unknown future version: ignore it, use
  defaults, log a warning, overwrite on the next change. No dialog.
- Divider dragged during playback: playback continues.

**Undo:** n/a (view state).

**Model notes:** `freecut-app::workspace::{WorkspaceState, SplitId,
WindowGeometry}` (serde). Floats are fine here — they are screen geometry, not
media time.

**Implementation note (2026-10-13):** `workspace.json` exists, next to
`settings.json`, with what the inspector remembers so far (UI-008): under
`inspector`, the Detail switch, the last lens per selection kind, and the
collapse and *More* state of each (selection kind, card) that differs from the
card's first state. `version` is 1. It is read on the job pool at startup
(a state the user changed before the file arrived wins), written 1 s after the
last change on a writer thread, atomically, and flushed when the window
closes; keys it doesn't know are kept. Splits and window geometry join it
when this feature is built.

*FreeCut decision (review fixes, 2026-10-13):* every split stops where the
regions beyond it reach their minimums (asset panel 240, inspector 280, Stage
320 wide; timeline 160, upper area 240 tall; the mixer 200), measured as cards
plus their gutters. Dragging a gutter further does nothing, and a window
too small for all the minimums keeps each panel at its own minimum. A panel's
content is never asked for a negative size.

---

### UI-003 Layout presets & reset layout — v1

**What the user sees:** View ▸ Layout lists built-in layouts, the user's saved
layouts, *Save layout as…*, *Manage layouts…* and *Reset current layout*.

Built-in layouts (FreeCut defaults):

| Layout | Upper / timeline | Asset / inspector width | Notes |
|---|---|---|---|
| Standard | 58 / 42 | 26 % / 24 % | UI-001 geometry |
| Big timeline | 40 / 60 | 22 % / 22 % | long-form cutting |
| Big preview | 70 / 30 | 26 % / collapsed | review, framing |
| Vertical video | 66 / 34 | 30 % / 28 % | tall, narrow player for 9:16 |
| Inspector focus | 58 / 42 | 22 % / 34 % | color and keyframe work |
| Library focus | 58 / 42 | 34 % / 22 % | browsing media, effects and elements |

Reference: the observed UI labels show four built-in arrangements (balanced,
a larger media panel, a larger inspector, and one for portrait video), a
reset for the current arrangement, a global "restore default layout" in
preferences, and three fixed slots for the user's own layouts. Some labels
hint at player- and timeline-focused variants as well. FreeCut covers all of
these with the six layouts above. It keeps named user layouts (up to 20)
instead of three numbered slots.

**Inputs / controls:** pick a layout; *Save layout as…* (name 1–40 characters,
unique, up to 20 user layouts); rename and delete user layouts (built-ins
cannot be deleted); *Reset current layout*. Preference *Use the Vertical video
layout for portrait projects* (default off). Preferences ▸ Appearance ▸
*Restore default layout* (button).

**Behavior / output:** A layout is a set of split fractions plus collapsed
flags. Applying it is instant (a 150 ms ease if reduced motion is off).
Resizing after applying shows the name as "Standard (modified)"; *Reset*
returns to the stored definition. The last used layout is remembered.
- *Restore default layout* applies Standard, clears the "(modified)" state of
  every built-in layout and re-expands collapsed panels. User layouts are
  kept; deleting them is *Manage layouts…*'s job.
- **Portrait hint:** the first time a portrait project (canvas taller than
  wide) is opened while the preference above is off and the current layout
  is not Vertical video, a one-time hint (UI-058) offers *Switch to Vertical
  video*, *Always for portrait projects* (turns the preference on) and *Not
  now*. Any choice dismisses the hint for good; *Reset hints* (UI-058) brings
  it back.

**Edge cases:**
- A user layout saved on a bigger screen is clamped to minimums (UI-002).
- Deleting the active user layout switches to Standard.
- An imported settings file (UI-050) with layouts that mention unknown regions
  from a newer version: unknown keys are ignored.
- Duplicate or empty names are rejected inline.
- A 21st user layout → *Save layout as…* is disabled with the reason "Up to
  20 layouts. Delete one in Manage layouts…".
- A layout applied while a panel is maximized or distraction-free mode is on
  (UI-048) → that mode ends first, then the layout applies.
- *Restore default layout* while detached panels exist (UI-054, later) → they
  dock back first.

**Undo:** n/a (view state).

**Model notes:** `LayoutPreset { id, name, splits, collapsed }` in
`workspace.json`; built-ins are code constants in `freecut-app`.

---

### UI-004 Asset panel category tabs — mvp

**What the user sees:** A strip of category tabs (icon over a one-word label)
across the top of the asset panel, grouped by intent — the *Shelf* of
[ui-concept §8](../design/ui-concept.md#8-shelf-and-drawer-the-asset-browser).
Each tab opens its content in the *Drawer* below it: a side list of
sub-categories with item counts (Favorites and Recent are entries in it; it
becomes a chip row when the panel is narrower than 360 pt), a toolbar with
the search field (plus Import and sort where they apply) and the item grid.
The "tabs" below are those Shelf tabs.

| Tab | Content | Owner spec | Priority |
|---|---|---|---|
| Media | project media (PM-050), bundled CC0 sample clips | `project-and-media.md` | mvp |
| Audio | imported audio, bundled music & sound effects (own / CC0; AU-030, AU-031), voiceover entry | `audio.md` | v1 (mvp shows audio in Media) |
| Text | *Add text* tile (TX-001), text style presets (v1) | `text-and-captions.md` | mvp |
| Elements | sub-tabs Stickers, Emoji, Shapes, Frames from our own / open-licensed packs and user images (OV-036) | `overlays-and-stickers.md` | v1 |
| Effects | video effects (FX-015) | `effects-transitions-animations.md` | v1 |
| Transitions | transitions (FX-004) | `effects-transitions-animations.md` | mvp (cross-dissolve only, FX-003) |
| Filters | our own looks (CL-003) | `color-and-filters.md` | v1 |
| Adjust | blank adjustment clip, saved looks, LUTs (bundled and imported, with Import) (CL-044) | `color-and-filters.md` | v1 |
| Captions | the Captions drawer (TX-057): captions from speech and from songs (local speech-to-text), typing and pasting, subtitle import/export, caption looks, the caption list | `text-and-captions.md`, `ai-tools.md` | v1 |
| Templates | the user's locally saved templates (OV-032, OV-033) | `overlays-and-stickers.md` | v1 |
| Generate | cloud generation with the user's own account: Image, Video and Runs pages (CG-006 to CG-009); shown only while cloud generation is on and connected | `cloud-generation.md` | v1 (opt-in, proposed) |

Each owning spec defines its tab's content, tile preview and "add" action; the
shared framework (search field, Favorites and Recent categories, tile grid,
hover-preview plumbing) is UI-006 and UI-007.

**Inputs / controls:** click a tab; with the asset panel focused,
`Ctrl+Tab` / `Ctrl+Shift+Tab` (macOS `⌃⇥` / `⌃⇧⇥`) cycle tabs; on the Shelf,
`Left` / `Right` move between tabs and `Down` or `Enter` moves into the
Drawer's side list; in the side list `Up` / `Down` move and `Right` enters the
grid. When the asset panel is collapsed to its rail, choosing a tab opens the
panel as an overlay over the player that closes after an add (peeking, the
default in the concept's Compact breakpoint); its pin button keeps it open.

**Behavior / output:**
- The last active tab and each tab's last sub-category are remembered
  (workspace state).
- Tabs with no content in the current build are hidden, not greyed out (the
  mvp shows Media, Text, Transitions).
- Compact density: tabs show icons only with tooltips; tabs that don't fit
  keep a minimum width and the strip scrolls sideways, with end buttons
  (ui-concept §8.1).
- The pinned state, the side list's width, and each tab's selected
  sub-category and folded groups are workspace state.
- Tab content is listed off the UI thread on first open. Bundled library
  indexes are embedded at build time, so they appear instantly.

**Edge cases:**
- Remembered tab hidden in this build → fall back to Media.
- Empty sub-category → empty state with the relevant action ("Import media…",
  "Import LUT…").
- Switching tabs is blocked while a drag that started in the panel is in
  progress.

**Undo:** n/a.

**Model notes:** `freecut-app::assets::{AssetTab, AssetProvider}`. Each owning
area registers a provider (list items, thumbnail request, "add at playhead"
op builder, drag payload). Items are identified by `AssetRef` (project media
id, bundled asset id, or user file path + content hash).

FreeCut default (mvp build): the tab row holds the tabs with content so
far, **Media**, **Audio** and (since 2026-10-12) **Text**, each as our own
glyph plus label ("🎞 Media", "♪ Audio", "🗛 Text"; Text holds TX-001's
Add text tile, drawn by `ui::text`); below 170 px of panel width only the glyphs show, with the label
as a tooltip. The active tab is remembered for the app session, across
opened projects; the Audio tab's chip, search and highlight start fresh with
each project. Tab buttons are disabled while an item dragged from the panel
is in flight. `Ctrl+Tab` cycling, the `»` overflow and the
`AssetProvider` registry are not built yet; the tabs are drawn directly
(`ui::assets`). Items dragged out of the panel carry `ui::assets::AssetDrag`
(`Media(id)` or `Text`), which the timeline accepts as a drop. This
row with emoji glyphs is the interim build; it moves to the Shelf tab strip,
the Drawer side list and the icon font in the concept's comfort reskin slice
R2 ([ui-concept §17.1](../design/ui-concept.md#171-comfort-reskin-slices-rev-2)).

---

### UI-005 Adding assets from the panel — mvp

**What the user sees:** Items are cards with a thumbnail, a name (two lines,
ellipsis), and badges: duration for media/audio, a dot when the media is used
in the timeline, a warning badge when it is offline, a spinner while it is
still being analysed. Hovering a card shows a round **+** button in its
corner.

**Inputs / controls:**
- **Click** a card: select it. From v1, clicking a media card also opens it in
  the player's media preview mode (EX-015, PM-060); a library card (text,
  transition, effect…) plays its tile preview (UI-007).
- **Double-click** a media card (v1): media preview mode and start playback.
- **+ button** or **Enter** (panel focused): the owning spec's default add —
  for media, TL-012's *Add to main track*. **Shift+Enter**: TL-012's *Add as
  overlay at playhead*.
- **Drag** onto the timeline, player or a clip (UI-037).
- Multi-select with `Ctrl`+click / `Shift`+click / `Ctrl+A` (PM-050).
- Arrow keys move the selection in the grid; `F2` renames (Media only,
  PM-057, v1); `Del` removes from the project (Media only, PM-058, which owns
  the confirmation for used items).

**Behavior / output:** The add is planned by the owning spec, never by this
area: media by TL-012 (main-track insert at the cut nearest the playhead,
audio-only media to an audio track, several items back to back in library
order), text by TX-001, elements by OV-036, transitions by FX-002/FX-009 (cut
chosen from the playhead's position inside the clip), effects by FX-010/FX-011,
filters by CL. This area supplies the playhead time, the selection and a track
hint, then shows the result: the new clip becomes selected, the inspector
shows it, and the timeline scrolls to reveal it.

**Edge cases:**
- The owner's planner refuses (for example the track limit is reached, or the
  item has no valid target) → nothing changes, no undo entry, and a toast
  shows the planner's reason (TL-043's validation-first rule).
- Offline media → allowed; the clip is created offline (PM-090).
- Media still being probed → the add is accepted and the clip appears once the
  probe finishes; a failed probe cancels it with a toast (TL-012).
- Unreadable media → already refused at import (PM-037); there is nothing to
  add.
- Stills use the project's still-image duration (PM-010).
- **+** pressed twice quickly → two adds, two undo entries (no coalescing for
  adds).

**Undo:** one entry per add, owned by the planner (`InsertClips` for media,
TL-012); undo removes the clips and any track the add created.

**Model notes:** The app builds a placement request for TL-012's pure planner
`plan_insert(seq, request)`, which yields the change set; all placement rules
live in `freecut-timeline`. No placement logic in `freecut-app`.

---

### UI-006 Library panel framework: search, favorites, recents & view options — v1

**What the user sees:** Every tab of the asset panel has the same frame: a
search field at the top, category chips with *Favorites* and *Recent* pinned
first in library tabs (OV-036, FX-016), a star on each card, and a
thumbnail-size control. The Media tab's own search, filters, sort, list view
and favorites are defined in PM-050…PM-056; this feature supplies the shared
widgets they use.

**Inputs / controls:**
- Search (`Ctrl+F` focuses the active tab's field): case- and
  accent-insensitive; in library tabs it matches names, tags, and both the
  localized and English names of bundled items. Media search rules: PM-054.
  A query holds at most 100 characters (FreeCut default); further typing is
  ignored and the field shows "100 / 100". Reference: the observed labels
  disagree here (one reading says 100 characters, a key name suggests 30);
  FreeCut takes the larger value.
- Recent searches: focusing an empty search field in a library tab lists
  that tab's last 10 queries (FreeCut default), newest first; `Up`/`Down`
  pick one and `Enter` runs it. A query is recorded when the user presses
  `Enter` or adds or applies an item from its results, not on every
  keystroke. *Clear search history* at the bottom of the list asks for
  confirmation, because it can't be undone. The Media tab keeps or skips
  history as PM-054 decides; the widget is the same.
- Category overview: a tab's *All* view shows each category as one row of
  cards with *Show all (N)* at its end; clicking it opens the category, and
  *Back* returns to the overview at the same scroll position.
- Thumbnail size: library tabs offer three steps (96 / 144 / 200 pt card
  width, FreeCut default); the Media tab uses PM-050's slider. `Ctrl`+wheel
  over the grid changes it too.
- Star or the `F` key toggles a favorite. In the Media tab this sets PM-056's
  per-project flag; in library tabs it is a per-user favorite.
- *Clear recent* in a Recent category's context menu (OV-039).

**Behavior / output:** Search is incremental (debounced 100 ms, FreeCut
default, matching PM-054) and runs off the UI thread above 2,000 items;
matched text is highlighted. No results shows an empty state with a *Clear
search* button and, when other library tabs do match, links such as "3
matches in Effects" that switch tab and keep the query. Reference: the
observed app fills its no-results state with popular items from an online
feed; FreeCut points to local matches instead. Library favorites, recents and
recent searches are per user, stored in the config folder (`favorites.json`,
`recents.json`), keyed by bundled item id or by content hash for user files
(LUTs, fonts, stickers), and survive app updates. The number of recents kept
per tab is the owner's (OV-039: 30). Searching by what a clip shows or
sounds like is not part of this framework; spoken-word search is AI-027.

**Edge cases:**
- A favorite or recent whose item no longer exists (pack removed, user file
  deleted) → shown as the owning tab decides (OV-038 lists them as
  unavailable with *Clean up*; FX-016 drops them silently).
- Favorites file damaged → start empty, keep the damaged file as a backup,
  log it (as OV-038).
- Two FreeCut windows starring at once → merged per item id, last write wins
  per item.
- Very long or right-to-left names → ellipsis; full name in the tooltip.
- `Ctrl+F` while typing in another field moves focus to the search box.
- Pasting a query longer than 100 characters → cut to the first 100, with
  the counter shown in the warning colour for 2 s.
- A recent search that no longer matches anything → still listed; running it
  shows the normal no-results state.
- A category with fewer cards than fit in one row → no *Show all* link.
- UI-050's *Library favorites and recents* reset group also clears recent
  searches.

**Undo:** n/a for library favorites and recents (user state). Media favorites
are PM-056's undoable `SetFavorite`.

**Model notes:** `freecut-app::library::{LibraryProvider, search}` with a small
fuzzy matcher (e.g. `nucleo-matcher`, MPL-2.0); providers are registered by
the owning areas. Favorites/recents I/O on the background executor.

---

### UI-007 Hover previews & try-before-apply — v1

**What the user sees:** Moving the pointer across a media card scrubs through
it (PM-060). Hovering an effect, transition, filter, animation or element card
plays a short loop on FreeCut's own CC0 sample footage. With a clip selected,
hovering a library card shows the item applied to that clip in the player,
without committing anything. *What* each preview shows and when it applies is
owned by FX-016 (effects, transitions, animations), the filter tab in
`color-and-filters.md` and OV-036 (elements); this feature is the shared hover
plumbing they rely on.

**Inputs / controls:** hover. Preferences: *Animated previews* (default on),
*Preview library items on the selected clip* (default on), *Audio while
hover-scrubbing media* (default off, PM-060; shown in UI-029).

**Behavior / output:**
- Hover starts a preview after 150 ms (FreeCut default) so sweeping the
  pointer across the grid doesn't start dozens of renders; leaving the card
  cancels its request within 100 ms.
- Media hover-scrub uses PM-060's cached filmstrip frames only; it never
  decodes on hover.
- Card loops are rendered locally by `freecut-render` at 160 × 90 on first
  view and stored in the media cache (PM-074).
- Try-before-apply hands `freecut-render` a temporary override that is not
  part of the model; behaviour during playback follows the owner (FX-016 keeps
  playing with the candidate applied).

**Edge cases:**
- A preview that can't render a frame within 100 ms shows a spinner in the
  player corner instead of stalling the UI.
- No clip selected → card loop only.
- Reduced motion on, or *Animated previews* off → no autoplay loops; a still
  poster frame is shown.
- Offline media cards → no scrub; the offline slate (EX-005).
- The selected clip is deleted or undone away while hovering → the preview
  override is dropped on the next frame.

**Undo:** n/a — previews never touch the model.

**Model notes:** `freecut-render` accepts a `PreviewOverride` alongside a frame
request; `freecut-app` owns hover state, the start delay and cancellation.

---

### UI-008 Inspector panel with context-dependent tabs — mvp

**What the user sees:** The right panel follows the selection. Its header
shows the item's type icon, name and a breadcrumb (sequence › track › time
range); below it, one row of tabs drawn as a segmented *lens strip*; below
that, a scrolling stack of *property cards*, drawn as full-width sections
separated by hairlines (the "sections" below). Every card uses
one grammar — header with summary chip, optional on/off switch and `...` menu;
rows of label · control · gutter, the gutter holding reset and keyframe
controls; advanced rows behind *More* — and an inspector-wide Detail switch
(Essentials / Everything) sets how much is shown. Presentation, wireframes and
the per-kind lens sets: [ui-concept §6–§7](../design/ui-concept.md#6-the-inspector).

| Selection | Tabs [sections] | Owner |
|---|---|---|
| Nothing | **Project** [name, canvas size & aspect, frame rate, background, duration (read-only), file location, *Project settings…*] | PM |
| Video clip | **Video** [Transform, Opacity & blend, Crop, Canvas fill, Mask, Chroma key, Stabilize] · **Audio** (if it has linked audio) · **Speed** · **Motion** · **Color** [pages Basic, HSL, Curves, Wheels, Masks; CL-043] · **Effects** | TR, AU, FX, CL |
| Image clip | **Video** · **Motion** · **Color** · **Effects** | TR, FX, CL |
| Audio clip | **Audio** [Volume, Fades, Pan, Normalize, Voice & effects] · **Speed** | AU, TR |
| Text / title | **Content** [Text, Paragraph, Speech (later), Convert] · **Style** [Type, Fill, Spacing; optional layers Outline, Shadow, Glow] · **Box & bubble** [Placement (transform, opacity, blend), optional layers Background, Bubble, Curve] · **Motion** [In / Loop / Out, Keyframes, Follow] · **Presets** [Looks, My looks, Templates, Default for new text] (cards per TX-054) | TX, FX, TR, OV |
| Caption item | **Captions** | TX |
| Sticker / shape | **Sticker** or **Shape** · **Motion** · **Video** [Transform, Opacity] | OV, FX, TR |
| Effect layer | **Effect** [parameters] | FX |
| Filter / adjustment layer | **Color** | CL |
| Transition | **Transition** [type, duration, parameters] | FX |
| Keyframe(s) | the clip's tabs plus a pinned **Keyframe** section [time, easing] | TR |
| Marker | **Marker** [name, color, note, duration] | TL |
| Track header | **Track** [name & color (TL-010), lock (TL-009), hide & mute (TL-008)] | TL |
| Several items | see UI-010 | — |

**Motion** (formerly *Animation*) is one tab for every animatable kind: the
clip's animation slots (FX-018, FX-019, TX-022) with an In / Loop / Out
timing strip and preset grid, then a *Keyframes* card listing the clip's
animated properties (TR-028..TR-030). The concept regroups some tabs further
for presentation (for example Video as *Picture* + *Mask & key*,
[ui-concept §6.3](../design/ui-concept.md#63-lenses-by-selection-kind)); the
property groups in this table stay authoritative.

The same column has a second face, the assistant (AS-001,
[ui-concept §11](../design/ui-concept.md#11-assistant-its-place-in-the-ui)),
switched at the column top; the assistant's behaviour belongs to
[assistant.md](assistant.md).

The mvp ships: Project (summary plus *Project settings…*, PM-011), Video
[Transform: position, scale, rotation, fit and flip (TR-001…TR-003)], Audio
[Volume, Fades (AU-005, AU-006)], Text [Content; Style: Font, Size, Color,
bold and italic (TX-002..TX-004)], Transition [Duration (FX-002)]. Everything else in the table arrives with its
owning feature. Track hide and mute (TL-008) are mvp, but in the mvp they are
toggled on the track header itself (TL-004) and from its context menu
(UI-035); the inspector's Track tab arrives with TL-010 (v1).

**Clip info section (v1):** every clip's first tab ends with a read-only
*Clip info* section, collapsed by default: clip name and kind, track,
timeline start, end and duration (UI-019's time format), source media name
and path with *Reveal* (PM-059), source in and out, a speed summary (TR), and
the clip id and media id. *Copy info* puts the block on the clipboard as
plain text for bug reports (UI-063). Nothing in it can be edited here.

Reference: the observed inspector tabs are basic edit, properties, advanced
settings, animation & effects, adjustment, canvas, audio and basic info
(passes disagree on whether "basic edit" and "basic info" are separate), and
selecting a clip can show its internal clip and source ids. FreeCut maps
them onto the table above: the canvas background is the Video tab's *Canvas
fill* (TR-011) and the project canvas is the Project tab; advanced settings
are collapsed sections inside each tab; the info view is the *Clip info*
section rather than a tab of its own.

**Inputs / controls:** click a tab; with the inspector focused,
`Ctrl+Tab` / `Ctrl+Shift+Tab` (macOS `⌃⇥` / `⌃⇧⇥`) cycle tabs; click a section
header to collapse it; ↺ per property and *Reset section* per section.

**Behavior / output:**
- **Tab memory:** the last active tab is remembered per selection kind. Picking
  another video clip while on Speed keeps Speed. If the remembered tab does not
  exist for the new selection (Speed for an image), the first tab is shown.
- Collapsed sections are remembered per (selection kind, section) in workspace
  state; scroll position is kept while the same item stays selected.
- Keyframed values follow the playhead, refreshed at UI frame rate (not per
  decoded frame).
- If the selection changes while a field holds typed, uncommitted text, the
  field is committed first (as if Enter was pressed). `Esc` reverts instead.
- Below 320 pt width, labels stack above controls.
- Large effect stacks are virtualized (only visible rows are built).

**Edge cases:**
- Clip on a locked track: everything read-only, banner "Track is locked" with
  an *Unlock* button.
- Clip with offline media: banner "Media offline" with *Relink…* (PM);
  properties remain editable — they are model data.
- Undo/redo removes the selected item → inspector falls back to Project.
- Selection changes during playback → allowed; playback continues.
- A clip inside a group → the clip's own properties, with a "Grouped" chip.
- *Clip info* for a clip with offline media → the last known path is shown
  with an "Offline" tag; *Reveal* is replaced by *Relink…* (PM-091).
- *Clip info* for a generated clip (text, element, color clip) → no source
  rows; ids only.

**Undo:** the inspector itself is n/a; each change is its owning area's
operation, coalesced per gesture. *Clip info* is read-only (n/a).

**Model notes:** `freecut-app::inspector::{InspectorKind, TabId, SectionId}`,
`fn tabs_for(&Selection, &Timeline) -> Vec<TabId>`. Each area provides a
section renderer that reads model state and emits edit ops. *Selection* is
defined in `timeline-editing.md`.

**Implementation note (2026-10-13):** "committed first" is how a selection
change meets a field with typed text: the inspector keeps showing the clips the
field belongs to until the field lets go and its value is committed to them,
then shows the new selection. When another gesture is open (a canvas drag over
another clip started the change and keeps the field's focus), the field waits
for it to end. A field's typing state belongs to the clips shown, so typed text
never reaches another clip; collapse state, lens memory and slider drags stay
per (kind, card, row).

---

### UI-009 Inspector control conventions — mvp

**What the user sees:** One consistent family of controls across all
inspector tabs: numeric field with label, slider + field, X/Y pair with a link
toggle (uniform scale), toggle, dropdown, segmented buttons, color swatch.
Animatable rows have a keyframe diamond on the right; a ↺ reset arrow appears
only when the value differs from its default.

**Inputs / controls:**

| Gesture | Effect |
|---|---|
| Click field | type a value; `Enter` commits, `Esc` cancels, `Tab` / `Shift+Tab` moves to next / previous field |
| Drag left/right on the label (or an unfocused field) | scrub: 1 step per 2 pt; `Shift` ×10, `Alt` ×0.1 |
| `Up` / `Down` in a focused field | ±1 step; `Shift` ±10 steps; `Alt` ±0.1 step |
| Mouse wheel | changes a field only while it is focused (never while just scrolling the panel) |
| ↺ or `Ctrl`+click on a slider | reset to default |
| Units in typed text (v1) | `90deg`, `50%`, `2s`, `12f` (frames), `-6dB` |
| Arithmetic in typed text (v1) | `1920/2`, `+10`, `*2` |

- Units shown: px, %, °, s or timecode, dB, Hz.
- Duration fields show seconds with one decimal ("0.5 s") or timecode per the
  time-display setting; input is parsed to rational ticks and rounded to the
  nearest frame (half rounds up).
- Color field: swatch → popover with saturation/value square, hue strip,
  alpha, hex input, eyedropper that samples the player (v1), and swatch rows:
  - *Palette*: FreeCut's own 12 colours (the same set as TR-011), fixed;
  - *Recent*: the 12 most recent colors (per user);
  - *Saved* (v1): the user's own colors, up to 48 (FreeCut default). **+**
    saves the current color; right-click a saved swatch for *Remove*, *Move
    left* and *Move right*. Stored per user in `colors.json` in the config
    folder, never in the project;
  - *Brand*: the active brand kit's colors (OV-043, later).
- Eyedropper (v1): click arms it, the pointer becomes a picker with a
  magnified readout, a click in the player samples the composited frame at
  the playhead (TR-011's rule), `Esc` cancels. Colours outside FreeCut's
  window are typed as hex instead.
- Keyframe diamond (v1; states and behaviour owned by TR-028/TR-030 in
  `clip-transform-and-speed.md`): empty outline = no keyframes; outline with a
  dot = animated, no key at the playhead; filled = key at the playhead. This
  area only adds colours (grey when not animated, accent otherwise). Click
  adds/removes a key at the playhead; small ‹ › arrows jump to the
  previous/next key.

**Behavior / output:** Scrubbing updates the preview continuously and creates
one undo entry on release; typing creates one entry on commit; no change → no
entry. Disabled controls explain why in their tooltip. Typed decimals accept
both `.` and `,`; display follows the UI language (UI-045).

**Edge cases:**
- Typed value outside the hard range → clamped, field flashes briefly. Slider
  ("soft") ranges may be narrower than hard limits; typing can exceed the
  slider range up to the hard limit (limits owned by each area).
- `Esc` during a scrub drag → value returns to its pre-drag value, no undo
  entry.
- Invalid text ("abc") → red outline; blur or `Esc` reverts. NaN or infinity
  never reaches the model.
- Integer-only fields round; time fields snap to frames.
- `Ctrl+Z` pressed during an active drag → ignored until release.
- Saving a color that is already saved → refused; the existing swatch flashes
  and the tooltip says "Already saved". Palette swatches can't be removed.
- **+** with 48 saved colors → disabled, tooltip "Saved colors are full (48).
  Remove one to add another." Reference: the observed app also caps its saved
  swatches (the count wasn't visible) and refuses additions past the cap.
- `colors.json` can't be written (read-only config folder, disk full) → one
  error toast (UI-040); the change is kept for this session.
- `colors.json` damaged → start with an empty *Saved* row, keep the damaged
  file as `colors.json.corrupt-<date>`, log it (as UI-018).

**Undo:** per owning operation, coalesced as above.

**Model notes:** `freecut-app::widgets::{NumberField, SliderField, Vec2Field,
ColorField, KeyframeButton}`. Values travel as typed property values: `f64` for
non-time quantities, `Rational` for anything measured in time. Never `f32`/
`f64` for time. Recent and saved colors live in
`freecut-app::widgets::ColorLibrary`, read and written on the background
executor; OV-043 supplies brand colors to it.

**Implementation note (2026-10-13):** a label scrub keeps its exact value from
the frame it began, whatever the stored value rounds to, and changes the
value by whole shown digits toward where it began, so the same distance moves
it as far left as right at any speed (× 0.1 with `Alt` included, whole-number
rows too). A colour picker drag released while moving is still one entry: the
release frame belongs to the drag. Every numeric field, the paragraph's box
width included, reverts on `Esc` during a drag with no entry.

---

### UI-010 Inspector multi-selection editing — v1

**What the user sees:** With several items selected the header reads
"3 clips selected" with type icons. Only tabs and sections common to all items
are shown. Fields whose values differ show "—".

**Inputs / controls:** same controls as UI-009.

**Behavior / output:**
- Typing a value sets it on every item.
- Scrubbing a mixed ("—") field adds the same delta to each item; scrubbing a
  uniform field keeps them equal.
- Mixed kinds (video + text): the shared subset is Video [Transform, Opacity]
  and Motion. If nothing is shared, a list of the selected items (name,
  type, track, start) is shown with filter buttons ("Only video clips",
  "Only text").
- Keyframe diamonds are hidden in multi-selection (multi-clip keyframing is
  later). Text content is hidden; text styling (font, size, color) is shared.

**Edge cases:**
- Items on locked tracks are skipped; banner "2 locked items won't change".
- A relative delta pushes one item past its limit → that item clamps, the
  others keep moving.
- Very large selections (1,000+ items) → still one operation; target < 100 ms.
- Clips + transitions selected → clip tabs only; transitions are listed.

**Undo:** one compound entry ("Change Opacity on 3 clips"); undo restores every
original value.

**Model notes:** uses the batch/compound operation from `timeline-editing.md`;
the inspector builds one sub-op per item.

**Implementation note (2026-10-13):** the text cards (text-and-captions.md,
*Text round 2*) and Placement's position follow this already: mixed values
show "—", typing or picking any value applies it to all, and editing one axis
of a position keeps each item's other axis.

---

### UI-011 Timeline toolbar — mvp

**What the user sees:** A 36 pt strip above the tracks (40 pt in the rev-2
concept, which wins; see the UI concept §13.26).
- Left: **Add** `+` menu (toolbar round, below) | **tool button** with a
  chevron for **Select** / **Blade** / **Hand** (v1, TL-024) | **Undo**,
  **Redo** (TL-043) | **Split** (TL-023), **Cut away before the playhead** and
  **Cut away after the playhead** (v1, TL-026), **Delete** (TL-025), the
  selection count | **Add marker** split button (v1, TL-037) | clip actions
  (all v1): **Freeze frame** (TL-031), **Reverse** (TR-043), **Flip** (TR-003;
  the action itself is mvp in the inspector), **Rotate 90°** (TR-005), **Crop**
  (TR-009).
- Right: **Record voiceover** (v1, AU-027) | toggles **Magnetic main track**
  (TL-005), **Snapping** (TL-028), **Follow main track** split button (v1,
  TL-029, with TL-057 in its menu; directly beside the magnet, since the two
  decide together how other tracks react to main-track edits), **Hover skim**
  split button (v1, TL-014) | **Zoom out**, zoom slider, **Zoom in**, **Zoom to
  fit** (TL-041) | a `⋯` toolbar menu.
- The `⋯` toolbar menu holds the less-used sequence modes, starting with
  **Ripple all tracks** (v1, TL-021) as a checkable item, plus any buttons
  that no longer fit when space is short (below).

The mvp strip therefore holds Undo, Redo, Split, Delete, Snapping,
Magnetic main track and the zoom controls; other buttons appear when their
commands exist (UI-030). In the mvp the `⋯` menu appears only when something
has folded into it.

Reference: CapCut's desktop timeline toolbar groups split/delete/crop-style
clip actions on the left and magnet/snap/zoom toggles on the right; FreeCut
follows that familiar split.

**Toolbar round (2026-10-13; FreeCut defaults unless a TL/AU entry says
otherwise):**
- **Add `+` menu** (replaces R5's add-track button in the same place):
  *Import media…* (`Ctrl+I`, PM-030), *Import folder…* (PM-032), *Add track ▸*
  one item per track kind (TL-006; the new track goes where TL-006 puts that
  kind), *Add text at playhead* (`Ctrl+T`, TX-001), *Record voiceover…*
  (AU-027). Items whose feature isn't built are not shown (UI-030).
- **Tool button:** shows the active tool's icon; a click switches between
  Select and the last other tool; the chevron lists Select (`A`), Blade (`B`)
  and Hand (`H`) as radio items (TL-024). The tooltip names the active tool and
  its key.
- **Split buttons** (marker, *Follow main track*, hover skim): one 2-part
  control, the icon part 28 pt and a 12 pt chevron part, sharing one chip
  outline; each part has its own hover and tooltip, the icon part runs the main
  command, the chevron opens the menu below the button. The *checked* state
  covers both parts. Keyboard focus visits the two parts as separate stops
  (UI-041), and screen readers announce them as "Hover skim, toggle button" and
  "Hover skim options, menu button" (UI-042).
- **Record voiceover:** a microphone button that opens AU-027's recorder panel
  anchored above the toolbar; while a take is counting down or recording, the
  button shows a `danger` fill with the elapsed time beside it, and
  a click stops the take (the panel's Stop). Without any input device it stays
  enabled and opens the panel in its Waiting state, which explains why.
- **Fold order** when space is short (extends the rule below): the clip-action
  group folds first, then the marker button, then the right-hand toggles (the
  magnet and *Follow main track* together, then snapping and skim), then the
  two cut-away buttons, then the zoom slider shrinks to its buttons, then the
  Add menu. The tool button, Undo/Redo, Split, Delete and Record never fold. A
  folded split button becomes a submenu in `⋯` with its main command first.
- **Command ids** (UI-030): `timeline.add_menu`, `timeline.tool.select`,
  `timeline.tool.blade`, `timeline.tool.hand`, `timeline.cut_before_playhead`,
  `timeline.cut_after_playhead`, `timeline.add_marker`,
  `timeline.add_marker_next_color`, `timeline.marker.next`,
  `timeline.marker.previous`, `timeline.markers.delete_on_clip`,
  `timeline.markers.delete_all`, `timeline.toggle_linked_selection`,
  `timeline.skim.toggle`, `timeline.skim.set_mode`, `audio.record_voiceover`,
  besides the existing ones below.

**Inputs / controls:** click; every button is a command with its shortcut in
the tooltip (UI-031).

**Behavior / output:** Buttons enable/disable from the command's availability
check. Toggles show an accent fill when on. Where a toggle's state is stored
is the owner's: snapping and hover skim are per-user preferences (TL-028,
TL-014); the magnetic main track, *Follow main track* and *Ripple all tracks*
are per-sequence flags saved in the project and changed only through undoable
`SetSequenceFlag` ops (TL-003; behaviour in TL-005, TL-029, TL-021; labels and
coalescing in TL-043). The toolbar reads them from the sequence being edited,
so opening a compound clip (TL-040) shows that nested sequence's flags.
Because *Ripple all tracks* is tucked into the `⋯` menu but changes what every
ripple edit does, the `⋯` button shows an accent dot while it is on, and its
tooltip says so (FreeCut default). When space is short, the clip-action group
folds into the `⋯` menu first, then the right-hand toggles (the magnet and
*Follow main track* fold together), then the zoom slider shrinks to its two
buttons. Folded toggles keep their check mark in the menu.

**Edge cases:**
- No selection → clip actions disabled, tooltip "Select a clip".
- Selection on a locked track → disabled, "The selected clip is on a locked
  track".
- Split with the playhead exactly on a cut → disabled, "The playhead is on a
  cut".
- Pressing Split during playback splits at the frame being shown
  (`timeline-editing.md`).
- Toggling *Follow main track* or *Ripple all tracks* during playback → applies
  at once and moves nothing: *Follow main track* only creates or clears
  attachments (TL-029), and *Ripple all tracks* only changes later ripple edits
  (TL-021).
- Main track locked → *Follow main track* and *Ripple all tracks* can still be
  toggled (no clip moves); switching the magnet on is refused per TL-005, with
  TL's reason in the tooltip.
- Sequence with an empty main track → *Follow main track* still toggles; there
  is just nothing to attach to (TL-029).
- Undo of a toggle while its button is folded into `⋯` → the check mark in the
  menu updates; the `⋯` dot follows *Ripple all tracks*.
- Cut-away buttons with nothing to act on → disabled with TL-026's reason
  ("Select a clip or put the playhead over the main track" / "Move the playhead
  inside the clip").
- Blade or Hand active while a text field elsewhere has focus → tool keys type
  into the field, as UI-030's focus rule says; the tool button still works.
- Record pressed while a take is already running in the panel → it stops that
  take (one recorder at a time; AU-027).
- Add marker with exactly one clip selected but the playhead outside it → a
  timeline marker is added (TL-037's rule); the tooltip of the flag says which
  owner it will get ("Add a marker to the clip" / "Add a marker to the
  timeline"; FreeCut text).
- Hover skim's *Picture and sound* item before AU-016 exists → not shown; `S`
  toggles Off and *Picture only* (TL-014, slice T5).

**Undo:** buttons dispatch undoable ops (TL, TR). The magnet, *Follow main
track* and *Ripple all tracks* toggles are undoable `SetSequenceFlag` changes
(TL-003): the magnet's entry carries its compaction (TL-005), *Follow main
track*'s carries the attachments it created or cleared (TL-029), and *Ripple
all tracks*' carries no content change (TL-021). Snapping, hover skim, linked
selection (TL-057) and the active tool are preferences or view state (n/a).
Markers and cut-away are TL ops (TL-037, TL-026); a voiceover take is AU-027's
`InsertRecordedClip`.

**Model notes:** `freecut-app::timeline_toolbar`; the toggles dispatch command
ids `timeline.toggle_magnet`, `timeline.toggle_follow_main_track` and
`timeline.toggle_ripple_all_tracks`, which build TL's flag ops. Ops are defined
in `timeline-editing.md` and `clip-transform-and-speed.md`.

Build notes (app, toolbar round T1–T4, 2026-10-14):
- The strip holds, left to right: Add `+` | tool button | Undo, Redo | Split,
  the two cut-away buttons, Delete, the count | marker split button | Mixer;
  then Record | magnet, *Follow main track*, snapping, hover skim | zoom | `⋯`.
  The Mixer toggle (R5) stands in the clip-action slot until those actions
  exist, so it folds first.
- `fold_toolbar(width, Widths) -> Fold` is pure; `Fold::steps()` is the fold
  sequence above, and a folded split button is a `⋯` submenu. A step that only
  trades one chip for the `⋯` chip is skipped, so two groups can fold at the
  same width.
- The Add menu leaves out *Import folder…* until PM-032 exists.
- Keys built: `A`/`B`/`H`, `Q`/`W` (`Shift`: ripple other tracks), `M`,
  `Alt+M`, `Shift+M`, `Ctrl+Shift+M`, `N`, `Shift+N`, `Alt+N`. The plain-`S`
  split alias is gone; `Mod+B` splits.
- *Select linked clips together* is kept beside snapping in the timeline's
  view state (not yet saved with the user settings), on by default. Stage
  clicks in the Player don't expand linked selections yet.

Build notes (app, T5 and T6c, 2026-10-14):
- *Hover skim* is a working split button: the toggle switches Off and the
  last on-mode (as `S` does), and the menu lists the modes this build has
  (TL-014). *Record voiceover* opens AU-027's panel, hanging above the button.
  While a take runs, the button has the `danger` fill, with the countdown and
  then the elapsed time beside it, and a click stops the take. A click while
  the panel is open and idle closes it. The Add menu now has
  *Record voiceover…*, and the Timeline menu has *Hover skim* and
  *Record Voiceover…*.

---

### UI-012 Top bar, application menu & save status — mvp

**What the user sees:** Left: a menu button (≡), the project name (click to
rename) and the save status defined by PM-012 — "Saved", "Saving…", "Unsaved
changes", "Save failed — Retry" — plus "Recovered" after a crash recovery
(PM-015, v1). Right: shortcuts button (opens the cheat sheet, UI-033, v1),
Preferences gear, **Export** (accent button, EX-021).

Menu contents (FreeCut default; an item appears only once its command exists
in the build, so the mvp menus are short):
- **File:** New project, New from template… (v1, OV-032), Open project…, Open
  recent ▸ (PM-017), Save, Save as…, Save a copy… (PM-012), Restore from
  backup… (v1, PM-016), Import media…, Import captions… (v1), Export…, Export
  frame… (v1, EX-014), Project settings…, Open in new window… (later,
  UI-057), Close project, Quit.
- **Edit:** Undo *‹op name›*, Redo *‹op name›*, Cut, Copy, Paste, Paste
  attributes… (v1, TL-034), Duplicate, Delete, Ripple delete, Select all,
  Deselect all, Group, Ungroup, Preferences…, Keyboard shortcuts….
- **View:** Layout ▸, Show asset panel, Show inspector, Maximize panel,
  Panels ▸ (UI-056), Windows ▸ (later, UI-054, UI-057), History (later,
  TL-044), Full-screen preview, Zoom ▸, Theme ▸.
- **Clip:** Split, Split all tracks, Trim start / end to playhead, Freeze
  frame, Reverse, Speed ▸, Enable/disable, Detach / restore audio (TL-030),
  Create / break apart compound clip (TL-040), Rename, Reveal in media panel,
  Reveal in file manager.
- **Timeline:** Add track ▸, Add marker, Add marker with next colour (v1,
  TL-037), Next / previous marker, Snapping, Magnetic main track, Follow main
  track (v1, TL-029), Ripple all tracks (v1, TL-021), Hover skim, Set range in
  / out, Clear range.
- **Playback:** Play/pause, Play range, Loop, Go to start / end, Previous /
  next frame, Previous / next edit, Go to timecode…, Mute preview audio (v1,
  EX-019), Preview resolution ▸ (EX-006), Use proxies (PM-081).
- **Help:** User guide, Troubleshooting (v1, UI-051), Keyboard shortcuts,
  Take the tour (later, UI-059), What's new (later, UI-060), Check this
  computer… (v1, UI-061), Open logs folder, Copy system info, Report an
  issue… (v1, UI-063), Suggest a feature (opens the project's issue tracker
  in the browser), Restart in safe mode (v1, UI-024), About FreeCut. In the
  mvp *Report an issue* simply opens the issue tracker in the browser.

Reference: the observed application menu has import (with a keyboard
accelerator), settings, help and FAQ, quit and a Window menu, plus a dock or
taskbar menu with open and quit; FreeCut's dock and jump-list entries are
UI-053's.

**Inputs / controls:** click; keyboard via shortcuts; menus close on `Esc`.

**Behavior / output:** Menus are generated from the command registry, show
right-aligned shortcuts, and show disabled reasons as tooltips. Toggle items
(Snapping, Magnetic main track, Follow main track, Ripple all tracks, Hover
skim, Loop, Mute preview audio, Use proxies) show a check mark for their
current state; the three sequence flags read the sequence being edited, like
the toolbar (UI-011). Undo/Redo items name the operation ("Undo Split clip",
labels from TL-043). Clicking a failed
save status opens the error details and PM-012's *Retry*. Clicking the
project name (v1) starts PM-018's rename inline. On macOS the menus are also
mirrored into the native menu bar in v1 (UI-053).

**Edge cases:**
- Very long project names → middle ellipsis, full name in tooltip.
- Failed-save status stays red until the next successful save.
- Export with an empty timeline → disabled with the reason EX-036's preflight
  gives.
- Rename to an empty name → reverts; other invalid names follow PM-018.
- *Paste attributes…* with TL-034's attribute clipboard empty, or with no
  clip selected → disabled, with the availability reason as the tooltip
  (e.g. "Copy a clip first", "Select clips to paste onto").

**Undo:** n/a. Renaming is PM-018's file operation (not undoable); menu items
dispatch their own ops.

**Model notes:** `freecut-app::shell::top_bar`, `freecut-app::menus` (built
from the registry).

**Implementation note (2026-10-12):** the accent **Export** button sits at
the right end of the menu bar row (there is no separate top bar yet), with
File › Export… (`⌘E` / `Ctrl+E`, shown in the menu) next to Save As….
While an export runs it reads "Exporting…" and reopens the progress view
(EX-021).

---

### UI-013 Dark theme & design tokens — mvp

**What the user sees:** FreeCut is dark by default: graphite panels floating
on a darker ground, controls lifted one step above the panel, translucent
hairlines, light grey text in three levels, one accent colour (Iris, a
blue-violet) for focus, selection and primary buttons, and a distinct colour
plus icon per clip type.

**Token values** — neutrals, text, accent, status, editing-surface colours
(clip kinds, playhead, proposed-edit ghosts, canvas guides), spacing scale,
two densities (Comfortable and Compact), radii, typography scale, elevation
and motion durations — are defined once, for the dark, light and
high-contrast themes, in
[ui-concept §12](../design/ui-concept.md#12-design-tokens), with their measured
contrast ratios. That table replaces the values this feature listed before
2026-10-12 (blue accent `#5B8CFF`, `bg.app #121316`, …); the concept's
revision 2 (2026-10-13) re-spaced the neutrals and added `bg.field`,
`border.strong`, timeline lane and ruler tokens and clip body wells. Clip
colours are unchanged: video & image `#2F6F8F`, audio `#2F7D5B`, text
`#9C6B1E`, sticker/shape `#7A4FA3`, effect layer `#A3476B`, filter/adjustment
layer `#4F5DB3`, caption `#8C7A2B`; transition overlay white at 25 %; selected
clip: a 2 pt accent outline with the fill unchanged (white outline in High
contrast).

Typography and metrics (summary of the concept, revision 2): UI font
**Inter** (OFL-1.1), 12 pt body, 11 pt small and caption, 10 pt micro, 13 pt
Medium section titles, 14 pt titles; timecode in **JetBrains Mono** (OFL-1.1)
12 pt; icons from **Phosphor** (MIT) at 14 / 16 pt, loaded as an icon font,
plus FreeCut's own glyphs for editing symbols; 2 pt spacing grid; control
height 28 pt on a 34 pt row pitch (Comfortable) / 24 pt on 28 pt (Compact);
radius 3 pt (badges), 5 pt (icon buttons, clips), 7 pt (controls, tiles),
12 pt (panels, menus, toasts), 16 pt (sheets).

**Inputs / controls:** none in mvp (dark only); see UI-014.

**Behavior / output:** Tokens are defined once and mapped to egui `Visuals`
plus the custom painters (timeline, player overlays, waveforms). Clip label
text switches between light and dark automatically based on the clip colour's
luminance so user-chosen track colours stay readable.

**Edge cases:**
- OS in light mode → FreeCut stays dark unless the user chooses otherwise.
- User track colour overrides (TL) → contrast rule above applies.
- Body and tertiary text must reach 4.5 : 1 against every surface text sits
  on (ground, well, panel, card, field, hover, active and the selection
  wash); control boundaries, focus rings and on/off states 3 : 1; a unit test
  checks the token table.

**Undo:** n/a.

**Model notes:** `freecut-app::theme::{Theme, Tokens, Density}`; the mapping
to egui 0.36 `Style` / `Visuals` is ui-concept §16.

---

### UI-014 Light, system & high-contrast themes — v1

**What the user sees:** Appearance ▸ Theme: **Dark** (default), **Light**,
**Follow system**, **High contrast**.

**Inputs / controls:** dropdown in Preferences; View ▸ Theme.

**Behavior / output:**
- Light and high-contrast token values are in
  [ui-concept §12](../design/ui-concept.md#12-design-tokens).
- High contrast: black background, white text, yellow `#FFD60A` focus ring at
  3 pt, visible borders on every control, accent buttons outlined.
- Changes apply on the next frame. *Follow system* uses eframe's system theme
  detection where the platform reports it and follows live changes; otherwise
  Dark.

**Edge cases:**
- The player surround stays neutral dark grey in every theme so colour
  judgement isn't skewed.
- Thumbnails and waveforms are not re-tinted.
- *Follow system* on a platform that doesn't report a theme → Dark.
- OS switches theme while a modal dialog is open → the dialog re-renders on
  the next frame with the new tokens.

**Undo:** n/a.

**Model notes:** `settings.appearance.theme`; token sets in `freecut-app`.

---

### UI-015 Interface scale — v1

**What the user sees:** Appearance ▸ Interface size slider, 75–200 % in 5 %
steps (default 100 %), applied on top of the operating system's display
scaling.

**Inputs / controls:** slider; command palette entries "Interface size:
larger / smaller".

**Behavior / output:** Sets the egui zoom factor immediately. egui's built-in
`Ctrl`+`+`/`-`/`0` zoom keys are disabled because they collide with timeline
zoom.

**Edge cases:**
- A scale at which the editor's minimum layout (UI-002) no longer fits the
  screen is capped, with a note.
- Thumbnails are re-requested at the new size when needed.
- Window moved to a monitor with a different OS scale → the user's factor is
  applied on top again; nothing jumps.

**Undo:** n/a.

**Model notes:** `settings.appearance.ui_scale`.

---

### UI-016 Home screen shell — mvp

**What the user sees:** When no project is open, the window shows the start
screen. Its content — **New project**, the quick-start canvas buttons,
**Open…** and the grid of project cards with their badges and "Not found"
states — is defined by PM-001 and PM-017. This area provides the frame around
it: a slim top strip with the FreeCut name, a keyboard-shortcuts button and a
Preferences gear; a banner area for recovery prompts (PM-015, v1) and safe
mode (UI-024, v1); from v1 a *Start from template* row under the quick-start
buttons (OV-032); and a footer with the version and a link to the local user
guide.

Reference: CapCut's desktop start page likewise puts a large create button
above a grid of recent drafts. Its observed UI labels show that the start
page is a hub: a sidebar switches between video editing, an image-design
workspace, an asset library, a management area and generative tools, and
much of its content (quick-tool grids, templates, first-visit tips) is
fetched from the network, with back, forward and refresh controls and an
error state for when loading fails. FreeCut's Home holds only local things —
projects, quick-start canvases, local templates (OV-032) — so it has no
loading or offline states; the rest is listed under Out of scope.

**Inputs / controls:** pointer per PM-001. Keyboard: arrows move between
cards, `Enter` opens the focused card, `Shift+F10` / Menu key opens its menu
(UI-035), `Ctrl+N` new, `Ctrl+O` open; `F2` and `Del` arrive with PM-018 (v1,
UI-017). Dropping files on the window follows PM-031 (UI-036).

**Behavior / output:**
- At launch keyboard focus is on **New project**; after closing a project,
  focus returns to that project's card.
- Opening shows a progress overlay with *Cancel* (PM-013's "Opening…"); the
  window keeps repainting because header reads, scans and opening all run off
  the UI thread (PM-001).
- Scroll position and, from v1, the grid/list choice are workspace state.

**Edge cases:**
- Missing, corrupt or newer-format projects and lock conflicts → shown and
  handled as PM-001 and PM-013 define.
- A project that fails to open → back on Home with an error toast and the
  card focused.
- Very narrow window → cards reflow down to one column; the top strip keeps
  the gear and shortcuts button.
- Hundreds of projects → PM-001's virtualized grid; keyboard navigation still
  scrolls the focused card into view.
- No projects yet (first launch) → the grid area shows an empty state with
  **New project** and **Open…**, and from the tour's release (UI-059, later)
  *Take the tour*.
- The banner area holds at most one banner at a time, in this order:
  recovery (PM-015), safe mode (UI-024), what's new (UI-060, later).

**Undo:** n/a (no project open).

**Model notes:** `freecut-app::home` renders PM-001's `ProjectHeader`s
(`freecut-project::read_header`, called on the I/O thread) and the PM-017
registry; this module adds only layout, focus and banners.

---

### UI-017 Project actions on Home: placement & input — v1

**What the user sees:** The project actions of PM-018 (Rename, Duplicate,
Reveal in file manager, Remove from list, Move to Trash) on a `⋯` button per
card and in the card's right-click menu; PM-001's v1 search field, sort menu
and grid/list toggle above the grid. List view columns: name, location,
modified, duration.

**Inputs / controls:** `F2` starts inline rename of the focused card; `Del`
asks to move the selected cards to the Trash; `Ctrl`/`Shift`+click
multi-selects (PM-018); `Ctrl+F` focuses the search field.

**Behavior / output:** What each action does, and which cards allow it, is
PM-018's. This area builds the menus from the command registry (UI-035), shows
the confirmation dialog with *Cancel* as the default button, and puts inline
errors (name taken, invalid characters) directly under the rename field.
With two or more cards selected, a bar above the grid shows a live count ("3
selected"), the actions valid for every selected card and *Clear
selection*; the count updates as cards are added or removed. Reference: the
observed project list shows the same kind of selection counter for batch
actions.

**Edge cases:**
- Multi-selection where some projects are refused (open in another instance,
  PM-018) → the action runs for the rest; one message lists the refused ones.
- Rename field loses focus with an invalid name → reverts to the old name.
- *Remove from list* not offered for a card (PM-018: projects in the default
  folder) → the item is hidden, not greyed.
- No OS trash available → PM-018's explanation is shown in place of the
  confirmation.
- A selected card scrolls out of view or is hidden by the search filter →
  it stays selected and counted; the bar says "3 selected (1 hidden by
  search)".
- `Esc` with a multi-selection → clears the selection before anything else
  on Home.

**Undo:** n/a (file operations outside a project; the OS trash restores
trashed projects).

**Model notes:** `freecut-app::home`; the operations themselves live in
`freecut-project` (PM-018) and run on the I/O thread.

---

### UI-018 Preferences window & settings storage — mvp

**What the user sees:** `Ctrl+,` opens Preferences as a non-modal window with a
section list and a search box. Sections: General, Appearance, Editing, New
projects (PM-020), Saving & backups (UI-022), Storage (UI-026), Performance,
Playback & audio, Mouse & trackpad (UI-038), Smart tools (AI-001, v1), Cloud generation (CG-001, v1, opt-in),
Keyboard shortcuts (UI-032), Export (EX), Advanced. Each row
has a label, a control, one line of help, a *Restart required* or *Applies
when a project is next opened* tag where relevant, and ↺ when changed. Each section ends with *Restore section
defaults*. The mvp window has only the Playback & audio section (AU-010's
output device and buffer size); the other sections appear as their features
ship.

**Inputs / controls — settings owned by this area (FreeCut defaults):**

| Key | Control | Default | Range / options | Applies | Feature | Priority |
|---|---|---|---|---|---|---|
| `general.language` | dropdown | System, fallback English | shipped locales | immediately | UI-045 | v1 |
| `general.startup` | radio | Show Home | Home / Reopen last project | next launch | UI-019 | v1 |
| `general.time_display` | dropdown | Timecode | Timecode / Seconds / Frames | immediately | UI-019 | v1 |
| `general.tooltips` | toggle | on | — | immediately | UI-039 | v1 |
| `general.remember_dialog_choices` | toggle | on | — | immediately | UI-019 | v1 |
| `general.hints` | toggle | on | — | immediately | UI-058 | v1 |
| `general.system_check_at_startup` | toggle | on | — | next launch | UI-061 | v1 |
| `general.file_manager_entry` | toggle | off | — | immediately | UI-053 | v1 (Windows) |
| `general.whats_new` | toggle | on | — | next update | UI-060 | later |
| `general.check_updates` | toggle | off | — | immediately | UI-055 | later |
| `general.include_prereleases` | toggle | off | — | next check | UI-055 | later |
| `appearance.theme` | dropdown | Dark | Dark / Light / Follow system / High contrast | immediately | UI-014 | v1 (mvp is Dark with no control) |
| `appearance.ui_scale` | slider | 100 % | 75–200 %, step 5 | immediately | UI-015 | v1 |
| `appearance.reduced_motion` | toggle | off | — | immediately | UI-043 | v1 |
| `appearance.animated_previews` | toggle | on | — | immediately | UI-007 | v1 |
| `appearance.preview_on_selected_clip` | toggle | on | — | immediately | UI-007 | v1 |
| `appearance.portrait_layout` | toggle | off | — | next project opened | UI-003 | v1 |
| `appearance.tool_suggestions` | toggle | on | — | immediately | UI-065 | later |
| `performance.gpu_adapter` | dropdown | Auto (high performance) | detected adapters | restart | UI-028 | v1 |
| `performance.compat_graphics` | toggle | off | — | restart | UI-028 | v1 |
| `performance.job_threads` | number | Auto (PM-073's pool size) | 1 – 64 | restart | UI-028 | v1 |
| `performance.memory_warnings` | toggle | on | — | immediately | UI-062 | v1 |
| `playback.pause_on_blur` | toggle | off | — | immediately | UI-029 | v1 |
| `input.timeline_wheel` | dropdown | Scroll tracks | Scroll tracks / Scroll time / Zoom time | immediately | UI-038 | v1 |
| `input.pinch_zoom` | toggle | on | — | immediately | UI-038 | v1 |
| `input.middle_drag_pan` | toggle | on | — | immediately | UI-038 | v1 |
| `advanced.log_level` | dropdown | Info | Error / Warn / Info / Debug | immediately | UI-052 | v1 |
| `advanced.show_incomplete_translations` | toggle | off | — | immediately | UI-045 | v1 |

**Settings hosted here but owned by other areas** (shown in the listed
section; defaults, ranges and meaning are the owner's and are not restated
beyond this summary):

| Section | Setting | Default (owner's) | Owner |
|---|---|---|---|
| Playback & audio | Output device; buffer size | System default; 1024 frames (256–4096) | AU-010 (mvp) |
| Playback & audio | Audio scrubbing | on | AU-016 |
| Playback & audio | Audio while hover-scrubbing media | off | PM-060 |
| Playback & audio | Return playhead to start point when playback stops | off | EX-002 |
| Editing | Timeline follows playhead during playback | Page | TL-013 |
| Editing | Undo depth | 200 (20 – 1,000) | TL-043 |
| Editing | Snapping on/off | on | TL-028 |
| Editing | Default transition and its duration | cross-dissolve, 0.5 s | FX-009, FX-002 |
| Editing | Default text duration | 3 s (0.5 – 60 s) | TX-001 |
| Editing | Auto-key animated properties | on | TR-028 |
| New projects | Location (default project folder), canvas, frame rate, still duration, proxy policy and other new-project defaults | — | PM-020 |
| Saving & backups | Autosave, its maximum interval, backups kept | on, 30 s, 20 | PM-014, PM-016 |
| Storage | Cache location, size limit, thumbnail sub-limit | platform cache folder, 20 GB, 2 GB | PM-074 |
| Storage | Warn below; pause background work below | 5 GB (1–100 GB); 1 GB (0.5–20 GB) | PM-113 |
| Performance | Hardware video decoding | Auto | EX-018 |
| Performance | Hardware encoding | Auto (Auto / On / Off) | EX-034 |
| Performance | RAM frame cache | 25 % of RAM, up to 8 GB | EX-016 |
| Smart tools | Analyze-on-import switches; analysis pool concurrency (max CPU jobs, throttle during playback); *Clear analysis cache* | all off; half the logical cores (at least 1); throttle on | AI-001 (v1) |
| Smart tools | Models: model folder, *Allow model downloads*, compute device, CPU thread limit, *Face & body analysis* | app data `models/`; on; Auto; all cores minus 1; on | AI-002 (v1) |
| Export | Export defaults and presets | — | EX-021, EX-041 |

**Behavior / output:** Changes apply immediately and are saved 500 ms after the
last change (background, atomic). The search box filters rows by label, help
text and keywords, in the current language and English. Preferences can stay
open while editing; the editor reflects changes live. Rows needing a restart
show a *Restart now* banner (the restart path saves the project first).
- **When a change applies** is part of each row's definition: *immediately*
  (the default), *restart required*, or *applies when a project is next
  opened* (for settings a project session reads once, such as
  `appearance.portrait_layout`). The last kind shows a *Reopen project* link
  in its banner, which closes and reopens the current project through
  UI-025's prompt.
- **Dependent settings never block each other.** When one setting makes
  another meaningless (graphics compatibility mode limits the adapter list to
  GL adapters; safe mode forces hardware decoding off), the dependent row is
  shown disabled with the reason, and its stored value is kept for when the
  other setting changes back. FreeCut never refuses a click because a "rival"
  setting is on. Reference: the observed app refuses to turn on a setting
  while a conflicting one is on.
- *Restore all settings to defaults* is UI-050's *Reset all settings…*;
  each section also has *Restore section defaults* (above).

**Edge cases:**
- `settings.json` unreadable → renamed to `settings.json.corrupt-<date>`,
  defaults used, toast "Preferences were reset because the file was damaged".
- Settings written by a newer FreeCut → known keys read, unknown keys kept
  untouched on write; never migrated downward.
- Read-only config folder → settings work in memory; one warning toast.
- Two instances change settings → last writer wins (documented; no live
  merging).
- Invalid values (e.g. a folder that is not writable) → rejected with an inline
  message; previous value kept.

**Undo:** n/a (settings are outside the project undo stack; each row has ↺).

**Model notes:** `freecut-app::settings::{Settings, SettingsStore}` — serde,
`#[serde(default)]` on every field, `version: u32` with forward migrations,
unknown keys preserved via a flattened `serde_json::Map`. Durations stored as
rationals (e.g. `{ "ticks": 5000, "timebase": 1000 }`), not floats. Owner-defined
settings are declared by their owning module (key, type, default, range, help
id) and stored in the same file; this area only renders and persists them.
Crates receive plain config structs they define themselves, e.g.
`freecut_media::DecodeConfig`, `freecut_render::RenderConfig`,
`freecut_audio::OutputConfig`.
- mvp implementation (FreeCut defaults): `freecut-app::settings` keeps the
  file as a JSON object and reads only the keys it knows
  (`playback.output_device`, a device name or `null` for the system default;
  `playback.buffer_frames`); every other key, at any depth, is written back
  untouched, and a higher `version` stays. A known key of the wrong type or
  out of range takes its default without the file counting as damaged; only
  a file that can't be read or isn't a JSON object is renamed to
  `settings.json.corrupt-YYYY-MM-DD-HHMMSS` (UTC; `-1`, `-2`… when taken).
  The folder is `~/Library/Application Support/FreeCut` on macOS,
  `%APPDATA%\FreeCut` on Windows and `$XDG_CONFIG_HOME/freecut` (else
  `~/.config/freecut`) elsewhere; the folder is created on first save. The
  file is read once at startup on the job pool. Writes run on one writer
  thread, in order: a temporary file in the same folder, flushed, then
  renamed over `settings.json`. The first failed write switches to
  in-memory settings with the one warning toast, and Preferences then says
  "Changes last until you quit". Closing the window writes a change still
  waiting for its 500 ms (waiting at most 1 s). The mvp toasts are the
  status line. The window opens with ⌘, / Ctrl+, (also File ›
  Preferences…), from Home too; its section list holds only Playback &
  audio, and the search box matches every typed word against each row's
  label, help text and keywords.
  Settings changed in Preferences before the file has been read replace
  only those settings when it arrives; the file's other values stay (a
  setting that differs from its default in memory wins over the file).

---

### UI-019 General settings: startup, time display, tooltips — v1

**What the user sees:** Preferences ▸ General: *On startup* (Show Home /
Reopen last project), *Show times as* (Timecode / Seconds / Frames), *Show
tooltips*, *Remember choices in dialogs* with a *Forget remembered choices*
button, *Show hints* and *Reset hints and "don't ask again" choices*
(UI-058), and *Language* (arrives with UI-045). The default project folder
is a new-project default and lives in Preferences ▸ New projects (PM-020).
Reference: the observed editing preferences include remembering the last
choices made in dialogs.

**Inputs / controls:** see the first table in UI-018.

**Behavior / output:**
- *Reopen last project* opens the most recent PM-017 entry directly, skipping
  Home, unless the last session crashed (then the recovery prompt wins,
  PM-015).
- *Show times as* changes every time display (player readout, ruler
  tooltips, inspector duration fields) at the UI edge only; the model is
  unaffected. Timecode follows TL-002's format; seconds format: `12.40 s`;
  frames format: `298 f`. Typed input in any format is parsed back to
  rational ticks and snapped to the frame grid (TL-001).
- *Remember choices in dialogs* (on): option dialogs open with the options
  the user last confirmed in them — for example the groups ticked in
  UI-050's reset dialog, the *Report an issue* inclusions (UI-063), the
  section checklist of a save-as-preset dialog (UI-064). Each
  owning dialog declares which of its fields are remembered; file names,
  ranges and anything that depends on the current selection never are. Off:
  every dialog opens with its defaults. The memory is per user, stored in
  `settings.json` under `dialogs.<dialog-id>`, never in the project. Folders
  in OS file dialogs follow the owners' own "last used folder" rules
  (PM-030, EX-014) whatever this switch says. Export settings are not dialog
  memory; EX-021 keeps them with the project.

**Edge cases:**
- Reopen-last target missing → Home is shown with a toast.
- Reopen-last target locked by another instance → Home is shown and PM-013's
  lock dialog appears.
- Frames format for very long timelines → no thousands separators inside
  numbers that users may type back.
- Switching *Show times as* while a duration field is being edited → the
  field commits first, then re-renders in the new format.
- A remembered dialog choice that is no longer valid (an option removed in a
  newer version, a preset that was deleted) → that field falls back to its
  default; the others keep their remembered values.
- *Forget remembered choices* → clears every `dialogs.*` entry at once; no
  confirmation, because it only restores defaults.

**Undo:** n/a.

**Model notes:** `settings.general.*`; formatting helpers in
`freecut-app::fmt` built on the rational time type from `freecut-timeline`.

---

### UI-020 Editing section in Preferences — v1

**What the user sees:** Preferences ▸ Editing groups the editing defaults other
areas define: timeline follow mode during playback (TL-013), undo depth
(TL-043), snapping (TL-028), default transition and its duration (FX-009,
FX-002), default text duration (TX-001) and auto-keying (TR-028). The
still-image duration is a project setting (PM-010), so this section only links
to Project settings for it.

**Inputs / controls:** duration fields (seconds, or timecode per UI-019),
numbers, dropdowns and toggles using UI-009's controls. Defaults and ranges are
the owners' (second table in UI-018).

**Behavior / output:** Each value is passed to its owner as a parameter (for
example FX-009's add-transition op receives the default duration). Defaults
apply only to items added after the change; existing clips never change. In
the mvp the owners use their built-in defaults and this section does not
exist.

**Edge cases:**
- A typed duration outside the owner's range → clamped to it, field flashes
  (UI-009).
- A duration that is not a whole number of frames → stored as typed; the
  owner rounds it to the project frame grid when used (e.g. FX-002 rounds a
  transition up to whole frames, never below one frame).
- Lowering undo depth below the current history length → the oldest entries
  are trimmed immediately (TL-043), after a one-time warning shown here.

**Undo:** n/a (preferences).

**Model notes:** values stored as rationals in `settings.json` under each
owner's key; consumers (TL, TX, FX, TR) receive them as parameters to their
ops, never by reading the settings store from `freecut-timeline`.

---

### UI-021 Autosave in the UI — v1

**What the user sees:** With autosave on (PM-014), the save status in the top
bar moves between "Saving…" and "Saved" by itself, and "Unsaved changes" only
appears when autosave is off. Repeated failures show PM-014's persistent
banner under the top bar with *Retry* and *Save as…*; a read-only project
shows PM-014's "autosave disabled" banner.

**Inputs / controls:** Preferences ▸ Saving & backups (UI-022) hosts PM-014's
switch and interval. `Ctrl+S` always saves immediately (PM-012).

**Behavior / output:** The schedule, debounce, retry policy and recovery
snapshots are PM-014's and PM-015's. This area feeds the scheduler the shell
events it needs: "a gesture is in progress" (drag, scrub, text editing — only
committed operations are saved), "window lost focus", "export is starting",
"close / quit requested". It also shows the status text and the window-title
dirty mark (UI-001) from the dirty flag (TL-043's saved position).

**Edge cases:**
- Status changes faster than 300 ms ("Saving…" → "Saved") → the "Saving…"
  text is held for at least 300 ms so it doesn't flicker (FreeCut default).
- Save fails while a modal dialog is open → the banner appears when the
  dialog closes; full-screen preview shows only the error toast (UI-040).
- Autosave turned off with unsaved changes → status switches to "Unsaved
  changes" at once; nothing is written until `Ctrl+S`.

**Undo:** n/a — saving never touches the undo history (PM-012, TL-043).

**Model notes:** PM-014's `AutosaveScheduler` (pure state machine in
`freecut-app`) receives events from `freecut-app::shell`; the status widget
reads its state. No timers or I/O on the UI thread.

---

### UI-022 Saving & backups section and restore entry point — v1

**What the user sees:** Preferences ▸ Saving & backups with PM-014's *Autosave*
switch and maximum interval, PM-016's number of backups kept and days of daily
backups, and a link *Open backups folder*. File ▸ *Restore from backup…*
opens PM-016's list for the current project.

**Inputs / controls:** the controls above (UI-009 conventions); the menu item.

**Behavior / output:** Values are written to settings and handed to PM-014 /
PM-016, which own their meaning, ranges and defaults. The restore list, *Open
as copy* and retention rules are PM-016's. This area only places them and
keeps the restore list non-modal so editing can continue while browsing.

**Edge cases:**
- *Restore from backup…* with no backups for this project → the menu item is
  disabled with the reason "No backups yet".
- Turning autosave off → a one-line note under the switch says recovery
  snapshots still run (PM-014).
- Backups folder missing → *Open backups folder* creates it first.

**Undo:** n/a (preferences; opening a backup creates a new project per
PM-016).

**Model notes:** `freecut-app::settings` section definition; backup listing and
restore in `freecut-project::backup` (PM-016) on the I/O thread.

---

### UI-023 Crash reports & app-level fault handling — v1

**What the user sees:** After an unexpected exit, the next launch shows
PM-015's recovery prompt for each affected project before Home. If FreeCut
itself wrote a crash report, the prompt carries an extra line *A crash report
was saved* with **Open crash folder**, **Copy report** and **Report an issue**
(opens UI-063's report dialog; nothing is sent unless the user takes the
report to the issue tracker).

**Inputs / controls:** the three buttons above; recovery choices are PM-015's.

**Behavior / output:**
- At start each FreeCut process writes `session-<pid>.lock` (process id, start
  time) in app data and removes it on a clean exit. A leftover marker whose
  process is gone means that session crashed; it feeds the crash-loop counter
  of UI-024. Project-level crash detection is PM-013's per-project lock and
  PM-015's snapshots, not this marker.
- A panic hook writes `crash/crash-<timestamp>.txt`: FreeCut version and
  commit, OS and CPU architecture, egui/eframe and wgpu versions, GPU adapter,
  backend and driver version, FFmpeg version, the command being run, the
  thread that failed (decode, render, audio, I/O or UI) with the panic message
  and source location, the error code when the failure came from a known
  error type (UI-040), backtrace, and the last 200 log lines (with the same
  redaction as UI-052). Nothing is uploaded.
- The recovery prompt's crash line has *Show details*, which expands the
  report summary (failed thread, message, error code, driver version) with
  **Copy all**; the full file stays one click away via **Open crash folder**.
  **Report an issue** opens UI-063 prefilled with "Crash or freeze".
- **Start-up failure** (no usable graphics adapter, an unusable config or app
  data folder, a panic before the first frame): FreeCut shows a native
  message box (through `rfd`, MIT) with the reason, the log path and *Start
  in safe mode* / *Quit*, because no egui window may exist yet.
- **Restart offer:** when a running session can't continue (the GPU device
  can't be recreated, below), the dialog offers **Restart FreeCut**. It asks
  PM-015 for a recovery snapshot, starts a new FreeCut process and exits. If
  the new process can't be started, the dialog shows the OS error code and
  the crash folder link instead.
- Worker threads (decode, render, I/O) run inside panic guards: a failure
  there marks the job as failed (PM-073) instead of taking down the app.
- Crash reports older than 30 days are pruned at start (FreeCut default).

**Edge cases:**
- Native crashes inside FFmpeg or a GPU driver bypass the panic hook; they are
  detected only through the leftover session marker, and no report is
  written (a native minidump is later).
- Several FreeCut instances: each owns its own marker, so a live instance is
  never mistaken for a crash.
- GPU device lost during a session → recreate the device; if that fails, ask
  PM-015 to write a recovery snapshot immediately and tell the user to
  restart.
- Crash folder not writable → the report goes to stderr / the log only.
- The last session's marker shows memory was nearly exhausted → the prompt
  says FreeCut probably ran out of memory, with UI-062's suggestions.
- A crash whose cause is unknown (native crash, no report) → the prompt says
  "FreeCut closed unexpectedly" without guessing a reason, and still offers
  **Report an issue**.
- An optional subsystem fails to start or stops working (audio output, a
  hardware decoder, the AI runtime, one detached panel) → only the features
  that need it are disabled, with a non-blocking banner "‹Subsystem› isn't
  available — restarting FreeCut may help" (*Restart FreeCut* / *Dismiss*).
  Only a failure of the core (graphics device, config and app-data folders)
  blocks, through the start-up failure or restart paths above. Reference:
  the observed app likewise separates a non-blocking "restart recommended"
  notice from a blocking "restart required" dialog.

Reference: the observed crash dialog shows the failing module, crash address
and reason, CPU architecture, graphics driver version and date, a numeric
error code and a "copy all" button, and offers to restart the app (which can
itself fail with an error code). FreeCut shows the same kinds of facts in
its own report format; a raw crash address is only meaningful with symbols,
so FreeCut records a symbolized backtrace instead.

**Undo:** n/a.

**Model notes:** `freecut-app::recovery::{SessionMarker, CrashReport}`; the
recovery prompt and snapshots are `freecut-app` / `freecut-project` code owned
by PM-015. The marker also carries UI-062's last memory sample.

---

### UI-024 Safe mode & crash-loop protection — v1

**What the user sees:** If FreeCut crashed within 60 s of starting twice in a
row, the next start offers **Start in safe mode**, **Start normally** or
**Quit**.

**Inputs / controls:** the dialog; Help ▸ *Restart in safe mode*.

**Behavior / output:** Safe mode, for this session only: hardware decoding off,
default settings in memory (the user's file is left untouched), no automatic
reopening of the last project, recovery dialog still shown, and a fallback
wgpu backend (GL, or a software adapter when available). A banner says "Safe
mode" with *Restart normally*. If safe mode runs stably, a hint suggests which
setting probably caused the crash (GPU adapter, hardware decoding) and offers
to keep safe mode's graphics settings as *Graphics compatibility mode*
(UI-028).

**Edge cases:**
- A crash caused by one project → safe mode never reopens it automatically.
- No fallback GPU adapter at all → a clear error explaining the minimum GPU
  requirements, with the log location.
- Crash inside safe mode too → the next start offers safe mode again and
  shows the crash folder link.

**Undo:** n/a.

**Model notes:** start-up counter derived from UI-023's session markers in the
app data folder; overrides applied to the in-memory `Settings` only.

---

### UI-025 Unsaved-changes prompt — mvp

**What the user sees:** When an action would close a project that has unsaved
changes, a dialog asks **Save** / **Don't save** / **Cancel**, naming the
project. In the mvp (no autosave) this is the only safety net; from v1 it
appears only when autosave is off or the last save failed (PM-014).

**Inputs / controls:** the dialog (`Enter` = Save, `Esc` = Cancel; *Save* is
the default button); `Ctrl+S`, `Ctrl+Shift+S` (PM-012).

**Behavior / output:**
- Triggers: Close project, Quit, Open another project, New project (PM-002),
  dropping a `.freecut` file on the editor (PM-031), OS shutdown / log-off
  request, `Alt+F4` / window close button.
- *Save* runs PM-012's save and closes only after it succeeds; *Don't save*
  closes without writing (from v1 PM-015 decides what happens to the recovery
  snapshot); *Cancel* returns to the editor unchanged.
- Quitting during an export: *Cancel export and quit* / *Keep working*
  (export behaviour per EX-022).
- Quitting while a save is running: wait up to 10 s with a progress note,
  then warn.
- Quitting while a copy or package job runs: PM-073's confirmation comes
  first.
- Quitting while a model download (AI-002, v1) runs: *Stop download and
  quit* / *Keep working*; the partial download is kept so it can resume
  (AI-002).
- **No "Are you sure you want to quit?" when nothing would be lost** (FreeCut
  default): with no unsaved changes and no running export, copy, package or
  download, Quit closes at once. Background jobs whose results are only cache
  (thumbnails, waveforms, proxies, analysis) are cancelled silently and
  resume next time (PM-073, AI-001). Reference: the observed app always asks
  before quitting, with an option to stop asking; FreeCut asks only when
  something would be lost, so it needs no such option.
- With several projects open (UI-057, later), the prompt runs once per
  project with unsaved changes, in window order; *Cancel* on any of them
  stops the quit and leaves every window open.

**Edge cases:**
- Save fails during the prompt → PM-012's error with *Save as…*; the app does
  not close.
- Read-only project → the prompt offers *Save as…* instead of *Save*.
- The project has unsaved changes but the dirty flag is clear because the
  user undid back to the saved point (TL-043) → no prompt.
- OS shutdown that doesn't wait for an answer → FreeCut attempts one save
  (v1: autosave already ran) and exits.

**Undo:** n/a.

**Model notes:** `freecut-app::persistence::close_guard`; dirty state from
TL-043's saved position; saving through `freecut-project::save` (PM-012) on
the I/O thread.

---

### UI-026 Storage section in Preferences — v1

**What the user sees:** Preferences ▸ Storage shows PM-074's controls: cache
location with *Change…* and *Open*, a usage bar split by PM-074's categories,
the size limit and thumbnail sub-limit, **Clear cache**, **Clear proxies** and
*Clean up project…*. Below them, PM-113's low-space thresholds (*Warn below*,
*Pause background work below*) and a *Folders* block that lists where
FreeCut keeps things, each with *Open*:

| Folder | Can be moved here? | Owner |
|---|---|---|
| Media cache (thumbnails, waveforms, render cache) | yes, *Change…* | PM-074 |
| Proxies (when a custom folder is set) | no — set in Project settings | PM-080 |
| Models | no — set in Smart tools ▸ Models | AI-002 (v1) |
| User libraries & presets (looks, styles, export presets, user stickers, LUTs, fonts) | no (FreeCut default) | owning specs, in app data (PM-042) |
| Backups, recovery snapshots, deleted projects | no | PM-014…PM-016, PM-104 |
| Logs, crash reports | no | UI-052, UI-023 |

Reference: the observed preferences let the user choose folders for
downloaded library assets and for user presets, and warn when the preset
folder has gone missing (for example on an unplugged drive). FreeCut
downloads no library assets, and keeps user libraries in app data so that
PM-042's lookup order stays fixed; a missing library folder is simply
recreated (UI-064).

**Inputs / controls:** as PM-074. This area adds a *Refresh* button for the
usage bar and a confirmation dialog for each clear action that states how much
will be freed.

**Behavior / output:**
- Usage is measured on the I/O thread when the section opens and on
  *Refresh*; the bar shows "Measuring…" until results arrive.
- Eviction rules, location changes, what counts as cache and what may never be
  deleted are PM-074's. A location move runs as a background task with
  progress and *Cancel* in the task list (UI-040).

**Edge cases:**
- Measurement takes longer than 10 s (huge cache on a slow drive) → partial
  totals are shown with "still counting".
- Section opened while a move or clear is running → controls are disabled
  with the reason, the bar shows live progress.
- Invalid new location → PM-074's refusal reason shown inline under the
  field.
- A folder in the *Folders* block doesn't exist yet → *Open* creates it
  first (as UI-022's backups folder).
- *Pause background work below* typed at or above *Warn below* → clamped
  below it, field flashes (PM-113 requires the pause value to stay lower).

**Undo:** n/a (cache maintenance); the confirmation dialog shows exactly what
will be freed.

**Model notes:** `freecut-media::cache` (PM-074) provides usage, clear and
move; `freecut-app` owns only this page.

---

### UI-027 Proxy controls placement — v1

**What the user sees:** Proxy controls in the places their owner expects:
- the per-project proxy policy, resolution and storage in the Project
  settings dialog (PM-080), with the new-project default in Preferences ▸ New
  projects (PM-020);
- *Create proxy* / *Delete proxy* in the media card context menu (UI-035);
- the "P" badge and progress ring on media cards (PM-080);
- the *Use proxies* toggle near the player and in Playback ▸ (PM-081, also
  listed in EX-006's preview menu), bound to `Alt+P` (UI-031);
- the player's "Proxy" tag when any visible frame comes from a proxy (PM-081).

**Inputs / controls:** the controls above; generation semantics, defaults
(policy Off, 720p, stored in the media cache) and thresholds are PM-080's.

**Behavior / output:** Proxy jobs appear in the task list (UI-040, PM-073).
PM-080's import-time suggestion ("this media may be slow to edit; create
proxies?") is shown as a warning toast with *Create proxies* and *Not now*.

**Edge cases:**
- Toggle switched on while no proxy exists → allowed; originals play and the
  status line hints "No proxies yet".
- Suggestion toast for many heavy files in one import → one toast for the
  whole batch, not one per file.
- Proxy failed → card badge with *Retry* in its context menu (PM-080).

**Undo:** n/a for UI placement; changing the project policy is PM-080's
`SetProjectSettings`.

**Model notes:** `freecut-app` only; `ProxyState` comes from PM-080.

---

### UI-028 Performance & GPU settings — v1

**What the user sees:** Preferences ▸ Performance: *Graphics adapter*,
*Graphics compatibility mode*, *Background job threads*, EX-018's *Hardware
video decoding* with a **Test** button, EX-034's *Hardware encoding*,
EX-016's *Frame cache memory*, UI-062's *Warn when memory is low*, and a
read-only *System* block (CPU, RAM, GPU, driver, graphics backend, FFmpeg
version, available hardware decoders/encoders) with *Run system check*
(UI-061). Preview resolution is chosen in the player menu (EX-006), not here.

Reference: CapCut's own troubleshooting pages advise turning on hardware
acceleration and lowering preview quality when playback lags; FreeCut exposes
both directly.

**Inputs / controls:**
- Graphics adapter: **Auto (high performance)** or one of the adapters wgpu
  reports (name, backend Metal / Vulkan / DX12 / GL, integrated or discrete).
  Restart required.
- Graphics compatibility mode (FreeCut default off; restart required): for
  machines where the preview turns black, flickers or looks blurry. It
  selects the GL backend, avoids optional GPU features (compute shaders,
  extended texture formats) and starts hardware decoding as Off. It is safe
  mode's graphics part (UI-024) as a permanent choice.
- Background job threads: **Auto** (PM-073's pool size) or 1–64; restart
  required.
- Hardware decoding: Auto / Off and its backends are EX-018's. *Test* decodes
  a built-in CC0 sample and reports success and speed.
- Hardware encoding: EX-034's Auto / On / Off, hosted here as the global
  preference; the export dialog can override it per export (EX-034).
- Frame cache: EX-016's budget and range.

Reference: the observed performance section has switches for GPU-drawn
interface with a choice of rendering backend, hardware decoding, hardware
encoding, proxies, workarounds for a black or blurry preview and for broken
exports, starts hardware acceleration off on unusual GPUs, and marks some
switches as needing a restart and others as needing the project reopened.
FreeCut's rows above cover them; the broken-export workaround is EX-034's
automatic software fallback, and proxies are PM-081's toggle.

**Behavior / output:** The adapter, compatibility mode and thread count apply
after restart; decoder settings reach decoders opened afterwards (EX-018).
- With compatibility mode on, *Graphics adapter* lists only GL-capable
  adapters; the row says why (UI-018's dependent-settings rule).
- A graphics driver on UI-061's known-problem list starts *Hardware video
  decoding* and the effective *Hardware encoding* as Off for that driver
  version, with a note under each row and the option to turn them back on
  (FreeCut default). Updating the driver clears the note.
- There is no HDR display setting: the preview is always SDR (CL-002).
  Reference: the observed app has an HDR display option; FreeCut leaves HDR
  display to a future color-management feature.

**Edge cases:**
- Chosen adapter missing at launch (external GPU unplugged) → Auto, with a
  toast.
- GL fallback lacks a feature (e.g. compute shaders) → the effects that need
  it show "Not supported on this graphics adapter" (`effects-transitions-animations.md`).
- *Test* on a machine without a hardware decoder → "Not available", no error.
- System block slow to gather (driver queries) → shown progressively; never
  blocks opening the section.
- Compatibility mode on, but no GL adapter exists → the toggle is refused at
  the next start with a toast and FreeCut starts on Auto (the user's value is
  kept for when a GL adapter appears).
- Hardware encoding *On* with no hardware encoder → EX-034's error, shown
  under the row as well as at export time.
- A laptop running FreeCut on its integrated GPU although a discrete one
  exists → UI-061's hint, linked from the adapter row.

**Undo:** n/a.

**Model notes:** `freecut-render::{AdapterInfo, RenderConfig}`;
`freecut-media::DecodeConfig` (EX-018), `HwEncodePref` (EX-034). Adapter
enumeration and the decode test run off the UI thread. FreeCut never resets
or installs drivers and never changes OS settings.

---

### UI-029 Playback & audio section in Preferences — mvp

**What the user sees:** Preferences ▸ Playback & audio: *Output device* and
*Buffer size* (AU-010, mvp); from v1 also *Play audio while dragging the
playhead* (AU-016), *Play audio while hover-scrubbing media* (PM-060),
*Return playhead to where playback started* (EX-002) and *Pause playback when
FreeCut loses focus* (this area, default off).

**Inputs / controls:** dropdowns and toggles; defaults and ranges are in
UI-018's tables.

**Behavior / output:** Device and buffer behaviour (switching, resampling, the
no-device banner) is AU-010's. This area lists devices, applies the choice
and, for *Pause playback when FreeCut loses focus*, sends a pause to the
transport (EX-002) when the window loses focus.

**Edge cases:**
- Device enumeration can block on some systems → runs on a background thread
  with a 2 s timeout; the dropdown shows "Searching…" meanwhile.
- The selected device is missing at launch → the dropdown shows it as
  "(not connected)" while AU-010 plays through the system default.
- Losing focus to FreeCut's own dialog (e.g. Preferences) does not count as
  losing focus.

**Undo:** n/a.

**Model notes:** `freecut-audio::OutputConfig` (AU-010); device listing via
cpal in `freecut-audio`, called off the UI thread.
- mvp implementation (FreeCut defaults): the engine's config is
  `freecut_audio::EngineConfig { device, buffer_frames }`. The device list is
  read each time Preferences opens (and with its ↻ button) by
  `freecut_audio::list_output_devices` on a helper thread; after 2 s
  without an answer the dropdown stops waiting and ↻ offers a retry, with
  the cause on hover. The dropdown lists "System default (current default's
  name)", then each device, then the saved device as "name (not connected)"
  when the list doesn't have it (by name only while the list is unknown).
  *Buffer size* offers 256, 512, 1024, 2048 and 4096 frames, each with its
  latency at 48 kHz ("1024 frames (21 ms at 48 kHz)"). Above the rows, one
  line says which device plays ("Playing on “Speakers”", or the AU-010
  reason while falling back). Each changed row shows ↺, and *Restore
  section defaults* resets both.

---

### UI-030 Command system & shortcut dispatch — mvp

**What the user sees:** Every action works the same from a button, menu,
context menu, shortcut or palette, with the same name, shortcut label and
disabled reason everywhere.

**Inputs / controls:** keyboard.

**Behavior / output:**
- **Registry.** Each command has a stable id (`timeline.split`), a label
  string id, a category, default bindings per platform, an availability check
  returning `Ok` or a disabled reason, a scope, and whether key-repeat is
  allowed.
- **Scopes:** *Global* (anywhere, including Home where meaningful),
  *Editor* (the whole editor except text fields), and panel scopes *Assets*,
  *Player*, *Inspector*, *Timeline*. A focused panel's binding wins over an
  Editor/Global binding for the same chord (e.g. `Del` in the asset panel
  removes media; in the timeline it deletes clips).
- **Text fields first.** When a text field has focus, keys without
  `Ctrl`/`Cmd` (letters, `Space`, arrows, `Del`, `Home`/`End`) and the text
  editing chords (`Ctrl+A/C/X/V/Z`, `Ctrl+Shift+Z`) belong to the field. A
  rich-text field (`text-and-captions.md`) may also claim `Ctrl+B/I/U`. Other
  `Ctrl` chords (Save, Export, Preferences) pass through after the field
  commits its value. `Ctrl+Z` inside a text field undoes typing first, not the
  timeline.
- **Repeat.** Frame step, nudge and zoom auto-repeat at the OS key-repeat
  rate; toggles and destructive commands (Delete, Split, Freeze) ignore
  repeats.
- **Modal dialogs** consume all keys except their own (`Enter`, `Esc`, `Tab`).
  In the mvp build (FreeCut default) no shortcut runs while the Restore audio
  confirmation, the unsaved-changes prompt, New project or About is open.
- **Key matching.** Letters and digits match the logical key (what the layout
  prints), so `Ctrl+Z` follows the Z key on AZERTY. Punctuation keys (comma,
  period, brackets, slash, backslash, backquote) match the physical key
  position when the layout has no such logical key, so these shortcuts still
  exist on non-US layouts.
- **Display.** macOS labels use symbols (`⇧⌘B`); Windows/Linux use text
  (`Ctrl+Shift+B`) with localized modifier names (e.g. `Strg` in German).
- egui's built-in UI zoom shortcuts are disabled (UI-015).

**Edge cases:**
- On Windows, `AltGr` arrives as `Ctrl+Alt`: `Ctrl+Alt+<key>` chords are
  ignored whenever a text field has focus, so typing `@` or `€` never fires a
  command.
- OS-reserved chords are never defaults and can't be assigned (UI-032):
  macOS `⌘H`, `⌘M`, `⌘Tab`, `⌘Space`, ⌘+backquote, `⌃←/→/↑/↓`
  (Spaces/Mission Control); Windows `Win+*`, `Alt+Tab`, `Alt+Space`, `Alt+F4` (mapped to Close
  window → UI-025); common Linux desktop chords `Super+*`, `Ctrl+Alt+←/→`,
  `Ctrl+Alt+T`, `Ctrl+Alt+Del`.
- A shortcut pressed while its command is disabled → nothing happens except a
  status-line hint with the reason (no beep).
- Keys pressed while a drag is in progress: only `Esc` (cancel) and the
  documented drag modifiers apply (UI-037).

**Undo:** dispatching has no undo of its own; commands that edit the model push
their op.

**Model notes:** `freecut-app::commands::{CommandId, Command, Scope, Binding,
Keymap, Dispatcher}`; an `is_enabled(&AppState) -> Result<(), DisabledReason>`
function per command. A unit test asserts that no two default bindings collide
within overlapping scopes on any platform.

---

### UI-031 Default keyboard shortcut map — mvp

**What the user sees:** The shortcuts below work out of the box. They follow
long-standing editing conventions (JKL shuttle, I/O range, `Ctrl+B` split) so
people coming from CapCut-class and other editors feel at home. Priority shows
when each binding ships (it follows its command's priority in the owning
spec). This table is the single source of default keys: owning specs point
here ("see UI-031") instead of proposing chords of their own, and where an
older proposal in another spec differs, this table wins (see *Superseded
proposals* below). Commands that ship without a default key are listed
explicitly as *unbound by default*, so leaving a key off is a decision, not
an omission.

**Inputs / controls:**

*File & application*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| New project | `Ctrl+N` | `⌘N` | Global | mvp |
| Open project… | `Ctrl+O` | `⌘O` | Global | mvp |
| Save | `Ctrl+S` | `⌘S` | Global | mvp |
| Save as… | `Ctrl+Shift+S` | `⇧⌘S` | Global | mvp |
| Import media… | `Ctrl+I` | `⌘I` | Global | mvp |
| Export… | `Ctrl+E` | `⌘E` | Global | mvp |
| Export current frame… (EX-014) | `Ctrl+Shift+E` | `⇧⌘E` | Editor | v1 |
| Close project (back to Home) | `Ctrl+W` | `⌘W` | Global | mvp |
| Quit | `Ctrl+Q` | `⌘Q` | Global | mvp |
| Preferences | `Ctrl+,` | `⌘,` | Global | mvp |
| Keyboard shortcut editor | `Ctrl+Alt+K` | `⌥⌘K` | Global | v1 |
| Shortcut cheat sheet | `Ctrl+/` | `⌘/` | Global | v1 |
| Command palette | `Ctrl+Shift+P` | `⇧⌘P` | Global | v1 |
| User guide | `F1` | `F1` | Global | v1 |
| Window full screen | `F11` | `⌃⌘F` | Global | v1 |

*Edit*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| Undo | `Ctrl+Z` | `⌘Z` | Editor | mvp |
| Redo | `Ctrl+Shift+Z`, `Ctrl+Y` | `⇧⌘Z` | Editor | mvp |
| Cut / Copy / Paste | `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | `⌘X` / `⌘C` / `⌘V` | Editor | v1 |
| Paste attributes… (TL-034) | `Ctrl+Alt+V` | `⌥⌘V` | Editor | v1 |
| Duplicate | `Ctrl+D` | `⌘D` | Editor | v1 |
| Delete | `Del`, `Backspace` | `⌫`, `⌦` | Editor | mvp |
| Ripple delete | `Shift+Del`, `Shift+Backspace` | `⇧⌫` | Editor | mvp |
| Select all | `Ctrl+A` | `⌘A` | focused panel | mvp |
| Select all clips on a track (TL-015) | `Ctrl+Alt+A` | `⌥⌘A` | Timeline | mvp |
| Deselect all | `Ctrl+Shift+A`, `Esc` | `⇧⌘A`, `Esc` | Editor | mvp |
| Copy style / Paste style (TX-020) | unbound by default (assignable) | unbound by default | Editor | v1 |
| Group / Ungroup | `Ctrl+G` / `Ctrl+Shift+G` | `⌘G` / `⇧⌘G` | Editor | v1 |
| Rename focused item | `F2` | `F2` | focused panel | v1 |
| Search asset panel | `Ctrl+F` | `⌘F` | Editor | v1 |

*Tools & timeline editing*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| Select tool | `A` | `A` | Editor | v1 |
| Blade tool (TL-024) | `B` | `B` | Editor | v1 |
| Hand tool (TL-024) | `H` | `H` | Editor | v1 |
| Split at playhead | `Ctrl+B` | `⌘B` | Editor | mvp |
| Split all tracks at playhead | `Ctrl+Shift+B` | `⇧⌘B` | Editor | mvp |
| Trim clip start to playhead | `Q` | `Q` | Editor | v1 |
| Trim clip end to playhead | `W` | `W` | Editor | v1 |
| Nudge selection 1 frame left / right | `,` / `.` | `,` / `.` | Timeline | v1 |
| Nudge selection 10 frames left / right | `Shift+,` / `Shift+.` | `⇧,` / `⇧.` | Timeline | v1 |
| Swap main-track clip with its neighbour (TL-019) | `Alt+,` / `Alt+.` | `⌥,` / `⌥.` | Timeline | v1 |
| Move selection to track above / below | `Alt+Up` / `Alt+Down` | `⌥↑` / `⌥↓` | Timeline | v1 |
| Select previous / next clip on target track | `Alt+Left` / `Alt+Right` | `⌥←` / `⌥→` | Timeline | v1 |
| Select clip under playhead on target track | `Enter` | `↩` | Timeline | v1 |
| Target track up / down | `Shift+Up` / `Shift+Down` | `⇧↑` / `⇧↓` | Timeline | v1 |
| Freeze frame | `Shift+F` | `⇧F` | Editor | v1 |
| Enable / disable clip (TL-036) | `Shift+E` | `⇧E` | Editor | v1 |
| Detach / restore audio (TL-030) | `Ctrl+Shift+D` | `⇧⌘D` | Editor | v1 |
| Create / break apart compound clip (TL-040) | `Alt+G` / `Alt+Shift+G` | `⌥G` / `⌥⇧G` | Editor | v1 |
| Add text at playhead | `Ctrl+T` | `⌘T` | Editor | mvp |
| Add default transition at nearest cut | `Ctrl+Shift+T` | `⇧⌘T` | Editor | v1 |
| Add marker (TL-037) | `M` | `M` | Editor | v1 |
| Add marker with next colour (TL-037) | `Alt+M` | `⌥M` | Editor | v1 |
| Next marker / previous marker (TL-037) | `Shift+M` / `Ctrl+Shift+M` | `⇧M` / `⇧⌘M` | Editor | v1 |
| Add / remove keyframe at playhead | `Alt+K` | `⌥K` | Editor | v1 |
| Add keyframe for all transform properties (TR-028) | `Alt+Shift+K` | `⌥⇧K` | Editor | v1 |
| Previous / next keyframe | `Alt+[` / `Alt+]` | `⌥[` / `⌥]` | Editor | v1 |
| Toggle snapping | `N` | `N` | Editor | mvp |
| Toggle magnetic main track (TL-005) | `Shift+N` | `⇧N` | Editor | mvp |
| Toggle Follow main track (TL-029) | `Alt+N` | `⌥N` | Editor | v1 |
| Toggle Ripple all tracks (TL-021) | unbound by default (assignable) | unbound by default | Editor | v1 |
| Toggle hover skim (TL-014) | `S` | `S` | Editor | v1 |
| Toggle audio scrubbing (AU-016) | `Shift+S` | `⇧S` | Editor | v1 |
| Zoom timeline in / out | `Ctrl+=` (also `Ctrl+Num+`) / `Ctrl+-` | `⌘=` / `⌘-` | Editor | mvp |
| Zoom timeline to fit | `Shift+Z` | `⇧Z` | Editor | mvp |
| Scroll timeline view one page left / right | `Page Up` / `Page Down` | `Page Up` / `Page Down` (`fn↑` / `fn↓`) | Timeline | v1 |
| Scroll tracks one page up / down | `Shift+Page Up` / `Shift+Page Down` | `⇧Page Up` / `⇧Page Down` | Timeline | v1 |
| Reverse, Flip horizontal / vertical, Rotate 90°, Crop, Speed…, Mute clip, Record voiceover, Record camera (PM-043), Record screen (PM-115), Select linked clips together (TL-057), Delete markers on this clip / Delete all timeline markers (TL-037) | unbound by default (assignable) | | | — |

*Player*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| Zoom preview in / out (EX-009) | `Ctrl+=` / `Ctrl+-` | `⌘=` / `⌘-` | Player | v1 |
| Zoom preview to fit (EX-009) | `Shift+Z` | `⇧Z` | Player | v1 |
| All guides off (EX-011) | `Ctrl+'` | `⌘'` | Editor | v1 |

*Playback & navigation*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| Play / pause | `Space` | `Space` | Editor (incl. asset panel) | mvp |
| Shuttle backward / stop / forward | `J` / `K` / `L` | `J` / `K` / `L` | Editor | v1 |
| Previous / next frame | `Left` / `Right` | `←` / `→` | Editor | mvp |
| Back / forward 10 frames (TL-013) | `Shift+Left` / `Shift+Right` | `⇧←` / `⇧→` | Editor | mvp |
| Previous / next edit point | `Up` / `Down` | `↑` / `↓` | Editor | mvp |
| Go to start / end | `Home` / `End` | `Home` / `End` (`fn←` / `fn→`) | Editor | mvp |
| Set range in / out (TL-038) | `I` / `O` | `I` / `O` | Editor | v1 |
| Clear range | `Alt+X` | `⌥X` | Editor | v1 |
| Play range (EX-008) | `Shift+Space` | `⇧Space` | Editor | v1 |
| Toggle loop playback (EX-008) | `Ctrl+Shift+L` | `⇧⌘L` | Editor | v1 |
| Mute preview audio (EX-019) | unbound by default (assignable) | unbound by default | Editor | v1 |
| Go to timecode… | `Ctrl+J` | `⌘J` | Editor | v1 |
| Full-screen preview | `Ctrl+Shift+F` | `⇧⌘F` | Editor | v1 |
| Use proxies on / off (PM-081) | `Alt+P` | `⌥P` | Editor | v1 |
| Close popup / cancel / exit full screen | `Esc` | `Esc` | Global | mvp |

*Panels & view*

| Action | Windows / Linux | macOS | Scope | Priority |
|---|---|---|---|---|
| Focus asset panel / player / inspector / timeline | `Ctrl+1` … `Ctrl+4` | `⌘1` … `⌘4` | Global | v1 |
| Show / hide asset panel; show / hide inspector (UI-002) | unbound by default (assignable) | unbound by default | Editor | mvp |
| Cycle panel focus forward / back | `F6` / `Shift+F6` | `F6` / `⇧F6` | Global | v1 |
| Maximize / restore focused panel (UI-048) | `` ` `` | `` ` `` | Editor | v1 |
| Next / previous tab in focused panel | `Ctrl+Tab` / `Ctrl+Shift+Tab` | `⌃⇥` / `⌃⇧⇥` | panel | v1 |
| Open context menu for focused item | `Shift+F10`, `Menu` key | — (`⌃`-click / two-finger click) | Global | v1 |

*Asset panel (when focused)*

| Action | Windows / Linux | macOS | Priority |
|---|---|---|---|
| Move selection | arrow keys | arrow keys | mvp |
| Add selected item (TL-012 *Add to main track* for media) | `Enter` | `↩` | mvp |
| Add as overlay at playhead (TL-012) | `Shift+Enter` | `⇧↩` | mvp |
| Toggle favorite | `F` | `F` | v1 |
| Remove media from project | `Del` | `⌫` | mvp |

*Modifier gestures* (held during a pointer gesture; rebindable to another
modifier only, UI-032)

| Gesture | Windows / Linux | macOS | Owner | Priority |
|---|---|---|---|---|
| Suspend snapping while dragging | hold `Ctrl` | hold `⌘` | TL-028, TL-018 | mvp |
| Overwrite on release (overlay tracks) | `Alt` at release | `⌥` at release | TL-018 | v1 |
| Add to / remove from selection | `Ctrl`+click | `⌘`+click | TL-015 | mvp |
| Range-select from the anchor clip | `Shift`+click | `⇧`+click | TL-015 | mvp |
| Select one member of a group | `Alt`+click | `⌥`+click | TL-015 | v1 |
| Constrain an on-canvas move to horizontal or vertical | hold `Shift` | hold `⇧` | TR-007 | mvp |
| Coarse / fine step while scrubbing a field | hold `Shift` (×10) / `Alt` (×0.1) | hold `⇧` / `⌥` | UI-009 | mvp |
| Pan the preview | hold `Space` and drag | hold `Space` and drag | EX-009 | v1 |
| Copy while dragging | none by default | none by default | — | later |

Reference: the observed reference app lets users bind play/pause, J-K-L
shuttle, entering and leaving full screen, 1- and 10-frame steps, player zoom
in / out / fit, showing guides and rulers, timeline zoom in / out / fit,
timeline scrolling in four directions, showing the media panel, import and
its recorder, plus modifier gestures for drag-to-duplicate, axis lock while
moving and a large numeric step. Every one of them has a row above.

**Behavior / output:**
- **J/K/L:** speeds, direction changes and the `K`+`J`/`L` frame-step chords
  are EX-007's; shuttle audio is AU-016's. This table only binds the keys.
- **Previous/next edit point** visits TL-013's cut points.
- **Split at playhead** splits the selected clips, or, with nothing selected,
  the main-track clip under the playhead; *Split all* cuts every clip under
  the playhead on unlocked tracks (both TL-023).
- **Esc order:** close the topmost popup/menu/hint (UI-058) → cancel the
  current drag or gesture → exit full-screen preview (EX-010) → leave media
  preview mode (EX-015) → leave a text field (reverting) → clear the
  selection.
- **Zoom keys follow focus:** with the Player focused, `Ctrl+=` / `Ctrl+-` /
  `Shift+Z` zoom the preview (EX-009); anywhere else in the editor they zoom
  the timeline (TL-041). This is UI-030's focused-panel rule, the same one
  that makes `Del` remove media in the asset panel.
- **Page keys scroll, arrows move the playhead:** `Page Up` / `Page Down`
  move the timeline view by one view width without moving the playhead;
  the scroll math (limits, padding) is TL-041's.
- **Why more commands are unbound by default:** the panel show/hide toggles
  have the View menu, the panel chevrons and `` ` `` (maximize); *Copy style*
  and *Paste style* are one click away through *Paste attributes…*
  (`Ctrl+Alt+V`, TL-034) and the text clip's context menu; the recorders are
  rare actions with their own buttons. All are assignable (UI-032).
- **The `N` family** is how the timeline reacts to edits: `N` snapping,
  `Shift+N` magnetic main track, `Alt+N` *Follow main track*. The **`M`
  family** is markers: `M` add, `Alt+M` add with the next palette colour,
  `Shift+M` / `Ctrl+Shift+M` next / previous.
- **Why two toggles are unbound by default:** *Ripple all tracks* changes
  what every later ripple edit does, so it should not flip from a stray key;
  it is one click away in the toolbar's `⋯` menu (UI-011) and the Timeline
  menu (UI-012). *Mute preview audio* has the player's speaker button (EX-019)
  and the Playback menu; the `M` chords are kept for markers, and `⌘M` is
  reserved by macOS. Either can be given a key in the shortcut editor
  (UI-032).
- **Superseded proposals.** Earlier drafts of other specs proposed keys that
  collided with or differed from this table. Those specs now say "see
  UI-031", and these rows apply:

  | Command (owner) | Earlier proposal | Problem | Default here |
  |---|---|---|---|
  | Export current frame (EX-014) | `Ctrl+Shift+S` | Save as | `Ctrl+Shift+E` |
  | Mute preview audio (EX-019) | `Ctrl+Shift+M` | previous marker | unbound by default |
  | Loop / play range (EX-008) | `\` / `Shift+\` | differed from this table | `Ctrl+Shift+L` / `Shift+Space` |
  | Magnetic main track (TL-005) | `P` | differed from this table | `Shift+N` |
  | Follow main track (TL-029) | `` ` `` | maximize panel (UI-048) | `Alt+N` |
  | Enable / disable clip (TL-036) | `V` | differed from this table | `Shift+E` |
  | Previous marker (TL-037) | `Alt+Shift+M` | differed from this table | `Ctrl+Shift+M` |
  | Detach / restore audio (TL-030) | `Ctrl+Shift+S` | Save as | `Ctrl+Shift+D` |

**Edge cases:**
- `Shift+,` produces `<` on US layouts: matched by physical key (UI-030).
- Keyboards without `Home`/`End`/`F`-keys (Mac laptops): `fn` combinations,
  or the command palette; every action is also in a menu.
- A command whose feature isn't built yet has no binding and is absent from
  the cheat sheet.
- On the macOS US layout `⌥N` is a dead key (it starts a tilde) and `⌥M`
  types `µ`. Outside text fields both are matched as Option + N / M through
  the physical-key fallback (UI-030), so the commands fire; inside a text
  field the field gets the character (UI-030's text-fields-first rule).
- An unbound-by-default command (*Ripple all tracks*, *Mute preview audio*)
  shows no shortcut label in menus and tooltips, appears in the shortcut
  editor with an empty chip, and is still reachable from the command palette
  (UI-034). Assigning it a key follows UI-032's conflict rules.
- `Alt+N` / `Alt+M` on Windows and Linux: FreeCut draws its own menus, so a
  plain `Alt`+letter never opens a menu mnemonic and reaches the dispatcher.
- Player focused and the user wants the timeline to fit → click the timeline
  (or `Ctrl+4`) first, or use the toolbar's *Zoom to fit*; the tooltip of the
  player's zoom menu names both keys.
- `Ctrl+'` on layouts where the apostrophe needs `Shift` or `AltGr` →
  matched by physical key (UI-030).
- Keyboards without `Page Up` / `Page Down` (Mac laptops) → `fn↑` / `fn↓`,
  which the OS reports as the same keys.
- A modifier gesture rebound to the modifier another gesture uses in the same
  place (e.g. snapping suspend moved to `Alt`, which is overwrite on
  release) → refused as a conflict by UI-032.

**Undo:** n/a (bindings); commands push their own ops.

**Model notes:** the table is the `default_keymap()` constant in
`freecut-app::commands`; one test renders it to Markdown and compares it with
this section's binding tables (not the *Superseded proposals* table) so the
spec and the code stay in sync. UI-030's no-collision test
runs over every row on all three platforms, including the rows for *Follow
main track*, *Ripple all tracks*, *Mute preview audio* and *Add marker with
next colour*. A second test requires each of those four commands to be present
in `default_keymap()`, either bound or registered with an explicit empty
binding list, and pins the contested chords in the table above to the
commands this spec gives them (`Ctrl+Shift+S` Save as, `Ctrl+Shift+M` previous
marker, `` ` `` maximize panel, `Shift+N` magnet, `Shift+E` enable/disable,
`Ctrl+Shift+L` loop, `Shift+Space` play range, `Alt+N` *Follow main track*),
so a later change can't quietly bring an old proposal back. The collision
test allows a panel-scoped binding to shadow an Editor binding only from an
explicit list (today: `Del` in Assets, and `Ctrl+=`, `Ctrl+-`, `Shift+Z` in
Player). The modifier-gesture table is part of `default_keymap()` (as
`GestureModifier` entries) and is included in the Markdown comparison.

*FreeCut decision (review fixes, 2026-10-13):* the File menu shows the
shortcut of every command that has one: New Project `⌘N`, Open `⌘O`, Import
Media `⌘I`, Save `⌘S`, Save As `⇧⌘S`, Export `⌘E`, Preferences `⌘,` (`Ctrl`
on Windows and Linux). Global shortcuts stand back while a control has
keyboard focus and while a list, menu or popup is open, since an open list
uses `Space`, `Home`, `End` and the arrows itself. A select gives focus back
to its trigger only when it closes from the keyboard (`Esc`, or `Enter` /
`Space` picking); a pointer pick or a click outside leaves the keyboard to
the app's shortcuts. A save that finishes after another project was opened
only shows its toast ("Saved A.freecut" or "Couldn't save A.freecut: …",
without Retry): it never sets the open project's file, save point or
"Save failed" state.

---

### UI-032 Shortcut editor & keymaps — v1

**What the user sees:** Preferences ▸ Keyboard shortcuts: a searchable list of
all commands grouped by category. Each row shows the action, up to two binding
chips, a **+** to add one, and ↺ when changed. At the top: *Keymap* dropdown,
*Search by key* toggle, *Import…*, *Export…*, *Reset all*.

**Inputs / controls:**
- Click a chip → "Press keys…"; the next chord is recorded. `Esc` cancels
  recording, `Backspace` clears the chip.
- Search by name (localized and English) or, with *Search by key*, by pressing
  a chord to see what it does.
- Keymap base: **FreeCut default**; later also an *alternative single-key
  layout* and a *classic NLE layout* (descriptive names; no preset is labelled
  with another product's name).
- *Show conflicts* filter: lists only rows marked ⚠ (below).
- Modifier gestures (UI-031's table) appear in their own group; their chip
  offers only a modifier choice (`Ctrl`/`⌘`, `Alt`/`⌥`, `Shift`/`⇧`), never
  a main key.
- *Hold* bindings (later): a chip can be marked *Hold* so the command runs
  while the key is held at least 300 ms (FreeCut default) and ends on
  release — for momentary commands such as "Blade tool while held". A tap
  and a hold of the same key may run different commands.

**Behavior / output:**
- Conflicts within overlapping scopes are shown inline ("Already used by
  Split — Replace / Cancel"); *Replace* moves the chord and leaves the other
  command unbound; *Cancel* leaves both commands unchanged.
- Conflicts that arrive without a recording — an imported keymap, or a new
  default in a FreeCut update that a user override now collides with — are
  marked ⚠ on both rows and counted at the top ("2 conflicts"). Until they
  are fixed, the chord runs the command in the narrower scope, and between
  equal scopes the user's own override wins over a default. FreeCut holds
  back no chords for future versions (the observed editor does); a later
  default that meets a user override simply shows up here as a conflict.
- OS-reserved chords (UI-030) are refused with a message.
- Recording refuses a key FreeCut can't identify (the platform reports no
  usable key, as with some vendor media keys): "This key can't be used for
  shortcuts". It also refuses chords with more than three modifiers: "Use at
  most three modifier keys" (FreeCut default).
- Every command can be left unbound except *Close popup / cancel* (`Esc`),
  which always keeps at least one binding, and modifier gestures, which
  always keep a modifier. Reference: the observed shortcut editor fixes the
  main key of some commands and forbids unbinding others (adding a keyframe
  from the keyframe panel, timeline zoom); FreeCut keeps those reachable from
  menus and the palette instead.
- Changes apply instantly; tooltips, menus and the cheat sheet update. There
  is no separate Save step, so closing the editor never asks to discard.
  Reference: the observed editor collects changes until the user saves and
  asks before discarding unsaved ones.
- Only overrides are stored (`keymap.json`: base keymap id + changed
  commands), so new commands in later versions get their defaults.
- Export/import writes/reads that JSON file. It is the local way to carry
  shortcuts to another computer (a synced folder, dotfiles); there is no
  account sync.

**Edge cases:**
- Binding a bare letter to a command → allowed; the field-first rule (UI-030)
  keeps typing safe.
- Imported keymap with unknown command ids → ignored with a summary.
- Imported keymap that conflicts with OS-reserved chords on this platform →
  those bindings are dropped with a summary.
- Chords that only exist on some layouts (e.g. `Alt+[` on German, where `[`
  needs `AltGr`) → physical-key matching, plus a hint to rebind.
- Recording a chord that text fields claim (`Ctrl+A/C/X/V/Z`, UI-030) for a
  command → allowed, with a note that a focused text field still gets it
  first.
- `Backspace` on the only binding of *Close popup / cancel* → refused with
  "This command needs a key".
- Imported keymap with a modifier gesture bound to a main key → that entry
  is dropped with a summary.

**Undo:** n/a; ↺ per row and *Reset all* (with confirmation).

**Model notes:** `freecut-app::commands::{Keymap, KeymapOverrides}`.

---

### UI-033 Shortcut cheat sheet — v1

**What the user sees:** `Ctrl+/` (or the keyboard button in the top bar) opens
an overlay of the most-used shortcuts in columns (Playback, Editing, Timeline,
File, Panels), showing the user's current bindings, with a search field and a
link *Edit shortcuts…*.

**Inputs / controls:** search; *All commands* toggle (off: most-used only);
`Esc` or the same chord closes it.

**Behavior / output:** Generated from the registry, with platform-specific
labels; the most-used view leaves unbound commands out. With *All commands* on, the overlay becomes the
complete reference list: every command in the build, grouped by category,
including unbound ones (shown with "—") and the modifier gestures. The
toggle's state is remembered (workspace state). Reference: the observed app
has a panel listing every command with its current binding.

**Edge cases:**
- Very long localized labels wrap; the overlay scrolls on small windows.
- A command rebound while the overlay is open (from the editor link) → the
  overlay refreshes when it regains focus.
- Search with no matches → "No shortcuts match" and a link to the editor.

**Undo:** n/a.

**Model notes:** `freecut-app::commands::cheat_sheet`.

---

### UI-034 Command palette — v1

**What the user sees:** `Ctrl+Shift+P` opens a search box at the top of the
window. Typing lists matching commands with their shortcut and category;
unavailable ones are greyed with their reason. It also finds settings ("Go to
setting: Autosave"), layouts, and panels.

**Inputs / controls:** type; `Up`/`Down` to choose; `Enter` runs; `Esc` closes.

**Behavior / output:** Fuzzy matching on localized and English labels plus
aliases (e.g. "cut" also finds Split). The five most recent commands are shown
first when the box is empty. Running a disabled command shows its reason.

**Edge cases:**
- Opening the palette while a text field has focus moves focus to the palette
  and back afterwards (the field's value is committed first).
- Commands that need a target (e.g. Rename) act on the selection that existed
  when the palette opened.
- The selection was deleted (e.g. by undo) while the palette was open → the
  command shows its disabled reason instead of running.

**Undo:** n/a; commands push their own ops.

**Model notes:** `freecut-app::commands::palette`.

---

### UI-035 Context menus — mvp

**What the user sees:** Right-click (or `Ctrl`-click / two-finger click on
macOS, `Shift+F10` / Menu key on the focused item) opens a menu of commands
for that item, with shortcuts right-aligned and disabled items explaining why
on hover.

| Target | Menu items (FreeCut default) |
|---|---|
| Timeline clip | Cut, Copy, Paste, Paste attributes… (TL-034), Duplicate, Delete, Ripple delete · Split at playhead · Enable/disable, Mute audio · Detach / restore audio · Speed ▸, Reverse, Freeze frame · Group / Ungroup, Create compound clip · Replace media…, Reveal in media panel, Reveal in file manager · Rename |
| Several clips | the subset valid for all, plus "Apply to N clips" wording |
| Empty track area | Paste here, Add marker here, Close gap (when on a gap), Select all clips after this point, Zoom to fit |
| Track header | Rename, Lock/unlock, Hide/show, Mute/unmute, Add track above / below, Delete track, Track color ▸ |
| Time ruler | Add marker, Set range in here / out here, Clear range, Zoom to fit |
| Marker | Rename, Color ▸, Delete, Delete all markers |
| Keyframe | Copy, Paste, Delete, Easing ▸ (owned by TR-031, TR-033) |
| Media item | Add at playhead, Preview, Rename, Relink… (offline, PM-091), Create / delete proxy (PM-080), Find on timeline (PM-055), Copy file path, Reveal in file manager, Properties (PM-059), Remove from project (PM-058) |
| Library item (effect, filter, sticker…) | Apply to selected clip, Add at playhead, Add to / remove from favorites |
| Player | Preview resolution ▸ (EX-006), Use proxies (PM-081), Show safe areas, Show grid (EX-011), Zoom ▸ (EX-009), Export frame… (EX-014), Set current frame as cover (PM-019), Full-screen preview (EX-010) |
| Text field | Cut, Copy, Paste, Select all |
| Home project card | PM-001's mvp items (Open, Reveal in file manager, Remove from list); PM-018's from v1 (UI-017) |

Items appear only once their command exists in the build, so the mvp menus
hold just the mvp commands (for a timeline clip: Delete, Ripple delete, Split
at playhead, Mute audio; for a track header: Hide/show and Mute/unmute
(TL-008), Add track above / below and Delete track (TL-006). Lock/unlock
arrives with TL-009 and Rename / Track color with TL-010, both v1).

FreeCut default (mvp build): the timeline-clip menu holds *Split at playhead*, *Delete*, *Mute
audio* (or *Unmute audio*, by AU-007's mixed-selection rule) and *Detach audio* / *Restore
audio* (TL-030). v1 adds *Resync* while a selected clip is out of sync (TL-030), and the
submenus *Sound source ▸* and *Add stream as clip ▸* for files with several audio streams
(AU-038). Right-clicking an unselected clip selects it first; with `Cmd`/`Ctrl` held it
is added to the selection. The media panel's menu on any item holds *Add at playhead* (UI-005's
add: a picture or video on the main track at the playhead, sound on the first free audio track
there), *Extract audio* on video items only (AU-004; disabled with "This video has no sound" when
there is none) and *Copy file path* (the file where it was found on this machine, else the stored
path). A row of the Audio panel has *Add at playhead* and *Copy file path*. Extracting switches to
the Audio tab with the Extracted chip on and the new (or already existing) item highlighted.

Owning specs may add pre-filtered entries to the timeline-clip menu that open
TL-034's *Paste attributes* dialog with only their own categories ticked (for
example *Copy color* / *Paste color*, CL-025). They are ordinary commands in
the registry and share TL-034's attribute clipboard; they never keep a
clipboard of their own.

**Tools ▸ submenu (v1):** the timeline-clip menu also gathers the one-click
tools that suit the clicked clip's kind, each an ordinary registry command
supplied by its owner and listed only once it exists in the build — for
example *Detect beats* (AU-025), *Auto color* (CL-019), *Remove silences*
(AI-003), *Detect scenes and split* (AI-004), and later *Auto captions*
(AI-010), *Remove background* (AI-014), *Auto reframe* (AI-016), *Smooth
slow motion* (AI-018) and *Enhance* (AI-019). Each item opens or runs the
owner's own tool exactly as its main entry point does. Reference: the
observed app gathers similar quick tools (color adjust, beat marking,
copy/paste attributes, enhancement, auto captions, background removal,
text-to-speech, pause removal) in a per-clip "more" menu, many of them paid
or server-based; FreeCut lists only local tools.

**Inputs / controls:** pointer or keyboard; arrows move, `Right` opens a
submenu, `Enter` runs, `Esc` closes. Submenus open after 200 ms hover.

**Behavior / output:** Right-clicking an unselected item first selects it
(replacing the selection unless `Ctrl`/`⌘` is held); right-clicking an item
that is part of the selection keeps the whole selection. Menus are built from
the registry, so they never offer an action the shortcut wouldn't allow.

**Edge cases:**
- Right-click on a clip on a locked track → editing items disabled, "Unlock
  track" offered at the top.
- Track header menu → *Hide/show* appears only on tracks with visual content
  and *Mute/unmute* only on tracks that carry audio, matching the header icons
  (TL-008); a video track with embedded audio shows both. Whether a toggle is
  allowed on a locked track is TL's rule (TL-008, TL-009); the menu only shows
  the command's availability result.
- Menu near a screen edge → flips to stay on screen.
- The model changes while a menu is open (autosave reload, recovery) → the
  menu closes.
- Right-click during playback → playback continues; destructive items apply at
  the frame shown when chosen.
- *Tools ▸* with no tool available for the clip kind (e.g. a text clip) → the
  submenu is hidden, not shown empty. A tool whose model isn't installed is
  listed with "(needs download)" and opens AI-002's prompt.

**Undo:** each item's own op; the menu itself is n/a.

**Model notes:** `freecut-app::context_menu::{MenuTarget, build_menu(target,
&AppState)}`.

---

### UI-036 Drag & drop from the operating system — mvp

**What the user sees:** Files dragged from Finder / Explorer / file managers
can be dropped on the Home screen, the asset panel, the timeline or the
player. While hovering, the target highlights and a hint describes what will
happen ("Import 3 files and add at 00:00:12:05").

**Inputs / controls:** drag and drop.

**Behavior / output:** What a drop does is owned elsewhere; this area provides
the hover highlight, the hint text and the routing:

| Dropped on | Result | Owner |
|---|---|---|
| Home | a `.freecut` file opens that project; media files start a new project (PM-002) and import them | PM-031, PM-013 |
| Asset panel | import into the project's media | PM-031 |
| Timeline | import, then place at the drop point | PM-031, TL-012 |
| Player | import into the media panel | PM-031 |

- Order of several files, folder handling (a toast in the mvp; PM-032 in v1)
  and the import report are PM-031's.
- Special types are routed to their owners: `.srt` / `.vtt` → captions
  (`text-and-captions.md`); `.cube` → LUT import (`color-and-filters.md`);
  `.ttf` / `.otf` → font import (`text-and-captions.md`); `.freecut` inside
  the editor → open it, after UI-025's prompt if needed.
- Probing and import happen in the background; the hint switches to a
  progress note in the status line (UI-040).

**Edge cases:**
- Unsupported files, URLs dragged from a browser, promised files, Wayland
  sessions without file drops → PM-031's handling; the hover hint already
  says "Can't import this" where the type is known before the drop.
- Drop on a locked track → TL-012 redirects it to a new track; the hint says
  so before release.
- Drop during playback → playback continues.
- Drop while an export runs → allowed (export renders from its own snapshot).
- Drag leaves the window without dropping → highlight and hint cleared.

**Undo:** per owner: a panel drop is PM-031's `ImportMedia`; a timeline drop is
one compound entry (import plus TL-012 placement, labelled "Add media").

**Model notes:** egui `HoveredFile` / `DroppedFile` in `freecut-app::dnd`,
handed to PM-031's `ImportQueue`; placement via TL-012's planner.

---

### UI-037 In-app drag & drop — mvp

**What the user sees:** Items can be dragged from the asset panel to the
timeline, the player or onto a clip, with a ghost preview showing exactly
where the item will land.

**Inputs / controls:**

| Drag | Drop target | Result (owner) | Priority |
|---|---|---|---|
| Media / text card | timeline track or new-track zone | place per TL-012 (TX-001 for text) | mvp |
| Element card | timeline track | place per OV-036 | v1 |
| Media card | player | add per TL-012's *Add to main track* | mvp |
| Transition card | a cut between two clips | apply transition (FX-002) | mvp |
| Effect / filter / animation card | a clip | apply to that clip (FX-010, CL, FX-018) | v1 |
| Effect / filter card | empty track area | create an effect / filter layer (FX-011, CL) | v1 |
| Track header | another track position | reorder tracks (TL-007) | v1 |
| Effect row in inspector | another row | reorder the effect stack (FX-013) | v1 |

During a drag: `Esc` cancels; holding `Ctrl`/`⌘` suspends snapping (TL-028).
Moving clips inside the timeline, including `Alt`/`⌥` on release to
overwrite, follows TL-018.

**Behavior / output:**
- The ghost is computed by the owner's pure planner on every pointer move
  (TL-012's `plan_insert`), so ghost and result always agree; it shows the
  item's length at the current zoom and a tooltip with the drop timecode and
  track name; the target track highlights.
- Near the timeline's left/right edges the view auto-scrolls, faster the
  closer the pointer is (up to two screen widths per second); near top/bottom
  it scrolls tracks.
- If the planner refuses, the cursor shows "not allowed", the ghost turns
  red and its tooltip gives the reason; releasing there does nothing.
- After a successful drop the new item is selected.

**Edge cases:**
- Drop on a locked track → TL-012 redirects it to a new track; the ghost
  shows the redirected position before release.
- Audio-only media over a video track → the ghost shows where TL-012 puts it
  (an audio track).
- Transition over a clip edge with no neighbour → refused with FX-002's
  reason.
- Drop past the end of the timeline → allowed per the owner; the timeline
  grows.
- Window loses focus mid-drag → drag cancelled, nothing changes.

**Undo:** one entry per drop, owned by the op performed (insert, apply effect,
reorder).

**Model notes:** egui drag-and-drop payloads carrying `AssetRef` (a project
media id, a bundled item id, or a user file path plus content hash); pointer
handling in `freecut-app::dnd`; planning and ops in the owning crates.

---

### UI-038 Mouse wheel, trackpad & pointer conventions — mvp

**What the user sees:** Predictable scrolling and zooming in every panel.

**Inputs / controls:**

| Where | Input | Effect | Owner |
|---|---|---|---|
| Timeline | wheel | scroll tracks vertically | TL-041 |
| Timeline | `Shift`+wheel, or horizontal trackpad swipe | scroll time horizontally | TL-041 |
| Timeline | `Ctrl`/`⌘`+wheel, or pinch | zoom around the pointer | TL-041 |
| Timeline | middle-button drag | pan | TL-041 |
| Timeline | `Alt`/`⌥`+wheel | height of the hovered track (v1) | TL-010 |
| Timeline ruler | click / drag | move / scrub the playhead | TL-013 |
| Player | `Ctrl`/`⌘`+wheel or pinch | zoom the preview (v1) | EX-009 |
| Asset grid | wheel | scroll the grid | this area |
| Asset grid | `Ctrl`/`⌘`+wheel | thumbnail size (v1) | UI-006 |
| Inspector | wheel | scroll the panel; never changes an unfocused field | this area |
| Any dropdown | wheel while closed | nothing (no accidental value change) | this area |

**Preferences ▸ Mouse & trackpad (v1)** changes the timeline rows above
(keys in UI-018's table):
- *Mouse wheel over the timeline*: **Scroll tracks** (default) / Scroll time
  / Zoom time. Whichever action the plain wheel takes, the one it replaced
  moves to `Ctrl`/`⌘`+wheel, so all three stay reachable (`Shift`+wheel stays
  horizontal scrolling).
- *Pinch to zoom* (on): trackpad pinch zooms the timeline and the player.
- *Middle-button drag pans* (on).

Reference: the observed app has a mouse settings section next to its
shortcut settings; its options were not visible, so these three are FreeCut's
choice.

**Behavior / output:** Zoom anchors and limits are TL-041's. This area makes
input consistent: high-resolution wheel and trackpad deltas are accumulated
before they become scroll steps, trackpad inertia is honoured, and wheel
events go to the panel under the pointer, not the focused one.

**Edge cases:**
- Fractional deltas from precision wheels and trackpads → accumulated, no
  jitter and no lost steps.
- At a zoom limit (TL-041) further input does nothing; no bounce animation.
- Wheel over a field that has keyboard focus → changes its value (UI-009);
  over any other field → scrolls the panel.
- Inertial scroll still running when the pointer crosses into another panel
  → the inertia stays with the panel where it started.
- *Mouse wheel over the timeline* set to Zoom time and a trackpad's
  two-finger vertical swipe → treated as wheel input, so it zooms too; the
  setting's help line says so.
- *Pinch to zoom* off → pinch does nothing (it is not turned into scrolling).

**Undo:** n/a (view state).

**Model notes:** `freecut-app::input`; floats only for screen deltas, converted
to rational time at the timeline's view mapping.

---

### UI-039 Tooltips — mvp

**What the user sees:** Hovering any icon or control for 500 ms shows a small
label with the action's name and shortcut ("Split  Ctrl+B" / "Split  ⌘B").
Disabled controls also say why ("Select a clip to split"). Moving to a
neighbouring control within 300 ms shows its tooltip immediately.

**Inputs / controls:** hover; keyboard focus shows the same tooltip after
500 ms; Preferences ▸ *Show tooltips* (on).

**Behavior / output:** Text comes from the command registry, so rebinding a
shortcut updates every tooltip. Tooltips never cover the control itself and
flip at screen edges. Long help (e.g. on Preferences rows) wraps at 320 pt.

**Edge cases:**
- Tooltips are suppressed during drags and playback scrubbing.
- With tooltips off, disabled reasons still appear in the status line on
  click.
- Truncated labels (names, tabs) always get a tooltip with the full text, even
  with tooltips off.

**Undo:** n/a.

**Model notes:** egui `on_hover_text` wrappers in `freecut-app::widgets`; the
same text is exposed to assistive technology (UI-042).

---

### UI-040 Notifications & background tasks — mvp

**What the user sees:** Small toasts in the bottom-right of the window, and a
task indicator in the status line ("Processing 12 files…", "Exporting 62 %").
This area owns both the indicator and the toasts; no other area draws its own.
Clicking the indicator opens the app's single background-task list, which
PM-073 owns (its rows, row actions and *Cancel all*). Rows come from PM-073's
media jobs, from AI-001's analysis jobs (v1) and, from v1, from EX-038's export
queue, each with progress, time remaining and the row actions its owner
defines (Pause, Cancel, Retry).

AI-001 adds no indicator, list or toast style of its own. It defines only its
job kinds (which become the row labels, e.g. "Detect scenes"), the per-clip
progress strips and result badges drawn on timeline clips, and the analysis
pool's concurrency settings, which UI-018 hosts under Preferences ▸ Smart
tools.

**Inputs / controls:** close (×) on toasts; action buttons inside toasts
(*Retry*, *Relink…*, *Show details*, *Copy*); row actions in the task list.

**Behavior / output:**

| Kind | Auto-dismiss | Example |
|---|---|---|
| Info | 4 s | "Snapping off" |
| Success | 3 s | "Export finished — Show in folder" |
| Warning | 8 s | "3 files were skipped" |
| Error | stays until closed | "Save failed — Retry / Save as…" |

- At most three toasts at once; the rest queue. Hovering pauses the timer.
- **Indicator text** sums up every running row, whatever its source (FreeCut
  default): rows of one kind show that kind's summary ("Processing 12 files…",
  "Transcribing 40 %"); mixed kinds show a running export's percent first when
  there is one, then a count ("Exporting 62 % · 3 more tasks"). A failed task
  turns the indicator red until the list is opened. With nothing running it
  shows nothing.
- Background work never blocks editing; job scheduling, priorities and
  throttling are PM-073's (AI-001's for its separate analysis pool, and
  EX-022 / EX-038's for exports). Finished tasks stay in the list for the
  session.
- Other areas raise toasts through one API with a kind, a message id and
  optional actions; they never draw their own.
- During full-screen preview only errors are shown.
- **Error codes** (v1): every error toast and error dialog carries a short
  code derived from the error type that caused it (for example `EXP-0012`),
  shown in small type with *Copy details* (message, code, time, the
  operation that failed). The local guide's troubleshooting page (UI-051) is
  indexed by these codes, and crash and diagnostic reports include them
  (UI-023, UI-063). An error with no specific type shows "Something went
  wrong" with the code `GEN-0000` and *Open logs folder*. Codes are stable
  across versions and never reused, like feature IDs.

**Edge cases:**
- The same message repeated → collapsed into one toast with a counter.
- Cancelling a task → the owner cleans up partial output (PM-073 for media
  jobs, EX-022 for exports); the row shows "Cancelled".
- A task fails after the project was closed → reported on Home as a toast.
- An analysis job and a media job for the same clip run at once → two rows in
  the one list and one indicator; the clip's progress strip on the timeline
  is AI-001's and is not repeated in the status line.
- An analysis job falls back from GPU to CPU (AI-001) → AI-001 raises a
  warning toast through this API; the row stays in PM-073's list.

**Undo:** n/a.

**Model notes:** `freecut-app::notify::{Toast, ToastKind}` for toasts and
`freecut-app::notify::TaskIndicator` for the status-line summary; the task
list renders PM-073's `JobQueue` state, into which AI-001's analysis pool
forwards its rows. Progress arrives over channels from the media, render,
audio and analysis workers. Error codes: each library crate's `thiserror`
enums expose `fn code(&self) -> &'static str` with a crate prefix (no shared
crate is needed, so `freecut-timeline` keeps no internal dependencies); the
app maps codes to message ids, and a unit test per crate checks that no two
variants share a code.

---

### UI-041 Keyboard navigation & focus — v1

**What the user sees:** A visible 2 pt accent focus ring on the focused
control. Everything in panels, dialogs and menus can be reached and operated
without a mouse.

**Inputs / controls:** `Tab` / `Shift+Tab` move within the focused panel or
dialog; `F6` / `Shift+F6` move between panels (`Ctrl+1…4` jump directly, v1);
`Enter` / `Space` activate; arrows move inside grids, lists, tab rows and
segmented controls; `Esc` follows the order in UI-031.

**Behavior / output:**
- Keyboard-only timeline editing: target track (`Shift+Up/Down`), select clip
  under playhead (`Enter`), previous/next clip (`Alt+Left/Right`), nudge
  (`,`/`.`, TL-019), move between tracks (`Alt+Up/Down`), trim to playhead
  (`Q`/`W`, TL-026), split (`Ctrl+B`) — every clip drag operation gets a
  keyboard path. (The mvp already has click-to-focus, `Tab` inside dialogs
  and the mvp shortcuts of UI-031; this feature completes full keyboard
  operation.)
- Dialogs focus their first field (or the safest button) on open and return
  focus to where it was on close.
- Focus is never trapped except in modal dialogs.

**Edge cases:**
- Focus on a control that disappears (selection changed) → focus moves to the
  panel itself.
- Space while focus is on a button activates the button, not playback (button
  focus counts as a field for UI-030 purposes).

**Undo:** n/a.

**Model notes:** egui focus APIs plus a panel-level focus manager in
`freecut-app::shell`.

---

### UI-042 Screen-reader support — v1

**What the user sees:** Screen readers (Narrator / NVDA on Windows, VoiceOver
on macOS, Orca on Linux) can read and operate FreeCut's controls.

**Inputs / controls:** the screen reader's own keys plus FreeCut's keyboard
paths; command *Announce playhead position* (unbound by default).

**Behavior / output:**
- egui widgets are exposed through AccessKit (eframe's `accesskit` feature),
  which maps to UI Automation (Windows), NSAccessibility (macOS) and AT-SPI
  (Linux).
- Custom-painted widgets (timeline clips, track headers, keyframes, player
  transport, waveform) add their own accessibility nodes with role, name and
  state, e.g. "Video clip beach.mp4, track V1, 00:00:05:00 to 00:00:12:10,
  selected".
- Toasts are announced as polite live-region updates; errors as assertive.
- Playhead position is announced on request and after jumps, not continuously
  during playback.

**Edge cases:**
- Thousands of clips → only clips in or near the visible range are exposed,
  plus a summary node ("Track V1, 312 clips").
- Accessibility tree updates are throttled during playback.

**Undo:** n/a.

**Model notes:** AccessKit (Apache-2.0/MIT) via eframe; node building in
`freecut-app::a11y`.

---

### UI-043 Visual accessibility — v1

**What the user sees:** Readable contrast in every theme, information that
doesn't rely on colour alone, and an option to calm motion.

**Inputs / controls:** Appearance ▸ *Reduce motion*, *Theme* (High contrast),
*Interface size* (UI-015).

**Behavior / output:**
- Contrast: body text ≥ 4.5 : 1, large text and control boundaries ≥ 3 : 1;
  high contrast ≥ 7 : 1 for text.
- Never colour alone: clip types also carry an icon, keyframe states use shape
  (hollow / outline / filled), errors combine icon + text, toggles show on/off
  state in their icon.
- Clip colours are chosen to stay distinguishable for common colour-vision
  deficiencies (checked with a simulator in CI).
- Minimum pointer target 24 × 24 pt.
- Reduce motion: no panel animations, no autoplaying library loops, no
  smooth-scrolling animation; the playhead still moves (it is content).
- No UI element flashes more than three times per second.

**Edge cases:**
- User-chosen track colours → automatic label contrast (UI-013).
- Video content itself may flash; that is the user's media and out of scope.
- OS "reduce motion" setting on → FreeCut's *Reduce motion* defaults to on at
  first run (it can still be changed).

**Undo:** n/a.

**Model notes:** token tests in `freecut-app::theme`.

*FreeCut decision (S2 app, 2026-10-13):* *Reduce motion* is stored as
`appearance.reduce_motion` in `settings.json` (default off; following the OS setting at first
run comes later). In this build it stops preset tiles playing on hover (TX-022): they keep
their still. A hovered tile then asks for no further frames either, so resting the pointer on
it doesn't keep the window redrawing (review fixes, 2026-10-13).

---

### UI-044 String catalog & locale framework — mvp

**What the user sees:** Nothing new in the mvp (English only) — but every
visible string already comes from a catalog, so translations can be added
without code changes. It is mvp although it is not part of the core loop
because every UI string written before it exists would have to be reworked.

**Inputs / controls:** none (developer-facing).

**Behavior / output:**
- All user-facing text lives in Project Fluent files
  (`crates/freecut-app/locales/<lang>/*.ftl`), embedded at build time, read
  through `fluent-bundle` with `fluent-langneg` for language negotiation
  (both Apache-2.0/MIT). `en-US` is the source and the fallback.
- Message ids are stable and descriptive (`timeline-split`,
  `toast-autosave-failed`); plurals and variables use Fluent selectors.
- Fluent's bidi isolation marks around variables are turned off for
  left-to-right languages (egui would draw them as boxes) and handled
  explicitly once RTL arrives (UI-046).
- A pseudo-locale (accented, ~40 % longer, bracketed text) can be enabled in
  debug builds to spot hard-coded strings and truncation.
- A CI check fails on string literals passed directly to UI widgets outside
  the catalog helpers.

**Edge cases:**
- Missing message in a language → English text; debug builds log the id.
- Message with a wrong variable name → English fallback plus a log line,
  never a crash.
- Project content (clip names, text layers, file names) is never translated.

**Undo:** n/a.

**Model notes:** `freecut-app::i18n::{Localizer, tr!}`; bundled asset names
(effects, filters, transitions) are catalog ids, not hard-coded English.

---

### UI-045 Interface languages & translations — v1

**What the user sees:** Preferences ▸ General ▸ *Language*: "System default"
plus every language that ships, each shown in its own name ("Deutsch",
"日本語"). The change applies immediately, without restart.

Reference: CapCut's desktop app also offers its language choice under its
general settings and ships many interface languages; FreeCut aims for a broad
set as volunteer translations land. Its observed UI labels show a language
picker with a follow-the-system option, a combined language-and-region
setting, a prompt offering to switch when the OS language differs, about 21
language names (some lists may be caption languages rather than interface
languages), and a restart to apply the change on some platforms.

**Inputs / controls:** dropdown; Advanced ▸ *Show incomplete translations*.

**Behavior / output:**
- Initial target languages (shipped once ≥ 90 % translated): English, Spanish,
  Portuguese (Brazil), French, German, Italian, Russian, Ukrainian, Polish,
  Turkish, Indonesian, Vietnamese, Japanese, Korean, Chinese (Simplified),
  Chinese (Traditional), Kazakh. Below 90 % they appear only with the
  Advanced toggle. Further candidates, taken up when translators volunteer:
  Spanish (Latin America) as a variant, Filipino, Malay, Norwegian (Bokmål),
  Romanian, Slovak; Arabic, Thai and Tamil need UI-046's shaping and
  right-to-left support first.
- No restart is needed (above). Alternative observed: the reference app
  restarts to apply a language change on some platforms.
- **New OS language prompt:** when *Language* is set to a specific language
  and, at launch, the OS language has changed to a different shipped
  language, Home shows a one-time hint (UI-058): "Your system language is
  now Deutsch. Use it in FreeCut?" — *Switch* / *Not now*. It is asked once
  per OS language. With *System default* nothing is asked: FreeCut simply
  follows.
- There is no region setting: number and date formats follow the UI
  language. Content languages — a caption track's language, translation and
  speech targets — are TX's and AI's and independent of the UI language.
- Numbers and dates follow the language (decimal separator in fields, "2 h
  ago" phrasing); timecode stays `HH:MM:SS:FF` in every language.
- Modifier names are localized on Windows/Linux (UI-030).
- Fonts: the bundled UI font covers Latin, Greek and Cyrillic. For CJK, Thai
  and other scripts FreeCut loads a matching system font (via `fontdb`, MIT)
  as a fallback; packagers may bundle Noto Sans CJK (OFL-1.1). If no suitable
  font exists, the language is still selectable and a warning explains which
  font to install.
- Layout tolerates ~40 % longer strings; overflowing tabs and buttons
  ellipsize with a tooltip.

**Edge cases:**
- System language not shipped → English (or the closest shipped variant, e.g.
  `pt-PT` → `pt-BR`).
- Switching language with dialogs open → they re-render in the new language.
- Typed decimals: both `.` and `,` accepted in every language.
- New OS language not shipped, or below 90 % with the Advanced toggle off →
  no prompt.
- *Not now* → not asked again for that OS language; a later change to yet
  another language asks again.

**Undo:** n/a.

**Model notes:** translations are contributed as `.ftl` files; `settings.
general.language` holds a BCP 47 tag or `system`.

---

### UI-046 Right-to-left & complex-script UI — later

**What the user sees:** Arabic, Hebrew and Persian interfaces with mirrored
layout (asset panel on the right, inspector on the left, chevrons flipped) and
correctly shaped text; Devanagari, Thai and similar scripts shaped correctly
in the UI.

**Inputs / controls:** language choice.

**Behavior / output:** Requires bidirectional text and complex shaping in the
UI text stack (egui's current text layout is limited here); the timeline's
time axis stays left-to-right, as in other editors.

**Edge cases:**
- Mixed-direction strings (an English file name in an Arabic sentence) use
  isolation marks.
- Numbers and timecode stay left-to-right.
- Keyboard arrows in mirrored panels follow visual direction; timeline arrows
  keep time direction.

**Undo:** n/a.

**Model notes:** depends on egui/text-shaping progress; tracked separately.
User text inside *video* (titles) is a separate concern of
`text-and-captions.md`.

---

### UI-047 History panel placement — later

**What the user sees:** TL-044's edit history panel (labels, newest at the top,
greyed entries after the current point) opens from View ▸ *History* as an
auxiliary panel in the inspector slot (UI-056), so it can sit beside the
inspector without covering the timeline. A marker shows the last saved point.

**Inputs / controls:** View ▸ *History*; click an entry to jump (TL-044);
`Esc` returns focus to the timeline.

**Behavior / output:** Jump semantics, labels and dirty-flag updates are
TL-043/TL-044's. This area hosts the list, virtualizes it, scrolls to the
current entry after every edit, undo or redo, and remembers whether the panel
was open (workspace state).

**Edge cases:**
- A jump that takes longer than 200 ms → progress shown in the status line;
  input to the timeline is held until it finishes.
- Project closed with the panel open → the panel empties and closes; it
  reopens with the next project if it was open.
- Panel open in safe mode or on a read-only project → still shown; jumping is
  allowed (it only changes in-memory state).

**Undo:** uses the undo stack itself; the panel adds no entries.

**Model notes:** reads `ChangeSet` labels and the history cursor from
`freecut-timeline::history` (TL-043); hosting in `freecut-app::shell`.

---

### UI-048 Panel maximize & distraction-free mode — v1

**What the user sees:** `` ` `` (or double-clicking a panel's header)
maximizes the focused panel to fill the window; pressing it again restores the
layout. View ▸ *Distraction-free* hides the top bar, status line and side
panels, leaving player and timeline.

**Inputs / controls:** `` ` ``, header double-click, View menu.

Each panel header also has a small maximize button (⤢, turning into ⤡ while
maximized), so the gesture is discoverable without the key.

**Behavior / output:** Purely temporary: nothing is written to the layout until
restore. Shortcuts keep working; a small "Restore" button sits in the corner.

**Edge cases:**
- Maximizing the timeline keeps playback (and audio) running; there is just
  no visible frame.
- Closing the project while maximized restores the layout first.
- A dialog that needs a hidden panel (e.g. *Reveal in media panel*) restores
  the layout before acting.

**Undo:** n/a.

**Model notes:** a transient flag in `freecut-app::shell`.

---

### UI-049 First-run setup — v1

**What the user sees:** On the very first launch only, one short screen:
language (pre-filled from the OS), theme (pre-set to High contrast when the
OS reports a high-contrast mode, UI-014), default project folder (PM-020's
location default), and a hardware check summary — UI-061's quick check and
rating ("Graphics: Apple M2 (Metal) ✓ · Hardware decoding: available ✓ ·
Rating: Great"), with *Details* opening the full report. Buttons: *Start
editing* and *Skip*; from the tour's release (UI-059, later) also a *Show
me around* checkbox. No sign-in, no account, no survey. Reference: the
observed app's first run asks about the user's role, goals and content
genres to personalise recommendations; FreeCut asks nothing of the kind
(see Out of scope).

**Inputs / controls:** the fields above; Help ▸ *Run first-time setup again*.

**Behavior / output:** *Skip* keeps defaults. The hardware check runs in the
background and can't block *Start editing*. Problems found (no adapter with the
required features, no hardware decoding) are explained in plain words with a
link to the guide.

**Edge cases:**
- Config folder not writable → setup completes in memory and reappears next
  launch with a warning.
- Headless/remote desktop sessions with a software GPU → warning about
  performance; continue allowed.
- Chosen project folder not writable → inline error; the previous default is
  kept.

**Undo:** n/a.

**Model notes:** `settings.first_run_done` flag.

---

### UI-050 Settings export, import & reset — v1

**What the user sees:** Preferences ▸ Advanced: *Export settings…*, *Import
settings…*, *Reset all settings…*.

**Inputs / controls:** file dialogs; reset confirmation with checkboxes:
*Settings*, *Keyboard shortcuts*, *Layouts*, *Recent projects list* (runs
PM-017's *Clear list*), *Library favorites and recents* (first three checked
by default).

**Behavior / output:** Export writes one JSON file with settings, keymap
overrides and user layouts (never recent projects, paths to private folders are
included only if the user ticks *Include folder locations*). Import validates
the version, previews what will change, and applies it. Reset restores
defaults for the ticked groups; the previous files are kept as `*.bak`.
- *Import from another FreeCut installation…* takes a folder instead of a
  file — the config folder of a portable copy, of another release channel or
  of a backup — and reads its `settings.json`, `keymap.json` and
  `workspace.json` the same way. The preview lists the groups found and lets
  the user untick any.
- Import runs on the I/O thread with a progress note and *Cancel*; nothing
  is applied until it finishes. Imported values apply immediately, except
  rows tagged *Restart required*, which raise UI-018's *Restart now* banner
  (*Restart now* / *Later*).

Reference: the observed app can copy language and editing preferences from
a sibling installation of itself, with progress, cancel and a restart (now
or later) at the end.

**Edge cases:**
- Importing a file from a newer version → known keys applied, unknown ignored
  with a summary.
- Imported folder paths that don't exist on this machine → skipped.
- Import of a damaged file → refused, nothing changed.
- Imported keymap with chords reserved on this OS → dropped with a summary
  (UI-032).
- Chosen installation folder holds no FreeCut settings → "No FreeCut
  settings found in this folder"; nothing changes.
- Chosen folder is this installation's own config folder → refused.
- Import cancelled → nothing applied, no `*.bak` written.

**Undo:** n/a; `*.bak` copies allow manual rollback.

**Model notes:** `freecut-app::settings::{export, import, reset}`.

---

### UI-051 About, licenses & help — mvp

**What the user sees:** Help ▸ *About FreeCut*: name, version, commit, build
date, "Licensed under GPL-3.0-or-later", a link to the source code, and tabs
*Third-party licenses* (every Rust crate and its license), *FFmpeg* (version,
build configuration, license), *Bundled assets* (fonts, icons, sample media
with their licenses: OFL-1.1, MIT, CC0…). From v1, Help ▸ *User guide* opens
the local guide in the browser. This is mvp although outside the core loop
because every distributed binary must carry the GPL, FFmpeg and font license
notices.

**Inputs / controls:** tabs, *Copy version info*.

**Behavior / output:** License texts are generated at build time (e.g. with
`cargo-about`) and embedded. The v1 user guide is shipped with the app (no
network needed).
- The v1 guide has a *Getting started* page (also linked from Home's footer),
  a *Troubleshooting* page indexed by UI-040's error codes and the system
  check's warnings (UI-061), and an FAQ. Help ▸ *Troubleshooting* opens it
  directly.
- About has no terms of service to accept: the license is GPL-3.0-or-later,
  and a one-paragraph privacy statement says FreeCut collects nothing. An
  update status line ("Up to date as of 12 Oct 2026") appears only after the
  user has checked for updates (UI-055); otherwise About doesn't mention
  updates.

Reference: the observed About page shows the version, an up-to-date
indicator, legal and third-party copyright notices and terms of service, and
the help menu links to an online help centre, an FAQ and getting-started
tutorials.

**Edge cases:**
- No default browser → the guide's file path is shown with *Copy path*.
- License generation fails or finds a crate without a known license → the
  release build fails in CI; FreeCut never ships without complete notices.
- Several hundred crate licenses → the tab has a search field and a
  virtualized list.

**Undo:** n/a.

**Model notes:** build script in `freecut-app`; FFmpeg details reported by
`freecut-media`.

**Implementation note (2026-10-13):** the first piece: the About window lists
the bundled fonts and icons (Inter, JetBrains Mono, Noto Sans: OFL-1.1;
Phosphor Icons: MIT), each with its licence text as shipped in
`assets/fonts`, compiled into the binary. The tabs, crate licences and FFmpeg
details come with the rest of this feature.

---

### UI-052 Logs & diagnostics — v1

**What the user sees:** Help ▸ *Open logs folder* and *Copy system info*;
Preferences ▸ Advanced ▸ *Log level*.

**Inputs / controls:** menu items; log level dropdown.

**Behavior / output:** Logs use `tracing` with daily rotation, at most 7 files
of 10 MB each. *Copy system info* puts a plain-text block on the clipboard: OS,
CPU, RAM, GPU adapter / backend / driver, FFmpeg version and hardware
decoders, FreeCut version and active non-default settings. Media file paths
and project names are excluded from system info and redacted in Info-level
logs (they appear only at Debug). Nothing is sent anywhere by FreeCut.
Gathering logs, crash reports and system info into one file for an issue
report is UI-063.

**Edge cases:**
- Log folder not writable → logging to stderr only, one warning.
- Debug level on for long sessions → rotation caps disk use.
- *Copy system info* when the clipboard is unavailable → the text is shown in
  a dialog with a *Select all* button.

**Undo:** n/a.

**Model notes:** `tracing`, `tracing-subscriber`, `tracing-appender`
(MIT); initialised in `freecut-app::main`.

---

### UI-053 Native OS integration — v1

**What the user sees:**
- macOS: FreeCut's menus also appear in the system menu bar (with the
  standard Window menu); the Dock icon shows export progress; the Dock menu
  offers *New project*, *Open…* and the 5 most recent projects (PM-017).
- Windows: taskbar button shows export progress; its jump list offers *New
  project* and the 5 most recent projects.
- Linux: the desktop entry offers a *New project* action.
- All: double-clicking a `.freecut` file opens it in FreeCut; the project file
  has its own icon; opening FreeCut with a project path (command line or OS
  "open with") skips Home.
- **File-manager entry (Windows, opt-in, v1):** Preferences ▸ General ▸ *Add
  "Edit in FreeCut" to the file manager* (off) adds a right-click entry for
  supported media files that starts FreeCut, creates a new project (PM-002)
  and imports the selected files into it (PM-031). On macOS the app bundle
  ships the same action as a Quick Action the user enables in System
  Settings (the preference row links there, because an app can't switch it
  on itself); Linux packages register FreeCut as an "Open with" handler for
  media types instead, so neither shows the toggle.

Reference: the observed app offers an optional Windows file-manager entry
for quick import (the OS asks for permission the first time) and a dock or
taskbar menu with open and quit.

**Inputs / controls:** OS-level; the preference above.

**Behavior / output:** Native menus are mirrored from the same command
registry (e.g. with the `muda` crate, MIT/Apache-2.0); in-window menus remain
for Windows/Linux. File associations are registered by the installers /
packages. While exporting, FreeCut asks the OS not to sleep
(`preview-and-export.md`).

**Edge cases:**
- A second launch with a project path while FreeCut runs → v1 opens a second
  instance (single-instance hand-off is later); the project lock (PM-013)
  prevents double editing.
- macOS "open file" events arriving before the window exists → queued and
  handled after start-up.
- *Edit in FreeCut* on files FreeCut can't import → PM-031's import report
  in the new project, as for a drop.
- *Edit in FreeCut* on many files at once (the OS may start one process per
  file) → the first process collects the paths for 500 ms (FreeCut default)
  and imports them into one project.
- File-manager entry turned off → the registry entries are removed; turning
  it off when they were already removed by hand → no error.
- Windows 11's compact right-click menu shows the entry only under *Show more
  options* unless FreeCut is installed as a packaged app; the preference's
  help line says so.
- Jump list or Dock entry for a project that no longer exists → PM-017's
  "Not found" handling.

**Undo:** n/a.

**Model notes:** platform glue in `freecut-app::platform` behind `cfg`
attributes; no `unsafe` outside media/render (use safe wrapper crates, e.g.
`winreg`, MIT, for the per-user registry entries — no administrator rights
needed).

---

### UI-054 Detachable panels & multi-monitor — later

**What the user sees:** Panels can be torn off into separate windows (e.g.
player on a second monitor, a floating inspector), and the player can go
full-screen on a chosen monitor while editing on another. Each of the four
regions (asset panel, player, inspector, timeline) and every auxiliary panel
(UI-056) can be detached. View ▸ *Windows* lists the detached windows (and,
with UI-057, the open projects) and has *Dock all windows*.

Reference: the observed app can pop each of its four regions out into its
own window, spread the editor across two screens, and manage this from a
Window menu.

**Inputs / controls:** drag a panel's header out of the window, or *Detach*
in the header's menu; View ▸ *Player on display ▸*; View ▸ *Windows ▸*.

**Behavior / output:** Built on egui multi-viewport support; layouts (UI-003)
remember detached windows and their monitors. Detaching the timeline leaves
the main window as asset panel, player and inspector, which then share the
full height.

**Edge cases:**
- Saved monitor missing → panel re-docks.
- Different refresh rates per monitor → the player paces to its own display.
- Detached window closed with its close button → the panel re-docks instead
  of disappearing.
- A detached window can't be created or drawn (the OS refuses the window, or
  its graphics surface fails on that monitor) → the panel shows a placeholder
  "This panel couldn't be shown here" with *Dock it back*; the rest of the
  editor keeps working.
- All four regions detached → the main window keeps the top bar and status
  line; closing it quits as usual (UI-025).

**Undo:** n/a.

**Model notes:** `freecut-app::shell` viewports.

---

### UI-055 Update notice (opt-in) — later

**What the user sees:** If the user turns on *Check for updates* (off by
default), FreeCut shows a quiet notice on Home when a newer release exists,
with a link to the release page.

**Inputs / controls:** Preferences ▸ General ▸ *Check for updates weekly*;
*Include pre-releases* (off); a *Check now* button there and in About.

**Behavior / output:** At most one HTTPS request per week to the project's
public release feed; no identifiers sent, nothing downloaded or installed
automatically. Distribution package managers (Linux) can disable the option at
build time.
- *Check now* makes one request on demand, even with the weekly check off,
  and reports "You're on the latest version (1.4.2)" or "1.5.0 is available"
  with a link to its release page.
- *Include pre-releases* also reports beta builds; joining a beta is just
  downloading a pre-release build from the release page.
- Updates are never forced, never block a feature and never install
  themselves. Reference: the observed app has automatic or notify-only
  updates, forced updates, features that wait for an update, and a beta
  program tied to its servers.

**Edge cases:**
- Offline → silently skipped; *Check now* says "Couldn't reach the release
  page".
- Release feed unreachable or malformed → no notice, logged at Debug.
- Build compiled without the update feature → the setting is hidden.
- Running a pre-release with *Include pre-releases* off → a newer stable
  release is still reported.

**Undo:** n/a.

**Model notes:** network code isolated in `freecut-app::update`, compiled out
with a cargo feature.

---

### UI-056 Auxiliary panels: scopes, mixer, captions, transcript, jobs, history — v1

**What the user sees:** View ▸ *Panels* lists the extra panels other areas
define: Scopes (`color-and-filters.md`), Mixer (AU-013), Captions
(`text-and-captions.md`), Transcript (AI-011), Jobs (EX-038), History
(TL-044, later) and Assistant (AS-001, `assistant.md`). Each opens as an
extra tab in one of the fixed regions — FreeCut defaults: Scopes, Captions,
Transcript, History and Assistant beside the inspector tabs; Mixer and Jobs as a strip that splits the timeline region
horizontally (mixer on the right, 280 pt). A tab can be moved to the other
allowed slot from its right-click menu.

**Inputs / controls:** View ▸ *Panels* toggles; tab right-click *Move to
asset slot / inspector slot / timeline strip*; close (×) on the tab.
Shortcuts are unbound by default (assignable, UI-032).

**Behavior / output:**
- Panels are tabs in fixed slots, not free-floating (free docking and
  detaching are UI-054, later). Open panels, their slot and the strip height
  are workspace state and part of layout presets (UI-003).
- A panel's content, update rate and controls are its owner's; this area
  gives it a region, focus handling (it becomes a focus scope for UI-030) and
  a minimum size (240 × 160 pt).
- Opening a panel whose owner feature is not built hides the menu item, as for
  commands (UI-030).

**Edge cases:**
- Window too small for the strip plus the timeline minimum → the strip
  collapses to its header; the timeline keeps at least 160 pt.
- A saved layout names a panel that no longer exists → ignored.
- Panel open during full-screen preview (EX-010) → hidden until full screen
  ends.
- Two panels in the same slot → shown as tabs; the last one opened is in
  front.

**Undo:** n/a (view state).

**Model notes:** `freecut-app::shell::{AuxPanel, PanelSlot}`; owners register
a panel with id, title string id, allowed slots and a render callback.

---

### UI-057 Several open projects in separate windows — later

**What the user sees:** File ▸ *Open in new window…*, and *Open in new
window* in a Home card's menu (UI-017), open a project in a second editor
window while the first stays open. Each window is a full editor (panels,
player, timeline, its own undo history) titled with its project's name.
View ▸ *Windows* (and the macOS Window menu) lists the open projects.
Reference: the observed app keeps several projects open as tabs inside one
editor, refuses a tab past a cap that its labels put at 5, and has buttons
to go back to the editor or to its home hub.

**Inputs / controls:** the menu items; OS window switching. No default key
(assignable, UI-032). At most **5** projects open at once (FreeCut default,
the same cap as observed).

**Behavior / output:**
- One FreeCut process, one window per project (egui multi-viewport, as
  UI-054). Each window has its own project session: edit model, undo stack
  (TL-043), player, autosave scheduler (PM-014) and project lock (PM-013).
- Shared between windows: settings, keymap, layouts, library favorites, the
  media cache (PM-074), the background-task list (PM-073, rows labelled with
  their project), the analysis pool (AI-001) and the export queue (EX-038).
  EX-016's RAM frame-cache budget is split between open projects, with the
  focused window's share largest (FreeCut default: half, the rest shared
  equally).
- Only one window plays at a time: starting playback in one window pauses
  the others, so there is one audio clock (EX-002).
- *Close project* in a window closes that window when other project windows
  are open, and returns it to Home when it is the last one.
- Each window's geometry is remembered per project id (workspace state).
- Copy and paste between windows use TL-032's clipboard; what can be pasted
  into another project is TL-032's and PM's rule, not this spec's.

**Edge cases:**
- A sixth project → refused with a toast "Up to 5 projects can be open at
  once. Close one first."
- Opening a project that is already open → its window comes to the front
  (PM-013).
- Opening a project *in the current window* while it has unsaved changes →
  UI-025's prompt (Save / Don't save / Cancel). Opening in a new window never
  prompts, because nothing closes. Reference: the observed app asks to
  discard unsaved edits in this situation.
- Quit with several windows → UI-025's prompt per project with unsaved
  changes; *Cancel* stops the quit.
- An export started from one window keeps running when that window closes
  (EX-038); only quitting asks about it (UI-025).
- Dragging clips from one window to another → not supported in this
  version; the other window shows no drop target.
- Memory runs low with several projects open → UI-062 suggests closing one.

**Undo:** n/a; each window has its own undo stack (TL-043), and undo never
crosses windows.

**Model notes:** `freecut-app::session::{ProjectSession, SessionId}`; one
`Shell` per viewport. PM-002's "one project per window" rule holds per
window. Shared services are owned by the app and handed to sessions as
handles; no model state is shared between sessions.

---

### UI-058 Hints and "don't ask again" — v1

**What the user sees:** Small, one-time hints next to the thing they explain,
each with × and *Don't show hints like this*. Examples (FreeCut's own
wording): on an empty timeline, "Drag media here, or select it and press
Enter"; after the first split, "Drag a piece to reorder it"; the first time
a property with a keyframe diamond is focused, "Click ◇ to animate this
value"; the portrait-layout hint (UI-003); the new-OS-language hint
(UI-045). Confirmations that are safe to skip carry a *Don't ask again*
checkbox. Preferences ▸ General has *Show hints* (on) and *Reset hints and
"don't ask again" choices*.

Reference: the observed app shows contextual tips (drag media onto a track,
drag effects onto the timeline, reorder after a split, right-click for scene
splitting, add a keyframe, import media), each of which can be silenced
either everywhere or for the current project only, and its leave-editor
prompt has its own "don't remind me" box.

**Inputs / controls:** × (dismiss this hint); *Don't show hints like this*
(this hint everywhere); *Not in this project* on hints the owner marks as
project-related; the *Don't ask again* checkbox on skippable confirmations;
the two Preferences rows.

**Behavior / output:**
- **Registry.** Owners register hints like commands: id, trigger, anchor
  (the control or region it points at), scope (user or project), text id,
  and how many times it may show (1 by default). This area decides placement,
  timing and storage.
- **Pacing (FreeCut defaults):** at most one hint on screen; at most one new
  hint per 2 minutes of active editing; none during playback, drags,
  full-screen preview or the guided tour (UI-059), and none over a modal
  dialog. A hint never takes keyboard focus; `Esc` closes it first (UI-031).
- Hint text uses the current bindings from the registry, so a rebound key is
  shown correctly.
- **Empty states are not hints.** An empty timeline or media panel always
  keeps its one-line description and action button; *Show hints* off only
  removes the extra coaching text.
- **Storage:** user-scope dismissals in `hints.json` (config folder);
  project-scope ones in the per-project view file (PM-021's
  `view/<project-id>.json`), so dismissing a hint never marks the project
  dirty and never travels with the project file. Alternative observed: the
  reference app stores per-project prompt choices with the project itself.
- **"Don't ask again" is allowed only on skippable confirmations.** Each
  confirmation in the registry is marked skippable or not. Prompts that
  protect unsaved work or files — UI-025's unsaved-changes prompt, moving
  projects to the Trash (PM-018), *Reset all* shortcuts (UI-032), clearing
  the cache (UI-026), clearing search history (UI-006) — are never
  skippable; a unit test pins that list.
- *Reset hints…* brings back every hint and every skipped confirmation:
  user scope at once, and project scope in each project the next time it is
  opened (a reset counter in `hints.json` is compared with the one stored
  in the view file).

**Edge cases:**
- The hint's anchor is hidden (panel collapsed, tab not active, window too
  small) → the hint waits; it is not used up.
- The trigger fires again after the hint was dismissed → nothing.
- Config folder read-only → dismissals last for this session only.
- Several hints eligible at first launch → shown one at a time under the
  pacing rule; any still pending when the user has clearly found the feature
  (they used its command) are dropped.
- A skipped confirmation whose action later fails → the error is shown as
  usual; skipping only removed the question.
- Screen reader running → hints are announced as polite live-region updates
  (UI-042).

**Undo:** n/a (user state, not project edits).

**Model notes:** `freecut-app::hints::{HintId, HintScope, HintRegistry,
ConfirmId}`. Eligibility and pacing are pure functions over (registry,
dismissals, clock, app state), unit-tested with a fake clock.

---

### UI-059 Guided tour — later

**What the user sees:** Help ▸ *Take the tour*, and a slim banner the first
time the editor opens ("New to FreeCut? Take a 2-minute tour · Not now").
The tour opens a small practice project built from the bundled CC0 sample
clips (PM-063) and walks through 7 steps: add media to the timeline, move and
trim a clip, split it, move around the timeline (zoom and the playhead),
select several clips, change a property in the inspector, and export. Each
step highlights one area, says what to do, shows "Step 3 of 7", and moves on
when the user has done it or presses *Next*.

Reference: the observed app has a skippable step-by-step walkthrough of the
same kinds of tasks (adding, moving and editing clips, moving around the
timeline, multi-select, export) that can be left part-way and replayed from
Help, with a banner the first time the editor opens.

**Inputs / controls:** *Next*, *Back*, *Skip tour*, *Exit*; `Enter` = Next,
`Esc` = Exit.

**Behavior / output:**
- The practice project lives in app data, is not added to recent projects
  and is deleted when the tour ends. If a project was open, it is closed
  first through UI-025's prompt and reopened when the tour ends.
- A step completes when its command runs (for example `timeline.split`) or
  its state condition holds, so it works with any key bindings (UI-030).
- Leaving part-way remembers the step; *Take the tour* then offers *Resume*
  and *Start over*.
- Hints (UI-058) are paused while the tour runs.

**Edge cases:**
- Sample pack not installed (a packager left it out) → the practice project
  uses color clips (PM-108) and a generated tone instead.
- The user removes something a step needs → the step offers *Reset this
  step*, which restores that step's starting state.
- The highlighted panel is collapsed or the window is too small → the step
  expands the panel (UI-002) before pointing at it.
- Screen reader running → each step's text is announced and every highlight
  has a text equivalent (UI-042).
- Export step → the export goes to a temporary folder and is deleted with the
  practice project, unless the user picks a folder.

**Undo:** edits in the practice project are ordinary undoable ops; the tour
itself is n/a.

**Model notes:** `freecut-app::tour::{TourStep, StepCondition}`; steps are
data (target region, text id, completion as a command id or a state
predicate), so they are unit-testable without a window.

---

### UI-060 What's new after an update — later

**What the user sees:** On the first launch after FreeCut's version changes,
a card on Home: "FreeCut 1.4 — what's new", with 3–6 highlights, each with
*Show me*, plus *Full release notes*. Help ▸ *What's new* reopens it.

Reference: the observed app shows a popup of each version's new features
after an update and a feature tour with "try it" buttons, some of which need
the network and show a retry state offline.

**Inputs / controls:** *Show me*, *Full release notes*, close (×);
Preferences ▸ General ▸ *Show what's new after updates* (on).

**Behavior / output:**
- Release notes are bundled with the app (no network) and go through the
  string catalog (UI-044) where translations exist.
- *Show me* runs the highlight's command, opens its panel or jumps to its
  Preferences row, using the command registry; a highlight whose command
  isn't in this build is skipped.
- Shown once per version; nothing is shown after a downgrade.

**Edge cases:**
- Several versions skipped → highlights of every skipped version are merged,
  newest first, up to 10.
- First install → not shown (UI-049's first-run setup instead).
- Recovery prompt or safe mode active → those banners come first (UI-016);
  the card waits for the next launch.
- Translation of the notes incomplete → English text for the missing parts.

**Undo:** n/a.

**Model notes:** `settings.last_seen_version`; notes compiled in with
`include_str!` from the repository's release-notes folder.

---

### UI-061 System check & hardware rating — v1

**What the user sees:** Help ▸ *Check this computer…* (also *Run system
check* in Preferences ▸ Performance) opens a report: an overall rating with
one line on what it means, then a list of checks marked ✓, ⚠ or ✕, each
warning with a plain-words fix, and *Copy report*. A quick version of the
check runs at every start-up and speaks up only when something is wrong.

| Rating (FreeCut's own names) | Meaning shown | Rule (FreeCut defaults) |
|---|---|---|
| **Great** | 4K editing without proxies | discrete GPU or Apple silicon, ≥ 16 GB RAM, ≥ 8 logical cores, hardware decoding of H.264 and HEVC |
| **Good** | 1080p editing; proxies for 4K | any hardware GPU, ≥ 16 GB RAM, ≥ 8 logical cores |
| **Basic** | 1080p editing with proxies | any hardware GPU, ≥ 8 GB RAM, ≥ 4 logical cores |
| **Below requirements** | editing may be very slow | less than Basic, or only a software graphics adapter |

Checks:

| Check | ✓ when | ⚠ / ✕ and what FreeCut says |
|---|---|---|
| Graphics adapter | wgpu finds an adapter with FreeCut's required features | ✕ none: editing isn't possible; UI-024's fallback adapter is tried first |
| Adapter kind | discrete GPU or Apple silicon | ⚠ integrated: heavy effects and 4K may be slow; ⚠ software adapter: very slow |
| Running on the faster GPU | the high-performance adapter is in use when there are two | ⚠ integrated GPU in use although a discrete one exists, with the OS's steps to assign FreeCut to the high-performance GPU |
| Graphics driver | not on FreeCut's known-problem list | ⚠ "Update your graphics driver", with the vendor's download page; UI-028 starts hardware decoding and encoding off for that driver |
| Hardware decoding / encoding | EX-018 and EX-034 probes succeed | ⚠ not available: 4K or HEVC playback may need proxies (PM-080) |
| Memory | ≥ 8 GB (8–15 GB adds "limits the rating to Basic") | ⚠ below 8 GB |
| Processor | 64-bit, ≥ 4 logical cores (4–7 adds "limits the rating to Basic") | ⚠ fewer than 4; a note when AI runtimes lack the instruction sets they expect (AI-002) |
| Operating system | at or above the minimums below | ✕ below them (installers refuse earlier) |
| Disk space | above PM-113's warning value on every watched volume | ⚠ with PM-113's text and *Storage settings…* |
| Folder access | config, app data, cache and logs are writable | ⚠ per folder; FreeCut keeps running (settings in memory, UI-018) |
| Audio output | at least one device (AU-010) | ⚠ "No audio output — playback will be silent" |

Platform minimums (FreeCut defaults): 64-bit only; Windows 10 or newer;
macOS 11 or newer; Linux with Vulkan 1.1, or OpenGL 3.3 / OpenGL ES 3.0 for
the fallback path. Alternative observed: the reference app supports macOS
from 10.14.6.

Reference: the observed app runs an environment test at launch and on
demand, rates the machine in four tiers (runs smoothly, recommended,
minimum, below requirements), lists requirements, and checks the graphics
card and driver (presence, health, age with its date), whether the discrete
GPU drives the monitor, the Windows theme, required video libraries, OS
compatibility mode, free disk and temp space, folder permissions, the network
and injected third-party modules. It can also install a bundled driver or
reset the graphics driver. FreeCut keeps the local, read-only checks above
and leaves out the rest (see Out of scope).

**Inputs / controls:** *Run again*, *Copy report*, the fix buttons (open the
guide's troubleshooting entry, open the OS's graphics settings where the OS
offers a link, *Storage settings…*, *Use proxies…*); Preferences ▸ General ▸
*Check this computer at startup* (on).

**Behavior / output:**
- **Start-up quick check:** after the first frame, on a background thread,
  only the cheap checks (adapter, driver list, memory, OS, folder access);
  the EX-018/EX-034 probes already run in the background. Problems raise one
  warning toast, "This computer may have trouble running FreeCut smoothly —
  Details", at most once per app version per problem; it is never a modal
  dialog. A missing adapter goes through UI-023's start-up failure path
  instead.
- The rating is advice only: FreeCut never refuses to edit because of it.
- Effects disabled for the adapter ("Not supported on this graphics
  adapter", UI-028) are listed under the adapter row.
- The known-problem driver list is a data file shipped with FreeCut and
  updated with each release (vendor, driver version range, problem, advice);
  nothing is fetched online.
- *Copy report* uses UI-052's redaction; nothing is sent anywhere.
- FreeCut never installs, updates or resets drivers and never changes OS
  settings.

**Edge cases:**
- Slow driver queries → rows fill in progressively; the report opens at
  once.
- Virtual machine or remote desktop with a software adapter → Below
  requirements, with a note that remote sessions are slow; editing allowed.
- Laptop on battery reporting only the integrated GPU → the hint mentions
  plugging in as well as the OS graphics preference.
- External GPU unplugged since the last start → UI-028's Auto fallback, and
  the start-up check reports the change once.
- A driver leaves the known-problem list in a newer FreeCut → the warning
  disappears after the update.
- Windows high-contrast mode on → an informational row suggesting FreeCut's
  High contrast theme (UI-014).
- Check run during playback or export → allowed; it reads cached probe
  results instead of re-probing encoders.
- A check itself fails (a query errors or times out after 10 s) → that row
  shows "Couldn't check" with the error, the rating is computed from the
  rest and marked "partial", and the report says projects are not affected.

**Undo:** n/a.

**Model notes:** `freecut-app::syscheck::{Check, CheckResult, Rating,
rate(&SystemInfo) -> Rating}`; `rate` and each check's verdict are pure and
table-tested. `SystemInfo` is gathered from `freecut-render::AdapterInfo`,
the `freecut-media` probes (EX-018, EX-034), `freecut-audio`'s device list,
`sysinfo` (MIT) for CPU and memory, and PM-113's free-space query.

---

### UI-062 Low-memory warnings & out-of-memory notice — v1

**What the user sees:** When the computer runs low on memory while editing, a
warning toast: "Your computer is low on memory. FreeCut may slow down or
close." with *Ways to use less memory*, which lists one-click actions: lower
the preview resolution (EX-006), turn on proxies (PM-081), shrink the frame
cache (EX-016), close other open projects (UI-057). If the last session ended
unexpectedly while memory was nearly used up, the next launch's recovery
prompt (PM-015, UI-023) says "FreeCut probably ran out of memory", with the
same suggestions and a note that very long projects and many 4K layers need
more memory.

Reference: the observed app explains on the next launch when its last
session ended because memory ran out, and suggests closing other apps, using
fewer clips or elements, exporting at a lower resolution or using a stronger
device.

**Inputs / controls:** the action buttons; Preferences ▸ Performance ▸ *Warn
when memory is low* (on).

**Behavior / output:**
- A background monitor samples available system memory and FreeCut's own
  memory use every 5 s (FreeCut default). "Low" means available memory below
  10 % of total RAM or below 2 GB, whichever is smaller, for 15 s in a row.
- The warning appears at most once per 10 minutes and clears itself when
  memory recovers.
- On low memory this area also sends a memory-pressure event; caches decide
  for themselves whether to shrink (EX-016, PM-070's thumbnail memory).
- Every 30 s the latest sample is written into the session marker (UI-023).
  A crashed session whose last sample was low is reported as a likely
  out-of-memory exit. The wording says "probably", because the OS gives no
  reason.

**Edge cases:**
- The OS ends FreeCut without warning (Linux OOM killer, macOS memory
  pressure) → only the next-launch notice is possible.
- Memory runs out inside FreeCut (allocation failure) → Rust aborts without
  running the panic hook; the marker-based notice still works.
- Memory figures unavailable (sandboxed builds) → no warnings, logged once.
- An export is running → it is never cancelled automatically; the toast adds
  *Pause export queue* (EX-038).
- GPU memory running out is not this feature: the renderer and AI-001 handle
  it (EX device loss, AI-001's CPU retry).

**Undo:** n/a.

**Model notes:** `freecut-app::memory::{MemoryMonitor, MemoryPressure}` using
`sysinfo` (MIT); thresholds and hysteresis are a pure state machine tested
with fake samples.

---

### UI-063 Diagnostic bundle & issue reports — v1

**What the user sees:** Help ▸ *Report an issue…* opens a small dialog:
- *What went wrong?* — Playback, Import, Export, Timeline editing, Text &
  captions, Audio, Effects & color, Smart tools, Saving & opening projects,
  Crash or freeze, Installation & updates, Other (FreeCut's list); for
  Playback and Export a second choice: black frames, blurry, flicker, wrong
  colors, stutter, other;
- *Describe it* (optional text);
- *Include*: System info (on), Recent logs (on), Crash reports from the
  last 7 days (on when any exist), *The current project file* (off), *File
  paths* (off);
- *Preview*, **Save bundle…** and **Open issue page**.

Help ▸ *Suggest a feature* opens the issue tracker's feature-request form.
The crash prompt (UI-023) opens this dialog with *Crash or freeze* chosen.

Reference: the observed app has an upload form with about 15 problem
categories (with black-frame, blur, flicker and color-shift details for
import and export), a description, screenshots and recordings, optional
project and media upload, logs and crash data with consent, an optional
email, attachments up to 200 MB, a feature-request form and a support bundle
of logs and projects. FreeCut keeps the local parts and uploads nothing.

**Inputs / controls:** the fields above; *Preview* shows exactly what will
be included before anything is written.

**Behavior / output:**
- **Save bundle…** writes `freecut-diagnostics-<date>.zip` to a folder the
  user picks (default: the OS Downloads folder) on the I/O thread, then
  reveals it. It holds `report.md` (area, details, description, FreeCut
  version, system info, the last 20 error codes with times), logs (the last
  2 days of UI-052's files), crash reports, and, if ticked, the last saved
  `.freecut` file plus a media list (names, sizes, codecs, durations). Media
  files themselves are never included.
- **Open issue page** copies `report.md` to the clipboard and opens the
  project's new-issue page in the browser; the user pastes it and attaches
  the zip there. Nothing goes into the URL, and FreeCut sends nothing.
- Redaction as UI-052: the home folder becomes `~`; project and media names
  are removed unless the project file is included; full paths only with
  *File paths* ticked.
- The bundle's size is shown before saving. Above 25 MB (FreeCut default) a
  note warns that issue trackers may refuse large attachments; nothing is
  capped.
- The *Include* choices are remembered (UI-019); the description is not.

**Edge cases:**
- No default browser → the issue URL is shown with *Copy link* (as UI-051).
- Clipboard unavailable → the report text is shown in a dialog with *Select
  all*.
- Destination not writable or disk full → inline error; choose another
  folder.
- The project has unsaved changes and *The current project file* is ticked →
  the last saved version is included and the dialog says so; FreeCut does
  not save on its own here.
- No project open → *The current project file* is hidden.
- Dialog closed after typing a description → the text is kept for the rest
  of the session. Reference: the observed form warns that typed text will be
  lost.

**Undo:** n/a.

**Model notes:** `freecut-app::diagnostics::{ReportArea, BundleOptions,
build_bundle}` with the `zip` crate (MIT); writing on the I/O executor. The
redaction function is shared with UI-052 and unit-tested.

---

### UI-064 Save-as-preset dialog & user preset conventions — v1

**What the user sees:** Every "save as preset" action — looks (CL-026), text
styles (TX-020), export presets (EX-041), effect presets (FX-024, later),
element styles (OV-044, later), retouch presets (AI-032, later) — opens the
same small dialog: a name field with a suggested name, what will be saved
(the owner's checklist or summary), and *Save*. The button reads "Saving…"
while the file is written; then a success toast "Saved to My looks" with
*Show*, or an error toast with *Retry*. The new preset appears first in its
"My …" category, selected.

Reference: the observed app shows saving, saved and failed states when the
user saves current settings as a preset, caps the number of presets, asks
the user to try again later when saving fails, and warns when its preset
folder is missing.

**Inputs / controls:** name 1–64 characters, any Unicode, trimmed; `Enter`
saves, `Esc` cancels; the owner's checklist.

**Behavior / output:**
- A duplicate name gets " (2)", " (3)" (as CL-026, TX-020 and EX-041); there
  is no overwrite prompt unless an owner defines one.
- No count limit by default (as CL-026). An owner may set one (AI-032: 50);
  the dialog then shows "12 of 50", and *Save* is disabled at the cap with
  the reason. Alternative observed: the reference app caps saved presets and
  refuses with a toast.
- The owner's serializer writes the file on the I/O thread, atomically, into
  the owner's user library in app data, named by id so any display name
  works (CL-026's rule).
- Rename, Duplicate and Delete are in each user preset's context menu;
  Delete asks for confirmation (and uses the OS trash where the owner says
  so, CL-026).
- Libraries are listed off the UI thread; an unreadable file is shown as
  unavailable with the reason, never silently deleted (CL-026's pattern).
- Applying a preset is the owner's undoable op; nothing here edits the
  project.

**Edge cases:**
- Library folder missing (deleted by hand) → recreated on save; if that
  fails, error toast with the reason and *Retry*. The dialog stays open with
  its values until the save succeeds or is cancelled.
- Disk full → PM-113's message.
- Name of only spaces → *Save* disabled.
- Two windows (UI-057) save presets at once → separate files; both lists
  refresh.
- Saving while the owner's checklist is all unticked → *Save* disabled with
  the owner's reason (e.g. CL-026's "Nothing to save").

**Undo:** n/a — preset libraries are user data, not project edits.

**Model notes:** `freecut-app::presets::{SavePresetDialog, PresetLibrary<T>}`,
generic over the owner's payload. Owners provide the kind, folder name,
schema version, serializer and optional cap from their own crates.

---

### UI-065 Tool suggestions for the selected clip — later

**What the user sees:** A slim *Suggestions* row at the bottom of the
inspector with up to 3 chips that fit the selected clip, for example: a
vertical clip in a wide project → *Auto reframe* (AI-016) or *Fill frame*
(TR-002); speech with no captions → *Auto captions* (AI-010); dark footage →
*Auto color* (CL-019); long pauses → *Remove silences* (AI-003); shaky
footage → *Stabilize* (TR-026). A chip opens the owner's own tool, never a
silent change. After the first use of a tool through a chip, a hint (UI-058)
points at its usual place in the interface.

Reference: the observed app has a side panel that suggests about 20 kinds of
one-click tools for the selected clip (retouching, captions, reframing,
tracking, color matching, upscaling, voice cleanup, frame interpolation,
relighting, background removal, filler-word and flicker removal,
transcript editing, speed ramps), most of them server-based or paid, and
points at each tool's regular place after first use. FreeCut suggests only
local tools.

**Inputs / controls:** click a chip; × on a chip (*Don't suggest this*);
Preferences ▸ Appearance ▸ *Show tool suggestions* (on).

**Behavior / output:**
- Rules read only data that already exists: clip and media metadata
  (orientation against the canvas, audio present, duration) and analysis
  records AI-001 and AI-035 have already stored (silences, loudness, shake,
  exposure, speech). The row never starts an analysis itself.
- Only tools present in the build are suggested; a tool whose model isn't
  installed says "needs download" and opens AI-002's prompt.
- Rules are a fixed table (condition → command id, priority); at most 3
  chips, highest priority first. Suggestions refresh when the selection
  changes, not continuously.

**Edge cases:**
- Several clips selected → chips only when every selected clip matches, and
  the tool then gets all of them (the owner's batch op); otherwise no row.
- The condition stops holding after an edit → the chip disappears; a click
  re-checks first.
- Clip on a locked track → no chips.
- *Don't suggest this* → remembered per user and rule (UI-058's store);
  *Reset hints* brings it back.

**Undo:** whatever the chosen tool does (its owner's op); the row itself is
n/a.

**Model notes:** `freecut-app::suggest::{Rule, Suggestion,
suggestions_for(&Selection, &AnalysisIndex) -> Vec<Suggestion>}`, pure and
table-tested.

## Interactions with other areas

- **`project-and-media.md` (PM):** owns the start-screen content and recent
  projects (PM-001, PM-017), project actions (PM-018), save / open / lock
  (PM-012, PM-013), autosave, crash recovery and backups (PM-014…PM-016), the
  view-state file (PM-021), import and drops (PM-030, PM-031), the media panel
  (PM-050…PM-060), the background job list (PM-073), the media cache (PM-074)
  and proxies (PM-080, PM-081). This area supplies the Home shell, the
  save-status display and close prompt, the Preferences pages that host PM's
  settings, the task indicator and the default keys for PM's commands
  (`Ctrl+N`, `Ctrl+O`, `Ctrl+S`, `Ctrl+Shift+S`, `Ctrl+I`, `F2`, `Del`,
  `Ctrl+F`, `F` for favorites, `Alt+P` for *Use proxies*). PM-113's
  low-space thresholds are hosted in Preferences ▸ Storage (UI-026) and its
  warnings feed the system check (UI-061); user libraries stay in app data
  per PM-042; project-scope hint dismissals live in PM-021's view file
  (UI-058); the guided tour builds its practice project from PM-063's sample
  pack (UI-059).
- **`timeline-editing.md` (TL):** selection, target track, placement and drop
  rules (TL-012), navigation (TL-013), snapping and magnet (TL-028, TL-005),
  *Follow main track* (TL-029) and *Ripple all tracks* (TL-021), whose state
  stays undoable per-sequence flags (TL-003), track hide & mute (TL-008, mvp)
  and lock (TL-009, v1), markers (TL-037), zoom and scroll (TL-041), undo
  stack, labels, coalescing and depth (TL-043), history panel (TL-044).
  **Paste attributes** (TL-034, v1): TL owns the command, its checklist
  dialog, the attribute clipboard and the `PropBag::category()` registry
  through which TR-034, CL-025, FX-017 and OV-044 register their categories;
  this area supplies only the Edit-menu item, the clip context-menu entry and
  the `Ctrl+Alt+V` binding. This area provides the toolbar (including the
  *Follow main track* button and the `⋯` menu with *Ripple all tracks*), the
  track-header context menu, the final keymap, context menus, keyboard-only
  editing paths and drag ghosts drawn from TL's planner. TL-015's
  select-all-on-a-track chord and its click modifiers, and TL-018's drag
  modifiers, are listed in UI-031 (the *Modifier gestures* table).
- **`clip-transform-and-speed.md` (TR):** Video/Speed inspector sections,
  keyframe diamond states (TR-028) and navigation (TR-030), easing menu. This
  area provides the shared control conventions (UI-009) and the default keys
  for TR's commands (UI-031).
- **`text-and-captions.md` (TX):** Text and Captions tabs, the Captions panel,
  rich-text shortcut claims, font and subtitle file drops, default text
  duration. TX-020's *Copy style* / *Paste style* are listed in UI-031 as
  unbound by default; its *Save style…* uses UI-064's dialog; the saved
  personal palette TX notes for the color swatch is UI-009's *Saved* row.
- **`audio.md` (AU):** Audio tab, mixer panel, voiceover button, audio
  scrubbing, output device and buffer settings (AU-010); the mixer's track
  mute (AU-011) toggles the same TL-008 flag as the track header. *Detect
  beats* (AU-025) is listed in the clip *Tools ▸* menu (UI-035); the system
  check reports a missing output device from AU-010's device list (UI-061).
- **`color-and-filters.md` (CL):** Filters/Adjust tabs, Color inspector tab,
  Scopes panel, `.cube` drops, eyedropper. CL-026's user-look rules (no cap,
  " (2)" suffixes, files named by id, trash on delete) are the pattern
  UI-064 generalises; CL-002's always-SDR preview is why UI-028 has no HDR
  display setting; *Auto color* (CL-019) appears in the clip *Tools ▸* menu
  (UI-035) and in suggestions (UI-065).
- **`effects-transitions-animations.md` (FX):** Effects/Transitions tabs,
  Animation and Effects inspector tabs, tile previews and try-before-apply
  (FX-016), default transition (FX-009), unsupported-adapter badges (listed
  by UI-061's adapter row), effect presets (FX-024) through UI-064.
- **`overlays-and-stickers.md` (OV):** Elements and Templates tabs, library
  favorites and recents (OV-038, OV-039), the start-screen template row
  (OV-032), the shared colour picker (UI-009), which shows OV-043's brand
  palette; element style presets (OV-044) use UI-064's dialog.
- **`ai-tools.md` (AI):** entry points placed in tabs/menus, the Transcript
  panel slot. AI-001 defines its job kinds, the per-clip progress strips and
  result badges, and the analysis pool's concurrency settings (hosted in
  Preferences ▸ Smart tools, UI-018); its jobs report rows into PM-073's
  single task list, and the status-line indicator and toasts are UI-040's.
  AI-002's model settings are hosted under Smart tools ▸ Models, and its
  download prompt is what *needs download* entries open (UI-035, UI-065).
  The clip *Tools ▸* menu and UI-065's suggestions only call AI's commands
  and read records AI-001 and AI-035 have already stored; they never start
  an analysis of their own.
- **`preview-and-export.md` (EX):** player region, transport and J/K/L
  (EX-002, EX-007), loop and play range (EX-008), preview resolution (EX-006),
  full-screen preview (EX-010), frame snapshot (EX-014), preview mute and
  audio monitoring (EX-019), hardware decoding and frame cache settings
  (EX-016, EX-018), export dialog, Jobs panel and progress, sleep prevention.
  EX's commands take their default keys from UI-031, now including player
  zoom (EX-009, Player scope) and *All guides off* (EX-011). EX-034's
  hardware-encoding preference is hosted in Preferences ▸ Performance
  (UI-028), and its encoder probe feeds the system check (UI-061).

## Out of scope (and why)

- **Cloud space, cloud drafts, cross-device sync, team spaces, shared
  projects, comments and review links** — FreeCut is local-only; projects are
  ordinary files the user can sync with any tool they like.
- **Accounts, sign-in, profiles, subscriptions, credits, paywalled items and
  "Pro" badges** — FreeCut is free forever with no account.
- **Online asset, music, template and trend feeds in the asset panel** —
  replaced by bundled own/open-licensed assets and user-imported files.
- **Telemetry, analytics and automatic crash upload** — crash reports stay
  local; the user decides whether to share them.
- **Direct publishing to social platforms from the top bar** — export to file
  only (`preview-and-export.md`).
- **Mobile, tablet, touch-first and web layouts** — desktop app only.
- **Arbitrary free-form docking in mvp/v1** — fixed regions plus presets keep
  the UI simple; detachable panels are UI-054 (later).
- **Hardware control surfaces (jog wheels, MIDI controllers), scripting and
  plugin UIs** — not planned for now.
- **An in-app AI chat assistant panel** — not part of this area; any
  assistant-style feature would be specified in `ai-tools.md`.

Reference-app themes in this area, seen in its UI labels, that FreeCut leaves
out:

- **Web-fed start hub** (quick-tool grids, template feeds, back / forward /
  refresh, first-visit tips from the network, loading-error states) — Home
  is local only (UI-016): projects, quick-start canvases and local templates
  (OV-032).
- **Separate image-design workspace and generative entry points on the hub**
  (text-to-video, script writer, camera-against-a-template) — FreeCut is a
  video editor; stills come from EX-014's snapshot, and generative tools are
  out of scope per `ai-tools.md`.
- **Online asset downloads** (download states, a downloads list, a ~450 MB
  free-space pre-check before a component download, a chosen download
  folder) — library content is bundled or imported (UI-004); the only
  downloads are AI-002's optional models, each confirmed by the user.
- **Trending and popular searches, popular-item fallbacks, cloud scene and
  sound search** — local search with local recent queries (UI-006);
  spoken-word search is AI-027.
- **Account-synced shortcut sets** (signing in replaces local shortcuts) —
  keymap export and import as a file (UI-032).
- **Notification and push preferences, inbox, follow prompts** — FreeCut
  sends no notifications outside its window.
- **Onboarding survey about role, goals and content genres** — FreeCut
  collects nothing about its users (UI-049).
- **Server-backed "what's new" tour items and feature promotions** — release
  notes ship with the app (UI-060).
- **Feedback upload form, attachment upload (up to 200 MB), contact email,
  support chat and network test** — UI-063 writes a local bundle and opens
  the public issue tracker; nothing is uploaded by FreeCut.
- **Paid or server-side quick tools in the clip menu and the suggestion
  panel** — only local tools appear (UI-035's *Tools ▸*, UI-065).
- **Launch at login and running in the system tray** — a video editor has no
  background work to do while closed.
- **Installing bundled GPU drivers and resetting the graphics driver from the
  app** — FreeCut never changes system software or settings; UI-061 links to
  the vendor's driver page.
- **Detecting third-party software injected into the process** (audio
  "enhancers", window-skinning tools, office add-ins) — it needs
  platform-specific process inspection outside FreeCut's safe-Rust rules;
  crash reports name the GPU driver and OS, which covers the common causes.
- **Network-stability, OS-compatibility-mode and temp-folder checks** —
  FreeCut needs no network, and it writes temporary files next to their
  targets (atomic writes) or in the media cache, so there is no separate temp
  folder to watch.
- **Language-and-region setting with regional feature gating** — no regions;
  formats follow the UI language (UI-045).
- **Forced updates, version-gated features and a server-managed beta
  program** — opt-in, notify-only update check with optional pre-releases
  (UI-055).
- **Tablet build layouts and window-size guards** — desktop only (above).

## Clean-room notes

- **Research basis:** public help-centre pages, public product pages,
  Wikipedia, and the public documentation of open-source editors (Shotcut,
  Kdenlive, OpenShot), plus a paraphrased inventory of the English UI labels
  of a locally installed copy of the reference app (CapCut desktop 9.5.0),
  taken from its bundled English localisation file (see Provenance in the
  [README](README.md)). Nothing was decompiled, disassembled or traced, and
  no assets, project or draft files, network
  traffic or APIs were inspected or copied. Facts taken from that inventory
  (option lists, limits such as the 5-project cap, the four layout presets
  and three user slots, the 10-frame step) are written in FreeCut's own words
  as observations, and wherever FreeCut chooses differently the spec says so
  in one line.
- **Layout:** the four-region editor layout is a convention shared by most
  editors (Kdenlive, Shotcut, Resolve, CapCut); FreeCut's sizes, tokens and
  arrangement details are its own.
- **Visual design:** FreeCut's own colour tokens (UI-013/014) and own app and
  file icons, drawn by contributors. No CapCut icons, artwork, colours copied
  from their UI, or screenshots are used in FreeCut or committed (screenshots
  served only as research observations; see Provenance in the README).
- **Icons:** Phosphor Icons (MIT), shipped as the upstream icon font; Lucide
  (ISC) as an alternative; editing-specific glyphs drawn by contributors
  (ui-concept §14).
- **Fonts:** Inter (OFL-1.1) for UI; JetBrains Mono (OFL-1.1) for timecode;
  Noto Sans / Noto Sans CJK (OFL-1.1) as optional fallbacks.
- **Sample media** for library previews and the decode test: recorded or
  rendered by contributors and released as CC0.
- **Keymap:** built from long-standing, widely documented editing conventions
  (JKL, I/O, split on `Ctrl+B`, etc.). No keymap preset is named after or
  marketed as matching CapCut.
- **Translations:** written by FreeCut contributors; CapCut's localized UI
  strings are never copied.
- **Library content:** every item shown in the asset panel comes from the
  owning spec's own or open-licensed packs (CC0 sample clips, music and sound
  effects; FreeCut-made looks, transitions, effects and elements) or from
  files the user imports; this area ships no content of its own beyond icons
  and fonts.
- **Help content:** hint texts, tour steps, release notes, the issue-report
  categories, the rating names (Great / Good / Basic / Below requirements)
  and the error-code scheme are written by FreeCut contributors. The
  known-problem graphics-driver list (UI-061) is compiled from GPU vendors'
  public release notes and FreeCut's own bug reports, never from the
  reference app.
- **Wording:** all labels and help texts are FreeCut's own; CapCut is named
  only in short reference notes in this spec, never in the UI. Labels seen in
  the reference app's UI were paraphrased for this spec and are not reused as
  UI text.

## Sources

- CapCut Help Center (index) — https://www.capcut.com/help/
- CapCut Help — User Interface & Settings category — https://www.capcut.com/help/interface-and-settings
- CapCut Help — How do I switch languages in CapCut? — https://www.capcut.com/help/how-to-switch-languages-in-capcut
- CapCut Help — Where can I find my current version? — https://www.capcut.com/help/where-to-find-my-current-version-capcut
- CapCut Help — Why did feature locations or the UI change after I updated? — https://www.capcut.com/help/capcut-ui-update-changes
- CapCut Help — Keyframes in CapCut PC — https://www.capcut.com/help/keyframes-in-capcut-pc
- CapCut Help — Export path permission — https://www.capcut.com/help/access-permission-to-export-path
- CapCut Help — Why is my video not showing in CapCut? — https://www.capcut.com/help/video-not-showing-in-capcut
- CapCut Help — Export stuck — https://www.capcut.com/help/export-stuck
- CapCut Help — Why are transitions lagging? — https://www.capcut.com/help/transitions-lagging
- CapCut Help — Template plays in slow motion — https://www.capcut.com/help/template-play-in-slow-motion
- CapCut Help — Drafts after a subscription ends — https://www.capcut.com/help/capcut-pro-draft-videos
- CapCut Help — Editing & Exporting category — https://www.capcut.com/help/editing-and-exporting
- CapCut Help — Download & Install category — https://www.capcut.com/help/download-and-install
- CapCut — How to use CapCut on PC — https://www.capcut.com/resource/how-to-use-capcut-on-pc
- CapCut — CapCut tutorial for beginners — https://www.capcut.com/resource/capcut-tutorial-for-beginners
- CapCut — CapCut desktop download (requirements, proxy and autosave notes) — https://www.capcut.com/resource/capcut-desktop-download
- CapCut — How to crop video in CapCut PC — https://www.capcut.com/resource/how-to-crop-video-in-capcut-pc
- CapCut — CapCut PC vs web vs mobile — https://www.capcut.com/resource/capcut-pc-vs-capcut-web-vs-capcut-mobile
- CapCut — Desktop video editor product page — https://www.capcut.com/tools/desktop-video-editor
- CapCut — public product sitemap (locale variants of the site) — https://www.capcut.com/sitemap/new/348188/product/sitemap_index.xml
- Wikipedia — CapCut — https://en.wikipedia.org/wiki/CapCut
- Shotcut — Keyboard shortcuts — https://shotcut.org/howtos/keyboard-shortcuts/
- Kdenlive documentation — Shortcuts — https://docs.kdenlive.org/en/user_interface/shortcuts.html
- Kdenlive documentation — User interface — https://docs.kdenlive.org/en/user_interface.html
- Kdenlive documentation — Workspace layouts — https://docs.kdenlive.org/en/user_interface/workspace_layouts.html
- Kdenlive documentation — Welcome and splash screen — https://docs.kdenlive.org/en/user_interface/welcome_and_splash_screen.html
- Kdenlive documentation — Settings menu — https://docs.kdenlive.org/en/user_interface/menu/settings_menu.html
- Kdenlive documentation — Configure shortcuts — https://docs.kdenlive.org/en/user_interface/menu/settings_menu/configure_shortcuts.html
- OpenShot user guide — Preferences — https://www.openshot.org/static/files/user-guide/preferences.html
- AccessKit — https://github.com/AccessKit/accesskit
- egui API documentation — https://docs.rs/egui/latest/egui/
- sysinfo crate documentation — https://docs.rs/sysinfo/latest/sysinfo/
- rfd (native dialogs) crate documentation — https://docs.rs/rfd/latest/rfd/
- winreg crate documentation — https://docs.rs/winreg/latest/winreg/
- zip crate documentation — https://docs.rs/zip/latest/zip/
- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
