# Projects & media library
> Status: draft · Owns: project settings (canvas size and aspect ratio, sequence frame rate, audio sample rate and channels, project background color, default fit for mismatched media, still-image default duration), the project file and its lifecycle (create, open, save, autosave, recovery, backups, versioning and migrations, portability, packaging), media references (media items, `MediaId`/`MediaRef`, bins, probe data, interpretation overrides, media status, relink), the media `Fingerprint` (the only media content key, PM-038), the project-level index of referenced asset files, the generated-media store (PM-041), the media cache root with its size limit and eviction (thumbnails, filmstrips, waveform peaks, and every other deterministic, regenerable output, including AI's analysis records; PM-074), the app's single background-task list (PM-073), proxies, camera recording into the project (PM-043) and the bundled CC0 sample media pack (PM-063), the Deleted projects hold (PM-104), the project's list of timelines and their per-timeline settings (PM-107; TL owns what is inside each sequence), color clips (PM-108), disk-space monitoring (PM-113), storage-location advisories (PM-114) and screen recording into the project (PM-115) · Crates: freecut-project, freecut-media, freecut-timeline, freecut-app

## Summary
This area covers what happens before and around editing. A FreeCut user creates a project, or reopens one from the start screen. They pick a canvas shape (16:9, 9:16, 1:1, 4:5, 21:9, 4:3, 3:4 or custom), or (from v1) let the project adopt the shape and frame rate of the first clip they add. They bring in video, audio, still images, image sequences and whole folders, either with a file dialog or by dragging them in. The media panel lets them browse, organize, search and preview what they imported. Behind the scenes, FreeCut probes every file, generates thumbnails, filmstrips and waveform data, optionally builds low-resolution proxies, saves continuously, keeps backups, and tells the user when files go missing so they can be relinked. A finished project can be collected into one portable folder. From v1, a small pack of FreeCut's own CC0 sample clips and stills lets people try things out before they have footage of their own, deleted projects can be restored for 30 days, and FreeCut warns before low disk space or slow drives get in the way. Later, a project can hold several timelines, and the webcam or the screen can be recorded straight into the project. Everything is local: there are no accounts, no cloud drafts and no uploads.

Conventions used below: `Mod` means Ctrl on Windows/Linux and Cmd on macOS. Key bindings are proposals, and the final keymap belongs to [workspace-and-shortcuts.md](workspace-and-shortcuts.md). "(FreeCut default)" marks a value we chose because public information was missing, inconsistent, or because we deliberately differ. Time values use TL's rational ticks and `FrameRate` type ([timeline-editing.md](timeline-editing.md), TL-001). Default locations come from the `directories` crate (MIT/Apache-2.0):

| What | macOS | Windows | Linux |
|---|---|---|---|
| Projects (default folder) | `~/Movies/FreeCut` | `%USERPROFILE%\Videos\FreeCut` | `$XDG_VIDEOS_DIR/FreeCut`, falling back to `~/Videos/FreeCut` |
| App data (registry, backups, recovery, view state, presets, Deleted projects hold PM-104, recent-exports list) | `~/Library/Application Support/FreeCut` | `%APPDATA%\FreeCut` | `$XDG_DATA_HOME/freecut` |
| Media cache (PM-074: thumbnails, filmstrips, peaks, proxies, and regenerable outputs such as stabilization data, interpolated frames, mattes, analysis records, processed audio and pre-renders) | `~/Library/Caches/FreeCut` | `%LOCALAPPDATA%\FreeCut\Cache` | `$XDG_CACHE_HOME/freecut` |
| Bundled sample media pack (PM-063, read-only, optional) | `FreeCut.app/Contents/Resources/samples` | `<install folder>\samples` | `<prefix>/share/freecut/samples` |

A project on disk (FreeCut default layout):

```
My Trip.freecut            project file (UTF-8 JSON)
My Trip.freecut.lock       present only while the project is open (PM-013)
My Trip Media/             created on demand
  Imported/                copies made by "copy media into project" (PM-039)
  Samples/                 copies of bundled sample media used in this project (PM-063)
  Generated/               generated-media store: files clips point at, such as TTS audio, stems,
                           upscaled or inpainted media, frame snapshots (PM-041)
  Recordings/              voiceover (AU-027) and camera (PM-043) recordings, also part of the store
My Trip Proxies/           only if proxies are stored with the project (PM-080)
```

Deterministic, regenerable outputs never go into `<Name> Media/`. They live in the media cache (PM-074), following PM-041's storage rule.

## Features

### PM-001 Start screen & project library — mvp
**What the user sees:** On launch, and after File > Close Project, a start screen appears. It has a large **New project** button, a row of quick-start canvas buttons (16:9, 9:16, 1:1, 4:5, 21:9), an **Open…** button, and a grid of project cards. Each card shows a cover tile, the name, timeline duration, a canvas badge (for example "9:16 · 1080p"), and when it was last modified. In the mvp the cover tile is a neutral tile at the project's aspect ratio; real covers arrive with PM-019 (v1). A project whose file can't be found shows a dimmed card marked "Not found", with its last known path in a tooltip.
**Inputs / controls:**
- Click or double-click a card to open it. You can also drop a `.freecut` file anywhere on the window.
- Card menu (right-click or "⋯"): Open, Reveal in file manager, Remove from list. The v1 actions are in PM-018.
- The mvp lists cards by last modified, newest first. In v1: a search field that filters cards live by name (case- and accent-insensitive substring match), a sort menu (Last modified, Name in natural order so "Trip 2" comes before "Trip 10", Date created, Duration), and a grid/list toggle.
- From v1, tabs above the grid: **Projects** (default), **Recent exports** and **Deleted projects** (PM-104). Later, PM-105 adds folder cards and a breadcrumb inside Projects.
- From v1, with several cards selected (PM-018), a bar shows the count ("3 selected") and the actions that apply to all of them.
**Behavior / output:**
- **Display formats (FreeCut defaults):** last modified is relative while it is recent: "Just now" (under 1 min), "12 min ago" (under 1 h), "Today 14:32", "Yesterday 09:10", then the date in the system's short format. Timeline duration uses `h:mm:ss` from one hour up and `m:ss` below it ("0:42", "1:02:03"); an empty timeline shows "0:00".
- The list combines two sources: `.freecut` files found directly in the default projects folder (a non-recursive scan on a background thread), and the recent-projects registry (PM-017). Entries are de-duplicated by project id.
- Cards appear progressively as headers are read. Only the header object of each file is deserialized for the card (PM-021). The UI thread never touches the disk.
- From v1, the cover image comes from app data (`covers/<project-id>.webp`), written at save time (PM-019). If there's no cover, the neutral tile is drawn.
- **Recent exports (v1):** the last 50 finished exports from any project, newest first. Each row shows the output file name, the project it came from, the date, size and duration, with **Show in folder**, **Open** and **Open project**. EX-039 appends a record `{ path, project_id, project_path, finished, bytes, duration, width, height }` to `exports.json` in app data when an export finishes; this tab only reads it. A file that no longer exists is greyed out with "File moved or deleted". **Clear list** empties the list, never the files.
- **Help link (v1):** below the grid, **Where are my projects?** opens a short panel that names the default projects folder (with **Reveal**), offers **Open…**, points to Deleted projects (PM-104) and to File ▸ Restore from Backup (PM-016), and suggests checking whether the drive holding the project is connected.
**Edge cases:**
- **No projects yet:** the grid area shows a single invitation to create the first project with **New project**, plus a note that a `.freecut` file can be dropped there.
- **Search with no matches (v1):** "No projects match '<query>'", with **Clear search**. Projects in Deleted projects are never matched here.
- **Hundreds of projects:** the grid is virtualized. The folder scan stops at 2,000 files and shows "Showing the first 2,000 projects" (FreeCut default).
- **Corrupt or unreadable file:** the card shows a warning icon and "Can't read this project". From v1, opening it offers the latest backup (PM-016).
- **File written by a newer FreeCut format:** the card shows a "Newer version" badge and opens read-only (PM-021).
- **Two files with the same project id** (the user copied a project in their file manager): both are listed by path. The second one opened gets a fresh id, which is written on its next save, so backups and covers never collide.
- **Default projects folder missing or unwritable** (for example, a redirected Videos folder that is offline): the start screen still works. **New project** then asks for a location.
- **Project on an unmounted removable drive:** shown as "Not found", and when the registry knows the volume name the card says which drive to connect ("On 'Card A', not connected"). From v1 the card menu offers **Locate…** (PM-017). When the drive returns, the card comes back on the next scan or when the window regains focus.
- **Project folder deleted or renamed in the file manager:** the card is "Not found" with its last known path; **Locate…** (v1) or **Remove from list** fixes it.
**Undo:** n/a. This is view state, and file actions are not document edits.
**Model notes:** `freecut-project::read_header(path) -> Result<ProjectHeader, OpenError>`, where `ProjectHeader { id: Uuid, name, format_version, app_version, created, modified, duration: Tick, canvas: CanvasSize, frame_rate: FrameRate }`. `freecut-app` owns the start screen, the scan job and the registry. The relative-time and duration formatters are pure functions with table tests (fake clock, locale-independent parts only). The recent-exports reader lives in `freecut-app::home`; the record type is EX's.

### PM-002 New project — mvp
**What the user sees:** **New project** (`Mod+N`) opens the editor straight away with an empty timeline. No dialog blocks the way. The project is named "Project 2026-10-11 14.32" (date and time, FreeCut default) and is already saved in the default projects folder. In the mvp the canvas label near the player reads "16:9 · 1920×1080 · 30 fps". From v1, when Auto mode (PM-006) is the default, it reads "Auto: matches your first clip" instead. A quick-start canvas button on the start screen creates the project with that ratio already fixed. **New project with settings…** (File menu, or Shift+click on **New project**) opens the settings dialog (PM-011) first, with name and location fields added.
**Inputs / controls:** Name (1–200 characters). Location (folder picker; defaults to the default projects folder). All PM-003…PM-010 settings.
**Behavior / output:**
- A new project file is created with a fresh UUID (v4). It gets the new-project defaults (PM-020 in v1, otherwise the built-ins: Manual mode in the mvp and Auto mode from v1, a 1920×1080 canvas, 30 fps, 48 kHz stereo, black background, Fit, 5 s stills), an empty media library and one root sequence. TL defines the default tracks. Every field exists in `ProjectSettings` from the mvp, even when the UI to change it arrives later (PM-008, PM-009, PM-010), so the file format doesn't change when those features ship.
- The file is written atomically (PM-012) and the lock is taken (PM-013) before the editor opens.
- If a project is already open, it is saved (or the user is asked, when autosave is off) and closed first. The mvp has one project per window and one window.
- No confirmation step: the project exists as soon as **New project** is clicked. An unwanted project is removed with Move to Trash (PM-018, PM-104). (The reference app asks for confirmation in one flow; FreeCut skips it because creating is cheap and reversible.)
**Edge cases:**
- **Name already used in the folder:** " (2)", " (3)", and so on is appended to the file name. On case-insensitive volumes (default macOS and Windows), names that differ only in case count as the same.
- **Display-name rules:** leading and trailing whitespace is trimmed. A name that is empty after trimming falls back to the default date-and-time name. The display name may contain any other character, including `<` and periods.
- **Characters not allowed in file names** (`/ \ : * ? " < > |`, control characters, a trailing dot or space, Windows reserved names such as `CON` or `NUL`): replaced with "_" in the file name. A leading dot is also replaced, so the file never becomes hidden on macOS or Linux. The display name keeps the original text.
- **Location unwritable or disk full on the first write:** an error appears with PM-012's reason text, and no project is created. The user stays on the start screen or in the dialog. When the default location's volume is full, the error offers **Choose another location…** (PM-113).
- **Location on an external or network drive:** allowed, with PM-114's one-time note that editing may be slower there. (One reference-app message refuses external devices as a project location; FreeCut allows them and only advises.)
- **Location inside another project's media folder:** allowed, with a warning that packaging may nest folders.
- **Location inside FreeCut's own app-data folders** (backups, Deleted projects, cache): refused, because those folders are managed and pruned by FreeCut.
**Undo:** n/a. Creating a project is a file operation, and the new project starts with an empty history.
**Model notes:** `ProjectSettings::default()` lives in `freecut-timeline::settings`. `freecut-project::create(dir, name, settings) -> Result<ProjectHandle, ProjectError>`. The file-name sanitizer and the display-name normalizer (trim, empty fallback) live in `freecut-project`, with table tests for Windows reserved names, leading and trailing dots and spaces, case-only collisions and Unicode.

### PM-003 Canvas aspect ratio presets — mvp
**What the user sees:** A **Ratio** dropdown in Project Settings (PM-011) and the quick-start buttons on the start screen. From v1 the same list also appears in the player toolbar's ratio menu, which EX-013 owns and which calls this operation. Each entry has a small proportional rectangle and a short usage hint:
- 16:9 (landscape, typical for video sites)
- 9:16 (vertical, short-form feeds)
- 1:1 (square)
- 4:5 (portrait feed posts)
- 21:9 (ultra-wide, cinematic)
- 4:3
- 3:4
- 2:1, 1.85:1 and 2.39:1 (wide cinema shapes), v1
- Custom…
- Auto (match first clip), v1, shown only while the project is still in Auto mode (PM-006)
- Original, v1, in EX-013's menu: a one-time match to the first main-track clip using PM-006's rules, without entering Auto mode

**Inputs / controls:** Pick a preset. Custom opens the width × height fields (PM-004).
**Behavior / output:**
- A preset sets the canvas to that ratio at the current **resolution tier**, which is the length of the short side: 720, 1080 (default), 1440 or 2160. The long side is `round_to_even(tier × ratio)`. Switching 16:9 1920×1080 to 9:16 gives 1080×1920.
- 21:9 uses the common ultra-wide pixel format 64:27 (2560×1080 at the 1080 tier). 2.39:1 is stored as the exact ratio 239:100 and rounded to even pixels like the others.
- Choosing a preset switches Auto mode (v1) to Manual.
- Changing the ratio when the timeline already has clips follows PM-011.

| Preset | 720 tier | 1080 tier (default) | 1440 tier | 2160 tier |
|---|---|---|---|---|
| 16:9 | 1280×720 | 1920×1080 | 2560×1440 | 3840×2160 |
| 9:16 | 720×1280 | 1080×1920 | 1440×2560 | 2160×3840 |
| 1:1 | 720×720 | 1080×1080 | 1440×1440 | 2160×2160 |
| 4:5 | 720×900 | 1080×1350 | 1440×1800 | 2160×2700 |
| 21:9 (64:27) | 1706×720 | 2560×1080 | 3414×1440 | 5120×2160 |
| 4:3 | 960×720 | 1440×1080 | 1920×1440 | 2880×2160 |
| 3:4 | 720×960 | 1080×1440 | 1440×1920 | 2160×2880 |
| 2:1 (v1) | 1440×720 | 2160×1080 | 2880×1440 | 4320×2160 |
| 1.85:1 (v1) | 1332×720 | 1998×1080 | 2664×1440 | 3996×2160 |
| 2.39:1 (v1) | 1720×720 | 2582×1080 | 3442×1440 | 5162×2160 |

**Edge cases:**
- **Custom size within 0.5 % of a preset ratio:** labeled with that preset (for example 1080×1352 shows as "4:5").
- **Canvases above about 8.9 megapixels** (for example 5120×2160): the dialog notes that some players can't play H.264 at that size. Export picks the encoder level ([preview-and-export.md](preview-and-export.md)).
- **Ratio changed several times** (16:9 → 9:16 → 16:9): the result is identical to the start, because layout is stored in canvas-normalized units (TR). This is tested.
- **Original (v1) with no visual clip on the main track:** disabled, with the reason "Needs a video or picture on the main track".
- **Original (v1) after the first clip was replaced or removed:** it matches whatever clip is first on the main track at the moment the command runs. It is a one-time match, never a live link, so later edits don't change the canvas again. A toast confirms the new size, with **Undo**.
**Undo:** `SetProjectSettings { before, after }`, labeled "Change aspect ratio".
**Model notes:** Only `CanvasSize { width: u32, height: u32 }` is stored. `AspectPreset::classify(CanvasSize) -> AspectPreset` and `AspectPreset::size_for(tier) -> CanvasSize` are pure functions in `freecut-timeline::settings` with table tests. The 0.5 % tolerance is checked with integer cross-multiplication, not floats. The preset is never stored separately, so it can't contradict the size.

### PM-004 Canvas resolution — mvp
**What the user sees:** A **Resolution** dropdown listing the tiers for the current ratio with exact pixels: "720p · 1280×720", "1080p · 1920×1080", "1440p · 2560×1440", "2160p (4K) · 3840×2160". It also has Custom width × height fields with a link-proportions toggle, and a **Swap** button (⇄) that exchanges width and height, turning a landscape canvas into the matching portrait one and back.
**Inputs / controls:** Tier dropdown. Custom: integers from 16 to 8192 px. Odd values are rounded up to even, with an inline note. The link toggle is on by default. **Swap** is one settings change (1920×1080 ↔ 1080×1920) and follows PM-011 when clips exist. (Some reference-app contexts require multiples of 4; FreeCut only requires even values, which every 4:2:0 encoder accepts; a hardware encoder that refuses a size falls back to software under EX-034's rules.)
**Behavior / output:** Sets the canvas pixel size. The preview renders at the canvas size scaled by the preview quality setting (EX). Export defaults to the canvas size (EX). Changing only the resolution never moves content visually, because positions are canvas-normalized (TR).
**Edge cases:**
- **8192 px cap:** this is wgpu's default 2D texture limit and a common encoder limit. Larger values are rejected inline.
- **Very small canvases** (under 128 px on a side): allowed, for things like stickers or icons, with a warning.
- **Above 3840×2160:** a performance note suggests proxies (PM-080).
- **Ratio that isn't a preset:** labeled "Custom (1.43:1)", using a reduced fraction or a two-decimal ratio.
**Undo:** `SetProjectSettings`.
**Model notes:** `CanvasSize::new(w, h) -> Result<CanvasSize, SettingsError>` enforces even values and the range. It uses integers only.

### PM-005 Sequence frame rate — mvp
**What the user sees:** A **Frame rate** dropdown: 23.976, 24, 25, 29.97, 30 (default), 50, 59.94 and 60 fps, each with a short hint (24 for a film look, 25/50 for regions with 50 Hz TV, 30/60 for phones and the web). In v1 the list adds 48, 119.88 and 120, plus Custom.
**Inputs / controls:** The dropdown. Labels show whole rates without decimals ("30 fps") and the 1001-denominator rates with their usual decimals ("23.976 fps", "29.97 fps", "59.94 fps"). (The reference app shows every rate with two decimals, for example "30.00", and its observed list has no 23.976; FreeCut keeps 23.976 because many cameras record it.) Custom (v1) takes a decimal or `num/den`. "23.976", "29.97" and "59.94" are recognized as their exact 1001-denominator rationals. The custom range is 1–240 fps, and the rate must also pass TL-001's validator (its frame duration must be a whole number of ticks). A rate that fails, for example 11 fps, is refused inline with the nearest valid rates suggested.
**Behavior / output:**
- The rate is stored as an exact TL `FrameRate` (24000/1001, 24/1, 25/1, 30000/1001, 30/1, 50/1, 60000/1001, 60/1).
- It sets the timeline frame grid (TL-001), the preview cadence and the export default (EX).
- Media at other rates is accepted unchanged. Each timeline frame shows the source frame at or before that instant. TL-001 defines this, and `freecut-media`/`freecut-render` implement it.
**Edge cases:**
- **29.97 vs 30 confusion:** 30000/1001 footage in a 30/1 project drops about one frame every 33 s. The media info panel (PM-059) flags "differs slightly from project rate".
- **Drop-frame timecode:** a display choice made by TL/UI. PM stores only the rate.
- **Changing the rate after editing:** follows PM-011, which calls TL-001's conform step.
**Undo:** `SetProjectSettings`.
**Model notes:** Uses `freecut-timeline::time::FrameRate`. The preset list is a const table in `freecut-timeline::settings`. No floats are used.

### PM-006 Match project settings to the first clip — v1
**What the user sees:** From v1, new projects start in **Auto** mode. (In the mvp, new projects start in Manual mode at 16:9 · 1920×1080 · 30 fps, and the user picks another shape with the quick-start buttons or the Ratio dropdown.) When the first video or still image lands on the timeline, the canvas and frame rate adopt that clip's properties. A toast says, for example, "Project set to 1080×1920 · 29.97 fps to match IMG_0412.MOV", with **Undo** and **Settings…** buttons.
**Inputs / controls:** "Auto (match first clip)" in the Ratio dropdown, available only while the timeline has no visual clips. A preference makes Auto the default for new projects (PM-020; on by default).
**Behavior / output:**
- **Trigger:** the first time a visual clip (video, still image or image sequence) is placed on a timeline that has no visual clips, while the mode is Auto. Importing into the media panel alone never triggers it, because a batch import has no clear "first". If one action places several clips, the clip that ends up earliest on the timeline wins (ties go to the lowest track).
- **Video source:**
  - The canvas becomes the display size, after rotation and pixel-aspect correction, rounded to even. If either side exceeds 8192, it is scaled down proportionally.
  - Frame rate:
    - Within 0.1 % of a preset: that preset.
    - Above 60 fps: the largest preset of 60 or less that divides it evenly (120 → 60, 100 → 50, 119.88 → 59.94, 240 → 60), otherwise 60.
    - Below 23.976 fps: the smallest preset that is an integer multiple of it (15 → 30, 12.5 → 25, 12 → 24, 10 → 30), otherwise 30.
    - Any other non-standard rate: the nearest preset by ratio.
    - VFR sources use their average rate.
- **Still image or image sequence:** the aspect ratio comes from the image after EXIF orientation. Within 0.5 % of a preset, that preset is used at the 1080 tier. Otherwise a custom size with a short side of 1080 is used (FreeCut default, which avoids 12-megapixel canvases from phone photos). The frame rate stays as it was (30 by default), except that an image sequence uses its own sequence rate, matched with the video rules.
- Once settings are applied, the mode becomes Manual.
**Edge cases:**
- **First clip is audio only:** nothing changes, and the project stays in Auto.
- **First clip still being probed:** placement waits for the probe (TL-012). If the probe fails, there is no placement and no match.
- **Source under 128 px on a side** (for example a 64×64 GIF): the preset for its ratio at the 1080 tier is used instead of a tiny canvas.
- **Non-square pixels** (1440×1080 stored with 4:3 pixel aspect): 1920×1080.
- **Rotated phone video** (stored as 1920×1080 with a 90° display rotation): 1080×1920.
- **Odd phone screen recording** (1170×2532): custom 1170×2532. It isn't snapped to 9:16, and the user can change it.
- **Undo after the match:** one undo reverts both the clip placement and the settings, and the mode returns to Auto. Redo reapplies the stored settings; they are not recomputed.
- **The matched clip is later deleted:** the settings and Manual mode stay.
**Undo:** A compound change set: TL's placement op plus `SetProjectSettings { before (Auto), after (Manual) }`, labeled "Add clip".
**Model notes:** `ProjectSettings.mode: SettingsMode { Auto, Manual }` (always `Manual` in mvp files). The pure function `freecut-timeline::settings::match_to(&MediaInfo) -> ProjectSettings` has table tests covering every rule above. Rate and ratio tolerances are compared as rationals (integer cross-multiplication), never as floats. TL's placement planner calls it when `mode == Auto` and the sequence has no visual clips.

### PM-007 Audio sample rate & channels — mvp
**What the user sees:** An **Audio** row in Project Settings: sample rate 48 kHz (default) or 44.1 kHz, and channels Stereo. In the mvp only 48 kHz stereo is offered. 44.1 kHz becomes selectable in v1.
**Inputs / controls:** A sample rate dropdown. The channel layout is fixed to stereo. Surround is out of scope here; see [audio.md](audio.md).
**Behavior / output:** Sets the mixer's output rate and layout ([audio.md](audio.md)) and the export default (EX). Sources at other rates or layouts are resampled and mixed by `freecut-audio`, with no pitch change.
**Edge cases:**
- **44.1 kHz music in a 48 kHz project:** resampled, no audible change.
- **Mono or 5.1 sources:** mapped and down-mixed by AU rules.
- **Changing the rate after editing:** no clip moves. Sample positions remain exact because TL ticks divide all common rates evenly (TL-001).
**Undo:** `SetProjectSettings`.
**Model notes:** `AudioFormat { sample_rate: u32 /* 44_100 | 48_000 */, layout: ChannelLayout::Stereo }` in `ProjectSettings`.

### PM-008 Default fit for mismatched media — v1
**What the user sees:** In the mvp every new clip is placed with Fit and there is no option. From v1, a Project Settings option, "When media doesn't match the canvas", offers three choices:
- **Fit** (default): the whole frame is visible, and empty areas show the background.
- **Fill**: the frame covers the canvas, and the overflow is cropped.
- **Original size**: one source pixel equals one canvas pixel.
**Inputs / controls:** Three-way choice, plus an **Apply to all clips…** button that runs TR's bulk transform reset.
**Behavior / output:** This is only the initial framing for clips placed from now on. TR owns the transform and computes it from the source display size, canvas size and fit mode ([clip-transform-and-speed.md](clip-transform-and-speed.md)). Changing the option does not touch existing clips.
**Edge cases:**
- **Tiny image with Fit:** upscaled to fit, which may be soft. No warning.
- **Huge image with Original size:** mostly off-canvas. That is allowed.
- **Vertical clip in a landscape project with Fit:** pillarboxed. The bars show the project background (PM-009), or TR's per-clip canvas background if one is set.
**Undo:** `SetProjectSettings`. Applying to all clips is a TR compound op.
**Model notes:** `PlacementFit { Fit, Fill, Original }` in `ProjectSettings`. TR reads it at placement time.

### PM-009 Project background color — v1
**What the user sees:** The project background shows wherever no clip covers the canvas (empty timeline spans, letterbox bars, transparent areas). In the mvp it is always black. From v1, a **Background** color swatch in Project Settings changes it, black by default.
**Inputs / controls:** A color picker (sRGB, hex field, eyedropper). Alpha is not offered, because the canvas is opaque. Transparent export is EX's decision.
**Behavior / output:** The compositor clears each frame to this color before drawing layers (FX render order). TR's per-clip canvas background fill (color, blur or image) draws above it, within that clip's span.
**Edge cases:**
- **Export with alpha** (if EX offers it): the project background is ignored, and EX documents this.
- **Changing the color during playback:** applies on the next frame.
**Undo:** `SetProjectSettings`.
**Model notes:** `background: Rgb8` in `ProjectSettings`. This is a color, not a time value, so an integer representation is enough.

### PM-010 Still-image default duration — v1
**What the user sees:** New still-image clips are placed with the project's still duration, 5.0 s by default. In the mvp this value is fixed (users trim the clip to change it). From v1, a Project Settings field, "Still image duration", changes it, and PM-020 can store a different default for new projects. Image sequences and animated images use their own length instead. (The reference app defaults to 3 s; FreeCut keeps 5 s, which TL-011's default durations already use and which suits photo slideshows, and lets the user pick 3 s.)
**Inputs / controls:** Seconds with one decimal, from 0.1 to 60.0 s (FreeCut default range). It is rounded to whole frames at the sequence rate, with a minimum of 1 frame.
**Behavior / output:** TL's insert planner uses it as the duration for image clips (TL default durations; OV-011 refers to it). Existing clips don't change.
**Edge cases:**
- **Value shorter than one frame at the current rate:** stored as 1 frame.
- **Changed in the middle of a drag-drop:** the drop uses the value at the time the drop is committed.
**Undo:** `SetProjectSettings`.
**Model notes:** `still_duration: Tick` in `ProjectSettings`. It is validated to be at least one frame at commit time.

### PM-011 Change project settings after editing — mvp
**What the user sees:** Project Settings can be opened at any time from File > Project Settings…, from a gear next to the player, or by clicking the canvas label ("1080×1920 · 30 fps").

The dialog shows:
- Name, and location (read-only, with **Reveal**)
- Ratio, Resolution, Frame rate, Audio; and in v1 Background (PM-009), Default fit (PM-008) and Still duration (PM-010)
- In v1: proxy policy (PM-080), copying media into the project (PM-039), and **Match to selected clip…**, which fills the fields from a selected media item or clip using PM-006's rules
- The working color space, shown read-only as "SDR · Rec.709". This is the single working and output space; HDR working space and HDR output are out of scope (see Out of scope). HDR sources are converted at decode, and CL-002 shows its one-time notice on the clip, so FreeCut has no per-import "convert color space?" prompt and no "mixed color spaces" warning.
- A read-only **File** line: format version, the FreeCut version that last saved it, and the created and modified dates (for example "Format 3 · saved by FreeCut 1.4.0 · created 2026-10-11"). It helps when a project behaves differently on another computer (PM-021).
- Later, with PM-107: a scope choice, **This timeline** or **Project defaults**, when the project has more than one timeline

When clips already exist, the dialog summarizes the consequences before **Apply**. For example, a changed ratio reframes every clip but keeps positions relative to the canvas. A changed frame rate offers TL's **Conform to frame grid** (on by default), which moves each edit point to the nearest frame of the new rate.
**Inputs / controls:** All the fields above, plus **Apply** and **Cancel**.
**Behavior / output:**
- **Ratio or resolution:** clip transforms are canvas-normalized (TR), so content keeps its relative position and size. Clips at Fit or Fill are re-fitted to the new canvas (TR). Text lengths (size, stroke, box width and so on) are stored in TX's ref px and multiplied by `canvas_short_edge / 1080` at render time (TX-001, [text-and-captions.md](text-and-captions.md)), so text keeps its size relative to the canvas's short edge. For example, switching 16:9 1920×1080 to 9:16 1080×1920 leaves text pixel sizes unchanged, and switching 1080p to 2160p doubles them. After a ratio change, AI's auto reframe (AI-016, later) may offer to reframe all clips as a separate step.
- **Frame rate:** calls TL-001's conform op when the box is checked. Clips that would shrink to 0 frames are kept at 1 frame (TL rule). Keyframe times are conformed by TR's helper in the same op.
- **Sample rate:** no edit changes.
- **Name:** renames the project file the same way PM-018 does.
- All changes in one Apply form one undo step.
**Edge cases:**
- **Locked tracks:** they are conformed too, and the summary lists them. Locking protects against user edits, not against a project-wide grid change, and skipping them would desync them from the other tracks.
- **Export running:** the dialog is read-only until the export finishes (export works on its own snapshot anyway).
- **Nested sequences (compound clips):** they share the root project settings, so they are conformed in the same op (FreeCut default).
- **Settings changed while proxies are in use:** proxies are unaffected.
- **Very large timelines:** the conform change set is computed on a snapshot off the UI thread. If that takes longer than 200 ms, an "Applying settings…" overlay with progress blocks editing until it is applied as one history entry (TL-043's rule for background work that changes the document). Cancel discards the computed change, and nothing is applied.
**Undo:** `SetProjectSettings { before, after }`, plus TL's `ConformToGrid` change set when conforming, combined into one compound entry. Undo restores the exact pre-conform ticks.
**Model notes:** A compound op in `freecut-timeline`. Property tests check that apply followed by invert is the identity and that conform never creates gaps or overlaps (TL's guarantee).

### PM-012 Save, Save As, Save a Copy — mvp
**What the user sees:** File > Save (`Mod+S`), Save As… (`Mod+Shift+S`) and Save a Copy…. Next to the project name there is a status: "Saved", "Saving…", "Unsaved changes" (only when autosave is off), or "Save failed — Retry".
**Inputs / controls:** Native save dialogs, using the `rfd` crate (MIT).
**Behavior / output:**
- **Save:** an immutable snapshot of the model is taken on the UI thread. This must be cheap, using structurally shared data. Serialization and I/O run on a background thread. The file is written as `<file>.tmp` in the same folder, flushed and fsynced, then atomically renamed over the target.
- **Save As:** writes to a new path and from then on edits that file. Relative media paths are recomputed for the new location (PM-100). The lock moves to the new file, and the old file stays as it was at its last save.
- **Save a Copy:** writes a copy with a new project id. Editing continues in the current file.
- Saving never clears the undo history (TL-043), and it moves TL-043's saved position to the current history entry, which clears the dirty flag.
- From v1, every save also refreshes the cover image in app data (PM-019).
- PM keeps the time of the last successful save. UI-012 shows it in the status tooltip ("Saved at 14:32"), and from v1 PM-014's status uses it.
- After **Save As** and **Save a Copy**, a success toast names the destination folder, with **Show in folder**.
- **Save failure reasons.** Each one has its own message and suggestion, shown in the error toast and in UI-012's failed-save details (FreeCut wording):

| Reason | Message and suggestion |
|---|---|
| No write permission | "FreeCut isn't allowed to write to this folder." Grant access in the system settings, or **Save As…** somewhere else. |
| Read-only file, folder or volume | "This location is read-only." **Save As…** somewhere else. |
| Not enough space | "Not enough free space on '<volume>'." Free some space (from v1, PM-113 says how much) or **Save As…** on another drive. |
| Path too long | "The file path is too long for this system." Use a shorter project name or a folder closer to the drive root. |
| Invalid path or name | "This location can't hold a project file." Pick another folder. |
| Volume disconnected | "The drive '<volume>' isn't connected." Reconnect it and **Retry**, or **Save As…** elsewhere. |
| File in use | "Another program is using the project file." **Retry** in a moment; FreeCut also retries 3 times over 2 s before reporting it. |
| Other I/O error | "Couldn't save (<system error>)." **Retry**, or **Save As…** elsewhere. |

**Edge cases:**
- **Disk full or permission denied:** an error toast states the reason from the table. The target file stays untouched, the temp file is removed, the project stays dirty, and from v1 autosave retries (PM-014).
- **Save As or Save a Copy onto an existing file:** the native dialog asks to replace it. Replacing a `.freecut` file that is open in another instance is refused because of its lock (PM-013).
- **File changed on disk by something else since the last load or save** (detected by size and mtime): the user is asked whether to Overwrite, Save As, or Cancel.
- **Network share where rename isn't atomic:** FreeCut copies the old file to `<file>.prev` first, then replaces it.
- **Cloud-synced folders:** written like any other folder. Sync conflicts are the sync tool's business, and the lock file (PM-013) makes accidental double editing less likely.
- **Saving while imports are pending:** pending items aren't in the model yet, so they're saved with the next save after they commit.
- **Saving while exporting:** allowed.
**Undo:** n/a (file operation).
**Model notes:** `freecut-project::save(&ProjectSnapshot, &Path) -> Result<(), SaveError>`. `SaveError` uses thiserror with variants `PermissionDenied`, `ReadOnly`, `NoSpace { volume }`, `PathTooLong`, `InvalidPath`, `VolumeUnavailable { volume }`, `InUse`, `Io`, `Serialize`, `ChangedOnDisk`. The mapping from `io::Error` kinds and platform error codes to these variants is a table-tested function; the wording lives in `freecut-app`'s string catalog (UI-044). The save coordinator in `freecut-app` serializes writes so at most one is in flight and at most one is queued.

### PM-013 Open project & single-writer lock — mvp
**What the user sees:**
- Ways to open: File > Open… (`Mod+O`, filtered to `*.freecut`), the start-screen cards, dropping a project file on the window, and in v1 the OS file association (double-click in the file manager).
- A short "Opening…" overlay shows. The editor appears once the model is loaded, and media checks continue in the background (PM-090).
**Inputs / controls:** File dialog. When a lock conflict is detected, a dialog offers **Open read-only**, **Open anyway**, or **Cancel**.
**Behavior / output:**
1. Parse the file.
2. Migrate it if it's an older format (PM-021).
3. Validate it: every clip's `MediaRef` resolves, values are in range, and there are no duplicate ids.
4. Build the model and open the editor.
5. In the background: resolve paths (PM-100), check files (PM-090, PM-093), and queue thumbnails and peaks (PM-073).

A lock file `<file>.lock` records `{ pid, hostname, user, app_version, started, refreshed }` and is refreshed every 60 s.
**Edge cases:**
- **Fresh lock (refreshed within 2 min) from another live process or another host:** the dialog says the project seems to be open elsewhere and names the host and time.
- **Stale lock** (process gone, or no refresh for over 2 min): treated as a crash. From v1, recovery is offered first (PM-015). Then the lock is taken over.
- **Read-only file or folder:** opens read-only with a banner. Save As works.
- **Validation problems** (for example a clip pointing to a missing media id, or a negative duration): FreeCut repairs what is safe to repair (drops unresolvable clips, clamps values) and shows a report. The repaired project opens with unsaved changes (TL-043's saved position is cleared) and an empty history, so the repair itself can't be undone; the original file is kept once as `<name>.freecut.bak-YYYYMMDD` before the first save instead.
- **Sequence frame rate whose frame duration isn't a whole number of ticks** (hand-edited or corrupt file): refused with a clear error, never guessed or repaired (TL-001).
- **JSON syntax error:** "This project file is damaged", with **Show in folder**, and from v1 **Open latest backup** (PM-016).
- **Opening the project that is already open:** the window simply gets focus.
- **Passing failures** (the file is briefly held by a sync tool, a backup program or a virus scanner): retried 3 times over 2 s. If it still fails: "The project file is busy right now. Try again in a moment", with **Retry**.
- **Project file or folder missing** (deleted, renamed, or on a drive that isn't connected): "Can't find this project", naming the last known path and, if known, the volume to connect. **Locate…** (v1) picks the file and updates the registry.
- **Project inside the OS Trash or FreeCut's Deleted projects hold (PM-104):** opens read-only, with a banner offering **Restore** first.
- **Project inside an archive** (a `.zip` that the file manager shows like a folder, or a temporary extraction folder the OS deletes later): refused with "Extract the archive to a normal folder first", because saves and relative media paths wouldn't survive.
- **Project in a location FreeCut can't use** (a read-only optical disc, a path the OS refuses to open): opens read-only when it can be read at all, otherwise "This location can't be opened". Save As works in both cases.
**Undo:** n/a.
**Model notes:** `freecut-project::open(&Path) -> Result<Opened { model, warnings: Vec<OpenWarning> }, OpenError>`. `freecut-project::lock::{acquire, refresh, release}`.

### PM-014 Autosave — v1
**What the user sees:** A preference, **Autosave**: On (default) or Off. With autosave on, the project is effectively always saved, which is what people used to mainstream consumer editors expect. The status next to the name shows "Saved" or "Saving…", and its tooltip gives the time of the last save ("Saved automatically at 14:32"), including before the user has ever pressed Save. (The reference app shows both an autosave label and, in some flows, an "edits not saved" prompt on close; FreeCut shows UI-025's prompt only when autosave is off or the last save failed.)
**Inputs / controls:**
- On/Off.
- Debounce: 2 s after the last edit (FreeCut default).
- Maximum interval during continuous editing: 30 s. Adjustable from 10 s to 10 min.
**Behavior / output:**
- While the project is dirty, it is saved through PM-012's path 2 s after edits stop, and at least every 30 s while edits continue.
- It is also saved when the window loses focus, before an export starts, on close and on quit.
- Only committed operations are saved. A drag that's still in progress is not.
- With autosave off, the same schedule writes recovery snapshots only (PM-015), and closing asks Save / Don't Save / Cancel.
**Edge cases:**
- **A save is already running when another is requested:** the requests coalesce into one pending save.
- **Repeated failures:** retried with exponential backoff from 2 s up to 5 min, with a persistent banner. Recovery snapshots keep being written if their location works.
- **Slow saves:** if a save takes more than 1 s, the minimum interval becomes 10× the last save duration, so very large projects on slow drives don't stall.
- **Read-only project:** autosave is disabled, with a banner.
- **Closing the project while an export runs or after one failed:** the project is saved first, so nothing is lost by leaving. Whether a running export continues in the background is EX-038's rule; a failed export stays in the task list (PM-073) with its reason and **Retry**, reachable from the start screen too.
- **Low disk space:** saves keep running, because project files are small (PM-113). A save that fails for lack of space shows PM-012's "Not enough free space" reason.
**Undo:** n/a.
**Model notes:** `AutosaveScheduler` is a pure state machine in `freecut-app`, tested with a fake clock. It uses PM-012's writer.

### PM-015 Crash recovery — v1
**What the user sees:** After FreeCut quits unexpectedly, the next launch asks whether to recover the named project's unsaved changes from a given time. The choices are **Recover**, **Open last saved**, or **Discard**.
**Inputs / controls:** The three buttons.
**Behavior / output:**
- While a project is open, a recovery snapshot `recovery/<project-id>.freecut` is written in app data on the autosave schedule whenever the project file itself is not being written (autosave off, or the last write failed). It is deleted on a clean close.
- On startup, a stale lock (PM-013) combined with a snapshot newer than the project file triggers the prompt.
- **Recover** loads the snapshot as the project, marked dirty. The next save writes the original path.
**Edge cases:**
- **The original project file has been moved or deleted since:** "Save recovered project as…".
- **The snapshot itself is corrupt:** FreeCut falls back to the newest backup (PM-016).
- **Discard:** moves the snapshot into the backups folder for 7 days, not to oblivion.
- **Undo history:** not persisted. It is empty after recovery, matching TL's rule that history lives in memory only.
**Undo:** n/a.
**Model notes:** The scheduling lives in `freecut-app`. The snapshot I/O lives in `freecut-project`.

### PM-016 Rolling backups & restore — v1
**What the user sees:** File > Restore from Backup… lists timestamped snapshots of the current project, with timeline duration and cover. **Open as copy** opens the selected snapshot.
**Inputs / controls:** The snapshot list and **Open as copy**. Preferences: number of snapshots to keep (default 20) and days of daily snapshots (default 14).
**Behavior / output:**
- After each successful save, if the newest backup is more than 5 minutes old, the saved file is copied to `backups/<project-id>/<UTC timestamp>.freecut` in app data.
- **Retention:** the newest 20 snapshots, plus the newest snapshot of each day for 14 days (FreeCut defaults).
- Opening a backup creates "<Name> (restored 2026-10-11 14.32).freecut" next to the original, with a new id. The original is never overwritten.
**Edge cases:**
- **Size cap:** total backup size is capped at 500 MB (FreeCut default). The oldest snapshots are pruned first, but the newest 3 per project are always kept.
- **Project moved to the trash:** its backups are kept for 30 days (PM-104's retention, whichever is longer), so a restored project still has its history of snapshots.
- **Backups in an older format:** migrated when opened (PM-021).
- **Disk full:** the backup is skipped with a warning, and the save still succeeds.
**Undo:** n/a.
**Model notes:** `freecut-project::backup::{snapshot, list, prune(&Policy, now)}`. `prune` is pure and has tests.

### PM-017 Recent projects — mvp
**What the user sees:** File > Open Recent lists the last 10 projects (FreeCut default) and has a **Clear list** item. The same registry feeds the start-screen cards (PM-001).
**Inputs / controls:** Menu items. In v1, **Locate…** is added for entries whose file is missing.
**Behavior / output:** The registry is a JSON file in app data. Each entry holds `{ project_id, path, name, last_opened, last_known_modified, canvas, frame_rate, duration }`. It is updated on open and save and written atomically. The path most recently opened goes to the top.
**Edge cases:**
- **Registry corrupt:** it is rebuilt from a default-folder scan, and the corrupt file is kept as `.bak`.
- **Size:** at most 200 entries; the oldest are dropped.
- **Missing files:** shown as "Not found" on the start screen, and greyed out with **Locate…** in the menu (v1).
**Undo:** n/a.
**Model notes:** The registry lives in `freecut-app`.

### PM-018 Project management actions — v1
**What the user sees:** On start-screen cards, and in the File menu for the current project: **Rename**, **Duplicate**, **Reveal in file manager**, **Remove from list**, **Move to Trash**. Cards support multi-select (`Mod`/Shift-click) for Move to Trash and Remove from list.
**Inputs / controls:** Context menu, inline rename, and a confirmation dialog for Move to Trash. The dialog names the project ("Move 'My Trip' to Deleted projects?") or, for a multi-selection, the count ("Move 4 projects to Deleted projects?"), and says they can be restored for 30 days.
**Behavior / output:**
- **Rename:** changes the display name and renames the `.freecut` file and its lock. Media folders keep their names. Relative paths still resolve because they sit beside the file. Names follow PM-002's rules: whitespace at either end is trimmed, an empty name is refused inline ("Enter a name"), and characters that files can't hold change only the file name, never the display name. (The reference app refuses names that start or end with a space or a period, and the `<` character; FreeCut trims spaces and maps the rest only in the file name.)
- **Duplicate:** copies the project file with a new id as "<Name> copy" (then "copy 2", and so on). Media is shared, not copied, and backups aren't copied. Copying a whole project into another existing one is TL-056's **Insert project…** (later, as a compound clip) or PM-107's **Import timeline from another project…** (later, as a separate timeline).
- **Move to Trash:** from v1 the project file goes to FreeCut's **Deleted projects** hold (PM-104) for 30 days, and only then to the OS Trash or Recycle Bin, using the `trash` crate (MIT). FreeCut never deletes it permanently. (Reference-app dialogs disagree on whether deletion is permanent or goes to a 30-day bin; FreeCut always uses the reversible path and words every dialog that way.) A checkbox, "Also move '<Name> Media' to Trash", is off by default and disabled if that folder doesn't exist; when ticked, that folder goes straight to the OS Trash (PM-104 explains why). The entry leaves the registry, and its cache entries age out through LRU (PM-074).
- **Remove from list:** offered only for projects outside the default folder, because projects inside it would reappear on the next scan.
**Edge cases:**
- **Renaming to an existing name:** inline error ("A project with this name is already in this folder"). Names that differ only in case count as the same on case-insensitive volumes.
- **Display name saved but the file rename fails** (the file is held by another program, or the folder became read-only): the new display name is kept in the project and the file keeps its old name. A toast says "Renamed, but the file couldn't be renamed (<reason>)", with **Retry**. The card shows the new name, and its tooltip shows the actual file name until a retry succeeds.
- **Duplicate fails** (disk full, no permission, the source vanished): the error gives PM-012's reason, and any partial copy that FreeCut created is removed.
- **Renaming or trashing a project open in another instance:** refused, because of the lock.
- **Trashing the currently open project:** it is saved and closed first.
- **No OS trash available** (for example, some minimal Linux setups): FreeCut explains that it can't move the file safely and suggests using the file manager. There is no silent permanent delete.
**Undo:** n/a. These are file operations. Deleted projects come back with PM-104's **Restore**, and media folders sent to the OS Trash come back through the OS.
**Model notes:** File operations live in `freecut-project`. The name sanitizer is shared with PM-002. Move to Trash calls PM-104's `trash::hold`.

### PM-019 Project cover image — v1
**What the user sees:** Each project card shows a cover. By default it is the first visual frame of the timeline. **Set current frame as cover** (player context menu or File menu) pins the frame under the playhead instead. **Reset cover** goes back to automatic.
**Inputs / controls:** The two commands.
**Behavior / output:** On save, the cover frame is rendered at 320 px on its long side by the compositor on a background thread, so overlays and text are included, and stored in app data as `covers/<project-id>.webp`. If the background render fails or is still running, the cover falls back to the media thumbnail (PM-070) of the earliest visual clip.
**Edge cases:**
- **Empty timeline:** a placeholder tile at the canvas ratio.
- **Pinned frame later beyond the timeline end:** falls back to automatic.
- **Offline media at the cover frame:** the offline slate (EX-005) is shown, which is acceptable.
- **Removing a cover:** **Reset cover** is the way back to automatic, which covers the reference app's "remove cover" and "pick again". A cover from an image file belongs to the exported video (EX-046), not to the project card.
- **Several timelines (PM-107, later):** the cover always comes from the primary timeline. Changing the primary timeline resets a pinned cover to automatic.
**Undo:** `SetCoverFrame { before, after }`. It is stored in the project (TL notes that PM owns the cover choice).
**Model notes:** `cover: Option<Tick>` in the project document. The renderer call goes through `freecut-render`.

### PM-020 New-project defaults & settings presets — v1
**What the user sees:** Preferences > New projects holds the defaults for: Auto/Manual mode, ratio, resolution tier, frame rate, audio, background, default fit, still duration, location, copying media into the project (PM-039) and proxy policy (PM-080). The Project Settings dialog adds a preset dropdown, **Save as preset…**, and **Use as default for new projects**.
**Inputs / controls:** Preset name (1–60 characters). The preset list can be renamed and deleted.
**Behavior / output:**
- Presets are stored in app data (JSON).
- FreeCut ships its own generically named presets:
  - "Vertical short video · 1080×1920 · 30 fps"
  - "Widescreen HD · 1920×1080 · 30 fps"
  - "Square · 1080×1080 · 30 fps"
  - "Portrait feed · 1080×1350 · 30 fps"
  - "Cinematic wide · 2560×1080 · 24 fps"
  - "4K UHD · 3840×2160 · 30 fps"
**Edge cases:**
- **Invalid values in a hand-edited config:** that preset is ignored, with a log warning.
- **Duplicate preset names:** a suffix is added.
- **A preset applied to a project with clips:** behaves exactly like PM-011.
**Undo:** Applying a preset to the open project is `SetProjectSettings`. Managing presets is n/a (app configuration).
**Model notes:** App configuration lives in `freecut-app`. The values reuse `ProjectSettings` validation from `freecut-timeline`.

### PM-021 Project file format, versioning & migrations — mvp
**What the user sees:** Usually nothing. The format version is shown read-only in Project Settings (PM-011), and otherwise only becomes visible in messages:
- For a file from a newer FreeCut: "This project was saved by FreeCut 2.3.0 (format 7). This version reads up to format 6, so it opens read-only. Update FreeCut to edit it." The version comes from the file's `app_version`.
- After a migration: "Project upgraded from an older format. The original was kept as …"
- A start-screen scan that finds newer-format projects badges each card "Newer version" (PM-001) instead of refusing the whole folder.

(The reference app either blocks a newer project until the app is updated, or lets the user open it after warning that some parts may not load. FreeCut opens it read-only, so nothing can be silently dropped. Its older-format upgrade needs the internet; FreeCut's migrations are always offline.)
**Inputs / controls:** None.
**Behavior / output:**
- **Structure:** one UTF-8 JSON object, `{ "format": "freecut-project", "format_version": N, "app_version": "x.y.z", "header": {…}, "settings": {…}, "library": {…}, "assets": {…}, "sequences": {…}, "cover": … }`. The `header` holds id, name, created and modified timestamps (RFC 3339, UTC), duration (ticks), canvas and frame rate, duplicated for fast listing (PM-001).
- **Values:** time values are TL ticks (integers). The document stores no floats for time.
- **Versioning:** `format_version` is an integer starting at 1 and is bumped for every schema change. Each bump ships a pure migration `migrate_vN(serde_json::Value) -> Result<Value, MigrationError>`. Migrations are chained on load, and each one has fixture tests (old file in, expected new file out).
- **After migration:** the first save keeps the pre-migration file once as `<name>.freecut.v<N>.bak`. FreeCut default: it is named from the version stored in the file being replaced, so it never shares a slot with a repair copy, and an existing intact copy is never overwritten. A save never writes a file that would not open again (it is decoded before it is written), so an unknown key kept from another variant of an object (PM-021) is dropped when it is named like a field the object now writes.
- **Newer files:** a file with a higher `format_version` opens read-only. Save and Save a Copy are disabled, because unknown data would be dropped.
- **Deterministic output:** stable key order, 2-space indentation and a trailing newline, so projects diff cleanly in version control.
- **What isn't in the file:** no binary data (thumbnails, peaks and proxies live in the cache). View state (zoom, scroll, media panel sort and filters, selected bin, source in/out marks) is kept in app data under `view/<project-id>.json`, so browsing never dirties the project.
**Edge cases:**
- **UTF-8 BOM:** accepted.
- **Invalid UTF-8:** error.
- **Tick values that would overflow `i64` arithmetic:** a validation error, using checked math (TL).
- **Unknown fields at the current version** (for example, from a development build or a fork): kept in memory as an opaque map and written back, with a warning in the log. Data FreeCut doesn't understand is never dropped on save.
- **Migration never changes what the user sees:** a migrated project renders the same frames and sounds as before. Where a migration touches values that affect rendering, a workspace-level integration test compares rendered frames before and after it.
- **Opening a migrated project without saving:** the file on disk stays in the old format until the first save, so another computer with the older FreeCut can still open it until then.
- **Paths that aren't valid UTF-8 on Linux:** stored as `{ "unix_bytes": "<base64>" }`.
- **Windows extended-length and UNC paths:** preserved.
**Undo:** n/a.
**Model notes:**
- `freecut-project` holds the serde structs for the current version only, the migration chain over `serde_json::Value`, `CURRENT_FORMAT_VERSION`, and thiserror error types.
- The in-memory model types live in `freecut-timeline`, which has no I/O. The project crate maps between the two.

### PM-030 Import media via dialog — mvp
**What the user sees:** An **Import** button at the top of the Media panel, plus File > Import Media… (`Mod+I`). Both open a native multi-select file dialog filtered to supported types, with an "All files" option. Picked files appear at once as pending tiles with a spinner. Each becomes a normal tile when its probe finishes. In v1, items land in the selected bin (PM-052).

From v1 the Import button has a small menu arrow next to it, gathering every way to add media in one place: **Files…** (this feature), **Folder…** (PM-032), **Image sequence…** (PM-033), **Color clip…** (PM-108); later **Record camera…** (PM-043) and **Record screen…** (PM-115). Entries for features that aren't in the build are hidden.
**Inputs / controls:** The native dialog, with `Mod`/Shift multi-select. The last-used import folder is remembered.
**Behavior / output:**
1. Each path is canonicalized.
2. A path that's already in the library is handled by PM-038. In the mvp, the existing tile is flashed instead of adding a duplicate.
3. Each new path joins the app's **import queue** as a pending item, and a background probe runs (PM-035).
4. Successful probes become `MediaItem`s.
5. When the whole batch finishes, or is cancelled, one `ImportMedia` op commits every successful item in natural name order.
6. Failures are reported (PM-037).
7. Thumbnail, filmstrip and peak jobs are queued (PM-073).
**Edge cases:**
- **Large batches (500+ files):** pending tiles are virtualized. Probes run at max(2, cores/2) at a time. It is still one op per batch.
- **File deleted or moved between picking and probing:** reported as "File not found".
- **File still being written** (a recording or copy in progress): probed as-is. If its size changes afterwards, PM-093 marks it Changed.
- **Cancelling:** done from the jobs popover. Items already probed are committed, and the rest are dropped.
- **Undo while a batch is pending:** undo affects only committed ops. The batch commits later as a new op on top.
- **Slow network drive:** a probe timeout of 30 s gives "Timed out reading file", with Retry.
- **Unicode, emoji or very long paths** (over 260 characters on Windows): supported through verbatim paths.
- **OS-protected locations** (for example, macOS privacy-protected folders): a permission error explains how to grant access.
- **macOS removable and network volumes:** the first read from a card, an external drive or a network share may trigger the system's own access prompt. If access is refused, those files fail with the Permission reason (PM-037), and the report names the volume, explains where to allow FreeCut in System Settings and offers **Open System Settings**. **Retry** works once access is granted, without restarting.
- **Online-only cloud placeholders:** may time out. The hint suggests making the file available offline. PM-114 also flags them.
- **Very large files:** no per-file size limit (the reference app refuses local files over 4 GB). Files over 4 GB are normal for camera footage, and FreeCut only reads them.
- **Media opened from the OS (v1, with UI-053):** choosing **Open with FreeCut** on media files in the file manager, or passing media paths on the command line, opens the start screen's **New project** with those files queued for import. If a project is already open, they are imported into it instead.
- **Media pasted from another open project** (TL clipboard): added through this same op, reusing the source project's probe data with no re-probe.
**Undo:** `ImportMedia { items }`. Its inverse removes those items, which can't be on the timeline at that point. Redo re-adds the same ids with the cached probe data.
**Model notes:**
- `freecut-media::probe(&Path) -> Result<MediaInfo, ProbeError>`.
- `freecut-timeline::library::{MediaItem, MediaLibrary, ImportMedia}`.
- `ImportQueue` lives in `freecut-app`.
- Pending items are app state, never document state.
- mvp queue (FreeCut defaults): a finished batch waits while a slider or field drag is in progress and commits once it ends, so neither the import nor the drag's single undo entry is lost. Opening or creating another project drops the old project's pending probes and files; they never land in the new project.

### PM-031 Drag-and-drop import — mvp
**What the user sees:** Files dragged from the OS file manager can be dropped in several places:
- **Media panel:** imports them. The panel shows a "Drop to import" highlight.
- **Timeline:** imports them and places them at the drop point. A ghost clip shows while probing; TL handles placement (TL-012).
- **Player:** imports into the panel, the same as a panel drop.
- **A `.freecut` file dropped anywhere except the timeline:** opens that project, after saving or closing the current one. Dropped on the timeline, it is inserted as a compound clip once TL-056 (later) ships; until then a drop on the timeline opens it too.
**Inputs / controls:** Drag and drop, using egui's hovered and dropped files.
**Behavior / output:**
- Dropping uses the same pipeline as PM-030.
- Multiple files are ordered by natural name.
- A drop on the timeline is one compound undo step: import plus placement.
- Folders dropped in the mvp show a toast saying folder import arrives in v1 (PM-032), and their loose files are ignored.
**Edge cases:**
- **Mixed supported and unsupported files:** the supported ones are imported, and the report lists the rest.
- **Drop on a locked track:** redirected to a new track (TL rule). The import still happens.
- **Probe failure during a timeline drop:** the drop is cancelled with a toast, and nothing changes (TL-012).
- **URLs dragged from a browser:** ignored, with "Only files on this computer can be imported". FreeCut never downloads.
- **Drop on an area that doesn't accept files** (the inspector, the effects and text libraries, a dialog): the pointer shows the no-drop cursor, and releasing there shows "Drop media on the media panel, the player or the timeline" (UI-036's convention). Nothing is imported.
- **Promised files** (macOS apps that hand over files lazily, such as photo libraries): may not arrive through the windowing layer. FreeCut shows a hint to export the files first (open question).
- **Linux Wayland sessions where drag-and-drop doesn't work:** the Import button is the fallback (open question).
**Undo:** Panel drop: `ImportMedia`. Timeline drop: a compound of `ImportMedia` and TL's placement, labeled "Add media".
**Model notes:** Handled in `freecut-app` through the shared `ImportQueue`.

### PM-032 Folder import — v1
**What the user sees:** File > Import Folder…, or dropping folders. Options: **Create bins from folders** (on), **Include subfolders** (on), **Detect image sequences** (on). For more than 1,000 files, FreeCut asks for confirmation, showing the count.
**Inputs / controls:** Folder picker and the three checkboxes.
**Behavior / output:**
- The tree is walked on a background thread with the `walkdir` crate (MIT/Unlicense). Symlinks are followed with a visited set, so loops end.
- Supported files are collected by extension and then probed. Content probing is the final judge (PM-036).
- Hidden and system entries are skipped: dotfiles, `Thumbs.db`, `.DS_Store`, `__MACOSX`, `$RECYCLE.BIN`.
- The folder hierarchy is mirrored as bins, but only for folders that contain supported media.
**Edge cases:**
- **Empty folder, or no supported files:** "No supported media found in this folder".
- **Unreadable subfolders:** skipped and listed in the report.
- **Camera card layouts:** the media files inside are imported (for example `DCIM/…`, `PRIVATE/M4ROOT/CLIP/…`, `AVCHD/BDMV/STREAM/…`). Sidecar metadata, thumbnails and low-resolution preview files are skipped. Spanned recordings (one take split across files) arrive as separate items.
- **Huge network folders:** progress and Cancel. Items already probed are committed.
**Undo:** One compound op: `CreateBin` for each mirrored folder, plus `ImportMedia`.
**Model notes:** The walker lives in `freecut-app`. Probing lives in `freecut-media`. Sequence detection comes from PM-033.

### PM-033 Image sequence import — v1
**What the user sees:** When imported stills are numbered frames (for example `render_0001.png … render_0240.png`), FreeCut asks whether to import them as one image-sequence item. A "Remember my choice" box is offered. File > Import Image Sequence… takes the first frame directly. The item shows a sequence badge and a frame count.
**Inputs / controls:** Sequence frame rate: defaults to the project rate, with the PM-005 presets available, and Custom in v1.
**Behavior / output:**
- **Detection:** same prefix, same suffix and the same extension, with a numeric run before the extension. The numbers may be zero-padded or variable-width. At least 3 frames are needed to prompt (FreeCut default).
- **Duration:** frame count ÷ rate.
- Frames are decoded individually by `freecut-media`. Alpha is preserved.
**Edge cases:**
- **Gaps in the numbering:** a warning names the missing range, and missing frames hold the previous frame. This rule is shared with OV-017.
- **Mixed frame sizes:** every frame is fitted to the first frame's size.
- **Mixed formats** (PNG and JPEG in one folder): treated as separate sequences.
- **Periods in names** (`shot.v2.0001.png`): the last numeric run before the extension is the frame number.
- **More than 10,000 frames:** indexed lazily.
- **EXR/DPX sequences:** later (they need CL's linear-light handling).
- **Changing the rate afterwards:** an interpretation override (PM-040).
**Undo:** `ImportMedia`. Rate changes use `SetInterpretation`.
**Model notes:** `MediaSource::ImageSequence { dir: StoredPath, prefix, suffix, digits, first, last, rate: FrameRate, missing: Vec<u32> }` versus `MediaSource::File(StoredPath)` (`StoredPath` is defined in PM-100). `detect_sequences(&[PathBuf]) -> Vec<SequenceCandidate>` is pure and has table tests.

### PM-034 Still & animated images — mvp
**What the user sees:** Photos and graphics import like video. Tiles show the image. A placed still lasts the project's still duration (PM-010). Animated GIF, APNG and animated WebP import as stills of their first frame in the mvp. From v1 they import as moving media with their own length, together with OV-012 (v1), and OV-012/OV-013 own their playback options.
**Inputs / controls:** None beyond the import controls.
**Behavior / output:**
- EXIF orientation is applied.
- **Large images:** an image is decoded once on a background thread, downscaled to the smaller of (2 × canvas long side) and 8192 px on its long side, and kept in the image cache, so the UI and preview never decode a 48-megapixel photo per frame. TR pan-and-zoom renders from this copy.
- **Alpha:** preserved for PNG (8- and 16-bit), WebP, TIFF and GIF.
- **ICC profiles:** in the mvp, images are treated as sRGB/Rec.709 (CL-002 documents the small shift for wide-gamut images).
- **Other conversions:** CMYK JPEGs are converted to RGB, and 16-bit images are reduced to 8-bit in the mvp.
- **Single-frame GIF:** a still.
**Edge cases:**
- **1×1 images:** allowed.
- **Zero-byte or truncated files:** an import error (PM-037).
- **Very wide banners (200:1):** allowed.
- **HEIC/HEIF and AVIF:** v1. They depend on the FFmpeg build (PM-036), never on codec packs installed in the OS. (The reference app sends Windows users to install the system's HEVC/HEIF extension; FreeCut's support is the same on every OS for a given build.)
- **Phone "live" and motion photos:** imported as plain stills here. Choosing the short video part instead is PM-109 (later).
- **SVG:** later. It would be rasterized with the `resvg` crate (MPL-2.0, GPL-compatible).
- **PSD:** out of scope.
**Undo:** `ImportMedia`.
**Model notes:** `MediaKind::{Video, Audio, Image, ImageSequence}`. `ImageInfo { width, height, orientation, has_alpha, animated: Option<AnimInfo> }`. Decoding stays in `freecut-media`.

### PM-035 Media probing & normalization — mvp
**What the user sees:** Correct orientation, duration and properties, without having to think about them. Pending tiles show a spinner until probing ends.
**Inputs / controls:** None. The overrides are in PM-040.
**Behavior / output:**
- **What the probe extracts:**
  - Container format, duration (from the container or streams; if missing, from a background full scan), start-time offset, start timecode, file size, mtime and fingerprint (PM-038).
  - **Video streams:** codec, profile, bit depth, pixel format, width and height, sample aspect ratio, display rotation and mirroring (display-matrix side data), nominal and average frame rate, VFR flag, field order, alpha presence (including VP8/VP9 alpha side data, so the alpha-capable decoder is chosen, as in OV-012), and color primaries, transfer, matrix and range (CL-002).
  - **Audio streams:** codec, sample rate, channel layout, duration and language.
  - **Subtitle streams:** listed for TX.
  - **Attached pictures (cover art):** used as tile art, never as video.
- **Normalization, applied in decode so every other crate sees display-ready frames:**
  - Rotation is applied, and non-square pixels are scaled to square.
  - Odd dimensions are supported.
  - VFR frames are delivered by timestamp.
  - The first-frame time offset is subtracted, so media time 0 is the first presentable frame. The audio/video offset is preserved, so sync holds.
- **VFR detection:** nominal and average rates disagree, or the spread of timestamp deltas over the first 300 packets is above 1 %.
- **Interlaced sources:** flagged with an "Interlaced" badge and shown as progressive in the mvp. v1 deinterlaces at decode with FFmpeg's `bwdif`.
**Edge cases:**
- **No video or audio stream:** unsupported.
- **Audio-only MP4/M4A:** an audio item.
- **MP3 with cover art:** an audio item that uses the art as its thumbnail.
- **Several video streams** (for example, stereo/spatial or 360° captures): the default stream is used, and the info panel lists the others.
- **Audio and video lengths differ** (10.0 s video, 10.2 s audio): media duration is the longer one, and the shorter stream is padded with black or silence.
- **MP4 edit lists and negative timestamps:** honored. Fixture-tested.
- **One-frame video:** duration of 1 frame, and usable.
- **Probe errors:** mapped to `ProbeError`. Jobs catch panics at the job boundary. Native crashes inside FFmpeg remain a risk (open question: probe in a subprocess).
**Undo:** n/a. Probe data is derived and travels inside the `MediaItem` created by the import op.
**Model notes:** `MediaInfo { container, duration: Tick, start_offset: Tick, video: Vec<VideoStreamInfo>, audio: Vec<AudioStreamInfo>, subtitles: Vec<SubtitleStreamInfo>, image: Option<ImageInfo>, timecode: Option<String> }` is plain data in `freecut-timeline::media`. `freecut-media` fills it, converting stream timebases to ticks exactly (TL-001). `VideoStreamInfo { codec, width, height, sar: (u32, u32), rotation: Rotation, mirrored: bool, rate_nominal: FrameRate, rate_avg: FrameRate, vfr: bool, field_order, has_alpha, bit_depth, color: ColorInfo }`.

### PM-036 Supported formats — mvp
**What the user sees:** The import dialog filter, the Help > Supported Formats page and import errors all come from one table. Support is decided by probing the content, not by the file extension, so a mislabeled but decodable file still works, and a renamed unsupported file still fails with a clear reason.
**Inputs / controls:** None.
**Behavior / output:**

| Kind | Containers / formats | Codecs |
|---|---|---|
| Video | MP4, MOV, M4V, MKV, WebM, AVI, MTS/M2TS/TS, MPG/MPEG, FLV, 3GP, OGV; MXF and WMV/ASF in v1 | H.264/AVC; H.265/HEVC (8/10-bit); VP8; VP9; AV1; MPEG-2; MPEG-4 Part 2; MJPEG; ProRes (all variants); DNxHD/DNxHR; FFV1; QuickTime Animation and PNG-in-MOV (alpha); CineForm in v1 |
| Audio | MP3, AAC/M4A, WAV, AIFF, FLAC, ALAC, Ogg Vorbis, Opus, AC-3/E-AC-3; WMA in v1 | PCM 16/24-bit and 32-bit float, plus the codecs implied by the containers |
| Image | JPEG, PNG, WebP, BMP, TIFF, GIF (animated → moving media in v1, PM-034), TGA; HEIC/HEIF and AVIF in v1 (build-dependent); SVG later; EXR/DPX later | — |

- The extension list exists only for the dialog filter.
- At startup, `freecut-media` checks the linked FFmpeg for each decoder, so the table reflects the actual build.
- Hardware decoding is an optional preview-performance setting owned by EX. Software decoding is always the fallback.
**Edge cases:**
- **Container supported but the codec isn't in this build:** "This file uses the <codec> codec, which this FreeCut build can't decode."
- **DRM-protected files** (store-bought video, protected audio): "This file is copy-protected and can't be imported." FreeCut never attempts to bypass protection.
- **Proprietary camera RAW video** (vendor SDK formats): unsupported, with an explicit message.
- **Playlists, shortcuts and archives** (`.m3u`, `.cda`, `.zip`): unsupported, with a hint where useful ("unzip first").
- **A `.freecut` file passed to Import Media:** "This is a FreeCut project. Open it instead?"
- **HEVC on Windows without the OS extension, and similar OS codec gaps:** irrelevant to FreeCut, which decodes with its own FFmpeg build. When the build lacks a decoder, the message names the codec as above; there is no "install a system codec" path.
- **Files that decode but scrub poorly** (camera transport streams, files without a seek index): supported as they are. PM-111 (later) can re-wrap them, and proxies (PM-080) help with heavy codecs. FreeCut never needs a mandatory "transcoding" step before a supported file can be used.
- **No size limit:** the table has no per-file cap (PM-030).
**Undo:** n/a.
**Model notes:** `freecut-media::formats::{SUPPORTED, dialog_filters(), runtime_capabilities()}`. Which decoders ship depends on our FFmpeg build and the licensing of each platform's binaries (open question).

### PM-037 Import failures & import report — mvp
**What the user sees:** A non-blocking toast, for example "Imported 18 of 20 files. 2 couldn't be imported", with a **Details** link. Details lists each file with a plain-language reason and a suggestion. **Copy report** and **Retry** (for transient errors) are available.

| Reason | Message |
|---|---|
| Not found | File not found |
| Permission | Permission denied (with how to grant access) |
| Empty | File is empty (0 bytes) |
| No streams | No video or audio inside |
| Unsupported codec | Unsupported codec X (convert it to MP4 with H.264 and AAC using any converter, then import again) |
| Corrupt | Damaged file |
| Incomplete | Recording looks unfinished (the file has no index) |
| Protected | Copy-protected |
| Timeout | Timed out reading the file |
| Decode error | Couldn't read the media inside (decoder error <code>). Details and **Copy report** include FFmpeg's error code and message, so the user can report it |
| Volume lost | The drive was disconnected during import |
| No space | Not enough free space to copy this file into the project (PM-039 only; linking needs no space) |

**Interrupted batches:** when a batch stops early (cancelled, the drive was unplugged, FreeCut was closed), the toast says how many items were already imported ("Import stopped. 12 of 40 files were imported"). Those 12 stay as one committed `ImportMedia` op, and **Retry** offers the rest.

**Inputs / controls:** Details, Copy report, Retry.
**Behavior / output:** Failures never create library items, which keeps the library clean. When many files fail, they are grouped by reason.
**Edge cases:**
- **Every file fails:** "Couldn't import any of the 3 files", with Details.
- **Very large projects:** there is no "project too complex to import into" failure. Libraries of 10,000 items are handled by the virtualized panel (PM-050) and the import queue (PM-030); only disk space (PM-113) and the files themselves can make an import fail.
- **Retry succeeds later:** those items commit as a new `ImportMedia` op.
**Undo:** n/a. Nothing is committed for failures.
**Model notes:** `ProbeError` (thiserror): `NotFound`, `PermissionDenied`, `Empty`, `NoStreams`, `UnsupportedCodec { codec }`, `Corrupt { detail }`, `Decode { code: i32, detail }`, `Incomplete`, `Protected`, `Timeout`, `VolumeLost`, `Io(io::Error)`. The copy step's `NoSpace` comes from PM-039's job, not from the probe. Messages are mapped to text in `freecut-app`.

### PM-038 Duplicate import detection — v1
**What the user sees:** Importing a file that is already in the project doesn't add it again. The existing tile flashes, and a toast says which bin it's in. If a file at a different path looks identical to an existing item, FreeCut asks whether to **Use existing** or **Import anyway**. A preference, **Allow duplicate media items**, is off by default.
**Inputs / controls:** The prompt and the preference.
**Behavior / output:**
- A file counts as a duplicate if it has the same canonical path, or the same **fingerprint** at another path.
- The fingerprint is: file size + an xxh3-64 hash of the first and last 1 MiB + the probed duration.
- **The only media content key.** This fingerprint is the one media content id in FreeCut. Every cache and every stored derived result keyed on media content starts from it and adds its own fields. No spec defines a second media hash or key. For example:
  - thumbnails: fingerprint + time + size + interpretation hash (PM-070)
  - waveform peaks: fingerprint + stream + format version (PM-072)
  - proxies: fingerprint + height (PM-080)
  - AI-001's analysis results: fingerprint + stream + source range + analysis kind + algorithm or model id and version + parameter hash
- **No media id and no mtime in content keys.** A copied, moved, relinked or duplicated file with the same content therefore reuses every stored result, in this project and in others. The `MediaId` identifies a library item in one project and is never part of a content key. Size and mtime are only cheap triggers that tell PM-093 when to recompute the fingerprint.
- **Staleness comes from PM-093.** When PM-093 reports an item as Changed and its recomputed fingerprint differs, results stored under the old fingerprint simply stop matching. Owners decide what to show (for example AI-001's "stale" state for document-held results). No other area runs its own change detection.
- The fingerprint is also used for relink verification (PM-091).
- The fingerprint is computed for every import from the mvp on, because mvp cache keys (PM-070, PM-072) and relink checks (PM-091) depend on it. Only the duplicate detection described here is v1.
**Edge cases:**
- **Same file through different case, a symlink or a hard link:** caught by canonicalization.
- **Two different files with the same fingerprint:** extremely unlikely. If it happens, the user's choice decides, and stream info is compared too.
- **Fingerprinting very large files on slow drives:** only 2 MiB is read, so it stays fast.
- **Importing a file that matches an Offline item** (same fingerprint, new location): instead of a second item, FreeCut asks "This file is the missing '<name>'. Relink it?" with **Relink** (default) or **Import as new item**. Relink goes through PM-091's op, so every clip keeps its edits and effects. (The reference app relinks a re-imported missing video but warns that the clip's effects may be lost; FreeCut's relink never touches clips.) This check runs from the mvp, because fingerprints and PM-091 are mvp; only the general duplicate prompt above is v1.
**Undo:** n/a when nothing is added. Otherwise `ImportMedia`. Choosing **Relink** is PM-091's `RelinkMedia`.
**Model notes:** `Fingerprint { size: u64, head_tail: u64, duration: Tick }` in `freecut-timeline::media` (plain data, so `freecut-analysis` and other crates can use it without I/O). It is computed in `freecut-media` with the `xxhash-rust` crate (BSL-1.0). Cache-key types elsewhere (for example AI-001's `AnalysisKey`) embed this type instead of their own hash.

### PM-039 Copy media into the project folder — v1
**What the user sees:** An option in Project Settings, and in the import dialog: **Copy imported files into the project's media folder**. It is off by default: media is linked in place, which avoids duplicates. When it's on, a "Copying…" progress appears, and items can be used straight away from their original location while they copy.
**Inputs / controls:** The checkbox.
**Behavior / output:**
- Files are copied to `<Name> Media/Imported/` under their own names; on a collision, " (2)" is added.
- Once every copy in the batch is verified by size and fingerprint, the items switch to their copies in one background-applied document change, as TL-043 requires for background work that changes the document. It appears in history as "Use copied media" and marks the project dirty.
- `MediaItem.storage` becomes `CopiedToProject`.
**Edge cases:**
- **Not enough space:** checked before starting (total size plus 1 GB margin). If short, the copy is refused and items stay linked.
- **Same volume:** still copied, never moved. FreeCut never moves the user's files.
- **Cancelling a 100 GB copy:** items stay linked, and partial copies are removed (FreeCut created them during this job).
- **Undoing the import:** the copied file stays on disk. The cleanup in PM-074 offers to trash unused copies.
- **Item removed, or its import undone, before the batch finishes:** that item is skipped by the switch, and its copy is left for PM-074's cleanup.
**Undo:** The import op is undoable as usual. The switch is `SetMediaStorage { items, before: (StoredPath, Linked), after: (StoredPath, CopiedToProject) }`; undoing it points the items back at the originals, which are still in place because FreeCut never moves the user's files. The file copying itself is n/a (file I/O).
**Model notes:** `MediaStorage { Linked, CopiedToProject }`. The copy job lives in `freecut-app`, and verification in `freecut-media`.

### PM-040 Interpret media (overrides) — v1
**What the user sees:** Media item menu > **Interpret…** opens a dialog with:
- **Frame rate:** for example, treat 120 fps slow-motion footage as 30 fps so every frame plays at quarter speed. The audio of retimed media is muted by default.
- **Field order:** progressive, top first, bottom first.
- **Pixel aspect ratio**
- **Rotation:** 0/90/180/270, and mirror.
- **Alpha:** straight, premultiplied, or ignore.
- **Color interpretation:** CL-002 owns the matrix and range override and its op.
- **Audio stream:** for files with several streams. AU defines "all streams mixed".
- **Image-sequence frame rate**

Overridden items show an "Interpreted" badge.
**Inputs / controls:** The fields above, each with **Auto (from file)** as its default.
**Behavior / output:**
- Overrides are stored on the `MediaItem`. Every clip using that item updates.
- If a frame-rate override changes the media duration while clips use the item, FreeCut asks:
  - **Keep clip lengths** (default): source ranges are rescaled to keep timeline durations.
  - **Keep source frames:** clip lengths change, and later clips on the same track aren't moved; overlaps are trimmed by TL's overwrite rule.
- Rotation and pixel-aspect overrides update the display size. TR re-fits Fit and Fill clips.
**Edge cases:**
- **Offline media:** overrides allowed.
- **Frame-rate override on audio-only media:** not offered.
- **Frame-rate override on VFR media:** conforms to constant rate by frame count.
- **Override rate that differs from the sequence rate:** allowed. A note under the field says the clip still plays on the sequence's frame grid, so frames are repeated or skipped (TL-001), and that motion may look less smooth.
- **Clip ranges that would exceed the new duration:** clamped and reported.
**Undo:** `SetInterpretation { media, before, after, clip_adjustments }`. The color part is CL's `SetColorInterpretation`.
**Model notes:** `Interpretation { frame_rate: Option<FrameRate>, field_order: Option<FieldOrder>, par: Option<(u32, u32)>, rotation: Option<Rotation>, mirrored: Option<bool>, alpha: Option<AlphaMode>, audio_stream: Option<u32>, color: Option<ColorOverride /* CL */> }` on `MediaItem`. Decode in `freecut-media` honors it.

### PM-041 Derived & generated media items — v1
**What the user sees:** Some media items are made by FreeCut, not imported. They appear in the media panel with a badge, and in v1 they are gathered in an automatic **Generated** bin. Examples:
- Extracted audio of a video (AU-004)
- Voiceover recordings (AU-027) and, later, camera and screen recordings (PM-043, PM-115)
- Text-to-speech audio (text-and-captions.md; engine AI-025)
- Separated stems: the user commands are AU-021's (the in-place **Voice isolation** dropdown and **Split into stems…**), and AI-023's engine writes the stems as lossless FLAC files into this store
- AI outputs that a clip points at as its media: upscaled or enhanced video (AI-019) and inpainted (object-removed) video (AI-022) ([ai-tools.md](ai-tools.md), which calls them "derived media")
- Frame snapshots added to the library (EX-014)
- Files produced by a local generator tool (AI-026)
- Results of cloud generation runs with the user's own account (CG-011, [cloud-generation.md](cloud-generation.md)), in `<Name> Media/Generated/Cloud/`

Images the user imports for other features, such as wipe patterns (FX-023), are ordinary imported media items, not generated ones. Copies of bundled samples are covered by PM-063.
**Inputs / controls:** None here. Each producing area has its own command.
**Behavior / output:**
- **Virtual items** point into another item without writing a file. For example, `MediaSource::AudioOf { source: MediaId, stream }` for extracted audio. They go offline and relink together with their source.
- **File-backed items** live in the **generated-media store**, `<Name> Media/Generated/`, or `<Name> Media/Recordings/` for recordings. Files are named descriptively (for example "IMG_0412 voice.wav"). The store is never evicted.
- **The storage rule (one rule for every spec):**
  - **Media cache (PM-074):** deterministic, regenerable outputs, which FreeCut can rebuild bit for bit from the source and the recorded settings. These include stabilization data (TR-026), optical-flow or AI-interpolated frames (TR-042, AI-018), cutout mattes (AI-014), analysis records (AI-001), processed or enhanced audio renders (AU-018, AI-024) and pre-rendered preview sections (EX-016). They are evictable. A missing entry is regenerated on demand, the owning feature shows its badge meanwhile, and **export waits for the regeneration** instead of exporting without it.
  - **Generated-media store (this feature):** derived files that a clip points at as its media and that the user approved or that are not bit-for-bit reproducible. These are TTS audio, stems, upscaled, enhanced or inpainted media, recordings, snapshots and external generator output. They are media items with `MediaOrigin::Generated`, never cache entries, so the evictable cache never holds the only copy of something a clip uses.
  - Where a spec still says "the project's cache folder", it means the media cache (PM-074), not this store.
- Each file-backed item records `MediaOrigin::Generated { recipe }`, so if its file goes missing, the relink dialog can offer **Regenerate** (when the producing feature supports it). A regenerated file is a new file and may differ slightly (see below).
**Edge cases:**
- **Project folder read-only:** the file falls back to a per-project folder in app data, with a warning that the project isn't portable until it is packaged (PM-101).
- **Source deleted:** the generated file keeps working.
- **Regeneration with a newer FreeCut or model:** may differ slightly. The user is told.
**Undo:** The producing area owns the compound op. For example, AU-021's **Split into stems…** adds the 2 or 4 stem items, new audio tracks with linked stem clips, and mutes (never deletes) the source clip's audio, all as one undo step. Undo removes the items but never deletes the files. PM-074's cleanup offers to trash unused generated files.
**Model notes:** `MediaOrigin { Imported, Generated { kind: GeneratedKind, parent: Option<MediaId>, source_range: Option<TimeRange>, recipe: serde_json::Value, produced_by: String }, Sample { sample_id: String, pack_version: u32 } }` on `MediaItem`, in `freecut-timeline::library` (plain data, no I/O). `Sample` is PM-063's. This single type is what other specs mean by `DerivedMedia { parent, kind, source_range }` (ai-tools.md) and by media provenance (AI-026, whose generator name and prompt go into `produced_by` and `recipe`); they don't define their own. `GeneratedKind` covers at least `Voiceover`, `CameraRecording`, `ScreenRecording` (PM-115), `Tts`, `Stem { which }`, `Upscaled`, `Inpainted`, `Snapshot`, `External` and `Cloud` (CG-011; `produced_by` is `"<service>:<model id>"`, for example `"higgsfield:seedance_2_5"`). Producing areas add variants for new file-backed outputs. Cache-only outputs such as mattes, interpolated frames and processed audio have no `GeneratedKind`, because they are never media items. `MediaSource::{File, ImageSequence, AudioOf}`, joined later by `Solid` (color clips, PM-108, with the origin `MediaOrigin::BuiltIn`) and `Embedded` (the video inside a motion photo, PM-109), which are virtual too. The store paths are computed by `freecut-project`.

### PM-042 Referenced asset files (LUTs, user stickers, imported fonts) — v1
**What the user sees:** Nothing new in the media panel. Some looks and elements depend on files that live in FreeCut's per-user libraries rather than in the media library: LUT files imported in CL, the user's own stickers (OV-020) and font files imported in TX (TX-018). When one of them isn't available on this computer, it is listed in the missing-files dialog (PM-091) under its own heading. Packaging (PM-101) can copy them into the package so the project opens complete on another computer.
**Inputs / controls:** **Locate…** per missing asset in the missing-files dialog. Nothing else here.
**Behavior / output:**
- Each owning area defines its own reference, and PM doesn't redefine it: CL's `LutRef` (SHA-256 content hash plus original file name), OV's user-sticker reference (content hash, OV-020) and TX's `FontRef` (family and style).
- PM derives a project-level **asset index** from the document: every distinct reference, with its kind, display name and usage count. It is computed on demand, never stored or edited, so it can't fall out of date.
- **Resolution on open:** first the owning area's user library in app data; then the `Assets/` folder next to the project file (written by PM-101); otherwise the asset is Missing, and the owning area shows its own fallback (the clip renders without the LUT, OV-018's missing-sticker placeholder, TX-006's font fallback chain).
- An asset found only in `Assets/` is used from there, and the owning area may offer to add it to the user's library.
- **Locate…** for a hash-addressed asset (LUT, sticker) hashes the chosen file with the owning area's hash. A match is accepted silently and handed to that area's library import. A mismatch warns that the file differs from the one the project used, with **Use anyway** or **Choose another**. Fonts go through TX-007's flow instead.
- Items that render as images (wipe patterns, background images) are ordinary media items, not assets.
**Edge cases:**
- **Same LUT used by 40 clips:** one index entry, showing "used by 40 clips".
- **A user-library file deleted while a project uses it** (for example a sticker removed under OV-020): listed as missing the next time the index is resolved (on open, or when the dialog opens).
- **The file the user originally imported from is edited or deleted:** nothing changes, because the libraries keep content-addressed copies. Assets therefore have no Changed state (unlike media in PM-093).
- **System-installed fonts:** referenced by family and style only. Packaging never copies them, and PM-101 lists them in the package notes.
- **Fonts whose license forbids embedding:** never collected (TX-007).
- **Use anyway with a different file:** every reference to the old hash is rewritten to the new one, in one step.
- **Project opened from a package on a computer that has none of the user's libraries:** everything resolves from `Assets/`.
**Undo:** Building the index and resolving files is n/a (derived). **Use anyway** rewrites references in the document as `RelinkAsset { key_before, key_after, touched }`, committed together with the dialog's `RelinkMedia` step (PM-091). Font replacement is TX-007's `ReplaceFontEverywhere`.
**Model notes:** `AssetKey { Lut(LutRef), Sticker(StickerRef), Font(FontRef) }`, reusing the reference types that CL, OV and TX define, and the pure `Document::asset_uses() -> Vec<AssetUse { key: AssetKey, display_name, uses: u32 }>` live in `freecut-timeline::library`. Lookups and hashing (SHA-256 for LUTs as CL specifies, through the `sha2` crate, MIT/Apache-2.0; stickers use whatever hash OV-020 specifies) run in `freecut-project` on a background thread.

### PM-043 Camera recording into the project — later
**What the user sees:** A **Record** button next to **Import** at the top of the Media panel opens a recording window. The window has:
- a live camera preview at the chosen aspect
- Camera, Format and Microphone dropdowns, plus an input level meter
- a countdown choice and a large **Record/Stop** button
- while recording: elapsed time, the file size so far, the free disk space and a dropped-frame counter

When the user stops, the take appears in the Media panel as a new item in the Generated bin, with a camera badge. If the user ticked **Add to timeline at playhead**, the take is also placed as a clip. This covers camera and microphone only; voice-only recording is AU-027, and screen capture (alone or with a camera overlay) is PM-115, which reuses this window.

Reference: the reference desktop editor's media area shows a Record entry beside Import. Its observed UI labels (paraphrased) cover camera, screen and combined capture; pause and resume; a retake that asks for confirmation; camera and microphone choice with an input level; exposure and focus lock indicators; naming the take; and placing camera and screen on separate tracks. The values below are FreeCut defaults.
**Inputs / controls (FreeCut defaults):**
- **Camera:** the cameras the OS reports, by name. Default: the camera used last, otherwise the system default.
- **Format:** the modes the device reports (resolution × frame rate × pixel format), listed as for example "1920×1080 · 30 fps". Default: the largest mode whose short side is 1080 or less at 30 fps, otherwise the closest mode.
- **Microphone:** AU-027's input-device list, plus **No audio**. Default: the device AU-027 used last, otherwise the system default. Input gain −20 to +20 dB, default 0 dB (same range as AU-027).
- **Countdown:** Off, 3 s (default) or 5 s. `Esc` cancels a running countdown.
- **Mirror preview:** on. It flips only the on-screen preview, the way people expect a selfie view to look. **Mirror the recording** is a separate checkbox, off, so text in the room reads correctly in the file.
- **Exposure lock** and **Focus lock:** two toggles, both off, shown only when the device exposes those controls. Locking holds the current values so brightness and focus don't drift mid-take, and a small lock mark shows on the preview.
- **Take name:** a text field, default "Camera 2026-10-12 14.32.05" (date and time). It names both the file and the media item; PM-002's file-name rules apply.
- **Pause / Resume:** available while recording. Pausing stops writing. Resuming continues the same file, and the paused span is left out, so the take plays straight through with no frozen section.
- **Retake:** during or right after a take. It asks "Discard this take and record again?", then moves the discarded file to the OS Trash and imports nothing.
- **Add to timeline at playhead:** off.
**Behavior / output:**
1. Capture runs on background threads, and the UI thread never opens a device. Preview frames reach the UI downscaled, at no more than the display's refresh rate.
2. **Clocks:** every video frame and audio buffer is timestamped against one monotonic capture clock. The audio clock is the master, and video timestamps are mapped onto it, so drift between a USB camera and a separate microphone doesn't build up over a long take. Frames the camera drops (common in low light) leave timestamp gaps, and the file is written as VFR. PM-035's VFR handling then applies like for any phone clip.
3. **File:** H.264 (libx264, 8-bit 4:2:0, CRF 18, a keyframe every second) plus AAC-LC 256 kb/s at 48 kHz, in a fragmented MP4 with 1 s fragments. A crash or power loss therefore loses at most the last second. If the encoder falls behind (the dropped-frame counter keeps rising), the window warns and switches libx264 to a faster preset for the rest of the take. Using a hardware H.264 encoder from EX's encoder list is an open question.
4. **Location:** `<Name> Media/Recordings/Camera 2026-10-12 14.32.05.mp4`, part of the generated-media store (PM-041). While recording, a marker file `<file>.recording` sits next to it and is removed when the file is finalized.
5. **On stop:** the file is finalized, probed (PM-035) and committed with `ImportMedia` as a generated item, `MediaOrigin::Generated { kind: CameraRecording, recipe: { camera name, mode, microphone } }`. With **Add to timeline at playhead** on, TL-012 places it at the playhead on the main track in the same compound step. If the project is still in Auto mode, PM-006 can match the canvas to the take like any first clip.
**Edge cases:**
- **No camera found:** the window says so and offers AU-027 for voice-only recording.
- **Permission denied:** the OS privacy settings block the camera (macOS and Windows), or on Linux the user can't open the video device or the sandbox's camera portal refuses. The window explains how to grant access, and nothing is recorded. The macOS app bundle must declare a camera usage description, which is a packaging requirement for this feature.
- **Camera in use by another app** (for example a video call): "The camera is being used by another app", with **Try again**.
- **Camera opens but sends no picture for 5 s:** the preview shows "The camera isn't sending a picture" with **Try again** and the camera list, never a silent black preview. Record stays disabled.
- **Pause held for a long time:** no limit. The file stays open, and the crash-safety rule (fragments of 1 s) still holds for what was written.
- **Retake after Add to timeline was ticked:** nothing was placed yet, so nothing is undone. A take that was already committed is removed with normal undo instead.
- **Device unplugged mid-take:** recording stops, the fragments written so far are kept and imported, and a notice says what happened.
- **Microphone lost mid-take:** video continues and the rest of the take is silent, with a notice. Stopping and restarting is up to the user.
- **The camera delivers a different mode than the one requested:** the file uses what arrives, and the window shows the actual size and frame rate.
- **Zero frames captured** (Stop during the countdown, or the device never delivered): no item is created, and the empty file is removed.
- **Disk nearly full:** a warning appears below 2 GB free, with an estimate of the recording time left at the current data rate ("About 9 min left"). Recording stops cleanly at 200 MB free. The take so far is kept. These thresholds are the recorder's own; PM-113's general warnings appear as well.
- **FreeCut crashes or the computer loses power mid-take:** the next time the project opens, a `.recording` marker without a matching item means an interrupted take. FreeCut offers **Import recovered take**, which imports the playable part, or **Move to Trash**.
- **Closing the project or quitting while recording:** FreeCut asks to stop and keep the take first.
- **Project folder read-only:** the take is written to the per-project fallback folder in app data, with PM-041's portability warning.
- **Export running:** recording is allowed, with a warning that the two compete for CPU and frames may drop. Playback is paused while recording, because this window doesn't play the timeline; pressing Play in the main window shows "Stop recording to play the timeline". Other editing stays available. Recording a reaction while the timeline plays is not offered; voice over picture is AU-027.
- **Very long takes:** there is no time limit other than disk space. Files over 4 GB are fine in MP4. PM-101 warns about FAT32 destinations when packaging.
**Undo:** `ImportMedia`. With **Add to timeline at playhead**, a compound of `ImportMedia` and TL's placement, labeled "Add recording". Undo never deletes the file (PM-041), and PM-074's **Clean up project…** offers to trash unused recordings.
**Model notes:**
- `freecut-media::capture` holds the camera side:
  - `list_cameras() -> Result<Vec<CameraInfo>, CaptureError>`
  - `CameraInfo { id, name, modes: Vec<CameraMode { width, height, rate: FrameRate, pixel_format }> }`, where rates are exact `FrameRate` rationals and never floats
  - `CaptureSession::start(CaptureConfig) -> Result<CaptureSession, CaptureError>`, which accepts PCM buffers through `push_audio` and is finished with `finish() -> Result<RecordedFile, CaptureError>`
- The backend is either FFmpeg's device inputs (AVFoundation on macOS, DirectShow on Windows, V4L2 on Linux) or the `nokhwa` crate (Apache-2.0/MIT, which wraps the same OS APIs). Which one to use is an open question. Either way it lives in `freecut-media`.
- Microphone capture reuses AU-027's input path in `freecut-audio`. `freecut-app` connects the two, so `freecut-media` never depends on `freecut-audio`.
- `CaptureError` (thiserror): `NoDevice`, `PermissionDenied`, `DeviceBusy`, `DeviceLost`, `NoFrames`, `UnsupportedMode`, `DiskFull`, `Io`.
- `CaptureSession::{pause, resume}` keep one output file. The timestamp offset that removes paused spans is computed by a pure function with tests (pause at 0 s, several pauses, pause during a dropped-frame gap). Exposure and focus locks go through `CameraControls`, which reports which controls the device supports.
- The recording window, the marker scan on open and the import step live in `freecut-app`. `GeneratedKind::CameraRecording` is PM-041's.

### PM-050 Media panel: grid view — mvp
**What the user sees:** The **Media** tab of the left asset panel (UI owns the panel layout) shows items as tiles. Each tile has:
- the thumbnail, or a mini waveform for audio (PM-072)
- the name, truncated in the middle
- a duration badge (`m:ss`, or a frame count for sequences)
- a type icon
- status overlays: pending spinner, Offline, and in v1, usage count, favorite and proxy

Hovering a tile shows a **+** button that adds the item at the playhead on the main track (TL). When the panel is empty, a large drop zone reads "Import media or drop files here". The panel header shows the item count ("Media · 128"); from v1, with filters active, it shows "12 of 128".
**Inputs / controls:**
- Click to select. `Mod`-click toggles, Shift-click selects a range, `Mod+A` selects all, and marquee selection comes in v1.
- Drag tiles onto the timeline (TL placement).
- `Delete`/`Backspace` removes the item (PM-058).
- Thumbnail size is fixed at 128 px in the mvp. v1 adds a size slider from 64 to 256 px.
- From v1, a single click shows the item in the player's media preview mode (EX-015, PM-060).
**Behavior / output:**
- The grid is virtualized, so only visible tiles are built. Their textures stream from the disk cache into the app's shared in-memory thumbnail texture cache, which TL-042 also uses (256 MB LRU).
- In the mvp, order is newest import first. Sorting comes in PM-053.
**Edge cases:**
- **10,000 items:** scrolling stays smooth.
- **Very long names:** truncated in the middle, with the full name in a tooltip.
- **Thumbnail failure:** a generic icon for the type.
- **Dragging a multi-selection:** clips are placed in the grid's visual order.
- **Type filter with no matches (v1, PM-054):** an empty state for that kind ("No audio in this project yet"), with **Import…**, instead of the generic drop zone.
- **Item not ready to place** (still probing, or a sample still copying, PM-063): dragging or **+** is accepted, and placement waits for it (TL-012), so there is never a "finish downloading first" refusal. The tile shows its spinner meanwhile.
- **Adding many items at once:** the **+** button shows a busy state until placement commits, and the panel stays responsive because placement waits on probes off the UI thread.
**Undo:** n/a (view). Actions delegate to their own ops.
**Model notes:** Lives in `freecut-app`. It reads `MediaLibrary` snapshots and never blocks on I/O.
- FreeCut default (mvp build): the header shows Import… and the item count ("12 items"). There is no hover **+** yet; a double-click appends a picture or video to the end of the main track, while an audio item goes on the first free audio track at the playhead (AU-030's add). The tile menu (UI-035) has *Add at playhead*, *Extract audio* and *Copy file path*. Audio tiles show ♪ instead of a poster; extracted items carry an "Extracted" badge.

### PM-051 Media panel: list view — v1
**What the user sees:** A grid/list toggle. List columns: Name, Type, Duration, Resolution, Frame rate, Audio (for example "48 kHz · stereo"), Size, Imported, File modified, Used, Path, Notes. Columns can be shown, hidden, resized and reordered. Clicking a header sorts by that column.
**Inputs / controls:** Header clicks, and a column chooser in the context menu.
**Behavior / output:** Same selection and drag semantics as the grid. Offline rows are shown in the warning color. Column layout (visibility, width, order) is a per-user preference, shared by all projects.
**Edge cases:**
- **Missing values** (frame rate on an audio file, resolution on audio): shown as "—" and sorted last.
- **Panel narrower than the visible columns:** the Name column stays pinned on the left and the rest scroll horizontally inside the panel.
- **Every column hidden but Name:** allowed. Name can't be hidden.
- **Pending items:** show the file name, with "…" in columns whose values come from the probe.
- **10,000 rows:** rows are virtualized, like the grid.
**Undo:** n/a (view state).
**Model notes:** Lives in `freecut-app`.

### PM-052 Bins (folders) — v1
**What the user sees:** A collapsible bin tree at the side of the media panel, with a breadcrumb when the panel is narrow. **New bin** is available, items can be dragged into bins, and bins can be nested. Each bin shows its item count. The root, **All media**, can show the contents of every bin.
**Inputs / controls:**
- Rename inline (F2 or double-click).
- Drag to nest or reorder.
- Delete bin.
- A "Show items in sub-bins" toggle, on by default at the root.
**Behavior / output:**
- Bins are virtual. They exist only in the project and never touch folders on disk.
- The **Generated** bin is created on demand (PM-041).
- Deleting a bin asks for one of two choices:
  - **Move its items to the parent bin** (default)
  - **Remove the bin and its items from the project**, which follows PM-058 for items that are in use
- The maximum nesting depth is 16 (FreeCut default). (The reference app caps its folders at 10 levels; FreeCut allows 16 because bins are virtual and cost nothing on disk.)
- Deleting a bin together with its items is undoable, so it needs only the two-choice dialog above, not an extra "this can't be undone" confirmation. Media files on disk are never touched.
- Bins never move files on disk. Moving files between real folders is the file manager's job; afterwards PM-090 and PM-091 find them again.
**Edge cases:**
- **Duplicate names under one parent:** a suffix is added.
- **Names:** whitespace at either end is trimmed. An empty name becomes "Untitled bin". Any other character is allowed, because bins are not folders.
- **Moving a bin into its own descendant:** refused.
- **Nesting past 16 levels:** the drop is refused with "Bins can be nested up to 16 levels".
- **Drop target removed meanwhile** (the bin was deleted by an undo during the drag): the drop is ignored with a short toast, and the items stay where they were.
- **Deleting the bin you're looking at:** the view moves to its parent.
**Undo:** `CreateBin`, `RenameBin`, `MoveBin`, `MoveItems { items, from, to }`, `DeleteBin { snapshot, items, mode }`.
**Model notes:** `Bin { id: BinId, name, parent: Option<BinId>, order: u32 }`. `MediaItem.bin: BinId`. Tree invariants (no cycles, unique order per parent) are property-tested in `freecut-timeline::library`.

### PM-053 Sort — v1
**What the user sees:** A sort menu: Name (natural), Date imported (default, newest first), File date (created or modified), Duration, Type, Size, Resolution, Usage, Favorites first. Each can be ascending or descending. Bins are listed before items.
**Inputs / controls:** The menu, plus list-view header clicks.
**Behavior / output:** Natural, case-insensitive ordering that respects the system locale where available. Ties fall back to name and then import order, so sorting is stable. The setting is saved per project as view state (PM-021).
**Edge cases:**
- **Pending items:** sorted by import time.
- **Missing values** (for example, duration of a still): sorted last.
**Undo:** n/a (view state).
**Model notes:** Pure comparators with tests, in `freecut-app::media_panel::sort`.

### PM-054 Search & filters — v1
**What the user sees:** A search field at the top of the panel (`Mod+F` while the panel has focus). Type chips: All · Video · Audio · Images · Sequences. Shape chips: Any (default) · Landscape · Portrait · Square. A status filter menu: Unused, Used, Favorites, Offline, Has proxy, Generated, Interpreted, and later Label (PM-110). Active filters are highlighted, and a **Clear** button resets them. When nothing matches, the panel says so, repeats the query and suggests clearing filters or checking the spelling. Focusing the empty field lists the last 10 searches in this project, with **Clear history**.
**Inputs / controls:** Text input (debounced 100 ms), the chips, the menu, and a scope toggle: "This bin and sub-bins" (default) or "Whole project".
**Behavior / output:**
- Search matches the display name, file name, notes and bin name. It is case- and accent-insensitive, and several words must all match.
- Results from other bins show their bin path.
- Choices within one filter category combine with OR. Different categories combine with AND.
- **Shape** uses the display size after rotation and pixel-aspect correction (PM-035) and any interpretation override (PM-040): wider than 1.05:1 is Landscape, taller than 1:1.05 is Portrait, otherwise Square (FreeCut default tolerance). Audio items match only Any.
- The search history is view state (`view/<project-id>.json`, PM-021), never in the project file.
- Searching inside clips is other areas' work: spoken words are AI-027 (later), which adds its chip to this field. Searching by what is seen in the picture is out of scope (ai-tools.md).
**Edge cases:**
- **Pending items:** match by file name. They match no Shape chip until probed.
- **10,000 items:** filtering finishes within one frame (16 ms), using precomputed folded keys.
**Undo:** n/a (view state).
**Model notes:** A pure matcher with tests, in `freecut-app`.

### PM-055 Usage indicators & find on timeline — v1
**What the user sees:** A tile badge with the number of timeline clips using the item (for example "×3"). Unused items show no badge. In the menu, **Find on timeline** selects those clips and scrolls to the first; using it again moves to the next. The opposite direction, revealing a clip's media item from the timeline, is TL-046 (later).
**Inputs / controls:** Menu commands.
**Behavior / output:**
- Usage is derived live from the document and counts every reference: clips on any track, including muted, hidden and locked ones; clips in nested sequences; TR canvas backgrounds; wipe patterns; and virtual and generated items whose recipe points at the item.
- Usage is never stored in the file.
**Edge cases:**
- **Item used only as a source for an extracted-audio item:** counted as used, so "remove unused" can't break the extracted item.
- **Usage changes during playback:** the badges update after the edit.
**Undo:** n/a. Selection changes follow TL's selection rules.
**Model notes:** `Document::media_usage() -> HashMap<MediaId, u32>` is computed in `freecut-timeline` and recomputed incrementally from each committed change set.

### PM-056 Favorites — v1
**What the user sees:** A star on hover and in the context menu marks an item as a favorite. The Favorites filter (PM-054) and the "Favorites first" sort (PM-053) use it.
**Inputs / controls:** The star toggle. The key binding is proposed by UI.
**Behavior / output:** A per-project flag. Favorites across projects belong to PM-062. The favorites in asset libraries (stickers, effects and so on) are OV's library model, not this one.
**Edge cases:**
- **Offline items:** can be starred.
- **Multi-selection with mixed states:** the action sets all of them to starred.
**Undo:** `SetFavorite { items, before, after }`. This is a document edit stored in the project.
**Model notes:** `MediaItem.favorite: bool`.

### PM-057 Rename, notes & poster frame — v1
**What the user sees:** Items can be renamed (F2, or double-click the name). The info panel has a **Notes** field. In source preview, **Set poster frame** chooses the frame used for the tile thumbnail.
**Inputs / controls:**
- Name: 1–200 characters. Empty reverts to the file name.
- Notes: up to 2,000 characters.
- Poster frame: the current source-preview time.
**Behavior / output:** The display name never renames the file on disk; the info panel shows both names. The poster time is stored in media time.
**Edge cases:**
- **Several items with the same display name:** allowed, because ids distinguish them.
- **Right-to-left or combining text:** fine.
- **Poster frame beyond the duration after a relink to a shorter file:** clamped to the last frame.
**Undo:** `RenameMedia`, `SetNotes`, `SetPosterFrame`, each with before/after values.
**Model notes:** `MediaItem { display_name: Option<String>, notes: String, poster: Option<Tick>, markers: Vec<SourceMarker> }`, with times in media ticks. `SourceMarker { at: Tick, label: String }` is the media-item marker list that AI's scene detection writes through its own "Add scene markers" op ([ai-tools.md](ai-tools.md)); source preview (PM-060) draws them as ticks on its scrub bar.

### PM-058 Remove from project & remove unused — mvp
**What the user sees:** Select items and press `Delete`, or choose **Remove from project**.
- **Unused items** are removed right away, with an Undo toast.
- **Used items** bring up a dialog saying that a number of timeline clips use the media. It offers **Remove media and clips** or **Cancel**.
- In v1, **Remove unused media…** lists every unused item and its total size, and asks for confirmation.

The wording always says "from project". Files on disk are never deleted. (Some reference-app dialogs warn that removing media can't be undone, while another says it can; in FreeCut removal is always on the undo stack, so the dialog only warns about the clips that use the media.)
**Inputs / controls:** The keys, the commands and the dialogs.
**Behavior / output:** A compound op removes the clips (TL removal without ripple, which leaves gaps; FreeCut default) and then the items. Cached thumbnails, peaks and proxies stay until LRU eviction, so undo is instant.
**Edge cases:**
- **Clips on locked tracks:** the removal is refused, with a message naming the track(s) to unlock. Lock semantics win.
- **Item still probing:** its jobs are cancelled.
- **Item that virtual items depend on** (for example, extracted audio): the dialog lists them and removes them together. File-backed generated children keep working.
- **Removing everything:** allowed, and the library becomes empty.
**Undo:** `RemoveMedia { items (with bin and order), removed_clips }`, a compound with TL's clip-removal change set. The inverse restores the items and clips exactly.
**Model notes:** A compound op in `freecut-timeline`.

### PM-059 Media info & reveal — v1
**What the user sees:** Selecting an item fills an **Info** section in the inspector (UI owns the inspector frame). It shows:
- display and file name, and the path (with a copy button)
- type, container and codecs
- resolution and display aspect, rotation, pixel aspect
- frame rate, with "variable" when the source is VFR, and the difference from the project rate if any
- duration and bit rate
- audio format and stream list
- color (SDR/HDR, range) and alpha
- start timecode
- size, and created/modified dates
- fingerprint, proxy and usage
- origin (imported or generated) and active overrides
- the media item id, with a copy button, for bug reports and support threads

The context menu has **Reveal in Finder/Explorer/file manager**, and in v1, **Open with default app**.
**Inputs / controls:** Copy path, Reveal, Open.
**Behavior / output:**
- **Reveal** uses each platform's mechanism: select the file in Finder, select it in Explorer, or ask the desktop's file-manager service to show it, falling back to opening the parent folder on Linux. It goes through the `opener` crate (MIT/Apache-2.0).
- **Multi-selection:** shows the count, total duration and total size.
**Edge cases:**
- **Offline item:** the path is shown in the warning color, with **Locate…** (PM-091).
- **Unmounted volume:** Reveal is disabled, and the tooltip explains why.
**Undo:** n/a.
**Model notes:** Lives in `freecut-app`. It reads `MediaInfo` and `MediaItem`.

### PM-060 Hover scrub & source preview — v1
**What the user sees:**
- **Hover scrub:** moving the pointer across a video tile scrubs through it, using 16 cached frames per item. Audio tiles show a moving line over the mini waveform. Source markers (PM-057) show as ticks along the tile's bottom edge while hovering.
- **Source preview:** a single click on a tile opens the item in the player's media preview mode. EX-015 owns that mode: its transport, the I/O marks, and returning to the timeline. Source-side Insert/Overwrite is TL-047 (later).
- The in/out range marked there is remembered per item, drawn as a highlighted span along the bottom of the tile, and used by the next add to the timeline (TL-012).
**Inputs / controls:** Pointer hover and click. The range itself is set with EX-015's controls. Audio while hover-scrubbing is off by default (FreeCut default). (The reference app scrubs a tile with the scroll wheel while hovering; FreeCut follows the pointer's position instead, because the wheel scrolls the panel.)
**Behavior / output:**
- Hover scrub uses only cached filmstrip frames (PM-071), so it never decodes on hover.
- Source playback uses the normal decode path, and proxies when they are on (PM-081).
- The preview range is per-item view state, kept in `view/<project-id>.json` (PM-021). Marking a range never dirties the project and isn't on the undo stack, matching TL-043's rule that range marks stay out of history.
**Edge cases:**
- **Offline item:** hover shows the offline slate (EX-005), and EX-015 disables transport.
- **Stills:** no scrubbing, and no range.
- **Filmstrip frames not cached yet:** the tile shows its thumbnail until they arrive. Hover never queues a decode on the UI thread.
- **In set after out:** the marks swap.
- **Stored range beyond the duration after a relink to a shorter file:** clamped to the new duration. If nothing remains, the range is cleared.
- **Import undone, then redone:** the stored range comes back, because it is keyed by `MediaId` and redo restores the same id.
**Undo:** n/a (view state). Placing media is TL's op (TL-012, and later TL-047).
**Model notes:** The hover frames come from the filmstrip job in `freecut-media` (PM-071). `PreviewRanges: HashMap<MediaId, TimeRange>` (media ticks) is view state in `freecut-app`, saved in the view file. EX-015's player reads it; where EX-015 mentions `MediaItem.preview_range`, this view-state map is meant, so it isn't a document field.

### PM-061 Replace media file — v1
**What the user sees:** **Replace with file…** in an item's menu swaps the file behind a library item for a different file, for example a re-exported graphic or a better take. Every clip, edit and effect is kept. This is different from TL's per-clip **Replace…**, which points one clip at another library item, and from relink (PM-091), which finds the same content in a new place.
**Inputs / controls:** A file dialog, then a summary dialog if something doesn't fit.
**Behavior / output:**
- The new file is probed.
- Clips keep their source in and out points in media time.
- If the new media is shorter than some clip ranges, the dialog lists those clips and offers **Trim clips to fit** (default) or **Keep (the missing part shows as offline)**.
- **Different resolution:** framing is kept, because it's normalized (TR).
- **Different frame rate:** positions are kept, because source times are rational.
- **Different kind:** video ↔ image is allowed. Visual ↔ audio-only is refused while clips sit on visual tracks.
**Edge cases:**
- **Replacing with a file that's already in the library:** the clips are pointed at the existing item, and the old item is removed (one compound op).
- **Replacing offline media:** works like a relink without validation.
**Undo:** `ReplaceMedia { media, before, after, clip_trims }`.
**Model notes:** A `freecut-timeline` op. The probe comes from `freecut-media`.

### PM-062 Personal library across projects — later
**What the user sees:** A **My library** tab in the asset panel, shared by all projects, for files the user reuses: logos, intro clips, their own sound effects. It has bins and favorites. Dragging an item into a project imports it as a normal media item. This is FreeCut's local answer to the reference app's stock library, cloud space and team asset areas in the media panel: the user points it at their own folders, and nothing is downloaded or synced.
**Inputs / controls:** Add files or folders, bins, favorites, remove.
**Behavior / output:** Stored in app data as file references, with an optional "copy into library folder". It reuses the library types and the relink tools.
**Edge cases:**
- **Library item offline:** relink as in PM-091.
- **Item removed from My library:** projects that imported it are unaffected.
**Undo:** Managing the library is n/a (outside any project document). Bringing an item into a project is `ImportMedia`.
**Model notes:** Lives in `freecut-app`. It reuses the `freecut-timeline::library` types.

### PM-063 Bundled CC0 sample media pack — v1
**What the user sees:** In the asset panel's **Media** tab (UI-004), a **Samples** sub-category sits below the project's own media. It shows a small set of short clips and stills made by FreeCut contributors. Each tile has a "Sample · CC0" badge, a hover scrub (PM-060) and a **+** button. Dragging a sample to the timeline, or pressing **+**, works like adding any media. The same files are what library previews play when no clip is selected: transition tiles (FX-004), effect and animation hover previews (FX-016, UI-007), filter thumbnails when the timeline is empty (CL-003), and the hardware-decoding **Test** (UI-028). Help ▸ About ▸ *Bundled assets* (UI-051) lists every sample with its creator and license. In the mvp the Media tab shows project media only.

Reference: public guides describe the reference editor offering an online library of stock clips and music inside the editor. FreeCut has no online catalogue (see Out of scope). This small offline pack is its substitute for "something to try things on". Music and sound effects are not part of it; they are AU-030 and AU-031.
**Inputs / controls:**
- The Samples sub-category, the tiles, drag and **+**.
- Preference **Show sample media in the Media tab:** on by default. Turning it off hides only the sub-category. Previews keep using the pack.
**Behavior / output:**
- **Contents (FreeCut default list, pack version 1).** Every item is recorded, photographed or rendered by FreeCut contributors and dedicated to the public domain under CC0 1.0, or generated from FFmpeg's synthetic test sources:

| Role | Items | Format |
|---|---|---|
| Landscape clips | 4 clips, 8 s, 1920×1080, 30 fps: a wide street scene with movement; foliage and sky (useful for color looks); one person speaking to camera, with clean speech; a slow pan across a textured surface | VP9 in WebM, with Opus 48 kHz stereo where the clip has sound |
| Portrait clips | 2 clips, 8 s, 1080×1920, 30 fps: a handheld walk; an object turning on a table | VP9 in WebM |
| Keying demo | 1 clip, 6 s, 1920×1080: a subject in front of an evenly lit green backdrop (TR chroma key) | VP9 in WebM |
| Stills | a 3840×2160 landscape photo, a 2160×3840 portrait photo, the neutral reference image CL-003 describes (color patches with skin, sky and foliage), and one cut-out object on a transparent background | JPEG (quality 90), PNG with alpha |
| Transition tile pair | 2 contrasting 640×360 stills that FX-004 renders tile loops from | PNG |
| Decode test | 4 clips, 2 s, 1920×1080, 30 fps, rendered from FFmpeg's test pattern and sine tone: H.264, HEVC (8-bit), VP9 and AV1 | MP4 and WebM |

- **Why VP9 and Opus:** the pack's everyday items use royalty-free codecs that every FFmpeg build FreeCut supports can decode, including distribution builds that leave out patent-encumbered decoders. Only the decode-test clips use other codecs. When this build can't decode one of them, UI-028 reports that codec as "not available in this build".
- **Size budget:** at most 60 MB installed for the whole pack, and at most 8 MB per file (FreeCut defaults). CI fails the build when either limit is exceeded. Tile previews, filmstrips and thumbnails made from samples are not shipped. They are generated on first use into the media cache (PM-074).
- **Licence manifest:** `samples/manifest.json`, our own schema:
  - top level: `format: "freecut-sample-pack"`, `pack_version`, and `samples: [...]`
  - each entry: `id` (stable, never reused for different content), `file`, `kind`, `title`, `tags`, `roles` (for example `media-tab`, `transition-tiles`, `effect-preview`, `filter-thumbnail`, `keying-demo`, `decode-test`), `sha256`, `bytes`, `license` (an SPDX identifier, `CC0-1.0`), `creator`, `origin` (`recorded`, `rendered` or `generated`) and `people_release` (true when an identifiable person appears and a signed model release is on file)
  - FreeCut finds a sample by role, never by file name, so packagers and future packs can swap files freely
- **In the source repository:** the files live in `assets/samples/` and follow the REUSE specification. A `REUSE.toml` annotation gives every file `SPDX-License-Identifier: CC0-1.0` and its creator, and the license text is in `LICENSES/CC0-1.0.txt`. The decode-test clips are made by a checked-in script from FFmpeg's synthetic sources, so anyone can reproduce them.
- **Where it is installed** (read-only; FreeCut never writes there):
  - macOS: `FreeCut.app/Contents/Resources/samples/`
  - Windows: `<install folder>\samples\`
  - Linux system packages: `<prefix>/share/freecut/samples/`, for example `/usr/share/freecut/samples/`. Flatpak and AppImage builds use the same relative path inside their prefix.
  - Lookup order: the `FREECUT_SAMPLES_DIR` environment variable (development builds and packagers' tests), then the platform location, then `samples/` next to the executable (portable builds). The first folder with a valid manifest wins.
- **Optional for packagers.** Official FreeCut builds include the pack. Distributions may ship it as a separate architecture-independent data package (the usual practice for large arch-independent data), as an optional add-on, or not at all. FreeCut must be fully usable without it, and CI runs the test suite with the pack absent. Without the pack:
  - the Samples sub-category is hidden (UI-004 hides empty content rather than greying it out)
  - FX-004, FX-016 and UI-007 previews use procedural frames drawn by `freecut-render` (gradients, a grid and a moving shape), so tiles still animate
  - CL-003's empty-timeline thumbnails use a procedurally drawn color chart, without skin or foliage tones
  - UI-028's **Test** first generates a 2 s clip from FFmpeg's synthetic sources with this build's H.264 encoder, into the media cache, and reports "not available" if that encoder is missing
  - About ▸ *Bundled assets* says "Sample media: not installed"
- **Adding a sample to a project:** the file is copied, on a background I/O job, to `<Name> Media/Samples/` under its own file name, and checked against the manifest's `sha256`. Then it is probed (PM-035) and committed with `ImportMedia` as a normal media item with `MediaOrigin::Sample { sample_id, pack_version }` and `MediaStorage::CopiedToProject`. Copying instead of pointing into the install folder keeps projects portable. App updates or packagers may move or remove the pack, but projects never depend on install paths. Packaging (PM-101) treats these copies like any other media. A sample of at most 8 MB copies in well under a second, and placement waits for it the way it waits for a probe (TL-012).
**Edge cases:**
- **Pack missing, or the manifest is unreadable or from a newer pack format:** everything falls back as described above. The log records why.
- **A file whose hash or size doesn't match the manifest** (corrupted install, or a packager edited files without updating the manifest): that sample is hidden, and roles that need it use the procedural fallback. Hashes are checked lazily, on a background thread, the first time each file is used.
- **Packager replaces the samples with their own:** allowed when the manifest is valid. About shows whatever creators and licenses the manifest lists. FreeCut checks that the fields are present, not that the licensing is true.
- **Same sample added twice to one project:** PM-038 recognizes the fingerprint and reuses the existing item. No second copy is made.
- **FreeCut updated and a sample changed or removed in the new pack:** existing projects are unaffected, because they hold their own copies. A changed sample gets a new `id`.
- **The project's copy of a sample was deleted:** the item goes Offline like any media (PM-090). The Relink dialog (PM-091) adds **Restore from sample pack** when the installed pack has the same `id` and `sha256`.
- **Project folder read-only:** the copy goes to the per-project fallback folder in app data, with PM-041's portability warning.
- **Disk full while copying:** a toast explains, and nothing is added.
- **Sample dropped on an empty timeline in Auto mode:** PM-006 matches the canvas to it like any first clip. A portrait sample gives a 1080×1920 project.
- **Sample with an identifiable person:** accepted into the pack only with a signed model release, because CC0 waives copyright but not the person's privacy or publicity rights. The release stays with the project maintainers and isn't shipped.
**Undo:** Adding a sample is `ImportMedia`. Dropping it on the timeline is a compound with TL's placement, labeled "Add media". The file copy is n/a (file I/O). Undo leaves the copy on disk, and PM-074's **Clean up project…** lists unused sample copies. Showing or hiding the sub-category is n/a (preference).
**Model notes:**
- `freecut-project::samples` holds the serde types and validation, with thiserror errors and table tests for valid and invalid manifests:
  - `SampleManifest { format, pack_version: u32, samples: Vec<SampleEntry> }`
  - `SampleEntry { id, file, kind, title, tags, roles: Vec<SampleRole>, sha256, bytes: u64, license: String, creator, origin: SampleOrigin, people_release: bool }`
  - `load(dir) -> Result<SampleManifest, ManifestError>`
- `freecut-app::samples::SamplePack` locates the folder, verifies hashes lazily on the background executor, and offers `by_role(SampleRole)` to FX, CL and UI previews. The copy and import job reuses PM-030's `ImportQueue`.
- `MediaOrigin::Sample { sample_id: String, pack_version: u32 }` is the variant added to PM-041's `MediaOrigin` in `freecut-timeline::library`.
- CI checks the size budget, that the manifest matches the files, `reuse lint`, and the test suite with the pack absent.

### PM-070 Media thumbnails — mvp
**What the user sees:** Video and image tiles show a representative frame. A shimmer shows while it is generated. Images with alpha show it over a checkerboard. Audio tiles are covered in PM-072.
**Inputs / controls:** The poster frame (PM-057). Otherwise automatic.
**Behavior / output:**
- **Poster time:** min(1 s, 10 % of the duration) (FreeCut default, which avoids black first frames), unless the user has set one.
- **Generation:** `freecut-media` decodes at up to 256 px tall. Rotation, pixel aspect and interpretation are applied, and HDR is tone-mapped by CL-002's decode rule.
- **Storage:** WebP in the media cache, keyed by (fingerprint, time, size, interpretation hash).
**Edge cases:**
- **One-frame media:** frame 0.
- **Decode fails at the poster time:** tries 0 s, then shows the generic type icon.
- **Animated images:** the first frame.
- **Corrupt cache entry:** silently regenerated.
**Undo:** n/a (derived cache).
**Model notes:** `freecut-media::thumbnail(&MediaSource, at: Tick, max_h: u32) -> Result<RgbaImage, DecodeError>`. Cache I/O goes through `freecut-media::cache`.

### PM-071 Timeline filmstrip frames — mvp
**What the user sees:** Video clips on the timeline show a strip of frames along their length. More frames appear as you zoom in. How the strip is laid out and drawn (tile width, sampling times, display modes) is TL-042; this feature covers producing and caching the frames TL-042 asks for.
**Inputs / controls:** Timeline zoom (TL-041) and TL-042's filmstrip preferences.
**Behavior / output:**
- **Serving requests:** each TL-042 request for a time is answered with the nearest frame already generated, and a closer frame is queued if the gap is visible at the current zoom.
- **Base strip:** 1 frame per 2 s of media at 64 px tall (FreeCut default), generated progressively in the background. For speed, it uses the nearest keyframe, so it's visually approximate.
- **On demand:** as you zoom in, denser frames down to every source frame are generated for the visible range only.
- **Reuse:** frames are stored per media item, not per clip, so trims, splits, speed changes and reversed clips (time mapping from TR) all reuse them.
- **Stills:** one repeated thumbnail.
**Edge cases:**
- **Three-hour files:** the base strip of 5,400 frames is built lazily, visible range first.
- **Offline media:** the striped offline pattern (TL-042); no frames are requested.
- **Proxies present:** filmstrips are generated from the proxy, which is faster, and stay valid because the timing matches.
**Undo:** n/a.
**Model notes:** A filmstrip job and cache in `freecut-media`, answering TL-042's `ThumbnailRequest`. Drawing is in `freecut-app` (TL-042).

### PM-072 Waveform data & audio tile previews — mvp
**What the user sees:** Audio tiles in the media panel show a mini waveform, or embedded cover art when there is one. Timeline waveforms are AU-009.
**Inputs / controls:** None.
**Behavior / output:**
- The peak data is defined by AU-009: a min/max pyramid at 256 samples per bin, ×4 per level, one per stream, built by a `freecut-media` job.
- This area owns where it is stored: the media cache, keyed by (fingerprint, stream index, format version).
- This area also owns the tile rendering, which takes the coarsest pyramid level that has at least 2 bins per pixel.
**Edge cases:**
- **Silent media:** a flat line.
- **Video without audio:** no waveform, and the thumbnail is used.
- **Cache from an older format version:** rebuilt.
- **10-hour recordings:** progressive. The tile shows the part that's ready.
**Undo:** n/a.
**Model notes:** Peak types are defined in `freecut-media` (AU-009). Tile drawing lives in `freecut-app`.

### PM-073 Background task list & media jobs — mvp
**What the user sees:** The status-bar task indicator (for example "Processing 12 files…") and the toasts belong to UI-040. Clicking the indicator opens the list this feature owns: the app's **single background-task list**. It has a row per job (Probe, Thumbnails, Filmstrip, Waveform, Proxy, Copy, Package, Relink search, and from v1 sample copies and Deleted-projects handover; later camera- and screen-take finalizing, Re-wrap (PM-111), Add projects from a folder (PM-106) and Move projects folder (PM-112)). Each row shows progress and time left, with Pause and Cancel, and the list has **Cancel all**. Failed jobs show the reason and a **Retry** button. Other areas report into this list and never draw a second one:
- From v1, AI-001's analysis jobs appear as rows labelled with AI-001's job kinds. AI-001 defines only those kinds, its per-clip progress strips and badges on the timeline, and its analysis pool's concurrency settings.
- From v1, EX-038's export queue appears as rows too.
- Cloud generation runs (CG-009) appear as rows with owner *Cloud* when that opt-in area is on.
**Inputs / controls:** Row actions and **Cancel all**. Row owners may disable an action that doesn't apply (for example Pause on an export that can't pause).
**Behavior / output:**
- One keyed job system, with duplicate requests merged.
- **Priority order:** probe; then visible thumbnails and waveforms; then the remaining thumbnails; then filmstrips; then proxies; then copy and package.
- **Worker pools:** CPU jobs run at max(1, cores/2) (FreeCut default). I/O jobs have a separate pool of 2.
- **Throttling:** background decoding is throttled during playback, because preview decode comes first. Proxy and copy jobs pause during export unless the user resumes them.
- **Low disk space (v1):** jobs that write large files pause with the state "Paused: low disk space on '<volume>'" and resume by themselves once PM-113 reports enough space. The row's **Resume** is disabled meanwhile, with that reason.
- **Per-item cancel:** cancelling one Proxy row cancels only that file's proxy (PM-080). Its item stays usable from the original.
- Results reach the UI thread through channels. The UI never waits on a job.
**Edge cases:**
- **Item removed:** its jobs are cancelled.
- **Quitting while a copy or package job runs:** FreeCut asks for confirmation. Derived-data jobs are simply cancelled.
- **A job panics:** contained, reported as failed, and logged.
- **Sleep or resume:** jobs continue.
**Undo:** n/a.
**Model notes:** `JobQueue` with keys and priorities in `freecut-app`. The work functions live in `freecut-media` and `freecut-project`. The priority logic is pure and tested. The list is a view over `TaskRow { id, owner: TaskOwner { Media, Analysis, Export, Cloud }, label, target, progress, eta, state, actions }`, which `JobQueue`, AI-001's analysis pool and EX-038's export queue all feed. AI-001's analysis pool is separate (its own concurrency settings) but reports into this list. UI-040 reads the list's summary for the status-bar indicator.

### PM-074 Media cache management — v1
**What the user sees:** Preferences > Storage shows:
- the cache location (Change… moves the cache or starts a fresh one)
- the current size by category: thumbnails and filmstrips, waveforms, image previews, analysis caches (stabilization, reverse, loudness and so on from TR/AU/CL, and AI-001's analysis records), mattes and interpolated frames (AI-014, TR-042, AI-018), processed audio (AU-018, AI-024), pre-rendered preview sections (EX-016), and proxies
- a total size limit (default 20 GB, FreeCut default; 1–500 GB or Unlimited). Inside it, thumbnails and filmstrips keep their own sub-limit, 2 GB by default (TL-042's preference), so proxies can't crowd them out.
- an age rule, **Remove entries not used for**: 7, 14, 30 (default), 60 or 90 days, or Never. It runs at startup and once a day, alongside the size limit.
- **Clear cache** and **Clear proxies**, plus per-category clear buttons in UI-026's usage bar (for example only re-wrapped files, PM-111, or only the render cache, EX-016)
- the total is measured in the background; until it is ready the field reads "Measuring…" (UI-026)

**Clean up project…** lists, for the current project, unused generated files and recordings, unused copies in the project media folder (including sample copies, PM-063), and orphaned proxies. Selected files are moved to the OS Trash.
**Inputs / controls:** The fields and buttons above.
**Behavior / output:**
- **What belongs here:** this one cache holds deterministic, regenerable data only, following PM-041's storage rule. That means thumbnails, filmstrips, peaks, image previews and proxies, plus stabilization data, optical-flow or AI-interpolated frames, mattes, analysis records, processed audio and pre-rendered sections. Every spec that says "the project's cache folder" means this cache. Files a clip points at as its media (TTS audio, stems, upscaled, enhanced or inpainted media, recordings) are never cache entries. They live in the generated-media store (PM-041).
- **Keys:** every entry keyed on media content starts from PM-038's fingerprint, never a `MediaId` or mtime, so entries are shared across projects and survive copies, file moves and relinks. Staleness follows PM-093: a changed file gets a new fingerprint, and old entries stop matching and age out.
- **Regeneration:** an entry is always rebuildable from the source media and the settings recorded in the project. When a needed entry is missing (evicted, cleared, or this is another computer), the owning feature regenerates it on demand and shows its badge meanwhile. Preview may fall back to the unprocessed source. **Export waits for regeneration**, with progress in the export dialog, and never exports without the processing.
- AI-001's analysis records live under the same root and count toward the same limit. AI owns their key fields (on top of the fingerprint) and its own **Clear analysis cache** button. Results that AI-001 keeps inside the project file, such as user-corrected transcripts, are document data, not cache.
- **Eviction:** LRU when over the limit, and by age under the age rule. Entries used by the open project's visible items are pinned, and proxies are evicted last.
- **Writes:** atomic (temp file plus rename), so two FreeCut processes can share the cache.
- **Leftover temporary files:** at startup FreeCut cleans up its own stale temporary files older than 24 h. Unfinished cache writes are deleted. The `<file>.tmp` files of interrupted saves (PM-012) next to registered projects that aren't open, and partial copies from copy or package jobs that a crash interrupted, are moved to the OS Trash instead, because they sit in the user's folders. It never touches `.freecut` files, the generated-media store or anything it didn't create. The same cleanup can be run from UI-026 as **Remove temporary files**.
- **Changing the location while a project is open:** allowed, with no restart. New entries go to the new location at once. Existing entries are moved by a background task (PM-073) when the user chose "move", and are read from the old location until they arrive. (The reference app requires closing the project and restarting for a cache move or clear; FreeCut's atomic, content-keyed entries make that unnecessary.)
- **Render cache:** EX-016's pre-rendered sections are one category of this cache and share its limit. (The reference app gives the render cache its own folder and size cap; FreeCut keeps one location, and the per-category clear covers the "clear render files" need.)
**Edge cases:**
- **Cache location on a removed drive:** FreeCut falls back to the default with a warning.
- **Clearing while editing:** everything regenerates on demand, and the preview may stutter briefly. Expensive entries such as mattes or interpolated frames show their owner's "re-analyzing" badge until rebuilt.
- **Project opened on another computer:** none of its cache entries exist there. They are rebuilt on demand, and the first export may take longer because it waits for them. Nothing a clip points at is lost, because those files are in the generated-media store, which travels with the project.
- **Disk nearly full:** the cache limit shrinks temporarily, and PM-113's warning is shown.
- **Clearing downloaded or bundled library caches:** FreeCut downloads nothing, so there is no asset-download cache to clear. Bundled packs are read-only and never cached here; previews made from them are ordinary cache entries.
- **Age rule set to 7 days on a project last opened a month ago:** its entries are gone and regenerate on next open. Nothing in the project breaks.
**Undo:** n/a. This is cache maintenance, and trashed files can be recovered through the OS.
**Model notes:** `freecut-media::cache::{path_for(key), put_atomic, get, evict_lru(limit), evict_older_than(age, now), usage_by_category(), sweep_temp(now)}`. The eviction and sweep decisions are pure functions over an entry listing, tested with a fake clock. The settings UI lives in `freecut-app`.

### PM-080 Proxy media — v1
**What the user sees:**
- Item menu: **Create proxy** and **Delete proxy**.
- A Project Settings policy: **Proxies: Off / Manual / Automatic for large media**. The default is Off, but FreeCut suggests proxies when you import heavy media. For example, it warns that editing 4K 10-bit HEVC may be slow on this computer and offers to create proxies.
- Proxy resolution: 540p, 720p (default) or 1080p, measured on the short side and never larger than the source.
- **Only for media larger than the proxy size:** on by default. Sources whose short side is at or below the proxy resolution get no proxy. With it off, such sources still get one when their codec is in the automatic threshold's heavy list below, because an H.264 proxy decodes faster than 10-bit HEVC at the same size.
- Storage: in the media cache (default), with the project (`<Name> Proxies/`) for portable proxy workflows, or in a **custom folder** on this computer (for example a fast SSD). Project Settings shows how much space this project's proxies use.
- Tiles show a "P" badge when a proxy is ready, a clock mark while it is queued, and a progress ring while one is generating.
- **Lag suggestion:** when EX-017 sees sustained dropped frames and the slow part is decoding a particular file, FreeCut offers "Create proxies for the 3 files slowing playback here", once per section, with **Don't suggest again for this project**.

(The reference app offers three automatic optimisation levels: off, one that acts only on clips that lag, and one that applies every strategy. FreeCut's Off / Manual / Automatic policy plus the lag suggestion cover the same ground; the "every strategy" level would be Automatic plus PM-111's automatic re-wrap, later.)
**Inputs / controls:** The commands, the policy, the resolution, the size rule and the storage choice (with **Choose folder…** for the custom one).
**Behavior / output:**
- **Proxy format:** a video-only H.264 transcode (8-bit 4:2:0, a keyframe every 10 frames for smooth scrubbing, CRF 23, using libx264 under GPL, compatible with FreeCut's license).
  - Proxies keep the source timestamps and frame cadence, including VFR, so proxy frame N is exactly original frame N.
  - Rotation and pixel aspect are baked in, so proxies are display-ready.
  - Audio always comes from the original.
- **Alpha sources:** get an alpha-capable intra proxy. FreeCut's default is VP9 with alpha in WebM, decoded with libvpx; the final codec choice is an open question. Transparency must survive editing (OV-012).
- **Images:** never get proxies, because PM-034's preview downscale covers them.
- **Image sequences:** proxied as video.
- **Automatic threshold** (FreeCut defaults): short side ≥ 1440 px, or codec in {HEVC 10-bit, AV1 ≥ 1440p, 4:2:2 or 10-bit H.264}, or bit rate above 100 Mb/s.
- **No project-file state:** proxies are found by naming convention, `<fingerprint>-<height>.mp4` or `.webm`, in both storage locations.
**Edge cases:**
- **Generation fails:** an error badge. Editing continues with the original.
- **Original changes** (PM-093): the proxy is stale, because the fingerprint key no longer matches, and it is regenerated.
- **Resolution setting changed:** existing proxies are marked stale and regenerated on demand.
- **Disk full:** the job pauses below PM-113's critical threshold and resumes when space returns. If space runs out mid-file anyway, that job fails cleanly and its partial file is removed.
- **Custom or project folder unwritable, missing or full:** proxies go to the media cache instead, with a warning naming the folder. The setting is kept, so they return there once it works again.
- **Cancel one proxy:** from the item menu (**Cancel proxy**) or its task row. The item plays from the original.
- **Policy turned Off with proxies on disk:** they stay until evicted or cleared (PM-074), and turning proxies back on reuses them at once. A one-line note says heavy media may stutter without them.
- **Clear proxies (PM-074):** asks for confirmation with the size to be freed. Projects keep working from the originals and can rebuild proxies at any time.
- **Truncated source:** the proxy covers what decodes, and the mismatch is flagged.
**Undo:** Changing the policy is `SetProjectSettings`. Creating and deleting proxies is n/a (derived data).
**Model notes:** `freecut-media::proxy::{ProxyJob, find_proxy(fingerprint, height)}`, searching the media cache, the project's proxy folder and the custom folder. `ProxyState { None, Queued, Generating(progress), Ready(path), Failed(reason), Stale }` is runtime state in `freecut-app`, never saved. The custom folder is a per-computer preference in app settings (shown in UI-026), not project data, which keeps the rule that proxies leave no state in the project file.

### PM-081 Proxy preview toggle & proxy-only editing — v1
**What the user sees:** A **Use proxies** toggle near the player, on by default whenever any proxy exists, with a key binding proposed by UI. When any visible frame comes from a proxy, the player shows a small "Proxy" tag. Export uses the originals whenever they are online; proxies stand in only for missing originals, after EX's warning (below), or in EX-051's draft export (later).
**Inputs / controls:** The toggle.
**Behavior / output:**
- **Decode choice:** with the toggle on, the decoder uses a Ready proxy, otherwise the original.
- **Original offline but proxy present:** the clip previews from the proxy, with an "Original offline" badge, and editing works normally. Export follows EX-005 and EX-036: the preflight warns that the proxy will stand in, offers **Relink…**, and the export is labeled as containing proxy media.
- **Uses beyond playback:** analysis features may run on proxies when originals are offline (CL, AI).
**Edge cases:**
- **First time proxies are used in a project:** a one-time tip explains that the preview is lighter and slightly softer while exports always use the original files, and where to turn proxies off (the toggle and Project Settings).
- **Toggling during playback:** switches at the next frame.
- **Some items with proxies and some without:** mixed freely.
- **Frame accuracy:** a test compares thumbnails of proxy frame N and original frame N on generated test media.
**Undo:** n/a (view state).
**Model notes:** The source selection lives in `freecut-media`'s decoder factory. The toggle lives in `freecut-app`.

### PM-082 Optimized (intermediate) media — later
**What the user sees:** **Create optimized media** converts hard-to-edit sources (long-GOP 10-bit HEVC, VFR phone footage, interlaced footage) into full-resolution, constant-rate, intra-frame intermediates. Unlike proxies, these are used for both preview and export.
**Inputs / controls:** Per item or per selection, with an estimate of the disk space needed.
**Behavior / output:** Transcodes to a visually lossless intra codec from our FFmpeg build (the codec is to be decided). VFR is conformed to the nearest preset rate. Interlaced media is deinterlaced. The item gains an "Optimized" variant, and decode prefers it.
**Edge cases:**
- **Large size:** files are much bigger, and the space estimate comes first.
- **Audio:** stays on the original's clock.
- **Clip positions after VFR conforming:** may shift by up to half a frame, so the user is warned.
**Undo:** Turning the variant on or off is undoable (`SetMediaVariant { media, before, after }`). Generating the file is n/a.
**Model notes:** Detailed design is deferred. Expected shape: `MediaItem.variant: MediaVariant { Original, Optimized { path: StoredPath, fingerprint: Fingerprint } }` in `freecut-timeline::library`, with the transcode job in `freecut-media` reusing the proxy job infrastructure (PM-080), and the decoder factory choosing the variant (PM-081).

### PM-090 Offline media detection & display — mvp
**What the user sees:**
- Missing media shows a red **Offline** overlay on its tile, over the last cached thumbnail dimmed if there is one.
- Timeline clips show TL's offline stripe with the file name.
- The player shows EX-005's offline slate with the file name, and the audio is silent.
- A banner appears, for example "3 media files are missing", with **Relink…** and **Dismiss** buttons. When every missing file sits under one folder or one volume, the banner names it and says what to do: "12 files on the drive 'Card A' are missing. Connect the drive, or Relink…". Files missing from several places get the count only. When the timeline on screen uses any of them, the banner says so ("4 clips on this timeline are affected").
- From v1, **Recheck media** (Media panel menu and File menu) re-runs the checks for every item at once, for example after copying files back into place.
**Inputs / controls:** The banner buttons and **Recheck media**.
**Behavior / output:**
- **Checks:** when a project opens, every path is checked in the background with parallel status calls, using a 2 s timeout per volume. Each item gets a status:
  - Online
  - Offline (not found)
  - Volume unavailable (its drive isn't mounted)
  - Permission denied
  - Changed (v1, PM-093)
  - Damaged (v1): the file is there but can no longer be decoded, found when PM-093's re-probe or a decode fails. The tile and the player say "This file is damaged", and the item menu offers **Replace with file…** (PM-061) and **Locate…** (PM-091).
- **Editing:** cached probe data keeps clip lengths correct, and offline clips stay fully editable within their known duration (TL).
- **Opening a project with missing media:** the banner appears, and the editor opens normally. FreeCut never blocks with a modal relink prompt on open. (The reference app opens a relink prompt; FreeCut keeps it one click away on the banner.)
- **Export with offline media:** EX-036's preflight lists the items as a warning with **Relink…**. If the user exports anyway, offline parts render as EX-005's slate with silence. PM supplies the status; EX decides the export rules. (The reference app blocks export until missing media is relinked; FreeCut warns instead, so a rough cut can still go out.)
**Edge cases:**
- **Slow network share:** marked "Volume unavailable", not Offline, and retried every 30 s and when a volume mounts. PM-114 explains it in its network-drive notice.
- **A drive is connected again:** its items go back to Online without a prompt, and a toast confirms it ("'Card A' connected. 12 files are back online"). If the drive comes back at a different path (a new drive letter), the items stay offline and the toast offers **Relink…**, which PM-091's folder check usually resolves in one step.
- **Damaged file in the middle of an edit:** the clips keep their length and show EX-005's error slate. Nothing is removed automatically.
- **Thousands of missing files:** still one banner, and the list is virtualized.
- **Same name, different content** (v1): status Changed, not Offline (PM-093).
- **Proxy available** (v1): PM-081 applies.
- **Assets** (v1, PM-042): listed in the same dialog under their own heading.
**Undo:** n/a (derived status).
**Model notes:** `MediaStatus { Online, Offline, VolumeUnavailable { volume }, PermissionDenied, Changed, Damaged { detail } }` is runtime-only. `freecut-media::check(&StoredPath, &Fingerprint) -> MediaStatus`. The banner's "one folder or volume" grouping is a pure function over the offline items' last known paths, with tests for one volume, several volumes, and UNC versus drive-letter forms. The slate itself is EX-005's, drawn procedurally by `freecut-render`. PM notifies status changes (for example a drive coming back) so EX can drop cached frames for that source.

### PM-091 Relink a missing file — mvp
**What the user sees:** The **Relink** dialog, opened from the banner, an item's **Locate…**, the info panel, or **Relink media…** in an offline timeline clip's context menu (which opens the dialog with that clip's item selected). It lists missing items with name, last known path, duration, resolution and usage count, and each row has **Locate…**. The dialog can show **All missing** (default) or **Selected only**. After you locate one file, FreeCut immediately checks that folder, and the same relative layout around it, for the other missing files. If it finds more, it offers to relink them too ("Found 11 more missing files in this folder. Relink them?"). A success toast names the folder used ("Relinked 12 files from …/DCIM/100CANON").
**Inputs / controls:** A file picker per item, the auto-match prompt, the All/Selected switch, and **Done**.
**Behavior / output:**
- Each candidate is validated against the old probe data: same kind; duration within 1 frame (or 100 ms for audio); same display resolution; same audio channel count.
  - **Fingerprint match:** accepted silently.
  - **Metadata matches but the fingerprint differs:** accepted with a note that the file differs, for example because it was re-exported.
  - **Metadata mismatch:** a warning lists the differences, with **Relink anyway** or **Choose another**.
- All relinks made in one dialog session form one undo step.
- Virtual items (PM-041) follow their source automatically.
- From v1, rows for generated items also offer **Regenerate** when the producing feature supports it (PM-041), and rows for sample copies offer **Restore from sample pack** when the installed pack has the same sample (PM-063). Both write a new file and then relink to it like **Locate…**.
**Edge cases:**
- **Picked file is already another item in the library:** FreeCut offers to merge, pointing the clips at the existing item.
- **Relinked file shorter than some clip ranges:** the warning names the clips, and the missing parts show the offline slate.
- **Project made on another OS** (`D:\Footage\a.mp4` opened on macOS): the relative path is tried first (PM-100). Otherwise this dialog appears, with folder search in v1.
- **Picking a folder in the mvp:** an error suggesting Locate. Folder search arrives in v1 (PM-092).
- **The chosen file can't be read at all** (damaged, unsupported, no permission): "Couldn't link this file", with PM-037's reason. The item stays offline.
- **The original is gone for good:** the row's menu offers **Replace with file…** (PM-061, v1), which accepts different content without the validation above, or **Remove from project** (PM-058). Leaving the item offline is also fine; the project keeps working.
- **A relink session with hundreds of items:** the auto-match check runs in the background with a "Linking… 140 of 300" counter (PM-073), and the dialog stays responsive.
**Undo:** `RelinkMedia { changes: Vec<(MediaId, old: StoredPath, new: StoredPath, old_info, new_info)> }`, combined with any `RelinkAsset` steps from the same dialog session (PM-042) into one entry. Undo makes the items point at the old paths again, so they're offline again.
**Model notes:** The op is in `freecut-timeline`. Validation is in `freecut-media` (`validate_candidate(&MediaInfo, &Path) -> Match`). The dialog is in `freecut-app`.

### PM-092 Batch relink by folder search — v1
**What the user sees:** **Search folder…** in the Relink dialog asks for a folder or drive. FreeCut searches it recursively, showing how many files it has scanned, then fills a results table. Each missing item shows:
- ✓ an exact match
- ≈ a metadata match
- ? several candidates, with a dropdown
- ✗ not found

**Relink N** applies the results.
**Inputs / controls:** Folder picker, Cancel, per-row candidate choice, and an optional slow **Match by content** mode, which fingerprints files of the same size regardless of name.
**Behavior / output:**
- **Name matching:** case-insensitive on Windows and macOS, case-sensitive first on Linux.
- **Scan limits:** depth 12; hidden and system folders skipped.
- **Image sequences:** matched by pattern.
- **Tie-breaking:** several candidates are ranked by fingerprint, then metadata.
**Edge cases:**
- **Searching a whole disk:** a warning says it may take a while. Cancelling keeps the matches found so far.
- **No matches:** the items stay offline.
**Undo:** One `RelinkMedia` for the whole batch.
**Model notes:** The walker lives in `freecut-app`, and the fingerprints come from `freecut-media`.

### PM-093 Changed-on-disk detection & refresh — v1
**What the user sees:** If a media file changes after import (for example, you re-export a graphic under the same name), its tile shows a **Changed** badge, and a banner offers **Refresh**. Referenced assets don't need this, because their libraries keep copies (PM-042). While FreeCut is open, files that are deleted, changed or reappear (a drive reconnects) are noticed within about a second.
**Inputs / controls:** **Refresh**, or **Keep previous info**.
**Behavior / output:**
- **Watching:** the `notify` crate (CC0/MIT/Apache-2.0) watches the parent folders of all referenced files, with events debounced over 1 s. On network shares without notifications, FreeCut polls when the window regains focus.
- **Detecting a change:** size or mtime differs → the fingerprint is recomputed. If it's unchanged, nothing happens. Otherwise the status becomes Changed.
- **While Changed:** the decoder already reads the new file. Thumbnails, peaks, proxies and every other content-keyed result (for example AI-001's analysis records) are invalidated through their keys, because the new fingerprint no longer matches (PM-038).
- **The only media change detection:** other areas don't watch files or compare mtimes themselves. They get staleness from this status and the new fingerprint.
- **Refresh:** commits the new probe data. If the duration shrank, clips are handled as in PM-061.
**Edge cases:**
- **A file still growing:** re-probed only after it has been stable for 2 s.
- **Editors that save atomically** (delete followed by create): coalesced into a single change.
- **Thousands of files:** watched per directory, not per file.
- **A file reappears:** it goes back to Online, with no prompt.
- **Re-probe fails after a change** (the file was truncated by an interrupted copy, or overwritten with something unreadable): status Damaged (PM-090), not Changed. **Refresh** is not offered; **Replace with file…** and **Locate…** are.
**Undo:** `UpdateMediaInfo { media, before, after, clip_trims }`. Status changes before Refresh are runtime state, n/a.
**Model notes:** The watcher lives in `freecut-app`. The probe is in `freecut-media` and the op in `freecut-timeline`.

### PM-100 Portable media paths — mvp
**What the user sees:** Usually nothing. Moving a project folder together with its media (keeping the same relative layout), or opening it on another computer, keeps the media online.
**Inputs / controls:** None.
**Behavior / output:**
- **What is stored:** each reference keeps an absolute path (as imported) and a relative path from the project file's folder, computed at every save. The relative path is omitted when the file is on a different volume or more than 8 `..` levels away. The project also records where it was last saved.
- **Resolution order on open:**
  1. If the project file has moved since its last save, try relative paths first. Otherwise try absolute paths first.
  2. Try the other kind.
  3. Look by file name inside `<Name> Media/`.
  4. Mark the item Offline.
- When a fallback finds a file, the resolved location is kept as runtime state next to the item. It is not a document edit: it creates no history entry and doesn't mark the project dirty. Whenever the project is next saved for any reason, the stored absolute path is refreshed from it; if the project is closed without saving, the next open simply resolves it again.
- Relative paths are stored with `/` separators.
**Edge cases:**
- **Windows drive letter changes** (USB drive E: becomes F:): resolved by the relative path when the project sits on the same drive. Otherwise the item is offline and relinking applies.
- **macOS duplicate volume names** (`/Volumes/Card 1` vs `/Volumes/Card 1 1`): handled by the relative path or relinking.
- **UNC path vs mapped drive:** both forms are tried.
- **Non-UTF-8 paths:** PM-021's encoding.
**Undo:** n/a. Fix-ups are not edits.
**Model notes:** `StoredPath { absolute: PathRepr, relative: Option<String> }` is plain data in `freecut-timeline::library` (it never touches the file system), used by `MediaSource` and the ops above. The resolver in `freecut-project` takes a `FileSystem` trait, so it can be tested with a virtual file system. The resolved-location map is runtime state in `freecut-app`.

### PM-101 Collect & package project — v1
**What the user sees:** File > **Package Project…** opens a dialog with:
- Destination folder, and package name (default "<Name> package")
- **Include**: "Media used on the timeline" (default) or "All media in the project"
- **Include generated media** (on)
- **Include proxies** (off)
- **Include LUTs and your own stickers** (on), the referenced assets from PM-042
- **Include imported font files the license allows** (on). It follows TX-007: fonts whose embedding permissions forbid it are skipped, with the reason given. System fonts are listed in the package notes, not copied.
- **Also create a .zip** (off)

The dialog shows the total size and free space, then progress and Cancel.
**Inputs / controls:** The fields above.
**Behavior / output:**
- **Package layout:** `<dest>/<package name>/` holds:
  - a copy of the project with a new id
  - `Media/`, with name collisions resolved by adding " (2)"
  - `Media/Generated/` and `Media/Recordings/` (the generated-media store; media-cache entries are never packaged, because they regenerate on demand, PM-074)
  - `Assets/`, holding the collected LUTs, stickers and font files, which PM-042's resolver checks on open
  - optionally `Proxies/`
  - `PACKAGE.txt`, our own text listing the contents, the FreeCut version, missing items and required system fonts
- **Paths:** the project copy's paths are rewritten as relative paths into the package. Image sequences are copied as folders.
- **Verification:** each copy is checked by size and fingerprint.
- **The original project:** untouched, and editing continues in it.
- **Zip:** uses zip64 for files over 4 GB.
**Edge cases:**
- **Offline items:** the pre-check lists them ("3 files are offline and won't be included") and offers **Relink first**.
- **A file referenced by two items:** copied once.
- **Destination inside the project's media folder or the package itself:** refused.
- **Not enough space:** refused before starting.
- **Cancelled:** the partial package folder, which this job created, is removed.
- **FAT32 destination with a file over 4 GB:** a warning.
- **Long Windows paths:** verbatim paths are used.
- **Several timelines (PM-107, later):** "Media used on the timeline" means media used on any of the project's timelines.
- **Package as a backup:** this is FreeCut's answer to cloud backup. A package on another drive, or in a folder the user syncs with their own tool, can be opened anywhere (PM-106 can add it to the project list).
**Undo:** n/a. The original project is unchanged.
**Model notes:** The pure `PackagePlan::build(&Document, &Options) -> PackagePlan` lives in `freecut-project`, with tests. Execution runs as a `freecut-app` job that calls `freecut-project` I/O.

### PM-102 Trim media to used ranges when packaging — later
**What the user sees:** A packaging option, **Trim media to the parts used (+ handles)**, with handles of 1 s by default (range 0–10 s).
**Inputs / controls:** The option and the handle length.
**Behavior / output:** Used source ranges (after TR's time mapping, including speed ramps and reverse) are merged when they are close together. Each range is cut by stream copy at keyframes, or re-encoded when that isn't possible. Clips are rewritten to point at the trimmed files with offset source times.
**Edge cases:**
- **Hard cases:** VFR media, MP4 edit lists, audio encoder priming, clips that need extra source for transitions (FX handles), and reversed or speed-ramped clips. These are why it's "later".
- **Frame accuracy:** every remapped clip must be frame-identical, checked by comparing rendered frames in tests.
**Undo:** n/a. It works on a copy.
**Model notes:** Detailed design is deferred. Expected shape: a pure `used_source_ranges(&Document, handles: Tick) -> HashMap<MediaId, Vec<TimeRange>>` in `freecut-timeline` (using TR's time mapping, all in ticks), a `PackagePlan` extension in `freecut-project` that rewrites clip source times, and the cutting and re-encoding done by `freecut-media`.

### PM-103 Timeline interchange (OpenTimelineIO) — later
**What the user sees:** File > Export Timeline As… OpenTimelineIO (`.otio`), and File > Import Timeline… for `.otio` files, so cuts can move between FreeCut and other editors that support OTIO. Dropping a timeline file on the window offers the same import. After OTIO, two more import formats can follow within this feature: Final Cut Pro XML (`.fcpxml`) and the older XML interchange format many editors still write (`.xml`), both mapped from their public documentation.

Reference: the reference app's import area has a separate timeline tab that accepts XML, FCPXML and AAF by file picker or drop, lists recently imported timeline files, and reports timeline-import failures separately from media failures.
**Inputs / controls:** File dialogs, drop, a target choice (**New project** or, once PM-107 exists, **New timeline in this project**; before that, **New tracks in this project**), and an import report.
**Behavior / output:** Media references (paths and ranges), tracks, clips, gaps and markers are mapped. Effects, text and transforms are not mapped, and the report says so. An import creates a new project, a new timeline (PM-107) or new tracks in the current one. File > Import Timeline ▸ Recent lists the last 10 timeline files imported on this computer (app data).
**Edge cases:**
- **Rate mismatches:** converted exactly through ticks.
- **Missing media in the OTIO file:** offline items.
- **Unsupported schema features:** skipped and listed.
- **AAF files:** not imported. AAF is a large binary container format with no implementation we have vetted for FreeCut's license; the message suggests converting it to OTIO with OpenTimelineIO's own tools first.
- **A timeline file that fails to parse:** its own report, "Couldn't import the timeline: <reason>", separate from PM-037's media report, with **Copy report**.
- **A timeline file passed to Import Media (PM-030):** "This is a timeline file. Import it as a timeline instead?"
**Undo:** Importing into the current project is one compound op (`ImportMedia` plus TL placement, or PM-107's `ImportTimeline`). Export is n/a.
**Model notes:** An OTIO serde mapping, written ourselves from the public OTIO schema, lives in `freecut-project::interop`. FCPXML and XML mappings, if added, live beside it and produce the same intermediate structure, so one importer builds the FreeCut timeline. Each mapping has fixture tests with files we wrote ourselves.

### PM-104 Deleted projects (30-day hold) — v1
**What the user sees:** A **Deleted projects** tab on the start screen (PM-001). Projects removed with PM-018's **Move to Trash** wait here before they go to the system Trash. Each row shows the cover, name, original location, the date it was deleted, "Kept until <date>", and the size held. Actions: **Restore**, **Delete now** and **Empty**. When the tab is empty it says "Nothing here. Deleted projects are kept for 30 days, then moved to the system Trash."

Reference: the reference app keeps deleted projects in a trash view with restore, permanent delete and clear-all, and purges items after 30 days. FreeCut adds one difference: what leaves the hold goes to the system Trash, never straight to oblivion.
**Inputs / controls:**
- Row selection (`Mod`/Shift-click), **Restore**, **Delete now**, **Empty**.
- **Delete now** and **Empty** confirm first, naming the project or the count, and say the files go to the system Trash.
- Preference **Keep deleted projects for:** 7, 14, 30 (default), 60 or 90 days, or **Send to the system Trash right away** (PM-018's plain behavior, without a hold).
**Behavior / output:**
- **Hold:** Move to Trash moves the `.freecut` file into `deleted/<project-id>/` in app data, with a record `{ project_id, name, original_path, deleted_at, bytes, media_folder_to_os_trash: bool, volume }`. Project files are small, so this is a rename on the same volume or a quick verified copy on another one.
- **Media stays put by default.** The project's `<Name> Media/` and `<Name> Proxies/` folders are not moved, so a restored project finds its media again at once. If PM-018's "Also move '<Name> Media' to Trash" box was ticked, that folder goes straight to the OS Trash, because it can be large and may sit on another volume; the record notes it.
- **Retention:** at startup and once a day, records older than the retention are handed to the OS Trash with the `trash` crate (PM-018). FreeCut never erases them itself. The project's backups (PM-016) and cover (PM-019) are kept at least as long as the held project, so a restore brings them back too.
- **Restore** moves the file back to its original path and adds it to the registry (PM-017), so it reappears under Projects. If the original folder is gone, or a different project now uses the name, a save dialog asks where to put it, suggesting " (2)". If the media folder had gone to the OS Trash, a notice says to put it back from there; once it is back in place, relative paths (PM-100) or PM-091's relink find it.
- **Delete now** and **Empty** hand the selected or all held projects to the OS Trash immediately.
- The hold is per user and per computer, because it lives in app data.
- There is no media trash: removing media from a project is undoable (PM-058) and never deletes files, and PM-074's clean-up moves unused files to the OS Trash.
**Edge cases:**
- **Project open in another instance:** can't be moved to the hold (PM-018's lock rule).
- **The open project:** saved and closed first (PM-018).
- **No OS Trash available:** **Delete now** and **Empty** are disabled with PM-018's explanation. Expired items stay in the hold, marked "Kept: no system Trash available", with **Reveal** so the user can remove them in the file manager.
- **App data on a full disk:** the move into the hold fails with PM-012's reason, and the project stays where it was.
- **Two held projects with the same name:** told apart by original location and deletion date.
- **Held file removed by hand from app data:** its row disappears on the next listing, with a line in the log.
- **Opening a held project directly** (double-click in the file manager): read-only with a **Restore** banner (PM-013).
- **Retention shortened:** items past the new limit are listed in a confirmation before they are handed to the OS Trash.
- **Restoring a project saved by a newer FreeCut:** restore works; opening follows PM-021's read-only rule.
- **Restoring onto a drive that isn't connected:** the save dialog opens at the default projects folder instead.
**Undo:** n/a (file operations outside any project). Restore is the way back.
**Model notes:** `freecut-project::trash::{hold(&Path, HoldOptions) -> Result<HeldProject, TrashError>, list() -> Vec<HeldProject>, restore(id, dest: Option<PathBuf>), release(id) /* to the OS Trash */, expired(&[HeldProject], retention, now) -> Vec<ProjectId>}`. `expired` is pure and tested with a fake clock. `TrashError` (thiserror): `Locked`, `NoSystemTrash`, `DestinationExists`, `Io`. The tab lives in `freecut-app::home` inside UI-016's frame; UI-017 places the actions.

### PM-105 Project folders on the start screen — later
**What the user sees:** Under **Projects**, sub-folders of the default projects folder appear as folder cards above the project cards, with a breadcrumb ("Projects › Clients › Acme"). **New folder**, **Rename**, **Move to folder…** and **Delete folder** are in the card menus. Projects and folders can be dragged onto folder cards or breadcrumb segments, and `Mod+X` then `Mod+V` on cards moves them.
**Inputs / controls (FreeCut defaults):**
- Folder name: 1–120 characters, trimmed. Characters that file names can't hold are refused inline, because a folder name is a real file name.
- Nesting: at most 10 levels below the projects folder (the same depth the reference app uses), which also keeps Windows paths short.
- The search field (PM-001) gains a scope: **This folder** (default) or **All folders**.
**Behavior / output:**
- Folders are real directories on disk. The start screen scans only the folder being viewed, one level at a time, like PM-001's scan. **All folders** search walks the tree on a background thread, capped at 2,000 projects like PM-001.
- **Moving a project** moves its `.freecut` file and, when present, its `<Name> Media/` and `<Name> Proxies/` folders together, so relative paths keep working (PM-100). Inside one volume this is a rename. The registry (PM-017) is updated with the new path.
- **Delete folder:** an empty folder is moved to the OS Trash at once. A folder holding projects sends each project through PM-104, so each one stays restorable, and then trashes the emptied folder. A folder that also holds files that aren't FreeCut projects or their media folders is refused, listing those files, so FreeCut never trashes the user's other files.
**Edge cases:**
- **Name already used in the parent:** refused inline.
- **Moving a folder into its own sub-folder:** refused.
- **Depth limit reached:** **New folder** is disabled there, with the reason.
- **Project open in another instance:** can't be moved (lock).
- **The open project:** can be moved after FreeCut saves it and releases its lock; it reopens from the new place.
- **A project with the same name already in the target:** **Keep both** (default) adds " (2)" to the file and its media folder, or **Cancel**.
- **Folder deleted or renamed in the file manager while shown:** "This folder no longer exists", and the view moves to the nearest existing parent.
- **Projects whose media lives elsewhere:** only the project's own sibling folders move; media linked from other places stays where it is.
**Undo:** n/a (file operations). Moving back is another move.
**Model notes:** `freecut-project::library_fs::{create_folder, rename_folder, move_project(src, dest_dir) -> Result<MovedProject, MoveError>, plan_move(&Path) -> MovePlan}`. `plan_move` decides which sibling folders travel and is pure, with tests. UI in `freecut-app::home`.

### PM-106 Add projects from a folder — later
**What the user sees:** Start screen ▸ **⋯** ▸ **Add projects from a folder…** picks a folder or a single `.freecut` file (an old projects folder, a backup drive, a package from PM-101, a colleague's handover). FreeCut scans it and lists what it found: name, location, last modified, format, and a status (**Ready**, **Already in your list**, **Newer version**, **Damaged**, **Empty**). The user ticks rows and chooses **Add to list** or **Copy into my projects folder**. Copying shows progress with Cancel and ends with a summary.

Reference: the reference app can import a single project file, or scan another installation's project folders and bulk-import them, with per-project status, a progress dialog and replace / keep-both choices on name clashes.
**Inputs / controls (FreeCut defaults):**
- Folder or file picker. The scan is recursive to a depth of 6.
- Row checkboxes. **Ready** rows start ticked; **Empty** rows start unticked.
- Mode: **Add to list** (default: the files stay where they are and join the registry, PM-017) or **Copy into my projects folder**.
- Name clashes in copy mode: **Keep both** (default, adds " (2)"), **Replace** (the existing project first goes to Deleted projects, PM-104; nothing is overwritten in place) or **Skip**.
**Behavior / output:**
- Only headers are read (PM-001). "Already in your list" is detected by project id and path.
- Copy mode copies each project file with its `<Name> Media/` folder and verifies the copies (size, and PM-038 fingerprints for media). Copied projects find their media through relative paths (PM-100). A copy keeps its project id unless that id is already in the list, in which case it gets a new one (PM-001's duplicate-id rule).
- A PM-101 package folder is recognized by its `PACKAGE.txt` and copied as a whole.
**Edge cases:**
- **Nothing found:** "No FreeCut projects in this folder".
- **Other editors' project files in the folder:** ignored. Importing other editors' native project formats is out of scope (see Out of scope); timelines from other editors come in through PM-103's interchange formats.
- **Empty project** (no clips and no media): shown as Empty and can still be added.
- **Damaged file:** listed as Damaged, can't be added; the row offers **Show in folder**.
- **Newer format:** can be added to the list; it opens read-only (PM-021).
- **Projects that link media outside their folder:** in copy mode those files stay linked by absolute path, and the summary says how many projects are affected.
- **Not enough space for the copy:** refused before starting (total plus a 1 GB margin, as in PM-039).
- **Cancel or a failure part-way:** projects already copied stay; the partly copied one is removed. The summary lists failures with **Retry**.
**Undo:** n/a (file operations).
**Model notes:** The scan and the copy plan are pure functions in `freecut-project` (tested on a virtual file system, as in PM-100). The job runs in `freecut-app` through PM-073.

### PM-107 Multiple timelines in one project — later
**What the user sees:** A project can hold several timelines, for example a long cut and a vertical short made from the same footage. A **Timelines** section at the top of the Media panel lists them, and a dropdown next to the timeline breadcrumb switches between them. Commands: **New timeline**, **Duplicate timeline**, **Rename**, **Delete**, **Set as primary** and **Timeline settings…**. The primary timeline has a star; it drives the start-screen card's duration and cover (PM-001, PM-019) and is the one a newly opened project shows first. Each timeline keeps its own playhead, zoom and scroll.

Reference: the reference app lets one project hold several timelines with automatic numbered names, renaming, deletion with confirmation, a pinned primary timeline, a per-timeline settings dialog and a per-project maximum.
**Inputs / controls (FreeCut defaults):**
- Names: 1–120 characters, trimmed. New timelines are named "Timeline 2", "Timeline 3" and so on; the first one is named "Timeline 1" when a second one is created. A duplicate name under the same project gets " (2)".
- At most 100 timelines per project. At the limit, **New timeline** is disabled with "A project can hold up to 100 timelines".
- **Timeline settings…** opens PM-011's dialog scoped to that timeline. Canvas, frame rate, background, default fit and still duration can differ per timeline; the audio sample rate stays project-wide because there is one mixer output (PM-007).
- **Delete** asks for confirmation, naming the timeline and how many clips it holds, and says the media stays in the project.
**Behavior / output:**
- **Model:** the project holds a list of root timelines, each with an optional settings override; a timeline without overrides inherits the project defaults (PM-003 to PM-010). TL owns each sequence's content; PM owns the list, names, order, the primary choice and the overrides. TL-003's "one timeline per project" changes when this ships.
- **Per-timeline settings** go through PM-011's flow, including conform, for that timeline only. The dialog asks whether a change applies to **This timeline** or to **Project defaults** (which all timelines without overrides follow).
- **One undo stack** across timelines (TL-043). Undoing an edit made in a timeline that isn't on screen switches to it first and highlights the change, so the user sees what was undone (the same idea as TL-040 for compound clips).
- **Media usage** (PM-055) counts clips on every timeline, and **Find on timeline** steps through them timeline by timeline. Removing used media (PM-058) lists the affected clips per timeline.
- **Timeline inside a timeline:** dragging a timeline from the list onto another timeline inserts a compound clip (TL-040) holding a copy of that timeline's content. Later edits to the source timeline don't propagate, because TL-040 lets each nested sequence belong to one compound clip; the clip's tooltip says which timeline it was copied from and when. A timeline can't be dropped into itself.
- **Import timeline from another project…:** picks a `.freecut` file and one of its timelines, then copies that timeline and the media items it uses into this project as a new timeline. Items whose fingerprint (PM-038) matches an existing item reuse it. The source is read the way TL-056 reads it (read-only, no lock, migrations in memory). TL-056 inserts another project as a compound clip inside the current timeline; this command keeps it as a timeline of its own.
- **Export** (EX-021) exports the timeline on screen. Packaging (PM-101) and Remove unused (PM-058) consider all timelines.
- The project header (PM-021) stores the primary timeline's duration, canvas and frame rate.
**Edge cases:**
- **Deleting the primary timeline:** the next one in the list becomes primary. Deleting the only timeline is refused, because a project always has at least one.
- **Delete is undoable,** so the confirmation is the only extra step. (The reference app says deletion removes everything on the timeline; in FreeCut undo brings it back.)
- **A timeline with compound clips:** duplicating or importing deep-copies its nested sequences (TL-040).
- **Older FreeCut opening a project with several timelines:** this ships with a format bump, so older versions open it read-only (PM-021).
- **Per-timeline frame rate different from the project default:** allowed. Media and proxies are shared and unaffected.
- **Backups, recovery and packaging:** cover the whole project file, so every timeline.
**Undo:** `CreateTimeline`, `DuplicateTimeline`, `RenameTimeline`, `DeleteTimeline { snapshot, index, was_primary }`, `SetPrimaryTimeline { before, after }`, `SetTimelineSettings { timeline, before, after }` (compounded with TL's conform when the rate changes) and `ImportTimeline { items, timeline }`. Switching timelines is view state.
**Model notes:** `Project { timelines: Vec<TimelineEntry { seq: SeqId, name: String, settings: Option<SettingsOverride> }>, primary: SeqId }` in `freecut-timeline`, with TL's `Timeline { root, sequences }` generalized to several roots. A format migration (PM-021) wraps the single root of older files as "Timeline 1". The import-from-project step reuses TL's `import_sequence` (TL-055, TL-056), which re-keys every id, and PM-030's cross-project media path (no re-probe). TL-003 and TL-043 need matching updates before this ships.

### PM-108 Color clips — v1
**What the user sees:** A built-in **Black** tile in the Media tab, next to the Samples sub-category (PM-063), and **Color clip…** in the Import menu (PM-030) and the Media panel's context menu. A color clip is a plain, full-frame color that can be placed like a still: as filler, as a backdrop under picture-in-picture, or as a base to fade from or to. Its tile shows the color with a "Color" badge.

Reference: the reference app offers a built-in plain black clip in its media area.
**Inputs / controls:**
- **Color clip…** opens a color picker (sRGB, hex field, eyedropper; the same control as PM-009) and a name field. The default name describes the color ("Black", "White", or "Color #1E90FF" for others).
- The item's menu adds **Change color…**.
- Color clips are opaque. Partial transparency comes from TR's opacity on the clip.
**Behavior / output:**
- A color clip is a virtual media item with no file: `MediaSource::Solid { color }`. Nothing is probed, decoded, cached or packaged; `freecut-render` draws it.
- It always fills the canvas of the sequence it is used in, so it fits any canvas and any later canvas change. TR's transform, crop and opacity apply to it like to any visual clip.
- It has no length of its own. It is placed with the still duration (PM-010) and can be trimmed to any length, like a still.
- Dragging the Black tile reuses an existing black item in the project when there is one, so the panel doesn't fill with copies. **Color clip…** always creates a new item.
- **Change color…** changes every clip that uses that item, in one step. To use two colors, make two items.
**Edge cases:**
- **Never offline:** there is no file, so relink, packaging and missing-media checks skip it.
- **Auto mode (PM-006):** a color clip doesn't trigger matching, because it has no size of its own.
- **Replace with file (PM-061):** allowed, treated like replacing a still.
- **Shape filter (PM-054):** follows the canvas of the timeline on screen.
- **Changing the color during playback:** applies on the next frame.
- **Export with alpha (EX-048, later):** color clips stay opaque.
**Undo:** Creating one is `ImportMedia` with a solid source (labeled "Add color clip"). **Change color…** is `SetSolidColor { media, before, after }`. Placing it is TL's op.
**Model notes:** `MediaSource::Solid { color: Rgb8 }` joins PM-041's `File`, `ImageSequence` and `AudioOf` variants in `freecut-timeline::library`, with `MediaOrigin::BuiltIn` as a new origin variant (there is nothing to regenerate, so it isn't `Generated`). Its `MediaInfo` is synthesized: kind Image, no duration limit, size taken from the sequence canvas at render time. It has no `Fingerprint`, because it has no file content; caches never key on it.

### PM-109 Live and motion photos — later
**What the user sees:** Some phone photos carry a short video of the moments around the shot: either as a separate short video file next to the photo, or embedded inside the photo file. When an import contains such photos, FreeCut asks once per batch: **Photo only** (default), **Short video only**, or **Both**. A "Remember my choice" box sets the preference. Imported short videos get a "Live" badge.

Reference: the reference app asks on import whether a live photo should be used as a still or as a short moving clip.
**Inputs / controls:** The prompt. Preference **Phone live photos:** Ask (default), Photo only, Short video only, Both.
**Behavior / output:**
- **Paired files:** a photo (HEIC or JPEG) and a short video (MOV) in the same import batch with the same base name and the same capture identifier in their metadata. When the identifier is missing but the names match and the video is 4 s or shorter, the pair is offered only after the user confirms the count in the prompt.
- **Embedded video:** a JPEG or HEIC whose XMP metadata declares an embedded video with its offset and length. The video part becomes a virtual item, `MediaSource::Embedded { file, offset, len }`, read in place through a byte-range input in `freecut-media`; no file is written. It goes offline and relinks together with its container file (PM-041's rule for virtual items).
- **Both:** two items side by side in the same bin, named "<name> (photo)" and "<name> (live)".
**Edge cases:**
- **Only one half in the batch** (the video was imported earlier): no prompt; each file imports as what it is.
- **HEIC not decodable in this build (PM-036):** only the video part is offered.
- **Embedded offset or length invalid:** imported as a photo only, with a log line.
- **Duplicate detection (PM-038):** an embedded video's fingerprint is computed over its byte range, so the same live photo imported twice is recognized.
- **Rotation:** the video's own display matrix applies (PM-035); it may differ from the photo's EXIF orientation.
**Undo:** `ImportMedia`.
**Model notes:** `detect_live_pairs(&[ProbedFile]) -> Vec<LivePair>` is pure, with table tests. XMP is parsed with our own small reader of the public XMP properties (an XML crate such as `quick-xml`, MIT). `MediaSource::Embedded` lives in `freecut-timeline::library`. The exact metadata fields are taken from the vendors' public format documentation, to be confirmed before implementation (open question).

### PM-110 Media color labels — later
**What the user sees:** Item menu ▸ **Label** ▸ one of 8 colors, or **None**. A labelled tile shows a colored strip along its edge, list view (PM-051) gets a Label column, PM-054's status filter gains **Label** (multi-select), and PM-053 can sort by label. **Clear labels** removes them from the selection.

Reference: the reference app's UI labels include about 16 media color tags with a clear-all action, apparently not yet shown to users.
**Inputs / controls:** The menu and filter. The 8 colors are TL's shared palette (TL-037, TL-045, TL-049), so a media label and a clip label look the same. (FreeCut uses 8 rather than about 16, so the shared palette stays easy to tell apart.) Preference **Clips take their media's label:** off by default; when on, clips placed later from a labelled item start with that label (TL-045).
**Behavior / output:** A per-item document field. Labels never affect output.
**Edge cases:**
- **Mixed selection:** choosing a color sets it on all selected items.
- **Offline, generated and color-clip items:** can be labelled.
- **Setting the color an item already has:** no history entry.
- **Bins:** not labelled (FreeCut default).
**Undo:** `SetMediaLabel { items, before, after }`.
**Model notes:** `MediaItem.label: Option<PaletteColor>`, using TL's palette type, in `freecut-timeline::library`.

### PM-111 Re-wrap hard-to-seek files — later
**What the user sees:** Some files scrub badly because of how they are packaged, not because of their codec: camera transport streams (`.mts`, `.m2ts`, `.ts`), MKV or WebM without a seek index, FLV, and AVI without an index. FreeCut can **re-wrap** them: copy the same compressed audio and video into a new container with a complete index, without re-encoding, so seeking becomes fast. The item menu has **Re-wrap for faster scrubbing**, Project Settings has a policy **Re-wrap automatically: Off (default) / Hard-to-seek files**, and re-wrapped items show an "R" badge.

Reference: the reference app can re-wrap some sources into a MOV container without re-encoding, as a background task with its own folder and size readout, and suggests it when such files lag.
**Inputs / controls:** The command, the policy, and a suggestion when EX-017 sees long seek stalls on such a file ("Re-wrap 2 files for smoother scrubbing?"), with **Don't suggest again for this project**.
**Behavior / output:**
- A background stream copy (PM-073) into MP4, or MOV or MKV when MP4 can't carry a stream. The estimated size (about the source size) is shown before it starts.
- The re-wrapped file is a media-cache entry (PM-074, category "Re-wrapped media"), keyed by fingerprint plus container version, because it is regenerable bit for bit.
- Preview decoding prefers the re-wrapped file, chosen by the same decoder factory as proxies (PM-081). **Export always reads the original**, like with proxies, so re-wrapping can never change an export.
- Timing: the re-wrap keeps every packet's timestamp relative to the first presentable frame (PM-035), and a test checks that frame N decodes identically from both files.
**Edge cases:**
- **Stream the target containers can't carry:** "Can't re-wrap this file", with a suggestion to create a proxy (PM-080) instead.
- **Subtitle and data streams:** dropped from the re-wrap, which is preview-only; the original keeps them.
- **Original changes (PM-093):** the fingerprint no longer matches, so the re-wrap is stale and is redone on demand.
- **Re-wrapped files cleared** (PM-074): playback falls back to the original, which may scrub slowly again.
- **Low disk space:** the job pauses (PM-113).
- **File already well indexed:** the command is still allowed but the suggestion never offers it.
**Undo:** n/a (cache). The policy is `SetProjectSettings`.
**Model notes:** `freecut-media::rewrap::{RewrapJob, find_rewrap(fingerprint)}` reuses the proxy job infrastructure (PM-080). The "hard to seek" test (container kind plus missing index) is a pure function over probe data. This differs from PM-082, which re-encodes and is used for export too.

### PM-112 Change the projects folder & move existing projects — later
**What the user sees:** Preferences ▸ Storage (UI-026) shows every place FreeCut keeps things on this computer: the projects folder, the media cache (PM-074), the custom proxy folder (PM-080), app data (backups, recovery, Deleted projects) and logs, each with **Reveal**. The projects folder has **Change…**. After a new folder is chosen, FreeCut asks: **Use it for new projects only** (default) or **Move my projects there**. Moving shows progress (projects done of total, size, time left) and **Cancel**, and ends with a toast naming the new folder.

Reference: the reference app lets the user pick separate folders for projects, downloaded assets, presets, proxies, re-wrapped files, the media cache and the render cache, and can move the projects folder with progress.
**Inputs / controls:** **Change…** and the two-way choice. For the cache and proxy folders, see PM-074 and PM-080.
**Behavior / output:**
- **New projects only:** sets the default location (PM-020). Existing projects stay where they are. Any project in the old folder that isn't in the registry yet is added to it (PM-017), so none disappears from the start screen when the scan moves to the new folder.
- **Move:** each project directly in the old folder (and in PM-105's sub-folders, when that exists) is moved together with its `<Name> Media/` and `<Name> Proxies/` folders, so relative media paths keep working (PM-100). On the same volume this is a rename. On another volume each file is copied, verified (bytes for project files, size and fingerprint for media), and then the originals are moved to the OS Trash, never erased.
- Projects outside the old folder (registry-only entries) are not moved.
- The registry (PM-017), Deleted projects (PM-104) and backup ids are updated; nothing in the projects changes.
**Edge cases:**
- **The open project:** skipped and listed ("Close it and run Move again"), because moving it means releasing its lock.
- **A project open in another instance:** skipped and listed.
- **New folder inside the old one, or the other way round:** refused.
- **Destination not empty:** allowed; name clashes get " (2)".
- **Not enough space:** refused before starting (total plus a 1 GB margin).
- **Destination unwritable:** refused with PM-012's reason text.
- **Destination on an external or network drive:** PM-114's note, then allowed.
- **FAT32 destination with a file over 4 GB:** that project is skipped and listed.
- **Cancel:** projects already moved stay in the new folder; the one in flight is rolled back (its partial copy is removed).
**Undo:** n/a (file operations). Moving back is another move.
**Model notes:** `freecut-project::relocate::plan(&Path, &Path) -> RelocatePlan` is pure and tested on a virtual file system; execution is a PM-073 job in `freecut-app` that calls `freecut-project` I/O. It reuses PM-105's `plan_move`.

### PM-113 Disk-space monitoring & low-space protection — v1
**What the user sees:** When a drive FreeCut writes to runs low, a warning names the drive and says how much to free: "Only 3.1 GB free on 'Macintosh HD' (project, cache). Free at least 1.9 GB to keep working smoothly", with **Storage settings…** (UI-026) and **Clean up project…** (PM-074). The warning stays in the status line until there is enough space again.
**Inputs / controls (FreeCut defaults):** Preferences ▸ Storage: **Warn below** 5 GB (1–100 GB) and **Pause background work below** 1 GB (0.5–20 GB, always lower than the warning value).
**Behavior / output:**
- **Watched volumes:** the ones holding the open project file and its media store (PM-041), the media cache (PM-074), the custom proxy folder (PM-080) and app data (backups, recovery, Deleted projects). Each volume is checked once even when several of these share it.
- **When:** every 30 s while a project is open, before each job that writes large files starts, and when such a job ends. The free-space query runs off the UI thread.
- **Below the warning value:** the warning above, and the media cache limit shrinks temporarily (PM-074).
- **Below the pause value:** jobs that write large files pause: proxies, re-wraps, copies into the project, packaging, pre-renders, filmstrips and thumbnails (PM-073 shows "Paused: low disk space"). They resume by themselves once space is above the warning value again.
- **Never paused:** saves, autosave, recovery snapshots and backups, because they protect work and are small. A backup that would itself push the volume below the pause value is skipped (PM-016).
- **Hysteresis:** the warning clears only when free space exceeds the warning value by 10 %, so it doesn't flicker. After a clean-up that didn't free enough, it stays and says how much is still needed.
- **FreeCut never refuses to start or to edit** because of free space. (The reference app requires about 10 GB free and won't run below that; FreeCut warns and protects instead, because saving a project needs only kilobytes.)
- Recording (PM-043, PM-115) uses its own thresholds, and exports check their destination in EX-036 and EX-037.
**Edge cases:**
- **Free space unknown** (some network shares): no warning; the log notes it.
- **Space runs out in the middle of a save:** PM-012's atomic write fails cleanly and the file on disk stays intact.
- **New project when the default location's volume is full:** PM-002 offers another location.
- **Space freed by something else:** paused jobs resume on the next check.
- **Read-only volumes:** not watched.
- **Very small drives** (for example a 64 GB laptop disk): the thresholds are user-adjustable, and the defaults never exceed 10 % of the volume's size.
**Undo:** n/a.
**Model notes:** A pure `SpaceMonitor` state machine (thresholds, hysteresis, which volumes, what to pause) in `freecut-app`, tested with fake readings. The query uses a cross-platform free-space crate such as `fs4` (MIT/Apache-2.0). Jobs ask a shared `StorageGate` before writing.

### PM-114 Storage-location advisories — v1
**What the user sees:** Short, dismissible notes when where files live is likely to slow editing down:
- After importing media from a network share, a slow external drive, a memory card or an online-only cloud placeholder: one toast per batch, "12 files are on a network drive. Editing may stutter", with **Copy into project** (PM-039), **Create proxies** (PM-080) and **Not now**.
- When the project file, the media cache or the proxy folder is placed on an external or network drive: a one-time note per location that editing may be slower there.
- When a network drive stops responding during editing: a banner, "The network drive '<name>' isn't responding. Its clips show as offline until it is back", with **Show files** and a **Don't show again for this project** box. Its items become Volume unavailable (PM-090).
- When EX-017 reports dropped frames on a laptop running on battery: its hover list adds "On battery power. Plug in for full speed".

Reference: the reference app warns when projects, its asset cache or export output sit on an external drive, shows a dialog with a per-project opt-out when network storage is unreachable, and lists tips such as turning on proxies, copying media to the local disk and plugging in the laptop.
**Inputs / controls:** **Don't show again** on each note (per project for media notes, per volume for location notes), and Preferences ▸ Storage ▸ **Storage hints:** On (default) / Off.
**Behavior / output:**
- **Volume kind:** internal, external (removable), network, optical or unknown, from the OS (the `sysinfo` crate's disk list, MIT, plus the mount's file-system type for network mounts such as SMB, NFS or WebDAV). Online-only cloud placeholders are flagged where the OS exposes that state.
- These notes never move or copy anything by themselves; copying only happens when the user picks **Copy into project**.
- PM supplies a "slow storage" reason to EX-017's list of causes when decode waits on disk reads.
- A volume the user dismissed is remembered by its volume id in app settings.
**Edge cases:**
- **Fast external SSD:** flagged once; **Don't show again** silences that volume for good.
- **Unknown volume kind:** no note.
- **Many files from several slow places in one import:** one toast with the total and the places listed in its details.
- **Network drive back:** the banner disappears and items go Online (PM-090).
- **Export to an external drive:** EX-023 owns the export folder; this area only adds the same one-time note.
**Undo:** n/a.
**Model notes:** `VolumeKind` classification in `freecut-app::platform` behind `cfg`, with no `unsafe` (safe crates only). Per-project dismissals are view state (PM-021); per-volume dismissals are app settings.

### PM-115 Screen recording into the project — later
**What the user sees:** The recording window of PM-043 gains a **Source** switch: **Camera**, **Screen**, or **Screen + camera**. Screen capture can take a whole display, a region dragged out on screen, or one window where the OS offers that. With a camera, a small camera picture can be placed in a corner of the recording. While recording, FreeCut's window steps aside and a small floating bar shows elapsed time with **Pause**, **Stop** and **Retake**; where the OS allows, the bar is left out of the capture. When the user stops, the takes appear in the Generated bin with a screen badge.

Reference: the reference app records the full screen or a dragged region, optionally with a webcam, puts camera and screen on separate tracks or one, lets the user choose camera, microphone with input level and cursor style, pauses and resumes, confirms before a retake, has a global start/stop shortcut, and warns that system audio needs a recent macOS.
**Inputs / controls (FreeCut defaults):**
- **Area:** Whole display (default: the display FreeCut's window is on), Region (drag out a rectangle, rounded to even pixels), or Window.
- **Frame rate:** 30 fps (default), 24 or 60.
- **Size:** display pixels, scaled down so the long side is at most 3840.
- **Cursor:** Show (default) or Hide. A click highlight ring is offered only where the capture backend itself reports clicks; FreeCut doesn't install a system-wide input monitor for it.
- **Microphone:** AU-027's device list, input level meter and gain (as in PM-043), or **No microphone**.
- **System audio:** off by default; disabled with the reason where the OS can't capture it.
- **Camera (Screen + camera):** PM-043's camera controls, plus **Camera on its own track** (default) or **Camera inside the screen recording**, a corner (bottom-right default) and a size (25 % of the width default).
- **Countdown:** as PM-043. **Add to timeline at playhead:** off.
- Start and stop from the recording window or the floating bar. A system-wide shortcut that works while FreeCut is in the background is an open question, because it needs OS-level registration.
**Behavior / output:**
- **Capture** runs in `freecut-media::capture` on background threads. The backend per OS is an open question: FFmpeg's screen inputs (AVFoundation on macOS, Desktop Duplication or GDI on Windows, X11 on Linux) or the OS's newer capture APIs through safe crates. On Linux Wayland sessions the desktop portal shows its own picker and permission dialog.
- **Files:** the screen goes to `<Name> Media/Recordings/Screen 2026-10-12 14.32.05.mp4` (H.264, libx264, 8-bit 4:2:0, CRF 18, fragmented MP4 as in PM-043). The microphone is the file's main audio stream; system audio, when on, is a second stream (AU's multi-stream handling, PM-040). With the camera on its own track, the camera goes to a separate file in PM-043's format; both share the capture clock so they line up exactly.
- **Screen frames arrive only when something changes,** so the file is written as VFR, and PM-035's VFR handling applies.
- **On stop:** each file is probed and imported as generated media (`GeneratedKind::ScreenRecording`, and `CameraRecording` for the camera file). With **Add to timeline at playhead**, the screen clip goes on the main track at the playhead and the camera clip on a track above, starting at the same time, sized and placed in the chosen corner with TR's transform, all in one compound step.
- **Pause, Retake and crash recovery** work as in PM-043.
**Edge cases:**
- **Screen-recording permission missing:** the window gives the OS's steps (macOS: allow FreeCut under Screen Recording in System Settings, then restart FreeCut, which macOS requires; Windows: no extra permission; Wayland: the portal dialog) and offers **Open System Settings** where possible. Camera and microphone permissions follow PM-043 and AU-027.
- **System audio not available** (older macOS versions, Linux without a monitor source): the option is disabled with the reason, and the user can record without it.
- **Display unplugged or its resolution changed mid-take:** recording stops and the take so far is kept.
- **Protected content on screen** (copy-protected video players): appears black in the recording; that is the OS's behavior, and the window says so in its help text.
- **Displays with different scaling:** a region stays on one display.
- **HDR display:** captured as SDR; CL-002's conversion notice applies if the file is tagged HDR.
- **Low disk space:** PM-043's thresholds and time-left estimate.
- **Very long takes:** limited only by disk space.
**Undo:** `ImportMedia`, or a compound with TL's placement labeled "Add recording". Undo never deletes the files (PM-041).
**Model notes:** `freecut-media::capture` gains `list_displays()`, `ScreenSource { Display(id), Region { display, rect }, Window(id) }` and `ScreenCaptureConfig`. `CaptureError` adds `ScreenPermissionDenied`, `SystemAudioUnsupported` and `DisplayLost`. `GeneratedKind::ScreenRecording` is added to PM-041's list. The placement plan (screen on main track, camera above in a corner) is a pure function in `freecut-timeline` with tests. The floating bar and window exclusion live in `freecut-app::platform`.

## Interactions with other areas
- **Timeline & core editing ([timeline-editing.md](timeline-editing.md), TL):**
  - TL owns ticks, `FrameRate`, sequences, clips, selection and the undo stack.
  - PM supplies the settings that sequences reference (canvas, rate, sample rate, background, still duration) and the conform flow (PM-011 calls TL-001's conform op).
  - PM owns `MediaId`/`MediaRef`. A `MediaRef` is a `MediaId` plus a stream selection.
  - Clips wait on PM probes before placement (TL-012). Usage counts come from the TL document.
  - TL-042 draws timeline filmstrips and waveforms. PM produces and caches the frames (PM-071) and owns the cache location and limits (PM-074).
  - TL's per-clip Replace (TL-035) differs from PM-061. Pasting across projects adds media through PM-030's op.
  - Revealing a clip's media item (TL-046) and in/out-range Insert/Overwrite from source preview (TL-047) are TL features; PM-060 provides the source mode they build on.
  - Background work that changes the document (for example PM-039's switch to copied media) adds one history entry when applied, as TL-043 requires.
  - PM-107 (later) owns the project's list of timelines, their names, the primary choice and per-timeline settings; TL keeps owning each sequence's content. TL-003's "one timeline per project" and TL-043's undo across timelines need matching updates before PM-107 ships. A timeline dropped onto another becomes a TL-040 compound clip holding a copy.
  - Color clips (PM-108) are placed by TL-012 like stills, with the still duration that TL-011's default durations and PM-010 share (5 s).
  - Media labels (PM-110) use TL's shared palette (TL-037, TL-045, TL-049), and can optionally seed TL-045's clip labels.
  - An offline clip's context menu offers **Relink media…** (PM-091).
  - TL-055's snippets and TL-056's inserted projects bring media in through PM-030 and PM-038 and read other projects with PM-021's migrations, without taking PM-013's lock. PM-107's **Import timeline from another project…** shares TL's `import_sequence` with them.
- **Transform, canvas, masks, keying, keyframes & speed ([clip-transform-and-speed.md](clip-transform-and-speed.md), TR):**
  - TR stores layout in canvas-normalized units, so PM-003/PM-004/PM-011 can change the canvas without breaking layouts, and proxies can swap in without reframing.
  - TR reads the default fit (PM-008) and the project background (PM-009).
  - Freeze frames reuse the same `MediaRef`, so they are not generated media.
  - The stabilization (TR-026), reverse and optical-flow (TR-042) caches live in the media cache (PM-074), keyed on PM-038's fingerprint. Export waits for them to regenerate when they are missing.
  - The keying demo sample (PM-063) is there for TR's chroma-key previews.
  - Color clips (PM-108) always fill the sequence canvas and take TR's transform, crop and opacity like any visual clip.
  - PM-115 places a camera recording in a corner over a screen recording using TR's transform.
- **Audio ([audio.md](audio.md), AU):**
  - AU owns waveforms (AU-009). PM stores the peak caches and draws tile previews (PM-072).
  - PM owns the sample-rate setting (PM-007).
  - Extracted audio is a virtual item, `AudioOf`. Voiceover recordings (AU-027) and stems are generated items (PM-041), stored in `<Name> Media/Recordings/` and `Generated/`. PM-043's camera recorder reuses AU-027's microphone input path and device list.
  - AU-021 owns both stem commands. The in-place **Voice isolation** dropdown switches a clip's playback to a stem. **Split into stems…** creates linked clips on new audio tracks (2 or 4 stems), mutes but never deletes the source audio, and is one undo step. Stem files are decoded at step 1 of AU's signal chain. AI-023 provides only the engine (PM-041).
  - Processed audio renders (AU-018's processed-audio cache) are media-cache entries (PM-074), not generated items.
  - Multi-stream selection is a PM interpretation that AU exposes in the inspector.
  - PM-115's screen recordings reuse AU-027's microphone path. System audio, when captured, is a second audio stream that AU's multi-stream handling exposes.
- **Color & filters ([color-and-filters.md](color-and-filters.md), CL):**
  - PM probes color tags into `ColorInfo`. The color-interpretation override lives on the `MediaItem`, using CL's op.
  - CL defines `LutRef` (SHA-256 plus file name) and the user LUT library. PM-042 indexes, resolves and relinks those references, and PM-101 collects the files.
  - CL tone-maps HDR to SDR at decode. SDR Rec.709 is the single working and output space (CL-002). There is no HDR working space, so HDR output is not planned either (see Out of scope).
  - CL-003's neutral reference image ships in the sample pack (PM-063), with a procedural fallback when the pack is absent.
  - Because CL-002 converts HDR at decode and shows its own one-time notice, PM has no per-import color-space prompt and no mixed-color-space warning (PM-011).
- **Effects, transitions & animations ([effects-transitions-animations.md](effects-transitions-animations.md), FX):**
  - FX owns render order, including where the project background clear (PM-009) and the offline slate sit.
  - FX-023 wipe patterns are media items.
  - FX-004's transition tiles and FX-016's previews use the sample pack's items by role (PM-063), with procedural frames when the pack is absent.
- **Overlays & stickers ([overlays-and-stickers.md](overlays-and-stickers.md), OV):**
  - OV relies on the still-image import and duration (PM-034, PM-010), sequence detection (PM-033), alpha probing (PM-035) and alpha-capable proxies (PM-080).
  - OV's asset-library favorites are separate from media favorites (PM-056).
  - OV-020 keeps the user's stickers in a per-user library. PM-042 indexes and resolves the project's references to them, and PM-101 collects them.
- **Text & captions ([text-and-captions.md](text-and-captions.md), TX):**
  - TX gets the canvas size and frame rate from PM. TX-001's rule is canonical for how text follows a canvas change: lengths in ref px are multiplied by `canvas_short_edge / 1080` (PM-011).
  - Text-to-speech audio goes into the generated-media store (PM-041).
  - TX owns `FontRef`, the font library and the missing-font flow (TX-007). PM-042 lists missing fonts alongside other assets, and font collection in packaging follows TX-007's license rule (PM-101).
  - Subtitle streams found by probing are offered to TX for import.
- **Smart / AI tools ([ai-tools.md](ai-tools.md), AI):**
  - AI outputs that a clip points at as its media (upscaled or enhanced video, inpainted video, stems, TTS audio, external generator output) become generated items in the PM-041 store. What ai-tools.md calls `DerivedMedia` and media provenance (AI-026) is PM-041's `MediaOrigin::Generated`.
  - Deterministic AI outputs (mattes from AI-014, AI-interpolated frames from AI-018, analysis records) are media-cache entries (PM-074). They are not generated items, and `GeneratedKind` has no `Matte`.
  - AI-023 is only the separation engine: the model, the job, the quality choice and the FLAC stems written as PM-041 generated media. Inference runs in `freecut-analysis`. The user commands are AU-021's.
  - AI-001's result key is PM-038's fingerprint + stream + source range + kind + algorithm or model id and version + parameter hash, with no media id or mtime. Staleness comes from PM-093.
  - AI's scene detection writes source markers onto media items (PM-057).
  - AI-001's analysis jobs appear as rows in PM-073's background-task list. AI-001 defines its job kinds, per-clip progress strips and badges, and its pool's concurrency settings. The indicator and toasts are UI-040's. Its analysis records live under the PM-074 cache root.
  - After a canvas ratio change (PM-011), AI-016 may offer to auto-reframe clips.
  - Local model files live in app data, managed by AI, and are not project media.
  - Analysis may run on proxies.
  - Spoken-word media search is AI-027, which adds its chip to PM-054's search field. Visual content search is in AI's Out of scope.
- **Preview & export ([preview-and-export.md](preview-and-export.md), EX):**
  - EX owns the player, preview quality and the hardware-decoding toggle.
  - Export defaults come from PM settings.
  - EX owns the offline and error slates (EX-005), media preview mode (EX-015, which PM-060 feeds) and the player's ratio menu (EX-013, which calls PM-003's operation).
  - EX-036's export preflight reads PM-090's media status. Export uses originals, with proxies standing in only for missing originals after a warning (PM-081).
  - EX-016's pre-rendered sections live in the PM-074 cache. Export waits for missing regenerable cache entries (PM-074).
  - EX-014's snapshots added to the library are generated items (PM-041).
  - EX-038's export queue reports rows into PM-073's task list.
  - HDR output is in EX's Out of scope, to be revisited only together with an HDR working space. SDR Rec.709 is the single output space.
  - Autosave runs before export (PM-014).
  - EX-039 appends a record for each finished export, which PM-001's **Recent exports** tab reads.
  - EX-017's playback indicator gets its suggestions from PM: create proxies (PM-080), re-wrap (PM-111), slow storage and battery power (PM-114). Timeline-side causes, such as many stacked overlay tracks (the reference app warns above 4 picture-in-picture tracks), are EX-017's own suggestions, not PM's.
  - With several timelines (PM-107, later), EX-021 exports the timeline on screen.
  - EX-036 warns about offline media instead of blocking export (PM-090); EX-036 and EX-037 check free space on the export destination, while PM-113 watches FreeCut's own folders.
  - PM-114's external-drive note also covers the export folder that EX-023 owns.
- **Workspace & shortcuts ([workspace-and-shortcuts.md](workspace-and-shortcuts.md), UI):**
  - UI owns the start-screen and panel layout, the inspector frame and the final key bindings.
  - UI-040 owns the status-bar task indicator and the toasts. PM-073 owns the single task list that the indicator opens.
  - UI-004's Media tab shows the sample pack's Samples sub-category (PM-063). UI-007's hover previews and UI-028's decode **Test** use the pack by role. UI-051's About lists its manifest.
  - UI-016 and UI-017 frame the start screen's new tabs and actions: **Recent exports** (PM-001), **Deleted projects** (PM-104), and later folders (PM-105) and **Add projects from a folder…** (PM-106).
  - UI-026's Storage section hosts PM-074's age rule and **Remove temporary files**, PM-080's custom proxy folder, PM-104's retention, PM-113's thresholds, PM-114's storage hints, and later PM-112's list of locations.
  - UI-012 shows PM-012's last-saved time and save-failure reasons. UI-036 owns the no-drop feedback PM-031 uses. UI-053's "open with" covers media files too (PM-030).
  - UI-040 shows PM-073's "Paused: low disk space" state (PM-113).
  - Proposed bindings: `Mod+N`, `Mod+O`, `Mod+S`, `Mod+Shift+S`, `Mod+I`, F2, `Delete`, and `Mod+F` in the panel.

## Out of scope (and why)
- **Cloud drafts, sync, shared team spaces, and signing in to reach projects:** FreeCut is local-only, with no account and no servers. Users can sync project folders with their own tools; relative paths (PM-100) and packaging (PM-101) make that work. This also rules out the reference app's cloud backup and download of projects and media, automatic upload from chosen devices, transfer lists, storage quotas, cloud folders, cloud media shown in the media panel, team spaces and shared brand kits as media sources, and the conflict choices between local and cloud copies. Local substitutes: autosave (PM-014), crash recovery (PM-015), rolling backups (PM-016), Deleted projects (PM-104), packaging (PM-101), the changed-on-disk check when saving (PM-012), the personal library (PM-062) and OV's local brand kit (OV-043).
- **Mobile and tablet editions, and moving projects between devices:** there are no other FreeCut editions to sync with. That drops the reference app's mobile-to-desktop project transfer, projects that are read-only on desktop or open only on mobile, syncing only the primary timeline to a smaller device, and importing photos by scanning a QR code with a phone. A FreeCut project moves between computers as a folder or a package (PM-101), and PM-021 keeps data it doesn't understand, so files from newer versions or forks aren't damaged.
- **Online stock media, music, template or asset libraries inside the media panel:** these would need servers and licensed catalogues. The offline substitutes are PM-063's small CC0 sample pack, OV's own or open-licensed packs and AU's bundled music and sound effects. Everything else is imported by the user. Footage from stock websites isn't added to the sample pack either, because it comes under each site's own licence terms rather than CC0. Without downloads, FreeCut also needs no download queue, pause or stop controls, Wi-Fi warnings, "finish downloading before adding to the timeline" rules or offline states for a library.
- **Licensed or premium assets inside projects:** the reference app removes assets whose licence has expired when a project opens, refuses to open some projects offline until online resources are updated, and keeps projects that use premium templates from moving to another computer. None of this applies: FreeCut's assets are bundled open-licensed packs or the user's own files, and a missing one follows the ordinary missing-asset path (PM-042, PM-091).
- **Online project or asset format upgrades:** FreeCut migrates older project files offline, keeping a copy of the original (PM-021). Nothing ever needs a network connection to open.
- **Uploading media for server-side analysis to sharpen search, and model-based search over an online stock library:** never. Spoken-word search runs locally (AI-027, later); searching by what is seen in the picture is in AI's Out of scope.
- **Upload-size caps:** the reference app caps some uploads at 1 GB or 500 MB and local imports at 4 GB per file. FreeCut uploads nothing and has no per-file import cap (PM-030); only free disk space matters (PM-113).
- **Cloud tools offered after a recording** (automatic captions, voice processing and retouching on a server): FreeCut's recordings are ordinary media, and the local tools in other areas apply to them: auto captions (AI-010), noise reduction (AU-018), speech enhancement (AI-024) and face retouch (AI-020).
- **Start-screen promotions:** no plan status, upgrade banners, countdown offers, partner promotions, onboarding surveys or rating prompts on the start screen or anywhere else. The start screen holds only the user's projects, exports and deleted projects (PM-001, PM-104).
- **Importing projects from other editors' native draft formats:** not part of this spec. Any such importer would need its own clean-room process, documenting only project files we created ourselves, per CLAUDE.md. Interchange goes through the open OTIO format (PM-103).
- **HDR project working space and HDR export:** SDR Rec.709 is the single working and output space. CL tone-maps HDR sources at decode (CL-002), and HDR output (formerly planned as EX-033) is in EX's Out of scope. No spec plans an HDR working space, and HDR output will only be revisited together with one, as a joint CL, EX and PM change. The reference app offers four project color spaces (Rec.709 SDR, Rec.2020 SDR, Rec.2100 HLG and Rec.2100 PQ), offers to convert media whose color space differs from the project's, and warns when spaces are mixed; those prompts are not needed while there is one working space.
- **Proprietary camera RAW video, and copy-protected media:** these need vendor SDKs or DRM circumvention. The second is never acceptable.
- **Phone-to-desktop transfer and QR pairing:** these belong to the OS or to separate tools, not to media management. Phones and cameras that mount as drives are imported with PM-030 and PM-032. Camera and microphone recording into the project is PM-043 (later), screen recording (with or without a camera) is PM-115 (later), and voice-only recording is AU-027. A teleprompter overlay in the recording window is not planned.
- **Showing pressed keys in screen recordings:** it needs system-wide keyboard monitoring, which the OS treats as a sensitive permission and which would capture everything the user types, including passwords. For the same reason PM-115 offers a click highlight only where the capture backend reports clicks itself.
- **Spanned-clip merging and camera metadata ingest** (reels, scene and take fields): niche. Spanned files import as separate items.
- **Multi-user locking and simultaneous editing:** single-user desktop app. The lock file (PM-013) only prevents accidents.
- **Persisting undo history across sessions:** TL keeps history in memory only. This could be revisited later.

## Clean-room notes
- **Our own creations:** every user-facing name, preset label (PM-020), folder name and message in this spec is FreeCut's own. Canvas presets are generic industry ratios, and frame rates are broadcast and web standards. No CapCut wording, preset names, icons or UI artwork are used.
- **Project format:** the `.freecut` JSON schema is ours. It is not modeled on any other editor's draft format, and no CapCut draft or project files were opened or inspected for this spec.
- **Placeholders, empty-state art and badges:** drawn by us (the offline slate is EX-005's procedural drawing, not an asset), or taken from an open icon set with license recorded, for example Lucide (ISC) or Phosphor Icons (MIT).
- **Test media:** generated with FFmpeg's built-in test sources (color bars, test patterns, sine tones) or recorded by contributors and released as CC0. No third-party footage is committed.
- **Sample media pack (PM-063):** every clip and still is shot, photographed or rendered by FreeCut contributors and dedicated to the public domain under CC0 1.0 (SPDX `CC0-1.0`), or generated from FFmpeg's synthetic sources. Contributors confirm the work is their own. The manifest and the repository's REUSE annotations record the creator. The pack never contains:
  - footage, stills, music or effects from CapCut's online library, or anything downloaded from inside any editor
  - stock-website footage, whose licences are not CC0
  - recognizable logos, brands, artwork or third-party music in shot
  - identifiable people without a signed model release, because CC0 waives copyright but not privacy or publicity rights
- **Camera and screen recording (PM-043, PM-115):** built on the operating systems' public camera and screen-capture APIs, through FFmpeg's device inputs, the `nokhwa` crate (Apache-2.0/MIT) or other safe open-source wrappers chosen later. The window layout, labels and defaults are ours. What comes from the reference app is only the list of capabilities in its paraphrased UI labels (screen, region and camera sources, separate tracks, pause and resume, retake, exposure and focus locks); none of its wording, icons or artwork is used.
- **Features added from the UI-label inventory** (PM-104 to PM-115, and the enrichments marked "the reference app…"): each one describes a capability in our own words, with FreeCut's own labels ("Deleted projects", "Color clip", "Re-wrap for faster scrubbing", "Storage hints" and so on) and FreeCut-chosen defaults. Where the reference app's observed values were adopted (30-day retention, 10 folder levels, the frame-rate list), they are plain numbers, not wording. Default timeline names ("Timeline 2") follow the generic numbering most editors use. Media labels use TL's own 8-color palette.
- **Libraries named in this spec:** `directories` (MIT/Apache-2.0), `rfd` (MIT), `walkdir` (MIT/Unlicense), `notify` (CC0/MIT/Apache-2.0), `trash` (MIT), `opener` (MIT/Apache-2.0), `xxhash-rust` (BSL-1.0), `sha2` (MIT/Apache-2.0), `zip` (MIT) for the optional package archive, `resvg` later (MPL-2.0), `nokhwa` later (Apache-2.0/MIT, one candidate camera backend for PM-043). The REUSE tool used in CI is a development tool and isn't shipped. FFmpeg encoders and decoders are used under the GPL build that matches FreeCut's GPL-3.0-or-later license, libx264 (GPL) is used for proxies, and libvpx (BSD-3-Clause) for alpha-capable proxies. All of these are compatible with GPL-3.0-or-later.
- **How this research was done:** from public help pages, tutorials, reviews and other editors' public manuals, plus the English UI labels of the reference app as installed locally (CapCut desktop 9.5.0), taken from its bundled English localization file (see Provenance in the [README](README.md)) and summarized as a paraphrased feature inventory. Nothing was decompiled, disassembled or traced, and no assets, strings or project files were copied. Behavior is described in our own words. Where public pages and the observed labels disagreed, the spec picks one FreeCut behavior and notes the alternative in one line.
- **Libraries named for the new features:** `fs4` (MIT/Apache-2.0) for free-space queries (PM-113), `sysinfo` (MIT) for volume kinds (PM-114) and `quick-xml` (MIT) for XMP metadata (PM-109), all compatible with GPL-3.0-or-later. They are candidates; the final choice is made at implementation time.

## Sources
- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- CapCut for PC & Mac: Desktop Tutorial for Beginners (2026) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- CapCut for PC & Mac – Complete Video Editing Tutorial (2024) — https://primalvideo.com/video-creation/editing/capcut-for-pc-mac-tutorial
- CapCut Help: Why does video quality change after exporting? (proxy setting) — https://www.capcut.com/help/video-quality-change-after-exporting
- CapCut Help: What if the edits don't match the preview? — https://www.capcut.com/help/editing-not-match-displayed
- CapCut Help: Why can't local assets be recognized during import? — https://www.capcut.com/help/can-not-local-assets-be-recognized-during-import
- CapCut Help: Why can't I import MP4 and JPG files? — https://www.capcut.com/help/can-not-import-mp4-and-jpg-files
- CapCut Help: How do I export 2K/4K videos? — https://www.capcut.com/help/export-videos-in-capcut
- CapCut Help: Editing & Exporting article index — https://www.capcut.com/help/editing-and-exporting?pageNo=4
- CapCut Help: How to set replaceable material clips — https://www.capcut.com/help/how-to-set-replaceable-material-clips
- CapCut resource: How to use CapCut on different devices — https://www.capcut.com/resource/how-to-use-capcut-on-pc
- CapCut resource: Frame rate and shutter speed — https://capcut.com/resource/frame-rate-and-shutter-speed
- TourBox: How to change the aspect ratio in CapCut — https://www.tourboxtech.com/en/news/how-to-change-aspect-ratio-capcut.html
- SendShort: Change aspect ratio in CapCut — https://sendshort.ai/guides/capcut-aspect-ratio/
- Miracamp: Best color space for CapCut (2025) — https://www.miracamp.com/learn/capcut/whats-the-best-color-space
- Miracamp: How to delete a project in CapCut — https://www.miracamp.com/learn/capcut/how-to-delete-a-project
- CapCut Guide: Back up and transfer CapCut projects — https://capcutguide.com/backup-transfer-capcut-projects/
- Ultimate Systems Blog: CapCut folders and cache paths (Mac and PC) — https://blog.usro.net/2025/08/capcut-folders-cache-mac-and-pc-paths/
- Filmora: How to recover lost CapCut projects after a crash — https://filmora.wondershare.com/answers/how-to-recover-lost-capcut-projects-after-crash.html
- CapEditCut community: Files do not have imported media — https://www.capeditcut.com/community/capcut/files-do-not-have-imported-media/
- Kdenlive manual: Project Settings — https://docs.kdenlive.org/en/project_and_asset_management/project_settings.html
- Kdenlive manual: Proxy settings — https://docs.kdenlive.org/en/project_and_asset_management/project_settings/proxy_settings.html
- Kdenlive manual: Archiving — https://docs.kdenlive.org/en/project_and_asset_management/file_management/archiving.html
- Kdenlive manual: File menu (open, backups, clip problems) — https://docs.kdenlive.org/en/user_interface/menu/file_menu.html
- Kdenlive manual: Using the Project Bin — https://docs.kdenlive.org/en/project_and_asset_management/project_bin/project_bin_use.html
- Shotcut forum (manual mirror): Settings > Proxy Editing — https://forum.shotcut.org/t/settings-proxy-editing/18517
- Shotcut forum (manual mirror): Settings > Video Mode — https://forum.shotcut.org/t/settings-video-mode/12790
- Linuxiac: Shotcut 26.1 brings hardware video decoding — https://linuxiac.com/shotcut-26-1-video-editor-brings-long-awaited-hardware-video-decoding/
- Larry Jordan: When you import media affects project frame rates in DaVinci Resolve — https://larryjordan.com/articles/the-importance-of-first-import-into-davinci-resolve/
- JayAreTV: Relink offline media in DaVinci Resolve — https://jayaretv.com/workflow/relink-offline-media-in-davinci-resolve/
- Indie Tips: Remove unused clips from DaVinci Resolve — https://indietips.com/?p=69391
- OpenCut docs: Project management — https://mintlify.wiki/OpenCut-app/OpenCut/features/project-management
- CapCut resource: CapCut tutorial for beginners (mentions built-in stock videos) — https://www.capcut.com/resource/capcut-tutorial-for-beginners
- Creative Commons: CC0 1.0 Universal deed — https://creativecommons.org/publicdomain/zero/1.0/
- REUSE Specification 3.3 (FSFE) — https://reuse.software/spec-3.3/
- Debian Developer's Reference: Best packaging practices (architecture-independent data) — https://www.debian.org/doc/manuals/developers-reference/best-pkging-practices.en.html
- FFmpeg documentation: Devices (avfoundation, dshow, v4l2 inputs) — https://ffmpeg.org/ffmpeg-devices.html
- FFmpeg documentation: Filters (synthetic video and audio test sources) — https://ffmpeg.org/ffmpeg-filters.html
- Kdenlive manual: Capture configuration — https://docs.kdenlive.org/en/getting_started/configure_kdenlive/configuration_capture.html
- nokhwa camera capture crate (README and licence) — https://github.com/l1npengtul/nokhwa
