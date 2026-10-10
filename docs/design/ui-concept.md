# FreeCut UI concept: "Quiet Stage"

> Status: draft, **revision 2** ("floating panels") · 2026-10-13 · Scope: the look, layout
> and interaction grammar of the FreeCut editor window. Behaviour, limits and undo rules stay
> in the feature specs (`docs/features/*.md`, mainly `workspace-and-shortcuts.md`, UI-xxx).
> Where this page and a spec disagree about *presentation* (where a control sits, what it
> looks like, which token it uses), this page wins. Where they disagree about *behaviour*
> (what an edit does, its ranges, its undo entry), the spec wins. · Crates: freecut-app (all
> of it), freecut-render (preview thumbnails and on-canvas overlays only), freecut-media
> (filmstrip frames for the timeline, decoded off the UI thread)

## Contents

1. [The idea in one paragraph](#1-the-idea-in-one-paragraph)
2. [Design principles](#2-design-principles)
3. [Window anatomy](#3-window-anatomy)
4. [Navigation model](#4-navigation-model)
5. [Layout variants](#5-layout-variants)
6. [The inspector](#6-the-inspector)
7. [The text inspector](#7-the-text-inspector)
8. [Shelf and Drawer (the asset browser)](#8-shelf-and-drawer-the-asset-browser)
9. [Stage and direct manipulation](#9-stage-and-direct-manipulation)
10. [Command bar](#10-command-bar)
11. [Assistant: its place in the UI](#11-assistant-its-place-in-the-ui)
12. [Design tokens](#12-design-tokens)
13. [Component inventory](#13-component-inventory)
14. [Iconography and fonts](#14-iconography-and-fonts)
15. [Accessibility checklist](#15-accessibility-checklist)
16. [Mapping to egui 0.36](#16-mapping-to-egui-036)
17. [Rollout slices](#17-rollout-slices)
18. [Open questions](#18-open-questions)
19. [Clean-room notes](#19-clean-room-notes)
20. [References](#20-references)

---

## Revision 2 at a glance

Revision 1 (2026-10-12) set FreeCut's grammar: a Stage, a Shelf and Drawer, lenses over
property cards, a Command bar and an Assistant face. Revision 2 keeps all of that and changes
how the window *feels*: calmer surfaces, one rhythm of sizes, and smaller, lighter type. The
layout and comfort patterns come from studying an open-source editor as a design reference
(§20); every value below is FreeCut's own. Sections that changed start with a *Rev. 2* note.

| Area | Revision 1 | Revision 2 | Where |
|---|---|---|---|
| Window | Flat panels touching each other, 1 pt dividers | **Floating panels**: rounded cards (`r.lg`) on a darker ground, 8 pt gutters instead of divider lines | §3.1, §3.4, §13.16 |
| Top bar | 44 pt, Command bar in a wide centre field | **Slim 36 pt bar** on the ground colour: menus, project name + save state, Command bar, Assistant, gear, accent **Export** | §3.5, §10 |
| Surfaces | Fields sunken (`bg.well`) | Controls **lifted** one step above the panel (new `bg.field`); hairlines translucent; new `border.strong` | §12.1 |
| Asset browser | Vertical Shelf rail + Drawer with a chip row | **Shelf = tab strip** across the asset panel (icon over label); **Drawer = side list with counts** + tile grid; chips only when the panel is narrow | §8 |
| Stage | Canvas fills the region, transport row underneath | **Panel header** with the output readout; canvas on a framed surround; **transport bar** with play exactly centred and a scrubbable timecode field | §9 |
| Inspector | Lens bar + rounded cards with gaps | Panel header 48 pt + **lens strip** 40 pt; cards drawn as **full-width sections** with hairlines; numeric fields hug the right edge and carry a **unit segment** | §6, §7 |
| Timeline | Toolbar + letter toggles, solid clips | **Sequence tabs**, an **icon toolbar**, icon track headers, **two-tone clips** (name strip + body) with **filmstrips** and bar waveforms, a playhead with a **head** | §13.26 |
| Type | 13 pt body, Semibold titles | **12 pt body**, 11 pt secondary, **Medium** titles | §12.6 |
| Radii | 4 / 8 / 12 | 3 / 5 / 7 / 12 / 16 | §12.5 |
| New components | — | Panel header, gutter splitter, tool button, unit field, select, colour field, help and reset badges, side list, media tile, thin scrollbar, transport, timeline surfaces, import button, sheets, Home grid | §13.16–§13.29 |
| Unchanged | Iris accent, clip colours, Phosphor icons, lenses, Detail switch, Command bar, Assistant behaviour, keyboard model, undo rules | — | — |

---

## 1. The idea in one paragraph

*Rev. 2: panels float on a ground; the asset browser, Stage and timeline have their own
headers and bars.*

The picture is the loudest thing on screen; everything else is quiet until it is needed.
FreeCut's window is a dark **ground** on which a few rounded **panels** float, separated by
narrow gutters instead of lines. The **Stage** (the player and its canvas) sits in the middle
panel, surrounded by four calm tools: the asset panel, whose **Shelf** of category tabs opens
a **Drawer** of everything you can add; an **Inspector** for the selected item (a strip of
*lenses* over a stack of identical *property sections*); the **Timeline**, with sequence tabs,
an icon toolbar and two-tone clips; and an **Assistant** (the AI editing assistant, AS-001)
that shares the inspector's panel and proposes edits as plans you can preview before anything
changes. One **Command bar** in the slim top bar finds any command, setting or asset, and hands
anything it can't match to the Assistant. One control height, one row rhythm and one inset run
through the whole window, colour fades are short, the keyboard reaches everything, advanced
settings stay folded until asked for, and every change can be previewed on hover and undone in
one step.

## 2. Design principles

| # | Principle | What it means in practice |
|---|---|---|
| P1 | **Content first, chrome quiet** | Neutral greys in one ordered ladder (§12.1); colour is reserved for media, clip kinds, the single accent and status. Panels are separated by gutters of ground, sections inside them by translucent hairlines; no gradients, no shadows on panels, no decorative borders. *(Rev. 2: gutters and hairlines replace divider lines.)* |
| P2 | **One accent, one meaning** | The accent (Iris) means "focus, selection, an active mode or the primary action". It is never used for decoration, large fills or a clip kind. |
| P3 | **Show the common, reveal the rest** | Every section shows its essential rows; advanced rows sit behind a *More* row. Optional style layers (outline, shadow, glow…) appear only when switched on, via *Add layer*. An inspector-wide **Detail** switch (Essentials / Everything) sets the default. |
| P4 | **Every property looks the same** | One section grammar and one row grammar (label · control · gutter) for every clip kind, so reset and keyframe controls are always in the same place. |
| P5 | **Try before you commit** | Hovering a preset, filter, animation or style previews it on the selected clip (UI-007). Assistant proposals preview as ghosts. Nothing is applied until a click, `Enter` or drop. |
| P6 | **Keyboard first** | Every surface is reachable with `F6` / `Ctrl+1…5`, every control with `Tab`, every command from the Command bar. Shortcuts are shown where the action lives. |
| P7 | **Nothing is scary** | Each gesture is one undo entry; `Esc` backs out of any drag, edit or preview; destructive actions name what they destroy. |
| P8 | **Accessible by default** | Contrast-checked tokens, a visible keyboard focus ring, shape plus colour for every state, reduced motion, and a scalable UI with two densities. |
| P9 | **The assistant proposes, the user decides** | The Assistant never edits silently. Its output is a proposal of ordinary edit requests that the user previews, trims and accepts as one undo entry. |
| P10 | **One rhythm** *(new in rev. 2)* | Controls are 28 pt tall on a 34 pt row pitch, every panel header is 48 pt, every panel and row uses a 14 pt side inset, every gutter is 8 pt. Controls sit *one step lighter* than the surface under them. Type is small and light (12 pt body, Medium titles). Hover and press are colour fades of `dur.fast`. Big targets, small visuals. |

**What makes it ours.** Revision 2 deliberately adopts a few conventions that make editors
comfortable to use, studied in an open-source editor (§20): floating panels with gutters, a
tab strip plus side list in the asset browser, a centred transport, two-tone timeline clips
and full-width property sections. These are widely shared patterns (the four-region layout,
category tabs, a side list of sub-categories with counts, a centred play button) rather than
anything owned by one product. What FreeCut draws and operates its own way: its graphite
palette with a slight cool tint and the Iris accent; Phosphor icons and its own editing
glyphs; categories grouped by intent; lenses with an Essentials / Everything **Detail**
switch, *More* rows and opt-in style layers; a central **Command bar** that hands requests to
an **Assistant** face; proposal ghosts; a context strip over the Stage; a pentagon-headed
playhead; keyboard reach to every control; and its names (Quiet Stage, Shelf, Drawer, Stage,
lens, Iris). Compared with the reference app the feature specs observe, FreeCut still keeps
its own arrangement details, sizes, tokens and wording (§19).

## 3. Window anatomy

### 3.1 Main window, landscape project (1440 × 900 pt, Comfortable density)

*Rev. 2: redrawn. Everything below the top bar is a floating panel on the ground.*

```
 ground (bg.app): the top bar, the 8 pt gutters and the status line are all ground
+-----------------------------------------------------------------------------------------------------+
| ooo File Edit Clip Timeline View  Beach trip - Saved  [/ Find or ask...  Ctrl K]  (AI) (*) [Export] |
|  .---------------------------. .------------------------------------------. .---------------------. |
|  | [M]   [A]   [T]  [E]  [X] | | Beach trip        1920 x 1080 - 30 fps   | | [T] Title 1 E|A (AI)| |
|  | Media Audio Text Elem FX  | |                        [Fit v] [safe]    | |   V3 > 4.0-9.0 s    | |
|  |---------------------------| |------------------------------------------| |---------------------| |
|  | v Media  | [Import] [o v] | |   .------------------------------------. | | [Content|Style|Box.]| |
|  |  All   9 | +-----+ +-----+| |   | Write here Style Animate Split ... | | |---------------------| |
|  |  Video 6 | |     | |     || |   '------------------------------------' | | Type  v       ? o ..| |
|  |  Audio 2 | | 0:22| | 0:09|| |   +------------------------------------+ | |  Font  [Inter     v]| |
|  |  Image 1 | +-----+ +-----+| |   |                                    | | |  Size   [ 64 |px]<K>| |
|  | v Mine   | beach   waves  | |   |            S T A G E               | | |---------------------| |
|  |  Fav   2 | +-----+ +-----+| |   |                                    | | | Fill  v   #FFF 100% | |
|  |  Recent 5| |     | |     || |   +------------------------------------+ | |---------------------| |
|  |          | +-----+ +-----+| |------------------------------------------| | Outline >  2px [on] | |
|  |          |                | |[0:04:12] / 1:20:00 |< < > > >| Full [fit]| | + Add layer         | |
|  '---------------------------' '------------------------------------------' '---------------------' |
|  .------------------------------------------------------------------------------------------------. |
|  | [Sequence 1] [Intro]  +                                                                    ... | |
|  | [+] (<)(>) | [split][del] | [freeze][rev] |   [voice] | [snap][mag][skim] | - -o- + [fit] ...  | |
|  |----------------------|-------------------------------------------------------------------------| |
|  |                      | 00:00:00      00:00:05      00:00:10  [#]   00:00:15                    | |
|  | [T] V3   (o)(lk) ... |                [=Title 1=====]         |                                | |
|  | [M] V1 * (o)(lk) ... | [beach.mp4 #########][waves.mp4 #######|###][sunset.mp4 ######]         | |
|  |                      | [~~~~~~~~~~~~~~~~~~~][~~~~~~~~~~~~~~~~~|~~~][~~~~~~~~~~~~~~~~]          | |
|  | [A] A1   (s)(lk) ... | [music.wav ~~~~~~~~~~~~~~~~~~~~~~~~~~~~|~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~] | |
|  '------------------------------------------------------------------------------------------------' |
|  Exporting 62 % - 2 more tasks                                       1/2 res - Proxy on - 29.97 fps |
+-----------------------------------------------------------------------------------------------------+
```

Legend: `ooo` macOS window buttons (the content runs under the title bar there) · `(AI)`
Assistant toggle · `(*)` Preferences · `[M] [A] [T] [E] [X]` Shelf tabs (icon over a short
label) · `[o v]` sort select · `E|A` the Detail switch · `<K>` keyframe navigator (§13.6) ·
`|< < > > >|` transport · `*` main track marker · `(o)(lk)(s)` track header icon buttons ·
`[#]` playhead head · `###` filmstrip · `~~~` waveform.

### 3.2 Regions

*Rev. 2: sizes and the panel shell changed; the region list and its spec owners did not.*

| Region | Default size (Comfortable) | Minimum | Role | Spec owner |
|---|---|---|---|---|
| **Top bar** | 36 pt high (32 Compact), on the ground | 32 | Menus, project name + save state (UI-012), **Command bar** (§10), Assistant toggle, Preferences, **Export** | UI-012, UI-034 |
| **Asset panel** | 26 % of the width (≈ 366 pt at 1440) | 240 (collapsed: 44 pt rail of Shelf icons) | **Shelf** tab strip (48 pt) over the **Drawer**: side list (112 pt) + tile grid | UI-004..UI-007 |
| **Stage panel** | the remainder | 320 pt wide | Panel header with output readout, context strip (§9.3), canvas, transport bar (44 pt) | EX (player), TR-007 (handles) |
| **Inspector panel** | 24 % (≈ 338 pt at 1440) | 280 (collapsed: 44 pt rail) | Inspector face (§6) or Assistant face (§11) | UI-008, UI-009 |
| **Timeline panel** | 42 % of the height between top bar and status line | 160 | Sequence tabs (32), toolbar (40, UI-011), ruler (28), track headers, lanes | TL, UI-011 |
| **Status line** | 22 pt, on the ground | 22 | Task indicator (UI-040), playback state chips | UI-040 |
| **Gutters** | 8 pt between panels and around them (4 in Compact) | — | Resize handles (§13.17); the only separation between panels | UI-002 |

Fixed positions, resizable splits (UI-002). A collapsed asset panel leaves a 44 pt rail of
Shelf icons; a collapsed inspector leaves a 44 pt rail of lens icons plus the Assistant icon.
Rails are floating panels too.

### 3.3 Layering (z-order)

*Rev. 2: a ground level and lifted controls added; panels have no shadow.*

0. **Ground** (`bg.app`): top bar, gutters, status line.
1. **Panels** (`bg.panel`, `r.lg`, no outline or shadow; High contrast adds a 1 pt
   `border.subtle` outline because ground and panel are both black).
2. Inside a panel: **wells** sunk one step (`bg.well`: segmented tracks, slider rails, unit
   segments, timeline lanes, the sequence tab strip), **bands** raised one step (`bg.card`:
   section headers, the current tab), and **controls lifted** one more (`bg.field`).
3. Floating strips: Stage context strip, timeline drag ghosts, a peeking asset panel (`e2`).
4. Popovers, menus, tooltips, toasts (`e2`).
5. Command bar results, modal sheets (`e3`, with `overlay.scrim` behind modals only).

### 3.4 Panel shell *(new in rev. 2)*

- **Panel:** a rounded card (`r.lg` = 12) in `bg.panel`. Content is clipped to the rounded
  corners: anything full-bleed (header hairlines, timeline lanes, the bottom section of a
  scrolled list) must not square the corners off (§16.3).
- **Panel header:** 48 pt (40 Compact), side inset 14 pt (10 Compact), a 1 pt `border.subtle`
  hairline along its bottom edge. Title in `type.label`, trailing icon buttons 2 pt apart. The
  Stage and the inspector use one; the asset panel's header *is* its Shelf tab strip and the
  timeline's header is its sequence tab strip, so the tops of all panels line up.
- **Panel body:** padding 14 pt sides (10 Compact) unless a section is full-bleed; 6 pt between
  children (4 Compact).
- **Gutters:** 8 pt of ground between panels and around the outside (4 Compact). A gutter is
  the resize handle of the split it separates: invisible at rest, a 2 pt `accent` line inset
  12 pt from both ends fades in on hover (`dur.fast`) and stays while dragging; the grab area
  is the gutter plus 2 pt on each side. Double-click resets the split (UI-002).
- **Focused place:** the focused panel shows a 2 pt `accent` line along the straight part of
  its top edge (inset 12 pt from both ends, so it never meets the rounded corners) — the
  rev. 1 rule of UI-001, redrawn for rounded panels.
- **Scrolling bodies** scroll with the wheel and trackpad only (mouse drags never scroll, so
  drags on fields, rails and handles are never stolen) and show the thin scrollbar of §13.24.

### 3.5 Top bar *(new in rev. 2; replaces the 44 pt bar)*

36 pt high (32 Compact), filled with the ground colour and with no bottom line, so the bar and
the gutters read as one frame around the panels. Left to right:

1. **Window buttons inset.** macOS: the content runs under a transparent title bar; 76 pt are
   kept free on the left for the system's window buttons. Windows / Linux: native decorations
   stay for now; FreeCut-drawn caption buttons are a later option (§18).
2. **Menus** *File Edit Clip Timeline View Playback Help* as text labels (`type.body`,
   `text.secondary`; hover or open: `bg.field` chip, `r.sm`, `text.primary`), 8 pt padding on
   each side, 2 pt apart. Below the Compact breakpoint they fold into one `≡` menu button.
3. **Project name** (`type.body`, `text.secondary`, middle ellipsis, click to rename in v1)
   and, 6 pt after it, the **save state** of PM-012 in `type.small`: `text.tertiary` for
   *Saved* / *Saving…*, `warning` for *Unsaved changes*, `danger` for *Save failed — Retry*.
4. **Command bar** field, centred on the whole bar: 26 pt tall, 360 pt wide (§10).
5. Right group (right to left): **Export** (§13.18: 26 pt tall, `r.sm`, `accent` fill,
   `text.on-accent`, export icon 12 pt + label in `type.label` at 12 pt, 10 pt side padding;
   hover `accent.hover`; it is the only filled accent control in the bar), 6 pt, the
   **Preferences** gear and, in v1, the **shortcuts** button (26 pt icon buttons), then the
   **Assistant** toggle `(AI)` (an icon button showing the checked state while its face is
   shown).
6. Empty bar space is a window handle: pressing and moving more than 4 pt starts a window
   drag; double-click toggles maximise.

On the Home screen (UI-016) the same bar shows only the FreeCut name on the left and the
shortcuts button and Preferences gear on the right.

## 4. Navigation model

*Rev. 2: the Shelf is now a tab strip, so its keys are Left / Right; the focus line follows
the rounded panels.*

Five places, one way to move between them:

| Place | Jump | What lives there | It answers |
|---|---|---|---|
| Shelf + Drawer | `Ctrl+1` | Media, audio, text, elements, effects, transitions, looks, captions, templates | "What can I add?" |
| Stage | `Ctrl+2` | The picture, on-canvas handles, context strip, transport | "What does it look like?" |
| Inspector | `Ctrl+3` | Lenses and sections for the selection | "How is this item set up?" |
| Timeline | `Ctrl+4` | Sequence tabs, toolbar, tracks, clips, playhead, keyframe lanes | "When does it happen?" |
| Assistant | `Ctrl+5` (proposed) | Conversation and proposals | "Can you do this for me?" |
| Command bar | `Ctrl+K` (proposed), `Ctrl+Shift+P` (UI-031) | Any command, setting, asset, or a request to the Assistant | "Where is…?" / "Do…" |

`F6` / `Shift+F6` cycles the places in the order above (UI-041). The focused place shows a
2 pt accent line along the straight part of its panel's top edge (§3.4, UI-001). New keys are
proposals until they are entered in UI-031's keymap.

**Selection drives the right panel.** Selecting a clip shows its lenses; selecting nothing
shows the Project sections; selecting a keyframe pins a *Keyframe* section on top (UI-008).
The inspector header shows a breadcrumb (track › time range) whose parts are links: the track
part selects the track, the time part moves the playhead to the clip start.

**The Shelf is for adding, the Inspector is for changing.** The same preset library (for
example text looks) appears in both places, filtered the same way: in the Drawer a click
*adds* a new item at the playhead; in the inspector's Presets lens a click *applies* to the
selection. Hover previews behave the same in both (UI-007).

**Typical flow: add a title.** `Ctrl+1` focuses the Shelf; `Right` to *Text*; `Down` enters
the Drawer's side list, `Right` moves into the grid; `Enter` on *Add text* → a text clip is
placed at the playhead (TX-001), selected, and the inspector opens on **Content** with the text
field focused and its words selected → type → the Stage updates per keystroke → `Tab` moves to
the lens strip, `→` to **Style** → pick a font → `Esc` returns focus to the timeline. Mouse
users can drag the *Add text* tile onto the Stage (placed at the drop point) or onto a track
(placed at the drop time).

## 5. Layout variants

### 5.1 Breakpoints

*Rev. 2: Compact also shrinks gutters and headers; the side list folds into chips when the
asset panel is narrow.*

| Window width (pt, after UI scale) | Name | Behaviour |
|---|---|---|
| ≥ 1680 | **Wide** | The Assistant may open as its own 320 pt panel beside the inspector (both visible). |
| 1280 – 1679 | **Standard** | §3.1. Inspector and Assistant share one panel (face switch). |
| 1024 – 1279 | **Compact** | Compact density metrics unless the user picked a density explicitly (Density = *Automatic* is the default): 4 pt gutters, 40 pt panel headers, 32 pt top bar. Shelf tabs show icons only, with tooltips; the lens strip is icon-only with tooltips; menus fold into `≡`. The asset panel may collapse to its rail; choosing a tab on the rail opens the panel as a *peeking* overlay over the Stage's left edge (`e2`), which closes after an add or `Esc`, unless pinned. |
| < 1024 (reachable only with an interface size above 100 %, since UI-001's minimum window is 1024 × 640) | **Tight** | Inspector collapses to its 44 pt rail; choosing a lens opens it as a 320 pt overlay sheet over the Stage's right edge; timeline minimum 160 pt. |

Independently of the window, an asset panel narrower than 360 pt hides its side list and shows
the same entries as a one-line chip row above the grid (§8.2).

Small laptops (1366 × 768 at 125 % scaling ≈ 1093 × 614 pt) land in **Compact**. The Stage
always keeps at least 320 pt of width; when it would shrink further, the asset panel collapses
to its rail first, then the inspector collapses to its rail (UI-002's order, restated with
these names).

### 5.2 Portrait projects (canvas taller than wide)

A 9:16 picture needs height, not width. When the active sequence is portrait and the user
picked the *Tall stage* layout (UI-003's *Vertical video*; offered once by its portrait
hint):

```
+------------------------------------------------------------------------------------------+
| ooo File Edit ...  Reel 03 - Saved     [/ Find or ask ...  Ctrl K]     (AI) (*) [Export] |
|  .-----------------------------. .--------------------. .-----------------------------.  |
|  | Shelf tabs (icons + labels) | | Reel 03  1080x1920 | | Inspector                   |  |
|  |-----------------------------| |--------------------| |  (wider: label column 96 pt,|  |
|  | side   | grid (3 columns)   | |  .--------------.  | |   two-column rows for X/Y)  |  |
|  | list   |                    | |  | context strip|  | |                             |  |
|  |        |                    | |  '--------------'  | |                             |  |
|  |        |                    | |   +------------+   | |                             |  |
|  |        |                    | |   |   STAGE    |   | |                             |  |
|  |        |                    | |   |   9:16     |   | |                             |  |
|  |        |                    | |   +------------+   | |                             |  |
|  |        |                    | | 0:04 / 0:30  |>|   | |                             |  |
|  '-----------------------------' '--------------------' '-----------------------------'  |
|  .------------------------------------------------------------------------------------.  |
|  | timeline (34 % height)                                                             |  |
|  '------------------------------------------------------------------------------------'  |
+------------------------------------------------------------------------------------------+
```

Upper area 66 %, timeline 34 %; asset panel and inspector each take about 30 % of the width;
the Stage panel is sized to the picture's aspect plus its 16 pt surround margin, so no width
is wasted on side letterboxing. Square projects use Standard.

### 5.3 Focus modes (built on UI-003 layouts)

| Mode | Built-in layout (UI-003) | When |
|---|---|---|
| **Cut** | Big timeline | Assembly and trimming |
| **Style** | Inspector focus | Text, color, keyframes |
| **Review** | Big preview | Watching, framing |
| **Browse** | Library focus | Picking media and assets |

Modes are only named layouts; switching is instant with reduced motion and a 160 ms ease
otherwise. `` ` `` still maximizes the focused place (UI-048).

## 6. The inspector

### 6.1 Anatomy

*Rev. 2: the header is a 48 pt panel header, lenses sit in their own 40 pt strip, and cards
are drawn as full-width sections separated by hairlines.*

```
.--------------------------------------------.
| [K] Name of item          E|A  < >  (AI)   |  panel header 48: kind icon, name (type.title),
|     V2 > 0:02.0 - 0:07.5                   |  breadcrumb, Detail switch, prev/next, Assistant
|--------------------------------------------|  hairline
| [ Lens | Lens | Lens | Lens | Lens ]       |  lens strip 40: segmented control, 28 tall
|--------------------------------------------|  hairline
| ! Track is locked              [Unlock]    |  banner slot (locked / offline / grouped)
|############################################|  section header band 36 (bg.card)
| Section title  v    summary    ? (r) [on].. |
|  Label                      [  64 | px ]<K>|  parameter row: field hugs the gutter
|  Label     [ Select                   v ]  |  form row: label column 76
|  More (3)                                  |
|--------------------------------------------|  hairline between sections
| Collapsed  >    2 px - black   (r) [on] .. |  folded section: summary instead of rows
| + Add layer                                |
|--------------------------------------------|
| [ Save to presets... ]       [ Reset lens ]|  footer (only where it applies)
'--------------------------------------------'
```

- **Panel header (48 pt):** kind icon (never colour alone, UI-043), the item's name in
  `type.title` (click to rename, TL) and below it the breadcrumb in `type.small`
  `text.tertiary` (§4); trailing: the Detail switch (an icon segmented control, 56 pt wide),
  *previous / next item on this track* (`Alt+Left/Right` in the timeline already does this,
  UI-041), and the Assistant face button (§11.1).
- **Lens strip (40 pt, 34 Compact):** a segmented control (§13.4) filling the panel width minus
  the 14 pt insets, with a hairline below. Lenses are the tabs of UI-008, renamed and regrouped
  where noted below. Labels in Standard, icon-only with tooltips in Compact. The last lens per
  selection kind is remembered (UI-008 tab memory). A kind with a single lens has no strip; the
  lens name is shown in `type.micro` capitals at the end of the breadcrumb line.
- **Banner slot:** at most one banner, highest severity first (locked > offline > grouped).
- **Section stack:** one scroll area, full width, no gaps (§6.2). Sections remember collapsed
  state per (selection kind, section) (UI-008). With the Detail switch on *Essentials*,
  sections show only essential rows and optional layers that are switched off are hidden
  behind *Add layer*.
- **Footer:** *Save to presets…* opens UI-064's dialog with the lens's owner checklist;
  *Reset lens* resets every section in the lens as one undo entry.

### 6.2 Property card grammar

*Rev. 2: cards are drawn as full-width **sections** ("card" stays the code name); the chevron
follows the title; help and reset badges join the header; numeric rows hug the right edge;
per-row reset moves to the label.*

Every section in every lens follows the same grammar:

```
 header:  Title  v      summary when collapsed      ?  (r)  [switch]  [...]
 rows:    Label  ................  control  [unit]   gutter
 more:    More (n)   -> reveals advanced rows in place
```

- **Layout:** sections run edge to edge inside the panel (no radius, no gap). A 1 pt
  `border.subtle` hairline sits above every section except the first.
- **Header band** (36 pt Comfortable / 30 pt Compact): `bg.card` fill; on hover `bg.field`
  (`dur.fast`); the whole band toggles the section. Left to right: title (`type.label`,
  starting at the 14 pt inset so titles line up with row labels); chevron after the title
  (12 pt, `text.secondary`: down when open, right when folded); flexible space; when folded, a
  *summary* that states the current value in a few words ("2 px · black", "Fade · 0.5 s",
  "off") in `type.caption` with tabular figures, `text.tertiary`, cut with an ellipsis; then
  the action slot, 2 pt apart: a **help badge** (§13.21) where the section needs explaining, a
  **reset badge** (dimmed to 40 % when nothing differs from the defaults, never hidden), the
  on/off **switch** for optional layers (outline, shadow, glow, background, curve, mask,
  key…), and the `...` menu with *Reset section*, *Copy settings*, *Paste settings* (TL-034's
  attribute clipboard), *Save to presets…* where the owner supports it, and *Remove layer* for
  optional layers. Badges and the switch take their own clicks, so pressing them never folds
  the section.
- **Body:** 4 pt above the first row, 8 pt below the last.
- **Rows** (control 28 pt on a 34 pt pitch; Compact 24 on 28): two shapes.
  - *Parameter rows* (numbers): the label hugs the left, the control hugs the right edge next
    to the gutter, and short labels leave air in the middle. The number field carries its unit
    in a trailing segment (§13.19), so the value is typed bare and said once. Only the one value
    a section is about (Volume, Opacity, Fade length) also gets a slider rail, as a two-line
    row: label alone on the first line, rail + field 4 pt below.
  - *Form rows* (selects, colours, fonts, paired fields): a label column of 76 pt (64 Compact;
    every row of a section uses the same width so controls line up), then the controls
    stretched across the rest; two-value fields (X and Y, W and H) split the width equally.
  - **Label:** `type.body`, `text.secondary`, cut with an ellipsis; dragging it scrubs the value
    (UI-009, horizontal-resize cursor); double-clicking it resets the value. A value that
    differs from its default shows a 4 pt `accent` dot just before the label (shape + colour:
    the dot is absent otherwise).
  - **Gutter** (56 pt; 50 Compact): holds the keyframe navigator (§13.6) on animatable rows.
    An empty gutter keeps its width so controls line up across sections.
  - **Reset** of one row: double-click the label, `Ctrl+Backspace` on the focused row
    (proposed), or the row's context menu; the section's reset badge resets the section.
- **More (n):** advanced rows of the section, revealed in place; the state is remembered with
  the section. Everything is still reachable with the keyboard and the Command bar
  ("Go to setting: Shadow blur").
- **Switched-off layers** keep their values and keyframes (TX-021); the section folds to its
  header and the summary reads "off". Rows that only matter once a layer has a size (outline
  colour while its width is 0) are hidden, not disabled.

### 6.3 Lenses by selection kind

Lenses present the property groups of UI-008's table; UI-008 stays the owner of which
groups exist. Names in **bold** are the lens labels.

| Selection | Lenses → cards |
|---|---|
| Nothing | **Project** → Canvas, Frame rate, Background, File |
| Video clip | **Picture** → Placement (transform, fit, flip), Opacity & blend, Crop, Canvas fill · **Mask & key** → Masks, Chroma key, Stabilize · **Look** → pages Basic (Auto, Match, Light, Color, LUT, Filter, Detail & finish), HSL, Curves, Wheels, Masks (CL-043) · **Effects** → effect stack · **Motion** → In / Loop / Out, Keyframes, Speed · **Sound** (if linked audio) → Volume, Fades, Pan, Voice |
| Image clip | **Picture** · **Look** · **Effects** · **Motion** (no Speed card) |
| Audio clip | **Sound** → Volume, Fades, Pan, Normalize · **Voice tools** · **Motion** → Keyframes, Speed |
| Text / title | **Content** · **Style** · **Box & bubble** · **Motion** · **Presets** (§7) |
| Caption item | **Content** (segment text, timing) · **Style** (track style vs. this segment's override) · **Motion** · **Presets** |
| Sticker / shape | **Element** (shape or sticker parameters) · **Picture** → Placement, Opacity · **Motion** · **Presets** |
| Effect layer | **Effect** → parameters |
| Filter / adjustment layer | **Look** (the same pages; Amount reads Mix; CL-043) |
| Transition | **Transition** → type, duration, parameters |
| Keyframe(s) | the clip's lenses plus a pinned **Keyframe** card (time, easing, curve) on top |
| Marker | **Marker** |
| Track header | **Track** |
| Several items | shared lenses only (UI-010); mixed values show "—" |

The **Motion** lens is the same everywhere: the In / Loop / Out timing strip (§13.11) and
preset grid where the clip kind has animation slots (FX-018, FX-019, TX-022), then a
**Keyframes** card listing every animated property of the clip with a navigator per row,
then *Speed* for clips that can be retimed. This gives the user one place to answer "what
moves, and when?".

### 6.4 Multi-selection, locked and offline items

Unchanged behaviour from UI-008 / UI-010; visually, mixed values show an em dash in the
control, keyframe gutters are empty, and a banner states what will be skipped. *Rev. 2:* a
one-line `type.small` sentence at the top of the stack names the selection ("Editing 3 titles");
sections that cannot apply to several items are left out rather than shown disabled.

*R4 decisions (2026-10-13):* the reset arrow left the row gutter: a changed row shows the
accent dot, and its label resets it on a double-click or from its context menu; the gutter holds
only the keyframe navigator, whose arrows are always drawn (greyed when there is no key that
way). Numeric rows are number fields with a unit segment; a rail stays on the one value a section
is about (Size, Opacity, Volume) as a two-line row, and Size keeps TX-054's −/+ steppers between
the rail and the field. A segmented choice too wide for the form column moves under its label.
The lens strip shows words, each segment as wide as its word, and drops to icons when the words
don't fit. In Light the segmented thumb is `bg.card` with a hairline, because `bg.hover` hardly
differs from `bg.well` there. Number fields centre their value (egui's drag value centres it;
typing is centred too so the text never jumps). Section reset badges exist where a section
reset already existed (the text sections and Placement); Volume and Fades keep *Reset lens* in
the footer. Previous / next item buttons and the Assistant face wait for their features.

## 7. The text inspector

The text inspector is the model for every other kind. Five lenses:

| Lens | Cards (Essentials rows **bold**; others behind *More*) | Specs |
|---|---|---|
| **Content** | *Text*: **multi-line editor** (on-canvas editing mirrors it), find & replace link · *Paragraph*: **alignment**, **wrap / box width**, vertical anchor, direction · *Speech* (later): read aloud as a voice clip, dictate · *Convert*: to captions | TX-002, TX-005, TX-015, TX-017, TX-034, TX-035, TX-043, TX-045 |
| **Style** | *Type*: **font** (picker with live previews), **size**, **emphasis** (bold, italic, underline, strike), **case** (as typed / UPPER / lower / Title), weight · *Fill*: **colour**, mode (solid / gradient / texture) · *Spacing*: **letters**, **lines** · optional layers via *Add layer*: *Outline*, *Shadow*, *Glow* | TX-003, TX-004, TX-008, TX-009, TX-010..TX-012, TX-014, TX-046..TX-048 |
| **Box & bubble** | *Placement*: **position**, **scale**, **rotation**, **opacity**, blend · optional layers: *Background* (**colour**, **opacity**, **shape**, **padding**, **roundness**; height, border), *Bubble* (**kind**, **tail**, **colour**; tail width and bend, padding, roundness, border), *Curve* (**amount**) — Background and Bubble are never on together · *Checks* (later): readability and safe-area warnings | TR-001, TR-015, OV-004, TX-013, TX-016, TX-055, TX-049 |
| **Motion** | *Timing strip* In / Loop / Out · *Preset grid* for the selected slot (live thumbnails) · *Pieces*: by **block / line / word / letter**, overlap, order · **length** (or pace), easing, direction, **intensity** · *Keyframes*: animated properties with navigators · *Follow* (v1, AI-008): attach to a tracked object | TX-021..TX-023, TX-056, FX-018, FX-019, FX-030, AI-008 |
| **Presets** | *Looks* (FreeCut's style presets) · *My looks* · *Templates* · *Default for new text* — all as preset grids with hover preview | TX-019, TX-020, TX-024 |

*Rev. 2 presentation notes:* the font picker trigger previews the chosen family by setting its name in that family (a
missing font turns the trigger `danger`-tinted and reads "Missing · ‹name›"); weight, easing
and other short lists use the Select of §13.20; colours use the colour field of §13.5; size,
letters and lines are parameter rows with unit segments (`px`, `%`, `×`); the Shadow and
Background switches carry help badges; *Placement* reads "Centred" in place of `0 %` offsets.

### 7.1 Style lens (Essentials, one optional layer switched on)

```
.--------------------------------------------.
| [T] Title 1                E|A  < >  (AI)  |
|     V3 > 0:04.0 - 0:09.0                   |
|--------------------------------------------|
| [Content|Style|Box & bubble|Motion|Presets]|
|############################################|
| Type  v                            (r) ... |
|  Font       [ Inter                   v ]  |
| *Size                       [  64 | px ]<K>|
|  Emphasis   [ B | I | U | S ]              |
|  Case       [ Aa | AA | aa | Ab ]          |
|  More (1)                                  |
|--------------------------------------------|
| Fill  v                            (r) ... |
|  Color      [# FFFFFF ] [##]               |
|  Opacity                    [ 100 |  % ]<K>|
|  More (2)                                  |
|--------------------------------------------|
| Spacing  v                         (r) ... |
|  Letters                    [   0 |  % ]<k>|
|  Lines                      [ 1.2 |  x ]<K>|
|--------------------------------------------|
| Outline  >    2 px - black   (r) [on]  ... |
| + Add layer: Shadow, Glow                  |
|--------------------------------------------|
| [ Save look... ]             [ Reset lens ]|
'--------------------------------------------'
```

`*` = changed from default (the 4 pt accent dot) · `(r)` = section reset badge ·
`<k>` = animatable, not animated · `<K>` = animated (§13.6) · `[# FFFFFF ] [##]` = hex field
and swatch (§13.5).

### 7.2 Motion lens

*Rev. 2: the Keyframes section is a small dopesheet.*

```
.--------------------------------------------.
| [Content|Style|Box & bubble|Motion|Presets]|
|############################################|
|  clip |=In=|~~~~~~~~ Loop ~~~~~~~~|=Out=|  |  drag a join to change In / Out length
|        0.5 s                      0.5 s    |
| [ In | Loop | Out ]       Pieces [Word  v] |
| +-------+ +-------+ +-------+ +-------+    |
| | none  | | fade  | | rise  | |  pop  |    |  tiles play on hover; the hovered
| +-------+ +-------+ +-------+ +-------+    |  tile also previews on the Stage
| +-------+ +-------+ +-------+ +-------+    |
| | blur  | | wipe  | | drop  | | spin  |    |
| +-------+ +-------+ +-------+ +-------+    |
|  Length                    [ 0.5 |  s ]    |
|  Easing     [ Ease out              v ]    |
|--------------------------------------------|
| Keyframes  v                       (r) ... |
|  |0:04     |0:05     |0:06     |0:07       |  22 pt ruler: press or drag to scrub
|  Size    <K> --<>---------<>-----<>---     |  34 pt lanes; diamonds click to jump,
|  Color   <K> -----<>------------<>----     |  drag to move (one undo entry on release)
|--------------------------------------------|
| Follow an object  >                  v1    |
'--------------------------------------------'
```

The Keyframes section paints its own lanes: a 22 pt ruler, one 34 pt lane per animated
property, 10 pt diamonds (filled `accent` at the playhead key, `bg.panel` with a 1.5 pt
`accent` outline elsewhere, 16 pt hit area), and the active row washed with `accent` at 8 %.
*lane* in the timeline remains available from the row's context menu (TR-029).

### 7.3 Content lens and on-canvas editing

Double-clicking the text on the Stage (or `Enter` with the text clip selected and the Stage
focused) enters edit mode: a caret on the canvas, the Content lens's editor mirrors the same
buffer, and selecting a range in either place scopes Style changes to that range (TX-015).
`Esc` leaves edit mode; the content change is one undo entry on leave (TX-002's coalescing).
*Rev. 2:* the Content editor is a multi-line field at least 64 pt tall, `bg.field`, `r.md`,
8 / 6 pt inner padding, with a 1 pt `accent` border while focused.

## 8. Shelf and Drawer (the asset browser)

*Rev. 2: rewritten. The Shelf is a tab strip across the top of the asset panel, and the
Drawer is a side list with counts beside a tile grid. Rev. 1's vertical rail survives only as
the collapsed asset panel; its chip row survives only for narrow panels.*

```
.--------------------------------------------------.
|  [M]   [A]   [T]  |  [E]   [X]   [~]  |  [L]  .. |  Shelf: tab strip, 48 pt, hairline below
| Media Audio  Text | Elem. Effects Trans. | Looks |  icon 16 over a type.caption label
|--------------------------------------------------|
| v Media    | [ Import ] [/ Search   ]   [o v]    |  content toolbar: 28 pt controls
|   All    9 |  +--------+ +--------+ +--------+   |
|   Video  6 |  |        | |        | |        |   |  tiles: 16:9, cover-cropped,
|   Audio  2 |  |   0:22 | |   0:09 | |   1:05 |   |  duration badge bottom-right
|   Images 1 |  +--------+ +--------+ +--------+   |
| v Made here|  beach.mp4  waves.mp4  intro.mov    |
|   Extract 1|  +--------+ +--------+              |
| v Mine     |  |        | |        |              |
|   Favs   2 |  |        | |        |              |
|   Recent 5 |  +--------+ +--------+              |
'--------------------------------------------------'
  side list 112 pt (resizable) | grid fills the rest
```

### 8.1 Shelf

A strip of category tabs that is also the asset panel's header: 48 pt tall (40 Compact), 4 pt
padding top and bottom, 8 pt at the sides, a 1 pt `border.subtle` hairline along the bottom.
Each tab is an icon (16 pt) above a one-word label (`type.caption`), 6 pt padding, 2 pt
between icon and label, `r.sm`; tabs share the width equally, 2 pt apart. Tabs sit straight on
the panel colour: idle = `text.secondary`, no fill; hover = `text.primary` (`dur.fast`);
current = `bg.field` fill and `text.primary`. No underline and no accent. Tabs are grouped by
intent with a 1 × 20 pt `border.subtle` rule between groups:

| Group | Tabs (UI-004 tab) |
|---|---|
| Bring in | **Media**, **Audio** |
| Write | **Text**, **Captions** |
| Decorate | **Elements**, **Effects**, **Transitions** |
| Grade | **Looks** (Filters), **Adjust** |
| Reuse | **Templates** |

Tabs whose owner feature isn't built are hidden (UI-004). When the tabs don't fit at a
minimum of 56 pt each, they keep that width and the strip scrolls sideways; a 22 pt end
button with a chevron and a fade from the panel colour appears at each end that has more tabs
behind it (click: scroll by 60 % of the strip, `dur.base`). In Compact the labels drop and the
tabs show icons with tooltips. `Left` / `Right` move along the tabs; `Down` or `Enter` moves
into the Drawer.

### 8.2 Drawer

- **Side list** (left): 112 pt by default, resizable between 80 pt and half the panel by a
  1 pt `border.subtle` hairline that turns `accent` on hover or drag (grab area ±3 pt,
  horizontal-resize cursor). Padding 4 pt; rows 1 pt apart; nothing wraps.
  - *Group heading* (26 pt, `r.sm`): chevron (10 pt) and title in `type.caption` Medium,
    `text.secondary`; hover `bg.field` + `text.primary`; click folds the group (remembered per
    tab, workspace state).
  - *Row* (24 pt, `r.sm`): label in `type.small` at an 18 pt indent (past the chevron), the
    item count right-aligned in `type.micro` tabular `text.tertiary` (hidden when zero).
    Idle transparent `text.secondary`; hover `bg.field`; current `bg.active` + `text.primary`.
  - Every Drawer lists *All* first, then the tab's sub-categories, then a **Mine** group with
    *Favorites* and *Recent* (UI-006). The Media Drawer's groups are *Media* (All, Video,
    Audio, Images) and *Made here* (sound extracted from videos, later rendered and generated
    items). Filtering by kind lives only in the side list; the grid has no second filter.
  - `Up` / `Down` move, `Right` enters the grid, `Left` folds a group.
- **Content toolbar** (one 28 pt row, 8 pt padding): the **Import** button (§13.27) on tabs
  that import (Media, Audio, Looks for `.cube` files), the search field (stretches; §13.15),
  and an icon-only **sort** select (§13.20: Date added, Name, Kind; sorting is display-only
  and never reorders the library).
- **Grid** (§13.23): auto-filled columns of at least 96 pt (80 Compact) with 8 pt gaps (6
  Compact), always filling the width exactly; 16:9 thumbnails; names under them. Catalogue
  tabs (Text looks, Effects, Transitions…) show the same tiles with a hover `+` that adds at
  the playhead (UI-005) and a star (favourite). Sections with headers appear only under *All*.
- **Selection in the grid:** click selects; `Shift` / `Ctrl` / `Cmd` click adds; a drag on
  empty floor draws a rubber band (`overlay.selection` + 1 pt `accent`) that selects on
  release. With several selected, a 28 pt strip above the grid (`bg.well`, `r.md`) reads
  "3 selected" with a *Delete* text button (danger on hover). Library selection is view
  state; deleting goes through the owner's remove operation and its undo entry.
- **Narrow panels:** below 360 pt of panel width the side list hides and its entries become a
  one-line chip row (§13.9) under the toolbar, scrolling sideways.
- **Pinned vs. peeking** (only when the asset panel is collapsed to its rail): choosing a tab
  on the rail opens the whole panel as a peeking overlay over the Stage's left edge (`e2`),
  which closes after an add, on `Esc`, or when the pointer has been away for 1.5 s with no
  keyboard focus inside. A pin button in the Shelf keeps it open. Pinning is workspace state.
- **Empty states** use §13.14 with a reason-specific sentence (nothing imported yet / nothing
  of this kind / no favourites yet / no matches).

*R2 decisions (2026-10-13):* the **Audio** Drawer keeps a list view instead of tiles: 40 pt
rows with the kind chip, name, length and origin, the item's waveform at the right when there is
room, and a `+` that adds at the playhead; its side list is *Audio* (All, Imported) and *Made
here* (Extracted). *Mine* appears only once Favorites and Recent (UI-006) exist. Removing library
items follows PM-058: unused items go at once; used items ask first; sound extracted from a
removed video stays, because it reads the file itself.

The **Text** Drawer lists: *Add text* (the first tile of the grid), *My looks* (TX-020 saved
looks), *Looks* (TX-019), *Templates* (TX-024), and *Captions* (import a subtitle file,
TX-031; generate captions, AI-010 later) as side-list rows.

The **Captions** Drawer (TX-057) lists *From speech*, *From a song*, *Type or paste*, *Files*,
then *Looks* and *My styles*, then *Caption list*. The two generating rows show one property
card (§6.2) each, ending in a full-width accent **Generate** button; while a job runs the card is
disabled and a progress line with **Cancel** sits under it. *Looks* is a §13.7 preset grid.
*Caption list* shows TX-026's caption list in the content area until UI-056 lets it float. Once
the Captions tab exists, the Text Drawer's *Captions* row becomes a link that opens it.

## 9. Stage and direct manipulation

*Rev. 2: the Stage panel gets a header and a transport bar; the canvas sits on a framed
surround; zoom and pan are defined.*

```
.---------------------------------------------------------------.
| Beach trip              1920 x 1080 - 30 fps   [Fit v] [safe] |  panel header 48
|---------------------------------------------------------------|
|  player.surround                                              |
|      .-------------------------------------------.  (context  |
|      |  Write here  Style  Animate  Split  ...   |   strip)   |
|      '-------------------------------------------'            |
|      +-------------------------------------------+   16 pt    |
|      |   canvas (1 pt border.strong ring)        |   margin   |
|      |           [o====box====o]                 |            |
|      |             ( 110 % - 15 deg )            |            |
|      +-------------------------------------------+            |
|---------------------------------------------------------------|
| [00:04:12] / 1:20:00   |< <|  (>)  |> >|  (spk) [Full v] [fit]|  transport 44
'---------------------------------------------------------------'
```

### 9.1 Canvas

- **Header (48 pt):** the sequence name (`type.label`), then a flexible space, the output
  readout "1920 × 1080 · 30 fps" in `type.small` tabular `text.tertiary` (a fact, not a
  control), the zoom select (*Fit*, 50 %, 100 %, 200 %, 400 %) and the safe-area toggle
  (EX owns both).
- **Surround:** the canvas sits on `player.surround` with at least 16 pt of margin (8 pt in
  Compact); a 1 pt `border.strong` ring outside the canvas lifts it off the surround. The
  surround stays neutral in every theme (UI-014).
- **Zoom and pan** (EX owns viewer zoom): *Fit* is the default and follows the panel size.
  `Ctrl` / `Cmd` + wheel or pinch zooms about the pointer, proportional to the wheel distance
  (factor `exp(−dy / 400 pt)`, at most ×2 or ÷2 per event), between 10 % and 800 %. While
  zoomed, the wheel pans, middle-drag or `Alt`-drag pans, and at least 64 pt of the canvas
  always stays inside the surround. Double-click on the surround, the *Fit* button or the zoom
  select's *Fit* returns to fitting; the *Fit* button is disabled while fitted.
- Handles (TR-007): 10 pt square corner and edge handles (hit area 20 pt), a rotation knob
  24 pt above the top edge on a 1 pt stem, the anchor as a small cross. Handles are white with
  a 1 pt dark stroke so they read on any footage; the selected item's outline is 1.5 pt
  `accent`. While a handle is dragged, a readout pill 8 pt below the box shows
  "scale % · angle°" (`type.caption` tabular, white on `overlay.badge`, `r.sm`).
- Snapping guides (TR-008) are 1 pt `canvas.guide` lines with a small distance label;
  safe-area overlays (EX) are dashed `canvas.safe`.
- Hover over an unselected visual item shows a 1 pt outline in `canvas.hover`; click
  selects; `Alt`-click cycles through stacked items under the pointer.
- **Transport bar (44 pt, 36 Compact):** panel fill with a hairline on top, 10 pt side padding,
  three groups 12 pt apart. The side groups stretch and the centre group does not, so *play*
  stays exactly centred however wide the timecode is.
  - Left: the **timecode field** (§13.25: 116 × 28, scrubbable) then "/ 00:01:20:00" in
    `type.small` `text.tertiary`.
  - Centre: *go to start*, *previous frame*, **play / pause**, *next frame*, *go to end* as
    26 pt icon buttons 2 pt apart (glyph 16; play 18 in `accent`, hover chip
    `overlay.selection`). Start / previous are disabled on the first frame and next / end on
    the last (40 % opacity, tooltip still explains).
  - Right: the speaker button (mute; a popover holds the preview volume), the **preview
    resolution** select (*Full*, *Half*, *Quarter* with "1:1 / 1:2 / 1:4" as dim details,
    EX-006), and the *Fit* button.

### 9.2 Undo-safe gestures

| Gesture | Live feedback | Commit | Cancel |
|---|---|---|---|
| Drag item / handle | Stage updates every frame; inspector values follow | One undo entry on release (UI-009 coalescing) | `Esc` during the drag restores the pre-drag value, no entry |
| `Shift`-drag | constrains (axis lock / uniform scale / 15° rotation steps) | same | same |
| Arrow keys on a selected item | 1 px nudge; `Shift` 10 px | Consecutive nudges within 1 s merge into one entry | `Ctrl+Z` |
| Double-click text | edit mode (§7.3) | one entry on leaving edit mode | `Esc` then `Ctrl+Z` |
| Hover a preset / look / animation | `PreviewOverride` on the Stage (UI-007), tile animates | Click or `Enter` = one entry | Move away or `Esc` |
| Assistant proposal | ghosts on Stage and timeline as soon as it is ready (§11.3) | *Accept* = one entry | *Discard* or `Esc` |
| Zoom / pan the view | live | not an edit (view state) | double-click the surround to fit |

Undo during a drag is ignored until release (UI-009). Gestures on a locked track show the
lock banner and a "Locked" cursor badge; nothing moves.

### 9.3 Context strip

A floating strip (`e2`, 32 pt high, radius `r.lg`, 26 pt buttons) centred above the canvas,
shown only while something is selected and the pointer or focus is in the Stage. It holds at
most five actions chosen for the selection kind, plus `...` for the rest. Examples:

| Selection | Actions |
|---|---|
| Text | Write here (edit mode, §7.3) · Style (opens Style lens) · Animate (opens Motion lens) · Split · `...` |
| Video / image | Crop · Fit / Fill · Look · Animate · `...` |
| Element | Replace · Style · Animate · Move up a layer · `...` |

Each action is a command (UI-030), so tooltips show its shortcut. The strip never covers the
selected item: if the item reaches the top of the canvas, the strip moves below the canvas
instead. *Show context strip* is a preference (default on).

## 10. Command bar

*Rev. 2: slimmer field in the 36 pt top bar.*

The top bar's centre field is the command palette of UI-034, always visible, plus the entry
to the Assistant.

```
+------------------------------------------------------------------+
| /  fade in|                                              Ctrl K  |
|------------------------------------------------------------------|
|  COMMANDS                                                        |
|  > Add fade-in to selected clip           Clip      Alt+Shift+I  |
|    Audio fade in: 0.5 s                   Setting                |
|  ASSETS                                                          |
|    Fade (In animation)                    Text > Motion          |
|  ASK                                                             |
|    Ask the assistant: "fade in"          Tab                    |
+------------------------------------------------------------------+
```

- Results are grouped: **Commands** (UI-030 registry, disabled ones greyed with their
  reason), **Settings** (Go to setting), **Assets** (library items from UI-006 providers;
  `Enter` applies to the selection or adds at the playhead), **Ask** (always last). Group
  headings are `type.micro` capitals in `text.tertiary`; the active result row is filled
  `bg.active`.
- `@` inside the query mentions a clip, track, marker or media item ("@Title 1"), which sets
  the scope of the command or of the Assistant request.
- `Up`/`Down` choose, `Enter` runs, `Tab` sends the text to the Assistant, `Esc` closes and
  returns focus. Five recent commands appear first when the field is empty (UI-034).
- The field is 26 pt tall and 360 pt wide in Standard (`bg.field`, `r.md`, magnifier glyph
  and shortcut hint in `text.tertiary`); the result list opens 4 pt below it at 560 pt wide
  (`e3`, `r.lg`). In Compact and Tight it shrinks to a search icon that expands on click or
  `Ctrl+K`.
- Shortcut proposal: `Ctrl+K` (free in UI-031) opens it; `Ctrl+Shift+P` keeps working.

## 11. Assistant: its place in the UI

The LLM editing assistant is specified in [assistant.md](../features/assistant.md) (AS-001
to AS-020) and [llm-editor.md](../architecture/llm-editor.md). This section defines **only
the UI side**: where it lives and how a proposal is drawn, previewed, trimmed and accepted.
Behaviour (what a proposal holds, when it is re-checked, what Accept commits, what leaves
the machine) belongs to those specs; the button names and states below follow them.

### 11.1 Placement

*Rev. 2: the face switch moved into the inspector's panel header.*

- **Standard / Compact:** a second face of the inspector panel (AS-001's FreeCut default
  placement; as a UI-056 auxiliary panel it can also be moved to the asset slot). The
  Assistant button `(AI)` in the inspector's panel header, or the top bar's `(AI)` button,
  switches faces (both show the checked state while the Assistant face is shown); the panel
  remembers the last face. Selecting an item while the Assistant face is showing does not
  switch faces; the scope chips update instead (AS-014).
- **Wide:** the Assistant can open as its own panel to the right of the inspector.
- **Tight:** an overlay sheet like the inspector's.
- **Command bar:** *Ask* sends the query straight into the Assistant face and shows it
  (AS-001's "ask" row).
- While the assistant is off (the default) the face shows AS-001's empty state with
  **Set up…**; no other part of the UI changes.

### 11.2 Face anatomy

The example shows a multi-step proposal (AS-011, later). Until then a proposal holds one
edit request (M-AI2), so the card has a single line.

```
.--------------------------------------------.
| Assistant      Local - qwen  (o)     (AI)  |  panel header 48: provider badge + privacy dot
|--------------------------------------------|
| (Selection: 3 clips) (Range) (Playhead)    |  context chips (AS-014); @mentions add
|--------------------------------------------|
|  You                                       |
|  Tighten the intro and add a title.        |
|                                            |
|  Assistant                                 |
|  "Tighter intro" - 4 changes               |  title is the model's; lines are FreeCut's
|  +--------------------------------------+  |
|  | [x] 1 Remove 2 pauses on A1  -1.8 s  |  |  each line = one planned change
|  | [x] 2 Trim beach.mp4 start   +0.6 s  |  |  hover a line: highlight + move playhead
|  | [x] 3 Add text "Summer" at 0:00      |  |
|  | [ ] 4 Title: look "Outline white"    |  |  unticked lines are re-planned out
|  |  Length 1:20.0 -> 1:17.6             |  |
|  | [Play] [Accept 3] [Adjust] [Discard] |  |
|  +--------------------------------------+  |
|                                            |
| [ Ask for a change...               ] [>]  |  Enter sends, Shift+Enter new line
'--------------------------------------------'
```

The proposal card is a `bg.card` block with a 1 pt `border.subtle` outline and `r.md`; its
*Accept* button is the primary (accent) button; the input is a `bg.field` multi-line field.

### 11.3 Proposal states

| State | What the user sees | Keys |
|---|---|---|
| Composing | Input enabled; context chips; example requests in the empty state | `Enter` send |
| Working | One activity line per tool call, written by FreeCut (AS-001), elapsed time and *Stop*; the rest of the app stays fully usable | `Esc` stop (AS-010) |
| Ready | Proposal card with FreeCut's change list (AS-007). The ghost layer is drawn at once: added clips hatched with a dashed accent outline, removed clips struck through, moved clips with a faint trace; the timeline toolbar switch shows *Current* / *Proposed* / *Overlay*. *Play* plays the proposed edit through a preview override from 2 s before the first change. Nothing in the document changes. | `Space` toggle line, `Enter` accept |
| Accepted | The card collapses to "Applied - 3 changes" with *Undo* while that entry is the newest; the Edit menu shows one entry, "Undo Assistant edit" (AS-008) | `Ctrl+Z` |
| Re-checking | The document changed while a proposal waits (user edit, undo, a background job): "Re-checking after your change…", then either "Re-checked after your edit" or the changed and failing lines highlighted with the planner's reasons, plus **Ask to update** (AS-009). A proposal made against an older document is never accepted as it was. | — |
| Incomplete | Stopped or limited turn (AS-010, AS-018): the planned lines are kept, collapsed, and can still be reviewed, accepted or discarded; lines the planner refused are listed with their reasons and are not part of what Accept applies | — |

Rules: the Assistant never applies without an explicit *Accept*; proposed changes are only
ever drawn as ghosts (hatching + dashed outline = shape, not colour only); every line comes
from an ordinary edit request, so its refusal reasons are the planner's own.

### 11.4 Accessibility

Change lines are a list with checkboxes; the proposal summary is announced as a polite live region;
ghost clips get accessibility nodes named "Proposed: …". Reduced motion turns off the
ghost shimmer; the hatching stays.

## 12. Design tokens

*Rev. 2: neutrals re-spaced into one ladder with a lifted control step, translucent hairlines,
timeline surface tokens, new radii, a smaller and lighter type scale, panel metrics. Token
names from rev. 1 are kept; new tokens are marked **new**.*

Token names are what `freecut-app::theme::Tokens` exposes; widgets never use raw colours.
All ratios below were computed with the WCAG 2.x relative-luminance formula; translucent
tokens were composited the way egui blends (premultiplied, in gamma space).

### 12.1 Colour: neutrals and text

The neutral ladder, darkest to lightest in Dark: ground < well < panel < card < field <
hover < active. Each step is about +6 to +9 per channel, with a slight cool tint that is
FreeCut's own. In Light the interactive steps run *darker* than the panel, so pressing reads
as pushing in.

| Token | Dark | Light | High contrast | Use |
|---|---|---|---|---|
| `bg.app` | `#101114` | `#D8D9DE` | `#000000` | the ground: top bar, gutters, status line, Home page |
| `bg.well` | `#15161A` | `#E3E4E8` | `#000000` | sunken areas inside a panel: segmented tracks, slider rails, unit segments, the sequence tab strip, checked tool chips |
| `bg.panel` | `#1C1D22` | `#F6F6F7` | `#000000` | panels |
| `bg.card` | `#24262B` | `#FFFFFF` | `#0A0A0A` | raised: section header bands, current tab, menus, popovers, tooltips |
| `bg.field` (**new**) | `#2B2D33` | `#EEEFF2` | `#141414` | controls at rest (fields, selects, buttons), hover chips on transparent controls |
| `bg.hover` | `#33353C` | `#E7E8EB` | `#1F1F1F` | hovered or focused controls, hovered rows |
| `bg.active` | `#3A3C43` | `#DADBDF` | `#333333` | pressed controls, current list row, hovered menu item, open select |
| `border.subtle` | white 8 % | black 10 % | `#FFFFFF` | hairlines (translucent, so one value works on every surface); High contrast: panel outlines |
| `border.strong` (**new**) | white 16 % | black 20 % | `#FFFFFF` | popover and menu outlines, hovered tile borders, the ring around the canvas, scrollbar thumbs |
| `border.control` | `#71757F` | `#80848D` | `#FFFFFF` | toggle-off outline in High contrast, checkbox outline |
| `text.primary` | `#ECEDEF` | `#16171A` | `#FFFFFF` | values, titles |
| `text.secondary` | `#B3B7BF` | `#4A4E57` | `#E6E6E6` | labels, menu labels, idle tabs |
| `text.tertiary` | `#A6AAB3` | `#585C65` | `#E6E6E6` | units, summaries, counts, readouts, timestamps |
| `text.disabled` | `#5C5F67` | `#A0A3AA` | `#8C8C8C` | disabled labels (exempt from contrast rules) |

Changed from rev. 1: every `bg.*` value except Light `bg.panel` / `bg.card` and the High
contrast column; Dark `text.secondary` and `text.tertiary` (brighter, so they keep 4.5 : 1 on
the new `bg.active`); `border.subtle` (now translucent); `border.control`; `text.disabled`.
In Dark, size and weight carry more of the secondary / tertiary distinction than colour.

Measured contrast (dark / light): `text.primary` on `bg.card` 12.9 / 17.9; `text.secondary`
on `bg.card` 7.5 / 8.3; `text.tertiary` on `bg.card` 6.5 / 6.7; the hardest pairs —
`text.tertiary` on `overlay.selection` over `bg.card` 4.6 / 4.8, on `bg.active` 4.7 / 4.8, and
light `text.tertiary` on `bg.app` 4.75 (the status line); `border.control` against `bg.card`
3.3 / 3.7. Every text and status colour reaches 4.5 : 1 on `bg.app`, `bg.well`, `bg.panel`,
`bg.card`, `bg.field`, `bg.hover`, `bg.active` and `overlay.selection` over a card or panel,
in both themes (7 : 1 for text in High contrast); the unit test checks all of them.

**No dim text.** There is no text token below `text.tertiary`. Readouts, placeholders, save
state and counts use `text.tertiary`, which passes 4.5 : 1; only disabled controls use
`text.disabled`.

### 12.2 Colour: accent and status

| Token | Dark | Light | High contrast | Use |
|---|---|---|---|---|
| `accent` (Iris) | `#8E86FF` | `#5A4FD8` | `#FFD60A` | keyboard focus ring, selection outlines, checked tools, play, primary button, switches on, gutter hover line, slider fill |
| `accent.hover` | `#A39DFF` | `#4B40C8` | `#FFE45C` | hover on accent elements |
| `accent.text` | `#A9A3FF` | `#4B40C8` | `#FFD60A` | links and accent-coloured text and glyphs |
| `text.on-accent` | `#0D0B26` | `#FFFFFF` | `#000000` | labels on accent fills |
| `danger` | `#FF8A85` | `#B1241C` | `#FF7474` | errors, destructive buttons on hover |
| `warning` | `#E9AE45` | `#835100` | `#FFB000` | warnings, unsaved state, near-clipping waveform peaks |
| `success` | `#4CC38A` | `#146942` | `#5CFF9D` | success, finished export |

Measured: `accent` against `bg.card` 5.0 / 5.9 and against `bg.panel` 5.6 / 5.5 (≥ 3 : 1 for
focus and state); `text.on-accent` on `accent` 6.4 / 5.9; status colours ≥ 4.5 on every
background of §12.1. Changed from rev. 1: dark `danger` (`#FD7B76` → `#FF8A85`).

The accent stays **Iris**, FreeCut's own blue-violet, chosen so it does not collide with any
clip colour (§12.3), with the warning amber, or with the white playhead. It is not
user-selectable (§18).

### 12.3 Colour: editing surfaces (all themes unless noted)

| Token | Value | Use |
|---|---|---|
| `clip.video` | `#2F6F8F` | video and image clips: name strip, all-strip clips |
| `clip.audio` | `#2F7D5B` | audio clips (sound extracted from a video: `#2A7474`, AU-001) |
| `clip.text` | `#9C6B1E` | text clips |
| `clip.element` | `#7A4FA3` | stickers, shapes |
| `clip.effect` | `#A3476B` | effect layers |
| `clip.adjust` | `#4F5DB3` | filter / adjustment layers |
| `clip.caption` | `#8C7A2B` | caption items |
| `clip.well` (**new**, derived) | the kind colour at 30 % over `bg.well` (Dark) / 18 % over `bg.panel` (Light); High contrast `#000000` | clip bodies under the name strip. Dark: video `#1D313D`, audio `#1D352E`, extracted `#1B3235`, text `#3E301B`, element `#332743`, effect `#402532`, adjust `#262B48`, caption `#39341F`. Light: video `#D2DEE4`, audio `#D2E0DB`, extracted `#D1DFDF`, text `#E6DDD0`, element `#E0D8E8`, effect `#E7D7DE`, adjust `#D8DAEB`, caption `#E3E0D2` |
| `clip.wave` (**new**, derived) | the kind colour mixed 50 % towards white (Dark) / 30 % towards black (Light), at 85 % | waveform bars on audio bodies and video sound bands |
| `clip.selected` | 2 pt `accent` outline inside the clip, fill unchanged (High contrast: 2 pt `#FFFFFF`, away from the yellow accent) | selected clip *(rev. 2: was white + lighter fill)* |
| `clip.proposed` | `accent` at 35 % in 45° hatching + 1.5 pt dashed `accent` outline | Assistant ghosts (AS-007) |
| `bg.lane` / `bg.lane-alt` (**new**) | Dark `#15161A` / `#18191D` · Light `#EAEBEE` / `#E4E5E9` · High contrast `#000000` / `#0A0A0A` | alternating timeline lanes (darker than the panel, so clips read as objects in a well) |
| `bg.ruler` (**new**) | Dark `#121317` · Light `#EFF0F2` · High contrast `#000000` | the timeline ruler and the header column's corner above the track headers |
| `timeline.grid` (**new**) | white 6 % (Dark) / black 7 % (Light) / white 30 % (High contrast) | vertical lines through the lanes at each labelled ruler tick |
| `playhead` | `#F4F4F6` dark / `#16171A` light / `#FFFFFF` High contrast | playhead line and head (independent of the accent) |
| `player.surround` | `#1E1E1E` | around the canvas in every theme (UI-014) |
| `canvas.guide` | `#FF4FA0` | snapping guides on the canvas |
| `canvas.safe` | white at 35 %, dashed | safe areas |
| `canvas.hover` | white at 60 % | hover outline on the canvas |
| `overlay.selection` | `accent` at 22 % | selected tiles and rows, rubber bands, timeline range, the play button's hover chip; also the soft halo colour where one is needed |
| `overlay.hover` | white at 6 % (dark) / black at 5 % (light) | hover wash on custom-painted areas |
| `overlay.badge` (**new**) | black at 55 % | badges and pills drawn over pictures (duration badges, the canvas readout pill, tile remove buttons), with white text |
| `overlay.scrim` | black at 55 % (Dark, High contrast) / 35 % (Light) | behind modal sheets only *(rev. 2: lighter in Light)* |

Clip label text picks white or `#111214` by luminance (UI-013's rule). Clip kinds also carry
an icon on the clip (UI-043).

### 12.4 Spacing, sizing and density

*Rev. 2: one rhythm: 28 pt controls on a 34 pt pitch, 14 pt insets, 48 pt panel headers,
8 pt gutters.*

Base unit 2 pt. Scale: `sp.1 2` · `sp.2 4` · `sp.3 6` · `sp.4 8` · `sp.5 12` · `sp.6 14` ·
`sp.7 16` · `sp.8 24` · `sp.9 32`.

| Metric | Comfortable (default) | Compact |
|---|---:|---:|
| Gutter between panels and around them (**new**) | 8 | 4 |
| Top bar height (**new**; was 44) | 36 | 32 |
| Panel header height (**new**) | 48 | 40 |
| Panel side inset (**new**; rows, headers, notes) | 14 | 10 |
| Control height (field, button, select) | 28 | 24 |
| Row gap (**new**) → row pitch | 6 → 34 | 4 → 28 |
| Section header band (card header) | 36 | 30 |
| Lens strip (was lens bar 36 / 32) | 40 | 34 |
| Inspector label column (form rows; was 88 / 76) | 76 | 64 |
| Row gutter (keyframe navigator; was 40) | 56 | 50 |
| Number field width, bare / with unit (**new**) | 64 / 84 | 56 / 72 |
| Icon button chip / hit area (was 28 / 32) | 26 / 28 | 24 / 24 |
| Shelf tab (**new**; was Shelf button 64 × 52) | ≥ 56 wide, 40 tall | ≥ 40 wide, 32 tall, icon only |
| Drawer side list (**new**) | 112 (80 – half) | 96 (72 – half) |
| Drawer tile, minimum width × 16:9 (was 88 × 66 fixed) | 96 | 80 |
| Grid gap | 8 | 6 |
| Card inner padding (now the side inset) | 14 | 10 |
| Gap between cards (now a hairline) | 0 | 0 |
| Timeline: sequence tabs / toolbar / ruler (**new**) | 32 / 40 / 28 | 28 / 34 / 24 |
| Timeline: track header column (**new**; was 132 fixed) | 176 (140 – 40 %) | 148 (120 – 40 %) |
| Timeline lane heights Small / Medium / Large (**new**) | 40 / 72 / 96 | 34 / 60 / 80 |
| Transport bar (**new**) | 44 | 36 |
| Stage surround margin | 16 | 8 |

Density is a preference (Appearance ▸ *Density*: Automatic, Comfortable, Compact;
default Automatic, which follows the §5.1 breakpoints), independent of the interface
scale (UI-015). The hit area is never below 24 × 24 pt (UI-043) except where §13.6 notes an
equivalent command.

### 12.5 Radii

*Rev. 2: 3 / 5 / 7 / 12 / 16 replace 4 / 8 / 12; 7 is the everyday control radius.*

| Token | pt | Use |
|---|---:|---|
| `r.xs` (**new**) | 3 | badges, chips inside controls, keyframe navigator chips, menu row hover |
| `r.sm` | 5 | icon buttons, tabs, side-list rows, timeline clips, tooltips, the segmented thumb, swatches, Export |
| `r.md` | 7 | fields, selects, text buttons, segmented tracks, tiles, inner cards (effect cards, proposal card), select popovers |
| `r.lg` | 12 | panels, menus, toasts, context strip, Command bar results |
| `r.xl` (**new**) | 16 | modal sheets (export, new project), Preferences |
| `r.pill` | half the height | chips, switches, status chips |

### 12.6 Typography

*Rev. 2: one step smaller and lighter: 12 pt body, 11 pt secondary, Medium (not Semibold)
titles; `type.micro` added; the display size grows for the Home heading.*

UI font **Inter** (OFL-1.1), numbers with tabular figures; timecode **JetBrains Mono**
(OFL-1.1); fallback **Noto Sans** (OFL-1.1, already bundled) and Noto Sans CJK on demand.
Weights used: Regular, Medium and (Home heading only) SemiBold — all already bundled.

| Token | Size / line (pt) | Weight | Use |
|---|---|---|---|
| `type.micro` (**new**) | 10 / 12 | Medium | counts, duration badges, ruler labels, result-group headings and form eyebrows (capitals, +0.8 pt tracking) |
| `type.caption` | 11 / 14 | Medium | units, chip labels, Shelf tab labels, clip names, summaries (tabular) |
| `type.small` | 11 / 14 | Regular | tile names, breadcrumbs, tooltips, notes, readouts *(was 12)* |
| `type.body` | 12 / 16 | Regular | values, menu items, row labels, field text *(was 13)* |
| `type.label` | 13 / 18 | Medium | section and panel titles; buttons use 12 pt Medium *(was Semibold)* |
| `type.title` | 14 / 20 | Medium | the inspector's item name, dialog titles *(was 15 Semibold)* |
| `type.heading` | 16 / 22 | Medium | empty-state titles, sheet titles *(was 18 Semibold)* |
| `type.display` | 32 / 38 | SemiBold, −0.6 pt tracking | Home heading only *(was 24)* |
| `type.mono` | 12 / 16 | Regular | timecode, frame counts *(was 13)* |

Nothing interactive is set below 11 pt. People who want larger text raise the interface size
(UI-015), which scales everything together.

### 12.7 Elevation

*Rev. 2: panels lost their level-0 role to the ground and carry no shadow; popover shadows
are softer and outlines use `border.strong`.*

| Level | Fill | Shadow (offset, blur, spread, colour) | Outline | Use |
|---|---|---|---|---|
| `e0` | `bg.app` | none | none | the ground |
| `e1` | `bg.panel` | none | none (High contrast: 1 pt `border.subtle`) | panels, rails |
| `e2` | `bg.card` | (0, 4), 12, 0, black 30 % dark / 10 % light | 1 pt `border.strong` | menus, popovers, tooltips, toasts, context strip, peeking asset panel |
| `e3` | `bg.panel` | (0, 12), 32, 0, black 45 % dark / 18 % light | 1 pt `border.strong` | Command bar results, modal sheets |

All values fit egui's `Shadow { offset: [i8; 2], blur: u8, spread: u8, color }`.

### 12.8 Motion

*Rev. 2: tooltips warm up; toasts leave more slowly than they arrive; every hover is a fade.*

| Token | Duration | Easing | Use |
|---|---:|---|---|
| `dur.fast` | 90 ms | ease-out cubic | hover and press fades (fills, borders, text and glyph colours), switch thumb, gutter line |
| `dur.base` | 160 ms | ease-out cubic (in), ease-in cubic (out) | section expand / collapse, segmented thumb slide, tab-strip scroll, layout change |
| `dur.slow` | 240 ms | ease-out cubic | Command bar, overlay sheets, export sheet, toasts arriving |
| `dur.exit` (**new**) | 360 ms | ease-in cubic | toasts leaving (with an 8 pt fall) |
| `delay.preview` | 150 ms | — | hover preview start (UI-007) |
| `delay.tooltip` | 500 ms | — | the first tooltip (UI-039); for 600 ms after a tooltip closes, the next one shows at once |
| `fps.thumb` | 24 fps | — | animated preset thumbnails, only while hovered or focused |

**Reduced motion** (UI-043): every duration becomes 0, preset thumbnails show a still poster
frame (a static "key pose"), the ghost shimmer is off; the playhead and progress
indicators still move because they are content.

## 13. Component inventory

*Rev. 2: §13.1–§13.15 restyled in place (numbers kept, code refers to them); §13.16–§13.29
are new.*

Every component lives in `freecut-app::widgets` (generic) or `freecut-app::shell` (window
parts). "Stock" means built from egui 0.36 widgets with our styling; "custom" means we paint
it ourselves with `Painter` and handle its `Response` and accessibility node. Shared state
rules for every control: **rest** as listed; **hover** one ladder step up (`dur.fast` fade);
**pressed** `bg.active`; **disabled** the whole control at 40 % opacity with its tooltip
still explaining why; **keyboard focus** the ring of §15.

### 13.1 Property card — custom container

Anatomy and rules in §6.2 (drawn as a full-width section). States: expanded, collapsed (with
summary), switched off, read-only (locked: values in `text.secondary`, no gutter controls),
mixed (multi-selection). Keyboard: `Enter`/`Space` on the header toggles; `Right`/`Left`
expand/collapse; the switch and badges are their own focus stops. Accessibility: a group node
named by the section title with expanded state; the switch is a toggle node. egui:
`CollapsingState` for open state + a full-width `Frame` (fill `bg.panel`, no radius, side
inset 14) + a custom header band painted `bg.card` and built with `Sides`, and a hairline
painted above every section but the first.

### 13.2 Property row — custom layout

Label (scrub handle, UI-009; double-click resets) · control · 56 pt gutter. The label shows a
horizontal-resize cursor on hover. A changed value shows the 4 pt `accent` dot before the
label. Parameter rows lay their control out right-to-left so the field hugs the gutter; form
rows use the 76 pt label column. Focus on the row's control also makes the row's context menu
and `Ctrl+Backspace` (proposed) reach *Reset*.

### 13.3 Slider with field — custom composite

Used only for the one value a section is about, as a two-line row (§6.2). A rail 4 pt tall in
`bg.well` with a 1 pt `border.subtle` outline, `r.xs`; the part up to the thumb's centre in
`accent`; ends inset by half the thumb. Thumb: a 14 pt circle in `text.primary` with a 1 pt
black-40 % rim, growing to 16 pt on hover and drag (`dur.fast`). Followed, 8 pt later, by the
number field (§13.19). Soft range on the rail, hard range when typing (UI-009). Drag on the
rail jumps then drags; `Shift` fine-tunes ×0.1; double-click resets. One undo entry per drag.
egui: stock `Slider` cannot share one gesture/undo boundary and soft/hard ranges cleanly with
a separate field, so this is a custom rail + our `NumberField` side by side, with gesture
start/end reported to the inspector for coalescing.

### 13.4 Segmented control — custom

28 pt tall (24 Compact). Track: `bg.well` with a 1 pt `border.subtle` outline, `r.md`;
segments share the width inside a 3 pt inset. One **thumb** in `bg.hover`, `r.sm`, no outline,
that slides to the chosen segment over `dur.base`. Labels `type.body`: `text.primary` on the
chosen or hovered segment, `text.secondary` otherwise. Icon segments (16 pt glyph) keep the
word as tooltip and accessible name. Arrow keys move the selection and wrap (radio-group
semantics). Used for: lens strip, case, alignment, fill mode, In/Loop/Out, Detail switch,
Theme and Density in Preferences. egui: custom painter; the thumb's x is
`animate_value_with_time`.

### 13.5 Swatch + colour popover — stock picker in a custom popover

*Rev. 2: an editable hex field joins the swatch.* Row layout: label, flexible space, a **hex
field** (86 × 24, `r.sm`, `bg.field`, a 20 pt leading `#` segment in `bg.well` with a hairline
seam; six digits in `type.mono` at 12 pt; accepts 3 or 6 digits and commits on `Enter` or when
it loses focus; text that is not a colour keeps the old value; alpha is kept because opacity
has its own row; 1 pt `accent` border while focused), then a 24 × 24 **swatch** (`r.sm`,
1 pt `border.subtle`, `border.strong` on hover; a checkerboard of 6 pt cells under translucent
colours). Clicking the swatch opens a `Popup` (`e2`, 240 pt wide, 4 pt below, right edges
aligned) with the saturation/value square, hue and alpha strips, hex input, eyedropper
(UI-009), and swatch rows *Palette*, *Recent*, *Saved* (20 pt swatches, `r.xs`, 8 per row).
Mixed selections show "—" in the hex field and a hatched swatch. egui: `color_picker` building
blocks for the square and strips; the hex field is a frameless `TextEdit` on a painted frame;
swatch rows and eyedropper are custom.

### 13.6 Keyframe navigator — custom

Lives in the row gutter (56 pt). Three parts: `<` previous key · diamond · `>` next key, each
an 18 × 24 pt target, 1 pt apart, with a `bg.field` hover chip (`r.xs`); glyphs 11 pt. Diamond
states follow TR-028 / UI-009 by **shape**: outline in `text.tertiary` = not animated;
outline with a centre dot in `accent` = animated, no key at the playhead; filled `accent` =
key at the playhead. *Rev. 2:* on animatable rows the arrows are always shown and greyed
(`text.disabled`) when there is no key in that direction, so the gutter never changes width;
they move the playhead to the previous / next key. The arrows are narrower than 24 pt; the
*Previous keyframe* / *Next keyframe* commands (UI-030, Command bar and keys) are the
equivalent control for UI-043's target-size rule. Click the diamond to add or remove a key at
the playhead. Accessibility: a button named "Keyframe for Size: none at playhead, 3 keys".

### 13.7 Preset grid with live thumbnails — custom

A virtualized grid of tiles using §13.23's layout (16:9, auto-filled columns). Each tile shows
a still poster until hovered or focused; then it plays a short loop rendered off the UI thread
by `freecut-render` from FreeCut's own sample (UI-007; cached in the media cache, PM-074). The
hovered tile also drives the Stage preview override when a compatible clip is selected. The
selected preset has a 2 pt `accent` ring and a check badge; a preset already applied to the
selection is `overlay.selection` with an `accent` outline. Keyboard: arrows move, `Enter`
applies, `Space` toggles preview playback. Only visible tiles request frames
(`ScrollArea::show_rows` or a viewport-based variant); a tile that has no frame within 100 ms
shows its kind well with a faint kind glyph, never a spinner.

### 13.8 Toggle switch — custom

32 × 18 pt pill; off = `bg.active` fill with a hollow thumb (High contrast: `border.control`
outline); on = `accent` fill with a `text.on-accent` thumb and a small check inside the thumb
(shape cue). The thumb slides over `dur.fast`. egui: custom widget (egui's demo toggle
pattern) using `animate_bool_with_time`.

### 13.9 Chips — stock-based

Pill (`r.pill`), 24 pt high, `bg.field`, label `type.caption`, 10 pt side padding; selected =
`overlay.selection` fill + `accent.text`. Used for scope chips, the narrow-panel chip row of
the Drawer (§8.2), and summary chips (read-only variant with no fill). Rows of chips scroll
sideways with the wheel and show no scrollbar. egui: `Button::selectable` with custom `Frame`.

### 13.10 Lens strip and Shelf tab — custom

Lens strip: a segmented control (§13.4) that never wraps: when it doesn't fit, labels drop to
icon-only. Shelf tab: §8.1 (icon 16 above a `type.caption` label; current = `bg.field` fill;
idle `text.secondary`). The rev. 1 Shelf *button* (rail) remains only as the collapsed asset
panel's rail: 44 pt wide, icons 20 pt, current = `bg.field` chip.

### 13.11 In / Loop / Out timing strip — custom

A 28 pt bar the width of the section representing the clip's duration: In segment at the
left, Out at the right, Loop between (TX-022 window `[in, d − out)`), each with its preset name
when wide enough. Dragging a join changes the In or Out length (snapped to frames, FX-018
limits), one undo entry per drag; clicking a segment selects that slot in the preset grid.
Segments use `accent` at 30 % (selected slot) or `bg.well` (others), and a pattern for Loop
(wavy line) so the three are distinguishable without colour.

### 13.12 Toast — custom (UI-040)

Bottom-right, 16 pt from the window's right edge and above the status line, max width 360 pt,
at least 32 pt tall, `e2`, `r.lg`. Icon + message (`type.body`) + up to two text buttons +
close. Arrives over `dur.slow` with an 8 pt rise; leaves over `dur.exit`. Kinds and timeouts
per UI-040; error toasts have a 3 pt `danger` bar on the left edge plus an error icon.

### 13.13 Banner — custom

Full-width strip at the top of the inspector or Stage (locked, offline, previewing plan).
Icon + one line + one action. `bg.field` fill with a 3 pt left bar in the status colour.

### 13.14 Empty state — custom layout

Centred in its region: a 28 pt line icon in `text.tertiary`, a `type.label` sentence that
says what goes here, a `type.small` detail line that depends on *why* it is empty, one
primary action button, and an optional secondary link. Examples: Drawer *My looks* with no
saved looks → "Looks you save appear here" + *Save current look*; timeline with no clips →
"Drop media here or press Ctrl+I". Empties inside a section use a one-line note instead.

### 13.15 Others (stock with tokens)

| Component | egui | Notes |
|---|---|---|
| Text buttons (default, primary, ghost, destructive) | `Button` | 28 pt (34 large), `r.md`, 12 pt side padding, 12 pt Medium label; default = `bg.field` + 1 pt `border.subtle`, hover `bg.hover` + `border.strong`; primary = `accent` fill, `text.on-accent`, hover `accent.hover`; ghost = no fill until hover; destructive = default until hover, then `danger` text and outline |
| Dropdown | the Select of §13.20 | font picker is a custom list that sets every family name in that family (rendered off thread, cached) |
| Number field | `DragValue` wrapped as `NumberField` (§13.19) | UI-009 unit/arithmetic parsing |
| Search field | `TextEdit` on a painted frame | 28 pt, `r.md`, `bg.field`, 12 pt magnifier glyph, placeholder in `text.tertiary`, a clear button once there is text, 1 pt `accent` border while focused |
| Multi-line text | `TextEdit::multiline` | Content lens (§7.3) |
| Tooltip | `on_hover_text` / `Tooltip` | name + shortcut (UI-039); `bg.card`, 1 pt `border.strong`, `r.sm`, `type.small`, 8 pt side padding, wraps at 280 pt, 6 pt from the control |
| Menu / context menu | `Popup`, `menu` | UI-035; `e2`, `r.lg`, 6 pt padding, 26 pt rows with `r.xs` hover fill `bg.active`, 9 pt separators (1 pt `border.subtle` inset 8 pt), shortcuts right-aligned in `text.tertiary` |
| Modal sheet | `Modal` | §13.28 |
| Command bar | `Area` + `TextEdit` + result list | §10 |

### 13.16 Panel and panel header — custom *(new)*

§3.4. A panel is a `Frame` filled `bg.panel` with `r.lg` on the ground; its content is
clipped to the rounded corners. The header is 48 pt (40 Compact): title (`type.label`) at the
side inset, optional readout in `type.small` `text.tertiary`, trailing icon buttons 2 pt
apart, and a 1 pt `border.subtle` hairline along the bottom. The focus line of §3.4 is painted
along the top edge. Accessibility: each panel is a region node named after its place.

### 13.17 Gutter splitter — custom *(new)*

§3.4. The 8 pt gutter (4 Compact) plus 2 pt on each side is the grab area; the cursor is the
horizontal or vertical resize cursor; a 2 pt `accent` line, inset 12 pt from both ends, fades
in on hover over `dur.fast` and stays while dragging. Dragging changes the split's fraction,
clamped to the regions' minimums (UI-002); double-click resets it. Inside panels, the
side-list and track-header splitters are a 1 pt `border.subtle` hairline with a ±3 pt grab
area that turns `accent` on hover or drag.

### 13.18 Icon button and tool button — custom *(new)*

A 26 pt chip (24 Compact), `r.sm`, transparent at rest with its glyph in `text.secondary`
(16 pt in toolbars and the transport, 14 pt in panel and track headers). Hover: `bg.field`
chip, glyph `text.primary`. Pressed: `bg.active`. **Checked** (a mode is on, a track is
locked / hidden / muted): `bg.well` chip with the glyph in `accent.text` and Phosphor's
*Fill* weight — shape and colour, never an accent slab. **Danger** (delete): on hover the chip
turns `danger` at 14 % and the glyph `danger`. **Disabled**: 40 % opacity, tooltip explains
why. Tooltips are a sentence plus the shortcut. A **split button** is an icon button and a
16 pt-wide chevron button joined with no gap, the chevron opening a menu. **Export** is the
filled variant: `accent` fill, `text.on-accent`, glyph 12 + label, height 26, `r.sm`.

### 13.19 Number field with unit segment — custom *(new)*

28 pt tall (24 Compact), `r.md`, `bg.field` (hover or focus `bg.hover`, `dur.fast`); 64 pt
wide bare, 84 pt with a unit (Compact 56 / 72). Optional leading prefix letter (X, Y, W, H) in
`type.caption` `text.tertiary`; the value in `type.body` tabular, left-aligned at 8 pt. The
**unit segment** is a square-cornered block on the trailing edge (clipped by the field's
radius): `bg.well`, a 1 pt `border.subtle` seam on its left, the unit (`%`, `°`, `s`, `×`,
`px`, `dB`) centred in `type.caption` `text.tertiary`, width = unit text + 14 pt. Interactions
(UI-009 behaviour, presentation here): dragging across the field scrubs the value (horizontal-resize
cursor; starts after 3 pt of travel; `Shift` ×10, `Alt` ×0.1); a press without travel focuses
the field and selects its text; `Up` / `Down` step; `Enter` or losing focus commits; `Esc`
restores and leaves. Mixed values show "—". A field joined to a select (W `[400]` `[px v]`)
squares the touching corners and leaves no gap.

### 13.20 Select — custom *(new; replaces stock `ComboBox` in panels)*

Trigger: 28 pt tall (24 Compact), `r.md`, `bg.field` (hover or open `bg.hover`), 8 pt left /
6 pt right padding, text `type.body`, a 12 pt caret in `text.secondary` at the trailing edge;
it is as wide as its widest option, so it does not change width when the choice changes. An
icon-only variant shows the current option's glyph. List: opens 4 pt below with the right
edges aligned, at least as wide as the trigger; `e2` fill `bg.card`, 1 pt `border.strong`,
`r.md`, 4 pt padding; rows 26 pt, `r.sm`; the row under the pointer or arrow keys is filled
`bg.active`; the current option carries an `accent.text` check on the right; optional detail
text right-aligned in `type.small` `text.tertiary`; rows that cannot be picked are
`text.disabled` and skip the cursor. At most 240 pt tall, then it scrolls with the thin
scrollbar, opening scrolled to the current option. Keyboard: on the trigger `Down`, `Up`,
`Enter` or `Space` open; in the list arrows, `Home`, `End` move, `Enter` / `Space` pick, `Esc`
closes and returns focus; a click outside closes.

### 13.21 Help badge and reset badge — custom *(new)*

16 pt circles in `bg.field` that centre in whatever row or header they sit in. **Help** `?`
(`type.micro`, `text.secondary`): on hover the fill turns `overlay.selection` and the mark
`accent.text`, the cursor becomes the help cursor, and a 260 pt-wide explanation pops over
beside the badge (`e2`, `type.small`) without moving anything in the panel. **Reset** (10 pt
counter-clockwise arrow): resets the section (and clears its keys where the owner says so) as
one undo entry; at 40 % opacity when nothing differs from the defaults, never hidden.

### 13.22 Side list — custom *(new)*

§8.2. Group headings 26 pt, rows 24 pt with right-aligned counts, resizable through its
hairline splitter (§13.17). Also used by Preferences (page list, §13.28) and, later, by
UI-056's auxiliary panels. Accessibility: a tree node per group, list items with their counts
in the accessible name ("Video, 6 items").

### 13.23 Media tile — custom *(new)*

Layout: `cols = max(1, ⌊(W + gap) / (min + gap)⌋)`, `tile_w = (W − (cols − 1) · gap) / cols`,
thumbnail `tile_w × tile_w · 9/16`, name 4 pt below (one line, `type.small`, cut by width),
row pitch = tile + 18 pt + gap. Thumbnail: `r.md`, clipped, on the kind's `clip.well` so an
item without a poster still says what it is; pictures are **cover**-cropped (fill, centre
crop); audio shows its waveform across the thumbnail in `clip.wave`; otherwise a 24 pt kind
glyph at 30 % of the kind colour. Duration badge: bottom-right, 4 pt in, `type.micro` tabular
white on `overlay.badge`, `r.xs`, short form (`0:22`, `1:05:09`); stills show their format
instead. Offline media: an `overlay.badge` wash with a `danger` *Offline* badge (top-left).
States: rest 1 pt `border.subtle`; hover `border.strong` plus a hover-only remove button
(18 pt, top-right, `overlay.badge`, `danger` on its own hover) or, on catalogue tiles, a `+`
add button and a star; selected 2 pt `accent`; name `text.primary` when selected, else
`text.secondary`; while dragged the tile fades to 45 %. Double-click opens the item on the
Stage as a source (EX); right-click opens its context menu.

### 13.24 Thin scrollbar — stock style / custom for painted areas *(new)*

An 8 pt gutter with no track fill and a 4 pt thumb (2 pt inset, `r.xs`, at least 24 pt long)
in `border.strong`, `text.tertiary` on hover or drag; hidden when nothing overflows; pressing
the gutter jumps there and keeps dragging. Stock `ScrollArea`s get it through the style
(§16.1); the timeline's painted lanes draw their own (§13.26).

### 13.25 Transport and timecode field — custom *(new)*

§9.1. The timecode field is a §13.19 field without a unit, 116 × 28 pt, `type.mono`,
showing `HH:MM:SS:FF` (UI-019's format): drag scrubs at 3 pt per frame (`Shift` ×10,
`Alt` ×0.1), `Up` / `Down` step one frame, typing a timecode and `Enter` seeks, `Esc` reverts.
Seeking is not an edit (no undo entry). Transport buttons are §13.18 icon buttons; play /
pause uses the `accent` glyph and an `overlay.selection` hover chip.

### 13.26 Timeline panel — custom *(new; presentation of TL and UI-011)*

The timeline stays one custom-painted area per band (no egui widget per clip). Top to bottom:

- **Sequence tabs** (32 pt, 28 Compact; the panel's header): a `bg.well` strip with a 1 pt
  hairline at its bottom, 8 pt side padding, tabs 2 pt apart standing on the hairline. A tab is
  30 pt tall, label (`type.body`, cut at 140 pt) at 10 pt from its left, top corners `r.sm`:
  idle transparent `text.secondary`; hover `bg.field`; current `bg.panel` (it joins the
  toolbar below) and `text.primary`. A close button (16 pt, `r.xs`) appears only while the tab
  is hovered and more than one tab exists. Double-click renames inline (commit on `Enter` or
  focus loss, `Esc` cancels); drag reorders with a 2 pt `accent` insertion bar; `+` (22 pt)
  after the last tab adds a sequence; a compound clip opened for editing (TL-040) appears as a
  tab of its own. Rename, reorder, add and close follow the owners' undo rules (TL).
- **Toolbar** (40 pt, 34 Compact; UI-011's content and order): `bg.panel`, a hairline at the
  bottom, 8 pt side padding, 4 pt between buttons, groups separated by a 1 × 18 pt
  `border.subtle` rule in a 9 pt slot; §13.18 tool buttons with 16 pt glyphs; toggles use the
  *checked* state (never an accent fill); the selection count appears in `accent.text`
  `type.caption` when more than one clip is selected; the zoom group (out, slider 96 pt, in,
  fit) sits at the right end before the `⋯` menu.
- **Ruler** (28 pt, 24 Compact): `bg.ruler`, a hairline at its bottom. Labelled ticks 1 × 8 pt
  in `border.strong` with `HH:MM:SS:FF` labels in `type.micro` tabular `text.tertiary` 4 pt to
  the right; four unlabelled 1 × 4 pt ticks between labels in `border.subtle`. The labelled
  interval is the first of 1, 2, 5, 10 frames, 1, 2, 5, 10, 15, 30 s, 1, 2, 5, 10, 30 min,
  1 h whose spacing is at least 88 pt. Each labelled tick continues down through the lanes as a
  1 pt `timeline.grid` line. Pressing or dragging the ruler scrubs. The selected clip's
  keyframes sit on the ruler floor as 9 pt diamonds (14 pt hit area): click jumps there, drag
  moves the keys at that instant. The header column's part of the ruler is a plain `bg.ruler`
  block.
- **Track header column** (176 pt, between 140 pt and 40 % of the panel; resizable by a 1 pt
  hairline splitter): `bg.panel`; each row exactly as tall as its lane with a 1 pt hairline at
  its bottom; padding 8 pt left, 4 right, 6 top. Content: the kind icon (14 pt) and the track
  name (`type.body`, cut by width; the main track also shows a magnet glyph in
  `text.tertiary`), then §13.18 icon buttons (24 pt chips, 14 pt glyphs): eye (hide), speaker
  (mute), lock, and a `...` menu (rename, colour, height: Auto / Small / Medium / Large,
  delete). On lanes of 72 pt and taller the buttons sit on a second line; on 40 pt lanes they
  share the name's line, right-aligned. A button is *checked* when it withholds something
  (hidden, muted, locked) — `bg.well` chip, `accent.text` Fill glyph.
- **Lanes:** alternating `bg.lane` / `bg.lane-alt`, a 1 pt `border.subtle` hairline under each.
  Lane heights Small 40 / Medium 72 / Large 96 (Compact 34 / 60 / 80). *Auto* (the default)
  picks by content: the main track Large, other video and image tracks and audio Medium,
  text, element, effect, adjustment and caption tracks Small, an empty track Medium. *(Review
  fixes, 2026-10-13: overlay tracks were Large, which pushed the sound below the fold at
  1440 × 900. The horizontal thin scrollbar has its own 8 pt strip under the lanes and never
  covers the last lane. A ruler timecode under the playhead's head fades out: hidden while the
  head covers it, fully shown 8 pt clear of it.)* The per-track height is view
  state unless TL decides to persist it (§18).
- **Clips:** inset 4 pt from the lane top and bottom, `r.sm`, separated by a 1 pt gap at every
  cut. *Two-tone* kinds (video, image, audio): an 18 pt **name strip** (16 Compact) in the kind
  colour with top corners rounded, the name in `type.caption` at 6 pt (label colour by
  luminance), badges at the strip's right end (§ below), then the **body** in `clip.well`.
  *All-strip* kinds (text, element, effect, adjustment, caption): the whole clip is the kind
  colour, with a 12 pt kind glyph at 6 pt and the name after it. Names hide under 32 pt of clip
  width. Rest: no outline; hover: 1 pt `border.strong`; **selected: 2 pt `accent` inside**
  (`clip.selected`); locked track: the lane colour at 50 % over its clips and the lock button
  checked.
  - **Filmstrip** (video, image): the body below the strip is tiled with frames as tall as the
    frame area and as wide as the source aspect gives; each tile shows the source frame under
    the middle of its visible part, so short and trimmed clips still show their own frames.
    Only tiles on screen are requested; until a frame arrives the tile shows the well with a
    20 pt kind glyph at 30 % of the kind colour. Frames come from a background job through
    `freecut-media` (never decoded on the UI thread).
  - **Sound band** (video with sound): a band at the bottom of the body, `clamp(0.3 × body,
    10, 28)` pt tall, `clip.well` darkened, 1 pt hairline on top, holding the waveform.
  - **Waveform** (audio bodies and sound bands): vertical bars 2 pt wide on a 3 pt pitch,
    symmetric about the band's centre, in `clip.wave`; bar parts at or above −1 dBFS (near
    clipping) in `warning`; the drawn height follows the clip's gain live.
  - **Volume line:** a horizontal white line, 1 pt at 70 % (2 pt, 100 % on hover or drag) with
    a 10 pt grab strip and a vertical-resize cursor; while hovered or dragged a white pill
    (16 pt, `r.sm`) shows the gain in dB ("−3.0 dB") above the line at its left end. Volume
    keyframes are 7 pt white dots with a 1 pt dark rim, `accent` when selected.
  - **Fades:** a wedge of black at 40 % with a 1 pt white-60 % edge line; on video only over
    the sound band.
  - **Badges** (speed, link, sync offset, muted, effects): `type.micro` labels on black-25 %
    chips (`r.xs`, height `min(strip − 5, 16)`) at the strip's right end, shown when the clip is
    wider than 60 pt.
  - **Transitions:** a white-15 % wash across the cut as wide as the transition, a 1 pt seam
    line and a 20 pt seam button (`r.xs`, transition glyph); `overlay.selection` when selected;
    dragging its edges changes the duration (FX owns the rules).
- **Playhead:** a 1 pt line in `playhead` through the ruler and lanes, with a **pentagon head**
  in the ruler: an 11 pt-wide block 7 pt tall ending in a 5 pt point on the line, outlined
  1 pt in `bg.ruler` so it reads over tick labels.
- **Overlays:** rubber band on empty lane floor (`overlay.selection` + 1 pt `accent`,
  committed on release, `Shift` adds); drop ghost while media is dragged in (the would-be clip
  at 80 % with a 2 pt solid `accent` outline, placed where the drop will land); blade line
  (1 pt `accent` at the frame under the pointer) while the blade tool is on; hover-skim line
  (1 pt `text.tertiary`) while skimming; Assistant ghosts (`clip.proposed`).
- **Scrolling and zoom:** a thin vertical scrollbar at the header column's right edge and a
  thin horizontal one along the bottom of the lanes (§13.24). Plain wheel scrolls tracks;
  horizontal wheel or `Shift` + wheel pans time; `Ctrl` / `Cmd` + wheel or pinch zooms about
  the pointer, proportional to the wheel distance (at most ×2 or ÷2 per event); toolbar
  buttons zoom by ×1.25; *Zoom to fit* fits the sequence. While scrubbing or dragging clips
  within 40 pt of the lanes' left or right edge, the view scrolls along. The header column
  scrolls with the lanes.
- **Gestures** (behaviour per TL): pressing a clip selects it first; under 4 pt of travel it is
  a click that also moves the playhead; within 6 pt of an edge it trims; snapping radius 8 pt.

*R5 decisions (2026-10-13).* What shipped differs from the text above in these places:
the sequence tab strip is read-only with one tab ("Sequence 1"), because TL has no
operations yet to add, rename, reorder or close sequences; the toolbar holds only commands
FreeCut already has (add a track, undo, redo, split, delete, add a title, the Mixer,
snapping, the magnetic main track, zoom out / slider / in / fit), and the blade, hand,
skim, follow-main and ripple-all buttons wait for their commands; lane heights picked in a
header's `...` menu are view state (§18 #9) and the menu also renames and deletes the track
through the existing undoable edits. Filmstrip frames are decoded on their own worker:
exact frames for time buckets up to 2 s (so files with long keyframe gaps don't repeat one
keyframe), the nearest keyframe for longer buckets; buckets are powers of two from 1/8 s
so zooming reuses frames, and the nearest ready frame stands in while a tile's own is
decoded. The volume pill sits by the pointer rather than at the line's left end, where a
fade-in would put it on the band's floor; the line, dots and pill take white or
`#111214` by the body's luminance so they read in Light too. Dragging the ruler's keyframe
diamonds, the rubber band on the lane floor (an empty-lane drag still scrubs, as before)
and the drop ghost's full clip preview (a lane highlight and a drop line show instead) are
not built yet.

### 13.27 Import button — custom *(new)*

28 pt tall, `r.md`, 16 pt side padding, import glyph 14 + label `type.label` at 12 pt, ground
`bg.panel` with a 1 pt **dashed** `border.strong` outline (4 pt dashes, 4 pt gaps, solid
corners) that says "drop files here" without a separate drop area. Hover: outline `accent`,
fill `bg.field`, label `text.primary`. While importing: 40 % opacity, "Importing… (3)", not
clickable. The whole window still accepts dropped files (PM).

### 13.28 Sheets: modals, export and Preferences — custom frames on stock containers *(new)*

- **Modal sheet** (new project, confirmations, settings that need a decision): `overlay.scrim`
  behind; `e3`, `r.xl`; width `min(600, window − 32)` unless the owner asks otherwise; a 44 pt
  header (centred title in `type.label`, close icon button at the right, hairline below);
  footer buttons right-aligned with the primary (accent) button last. Form rows: an 88 pt
  label column with `type.micro` capitals in `text.tertiary` as labels, 1 pt hairlines between
  rows; choice ladders as chips (§13.9) or a segmented control.
- **Export sheet** (EX owns behaviour): a large sheet that slides up over `dur.slow`, as wide as
  `min(window − 48, 1160)`; 48 pt header with a centred title and the close button (absent
  while an export runs); content in 3 columns at window widths ≥ 1100 pt, 2 at ≥ 760 pt, else
  1, 12 pt apart; sections as in §6.2 with 32 pt headers, help badges and an 88 pt label
  column; a *Preview* section (current frame + seek track), a *Summary* fact list (rows of
  `bg.card` separated by 1 pt hairlines, labels `text.secondary`, values tabular
  `text.primary`), quality as three 72 pt cards (name, one-line note, size estimate; current =
  `overlay.selection` + 1 pt `accent`); footer with the primary button 34 pt tall, at least
  280 pt wide, reading "Export · ≈ 120 MB"; running / done / failed shown in the footer with a
  4 pt progress bar (`accent`, `success` when done) and failures in a `danger`-tinted box with a
  *Copy details* chip.
- **Preferences** (UI-018, stays non-modal): a fixed 680 × 480 pt window (the sheet never
  resizes between pages), `r.xl`, `e3`; a 44 pt header with centred title and close; a page
  side list (§13.22, 168 pt, 30 pt rows with 16 pt icons, the search field on top) beside the
  page; settings as sections (§6.2): a boolean setting is a section header with a switch and
  no body; Theme and Density are segmented controls; help badges carry the hints; UI-018's
  ↺ marker on changed settings and its *Restore section* stay.

### 13.29 Home screen — custom layout *(new; UI-016 presentation)*

The Home screen sits on `bg.app` like the editor. A **rail** panel on the left (`bg.panel`,
`r.lg`, 8 pt from the window edges): 200 pt wide, 56 pt (icons only, labels in tooltips)
under a 960 pt window, hidden under 720 pt (its actions then sit beside the heading as 34 pt
round buttons). Rail rows are 36 pt, `r.md`, 16 pt icon + `type.label`; idle `text.secondary`,
hover `bg.field`; *New project* at the top, *Open project…* at the bottom. **Page:** a column
centred at most 1120 pt wide, side margin `clamp(width / 16, 24, 64)`, top margin
`clamp(height / 10, 32, 96)`; heading "Projects" in `type.display`, 8 pt, a one-line subtitle
in `type.body` at 14 pt `text.secondary`, 28 pt, then the **grid**: auto-filled columns at least
216 pt wide, 16 pt gaps; each card `r.lg`, `bg.panel`, 1 pt `border.subtle`, a 16:9 poster on
`bg.well`, then 64 pt of text (name `type.label`, details and "edited 2 days ago" in
`type.small` `text.tertiary`). The first card is *New project* (a centred `accent` plus and a
label). Hover: `bg.card` + `border.strong` and a × (24 pt circle on `overlay.badge`) in the
poster's top-right corner for PM-017's *remove from list*. PM-001's quick-start canvas buttons
sit between the subtitle and the grid as a row of chips with small aspect glyphs; recovery and
safe-mode banners (§13.13) sit above the heading; a footer line with the version and the user
guide link closes the page in `type.small` `text.tertiary`. The new-project dialog is a §13.28
modal sheet whose primary button is the accent *Create*. The contents of Home stay PM-001 and
PM-017's; this section only lays them out.

**R6 decisions (2026-10-13).** How R6 maps §13.12, §13.28 and §13.29 onto what exists:

- *Recent projects.* Home's grid needs PM-017's registry, which didn't exist: `recent.json`
  sits next to `settings.json`, is read once on the worker pool (which also marks files that
  are gone as "Not found"), is updated on open and save, keeps 200 entries and is written a
  second after a change by the workspace store's writer. File ▸ Open Recent lists ten with
  *Clear List*. Home's × and *Remove from list* forget an entry; the file stays.
- *Covers.* PM-001's mvp tile is neutral; R6 adds the cover early: when a save succeeds, the
  frame on the Stage is scaled to 480 pt wide and written as `covers/<hash>.bmp` on the pool
  (an uncompressed format FreeCut writes and reads itself, so no new crate); Home reads covers
  on the pool. Without a cover the card keeps the neutral tile at the canvas's shape.
- *Times.* "Edited …" is relative all the way ("Just now", "12 min ago", "3 h ago",
  "Yesterday", "4 days ago", "2 weeks ago", "5 months ago"): PM-001's "Today 14:32" needs the
  local time zone, which the app has no source for yet.
- *Home.* The rail also shows a current *Projects* row (where later pages such as Recent
  exports go). Quick-start chips create a project with that canvas at 30 fps at once; the
  *New project* card and rail row open the sheet. The footer links to About, since the user
  guide (UI-051, v1) doesn't exist. At launch the keyboard starts on *New project* (UI-016).
- *Toasts.* Every message the app had for the status line is now a toast (UI-040's single
  API); the status line keeps only the export's task indicator. Kinds in use: info (most
  messages), success (saved, export finished with *Show in folder*), warning (a settings file
  that was set aside), error (open and save failures, save with *Retry*; export failed with
  *Details*). Toasts sit above modal sheets.
- *Export sheet.* Sections: Preview (the Stage's frame and a seek track that moves the
  playhead, no undo entry), Summary, Video, Audio (its switch in the header), File (name,
  folder, Replace / Keep both as chips in a note box), Quality (three cards: Higher,
  Recommended, Smaller), Checks (the preflight list; clips stay clickable). While an export
  runs, finishes or fails the right-hand columns show its state and the footer its bar and
  buttons; Esc and the scrim still hide a running export, which carries on.
- *Preferences.* Stays a non-modal window, now 680 × 480 with a 168 pt page list (search on
  top). Each setting is a section: title, help badge, and UI-018's reset marker only while the
  value differs ("Changed ↺"). Reduce motion is a section with its switch and no body. Esc does
  not close it (it didn't before).
- *Menus.* The top bar's menus rest without fills, 26 pt rows, at least 200 pt wide. Context
  menus elsewhere keep egui's menu look with the theme's tokens.

## 14. Iconography and fonts

*Rev. 2: sizes updated; checked states use the Fill weight on a well chip.*

- **One general icon set:** **Phosphor Icons** (MIT), *Regular* weight for controls and
  *Fill* weight for selected, checked or "on" states, shipped as the upstream icon font and
  registered in egui's `FontDefinitions` with our own codepoint constants
  (`freecut-app::theme::icons`), so we don't depend on a wrapper crate keeping pace with
  egui releases. Alternative if Phosphor ever stops fitting: **Lucide** (ISC; contains icons
  derived from Feather, MIT), taken from its upstream release. MIT and ISC are permissive and
  GPL-3.0-compatible. We never mix two general sets in one build.
- **Our own glyphs** for editing-specific symbols no general set covers well: keyframe
  diamond states, In / Loop / Out, magnetic main track, ripple, snap, blade, proxy, the
  Assistant mark, clip-kind badges, the playhead head. They are drawn by contributors as
  vector paths painted in code (`theme::glyphs`) on a 16 pt grid with 1.5 pt strokes and round
  joins, matching Phosphor Regular's weight. No glyph is traced or adapted from another editor.
- **Sizes:** 16 pt in toolbars, the transport (play 18) and above Shelf tab labels; 14 pt in
  panel headers, track headers and inspector rows; 12 pt inside filled 26 pt buttons (Export)
  and select carets; 10–11 pt chevrons and navigator glyphs; 20 pt on collapsed rails; 28 pt in
  empty states. Icons take `text.secondary` at rest and `text.primary` on hover; checked icons
  take `accent.text` with the Fill weight; colour never carries meaning on its own (a state
  change also changes weight Regular → Fill or adds a mark).
- **Replace the emoji glyphs** the mvp tab row and transport use today with set icons.
- **Fonts:** Inter and JetBrains Mono are added to `assets/fonts/` with OFL-1.1 and listed in
  `assets/ASSETS.md`; Noto Sans is already bundled there. Licence texts appear in the About
  window (UI-051).

## 15. Accessibility checklist

| Need | How the concept meets it |
|---|---|
| Contrast | Token table §12 meets 4.5 : 1 for body and tertiary text on every surface text sits on (ground, well, panel, card, field, hover, active, selection), 3 : 1 for control boundaries and focus; High contrast meets 7 : 1 for text; a unit test checks the table (UI-013). There is no "dim" text token below 4.5 : 1. |
| Visible focus | *Rev. 2:* the ring appears for **keyboard** focus (focus that arrived by `Tab`, arrow keys or `F6`), not after a mouse click, on buttons, tabs, tiles, segmented controls and badges; text fields show a 1 pt `accent` border whenever they are being edited. The ring is 2 pt `accent` with 1 pt offset (3 pt yellow in High contrast); the focused place has its 2 pt top line (§3.4). Stock egui widgets get it as the border of their `active` state and as the text field's selection stroke; in High contrast every frameless control (icon buttons, badges, the segmented track) also draws a `border.control` outline and panels a 1 pt outline. |
| Not colour alone | Keyframe states by shape, switches with a check mark, checked tools by the Fill glyph on a well chip, changed values by a dot, selected clips and tiles by a thicker outline, clip kinds with icons, Assistant ghosts with hatching and dashed outlines, error toasts with an icon and bar. |
| Hit targets | ≥ 28 pt for icon buttons in Comfortable (26 pt chip), never < 24 pt; visuals may be smaller than their hit areas (canvas handles 10 pt in 20 pt, splitters 1 pt in 7 pt). The keyframe navigator's arrows (18 pt wide) have equivalent commands (§13.6). |
| Keyboard | §4 navigation; every custom widget implements arrow / `Enter` / `Space` / `Esc`; no keyboard traps outside modals (UI-041). Shortcuts fire only when no text field has taken the key. |
| Screen readers | AccessKit via eframe; every custom widget in §13 creates a node with role, name, value and state (UI-042). |
| Reduced motion | §12.8. |
| Scalable UI | Interface size 75–200 % (UI-015) × two densities; layouts reflow at the §5.1 breakpoints, which are measured after scaling. The smaller rev. 2 type scale is the 100 % default; nothing interactive is below 11 pt. |
| Reading load | One accent, ≤ 3 text levels per section, summaries instead of open sections, Essentials by default. |

## 16. Mapping to egui 0.36

Checked against the egui 0.36.2 sources in the local registry (rev. 2 additions re-checked on
2026-10-13).

### 16.1 What stock egui gives us

| Need | egui 0.36 API | Notes |
|---|---|---|
| Fixed regions, resizable splits | `Panel::left / right / top / bottom` + `CentralPanel` | 0.36 merged the old `SidePanel` / `TopBottomPanel` into `Panel`; UI-001's notes should use the new names. |
| Floating panels on a ground *(rev. 2)* | `Panel::…frame(Frame::NONE.fill(bg.app).inner_margin(…))` + `show_separator_line(false)`, then a `Frame::new().fill(bg.panel).corner_radius(r.lg)` inside | Each side panel's ground margin is half a gutter on the sides it shares, so neighbours add up to 8 pt. The stock resize line is drawn with the parent style's `widgets.hovered / active.fg_stroke`: set those to a 2 pt `accent` stroke in a style scope around the panel calls and restore them inside the panel body. A fully custom dock (pure layout + `scope_builder(UiBuilder::new().max_rect(..))`) can come with UI-054. |
| Rail ↔ expanded panels (asset rail, inspector rail) | `Panel::show_switched` | Animates between a thin and a thick panel and shares one resize handle, which matches the collapse-to-rail behaviour of UI-002. |
| Peeking asset panel, overlay sheets | `Area` with `e2` frame for peeking; `Panel::show_collapsible` (slide) for pinned | |
| Window chrome *(rev. 2)* | `ViewportBuilder::with_fullsize_content_view(true)`, `with_title_shown(false)`, `with_titlebar_shown(false)` (macOS); `ViewportCommand::StartDrag`, `ViewportCommand::Maximized(..)` | Top bar background registers `Sense::click_and_drag()` *before* its controls so the controls win; `with_decorations(false)` only if FreeCut draws its own caption buttons later. |
| Modal dialogs, sheets | `Modal` (with `backdrop_color`, custom `frame`) | Export sheet: `Area` at `Order::Foreground` over a click-blocking scrim, y driven by `animate_bool_with_time`, or `Modal` with a large frame. |
| Menus, popovers, colour popover, Select list | `Popup`, `menu`; `Popup::from_toggle_button_response(&resp).align(RectAlign::BOTTOM_END).gap(4.0)` | Menu labels in the top bar: `MenuButton::from_button(Button::new(..).frame_when_inactive(false))`. |
| Tooltips | `Tooltip`, `Response::on_hover_text`; `style.interaction.tooltip_delay = 0.5`, `tooltip_grace_time = 0.6` | Grace time gives §12.8's "next tooltip at once". |
| Two-sided header rows | `Sides` | section headers, panel headers, top bar, transport side groups |
| Toggle-style buttons, chips | `Button::selectable`, atoms | |
| Virtualized lists and grids | `ScrollArea::show_rows`, `show_viewport` | preset grids, Drawer, effect stacks |
| Thin scrollbars, wheel-only scrolling *(rev. 2)* | `style.spacing.scroll = ScrollStyle { bar_width: 4.0, bar_inner_margin: 2.0, handle_min_length: 24.0, floating: true, ..ScrollStyle::thin() }`; `ScrollArea::scroll_bar_visibility(VisibleWhenNeeded)`; `ScrollArea::scroll_source(ScrollSource::SCROLL_BAR \| ScrollSource::MOUSE_WHEEL)` | The default `DragScroll::OnTouch` already ignores mouse drags; the explicit source documents the rule. |
| Animation | `Context::animate_bool_with_time(_and_easing)`, `animate_value_with_time`, `Style::animation_time`; colours with `Color32::lerp_to_gamma` | durations from §12.8; 0 under reduced motion |
| Drag and drop *(rev. 2)* | `Ui::dnd_drag_source`, `Response::dnd_set_drag_payload / dnd_hover_payload / dnd_release_payload`, `Ui::multiply_opacity(0.45)` on the dragged tile | assets to timeline (`AssetDrag`), tab reorder |
| Painting *(rev. 2)* | `Shape::dashed_line` (Import outline, guides), `Shape::convex_polygon` (playhead head, diamonds), `Painter::image` with a UV rect (filmstrip tiles, cover crop), `Shadow::as_shape`, `Image::corner_radius`, `CornerRadius { nw, ne, sw, se }` per corner | |
| Text details *(rev. 2)* | `TextFormat::extra_letter_spacing` (capitals, display heading), `LayoutJob` with one row and an overflow character (ellipsis) | |
| UI scale | `Context::set_zoom_factor` | UI-015 |
| Theme mapping | `Visuals`: `panel_fill`, `window_fill`, `extreme_bg_color` (= `bg.well`), `text_edit_bg_color` (= `bg.field`), `faint_bg_color`, `selection`, `widgets.{noninteractive, inactive, hovered, active, open}` (inactive / hovered / active `weak_bg_fill` = `bg.field` / `bg.hover` / `bg.active`), `window_corner_radius` (`r.xl`), `menu_corner_radius` (`r.lg`), `window_stroke` (`border.strong`), `window_shadow`, `popup_shadow`, `warn_fg_color`, `error_fg_color`, `hyperlink_color`, `text_cursor`, `disabled_alpha` | `CornerRadius` and `Shadow` fields are `u8`/`i8`, which every token value fits. Translucent tokens are built premultiplied (`Color32::from_white_alpha`, `from_black_alpha`). |
| Spacing | `Spacing`: `item_spacing` (6, 6) / (4, 4), `button_padding`, `interact_size.y` (28 / 24), `menu_margin` (6), `indent`, `slider_rail_height`, `scroll` | set per density |
| Text styles | `TextStyle::{Small, Body, Button, Heading, Monospace}` + `TextStyle::Name("micro" / "caption" / "label" / "title" / "display")` | Inter via `FontDefinitions` |
| Colour picking | `widgets::color_picker` | square and strips reused inside our popover |
| Accessibility | eframe feature `accesskit` | custom widgets add nodes |
| Thumbnails, filmstrips | `TextureHandle` updated from worker results | upload on the UI thread is a copy, never a decode |

### 16.2 What we build ourselves

Property section header and row grid, slider-with-field, segmented control (with its sliding
thumb), switch, keyframe navigator, number field with unit segment, Select, colour field,
help and reset badges, preset grid tiles with animated thumbnails, media tiles and the
auto-fill grid, Shelf tabs, side list, Import button, In/Loop/Out strip, panel headers, gutter
splitter line and corner masks, transport and timecode field, context strip, command bar result
list, Assistant proposal card, toasts, banners, empty states, all canvas overlays (handles,
guides, readout pill, ghosts), the Home grid, and the timeline (already custom; rev. 2 splits
it into tabs, toolbar, ruler, header column and lanes).

### 16.3 Implementation rules

- **Tokens first:** `theme::Tokens { dark, light, high_contrast }` as `const` tables;
  `theme::apply(ctx, &Tokens, Density)` maps them to `Style`/`Visuals` once per change, not
  per frame.
- **No work on the UI thread:** thumbnails, filmstrip frames, font previews and preview loops
  come from `freecut-render` / `freecut-media` workers as pixel buffers; the UI uploads them to
  textures and repaints with `request_repaint_after(1 / fps.thumb)` only while a tile is
  animating.
- **Rounded clipping:** egui clip rects are rectangles. Full-bleed children of a panel either
  paint with matching per-corner radii, or the panel paints four small corner masks in the
  ground colour over its content (a fan of triangles from each corner point to a quarter
  arc), which is exact because the ground is flat. One helper owns this.
- **Pure geometry:** every layout rule with numbers in it (gutters, grid columns, cover-crop
  UVs, ruler tick interval, lane heights, sound-band height, export column count, Home grid,
  zoom factor, corner-mask points) is a pure function with unit tests; painting code only
  calls them.
- **Budgets:** inspector build under 2 ms for a text clip with all sections open; Drawer grid
  of 500 items scrolls at display rate (virtualized); the timeline requests filmstrip frames
  only for visible tiles.
- **Stable ids:** every section, row and tile gets an `Id` from (selection kind, section id,
  row id), so collapse state and focus survive selection changes.
- **Undo boundaries:** composites report `gesture_started` / `gesture_ended` so the
  inspector can coalesce into one undo entry (UI-009, TL-043).

## 17. Rollout slices

Each slice is end-to-end, shippable and testable on its own (Pragmatic Programmer's
tracer-bullet approach).

| Slice | Delivers | Touches |
|---|---|---|
| S1 | Tokens, Dark/Light/High-contrast `Visuals`, density, Inter + JetBrains Mono, icon font | `theme`, assets |
| S2 | Property card + row + gutter; convert today's Text, Transform and Audio sections | `widgets`, `ui::inspector`, `ui::props` |
| S3 | Lens bar; text lenses Content and Style (mvp rows) and Box & bubble (Placement) | `ui::inspector` |
| S4 | *Superseded by R2:* Shelf rail + Drawer (pinned) replacing the asset tab row; chips; empty states | `ui::assets` |
| S5 | Command bar (UI-034) with commands, settings and assets | `commands::palette` |
| S6 | Stage context strip; Esc-cancel on canvas drags | `ui::player`, `ui::canvas` |
| S7 | Preset grid with live thumbnails; Motion lens with timing strip (when TX-019/TX-022 land) | `widgets`, render workers |
| S8 | Assistant face, proposal card and ghost previews (AS-001, AS-007, AS-008; milestones M-AI1 and M-AI2) | `assistant_ui` (UI only) |
| S9 | Breakpoints: Compact peeking asset panel, Tight overlay sheets, Wide Assistant panel | `shell` |

### 17.1 Comfort reskin slices (rev. 2)

The rev. 2 look ships as six slices on top of what S0–S3 already built. Each keeps the app
working and ends with fmt, clippy and tests green plus a manual check-list pass.

| Slice | Delivers | Touches |
|---|---|---|
| R1 | Rev. 2 tokens, radii, type scale and metrics; lifted controls; thin scrollbars and tooltip timing; ground + floating panels + gutters; slim top bar with project name, save state and accent Export; macOS full-size content | `theme`, `widgets::{panel, focus_ring, icon_button}`, `app.rs` (editor shell, top bar), `main.rs` |
| R2 | Shelf tab strip, Drawer side list with counts, auto-fill media grid with cover thumbnails and badges, Import button, sort select, library selection | `ui::{assets, media, audio, text}`, new `widgets::{shelf_tab, side_list, tile}` |
| R3 | Stage panel header, surround and ring, transport bar with centred play and timecode field, preview resolution select, zoom and pan | `ui::{player, canvas}`, `theme::icons` |
| R4 | Inspector restyle onto the S0 widgets: panel header + lens strip, drawer sections, right-hugging rows, unit segments, sliding segmented thumb, Select, colour field, help / reset badges, navigator with greyed arrows | `widgets::*`, `ui::inspector::*`, `ui::{style_cards, box_cards, rows}` |
| R5 | Timeline restyle: sequence tabs, icon toolbar (UI-011), icon track headers with resizable column, lanes and grid, two-tone clips, filmstrip thumbnails (background job via `freecut-media`), bar waveforms, pentagon playhead, thin scrollbars, wheel zoom | `ui::timeline` (split), new `ui::timeline_toolbar`, new `thumbs` job, `theme::surface` |
| R6 | Export sheet, Preferences sheet, modal sheet style, Home screen and new-project dialog, toasts | `ui::{export, preferences}`, `app.rs` (`home`, new-project dialog) |

## 18. Open questions

1. **Lens naming vs. owning specs.** Resolved 2026-10-12: TX-022, FX-016 and FX-018 now
   call the animation tab the **Motion** lens.
2. **New shortcuts** (`Ctrl+K` command bar, `Ctrl+5` Assistant, `Ctrl+Backspace` reset focused
   row) need entries in UI-031.
3. **Accent hue** changes from the earlier blue (`#5B8CFF`) to Iris; a quick check with
   colour-vision simulation against the clip palette is still to be done in CI (UI-043).
4. **Assistant scope in multi-window** (UI-057): settled by AS-001: each window has its own
   panel, conversation and proposal.
5. **Peek timeout** (1.5 s) for the collapsed asset panel should be validated with users; it
   may need to be a preference.
6. *(Rev. 2)* **UI-009's per-row reset arrow** moves to the label (double-click), the changed
   dot, the row context menu and the section's reset badge (§6.2). UI-009's text should be
   updated to point here.
7. *(Rev. 2)* **UI-011** says the toolbar is 36 pt and toggles show an accent fill; rev. 2 draws
   it 40 pt (34 Compact) with *checked* chips (§13.26). UI-011 should point here.
8. *(Rev. 2)* **UI-002's dividers** (hit area 6 pt, 1 pt line, 3 pt on hover) are now gutters
   (§3.4, §13.17); its text should point here.
9. *(Rev. 2)* **Track heights:** is the per-track height (Auto / Small / Medium / Large) view
   state or saved in the project? If saved, TL needs a `TrackHeader` field and an undoable op.
10. *(Rev. 2)* **User-selectable accents** (a row of accent swatches in Appearance) are not in
    any spec; Iris stays the only accent unless UI-014 adds the option and the contrast test
    covers every choice.
11. *(Rev. 2)* **Asset browser arrangement:** rev. 1 avoided a horizontal category strip with a
    side list because the reference app the specs observe also uses one; rev. 2 adopts that
    common arrangement for comfort (with FreeCut's own grouping, counts, tokens and names).
    Confirm the trade-off with the maintainers.
12. *(Rev. 2)* **Own window decorations on Windows / Linux** (caption buttons drawn by FreeCut,
    so the top bar can reach the top edge as on macOS): later, because native resize borders and
    snap layouts must keep working.
13. *(Rev. 2)* **12 pt body text:** validate with users at 100 % interface size on a 1080p
    laptop; if it reads too small, raise `type.body` to 12.5 pt rather than going back to 13.

## 19. Clean-room notes

- Revision 1 was designed from FreeCut's own principles and from widely known editor
  conventions. Knowledge of the reference app the feature specs observe was used only to list
  which features exist (via our own paraphrased specs) and to make sure FreeCut's
  arrangement, names and look are its own. No screenshots, artwork, icons, colours, layouts
  measured from that app, or label texts were copied.
- Revision 2 adds layout and comfort patterns from an open-source editor studied as a design
  reference (§20). From it we took only what a careful user could describe: how its window is
  arranged, how big things are, how states look and how controls behave, restated in our own
  words. Its measured numbers were treated as a starting point: FreeCut's colours, accent,
  icons, fonts, names, wording and many sizes are its own (§12). Its source code, markup,
  icons, images, fonts and texts were not copied, and its name and logo appear only in §20.
- All names in this document (Quiet Stage, Shelf, Drawer, Stage, lens, section, Context
  strip, Command bar, Iris, ground, gutter) are FreeCut's own or plain words. The assistant is
  called by its generic name, as in assistant.md.
- Icons: Phosphor (MIT) or Lucide (ISC), from their upstream releases, plus glyphs drawn by
  contributors. Fonts: Inter, JetBrains Mono, Noto Sans (all OFL-1.1). All are compatible with
  GPL-3.0-or-later and are listed in `assets/ASSETS.md` when added.
- Sample footage used for preset thumbnails is FreeCut's own CC0 material (UI-007).

## 20. References

- Layout and comfort patterns studied from Concat (AGPL-3.0-or-later, open source) as a design
  reference; no code, markup, icons, images or text were copied; the Concat name and logo are
  not used.
- The study notes (measured values and behaviour in our own words) were kept outside the
  repository; this page is the only place their conclusions are recorded.

*Review fixes (2026-10-13), settled defaults:* the asset toolbar's search field keeps at least
150 pt. Where the label would leave it less, Import shows its glyph alone (label in the tooltip)
and the sort select drops its caret. In a narrow Stage the transport folds the length, then
*Fit*, then the speaker's master-volume chevron (the Mixer still has it), and only then the
preview resolution. The − / + steppers keep a 2 pt gap so High contrast outlines them as two
buttons. A Home card whose project file is gone dims its cover only: the name is `text.secondary`
and the details stay at `text.tertiary` at full opacity. In the Export sheet the preview gives up
height (between 120 and 260 pt) so the Summary under it ends inside the body without scrolling.
