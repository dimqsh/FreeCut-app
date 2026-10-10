# FreeCut feature specs

This folder holds FreeCut's **feature specifications**: one spec per product area, written before code. Each feature says what the user sees, which controls it has (ranges, defaults, units), what it produces, its edge cases, how the edit is undone, and which crate owns which part. Implementation follows the workflow in [CLAUDE.md](../../CLAUDE.md): write or update the spec here, implement the model in `freecut-timeline` first, add tests, then build the UI. Crate-level designs that turn these specs into types and pipelines live in [`../architecture/`](../architecture/).

**Clean-room rule.** Every spec describes behavior in our own words. The sources are public help pages, tutorials, open standards and other editors' public manuals, plus the reference app's English UI labels and observations of the app itself, paraphrased into options, ranges, limits and error states. Nothing from the reference app is copied: no code, assets, presets, models, project files or label text. Its code is never decompiled, disassembled or traced, and none of its online services are touched. Every label in the specs is FreeCut's own wording.

### Priorities

| Priority | Meaning |
|---|---|
| **mvp** | Required for the core loop: create project → import media → arrange/trim/split/delete on a multi-track timeline with undo → preview playback → basic text → clip volume/fades → basic transform → one cross-dissolve transition → export MP4 (H.264/AAC) → save/load project. Two mvp items sit outside the loop because adding them later would mean rework or an unshippable binary: the string catalog (UI-044) and the license notices (UI-051). |
| **v1** | The everyday features people expect from a CapCut-class editor: speed curves, keyframes, filters/LUTs, adjustments, masks, chroma key, text animations, caption import/export, transition and effect libraries, stickers from our own assets, audio effects, proxies, and so on. |
| **later** | ML-heavy, niche or very large features. |

When an mvp feature mentions behavior from a later-priority feature (for example a locked track, TL-009), that behavior applies only once the later feature ships.

### Spec template

Every spec uses the same skeleton:

```
# <Area title>
> Status: draft · Owns: <shared concepts or "—"> · Crates: <crates>

## Summary                         2–4 sentences
## Features
### <ID> <Feature name> — <mvp|v1|later>
**What the user sees:**
**Inputs / controls:**            ranges, defaults, units; "(FreeCut default)" where public info is missing
**Behavior / output:**
**Edge cases:**
**Undo:**                         the edit op(s) and how they reverse, or "n/a (view state)"
**Model notes:**                  types/ops implied, which crate owns which part
## Interactions with other areas
## Out of scope (and why)
## Clean-room notes                what we create or substitute, with the open alternative and its license
## Sources                         one line per public URL (title — URL), no quoted text
```

### Conventions every spec follows

- **IDs** are `<prefix>-<number>` (for example `TL-023`). They are stable and never reused; a dropped feature keeps its number reserved (EX-033, formerly HDR export). Specs refer to each other by ID.
- **One owner per shared concept.** A concept is defined once, in the spec listed as its owner in the [Spec index](#spec-index); every other spec links to it instead of redefining it. In particular: time, tracks, clips, selection and undo semantics → TL; keyframes, easing and speed → TR; project settings and media references → PM; per-frame compositing order → FX; default keys → UI-031.
- **Time is rational** (`i64` ticks at 1/705,600,000 s, TL-001). Only time has to be rational: each spec picks its own type for other values and animates them through TR-027's `Animatable<T: Interpolate>`.
- **Single sources of truth:** one render pipeline for preview and export (FX-001); one undo stack (TL-043); one default keymap (UI-031); one background-task list (PM-073, surfaced by UI-040); one media content key (PM-038's `Fingerprint`); one Paste-attributes registry (TL-034); one storage rule — regenerable outputs go to the media cache (PM-074), files a clip points at go to the generated-media store (PM-041).
- **Local only:** no account, no cloud, no telemetry, no watermark. Cloud features of the reference editor become local tools, our own or open-licensed bundled assets, user-imported assets, optional local open-source models — or are listed as out of scope. The one exception is the opt-in assistant (AS): it is off by default, the user may point it at a local model server or at a hosted language-model API with their own key, FreeCut runs no service of its own, and AS-004 shows exactly what is sent. A second opt-in exception is **proposed and awaits the owners' confirmation**: cloud generation (CG) with the user's own Higgsfield account. It is off by default, FreeCut runs no service for it either, every paid run needs a cost preflight and an explicit click (CG-005), and CG-004 shows exactly what is sent. Local tools stay the default wherever one exists.
- "(FreeCut default)" marks a value we chose ourselves.
- In this README, crate names drop the `freecut-` prefix: `timeline`, `media`, `render`, `audio`, `project`, `app`, plus `analysis`, the ML/analysis crate that AI-001 proposes, `assistant`, the language-model assistant crate that [llm-editor.md](../architecture/llm-editor.md) proposes for AS, and `cloud`, the cloud-generation crate that [cloud-generation.md](../architecture/cloud-generation.md) proposes for CG.

## Spec index

| Prefix | Spec | Area | Owns (shared concepts) | mvp | v1 | later | Total |
|---|---|---|---|---:|---:|---:|---:|
| PM | [project-and-media.md](project-and-media.md) | Projects & media library | Project settings (canvas size & ratio, frame rate, sample rate, background, default fit, still duration); the project file and its lifecycle (save/open/lock, autosave, recovery, backups, format versions & migrations, portable paths, packaging); the Deleted-projects hold (PM-104); the project's list of timelines and per-timeline settings (PM-107; TL owns each sequence's content); media references (`MediaId`/`MediaRef`, bins, probe data, interpretation, status, relink); color clips (PM-108); the media `Fingerprint` (PM-038); referenced-asset index; generated-media store (PM-041); media cache (PM-074); the single background-task list (PM-073); proxies; disk-space monitoring & storage-location advisories (PM-113, PM-114); camera and screen recording; CC0 sample pack | 26 | 38 | 13 | 77 |
| TL | [timeline-editing.md](timeline-editing.md) | Timeline & core editing | Time representation (ticks, frame grid); sequences; tracks (kinds, layering bands, flags, colors); clips (names, links); selection; undo/redo semantics; batch-edit scopes & skip reporting (TL-053); per-clip operation availability (TL-054); markers; groups; linked selection (TL-057); compound clips & clip snippets (TL-055); clipboard and the Paste-attributes registry (TL-034) | 19 | 29 | 9 | 57 |
| TR | [clip-transform-and-speed.md](clip-transform-and-speed.md) | Transform, canvas, masks, keying, keyframes & speed | Keyframes & easing (`Animatable<T: Interpolate>`, the one split rule); the project-wide rule for non-time values; retiming (time map, constant speed, curves, reverse, frame sampling, freeze-frame source); transform & fit; on-canvas align & distribute; crop; opacity; masks; chroma/luma key; stabilization; canvas background fill | 4 | 36 | 8 | 48 |
| TX | [text-and-captions.md](text-and-captions.md) | Text, titles & captions | Text clip payload (`RichText`, `TextStyle`, layout); font registry & fallback; font collections; the text lens contents & card behaviour (TX-054); bubble & tag boxes (TX-055); text animation presets & their motion catalogue (TX-056); text styles & templates; caption track payload (segments, track style, secondary line, word timings & word states, line rules, caption looks, progressive display, caption emoji); subtitle formats; how recognition, translation, speech, dictation and lyric results land in text and on caption tracks | 6 | 38 | 13 | 57 |
| AU | [audio.md](audio.md) | Audio | Gain/loudness units (dB, LUFS, pan, cents); `ClipAudio` (including echo reduction); the signal chain & master bus; master/track audio slots, project mute & solo; the *All streams mixed* stream choice; audio-tool availability rules & the silent-source flag; the before/after compare bypass; gain overlay; Keep pitch & time-stretch; beat detection (audio side); voice isolation & *Split into stems*; audio crossfades; ducking; audio auto-fix assistants; voiceover recording; audio sync; music & SFX libraries; audio-only export options | 8 | 28 | 6 | 42 |
| CL | [color-and-filters.md](color-and-filters.md) | Color, adjustments & filters | The color stack and its internal order; source color interpretation; HDR→SDR tone mapping & export color tags; color parameter conventions; look presets; LUT assets & parsing; adjustment-clip content; color-mask lists inside a stack (the mask type is TR's); classic denoise, deflicker & dehaze; scopes | 1 | 35 | 9 | 45 |
| FX | [effects-transitions-animations.md](effects-transitions-animations.md) | Effects, transitions & clip animations | Per-frame compositing / render order (FX-001 stages, including where other areas slot in); the transition model; effect items, stacking order & effect regions (FX-027); clip entrance/exit/continuous animation slots & slot policy; the animation preset manifest, Intensity & clip-preset motion definitions (FX-030) | 4 | 17 | 9 | 30 |
| OV | [overlays-and-stickers.md](overlays-and-stickers.md) | Overlays, stickers, shapes & templates | `BlendMode`; element styling at FX-001's L8b; clip-time → animation-time mapping (OV-013); element clips (stickers, emoji, shapes, lines, frames, custom paths, charts) and the chart data model (`ChartSpec`); border/corner radius, outline & shadow, clip frames; layer-order commands; element-pack manifest; asset-credits sheet; user templates & placeholder slots | 2 | 31 | 21 | 54 |
| AI | [ai-tools.md](ai-tools.md) | Smart / AI-assisted tools | Analysis job kinds, result key & records; smart-tool compatibility rules; the local ML runtime (`freecut-analysis`, proposed) & model manager; transcripts & transcript search; motion-track and plane-track data; face tracks; AI mattes; depth records; beat-grid refinement; smart motion plans (reframe pins, camera moves); retouch presets; health-check findings & applied-fix records; the ML engines behind AU-020/AU-021 and TX's speech features | 0 | 10 | 27 | 37 |
| EX | [preview-and-export.md](preview-and-export.md) | Preview player & export | Playback transport & clock policy; scrub/seek frame delivery; offline/error slates; preview resolution, playback priority & render cache; viewer zoom, fullscreen, rulers, guides & safe areas; snapshot; the export dialog, settings model & presets; the export job, its failure catalogue & queue; output size, frame rate, codec, container, rate control & color tags; export range & per-clip export; preflight; export cover & cover designer | 18 | 30 | 11 | 59 |
| UI | [workspace-and-shortcuts.md](workspace-and-shortcuts.md) | Workspace layout, inspector & shortcuts | App shell & window layout; workspace state; auxiliary-panel docking; the library-panel framework; inspector framework & control conventions; command registry & default keymap (UI-031); context-menu, drag-and-drop & tooltip conventions; Preferences storage & window; save-status presentation; crash reports & safe mode; toasts & the task indicator; hints & the "don't ask again" registry; the system check; diagnostic bundles; the shared save-as-preset dialog; theme tokens; localization & accessibility baseline | 23 | 34 | 8 | 65 |
| AS | [assistant.md](assistant.md) | LLM assistant | The assistant panel and conversation; provider settings and API-key storage; the tool surface a language model sees (query, edit, action and macro tools, generated from `EditRequest`) and its argument conventions; proposals (change list, ghost preview, accept / adjust / discard as one undo entry, re-checks after document changes); the disclosure of what leaves the machine; assistant limits and evaluation scenarios | 0 | 14 | 6 | 20 |
| CG | [cloud-generation.md](cloud-generation.md) | Cloud generation (Higgsfield) | Connecting a user-owned generation service account (sign-in, pasted token, keychain, disconnect); the generation model catalogue and filters; the run sheet (cost preflight, what is sent, explicit spend); the asset shelf's Generate tab; cloud job states, the unknown-outcome rule and the cloud job ledger; source-range and upload rules for AI edit actions on clips; placement of generated results; spending limits; the assistant's generation tools | 0 | 18 | 8 | 26 |
| | **All specs** | | | **111** | **358** | **148** | **617** |

## Feature catalog

One table per spec. IDs link to the feature's section in its spec. The Crates column lists every crate the feature touches, model first and UI last: `timeline`, `media`, `render`, `audio`, `project`, `analysis`, `assistant`, `app`.

### PM — [Projects & media library](project-and-media.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [PM-001](project-and-media.md#pm-001-start-screen--project-library--mvp) | Start screen & project library | mvp | `project`, `app` |
| [PM-002](project-and-media.md#pm-002-new-project--mvp) | New project | mvp | `timeline`, `project`, `app` |
| [PM-003](project-and-media.md#pm-003-canvas-aspect-ratio-presets--mvp) | Canvas aspect ratio presets | mvp | `timeline`, `app` |
| [PM-004](project-and-media.md#pm-004-canvas-resolution--mvp) | Canvas resolution | mvp | `timeline`, `app` |
| [PM-005](project-and-media.md#pm-005-sequence-frame-rate--mvp) | Sequence frame rate | mvp | `timeline`, `app` |
| [PM-006](project-and-media.md#pm-006-match-project-settings-to-the-first-clip--v1) | Match project settings to the first clip | v1 | `timeline`, `app` |
| [PM-007](project-and-media.md#pm-007-audio-sample-rate--channels--mvp) | Audio sample rate & channels | mvp | `timeline`, `audio`, `app` |
| [PM-008](project-and-media.md#pm-008-default-fit-for-mismatched-media--v1) | Default fit for mismatched media | v1 | `timeline`, `app` |
| [PM-009](project-and-media.md#pm-009-project-background-color--v1) | Project background color | v1 | `timeline`, `render`, `app` |
| [PM-010](project-and-media.md#pm-010-still-image-default-duration--v1) | Still-image default duration | v1 | `timeline`, `app` |
| [PM-011](project-and-media.md#pm-011-change-project-settings-after-editing--mvp) | Change project settings after editing | mvp | `timeline`, `app` |
| [PM-012](project-and-media.md#pm-012-save-save-as-save-a-copy--mvp) | Save, Save As, Save a Copy | mvp | `project`, `app` |
| [PM-013](project-and-media.md#pm-013-open-project--single-writer-lock--mvp) | Open project & single-writer lock | mvp | `project`, `app` |
| [PM-014](project-and-media.md#pm-014-autosave--v1) | Autosave | v1 | `project`, `app` |
| [PM-015](project-and-media.md#pm-015-crash-recovery--v1) | Crash recovery | v1 | `project`, `app` |
| [PM-016](project-and-media.md#pm-016-rolling-backups--restore--v1) | Rolling backups & restore | v1 | `project`, `app` |
| [PM-017](project-and-media.md#pm-017-recent-projects--mvp) | Recent projects | mvp | `app` |
| [PM-018](project-and-media.md#pm-018-project-management-actions--v1) | Project management actions | v1 | `project`, `app` |
| [PM-019](project-and-media.md#pm-019-project-cover-image--v1) | Project cover image | v1 | `render`, `project`, `app` |
| [PM-020](project-and-media.md#pm-020-new-project-defaults--settings-presets--v1) | New-project defaults & settings presets | v1 | `timeline`, `app` |
| [PM-021](project-and-media.md#pm-021-project-file-format-versioning--migrations--mvp) | Project file format, versioning & migrations | mvp | `timeline`, `project` |
| [PM-030](project-and-media.md#pm-030-import-media-via-dialog--mvp) | Import media via dialog | mvp | `timeline`, `media`, `app` |
| [PM-031](project-and-media.md#pm-031-drag-and-drop-import--mvp) | Drag-and-drop import | mvp | `timeline`, `media`, `app` |
| [PM-032](project-and-media.md#pm-032-folder-import--v1) | Folder import | v1 | `timeline`, `media`, `app` |
| [PM-033](project-and-media.md#pm-033-image-sequence-import--v1) | Image sequence import | v1 | `timeline`, `media`, `app` |
| [PM-034](project-and-media.md#pm-034-still--animated-images--mvp) | Still & animated images | mvp | `media` |
| [PM-035](project-and-media.md#pm-035-media-probing--normalization--mvp) | Media probing & normalization | mvp | `timeline`, `media` |
| [PM-036](project-and-media.md#pm-036-supported-formats--mvp) | Supported formats | mvp | `media`, `app` |
| [PM-037](project-and-media.md#pm-037-import-failures--import-report--mvp) | Import failures & import report | mvp | `media`, `app` |
| [PM-038](project-and-media.md#pm-038-duplicate-import-detection--v1) | Duplicate import detection | v1 | `timeline`, `media`, `app` |
| [PM-039](project-and-media.md#pm-039-copy-media-into-the-project-folder--v1) | Copy media into the project folder | v1 | `timeline`, `media`, `project`, `app` |
| [PM-040](project-and-media.md#pm-040-interpret-media-overrides--v1) | Interpret media (overrides) | v1 | `timeline`, `media`, `app` |
| [PM-041](project-and-media.md#pm-041-derived--generated-media-items--v1) | Derived & generated media items | v1 | `timeline`, `media`, `project` |
| [PM-042](project-and-media.md#pm-042-referenced-asset-files-luts-user-stickers-imported-fonts--v1) | Referenced asset files (LUTs, user stickers, imported fonts) | v1 | `timeline`, `project`, `app` |
| [PM-043](project-and-media.md#pm-043-camera-recording-into-the-project--later) | Camera recording into the project | later | `timeline`, `media`, `audio`, `app` |
| [PM-050](project-and-media.md#pm-050-media-panel-grid-view--mvp) | Media panel: grid view | mvp | `media`, `app` |
| [PM-051](project-and-media.md#pm-051-media-panel-list-view--v1) | Media panel: list view | v1 | `app` |
| [PM-052](project-and-media.md#pm-052-bins-folders--v1) | Bins (folders) | v1 | `timeline`, `app` |
| [PM-053](project-and-media.md#pm-053-sort--v1) | Sort | v1 | `app` |
| [PM-054](project-and-media.md#pm-054-search--filters--v1) | Search & filters | v1 | `app` |
| [PM-055](project-and-media.md#pm-055-usage-indicators--find-on-timeline--v1) | Usage indicators & find on timeline | v1 | `timeline`, `app` |
| [PM-056](project-and-media.md#pm-056-favorites--v1) | Favorites | v1 | `timeline`, `app` |
| [PM-057](project-and-media.md#pm-057-rename-notes--poster-frame--v1) | Rename, notes & poster frame | v1 | `timeline`, `app` |
| [PM-058](project-and-media.md#pm-058-remove-from-project--remove-unused--mvp) | Remove from project & remove unused | mvp | `timeline`, `app` |
| [PM-059](project-and-media.md#pm-059-media-info--reveal--v1) | Media info & reveal | v1 | `media`, `app` |
| [PM-060](project-and-media.md#pm-060-hover-scrub--source-preview--v1) | Hover scrub & source preview | v1 | `media`, `app` |
| [PM-061](project-and-media.md#pm-061-replace-media-file--v1) | Replace media file | v1 | `timeline`, `media`, `app` |
| [PM-062](project-and-media.md#pm-062-personal-library-across-projects--later) | Personal library across projects | later | `timeline`, `app` |
| [PM-063](project-and-media.md#pm-063-bundled-cc0-sample-media-pack--v1) | Bundled CC0 sample media pack | v1 | `timeline`, `render`, `project`, `app` |
| [PM-070](project-and-media.md#pm-070-media-thumbnails--mvp) | Media thumbnails | mvp | `media`, `app` |
| [PM-071](project-and-media.md#pm-071-timeline-filmstrip-frames--mvp) | Timeline filmstrip frames | mvp | `media`, `app` |
| [PM-072](project-and-media.md#pm-072-waveform-data--audio-tile-previews--mvp) | Waveform data & audio tile previews | mvp | `media`, `app` |
| [PM-073](project-and-media.md#pm-073-background-task-list--media-jobs--mvp) | Background task list & media jobs | mvp | `media`, `project`, `app` |
| [PM-074](project-and-media.md#pm-074-media-cache-management--v1) | Media cache management | v1 | `media`, `app` |
| [PM-080](project-and-media.md#pm-080-proxy-media--v1) | Proxy media | v1 | `media`, `app` |
| [PM-081](project-and-media.md#pm-081-proxy-preview-toggle--proxy-only-editing--v1) | Proxy preview toggle & proxy-only editing | v1 | `media`, `app` |
| [PM-082](project-and-media.md#pm-082-optimized-intermediate-media--later) | Optimized (intermediate) media | later | `timeline`, `media` |
| [PM-090](project-and-media.md#pm-090-offline-media-detection--display--mvp) | Offline media detection & display | mvp | `media`, `app` |
| [PM-091](project-and-media.md#pm-091-relink-a-missing-file--mvp) | Relink a missing file | mvp | `timeline`, `media`, `app` |
| [PM-092](project-and-media.md#pm-092-batch-relink-by-folder-search--v1) | Batch relink by folder search | v1 | `media`, `app` |
| [PM-093](project-and-media.md#pm-093-changed-on-disk-detection--refresh--v1) | Changed-on-disk detection & refresh | v1 | `timeline`, `media`, `app` |
| [PM-100](project-and-media.md#pm-100-portable-media-paths--mvp) | Portable media paths | mvp | `timeline`, `project`, `app` |
| [PM-101](project-and-media.md#pm-101-collect--package-project--v1) | Collect & package project | v1 | `project`, `app` |
| [PM-102](project-and-media.md#pm-102-trim-media-to-used-ranges-when-packaging--later) | Trim media to used ranges when packaging | later | `timeline`, `media`, `project` |
| [PM-103](project-and-media.md#pm-103-timeline-interchange-opentimelineio--later) | Timeline interchange (OpenTimelineIO) | later | `timeline`, `project` |
| [PM-104](project-and-media.md#pm-104-deleted-projects-30-day-hold--v1) | Deleted projects (30-day hold) | v1 | `project`, `app` |
| [PM-105](project-and-media.md#pm-105-project-folders-on-the-start-screen--later) | Project folders on the start screen | later | `project`, `app` |
| [PM-106](project-and-media.md#pm-106-add-projects-from-a-folder--later) | Add projects from a folder | later | `project`, `app` |
| [PM-107](project-and-media.md#pm-107-multiple-timelines-in-one-project--later) | Multiple timelines in one project | later | `timeline`, `project`, `app` |
| [PM-108](project-and-media.md#pm-108-color-clips--v1) | Color clips | v1 | `timeline`, `render`, `app` |
| [PM-109](project-and-media.md#pm-109-live-and-motion-photos--later) | Live and motion photos | later | `timeline`, `media` |
| [PM-110](project-and-media.md#pm-110-media-color-labels--later) | Media color labels | later | `timeline`, `app` |
| [PM-111](project-and-media.md#pm-111-re-wrap-hard-to-seek-files--later) | Re-wrap hard-to-seek files | later | `media`, `app` |
| [PM-112](project-and-media.md#pm-112-change-the-projects-folder--move-existing-projects--later) | Change the projects folder & move existing projects | later | `project`, `app` |
| [PM-113](project-and-media.md#pm-113-disk-space-monitoring--low-space-protection--v1) | Disk-space monitoring & low-space protection | v1 | `app` |
| [PM-114](project-and-media.md#pm-114-storage-location-advisories--v1) | Storage-location advisories | v1 | `app` |
| [PM-115](project-and-media.md#pm-115-screen-recording-into-the-project--later) | Screen recording into the project | later | `timeline`, `media`, `app` |

### TL — [Timeline & core editing](timeline-editing.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [TL-001](timeline-editing.md#tl-001-rational-time--frame-grid--mvp) | Rational time & frame grid | mvp | `timeline`, `media` |
| [TL-002](timeline-editing.md#tl-002-timecode-display--ruler--mvp) | Timecode display & ruler | mvp | `timeline`, `app` |
| [TL-003](timeline-editing.md#tl-003-sequence--timeline-duration--mvp) | Sequence & timeline duration | mvp | `timeline`, `project` |
| [TL-004](timeline-editing.md#tl-004-track-model-main-track-overlay-tracks-kinds--layer-order--mvp) | Track model: main track, overlay tracks, kinds & layer order | mvp | `timeline`, `project` |
| [TL-005](timeline-editing.md#tl-005-magnetic-main-track--mvp) | Magnetic main track | mvp | `timeline`, `app` |
| [TL-006](timeline-editing.md#tl-006-add-auto-create--remove-tracks--mvp) | Add, auto-create & remove tracks | mvp | `timeline`, `app` |
| [TL-007](timeline-editing.md#tl-007-reorder-tracks--flexible-stacking--v1) | Reorder tracks & flexible stacking | v1 | `timeline`, `app` |
| [TL-008](timeline-editing.md#tl-008-track-mute--hide--mvp) | Track mute & hide | mvp | `timeline`, `render`, `audio`, `app` |
| [TL-009](timeline-editing.md#tl-009-track-lock--v1) | Track lock | v1 | `timeline`, `app` |
| [TL-010](timeline-editing.md#tl-010-track-height-collapse--naming--v1) | Track height, collapse & naming | v1 | `timeline`, `project`, `app` |
| [TL-011](timeline-editing.md#tl-011-clip-model--mvp) | Clip model | mvp | `timeline` |
| [TL-012](timeline-editing.md#tl-012-add-media-to-the-timeline--mvp) | Add media to the timeline | mvp | `timeline`, `app` |
| [TL-013](timeline-editing.md#tl-013-playhead--navigation--mvp) | Playhead & navigation | mvp | `timeline`, `app` |
| [TL-014](timeline-editing.md#tl-014-hover-skim-preview--v1) | Hover skim preview | v1 | `render`, `app` |
| [TL-015](timeline-editing.md#tl-015-clip-selection--mvp) | Clip selection | mvp | `timeline`, `app` |
| [TL-016](timeline-editing.md#tl-016-marquee-selection--v1) | Marquee selection | v1 | `timeline`, `app` |
| [TL-017](timeline-editing.md#tl-017-select-forward--backward-from-playhead--v1) | Select forward / backward from playhead | v1 | `timeline`, `app` |
| [TL-018](timeline-editing.md#tl-018-move--drag-clips--mvp) | Move & drag clips | mvp | `timeline`, `app` |
| [TL-019](timeline-editing.md#tl-019-keyboard-nudge--reorder--v1) | Keyboard nudge & reorder | v1 | `timeline`, `app` |
| [TL-020](timeline-editing.md#tl-020-trim-clip-edges--mvp) | Trim clip edges | mvp | `timeline`, `app` |
| [TL-021](timeline-editing.md#tl-021-ripple-trim--ripple-across-tracks--v1) | Ripple trim & ripple across tracks | v1 | `timeline`, `app` |
| [TL-022](timeline-editing.md#tl-022-slip--roll-edits--later) | Slip & roll edits | later | `timeline`, `app` |
| [TL-023](timeline-editing.md#tl-023-split-at-playhead--mvp) | Split at playhead | mvp | `timeline`, `app` |
| [TL-024](timeline-editing.md#tl-024-blade-tool--v1) | Blade tool | v1 | `timeline`, `app` |
| [TL-025](timeline-editing.md#tl-025-delete--ripple-delete--mvp) | Delete & ripple delete | mvp | `timeline`, `app` |
| [TL-026](timeline-editing.md#tl-026-trim-to-playhead-cut-away-the-head-or-tail-at-the-playhead--v1) | Trim to playhead (cut away the head or tail at the playhead) | v1 | `timeline`, `app` |
| [TL-027](timeline-editing.md#tl-027-gaps-select-close-close-all--v1) | Gaps: select, close, close all | v1 | `timeline`, `app` |
| [TL-028](timeline-editing.md#tl-028-snapping--mvp) | Snapping | mvp | `timeline`, `app` |
| [TL-029](timeline-editing.md#tl-029-attached-items-following-main-track-clips--v1) | Attached items (following main-track clips) | v1 | `timeline`, `app` |
| [TL-030](timeline-editing.md#tl-030-embedded-audio-detach--restore--v1) | Embedded audio: detach & restore | v1 | `timeline`, `audio`, `app` |
| [TL-031](timeline-editing.md#tl-031-freeze-frame--v1) | Freeze frame | v1 | `timeline`, `media`, `render` |
| [TL-032](timeline-editing.md#tl-032-copy-cut--paste--v1) | Copy, cut & paste | v1 | `timeline`, `project`, `app` |
| [TL-033](timeline-editing.md#tl-033-duplicate--v1) | Duplicate | v1 | `timeline`, `app` |
| [TL-034](timeline-editing.md#tl-034-paste-attributes--v1) | Paste attributes | v1 | `timeline`, `app` |
| [TL-035](timeline-editing.md#tl-035-replace-clip-media--v1) | Replace clip media | v1 | `timeline`, `app` |
| [TL-036](timeline-editing.md#tl-036-enable--disable-clip--v1) | Enable / disable clip | v1 | `timeline`, `render`, `audio` |
| [TL-037](timeline-editing.md#tl-037-markers--v1) | Markers | v1 | `timeline`, `app` |
| [TL-038](timeline-editing.md#tl-038-inout-range-marks--v1) | In/out range marks | v1 | `timeline`, `project`, `app` |
| [TL-039](timeline-editing.md#tl-039-group--ungroup--v1) | Group / ungroup | v1 | `timeline`, `app` |
| [TL-040](timeline-editing.md#tl-040-compound-clips--v1) | Compound clips | v1 | `timeline`, `render`, `project`, `app` |
| [TL-041](timeline-editing.md#tl-041-zoom--scroll--mvp) | Zoom & scroll | mvp | `timeline`, `app` |
| [TL-042](timeline-editing.md#tl-042-timeline-thumbnails--waveforms--mvp) | Timeline thumbnails & waveforms | mvp | `media`, `app` |
| [TL-043](timeline-editing.md#tl-043-undo--redo--mvp) | Undo / redo | mvp | `timeline`, `app` |
| [TL-044](timeline-editing.md#tl-044-edit-history-panel--later) | Edit history panel | later | `timeline`, `app` |
| [TL-045](timeline-editing.md#tl-045-clip-color-labels--later) | Clip color labels | later | `timeline`, `app` |
| [TL-046](timeline-editing.md#tl-046-reveal-source--match-frame--later) | Reveal source & match frame | later | `timeline`, `app` |
| [TL-047](timeline-editing.md#tl-047-insert--overwrite-edits-from-the-source-preview--later) | Insert & overwrite edits from the source preview | later | `timeline`, `app` |
| [TL-048](timeline-editing.md#tl-048-rename-clip--v1) | Rename clip | v1 | `timeline`, `project`, `app` |
| [TL-049](timeline-editing.md#tl-049-track-color--v1) | Track color | v1 | `timeline`, `app` |
| [TL-050](timeline-editing.md#tl-050-sub-frame-edit-points-for-audio-only-clips--later) | Sub-frame edit points for audio-only clips | later | `timeline`, `app` |
| [TL-051](timeline-editing.md#tl-051-trim-several-clips-at-once--v1) | Trim several clips at once | v1 | `timeline`, `app` |
| [TL-052](timeline-editing.md#tl-052-join-clips-heal-a-through-edit--later) | Join clips (heal a through edit) | later | `timeline`, `app` |
| [TL-053](timeline-editing.md#tl-053-batch-edits-scopes-skipped-items--one-history-entry--v1) | Batch edits: scopes, skipped items & one history entry | v1 | `timeline` |
| [TL-054](timeline-editing.md#tl-054-operation-availability-per-clip--v1) | Operation availability per clip | v1 | `timeline`, `app` |
| [TL-055](timeline-editing.md#tl-055-clip-snippets-save-and-reuse-compound-clips--later) | Clip snippets: save and reuse compound clips | later | `timeline`, `project`, `app` |
| [TL-056](timeline-editing.md#tl-056-insert-another-project-as-a-compound-clip--later) | Insert another project as a compound clip | later | `timeline`, `project`, `app` |
| [TL-057](timeline-editing.md#tl-057-linked-selection--v1) | Linked selection | v1 | `timeline`, `app` |

### TR — [Transform, canvas, masks, keying, keyframes & speed](clip-transform-and-speed.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [TR-001](clip-transform-and-speed.md#tr-001-clip-transform-position-scale-rotation--mvp) | Clip transform: position, scale, rotation | mvp | `timeline`, `render`, `project`, `app` |
| [TR-002](clip-transform-and-speed.md#tr-002-fit-mode-and-the-100--baseline--mvp) | Fit mode and the 100 % baseline | mvp | `timeline`, `media`, `render` |
| [TR-003](clip-transform-and-speed.md#tr-003-flip-horizontal--vertical--mvp) | Flip horizontal / vertical | mvp | `timeline`, `render` |
| [TR-004](clip-transform-and-speed.md#tr-004-non-uniform-scale-independent-widthheight--v1) | Non-uniform scale (independent width/height) | v1 | `timeline`, `render`, `app` |
| [TR-005](clip-transform-and-speed.md#tr-005-quick-rotate-90--v1) | Quick rotate 90° | v1 | `timeline`, `app` |
| [TR-006](clip-transform-and-speed.md#tr-006-anchor-point--v1) | Anchor point | v1 | `timeline`, `render`, `app` |
| [TR-007](clip-transform-and-speed.md#tr-007-on-canvas-transform-handles-in-the-player--mvp) | On-canvas transform handles in the player | mvp | `timeline`, `app` |
| [TR-008](clip-transform-and-speed.md#tr-008-snapping-and-alignment-guides-on-the-canvas--v1) | Snapping and alignment guides on the canvas | v1 | `app` |
| [TR-009](clip-transform-and-speed.md#tr-009-crop-free-and-fixed-ratios--v1) | Crop (free and fixed ratios) | v1 | `timeline`, `render`, `app` |
| [TR-010](clip-transform-and-speed.md#tr-010-straighten-inside-the-crop-view--v1) | Straighten inside the crop view | v1 | `timeline`, `render`, `app` |
| [TR-011](clip-transform-and-speed.md#tr-011-canvas-background-solid-color--v1) | Canvas background: solid color | v1 | `timeline`, `render`, `app` |
| [TR-012](clip-transform-and-speed.md#tr-012-canvas-background-blurred-fill--v1) | Canvas background: blurred fill | v1 | `timeline`, `render` |
| [TR-013](clip-transform-and-speed.md#tr-013-canvas-background-image-fill--v1) | Canvas background: image fill | v1 | `timeline`, `render`, `app` |
| [TR-014](clip-transform-and-speed.md#tr-014-apply-background-to-all-clips--v1) | Apply background to all clips | v1 | `timeline`, `app` |
| [TR-015](clip-transform-and-speed.md#tr-015-opacity--v1) | Opacity | v1 | `timeline`, `render` |
| [TR-016](clip-transform-and-speed.md#tr-016-blend-mode-placement-in-the-compositing-section--v1) | Blend mode placement in the Compositing section | v1 | `timeline`, `render`, `app` |
| [TR-017](clip-transform-and-speed.md#tr-017-shape-masks--v1) | Shape masks | v1 | `timeline`, `render` |
| [TR-018](clip-transform-and-speed.md#tr-018-mask-feather-invert-and-shape-parameters--v1) | Mask feather, invert and shape parameters | v1 | `timeline`, `render` |
| [TR-019](clip-transform-and-speed.md#tr-019-on-canvas-mask-editing--v1) | On-canvas mask editing | v1 | `timeline`, `app` |
| [TR-020](clip-transform-and-speed.md#tr-020-multiple-masks-per-clip-with-combine-modes--later) | Multiple masks per clip with combine modes | later | `timeline`, `render`, `project` |
| [TR-021](clip-transform-and-speed.md#tr-021-freehand-pen-mask-bezier-path--later) | Freehand pen mask (Bezier path) | later | `timeline`, `render`, `app` |
| [TR-022](clip-transform-and-speed.md#tr-022-chroma-key-with-color-picker--v1) | Chroma key with color picker | v1 | `timeline`, `render`, `app` |
| [TR-023](clip-transform-and-speed.md#tr-023-spill-removal-and-edge-shrink--v1) | Spill removal and edge shrink | v1 | `timeline`, `render` |
| [TR-024](clip-transform-and-speed.md#tr-024-matte-preview-for-keys-and-masks--v1) | Matte preview for keys and masks | v1 | `render`, `app` |
| [TR-025](clip-transform-and-speed.md#tr-025-luma-key--later) | Luma key | later | `timeline`, `render` |
| [TR-026](clip-transform-and-speed.md#tr-026-video-stabilization--v1) | Video stabilization | v1 | `timeline`, `media`, `render` |
| [TR-027](clip-transform-and-speed.md#tr-027-keyframe-model-and-animatable-properties--v1) | Keyframe model and animatable properties | v1 | `timeline`, `project` |
| [TR-028](clip-transform-and-speed.md#tr-028-add-update-and-remove-keyframes-from-the-inspector--v1) | Add, update and remove keyframes from the inspector | v1 | `timeline`, `app` |
| [TR-029](clip-transform-and-speed.md#tr-029-keyframes-on-the-timeline-view-move-select-delete--v1) | Keyframes on the timeline: view, move, select, delete | v1 | `timeline`, `app` |
| [TR-030](clip-transform-and-speed.md#tr-030-keyframe-navigation--v1) | Keyframe navigation | v1 | `timeline`, `app` |
| [TR-031](clip-transform-and-speed.md#tr-031-interpolation-and-easing-presets--v1) | Interpolation and easing presets | v1 | `timeline` |
| [TR-032](clip-transform-and-speed.md#tr-032-custom-easing-curve-editor--v1) | Custom easing curve editor | v1 | `timeline`, `app` |
| [TR-033](clip-transform-and-speed.md#tr-033-copy-and-paste-keyframes--v1) | Copy and paste keyframes | v1 | `timeline`, `app` |
| [TR-034](clip-transform-and-speed.md#tr-034-this-areas-groups-in-paste-attributes--v1) | This area's groups in Paste attributes | v1 | `timeline` |
| [TR-035](clip-transform-and-speed.md#tr-035-curved-motion-paths--later) | Curved motion paths | later | `timeline`, `render`, `app` |
| [TR-036](clip-transform-and-speed.md#tr-036-retiming-model-time-map--v1) | Retiming model (time map) | v1 | `timeline`, `media`, `render`, `audio` |
| [TR-037](clip-transform-and-speed.md#tr-037-constant-speed--v1) | Constant speed | v1 | `timeline`, `app` |
| [TR-038](clip-transform-and-speed.md#tr-038-audio-follows-the-time-map--v1) | Audio follows the time map | v1 | `timeline`, `audio` |
| [TR-039](clip-transform-and-speed.md#tr-039-speed-curve-presets--v1) | Speed curve presets | v1 | `timeline`, `app` |
| [TR-040](clip-transform-and-speed.md#tr-040-custom-speed-curve-editor--v1) | Custom speed curve editor | v1 | `timeline`, `app` |
| [TR-041](clip-transform-and-speed.md#tr-041-frame-sampling-for-retimed-clips-nearest--frame-blend--v1) | Frame sampling for retimed clips (nearest / frame blend) | v1 | `timeline`, `media`, `render`, `project` |
| [TR-042](clip-transform-and-speed.md#tr-042-optical-flow-smooth-slow-motion--later) | Optical-flow smooth slow motion | later | `media`, `render` |
| [TR-043](clip-transform-and-speed.md#tr-043-reverse--v1) | Reverse | v1 | `timeline`, `media` |
| [TR-044](clip-transform-and-speed.md#tr-044-freeze-frame-still-frozen-frame-and-carried-over-settings--v1) | Freeze-frame still: frozen frame and carried-over settings | v1 | `timeline` |
| [TR-045](clip-transform-and-speed.md#tr-045-align-and-distribute-on-the-canvas--v1) | Align and distribute on the canvas | v1 | `timeline`, `app` |
| [TR-046](clip-transform-and-speed.md#tr-046-text-shaped-mask--later) | Text-shaped mask | later | `timeline`, `render`, `project` |
| [TR-047](clip-transform-and-speed.md#tr-047-perspective-tilt-plane-rotation--later) | Perspective tilt (plane rotation) | later | `timeline`, `render` |
| [TR-048](clip-transform-and-speed.md#tr-048-beat-synced-speed-ramp--later) | Beat-synced speed ramp | later | `timeline`, `app` |

### TX — [Text, titles & captions](text-and-captions.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [TX-001](text-and-captions.md#tx-001-add-a-text-clip--mvp) | Add a text clip | mvp | `timeline`, `render`, `project`, `app` |
| [TX-002](text-and-captions.md#tx-002-edit-text-content--mvp) | Edit text content | mvp | `timeline`, `app` |
| [TX-003](text-and-captions.md#tx-003-font-picker-bundled--system-fonts--mvp) | Font picker (bundled + system fonts) | mvp | `timeline`, `render`, `app` |
| [TX-004](text-and-captions.md#tx-004-size-fill-color-bold-and-italic--mvp) | Size, fill color, bold and italic | mvp | `timeline`, `render` |
| [TX-005](text-and-captions.md#tx-005-paragraph-layout-alignment-line-breaks-and-wrapping--mvp) | Paragraph layout: alignment, line breaks and wrapping | mvp | `timeline`, `render` |
| [TX-006](text-and-captions.md#tx-006-unicode-complex-scripts-and-font-fallback--mvp) | Unicode, complex scripts and font fallback | mvp | `render` |
| [TX-007](text-and-captions.md#tx-007-missing-fonts-when-opening-a-project--v1) | Missing fonts when opening a project | v1 | `timeline`, `render`, `app` |
| [TX-008](text-and-captions.md#tx-008-underline-strikethrough-and-letter-case--v1) | Underline, strikethrough and letter case | v1 | `timeline`, `render` |
| [TX-009](text-and-captions.md#tx-009-letter-and-line-spacing--v1) | Letter and line spacing | v1 | `timeline`, `render` |
| [TX-010](text-and-captions.md#tx-010-outline-stroke--v1) | Outline (stroke) | v1 | `timeline`, `render` |
| [TX-011](text-and-captions.md#tx-011-drop-shadow--v1) | Drop shadow | v1 | `timeline`, `render` |
| [TX-012](text-and-captions.md#tx-012-glow--v1) | Glow | v1 | `timeline`, `render` |
| [TX-013](text-and-captions.md#tx-013-background-box--v1) | Background box | v1 | `timeline`, `render` |
| [TX-014](text-and-captions.md#tx-014-gradient-fill--v1) | Gradient fill | v1 | `timeline`, `render` |
| [TX-015](text-and-captions.md#tx-015-per-range-rich-text-styling--v1) | Per-range (rich-text) styling | v1 | `timeline`, `render`, `app` |
| [TX-016](text-and-captions.md#tx-016-curved-text--v1) | Curved text | v1 | `timeline`, `render` |
| [TX-017](text-and-captions.md#tx-017-vertical-text-layout--later) | Vertical text layout | later | `timeline`, `render` |
| [TX-018](text-and-captions.md#tx-018-import-a-font-file--v1) | Import a font file | v1 | `render`, `app` |
| [TX-019](text-and-captions.md#tx-019-built-in-text-style-presets--v1) | Built-in text style presets | v1 | `timeline`, `project`, `app` |
| [TX-020](text-and-captions.md#tx-020-saved-styles-copypaste-style-and-default-style--v1) | Saved styles, copy/paste style and default style | v1 | `timeline`, `project`, `app` |
| [TX-021](text-and-captions.md#tx-021-keyframable-text-properties--v1) | Keyframable text properties | v1 | `timeline`, `render` |
| [TX-022](text-and-captions.md#tx-022-text-animations-in-out-and-loop--v1) | Text animations: In, Out and Loop | v1 | `timeline`, `render` |
| [TX-023](text-and-captions.md#tx-023-per-character-per-word-and-per-line-animations--v1) | Per-character, per-word and per-line animations | v1 | `timeline`, `render` |
| [TX-024](text-and-captions.md#tx-024-text-templates--v1) | Text templates | v1 | `timeline`, `project`, `app` |
| [TX-025](text-and-captions.md#tx-025-caption-tracks-and-segments--v1) | Caption tracks and segments | v1 | `timeline`, `render` |
| [TX-026](text-and-captions.md#tx-026-caption-editor-panel--v1) | Caption editor panel | v1 | `timeline`, `app` |
| [TX-027](text-and-captions.md#tx-027-split-merge-insert-and-delete-captions--v1) | Split, merge, insert and delete captions | v1 | `timeline`, `app` |
| [TX-028](text-and-captions.md#tx-028-caption-track-style-and-batch-styling--v1) | Caption track style and batch styling | v1 | `timeline`, `render` |
| [TX-029](text-and-captions.md#tx-029-caption-timing-tools--v1) | Caption timing tools | v1 | `timeline`, `app` |
| [TX-030](text-and-captions.md#tx-030-script-to-captions-and-tap-to-sync--v1) | Script to captions and tap-to-sync | v1 | `timeline`, `app` |
| [TX-031](text-and-captions.md#tx-031-import-subtitle-files--v1) | Import subtitle files | v1 | `timeline`, `project`, `app` |
| [TX-032](text-and-captions.md#tx-032-export-subtitle-files--v1) | Export subtitle files | v1 | `project`, `app` |
| [TX-033](text-and-captions.md#tx-033-captions-in-the-exported-video--v1) | Captions in the exported video | v1 | `media`, `render` |
| [TX-034](text-and-captions.md#tx-034-find-and-replace-in-text-and-captions--v1) | Find and replace in text and captions | v1 | `timeline`, `app` |
| [TX-035](text-and-captions.md#tx-035-convert-between-text-clips-and-captions--v1) | Convert between text clips and captions | v1 | `timeline` |
| [TX-036](text-and-captions.md#tx-036-bilingual-captions-secondary-line--v1) | Bilingual captions (secondary line) | v1 | `timeline`, `render` |
| [TX-037](text-and-captions.md#tx-037-caption-line-rules-and-re-flow--v1) | Caption line rules and re-flow | v1 | `timeline` |
| [TX-038](text-and-captions.md#tx-038-recognized-captions-from-speech--v1) | Recognized captions from speech | v1 | `timeline`, `analysis` |
| [TX-039](text-and-captions.md#tx-039-word-timings-and-active-word-highlight--v1) | Word timings and active-word highlight | v1 | `timeline`, `render` |
| [TX-040](text-and-captions.md#tx-040-automatic-keyword-emphasis--later) | Automatic keyword emphasis | later | `timeline`, `analysis` |
| [TX-041](text-and-captions.md#tx-041-translated-captions--later) | Translated captions | later | `timeline`, `analysis` |
| [TX-042](text-and-captions.md#tx-042-speaker-labels--later) | Speaker labels | later | `timeline`, `project` |
| [TX-043](text-and-captions.md#tx-043-speech-from-text-and-captions--later) | Speech from text and captions | later | `timeline`, `media`, `analysis` |
| [TX-044](text-and-captions.md#tx-044-lyric-captions--v1) | Lyric captions | v1 | `timeline`, `analysis` |
| [TX-045](text-and-captions.md#tx-045-dictate-text-with-the-local-speech-model--later) | Dictate text with the local speech model | later | `timeline`, `audio`, `analysis`, `app` |
| [TX-046](text-and-captions.md#tx-046-image-texture-fill--later) | Image (texture) fill | later | `timeline`, `media`, `render` |
| [TX-047](text-and-captions.md#tx-047-multi-color-word-fill--later) | Multi-color word fill | later | `timeline`, `render` |
| [TX-048](text-and-captions.md#tx-048-stacked-outlines-and-shadows-inner-and-long-shadows--later) | Stacked outlines and shadows, inner and long shadows | later | `timeline`, `render` |
| [TX-049](text-and-captions.md#tx-049-readability-and-frame-fit-checks--later) | Readability and frame-fit checks | later | `render`, `app` |
| [TX-050](text-and-captions.md#tx-050-font-collections--later) | Font collections | later | `app` |
| [TX-051](text-and-captions.md#tx-051-caption-looks-animated-caption-presets--v1) | Caption looks (animated caption presets) | v1 | `timeline`, `render`, `project`, `app` |
| [TX-052](text-and-captions.md#tx-052-progressive-caption-display-word-line-and-screen--later) | Progressive caption display: word, line and screen | later | `timeline`, `render` |
| [TX-053](text-and-captions.md#tx-053-automatic-emoji-on-captions--later) | Automatic emoji on captions | later | `timeline`, `app` |
| [TX-054](text-and-captions.md#tx-054-text-inspector-lens-contents-and-card-behaviour--v1) | Text inspector: lens contents and card behaviour | v1 | `timeline`, `app` |
| [TX-055](text-and-captions.md#tx-055-bubble-and-tag-boxes--v1) | Bubble and tag boxes | v1 | `timeline`, `render` |
| [TX-056](text-and-captions.md#tx-056-text-animation-preset-catalogue--v1) | Text animation preset catalogue | v1 | `timeline`, `render`, `project`, `app` |
| [TX-057](text-and-captions.md#tx-057-captions-drawer--v1) | Captions drawer | v1 | `analysis`, `app` |

### AU — [Audio](audio.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [AU-001](audio.md#au-001-audio-tracks-and-audio-clips--mvp) | Audio tracks and audio clips | mvp | `timeline`, `media`, `audio`, `project` |
| [AU-002](audio.md#au-002-audio-inside-video-clips--mvp) | Audio inside video clips | mvp | `timeline`, `media`, `audio` |
| [AU-003](audio.md#au-003-detached-audio-settings-hand-off-and-transitions--v1) | Detached audio: settings hand-off and transitions | v1 | `timeline` |
| [AU-004](audio.md#au-004-pull-the-sound-out-of-a-video-file-in-the-media-library--v1) | Pull the sound out of a video file in the media library | v1 | `timeline`, `media` |
| [AU-005](audio.md#au-005-clip-volume--mvp) | Clip volume | mvp | `timeline`, `audio`, `app` |
| [AU-006](audio.md#au-006-fade-in--fade-out--mvp) | Fade in / fade out | mvp | `timeline`, `audio` |
| [AU-007](audio.md#au-007-clip-mute--mvp) | Clip mute | mvp | `timeline`, `audio` |
| [AU-008](audio.md#au-008-volume-keyframes-and-the-on-clip-volume-line--v1) | Volume keyframes and the on-clip volume line | v1 | `timeline`, `audio` |
| [AU-009](audio.md#au-009-gain-overlay-on-waveforms--mvp) | Gain overlay on waveforms | mvp | `timeline`, `media`, `app` |
| [AU-010](audio.md#au-010-mixing-engine-master-bus-and-playback-output--mvp) | Mixing engine, master bus and playback output | mvp | `timeline`, `media`, `audio`, `project` |
| [AU-011](audio.md#au-011-track-mute-in-the-mixer--mvp) | Track mute in the mixer | mvp | `timeline`, `audio` |
| [AU-012](audio.md#au-012-track-solo--v1) | Track solo | v1 | `audio`, `app` |
| [AU-013](audio.md#au-013-track-volume-pan-and-mixer-panel--v1) | Track volume, pan and mixer panel | v1 | `timeline`, `audio`, `app` |
| [AU-014](audio.md#au-014-clip-pan-and-channel-mapping--v1) | Clip pan and channel mapping | v1 | `timeline`, `audio` |
| [AU-015](audio.md#au-015-level-meters-and-loudness-readout--v1) | Level meters and loudness readout | v1 | `audio`, `app` |
| [AU-016](audio.md#au-016-audio-scrubbing--v1) | Audio scrubbing | v1 | `audio` |
| [AU-017](audio.md#au-017-loudness-normalization--v1) | Loudness normalization | v1 | `media`, `audio`, `project` |
| [AU-018](audio.md#au-018-noise-reduction--v1) | Noise reduction | v1 | `media`, `audio` |
| [AU-019](audio.md#au-019-voice-polish--v1) | Voice polish | v1 | `audio` |
| [AU-020](audio.md#au-020-voice-clarity-ml-speech-enhancement--later) | Voice clarity (ML speech enhancement) | later | `timeline`, `media`, `audio`, `analysis` |
| [AU-021](audio.md#au-021-voice-isolation-and-split-into-stems--later) | Voice isolation and "Split into stems" | later | `timeline`, `media`, `audio`, `analysis` |
| [AU-022](audio.md#au-022-voice-effects-voice-changer--v1) | Voice effects (voice changer) | v1 | `timeline`, `audio` |
| [AU-023](audio.md#au-023-keep-pitch-and-the-time-stretch-engine--v1) | Keep pitch and the time-stretch engine | v1 | `timeline`, `audio` |
| [AU-024](audio.md#au-024-pitch-shift--v1) | Pitch shift | v1 | `timeline`, `audio` |
| [AU-025](audio.md#au-025-beat-detection-and-beat-markers--v1) | Beat detection and beat markers | v1 | `timeline`, `media`, `audio`, `project`, `app` |
| [AU-026](audio.md#au-026-tap-beat-latency-compensation--v1) | Tap-beat latency compensation | v1 | `audio`, `app` |
| [AU-027](audio.md#au-027-voiceover-recording--v1) | Voiceover recording | v1 | `timeline`, `audio`, `app` |
| [AU-028](audio.md#au-028-automatic-ducking--v1) | Automatic ducking | v1 | `timeline`, `audio` |
| [AU-029](audio.md#au-029-audio-crossfades--v1) | Audio crossfades | v1 | `timeline`, `audio` |
| [AU-030](audio.md#au-030-music-library-panel--v1) | Music library panel | v1 | `media`, `app` |
| [AU-031](audio.md#au-031-sound-effects-library--v1) | Sound effects library | v1 | `media`, `app` |
| [AU-032](audio.md#au-032-audio-only-export--v1) | Audio-only export | v1 | `media`, `audio`, `project` |
| [AU-033](audio.md#au-033-stems-and-separate-audio-export--later) | Stems and separate-audio export | later | `media`, `audio` |
| [AU-034](audio.md#au-034-basic-audio-filters-eq-filters-dynamics--later) | Basic audio filters (EQ, filters, dynamics) | later | `timeline`, `audio` |
| [AU-035](audio.md#au-035-waveform-based-sync-of-external-audio--later) | Waveform-based sync of external audio | later | `timeline`, `audio` |
| [AU-036](audio.md#au-036-keyboard-volume-steps--v1) | Keyboard volume steps | v1 | `timeline`, `app` |
| [AU-037](audio.md#au-037-echo-reduction--v1) | Echo reduction | v1 | `timeline`, `audio` |
| [AU-038](audio.md#au-038-multi-stream-sources-stream-notice-picker-and-stream-as-clip--v1) | Multi-stream sources: stream notice, picker and stream as clip | v1 | `timeline`, `media`, `audio`, `app` |
| [AU-039](audio.md#au-039-mute-all-video-sound-and-project-mute--v1) | Mute all video sound and project mute | v1 | `timeline`, `audio`, `app` |
| [AU-040](audio.md#au-040-audio-tool-availability-and-batch-results--v1) | Audio tool availability and batch results | v1 | `timeline`, `media`, `app` |
| [AU-041](audio.md#au-041-beforeafter-compare-for-audio-processing--v1) | Before/after compare for audio processing | v1 | `audio`, `app` |
| [AU-042](audio.md#au-042-audio-auto-fix-assistants--later) | Audio auto-fix assistants | later | `timeline`, `audio`, `project`, `app` |

### CL — [Color, adjustments & filters](color-and-filters.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [CL-001](color-and-filters.md#cl-001-color-stack-on-visual-items--v1) | Color stack on visual items | v1 | `timeline`, `render`, `project` |
| [CL-002](color-and-filters.md#cl-002-color-managed-decode-and-encode-baseline--mvp) | Color-managed decode and encode baseline | mvp | `media`, `render` |
| [CL-003](color-and-filters.md#cl-003-filter-library-browser--v1) | Filter library browser | v1 | `render`, `project`, `app` |
| [CL-004](color-and-filters.md#cl-004-apply-filter-to-a-clip-with-intensity--v1) | Apply filter to a clip, with intensity | v1 | `timeline`, `render` |
| [CL-005](color-and-filters.md#cl-005-bundled-freecut-look-presets--v1) | Bundled FreeCut look presets | v1 | `render` |
| [CL-006](color-and-filters.md#cl-006-adjustment-clips-filter-and-adjustment-layers--v1) | Adjustment clips (filter and adjustment layers) | v1 | `timeline`, `render` |
| [CL-007](color-and-filters.md#cl-007-light-adjustments--v1) | Light adjustments | v1 | `timeline`, `render` |
| [CL-008](color-and-filters.md#cl-008-color-balance-temperature-tint-saturation-vibrance--v1) | Color balance: temperature, tint, saturation, vibrance | v1 | `timeline`, `render` |
| [CL-009](color-and-filters.md#cl-009-detail-sharpen-and-clarity--v1) | Detail: sharpen and clarity | v1 | `timeline`, `render` |
| [CL-010](color-and-filters.md#cl-010-vignette--v1) | Vignette | v1 | `timeline`, `render` |
| [CL-011](color-and-filters.md#cl-011-film-grain--v1) | Film grain | v1 | `timeline`, `render` |
| [CL-012](color-and-filters.md#cl-012-fade-matte--v1) | Fade (matte) | v1 | `timeline`, `render` |
| [CL-013](color-and-filters.md#cl-013-hsl-per-hue-band--v1) | HSL per hue band | v1 | `timeline`, `render` |
| [CL-014](color-and-filters.md#cl-014-curves--v1) | Curves | v1 | `timeline`, `render` |
| [CL-015](color-and-filters.md#cl-015-color-wheels--v1) | Color wheels | v1 | `timeline`, `render` |
| [CL-016](color-and-filters.md#cl-016-lut-import-and-lut-library--v1) | LUT import and LUT library | v1 | `timeline`, `media`, `render`, `project`, `app` |
| [CL-017](color-and-filters.md#cl-017-apply-lut-with-intensity--v1) | Apply LUT with intensity | v1 | `timeline`, `render` |
| [CL-018](color-and-filters.md#cl-018-white-balance-picker--v1) | White-balance picker | v1 | `render`, `app` |
| [CL-019](color-and-filters.md#cl-019-auto-color--v1) | Auto color | v1 | `media`, `render`, `app` |
| [CL-020](color-and-filters.md#cl-020-color-match-to-a-reference--v1) | Color match to a reference | v1 | `timeline`, `render` |
| [CL-021](color-and-filters.md#cl-021-histogram--v1) | Histogram | v1 | `render`, `app` |
| [CL-022](color-and-filters.md#cl-022-waveform-rgb-parade-and-vectorscope--later) | Waveform, RGB parade and vectorscope | later | `render`, `app` |
| [CL-023](color-and-filters.md#cl-023-bypass-and-beforeafter-compare--v1) | Bypass and before/after compare | v1 | `timeline`, `render`, `app` |
| [CL-024](color-and-filters.md#cl-024-reset--v1) | Reset | v1 | `timeline`, `app` |
| [CL-025](color-and-filters.md#cl-025-apply-to-all-and-copypaste-color--v1) | Apply to all, and copy/paste color | v1 | `timeline` |
| [CL-026](color-and-filters.md#cl-026-save-as-look-preset-user-looks--v1) | Save as look preset (user looks) | v1 | `project`, `app` |
| [CL-027](color-and-filters.md#cl-027-share-looks-and-export-a-grade-as-lut--later) | Share looks and export a grade as LUT | later | `media`, `render` |
| [CL-028](color-and-filters.md#cl-028-keyframed-color-parameters--v1) | Keyframed color parameters | v1 | `timeline`, `render` |
| [CL-029](color-and-filters.md#cl-029-secondary-hue-and-luma-curves--later) | Secondary hue and luma curves | later | `timeline`, `render` |
| [CL-030](color-and-filters.md#cl-030-skin-protection--v1) | Skin protection | v1 | `timeline`, `render`, `app` |
| [CL-031](color-and-filters.md#cl-031-masked-local-color-adjustments--v1) | Masked (local) color adjustments | v1 | `timeline`, `render`, `app` |
| [CL-032](color-and-filters.md#cl-032-input-transform-for-log-and-camera-footage--later) | Input transform for log and camera footage | later | `timeline`, `render` |
| [CL-033](color-and-filters.md#cl-033-video-noise-reduction-denoise--v1) | Video noise reduction (Denoise) | v1 | `timeline`, `media`, `render` |
| [CL-034](color-and-filters.md#cl-034-deflicker--v1) | Deflicker | v1 | `timeline`, `media`, `render`, `app` |
| [CL-035](color-and-filters.md#cl-035-overall-color-amount--v1) | Overall color amount | v1 | `timeline`, `render` |
| [CL-036](color-and-filters.md#cl-036-dehaze--later) | Dehaze | later | `timeline`, `render` |
| [CL-037](color-and-filters.md#cl-037-custom-color-targets-hsl-picks--later) | Custom color targets (HSL picks) | later | `timeline`, `render` |
| [CL-038](color-and-filters.md#cl-038-log-range-color-wheels--later) | Log-range color wheels | later | `timeline`, `render` |
| [CL-039](color-and-filters.md#cl-039-match-several-clips-to-one-reference--v1) | Match several clips to one reference | v1 | `timeline`, `render`, `app` |
| [CL-040](color-and-filters.md#cl-040-look-parameters--later) | Look parameters | later | `timeline`, `render` |
| [CL-041](color-and-filters.md#cl-041-hdr-source-conversion-options--v1) | HDR source conversion options | v1 | `timeline`, `render`, `app` |
| [CL-042](color-and-filters.md#cl-042-grade-after-clip-effects--later) | Grade after clip effects | later | `timeline`, `render` |
| [CL-043](color-and-filters.md#cl-043-color-tab-pages-and-section-controls--v1) | Color tab pages and section controls | v1 | `timeline`, `app` |
| [CL-044](color-and-filters.md#cl-044-adjust-tab-in-the-asset-panel--v1) | Adjust tab in the asset panel | v1 | `timeline`, `project`, `render`, `app` |
| [CL-045](color-and-filters.md#cl-045-bundled-freecut-luts--v1) | Bundled FreeCut LUTs | v1 | `media`, `render`, `app` |

### FX — [Effects, transitions & clip animations](effects-transitions-animations.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [FX-001](effects-transitions-animations.md#fx-001-per-frame-compositing-pipeline--mvp) | Per-frame compositing pipeline | mvp | `timeline`, `render` |
| [FX-002](effects-transitions-animations.md#fx-002-transitions-between-adjacent-clips--mvp) | Transitions between adjacent clips | mvp | `timeline`, `render`, `project` |
| [FX-003](effects-transitions-animations.md#fx-003-cross-dissolve--mvp) | Cross-dissolve | mvp | `render` |
| [FX-004](effects-transitions-animations.md#fx-004-transition-library--v1) | Transition library | v1 | `render`, `app` |
| [FX-005](effects-transitions-animations.md#fx-005-transition-parameters--v1) | Transition parameters | v1 | `timeline`, `render` |
| [FX-006](effects-transitions-animations.md#fx-006-apply-to-every-cut--v1) | Apply to every cut | v1 | `timeline` |
| [FX-007](effects-transitions-animations.md#fx-007-audio-during-transitions--mvp) | Audio during transitions | mvp | `timeline`, `audio` |
| [FX-008](effects-transitions-animations.md#fx-008-transitions-that-keep-timeline-length-handle-mode--later) | Transitions that keep timeline length (handle mode) | later | `timeline`, `render` |
| [FX-009](effects-transitions-animations.md#fx-009-default-transition-and-quick-add--v1) | Default transition and quick add | v1 | `timeline`, `app` |
| [FX-010](effects-transitions-animations.md#fx-010-clip-effects--v1) | Clip effects | v1 | `timeline`, `render` |
| [FX-011](effects-transitions-animations.md#fx-011-effect-layers--v1) | Effect layers | v1 | `timeline`, `render` |
| [FX-012](effects-transitions-animations.md#fx-012-effect-parameters--v1) | Effect parameters | v1 | `timeline`, `render`, `app` |
| [FX-013](effects-transitions-animations.md#fx-013-effect-stacking-order--v1) | Effect stacking order | v1 | `timeline` |
| [FX-014](effects-transitions-animations.md#fx-014-effect-parameter-keyframes--v1) | Effect parameter keyframes | v1 | `timeline`, `render` |
| [FX-015](effects-transitions-animations.md#fx-015-built-in-video-effect-library--v1) | Built-in video effect library | v1 | `render` |
| [FX-016](effects-transitions-animations.md#fx-016-library-browsing-search-favorites-preview--v1) | Library browsing: search, favorites, preview | v1 | `render`, `app` |
| [FX-017](effects-transitions-animations.md#fx-017-copy-and-paste-effects-animations-and-transitions--v1) | Copy and paste effects, animations and transitions | v1 | `timeline`, `app` |
| [FX-018](effects-transitions-animations.md#fx-018-entrance-and-exit-animations--v1) | Entrance and exit animations | v1 | `timeline`, `render` |
| [FX-019](effects-transitions-animations.md#fx-019-continuous-animations--v1) | Continuous animations | v1 | `timeline` |
| [FX-020](effects-transitions-animations.md#fx-020-person-tracking-effects--later) | Person-tracking effects | later | `render`, `analysis` |
| [FX-021](effects-transitions-animations.md#fx-021-face-tracking-effects--later) | Face-tracking effects | later | `render`, `analysis` |
| [FX-022](effects-transitions-animations.md#fx-022-motion-blur-for-animated-movement--later) | Motion blur for animated movement | later | `timeline`, `render` |
| [FX-023](effects-transitions-animations.md#fx-023-custom-wipe-transitions-from-user-images--later) | Custom wipe transitions from user images | later | `timeline`, `media`, `render` |
| [FX-024](effects-transitions-animations.md#fx-024-saved-effect-presets--later) | Saved effect presets | later | `timeline`, `project`, `app` |
| [FX-025](effects-transitions-animations.md#fx-025-custom-shader-effects-advanced--later) | Custom shader effects (advanced) | later | `render`, `project` |
| [FX-026](effects-transitions-animations.md#fx-026-apply-an-effect-to-all-clips--v1) | Apply an effect to all clips | v1 | `timeline`, `app` |
| [FX-027](effects-transitions-animations.md#fx-027-effect-regions-masks-on-effects--v1) | Effect regions (masks on effects) | v1 | `timeline`, `render`, `app` |
| [FX-028](effects-transitions-animations.md#fx-028-motion-blur-from-footage--later) | Motion blur from footage | later | `timeline`, `media`, `render` |
| [FX-029](effects-transitions-animations.md#fx-029-photo-parallax--later) | Photo parallax | later | `render`, `analysis` |
| [FX-030](effects-transitions-animations.md#fx-030-animation-motion-definitions-and-preset-manifest--v1) | Animation motion definitions and preset manifest | v1 | `timeline`, `project` |

### OV — [Overlays, stickers, shapes & templates](overlays-and-stickers.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [OV-001](overlays-and-stickers.md#ov-001-overlay-clips-on-upper-tracks-picture-in-picture--mvp) | Overlay clips on upper tracks (picture-in-picture) | mvp | `timeline`, `render` |
| [OV-002](overlays-and-stickers.md#ov-002-stack-several-items-as-layers-at-the-playhead--v1) | Stack several items as layers at the playhead | v1 | `timeline` |
| [OV-003](overlays-and-stickers.md#ov-003-picture-in-picture-position-presets--v1) | Picture-in-picture position presets | v1 | `timeline` |
| [OV-004](overlays-and-stickers.md#ov-004-layer-blend-modes--v1) | Layer blend modes | v1 | `timeline`, `render`, `project`, `app` |
| [OV-005](overlays-and-stickers.md#ov-005-overlay-border-and-rounded-corners--v1) | Overlay border and rounded corners | v1 | `timeline`, `render` |
| [OV-006](overlays-and-stickers.md#ov-006-element-outline-and-drop-shadow--v1) | Element outline and drop shadow | v1 | `timeline`, `render` |
| [OV-007](overlays-and-stickers.md#ov-007-layer-order-commands--v1) | Layer order commands | v1 | `timeline` |
| [OV-008](overlays-and-stickers.md#ov-008-click-through-selection-in-the-preview--v1) | Click-through selection in the preview | v1 | `timeline`, `render`, `app` |
| [OV-009](overlays-and-stickers.md#ov-009-span-an-overlay-across-the-project--v1) | Span an overlay across the project | v1 | `timeline` |
| [OV-010](overlays-and-stickers.md#ov-010-replace-an-elements-source-keeping-its-settings--v1) | Replace an element's source, keeping its settings | v1 | `timeline`, `media` |
| [OV-011](overlays-and-stickers.md#ov-011-still-image-overlays-with-transparency--mvp) | Still-image overlays with transparency | mvp | `media`, `render` |
| [OV-012](overlays-and-stickers.md#ov-012-animated-image-overlays-gif-apng-animated-webp--v1) | Animated image overlays (GIF, APNG, animated WebP) | v1 | `timeline`, `media` |
| [OV-013](overlays-and-stickers.md#ov-013-animated-element-loop--playback-options--v1) | Animated element loop & playback options | v1 | `timeline` |
| [OV-014](overlays-and-stickers.md#ov-014-video-overlays-with-alpha--v1) | Video overlays with alpha | v1 | `media` |
| [OV-015](overlays-and-stickers.md#ov-015-svg-vector-overlays--v1) | SVG vector overlays | v1 | `timeline`, `media`, `render` |
| [OV-016](overlays-and-stickers.md#ov-016-lottie-style-vector-animations--later) | Lottie-style vector animations | later | `media`, `render` |
| [OV-017](overlays-and-stickers.md#ov-017-looping-image-sequence-overlays--later) | Looping image-sequence overlays | later | `timeline`, `media` |
| [OV-018](overlays-and-stickers.md#ov-018-sticker-clips--v1) | Sticker clips | v1 | `timeline`, `project`, `app` |
| [OV-019](overlays-and-stickers.md#ov-019-bundled-sticker-packs--v1) | Bundled sticker packs | v1 | `media`, `app` |
| [OV-020](overlays-and-stickers.md#ov-020-import-your-own-stickers-my-stickers--v1) | Import your own stickers ("My stickers") | v1 | `media`, `app` |
| [OV-021](overlays-and-stickers.md#ov-021-emoji-elements--v1) | Emoji elements | v1 | `timeline`, `render` |
| [OV-022](overlays-and-stickers.md#ov-022-arrow-tips-that-follow-a-tracked-object--later) | Arrow tips that follow a tracked object | later | `timeline` |
| [OV-023](overlays-and-stickers.md#ov-023-make-a-sticker-from-a-cut-out--later) | Make a sticker from a cut-out | later | `media`, `analysis`, `app` |
| [OV-024](overlays-and-stickers.md#ov-024-shape-elements--v1) | Shape elements | v1 | `timeline`, `render` |
| [OV-025](overlays-and-stickers.md#ov-025-shape-fill-and-stroke--v1) | Shape fill and stroke | v1 | `timeline`, `render` |
| [OV-026](overlays-and-stickers.md#ov-026-shape-gradient-fills--later) | Shape gradient fills | later | `timeline`, `render` |
| [OV-027](overlays-and-stickers.md#ov-027-lines-and-arrows--v1) | Lines and arrows | v1 | `timeline`, `render` |
| [OV-028](overlays-and-stickers.md#ov-028-speech-bubble-and-callout-shapes--later) | Speech-bubble and callout shapes | later | `timeline`, `render` |
| [OV-029](overlays-and-stickers.md#ov-029-decorative-frames-and-borders--v1) | Decorative frames and borders | v1 | `timeline`, `render` |
| [OV-030](overlays-and-stickers.md#ov-030-save-project-as-template--v1) | Save project as template | v1 | `project`, `app` |
| [OV-031](overlays-and-stickers.md#ov-031-replaceable-placeholder-slots--v1) | Replaceable placeholder slots | v1 | `timeline` |
| [OV-032](overlays-and-stickers.md#ov-032-start-a-project-from-a-template-and-fill-its-slots--v1) | Start a project from a template and fill its slots | v1 | `timeline`, `media`, `project`, `app` |
| [OV-033](overlays-and-stickers.md#ov-033-template-library-management--v1) | Template library management | v1 | `project`, `app` |
| [OV-034](overlays-and-stickers.md#ov-034-template-package-export-and-import--later) | Template package export and import | later | `project`, `app` |
| [OV-035](overlays-and-stickers.md#ov-035-bundled-starter-templates--later) | Bundled starter templates | later | `project`, `app` |
| [OV-036](overlays-and-stickers.md#ov-036-elements-library-panel--v1) | Elements library panel | v1 | `media`, `app` |
| [OV-037](overlays-and-stickers.md#ov-037-library-search-and-tags--v1) | Library search and tags | v1 | `app` |
| [OV-038](overlays-and-stickers.md#ov-038-favorites--v1) | Favorites | v1 | `app` |
| [OV-039](overlays-and-stickers.md#ov-039-recently-used--v1) | Recently used | v1 | `app` |
| [OV-040](overlays-and-stickers.md#ov-040-asset-packs-with-licence-metadata--v1) | Asset packs with licence metadata | v1 | `project`, `app` |
| [OV-041](overlays-and-stickers.md#ov-041-credits-sheet-for-used-assets--v1) | Credits sheet for used assets | v1 | `app` |
| [OV-042](overlays-and-stickers.md#ov-042-user-collections-for-imported-stickers--later) | User collections for imported stickers | later | `app` |
| [OV-043](overlays-and-stickers.md#ov-043-local-brand-kit--later) | Local brand kit | later | `app` |
| [OV-044](overlays-and-stickers.md#ov-044-element-style-presets--later) | Element style presets | later | `timeline`, `app` |
| [OV-045](overlays-and-stickers.md#ov-045-multi-overlay-layout-presets-split-screen--later) | Multi-overlay layout presets (split screen) | later | `timeline` |
| [OV-046](overlays-and-stickers.md#ov-046-transform-a-group-as-one-in-the-preview--later) | Transform a group as one in the preview | later | `timeline`, `app` |
| [OV-047](overlays-and-stickers.md#ov-047-animated-stickers-from-numbered-frames--later) | Animated stickers from numbered frames | later | `media`, `app` |
| [OV-048](overlays-and-stickers.md#ov-048-export-stickers-as-a-shareable-pack--later) | Export stickers as a shareable pack | later | `project`, `app` |
| [OV-049](overlays-and-stickers.md#ov-049-custom-path-shapes-drawn-with-a-pen-tool--later) | Custom path shapes drawn with a pen tool | later | `timeline`, `render`, `app` |
| [OV-050](overlays-and-stickers.md#ov-050-image-fill-for-shapes--later) | Image fill for shapes | later | `timeline`, `render` |
| [OV-051](overlays-and-stickers.md#ov-051-chart-elements-and-the-chart-editor--later) | Chart elements and the chart editor | later | `timeline`, `render`, `project`, `app` |
| [OV-052](overlays-and-stickers.md#ov-052-chart-data-table-editing-and-spreadsheet-import--later) | Chart data: table editing and spreadsheet import | later | `timeline`, `app` |
| [OV-053](overlays-and-stickers.md#ov-053-chart-styling--later) | Chart styling | later | `timeline`, `render` |
| [OV-054](overlays-and-stickers.md#ov-054-chart-build-animation--later) | Chart build animation | later | `timeline`, `render` |

### AI — [Smart / AI-assisted tools](ai-tools.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [AI-001](ai-tools.md#ai-001-background-analysis-jobs--result-cache--v1) | Background analysis jobs & result cache | v1 | `timeline`, `media`, `analysis`, `app` |
| [AI-002](ai-tools.md#ai-002-local-model-manager--ai-privacy-settings--v1) | Local model manager & AI privacy settings | v1 | `analysis`, `app` |
| [AI-003](ai-tools.md#ai-003-silence-detection--removal--v1) | Silence detection & removal | v1 | `timeline`, `media`, `analysis`, `app` |
| [AI-004](ai-tools.md#ai-004-scene--shot-detection--auto-split--v1) | Scene / shot detection & auto split | v1 | `timeline`, `media`, `analysis` |
| [AI-005](ai-tools.md#ai-005-beat-grid-refinement--v1) | Beat grid refinement | v1 | `timeline`, `audio` |
| [AI-006](ai-tools.md#ai-006-auto-beat-sync-fit-clips-to-beats--v1) | Auto beat sync (fit clips to beats) | v1 | `timeline` |
| [AI-007](ai-tools.md#ai-007-stabilization-aware-analysis--v1) | Stabilization-aware analysis | v1 | `render`, `app` |
| [AI-008](ai-tools.md#ai-008-motion-tracking-attach-to-a-moving-object--v1) | Motion tracking (attach to a moving object) | v1 | `timeline`, `render`, `analysis` |
| [AI-009](ai-tools.md#ai-009-local-speech-transcription--v1) | Local speech transcription | v1 | `media`, `project`, `analysis` |
| [AI-010](ai-tools.md#ai-010-auto-captions--v1) | Auto captions | v1 | `timeline`, `analysis` |
| [AI-011](ai-tools.md#ai-011-transcript-based-editing--later) | Transcript-based editing | later | `timeline`, `app` |
| [AI-012](ai-tools.md#ai-012-filler-word-pause--repeat-cleanup--later) | Filler-word, pause & repeat cleanup | later | `timeline`, `analysis` |
| [AI-013](ai-tools.md#ai-013-caption-translation--bilingual-captions--later) | Caption translation & bilingual captions | later | `timeline`, `analysis` |
| [AI-014](ai-tools.md#ai-014-background-removal-automatic-cutout--later) | Background removal (automatic cutout) | later | `media`, `render`, `analysis` |
| [AI-015](ai-tools.md#ai-015-cutout-refinement--object-selection--later) | Cutout refinement & object selection | later | `timeline`, `analysis` |
| [AI-016](ai-tools.md#ai-016-auto-reframe--later) | Auto reframe | later | `timeline`, `analysis` |
| [AI-017](ai-tools.md#ai-017-subject-follow-automatic-pan--zoom--later) | Subject follow (automatic pan & zoom) | later | `timeline`, `analysis` |
| [AI-018](ai-tools.md#ai-018-ai-frame-interpolation-for-smooth-slow-motion--later) | AI frame interpolation for smooth slow motion | later | `media`, `render`, `analysis` |
| [AI-019](ai-tools.md#ai-019-upscale--enhance-quality--later) | Upscale & enhance quality | later | `media`, `analysis` |
| [AI-020](ai-tools.md#ai-020-face-retouch--later) | Face retouch | later | `render`, `analysis` |
| [AI-021](ai-tools.md#ai-021-automatic-face-blur-privacy--later) | Automatic face blur (privacy) | later | `render`, `analysis` |
| [AI-022](ai-tools.md#ai-022-object-removal-video-inpainting--later) | Object removal (video inpainting) | later | `media`, `analysis` |
| [AI-023](ai-tools.md#ai-023-voice--stem-separation-engine--later) | Voice & stem separation engine | later | `media`, `audio`, `analysis` |
| [AI-024](ai-tools.md#ai-024-speech-enhancement-model-engine--later) | Speech-enhancement model engine | later | `media`, `audio`, `analysis` |
| [AI-025](ai-tools.md#ai-025-text-to-speech-engine--voices--later) | Text-to-speech engine & voices | later | `media`, `analysis` |
| [AI-026](ai-tools.md#ai-026-generative-media-extension-point--later) | Generative media extension point | later | `app` |
| [AI-027](ai-tools.md#ai-027-search-media-by-spoken-words--later) | Search media by spoken words | later | `analysis`, `app` |
| [AI-028](ai-tools.md#ai-028-colorize-black-and-white-footage--later) | Colorize black-and-white footage | later | `media`, `analysis` |
| [AI-029](ai-tools.md#ai-029-auto-montage-beat-synced-auto-edit--later) | Auto montage (beat-synced auto edit) | later | `timeline`, `analysis` |
| [AI-030](ai-tools.md#ai-030-smart-camera-moves-subject--and-beat-aware--later) | Smart camera moves (subject- and beat-aware) | later | `timeline`, `analysis` |
| [AI-031](ai-tools.md#ai-031-planar-tracking-corner-pin--later) | Planar tracking (corner pin) | later | `timeline`, `render`, `analysis` |
| [AI-032](ai-tools.md#ai-032-retouch-presets--later) | Retouch presets | later | `timeline`, `app` |
| [AI-033](ai-tools.md#ai-033-makeup--hair-color-tints--later) | Makeup & hair color tints | later | `render`, `analysis` |
| [AI-034](ai-tools.md#ai-034-virtual-relighting--later) | Virtual relighting | later | `render`, `analysis` |
| [AI-035](ai-tools.md#ai-035-project-health-check--one-click-fixes--later) | Project health check & one-click fixes | later | `project`, `analysis`, `app` |
| [AI-036](ai-tools.md#ai-036-clips-from-long-recordings--later) | Clips from long recordings | later | `timeline`, `analysis`, `app` |
| [AI-037](ai-tools.md#ai-037-organize-media-into-bins--later) | Organize media into bins | later | `timeline`, `media`, `analysis` |

### EX — [Preview player & export](preview-and-export.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [EX-001](preview-and-export.md#ex-001-player-panel-and-viewer--mvp) | Player panel and viewer | mvp | `render`, `app` |
| [EX-002](preview-and-export.md#ex-002-play--pause-and-the-playback-clock--mvp) | Play / pause and the playback clock | mvp | `media`, `render`, `audio`, `app` |
| [EX-003](preview-and-export.md#ex-003-transport-controls-frame-stepping-and-go-to-time--mvp) | Transport controls, frame stepping and go-to-time | mvp | `timeline`, `app` |
| [EX-004](preview-and-export.md#ex-004-scrub-and-seek-frame-delivery--mvp) | Scrub and seek frame delivery | mvp | `media`, `render` |
| [EX-005](preview-and-export.md#ex-005-offline-missing-and-error-slates--mvp) | Offline, missing and error slates | mvp | `render` |
| [EX-006](preview-and-export.md#ex-006-preview-resolution-and-paused-full-quality-refresh--mvp) | Preview resolution and paused full-quality refresh | mvp | `render`, `project`, `app` |
| [EX-007](preview-and-export.md#ex-007-jkl-shuttle-and-variable-speed-playback--v1) | J/K/L shuttle and variable-speed playback | v1 | `media`, `audio`, `app` |
| [EX-008](preview-and-export.md#ex-008-loop-playback-and-play-range--v1) | Loop playback and play range | v1 | `timeline`, `audio`, `app` |
| [EX-009](preview-and-export.md#ex-009-viewer-zoom-and-pan--v1) | Viewer zoom and pan | v1 | `app` |
| [EX-010](preview-and-export.md#ex-010-fullscreen-preview--v1) | Fullscreen preview | v1 | `app` |
| [EX-011](preview-and-export.md#ex-011-safe-areas-grid-and-aspect-guides--v1) | Safe areas, grid and aspect guides | v1 | `project`, `app` |
| [EX-012](preview-and-export.md#ex-012-vertical-feed-ui-zone-overlay--v1) | Vertical-feed UI zone overlay | v1 | `app` |
| [EX-013](preview-and-export.md#ex-013-canvas-ratio-menu-in-the-player--v1) | Canvas ratio menu in the player | v1 | `timeline`, `app` |
| [EX-014](preview-and-export.md#ex-014-snapshot-save-or-copy-the-current-frame--v1) | Snapshot: save or copy the current frame | v1 | `media`, `render`, `app` |
| [EX-015](preview-and-export.md#ex-015-media-preview-mode-source-viewing-in-the-player--v1) | Media preview mode (source viewing in the player) | v1 | `media`, `audio`, `app` |
| [EX-016](preview-and-export.md#ex-016-render-cache-and-pre-render--v1) | Render cache and pre-render | v1 | `timeline`, `media`, `render` |
| [EX-017](preview-and-export.md#ex-017-playback-performance-indicator--v1) | Playback performance indicator | v1 | `render`, `app` |
| [EX-018](preview-and-export.md#ex-018-hardware-accelerated-decoding-for-preview--v1) | Hardware-accelerated decoding for preview | v1 | `media` |
| [EX-019](preview-and-export.md#ex-019-audio-monitoring-controls-on-the-player--v1) | Audio monitoring controls on the player | v1 | `audio`, `app` |
| [EX-020](preview-and-export.md#ex-020-external-display--secondary-preview-window--later) | External display / secondary preview window | later | `render`, `app` |
| [EX-021](preview-and-export.md#ex-021-export-dialog--mvp) | Export dialog | mvp | `project`, `app` |
| [EX-022](preview-and-export.md#ex-022-export-job-render-encode-progress-and-cancel--mvp) | Export job: render, encode, progress and cancel | mvp | `timeline`, `media`, `render`, `audio`, `app` |
| [EX-023](preview-and-export.md#ex-023-output-file-name-folder-and-overwrite-rules--mvp) | Output file name, folder and overwrite rules | mvp | `project`, `app` |
| [EX-024](preview-and-export.md#ex-024-output-resolution-presets--mvp) | Output resolution presets | mvp | `render`, `project` |
| [EX-025](preview-and-export.md#ex-025-custom-output-size-and-8k--v1) | Custom output size and 8K | v1 | `render`, `project` |
| [EX-026](preview-and-export.md#ex-026-output-frame-rate--mvp) | Output frame rate | mvp | `timeline`, `project` |
| [EX-027](preview-and-export.md#ex-027-quality-presets-bitrate-tiers--mvp) | Quality presets (bitrate tiers) | mvp | `media`, `project` |
| [EX-028](preview-and-export.md#ex-028-custom-rate-control--v1) | Custom rate control | v1 | `media`, `project` |
| [EX-029](preview-and-export.md#ex-029-baseline-format-h264--aac-in-mp4--mvp) | Baseline format: H.264 + AAC in MP4 | mvp | `media` |
| [EX-030](preview-and-export.md#ex-030-more-codecs-and-containers-and-10-bit-output--v1) | More codecs and containers, and 10-bit output | v1 | `media`, `project` |
| [EX-031](preview-and-export.md#ex-031-audio-settings-for-video-exports--mvp) | Audio settings for video exports | mvp | `media`, `audio`, `project` |
| [EX-032](preview-and-export.md#ex-032-output-color-baseline-bt709-tags-8-bit-dithering--mvp) | Output color baseline: BT.709 tags, 8-bit, dithering | mvp | `media`, `render` |
| [EX-034](preview-and-export.md#ex-034-hardware-accelerated-encoding--v1) | Hardware-accelerated encoding | v1 | `media`, `project` |
| [EX-035](preview-and-export.md#ex-035-export-range-whole-timeline-inout-selected-clips--v1) | Export range: whole timeline, in–out, selected clips | v1 | `timeline`, `project` |
| [EX-036](preview-and-export.md#ex-036-export-preflight-checks--mvp) | Export preflight checks | mvp | `timeline`, `project`, `app` |
| [EX-037](preview-and-export.md#ex-037-size-duration-and-disk-space-readout--v1) | Size, duration and disk-space readout | v1 | `project`, `app` |
| [EX-038](preview-and-export.md#ex-038-background-export-and-export-queue--v1) | Background export and export queue | v1 | `project`, `app` |
| [EX-039](preview-and-export.md#ex-039-export-finished-result-view-and-actions--mvp) | Export finished: result view and actions | mvp | `app` |
| [EX-040](preview-and-export.md#ex-040-no-watermark-no-paywall-no-account-no-upload--mvp) | No watermark, no paywall, no account, no upload | mvp | `media`, `render` |
| [EX-041](preview-and-export.md#ex-041-export-presets-built-in-and-user--v1) | Export presets: built-in and user | v1 | `project`, `app` |
| [EX-042](preview-and-export.md#ex-042-platform-presets-by-aspect-ratio--v1) | Platform presets by aspect ratio | v1 | `render`, `project` |
| [EX-043](preview-and-export.md#ex-043-animated-gif-export--v1) | Animated GIF export | v1 | `media`, `project` |
| [EX-044](preview-and-export.md#ex-044-audio-only-export--v1) | Audio-only export | v1 | `media`, `audio`, `project` |
| [EX-045](preview-and-export.md#ex-045-captions-and-subtitle-files-in-export--v1) | Captions and subtitle files in export | v1 | `media`, `project` |
| [EX-046](preview-and-export.md#ex-046-cover-image-and-file-metadata--v1) | Cover image and file metadata | v1 | `media`, `render`, `project` |
| [EX-047](preview-and-export.md#ex-047-image-sequence-export--later) | Image sequence export | later | `media`, `render` |
| [EX-048](preview-and-export.md#ex-048-export-with-transparency-alpha--later) | Export with transparency (alpha) | later | `media`, `render` |
| [EX-049](preview-and-export.md#ex-049-chapters-from-markers--later) | Chapters from markers | later | `timeline`, `media` |
| [EX-050](preview-and-export.md#ex-050-batch-and-multi-range-export--later) | Batch and multi-range export | later | `app` |
| [EX-051](preview-and-export.md#ex-051-draft-export-fast-test-render--later) | Draft export (fast test render) | later | `render`, `app` |
| [EX-052](preview-and-export.md#ex-052-command-line-export--later) | Command-line export | later | `app` |
| [EX-053](preview-and-export.md#ex-053-cover-designer--later) | Cover designer | later | `timeline`, `media`, `render`, `project`, `app` |
| [EX-054](preview-and-export.md#ex-054-clip-startend-jumps-and-locate-in-timeline--v1) | Clip start/end jumps and locate in timeline | v1 | `timeline`, `app` |
| [EX-055](preview-and-export.md#ex-055-player-rulers-and-guide-lines--v1) | Player rulers and guide lines | v1 | `project`, `app` |
| [EX-056](preview-and-export.md#ex-056-playback-priority-smooth-or-sharp--v1) | Playback priority: smooth or sharp | v1 | `render`, `app` |
| [EX-057](preview-and-export.md#ex-057-several-outputs-from-one-export--v1) | Several outputs from one export | v1 | `media`, `audio`, `project` |
| [EX-058](preview-and-export.md#ex-058-export-clips-as-separate-files--later) | Export clips as separate files | later | `timeline`, `app` |
| [EX-059](preview-and-export.md#ex-059-your-own-logo-on-exports--later) | Your own logo on exports | later | `media`, `render`, `project` |
| [EX-060](preview-and-export.md#ex-060-motion-blur-for-all-moving-clips--later) | Motion blur for all moving clips | later | `timeline`, `app` |

### UI — [Workspace layout, inspector & shortcuts](workspace-and-shortcuts.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [UI-001](workspace-and-shortcuts.md#ui-001-main-editor-window-layout--mvp) | Main editor window layout | mvp | `app` |
| [UI-002](workspace-and-shortcuts.md#ui-002-panel-resizing-collapsing--window-state--mvp) | Panel resizing, collapsing & window state | mvp | `app` |
| [UI-003](workspace-and-shortcuts.md#ui-003-layout-presets--reset-layout--v1) | Layout presets & reset layout | v1 | `app` |
| [UI-004](workspace-and-shortcuts.md#ui-004-asset-panel-category-tabs--mvp) | Asset panel category tabs | mvp | `app` |
| [UI-005](workspace-and-shortcuts.md#ui-005-adding-assets-from-the-panel--mvp) | Adding assets from the panel | mvp | `timeline`, `app` |
| [UI-006](workspace-and-shortcuts.md#ui-006-library-panel-framework-search-favorites-recents--view-options--v1) | Library panel framework: search, favorites, recents & view options | v1 | `app` |
| [UI-007](workspace-and-shortcuts.md#ui-007-hover-previews--try-before-apply--v1) | Hover previews & try-before-apply | v1 | `render`, `app` |
| [UI-008](workspace-and-shortcuts.md#ui-008-inspector-panel-with-context-dependent-tabs--mvp) | Inspector panel with context-dependent tabs | mvp | `app` |
| [UI-009](workspace-and-shortcuts.md#ui-009-inspector-control-conventions--mvp) | Inspector control conventions | mvp | `app` |
| [UI-010](workspace-and-shortcuts.md#ui-010-inspector-multi-selection-editing--v1) | Inspector multi-selection editing | v1 | `app` |
| [UI-011](workspace-and-shortcuts.md#ui-011-timeline-toolbar--mvp) | Timeline toolbar | mvp | `app` |
| [UI-012](workspace-and-shortcuts.md#ui-012-top-bar-application-menu--save-status--mvp) | Top bar, application menu & save status | mvp | `app` |
| [UI-013](workspace-and-shortcuts.md#ui-013-dark-theme--design-tokens--mvp) | Dark theme & design tokens | mvp | `app` |
| [UI-014](workspace-and-shortcuts.md#ui-014-light-system--high-contrast-themes--v1) | Light, system & high-contrast themes | v1 | `app` |
| [UI-015](workspace-and-shortcuts.md#ui-015-interface-scale--v1) | Interface scale | v1 | `app` |
| [UI-016](workspace-and-shortcuts.md#ui-016-home-screen-shell--mvp) | Home screen shell | mvp | `project`, `app` |
| [UI-017](workspace-and-shortcuts.md#ui-017-project-actions-on-home-placement--input--v1) | Project actions on Home: placement & input | v1 | `project`, `app` |
| [UI-018](workspace-and-shortcuts.md#ui-018-preferences-window--settings-storage--mvp) | Preferences window & settings storage | mvp | `app` |
| [UI-019](workspace-and-shortcuts.md#ui-019-general-settings-startup-time-display-tooltips--v1) | General settings: startup, time display, tooltips | v1 | `timeline`, `app` |
| [UI-020](workspace-and-shortcuts.md#ui-020-editing-section-in-preferences--v1) | Editing section in Preferences | v1 | `app` |
| [UI-021](workspace-and-shortcuts.md#ui-021-autosave-in-the-ui--v1) | Autosave in the UI | v1 | `app` |
| [UI-022](workspace-and-shortcuts.md#ui-022-saving--backups-section-and-restore-entry-point--v1) | Saving & backups section and restore entry point | v1 | `project`, `app` |
| [UI-023](workspace-and-shortcuts.md#ui-023-crash-reports--app-level-fault-handling--v1) | Crash reports & app-level fault handling | v1 | `project`, `app` |
| [UI-024](workspace-and-shortcuts.md#ui-024-safe-mode--crash-loop-protection--v1) | Safe mode & crash-loop protection | v1 | `app` |
| [UI-025](workspace-and-shortcuts.md#ui-025-unsaved-changes-prompt--mvp) | Unsaved-changes prompt | mvp | `project`, `app` |
| [UI-026](workspace-and-shortcuts.md#ui-026-storage-section-in-preferences--v1) | Storage section in Preferences | v1 | `media`, `app` |
| [UI-027](workspace-and-shortcuts.md#ui-027-proxy-controls-placement--v1) | Proxy controls placement | v1 | `app` |
| [UI-028](workspace-and-shortcuts.md#ui-028-performance--gpu-settings--v1) | Performance & GPU settings | v1 | `media`, `render`, `app` |
| [UI-029](workspace-and-shortcuts.md#ui-029-playback--audio-section-in-preferences--mvp) | Playback & audio section in Preferences | mvp | `audio`, `app` |
| [UI-030](workspace-and-shortcuts.md#ui-030-command-system--shortcut-dispatch--mvp) | Command system & shortcut dispatch | mvp | `app` |
| [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp) | Default keyboard shortcut map | mvp | `app` |
| [UI-032](workspace-and-shortcuts.md#ui-032-shortcut-editor--keymaps--v1) | Shortcut editor & keymaps | v1 | `app` |
| [UI-033](workspace-and-shortcuts.md#ui-033-shortcut-cheat-sheet--v1) | Shortcut cheat sheet | v1 | `app` |
| [UI-034](workspace-and-shortcuts.md#ui-034-command-palette--v1) | Command palette | v1 | `app` |
| [UI-035](workspace-and-shortcuts.md#ui-035-context-menus--mvp) | Context menus | mvp | `app` |
| [UI-036](workspace-and-shortcuts.md#ui-036-drag--drop-from-the-operating-system--mvp) | Drag & drop from the operating system | mvp | `app` |
| [UI-037](workspace-and-shortcuts.md#ui-037-in-app-drag--drop--mvp) | In-app drag & drop | mvp | `app` |
| [UI-038](workspace-and-shortcuts.md#ui-038-mouse-wheel-trackpad--pointer-conventions--mvp) | Mouse wheel, trackpad & pointer conventions | mvp | `app` |
| [UI-039](workspace-and-shortcuts.md#ui-039-tooltips--mvp) | Tooltips | mvp | `app` |
| [UI-040](workspace-and-shortcuts.md#ui-040-notifications--background-tasks--mvp) | Notifications & background tasks | mvp | `app` |
| [UI-041](workspace-and-shortcuts.md#ui-041-keyboard-navigation--focus--v1) | Keyboard navigation & focus | v1 | `app` |
| [UI-042](workspace-and-shortcuts.md#ui-042-screen-reader-support--v1) | Screen-reader support | v1 | `app` |
| [UI-043](workspace-and-shortcuts.md#ui-043-visual-accessibility--v1) | Visual accessibility | v1 | `app` |
| [UI-044](workspace-and-shortcuts.md#ui-044-string-catalog--locale-framework--mvp) | String catalog & locale framework | mvp | `app` |
| [UI-045](workspace-and-shortcuts.md#ui-045-interface-languages--translations--v1) | Interface languages & translations | v1 | `app` |
| [UI-046](workspace-and-shortcuts.md#ui-046-right-to-left--complex-script-ui--later) | Right-to-left & complex-script UI | later | `app` |
| [UI-047](workspace-and-shortcuts.md#ui-047-history-panel-placement--later) | History panel placement | later | `timeline`, `app` |
| [UI-048](workspace-and-shortcuts.md#ui-048-panel-maximize--distraction-free-mode--v1) | Panel maximize & distraction-free mode | v1 | `app` |
| [UI-049](workspace-and-shortcuts.md#ui-049-first-run-setup--v1) | First-run setup | v1 | `app` |
| [UI-050](workspace-and-shortcuts.md#ui-050-settings-export-import--reset--v1) | Settings export, import & reset | v1 | `app` |
| [UI-051](workspace-and-shortcuts.md#ui-051-about-licenses--help--mvp) | About, licenses & help | mvp | `media`, `app` |
| [UI-052](workspace-and-shortcuts.md#ui-052-logs--diagnostics--v1) | Logs & diagnostics | v1 | `app` |
| [UI-053](workspace-and-shortcuts.md#ui-053-native-os-integration--v1) | Native OS integration | v1 | `app` |
| [UI-054](workspace-and-shortcuts.md#ui-054-detachable-panels--multi-monitor--later) | Detachable panels & multi-monitor | later | `app` |
| [UI-055](workspace-and-shortcuts.md#ui-055-update-notice-opt-in--later) | Update notice (opt-in) | later | `app` |
| [UI-056](workspace-and-shortcuts.md#ui-056-auxiliary-panels-scopes-mixer-captions-transcript-jobs-history--v1) | Auxiliary panels: scopes, mixer, captions, transcript, jobs, history | v1 | `app` |
| [UI-057](workspace-and-shortcuts.md#ui-057-several-open-projects-in-separate-windows--later) | Several open projects in separate windows | later | `app` |
| [UI-058](workspace-and-shortcuts.md#ui-058-hints-and-dont-ask-again--v1) | Hints and "don't ask again" | v1 | `app` |
| [UI-059](workspace-and-shortcuts.md#ui-059-guided-tour--later) | Guided tour | later | `app` |
| [UI-060](workspace-and-shortcuts.md#ui-060-whats-new-after-an-update--later) | What's new after an update | later | `app` |
| [UI-061](workspace-and-shortcuts.md#ui-061-system-check--hardware-rating--v1) | System check & hardware rating | v1 | `media`, `render`, `audio`, `app` |
| [UI-062](workspace-and-shortcuts.md#ui-062-low-memory-warnings--out-of-memory-notice--v1) | Low-memory warnings & out-of-memory notice | v1 | `app` |
| [UI-063](workspace-and-shortcuts.md#ui-063-diagnostic-bundle--issue-reports--v1) | Diagnostic bundle & issue reports | v1 | `app` |
| [UI-064](workspace-and-shortcuts.md#ui-064-save-as-preset-dialog--user-preset-conventions--v1) | Save-as-preset dialog & user preset conventions | v1 | `app` |
| [UI-065](workspace-and-shortcuts.md#ui-065-tool-suggestions-for-the-selected-clip--later) | Tool suggestions for the selected clip | later | `analysis`, `app` |

### AS — [LLM assistant](assistant.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [AS-001](assistant.md#as-001-assistant-panel-and-conversation--v1) | Assistant panel and conversation | v1 | `assistant`, `app` |
| [AS-002](assistant.md#as-002-providers-and-model-choice--v1) | Providers and model choice | v1 | `assistant`, `app` |
| [AS-003](assistant.md#as-003-api-keys-in-the-os-keychain--v1) | API keys in the OS keychain | v1 | `assistant`, `app` |
| [AS-004](assistant.md#as-004-what-leaves-the-machine-disclosure-and-consent--v1) | What leaves the machine: disclosure and consent | v1 | `assistant`, `app` |
| [AS-005](assistant.md#as-005-questions-about-the-project-read-only--v1) | Questions about the project (read-only) | v1 | `timeline`, `assistant`, `app` |
| [AS-006](assistant.md#as-006-edit-tools--v1) | Edit tools | v1 | `timeline`, `assistant` |
| [AS-007](assistant.md#as-007-proposal-preview-ghost-timeline-and-change-list--v1) | Proposal preview: ghost timeline and change list | v1 | `timeline`, `assistant`, `app` |
| [AS-008](assistant.md#as-008-accept-adjust-or-discard--v1) | Accept, adjust or discard | v1 | `timeline`, `assistant`, `app` |
| [AS-009](assistant.md#as-009-proposal-out-of-date--v1) | Proposal out of date | v1 | `timeline`, `assistant`, `app` |
| [AS-010](assistant.md#as-010-stop-and-cancel--v1) | Stop and cancel | v1 | `media`, `assistant`, `app` |
| [AS-011](assistant.md#as-011-multi-step-proposals--later) | Multi-step proposals | later | `timeline`, `assistant`, `app` |
| [AS-012](assistant.md#as-012-macro-tools--later) | Macro tools | later | `timeline`, `assistant` |
| [AS-013](assistant.md#as-013-actions-outside-the-document--later) | Actions outside the document | later | `project`, `assistant`, `app` |
| [AS-014](assistant.md#as-014-scope-selection-range-and-playhead--v1) | Scope: selection, range and playhead | v1 | `assistant`, `app` |
| [AS-015](assistant.md#as-015-transcript--and-caption-aware-editing--later) | Transcript- and caption-aware editing | later | `timeline`, `assistant`, `app` |
| [AS-016](assistant.md#as-016-frames-for-the-model-vision--later) | Frames for the model (vision) | later | `render`, `assistant`, `app` |
| [AS-017](assistant.md#as-017-sound-aware-questions--later) | Sound-aware questions | later | `media`, `audio`, `assistant`, `app` |
| [AS-018](assistant.md#as-018-limits-and-spending-guard--v1) | Limits and spending guard | v1 | `assistant`, `app` |
| [AS-019](assistant.md#as-019-content-from-media-is-data-not-instructions--v1) | Content from media is data, not instructions | v1 | `assistant` |
| [AS-020](assistant.md#as-020-conversation-history-and-storage--v1) | Conversation history and storage | v1 | `project`, `assistant`, `app` |

### CG — [Cloud generation (Higgsfield)](cloud-generation.md)

| ID | Feature | Priority | Crates |
|---|---|---|---|
| [CG-001](cloud-generation.md#cg-001-connect-a-higgsfield-account--v1) | Connect a Higgsfield account | v1 | `cloud`, `app` |
| [CG-002](cloud-generation.md#cg-002-credentials-in-the-os-keychain--disconnect--v1) | Credentials in the OS keychain & disconnect | v1 | `cloud`, `app` |
| [CG-003](cloud-generation.md#cg-003-model-catalogue--model-picker--v1) | Model catalogue & model picker | v1 | `cloud`, `app` |
| [CG-004](cloud-generation.md#cg-004-what-leaves-the-machine-disclosure-and-data-notice--v1) | What leaves the machine: disclosure and data notice | v1 | `cloud`, `app` |
| [CG-005](cloud-generation.md#cg-005-cost-preflight-and-run-confirmation--v1) | Cost preflight and run confirmation | v1 | `cloud`, `app` |
| [CG-006](cloud-generation.md#cg-006-generate-tab-images--v1) | Generate tab: images | v1 | `cloud`, `app` |
| [CG-007](cloud-generation.md#cg-007-generate-tab-video-from-a-prompt-or-frames--v1) | Generate tab: video from a prompt or frames | v1 | `timeline`, `cloud`, `app` |
| [CG-008](cloud-generation.md#cg-008-references-from-the-library-timeline-and-current-frame--v1) | References from the library, timeline and current frame | v1 | `media`, `render`, `cloud`, `app` |
| [CG-009](cloud-generation.md#cg-009-cloud-jobs-in-the-task-list-and-the-runs-list--v1) | Cloud jobs in the task list and the Runs list | v1 | `project`, `cloud`, `app` |
| [CG-010](cloud-generation.md#cg-010-unknown-outcomes-are-never-resubmitted--v1) | Unknown outcomes are never resubmitted | v1 | `cloud`, `app` |
| [CG-011](cloud-generation.md#cg-011-results-as-generated-media-with-provenance--v1) | Results as generated media with provenance | v1 | `timeline`, `media`, `project`, `cloud`, `app` |
| [CG-012](cloud-generation.md#cg-012-placing-results-on-the-timeline--v1) | Placing results on the timeline | v1 | `timeline`, `app` |
| [CG-013](cloud-generation.md#cg-013-results-that-finish-while-the-project-is-closed--v1) | Results that finish while the project is closed | v1 | `project`, `cloud`, `app` |
| [CG-014](cloud-generation.md#cg-014-source-ranges-and-uploads-for-clip-actions--v1) | Source ranges and uploads for clip actions | v1 | `timeline`, `media`, `cloud`, `app` |
| [CG-015](cloud-generation.md#cg-015-edit-a-clip-with-a-prompt--v1) | Edit a clip with a prompt | v1 | `media`, `cloud`, `app` |
| [CG-016](cloud-generation.md#cg-016-extend-a-clip-forward-or-backward--v1) | Extend a clip forward or backward | v1 | `timeline`, `media`, `cloud`, `app` |
| [CG-017](cloud-generation.md#cg-017-remove-background-cloud--later) | Remove background (cloud) | later | `media`, `cloud`, `app` |
| [CG-018](cloud-generation.md#cg-018-upscale-cloud--later) | Upscale (cloud) | later | `media`, `cloud`, `app` |
| [CG-019](cloud-generation.md#cg-019-reframe-to-the-project-aspect-generative--later) | Reframe to the project aspect (generative) | later | `media`, `cloud`, `app` |
| [CG-020](cloud-generation.md#cg-020-sdr-to-hdr--later) | SDR to HDR | later | `media`, `cloud`, `app` |
| [CG-021](cloud-generation.md#cg-021-dub-to-another-language--later) | Dub to another language | later | `timeline`, `media`, `cloud`, `app` |
| [CG-022](cloud-generation.md#cg-022-lip-sync-to-an-audio-clip--later) | Lip sync to an audio clip | later | `media`, `cloud`, `app` |
| [CG-023](cloud-generation.md#cg-023-voice-change--later) | Voice change | later | `media`, `cloud`, `app` |
| [CG-024](cloud-generation.md#cg-024-generation-tools-for-the-assistant--later) | Generation tools for the assistant | later | `assistant`, `app` |
| [CG-025](cloud-generation.md#cg-025-spending-limits--v1) | Spending limits | v1 | `cloud`, `app` |
| [CG-026](cloud-generation.md#cg-026-service-questions-adjustments-and-follow-up-steps--v1) | Service questions, adjustments and follow-up steps | v1 | `cloud`, `app` |

## Roadmap

The roadmap is a sequence of tracer bullets: each milestone is a thin, end-to-end slice that leaves FreeCut buildable, tested and usable, and each later milestone widens it. M0–M5 deliver the MVP, M6–M8 deliver v1, and M9 is the pool of `later` features. The assistant track M-AI1–M-AI4 (AS, [llm-editor.md](../architecture/llm-editor.md) §9) runs alongside them: it builds on the finished MVP and picks up each area's edit requests as they land. The cloud generation track M-CG1–M-CG4 (CG, [cloud-generation.md](../architecture/cloud-generation.md) §14, slices H1–H4) is a second side track: it is opt-in, needs the owners' confirmation of the second *Local only* exception before it starts, builds on M5, and its last milestone waits for M-AI3.

**Rules for every milestone**

- **Dependency direction first.** The model lands in `timeline` before any UI uses it (M1 before M3); `media` and `project` come before `render`, `audio` and the editor that consume them; `app` wires the rest.
- **Assigned where finished.** A feature is listed in the milestone where it becomes complete for the user. Earlier milestones may build parts of it — M1 builds the planners that M3's gestures call, and FX-001's later pipeline stages are pass-throughs until the features that feed them land, so the stage order never changes.
- **Definition of done:** `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace` pass on macOS, Windows and Linux; every new edit op has its apply → undo → redo test; nothing decodes, renders or does file I/O on the UI thread; specs are updated first when behavior changes.

### Overview

| Milestone | Tier | Features | Demo |
|---|---|---:|---|
| [M0 Workspace scaffold & quality gates](#m0--workspace-scaffold--quality-gates) | mvp | 2 | A contributor can clone the repository, get green fmt/clippy/test runs on all three platforms, and `cargo run -p freecut-app` to open an empty FreeCut window whose About box lists FreeCut's GPL notice, the linked FFmpeg build and every third-party license. |
| [M1 Timeline model core (pure, fully tested)](#m1--timeline-model-core-pure-fully-tested) | mvp | 13 | A contributor can script an edit in a unit test — insert clips on the magnetic main track, move, trim, split and ripple-delete them — and undo and redo every step back to the original document while the property tests keep every track invariant. |
| [M2 Projects & media library (media I/O, save/load)](#m2--projects--media-library-media-io-saveload) | mvp | 35 | A user can create a 9:16 project from the Home screen, import videos, photos and songs by dialog or drag-and-drop, watch thumbnails and waveforms fill in while probing runs in the background, save, quit, move the project folder, reopen it and relink a file that went missing. |
| [M3 Editor shell, timeline editing & preview playback](#m3--editor-shell-timeline-editing--preview-playback) | mvp | 32 | A user can drag clips from the media panel onto a magnetic multi-track timeline, rearrange, trim, split and delete them with snapping and undo, and play the edit back with picture and sound in sync. |
| [M4 Export MP4 (H.264/AAC)](#m4--export-mp4-h264aac) | mvp | 12 | A user can export their edit as an MP4 that plays in any standard player, see a preflight warning about offline media first, and cancel a running export without leaving a partial file behind. |
| [M5 Text, transform, transitions & clip audio — MVP complete](#m5--text-transform-transitions--clip-audio--mvp-complete) | mvp | 17 | A user can make a short vertical video end to end — cut clips, add a title, place a picture-in-picture overlay, cross-dissolve between shots with the sound crossfading, fade the music out — and export a watermark-free MP4. |
| [M6 Everyday editing, media management & workspace](#m6--everyday-editing-media-management--workspace) | v1 | 113 | A user can cut a long 4K interview smoothly on proxies — with markers, in/out ranges, the blade, *Ripple all tracks*, groups and compound clips — organize the media into bins, rebind shortcuts, and lose nothing when FreeCut crashes mid-edit. |
| [M7 Motion & look: keyframes, speed, color, effects & transitions](#m7--motion--look-keyframes-speed-color-effects--transitions) | v1 | 105 | A user can keyframe a zoom-and-pan, speed-ramp and reverse a clip, key out a green screen, stabilize and mask a shot, grade it with looks, curves and a LUT, add effects and library transitions, animate clips in and out, and pin a title to a moving object — all previewed live and exported identically. |
| [M8 Words, sound, graphics & delivery](#m8--words-sound-graphics--delivery) | v1 | 108 | A user can style and animate titles, auto-caption an interview on their own machine, import, fix and burn in SRT captions, decorate with stickers, shapes and a saved template, clean up a voiceover and duck the music under it, cut to the beat, remove silences, and queue HEVC, ProRes, GIF and audio-only exports from presets. |
| [M9 Later bucket](#m9--later-bucket) | later | 134 | A user can edit by transcript, remove backgrounds and objects, reframe, upscale or colorize footage with local open-source models, and use advanced tools such as slip/roll, OTIO interchange and alpha or command-line export — without any cloud service. |
| [M-AI1 Assistant: query tools and read-only chat](#m-ai1--assistant-query-tools-and-read-only-chat) | v1 | 9 | A user can switch on the assistant with a local model server or their own Anthropic key, ask questions about the project, click the answer's clip chips to find them, and see exactly what was sent. |
| [M-AI2 Assistant: single-request edits with preview](#m-ai2--assistant-single-request-edits-with-preview) | v1 | 5 | A user can ask for one edit in plain language, see it as a ghost on the timeline with a FreeCut-written change list, play it, and accept it as one undo entry. |
| [M-AI3 Assistant: multi-step plans, macros and actions](#m-ai3--assistant-multi-step-plans-macros-and-actions) | later | 3 | A user can ask for several edits at once, untick parts of the proposal, use ready-made recipes such as a title card, and have the export dialog opened pre-filled. |
| [M-AI4 Assistant: vision- and transcript-aware editing](#m-ai4--assistant-vision--and-transcript-aware-editing) | later | 3 | A user can edit by what is said and seen — "remove the part about pricing", "is the title readable?" — with transcripts, levels and frames read locally and only what AS-004 discloses sent. |
| [M-CG1 Cloud generation: account, catalogue and cost](#m-cg1--cloud-generation-account-catalogue-and-cost) | v1 | 3 | A user can switch on cloud generation, connect their own Higgsfield account in the browser, browse the models in a Generate tab filtered to the project's aspect ratio, and see what a generation would cost — without spending anything. |
| [M-CG2 Cloud generation: Generate tab, jobs and results](#m-cg2--cloud-generation-generate-tab-jobs-and-results) | v1 | 12 | A user can generate images and short videos from a prompt or a frame, confirm each run's cost and what is sent, quit mid-run without losing it, and find the results in the Generated bin with a full record of how they were made. |
| [M-CG3 Cloud generation: AI edit actions on clips](#m-cg3--cloud-generation-ai-edit-actions-on-clips) | v1 + later | 10 | A user can right-click a clip to edit a section with a prompt or extend it forward or backward, and later remove backgrounds, upscale, reframe, dub, lip-sync or change voices in the cloud — each placed back as one undo step. |
| [M-CG4 Cloud generation: assistant tools](#m-cg4--cloud-generation-assistant-tools) | later | 1 | A user can ask the assistant for a generated shot or an extension and confirm the run in FreeCut's own cost sheet before anything is spent. |

### Next build — Text round 2

The next build is a cross-milestone slice that pulls the text and text-animation features forward from M7 and M8, so FreeCut's titles reach what users expect from a modern short-form editor before the rest of v1. Priorities and milestone assignments above do not change. The exact scope, demo and what stays later are in [text-and-captions.md › Text round 2](text-and-captions.md#text-round-2--next-build-scope). In short:

- **Builds:** TX-008, TX-009, TX-010, TX-011, TX-012, TX-013, TX-014, TX-016, TX-019, TX-020, TX-021, TX-022, TX-023, TX-054, TX-055, TX-056, FX-018, FX-019, FX-030, plus the unbuilt parts of TX-001, TX-003 and TX-005.
- **Groundwork, only as far as text needs it:** TR-027, TR-028, TR-030, TR-031, UI-006, UI-007, UI-009 (keyframe diamond), UI-064, TL-034 (Text style and Animations categories), and the UI concept's slices S1, S2, S3 and S7. Renderer additions for the CPU compositor (per-glyph placement, distance-transform outline, glow and shadow, premultiplied text layers, projective sampling) are recorded in [architecture README C46](../architecture/README.md#6-conflicts-between-the-studies-and-how-they-are-resolved).
- **After round 2 (still v1):** on-canvas text edit mode (TX-002) and the radial-gradient centre handle (TX-014), TX-007, TX-015, TX-018, TX-024, TX-025 to TX-037, AI-008's Follow card, FX-017. **Still later:** TX-017, TX-038 to TX-053, OV-028 (TX-038, TX-039, TX-044 and TX-051 have since moved to v1 with the captions build).

### Next build — Timeline toolbar round

A cross-milestone slice that brings the timeline toolbar up to what users expect from a modern short-form editor: an Add menu, a Select / Blade / Hand tool menu, cut away before or after the playhead, markers, the *Follow main track* switch with its item-kind menu and linked selection, hover skim with its mode menu, and voiceover recording. Priorities and milestone assignments above do not change. The mapping of each toolbar function to its owner, the ordered slices T1–T6 (crates, contracts, tests, acceptance) and the feasibility notes are in [timeline-editing.md › Toolbar round — build slices](timeline-editing.md#toolbar-round--build-slices). In short:

- **Builds:** TL-024 (with the Hand tool), TL-026, TL-037, TL-057 (new), TL-029, TL-014, AU-027, and UI-011's toolbar layout; UI-031 gains `H` for the Hand tool.
- **Order:** T1 toolbar, tools and cut-away → T2 markers → T3 linked selection → T4 *Follow main track* → T5 hover skim → T6 voiceover (capture core, placement, panel).
- **Waits for its owner:** hover skim's *Picture and sound* (AU-016's scrub audio) and the recorder's cleanup toggles and voice effect (AU-018, AU-019, AU-020, AU-022, AU-037); they appear when those land.

### Next build — Captions & speech

A cross-milestone slice that brings captions up to what users expect from a modern short-form editor: a Captions drawer on the Shelf, typed and imported captions burned into the export, captions generated from speech on the user's own machine with automatic language detection, filler-word marks, lyrics for songs, caption looks and a second-language line. Nothing is gated. Because the user asked for these now, the features this build completes move from later to v1 and from M9 to M8 (round-3 review, 2026-10-13): TX-038, TX-039, TX-044, TX-051, AI-002, AI-009 and AI-010. TX-041, AI-012 and AI-013 are built only in part (the English and assistant engines, the lexicons and filler marks, the window and split-back helpers) and stay later until their local translation models and the transcript review sheet exist. Scope, demo and decisions: [text-and-captions.md › Captions & speech build](text-and-captions.md#captions--speech-build--scope); design, engine choice and slice contracts: [speech-and-captions.md](../architecture/speech-and-captions.md) (architecture C47–C51).

- **Builds:** TX-025 to TX-028, TX-031 to TX-033 (SRT, WebVTT, LRC; burn-in and sidecar), TX-036 to TX-039, TX-041, TX-044, TX-051 (first looks), TX-057 (new), AI-001 (minimal pool), AI-002 (speech packs), AI-009, AI-010, AI-012 (lexicons), AI-013 (windows and split-back).
- **Order:** CS1 caption model, files, list and burn-in → CS2 `freecut-analysis`, whisper.cpp recognizer, model downloads and Auto captions → CS3 filler words, lyrics and caption looks → CS4 the second line.
- **Waits for its owner:** local translation models (AI-013's ONNX runtime), vocal separation (AI-023) for the best lyrics, embedded subtitle streams and the remaining file formats (TX-033 *Embed*, ASS, SBV), and translation through the assistant provider, which needs M-AI1.

### Next build — Adjustments round 1

A cross-milestone slice that brings grading up to what users expect from a modern short-form editor: a Color tab with Basic, HSL, Curves, Wheels and Masks pages whose sections each have an on/off switch, reset and keyframes; automatic correction and color match; LUTs with import, a bundled set and skin protection; blank adjustment clips and saved looks in an Adjust tab. Nothing is gated. Priorities and milestone assignments above do not change, except that CL-030 and CL-031 move from later to v1. The scope, the CPU color pipeline and the slice contracts are in [color-and-filters.md › Adjustments round 1](color-and-filters.md#adjustments-round-1--next-build-scope); the renderer decision is [architecture README C52](../architecture/README.md#6-conflicts-between-the-studies-and-how-they-are-resolved).

- **Builds:** CL-001, CL-006, CL-007, CL-008, CL-013, CL-014, CL-015, CL-016, CL-017, CL-019 (revised: a stored correction under the sliders), CL-020, CL-023 (switches), CL-024, CL-026, CL-028, CL-030, CL-031, CL-035, CL-043 (new), CL-044 (new), CL-045 (new).
- **Order:** K1 stack model, Light and Color, CPU grading, Basic page → K2 HSL, Curves, Wheels → K3 LUTs and skin protection → K4 adjustment clips, saved looks, Adjust tab → K5 Auto and Match → K6 color masks (with TR's shared mask core). K7 (Detail & finish) is optional and comes after.
- **Waits for its owner:** the Filters tab and bundled looks (CL-003 to CL-005), scopes (CL-021) and Compare (CL-023), which stay in M7; brush masks (a stroke mask shape from TR) and edge-following masks (AI-015).

### Round 3 — order of the three builds

The toolbar, adjustments and captions builds above were requested together (round 3) and are built in one order, recorded in [architecture README C54](../architecture/README.md#6-conflicts-between-the-studies-and-how-they-are-resolved):

- **Trunk (one slice at a time, because nearly every slice edits `timeline`, `app` and the project format):** T1 → T2 → T3 → T4 → T5 → T6b → T6c → K1 → K2 → K3 → K4 → K5 → K6 → CS1 → CS2 → CS3 → CS4. The toolbar goes first because it is the smallest and adds no crate; colour comes next because it only extends the existing CPU compositor (C52); captions and speech come last because CS2 adds the seventh crate and a C++ build (C47, C48).
- **Side lanes (crate-disjoint, may run beside the trunk):** T6a (`audio` only); K3a, the FFmpeg-free `.cube`/`.3dl` parsers (`media` only); CS1a, the SRT/WebVTT/LRC parsers and writers (`project::subtitles` only); CS2a, the 16 kHz PCM reader (`media` only); CS2b, the pure core of `freecut-analysis` (new crate only, after the maintainer approves its CLAUDE.md lines). T5 (`app` only) may pair with the model half of K1.
- **Format versions** are taken in merge order, one per slice that changes persisted data (C54).
- **Priorities:** TX-038, TX-039, TX-044, TX-051, AI-002, AI-009 and AI-010 moved from later to v1 for the captions build; CL-030 and CL-031 moved for the adjustments build.

### M0 — Workspace scaffold & quality gates

**Tier:** mvp · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `app`

**Goal.** Set up the Cargo workspace with the six crates wired in the allowed dependency direction (`timeline` depends on nothing internal; `media`, `project` → `timeline`; `render`/`audio` → `media` + `timeline`; `app` → everything), the shared conventions (thiserror in libraries, anyhow only in `app`, `tracing` logs, no `unwrap()` outside tests, `unsafe` only in `media`/`render`), and CI that runs `cargo fmt`, `cargo clippy -D warnings` and `cargo test` on macOS, Windows and Linux with FFmpeg linked. Retire the two risks that are cheapest to fix early: every UI string goes through the Fluent catalog with a CI lint (UI-044), and license notices for crates, FFmpeg and bundled assets are generated and embedded at build time (UI-051).

**Features (2):**

- UI (2): UI-044, UI-051

**Demo.** A contributor can clone the repository, get green fmt/clippy/test runs on all three platforms, and `cargo run -p freecut-app` to open an empty FreeCut window whose About box lists FreeCut's GPL notice, the linked FFmpeg build and every third-party license.

### M1 — Timeline model core (pure, fully tested)

**Tier:** mvp · **Crates touched:** `timeline`

**Goal.** Build the edit model in `freecut-timeline` with no I/O: rational ticks and the frame grid, sequences, the track model with its bands and invariants, clips and links, the magnetic main track, automatic track creation/removal, and the pure planners that every UI gesture will call later — insert, move, trim, split, delete/ripple delete and snap targets — on top of one linear undo history of `ChangeSet`s with coalescing and a saved-position dirty flag. Every op ships with the apply → undo → redo test and `proptest` runs against `Sequence::validate()`. Decide here how the document is structurally shared (`Arc` nodes) so later saves (PM-012) and background exports (EX-038) can snapshot it cheaply. The UI for these ops arrives in M3.

**Features (13):**

- TL (13): TL-001, TL-003, TL-004, TL-005, TL-006, TL-011, TL-012, TL-018, TL-020, TL-023, TL-025, TL-028, TL-043

**Demo.** A contributor can script an edit in a unit test — insert clips on the magnetic main track, move, trim, split and ripple-delete them — and undo and redo every step back to the original document while the property tests keep every track invariant.

### M2 — Projects & media library (media I/O, save/load)

**Tier:** mvp · **Crates touched:** `timeline`, `media`, `project`, `app`

**Goal.** First end-to-end slice through `media` and `project`: the Home screen, new projects with canvas ratio/resolution/frame-rate/audio settings, the versioned JSON project file written atomically with a single-writer lock, open/save/recent projects, portable media paths, and the unsaved-changes prompt. Media is imported by dialog or by dropping files, probed and normalized off the UI thread, shown with thumbnails and audio previews in the media grid, removable from the project, reported when it fails, flagged when offline and relinkable. The editor window frame (four regions, with the player, inspector and timeline still as placeholders), the top bar with save status, the dark theme, the command registry that builds the menus, OS drag-and-drop routing, and the toast/task-indicator layer over PM-073's background-task list all land here.

**Features (35):**

- PM (25): PM-001, PM-002, PM-003, PM-004, PM-005, PM-007, PM-011, PM-012, PM-013, PM-017, PM-021, PM-030, PM-031, PM-034, PM-035, PM-036, PM-037, PM-050, PM-058, PM-070, PM-072, PM-073, PM-090, PM-091, PM-100
- UI (10): UI-001, UI-002, UI-004, UI-012, UI-013, UI-016, UI-025, UI-030, UI-036, UI-040

**Demo.** A user can create a 9:16 project from the Home screen, import videos, photos and songs by dialog or drag-and-drop, watch thumbnails and waveforms fill in while probing runs in the background, save, quit, move the project folder, reopen it and relink a file that went missing.

### M3 — Editor shell, timeline editing & preview playback

**Tier:** mvp · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `app`

**Goal.** Make the editor usable for cutting. The timeline UI exposes M1's planners (drag/drop ghosts, trim, split, delete, snapping) with the ruler and timecode, playhead navigation, selection, zoom/scroll, filmstrips and waveforms, and track hide/mute. The inspector frame, timeline toolbar, the default keymap, context menus, in-app drag-and-drop, pointer conventions, tooltips and the first Preferences page (audio device and buffer) complete the shell. `freecut-render` gets the FX-001 compositor with the MVP stages (L1 source sample, L2 normalize with CL-002's color handling, L8 base transform with TR-002's fit baseline, L9 opacity, L10 Normal blend) and alpha-correct still overlays. The player shows the frame at the playhead, plays on the audio master clock, steps frames, scrubs with latest-wins frame requests, draws offline slates and scales preview resolution; `freecut-audio` mixes audio tracks and embedded clip audio to a stereo master with a safety limiter.

**Features (32):**

- PM (1): PM-071
- TL (6): TL-002, TL-008, TL-013, TL-015, TL-041, TL-042
- TR (1): TR-002
- AU (4): AU-001, AU-002, AU-010, AU-011
- CL (1): CL-002
- FX (1): FX-001
- OV (1): OV-011
- EX (6): EX-001, EX-002, EX-003, EX-004, EX-005, EX-006
- UI (11): UI-005, UI-008, UI-009, UI-011, UI-018, UI-029, UI-031, UI-035, UI-037, UI-038, UI-039

**Demo.** A user can drag clips from the media panel onto a magnetic multi-track timeline, rearrange, trim, split and delete them with snapping and undo, and play the edit back with picture and sound in sync.

### M4 — Export MP4 (H.264/AAC)

**Tier:** mvp · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `app`

**Goal.** Close the core loop. The export dialog offers canvas-size and short-side resolution presets, the project frame rate, three quality tiers and AAC audio; preflight blocks impossible exports and warns about offline media. The export job snapshots the document, renders every frame through the same FX-001 pipeline at output size off the UI thread, mixes audio offline, converts to BT.709 limited-range 4:2:0 with dithering, and writes an H.264/AAC MP4 with fast start through a `.part` file, with progress, ETA and clean cancel. Sanitized names and overwrite rules, the result view, and the no-watermark regression test (EX-040) complete it.

**Features (12):**

- EX (12): EX-021, EX-022, EX-023, EX-024, EX-026, EX-027, EX-029, EX-031, EX-032, EX-036, EX-039, EX-040

**Demo.** A user can export their edit as an MP4 that plays in any standard player, see a preflight warning about offline media first, and cancel a running export without leaving a partial file behind.

### M5 — Text, transform, transitions & clip audio — MVP complete

**Tier:** mvp · **Crates touched:** `timeline`, `render`, `audio`, `project`, `app`

**Goal.** Finish the MVP feature set on top of the working loop: text clips with the font picker, size, fill, bold/italic, paragraph layout and complex-script fallback; clip position/scale/rotation and flips with on-canvas handles, which also make picture-in-picture overlays on upper tracks useful; the transition model (shorten-track placement, FX-002) with the cross-dissolve and equal-power audio crossfade under it (the AU-029 gain law ships with the MVP mixer); clip volume, fades and mute with the gain overlay on waveforms. The inspector's MVP sections (Video › Transform, Audio › Volume & Fades, Text, Transition) are filled in.

**Features (17):**

- TR (3): TR-001, TR-003, TR-007
- TX (6): TX-001, TX-002, TX-003, TX-004, TX-005, TX-006
- AU (4): AU-005, AU-006, AU-007, AU-009
- FX (3): FX-002, FX-003, FX-007
- OV (1): OV-001

**Demo.** A user can make a short vertical video end to end — cut clips, add a title, place a picture-in-picture overlay, cross-dissolve between shots with the sound crossfading, fade the music out — and export a watermark-free MP4.

### M6 — Everyday editing, media management & workspace

**Tier:** v1 · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `app`

**Goal.** Make daily cutting fast and safe. Timeline: track lock, reorder and Flexible stacking, track height, names and colors, hover skim, marquee and select-forward, keyboard nudge, ripple trim and *Ripple all tracks*, blade, trim-to-playhead, gaps, attached items, detach/restore audio (with AU-003's settings hand-off), copy/cut/paste, duplicate, the Paste-attributes registry, replace, enable/disable, markers, in/out ranges with lift/extract, groups, compound clips, clip rename and trimming several clips at once, plus the shared batch-edit scopes with one history entry (TL-053) and the per-clip availability reasons (TL-054) that every multi-clip command reuses. Project safety nets: autosave, crash recovery, rolling backups, project management with the 30-day Deleted-projects hold, cover image, new-project presets, match-to-first-clip, disk-space monitoring with low-space protection, and storage-location advisories. Media library: folders, image sequences, duplicate detection, copy-into-project, interpretation overrides, the generated-media store, color clips, extract audio and multi-stream audio sources (stream picker, *Add stream as clip*), list view, bins, sort, search, usage, favorites, notes, info, hover scrub with media preview mode, replace file and the CC0 sample pack; cache management, proxies, batch relink and change detection. Player: loop and play range, clip start/end jumps and locate-in-timeline, zoom, fullscreen, guides and safe areas, rulers and user guide lines, vertical-feed overlay, ratio menu, snapshot, render cache, performance readout, playback priority (smooth or sharp), hardware decoding. Workspace: layout presets, the library-panel framework and hover previews, multi-selection inspector, themes and interface scale, the remaining Preferences sections, crash reports and safe mode, shortcut editor, cheat sheet, command palette, keyboard, screen-reader and visual accessibility, translations, first-run setup, hints with "don't ask again", settings import/export, logs, diagnostic bundles, the system check with its hardware rating, low-memory warnings and native OS integration.

**Features (113):**

- PM (36): PM-006, PM-008, PM-009, PM-010, PM-014, PM-015, PM-016, PM-018, PM-019, PM-020, PM-032, PM-033, PM-038, PM-039, PM-040, PM-041, PM-051, PM-052, PM-053, PM-054, PM-055, PM-056, PM-057, PM-059, PM-060, PM-061, PM-063, PM-074, PM-080, PM-081, PM-092, PM-093, PM-104, PM-108, PM-113, PM-114
- TL (28): TL-007, TL-009, TL-010, TL-014, TL-016, TL-017, TL-019, TL-021, TL-024, TL-026, TL-027, TL-029, TL-030, TL-032, TL-033, TL-034, TL-035, TL-036, TL-037, TL-038, TL-039, TL-040, TL-048, TL-049, TL-051, TL-053, TL-054, TL-057
- AU (3): AU-003, AU-004, AU-038
- EX (14): EX-008, EX-009, EX-010, EX-011, EX-012, EX-013, EX-014, EX-015, EX-016, EX-017, EX-018, EX-054, EX-055, EX-056
- UI (32): UI-003, UI-006, UI-007, UI-010, UI-014, UI-015, UI-017, UI-019, UI-020, UI-021, UI-022, UI-023, UI-024, UI-026, UI-027, UI-028, UI-032, UI-033, UI-034, UI-041, UI-042, UI-043, UI-045, UI-048, UI-049, UI-050, UI-052, UI-053, UI-058, UI-061, UI-062, UI-063

**Demo.** A user can cut a long 4K interview smoothly on proxies — with markers, in/out ranges, the blade, *Ripple all tracks*, groups and compound clips — organize the media into bins, rebind shortcuts, and lose nothing when FreeCut crashes mid-edit.

### M7 — Motion & look: keyframes, speed, color, effects & transitions

**Tier:** v1 · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `analysis`, `app`

**Goal.** Everything that changes how a clip moves and looks. The keyframe engine (TR-027) with inspector and timeline keyframe editing, easing presets and the curve editor, which every later animatable property plugs into; the rest of the transform (non-uniform scale, rotate 90°, anchor, canvas snapping, align and distribute), crop and straighten, canvas background fills, opacity and blend modes (OV-004), shape masks with on-canvas editing, chroma key with spill removal, matte preview and stabilization. Retiming: the time map, constant speed, speed-curve presets and editor, frame sampling, reverse and freeze frame, with AU-023's Keep-pitch time-stretch engine, audio scrubbing and J/K/L shuttle. The color stack: filters and looks, adjustment clips, light/color/detail/vignette/grain/fade, HSL, curves, wheels, LUTs, white-balance picker, auto color, color match and match groups (several clips to one reference), histogram, compare, reset, apply-to-all, user looks, keyframed color, the overall color amount, denoise and deflicker, and HDR source conversion options. Effects and transitions: clip effects, effect layers, parameters, stacking, keyframes, effect regions (masks on effects, optionally tracked), apply an effect to all clips, the built-in effect and transition libraries, transition parameters, apply-to-every-cut, default transition, copy/paste of effects and animations, entrance/exit and continuous animations. Compositing for overlays: stack as layers, PiP presets, border and rounded corners, outline and shadow, layer order, click-through selection, span across the project. The analysis job system (AI-001) arrives here with stabilization-aware motion tracking, introducing the proposed `freecut-analysis` crate, plus the auxiliary-panel slots (Scopes) and the shared save-as-preset dialog (UI-064), whose first user is *Save as look preset* (CL-026).

**Features (105):**

- TL (1): TL-031
- TR (36): TR-004, TR-005, TR-006, TR-008, TR-009, TR-010, TR-011, TR-012, TR-013, TR-014, TR-015, TR-016, TR-017, TR-018, TR-019, TR-022, TR-023, TR-024, TR-026, TR-027, TR-028, TR-029, TR-030, TR-031, TR-032, TR-033, TR-034, TR-036, TR-037, TR-038, TR-039, TR-040, TR-041, TR-043, TR-044, TR-045
- AU (2): AU-016, AU-023
- CL (35): CL-001, CL-003, CL-004, CL-005, CL-006, CL-007, CL-008, CL-009, CL-010, CL-011, CL-012, CL-013, CL-014, CL-015, CL-016, CL-017, CL-018, CL-019, CL-020, CL-021, CL-023, CL-024, CL-025, CL-026, CL-028, CL-030, CL-031, CL-033, CL-034, CL-035, CL-039, CL-041, CL-043, CL-044, CL-045
- FX (17): FX-004, FX-005, FX-006, FX-009, FX-010, FX-011, FX-012, FX-013, FX-014, FX-015, FX-016, FX-017, FX-018, FX-019, FX-026, FX-027, FX-030
- OV (8): OV-002, OV-003, OV-004, OV-005, OV-006, OV-007, OV-008, OV-009
- AI (3): AI-001, AI-007, AI-008
- EX (1): EX-007
- UI (2): UI-056, UI-064

**Demo.** A user can keyframe a zoom-and-pan, speed-ramp and reverse a clip, key out a green screen, stabilize and mask a shot, grade it with looks, curves and a LUT, add effects and library transitions, animate clips in and out, and pin a title to a moving object — all previewed live and exported identically.

### M8 — Words, sound, graphics & delivery

**Tier:** v1 · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `analysis`, `app`

**Goal.** Titles, captions, audio and output. Text: missing-font flow, decoration and case, spacing, outline, shadow, glow, background box, gradient, rich-text ranges, curved text, font import, style presets, saved styles, keyframable text, In/Out/Loop and per-unit animations with the text animation catalogue, the text inspector lenses, bubble and tag boxes, text templates. Captions: caption tracks and the Captions panel, the Captions drawer, captions recognized from speech on the user's own machine (the local speech model, its manager and confirmed downloads, word timings, filler-word marks), lyric captions and caption looks (pulled forward from M9 by the captions build), split/merge, track styling, timing tools, script-to-captions and tap-to-sync, subtitle import (SRT, WebVTT, ASS, LRC, SBV, timecoded TXT) and export (SRT, WebVTT, ASS, TXT, LRC), burn-in or embedded streams, find and replace, text↔caption conversion, bilingual lines and line rules. Graphics: animated images, alpha video, SVG, sticker clips and bundled packs, *My stickers*, emoji, shapes, lines and arrows, decorative frames, the elements library with search, favorites and recents, pack licence manifests and the credits sheet, element replacement, project templates with placeholder slots. Audio: volume keyframes, solo, mixer with track volume and pan, clip pan and channel map, meters and loudness, normalization, noise reduction, echo reduction, voice polish, voice effects, pitch, beat detection with tap compensation, voiceover recording, ducking, crossfades, *Mute video sound* and project mute, audio-tool availability with batch results, before/after compare for audio processing, music and sound-effect libraries, audio-only export, keyboard volume steps. Smart editing without models: silence removal, scene detection, beat-grid refinement and beat sync. Delivery: more codecs and 10-bit, custom size and 8K, custom rate control, hardware encoding, export ranges, size readout, background queue, built-in, user and platform presets, GIF, audio-only, several outputs from one export (a companion audio file or GIF), captions in export, cover and metadata, player audio monitoring, referenced-asset index and *Collect & package project*.

**Features (108):**

- PM (2): PM-042, PM-101
- TX (38): TX-007, TX-008, TX-009, TX-010, TX-011, TX-012, TX-013, TX-014, TX-015, TX-016, TX-018, TX-019, TX-020, TX-021, TX-022, TX-023, TX-024, TX-025, TX-026, TX-027, TX-028, TX-029, TX-030, TX-031, TX-032, TX-033, TX-034, TX-035, TX-036, TX-037, TX-038, TX-039, TX-044, TX-051, TX-054, TX-055, TX-056, TX-057
- AU (23): AU-008, AU-012, AU-013, AU-014, AU-015, AU-017, AU-018, AU-019, AU-022, AU-024, AU-025, AU-026, AU-027, AU-028, AU-029, AU-030, AU-031, AU-032, AU-036, AU-037, AU-039, AU-040, AU-041
- OV (23): OV-010, OV-012, OV-013, OV-014, OV-015, OV-018, OV-019, OV-020, OV-021, OV-024, OV-025, OV-027, OV-029, OV-030, OV-031, OV-032, OV-033, OV-036, OV-037, OV-038, OV-039, OV-040, OV-041
- AI (7): AI-002, AI-003, AI-004, AI-005, AI-006, AI-009, AI-010
- EX (15): EX-019, EX-025, EX-028, EX-030, EX-034, EX-035, EX-037, EX-038, EX-041, EX-042, EX-043, EX-044, EX-045, EX-046, EX-057

**Demo.** A user can style and animate titles, auto-caption an interview on their own machine, import, fix and burn in SRT captions, decorate with stickers, shapes and a saved template, clean up a voiceover and duck the music under it, cut to the beat, remove silences, and queue HEVC, ProRes, GIF and audio-only exports from presets.

### M9 — Later bucket

**Tier:** later · **Crates touched:** `timeline`, `media`, `render`, `audio`, `project`, `analysis`, `app`

**Goal.** Not one release: the `later` features, picked up as contributors have capacity, each still following spec → model → tests → UI. Model-backed tools all depend on AI-002 (model manager, explicit-consent downloads) and the ML runtime in `freecut-analysis`; the captions build brings both, with transcription (AI-009) and auto captions (AI-010, TX-038, TX-039), into M8, and every later pack reuses them. The chains that reuse each other: auto captions → keyword emphasis (TX-040), transcript editing, cleanup, translation and spoken-word search (AI-011, AI-012, AI-013/TX-041, AI-027); matting (AI-014) → refinement (AI-015), cut-out stickers (OV-023) and person effects (FX-020); face tracks (AI-021) → face effects (FX-021) and retouch (AI-020); the separation and speech-enhancement engines (AI-023, AI-024) → AU-021 and AU-020. The newer model-backed items join the same chains: transcription (AI-009) → dictation (TX-045) and clips from long recordings (AI-036); AI-016's subject detection → smart camera moves (AI-030); face tracks and retouch → retouch presets and makeup & hair tints (AI-032, AI-033); the depth records of virtual relighting (AI-034) → photo parallax (FX-029); TR-042's flow estimator → motion blur from footage (FX-028). Non-ML items (slip/roll, join clips, history panel, clip snippets and *Insert project* (both built on TL's `import_sequence`, which PM-107's multiple timelines reuse), OTIO interchange, alpha, image sequence, per-clip and command-line export, curved motion paths, multiple masks, text-shaped masks, perspective tilt, Lottie, pen-drawn shapes and image fills, charts (OV-051 first, then OV-052 to OV-054), sticker packs, progressive caption display, brand kit, project folders, detachable panels, several project windows, the guided tour, …) can land in any order once their dependencies above exist.

**Features (134):**

- PM (13): PM-043, PM-062, PM-082, PM-102, PM-103, PM-105, PM-106, PM-107, PM-109, PM-110, PM-111, PM-112, PM-115
- TL (9): TL-022, TL-044, TL-045, TL-046, TL-047, TL-050, TL-052, TL-055, TL-056
- TR (8): TR-020, TR-021, TR-025, TR-035, TR-042, TR-046, TR-047, TR-048
- TX (13): TX-017, TX-040, TX-041, TX-042, TX-043, TX-045, TX-046, TX-047, TX-048, TX-049, TX-050, TX-052, TX-053
- AU (6): AU-020, AU-021, AU-033, AU-034, AU-035, AU-042
- CL (9): CL-022, CL-027, CL-029, CL-032, CL-036, CL-037, CL-038, CL-040, CL-042
- FX (9): FX-008, FX-020, FX-021, FX-022, FX-023, FX-024, FX-025, FX-028, FX-029
- OV (21): OV-016, OV-017, OV-022, OV-023, OV-026, OV-028, OV-034, OV-035, OV-042, OV-043, OV-044, OV-045, OV-046, OV-047, OV-048, OV-049, OV-050, OV-051, OV-052, OV-053, OV-054
- AI (27): AI-011, AI-012, AI-013, AI-014, AI-015, AI-016, AI-017, AI-018, AI-019, AI-020, AI-021, AI-022, AI-023, AI-024, AI-025, AI-026, AI-027, AI-028, AI-029, AI-030, AI-031, AI-032, AI-033, AI-034, AI-035, AI-036, AI-037
- EX (11): EX-020, EX-047, EX-048, EX-049, EX-050, EX-051, EX-052, EX-053, EX-058, EX-059, EX-060
- UI (8): UI-046, UI-047, UI-054, UI-055, UI-057, UI-059, UI-060, UI-065

**Demo.** A user can edit by transcript, remove backgrounds and objects, reframe, upscale or colorize footage with local open-source models, and use advanced tools such as slip/roll, OTIO interchange and alpha or command-line export — without any cloud service.

### M-AI1 — Assistant: query tools and read-only chat

**Tier:** v1 · **Crates touched:** `timeline`, `media`, `project`, `assistant`, `app`

**Goal.** Create `freecut-assistant` with its dependency guard (timeline, project, and media without FFmpeg for the cancel token), the sans-I/O agent loop, the provider trait with the local OpenAI-compatible and Anthropic Messages providers behind the `http` feature, keys in the OS keychain, the disclosure dialog and *Show last request*, and the panel with streaming replies, activity lines and clip chips. Only read-only query tools exist; CI replays recorded model replies with no network. Prerequisite: M5.

**Features (9):**

- AS (9): AS-001, AS-002, AS-003, AS-004, AS-005, AS-010, AS-018, AS-019, AS-020

**Demo.** A user can switch on the assistant with a local model server or their own Anthropic key, ask questions about the project, click the answer's clip chips to find them, and see exactly what was sent.

### M-AI2 — Assistant: single-request edits with preview

**Tier:** v1 · **Crates touched:** `timeline`, `assistant`, `app`

**Goal.** Give the model one tool per current `EditRequest` (with the coverage test that forces every new request to get a tool or a documented exclusion), exact time and value parsing, the change summary, the proposal card, the ghost overlay and *Play proposal*, and commit an accepted proposal as one `AssistantEdit` history entry, with re-checks when the document changes first. Prerequisite: M-AI1.

**Features (5):**

- AS (5): AS-006, AS-007, AS-008, AS-009, AS-014

**Demo.** A user can ask for one edit in plain language, see it as a ghost on the timeline with a FreeCut-written change list, play it, and accept it as one undo entry.

### M-AI3 — Assistant: multi-step plans, macros and actions

**Tier:** later · **Crates touched:** `timeline`, `project`, `assistant`, `app`

**Goal.** Multi-step proposals through `PlanChain` (up to 200 steps, groups, dependency-aware unticking, partial accept, clarifying questions), macros built from the same requests (title card and apply-to-matching first; pause removal, transcript rough cut, captions and beat cut as AI-003, AI-006, AI-009/AI-011 and AI-010/TX-025 land), and actions that FreeCut confirms (export dialog, import of user-chosen files, local analysis jobs). Prerequisite: M-AI2.

**Features (3):**

- AS (3): AS-011, AS-012, AS-013

**Demo.** A user can ask for several edits at once, untick parts of the proposal, use ready-made recipes such as a title card, and have the export dialog opened pre-filled.

### M-AI4 — Assistant: vision- and transcript-aware editing

**Tier:** later · **Crates touched:** `media`, `render`, `audio`, `assistant`, `app`

**Goal.** Host queries for transcripts and word search, captions, rendered frames, loudness, beats and pauses, with frame consent and limits, so the model can cut on words and check what the picture shows. Prerequisites: M-AI3, AI-009, TX-025, AU-015, AU-025, AI-003.

**Features (3):**

- AS (3): AS-015, AS-016, AS-017

**Demo.** A user can edit by what is said and seen — "remove the part about pricing", "is the title readable?" — with transcripts, levels and frames read locally and only what AS-004 discloses sent.

### M-CG1 — Cloud generation: account, catalogue and cost

**Tier:** v1 · **Crates touched:** `cloud`, `app`

**Goal.** Slice H1 of [cloud-generation.md](../architecture/cloud-generation.md) §14. Create `freecut-cloud` (after the owner approves its CLAUDE.md lines, architecture C55) with its dependency guard, the MCP client (JSON-RPC over Streamable HTTP), OAuth sign-in in the system browser with a loopback redirect, the pasted-token path, keys in the OS keychain, the model catalogue and the cost preflight. The Generate tab exists with its forms and **Check cost**, but no Run. CI uses recorded exchanges with no network. H1 also builds the account notice of CG-004 and the preflight half of CG-005. Prerequisite: M5 and the owners' confirmation of the second *Local only* exception.

**Features (3):**

- CG (3): CG-001, CG-002, CG-003

**Demo.** A user can switch on cloud generation, connect their own Higgsfield account in the browser, browse the models in a Generate tab filtered to the project's aspect ratio, and see what a generation would cost — without spending anything.

### M-CG2 — Cloud generation: Generate tab, jobs and results

**Tier:** v1 · **Crates touched:** `timeline`, `media`, `render`, `project`, `cloud`, `app`

**Goal.** Slice H2: the run sheet with cost, Sends list and explicit Run; uploads through presigned PUTs; submission with no automatic resubmission; the cloud ledger and poller; downloads into the generated-media store with checksums; import as `GeneratedMediaKind::Cloud` items with recipes; placement as one undo entry; results that finish while the project is closed; limits and the service's questions. Prerequisite: M-CG1.

**Features (12):**

- CG (12): CG-004, CG-005, CG-006, CG-007, CG-008, CG-009, CG-010, CG-011, CG-012, CG-013, CG-025, CG-026

**Demo.** A user can generate images and short videos from a prompt or a frame, confirm each run's cost and what is sent, quit mid-run without losing it, and find the results in the Generated bin with a full record of how they were made.

### M-CG3 — Cloud generation: AI edit actions on clips

**Tier:** v1 (CG-014–CG-016) + later · **Crates touched:** `timeline`, `media`, `cloud`, `app`

**Goal.** Slice H3: the pure `source_section` rule, frame-accurate section uploads without metadata, and the clip actions. H3a (v1) builds edit with a prompt and extend; H3b adds background removal, upscale and generative reframe once recorded runs confirm their output formats; H3c adds dub, lip sync and voice change after the AI owner's decision (see Open questions); H3d (SDR to HDR) waits until HDR is reopened. Prerequisite: M-CG2.

**Features (10):**

- CG (10): CG-014, CG-015, CG-016, CG-017, CG-018, CG-019, CG-020, CG-021, CG-022, CG-023

**Demo.** A user can right-click a clip to edit a section with a prompt or extend it forward or backward, and later remove backgrounds, upscale, reframe, dub, lip-sync or change voices in the cloud — each placed back as one undo step.

### M-CG4 — Cloud generation: assistant tools

**Tier:** later · **Crates touched:** `assistant`, `app`

**Goal.** Slice H4: the assistant's model-list query and its two action tools, routed to the same run sheet; at most 4 sheets per turn; results placed through ordinary proposals. Prerequisites: M-AI3, M-CG2 and H3a.

**Features (1):**

- CG (1): CG-024

**Demo.** A user can ask the assistant for a generated shot or an extension and confirm the run in FreeCut's own cost sheet before anything is spent.

### Coverage

Every feature appears in exactly one milestone; all mvp features are in M0–M5, AS features are in the assistant track, and CG features are in the cloud generation track. Counts per spec:

| Milestone | PM | TL | TR | TX | AU | CL | FX | OV | AI | EX | UI | AS | CG | Total |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| M0 | · | · | · | · | · | · | · | · | · | · | 2 | · | · | 2 |
| M1 | · | 13 | · | · | · | · | · | · | · | · | · | · | · | 13 |
| M2 | 25 | · | · | · | · | · | · | · | · | · | 10 | · | · | 35 |
| M3 | 1 | 6 | 1 | · | 4 | 1 | 1 | 1 | · | 6 | 11 | · | · | 32 |
| M4 | · | · | · | · | · | · | · | · | · | 12 | · | · | · | 12 |
| M5 | · | · | 3 | 6 | 4 | · | 3 | 1 | · | · | · | · | · | 17 |
| M6 | 36 | 28 | · | · | 3 | · | · | · | · | 14 | 32 | · | · | 113 |
| M7 | · | 1 | 36 | · | 2 | 35 | 17 | 8 | 3 | 1 | 2 | · | · | 105 |
| M8 | 2 | · | · | 38 | 23 | · | · | 23 | 7 | 15 | · | · | · | 108 |
| M9 | 13 | 9 | 8 | 13 | 6 | 9 | 9 | 21 | 27 | 11 | 8 | · | · | 134 |
| M-AI1 | · | · | · | · | · | · | · | · | · | · | · | 9 | · | 9 |
| M-AI2 | · | · | · | · | · | · | · | · | · | · | · | 5 | · | 5 |
| M-AI3 | · | · | · | · | · | · | · | · | · | · | · | 3 | · | 3 |
| M-AI4 | · | · | · | · | · | · | · | · | · | · | · | 3 | · | 3 |
| M-CG1 | · | · | · | · | · | · | · | · | · | · | · | · | 3 | 3 |
| M-CG2 | · | · | · | · | · | · | · | · | · | · | · | · | 12 | 12 |
| M-CG3 | · | · | · | · | · | · | · | · | · | · | · | · | 10 | 10 |
| M-CG4 | · | · | · | · | · | · | · | · | · | · | · | · | 1 | 1 |
| **All** | **77** | **57** | **48** | **57** | **42** | **45** | **30** | **54** | **37** | **59** | **65** | **20** | **26** | **617** |

## Open questions

Collected from the specs. Each item names where it was raised; settle it in the owning spec (and remove it here) before the milestone that needs the answer.

### Architecture & crates

- **`analysis` crate: adopted, CLAUDE.md still to update.** Architecture C48 adopts `freecut-analysis` with captions slice CS2 ([AI-001](ai-tools.md#ai-001-background-analysis-jobs--result-cache--v1)): it depends only on `freecut-timeline`, only `freecut-app` depends on it, and whisper.cpp sits behind its `ml-whisper` feature (the app's default `stt`). CLAUDE.md's crate list and dependency sentence need the maintainer's update before CS2 lands (the proposed lines are in the round-3 build plan; architecture C54 keeps CS2's crate work from starting until they are approved); M7's analysis tools ([AI-003](ai-tools.md#ai-003-silence-detection--removal--v1), [AI-004](ai-tools.md#ai-004-scene--shot-detection--auto-split--v1)) use the same crate.
- **Record the implied dependency edges (M0).** CLAUDE.md states only `app` → everything, `render`/`audio` → `media` + `timeline`, and `timeline` → nothing. The specs also rely on `media` → `timeline` (it fills `freecut-timeline::media` types and uses [TL-001](timeline-editing.md#tl-001-rational-time--frame-grid--mvp)'s tick conversion, [PM-035](project-and-media.md#pm-035-media-probing--normalization--mvp)) and `project` → `timeline` (it maps the in-memory model, [PM-021](project-and-media.md#pm-021-project-file-format-versioning--migrations--mvp)).
- **Cheap document snapshots (M1).** Background saves ([PM-012](project-and-media.md#pm-012-save-save-as-save-a-copy--mvp)) and queued exports ([EX-038](preview-and-export.md#ex-038-background-export-and-export-queue--v1)) both need structurally shared, cheaply cloned snapshots of the document (`Arc`-shared nodes). [EX-038](preview-and-export.md#ex-038-background-export-and-export-queue--v1) calls this a TL design requirement, but timeline-editing.md does not yet say how the model is represented.
- **Two `JobQueue`s in `freecut-app`.** [PM-073](project-and-media.md#pm-073-background-task-list--media-jobs--mvp) defines a `JobQueue` for media jobs and [EX-038](preview-and-export.md#ex-038-background-export-and-export-queue--v1) defines `freecut-app::jobs::JobQueue` for exports; both feed [PM-073](project-and-media.md#pm-073-background-task-list--media-jobs--mvp)'s single task list. Rename one or merge them.
- **Command-line export (later).** [EX-052](preview-and-export.md#ex-052-command-line-export--later) leaves open whether `freecut export` is an entry point of `freecut-app` or a separate `freecut-cli` binary crate.
- **Optical-flow host crate (later).** [FX-028](effects-transitions-animations.md#fx-028-motion-blur-from-footage--later)'s motion blur from footage and [TR-042](clip-transform-and-speed.md#tr-042-optical-flow-smooth-slow-motion--later)'s smooth slow motion share one dense flow estimator; whether it runs as GPU compute in `freecut-render` or lives in `freecut-media` is to be decided for both together.

### Media, FFmpeg & codecs

- **Which codecs official builds ship.** Decoder availability ([PM-036](project-and-media.md#pm-036-supported-formats--mvp)) and encoder availability ([EX-029](preview-and-export.md#ex-029-baseline-format-h264--aac-in-mp4--mvp), [EX-030](preview-and-export.md#ex-030-more-codecs-and-containers-and-10-bit-output--v1), [EX-034](preview-and-export.md#ex-034-hardware-accelerated-encoding--v1)) depend on our FFmpeg build and on the licensing and patent situation of each platform's binaries. FreeCut detects codecs at runtime, but the build matrix is still an undecided packaging question.
- **Probe in a subprocess?** Native crashes inside FFmpeg while probing remain a risk ([PM-035](project-and-media.md#pm-035-media-probing--normalization--mvp)).
- **Alpha-capable proxy codec.** VP9 with alpha in WebM is the default; the final choice is open ([PM-080](project-and-media.md#pm-080-proxy-media--v1), [OV-014](overlays-and-stickers.md#ov-014-video-overlays-with-alpha--v1)).
- **Optimized-media intermediate codec** — still to be decided ([PM-082](project-and-media.md#pm-082-optimized-intermediate-media--later)).
- **Render-cache segment format** — ProRes 422 LT, pending performance tests ([EX-016](preview-and-export.md#ex-016-render-cache-and-pre-render--v1)).
- **Camera recording** — FFmpeg device inputs or the `nokhwa` crate as backend, and whether takes may use a hardware H.264 encoder ([PM-043](project-and-media.md#pm-043-camera-recording-into-the-project--later)).
- **Screen recording** — the capture backend per OS (FFmpeg's screen inputs, or the OS's newer capture APIs through safe crates; the desktop portal on Linux Wayland), and whether to offer a system-wide start/stop shortcut, which needs OS-level registration ([PM-115](project-and-media.md#pm-115-screen-recording-into-the-project--later)).
- **Live and motion photo metadata** — the exact XMP and container fields that pair a still with its motion part are to be confirmed from the vendors' public format documentation before implementation ([PM-109](project-and-media.md#pm-109-live-and-motion-photos--later)).
- **Drag-and-drop gaps** — macOS promised files (photo libraries) and Linux Wayland sessions without file drops currently fall back to the Import button ([PM-031](project-and-media.md#pm-031-drag-and-drop-import--mvp), [UI-036](workspace-and-shortcuts.md#ui-036-drag--drop-from-the-operating-system--mvp)).

### Audio

- **Recorder defaults to confirm (toolbar round)** — [AU-027](audio.md#au-027-voiceover-recording--v1) now blocks timeline edits while a take is counting down or recording, keeps a cancelled take in the Takes list instead of deleting it, and writes WAV with FreeCut's own RF64-capable writer instead of `hound` (audio-engine.md §12). [TL-057](timeline-editing.md#tl-057-linked-selection--v1) turns linked selection on by default, which changes [TL-030](timeline-editing.md#tl-030-embedded-audio-detach--restore--v1)'s earlier "partners are not auto-selected" default.
- **Time-stretch engine (needed by M7)** for Keep pitch, speed curves and shuttle audio: our own WSOLA or phase vocoder, signalsmith-stretch (MIT), or Rubber Band (GPL-2.0-or-later, whose licence blocks App Store distribution without a commercial licence) ([AU-023](audio.md#au-023-keep-pitch-and-the-time-stretch-engine--v1), [TR-038](clip-transform-and-speed.md#tr-038-audio-follows-the-time-map--v1), [EX-007](preview-and-export.md#ex-007-jkl-shuttle-and-variable-speed-playback--v1)).
- **Beat markers on detach** — copying a clip's beat markers to its detached audio clip is an open request to TL ([AU-025](audio.md#au-025-beat-detection-and-beat-markers--v1), [TL-030](timeline-editing.md#tl-030-embedded-audio-detach--restore--v1)).
- **Reverse shuttle audio** is silent for now, to be revisited after user testing ([EX-007](preview-and-export.md#ex-007-jkl-shuttle-and-variable-speed-playback--v1), [AU-016](audio.md#au-016-audio-scrubbing--v1)).

### Color

- **Auto stored under the sliders (Adjustments round 1)** — [CL-019](color-and-filters.md#cl-019-auto-color--v1) now keeps its result as offsets under the user's sliders, with a switch, a keyframable Strength and *Apply to sliders*, where the earlier draft wrote straight into the sliders. Confirm before K5.
- **Deleting saved looks and imported LUT files** ([CL-026](color-and-filters.md#cl-026-save-as-look-preset-user-looks--v1), [CL-016](color-and-filters.md#cl-016-lut-import-and-lut-library--v1)) — the workspace has no trash crate; either add one (for example `trash`) or delete permanently after a confirmation that says so. Decide before K3.
- **Lattice accuracy and dither** — the 33³ accuracy bounds (max 2/255, mean 0.4/255), the slope-4 switch to 65³ and the 1080p budget are estimates that K1 measures; round 1 has no dither, so strong curves on 8-bit sources may band until the GPU float path or an ordered dither exists (architecture C52).
- **LUT hash** — `LutRef` uses XXH3-128 instead of SHA-256 (CL-016); the spec owner signs off.

### Text & graphics

- **CJK fonts** — whether to bundle the Noto CJK pack by default or keep it a separate v1 download ([TX-006](text-and-captions.md#tx-006-unicode-complex-scripts-and-font-fallback--mvp)).
- **Text-stack licences** — cosmic-text, rustybuzz, fontdb, ttf-parser, swash and the Unicode/encoding crates are to be verified at integration; vertical layout ([TX-017](text-and-captions.md#tx-017-vertical-text-layout--later)) waits on shaping-stack support.
- **`resvg` licence** — [PM-034](project-and-media.md#pm-034-still--animated-images--mvp) lists it as MPL-2.0, while [OV-015](overlays-and-stickers.md#ov-015-svg-vector-overlays--v1) says recent releases are Apache-2.0/MIT; confirm at adoption and align both specs.
- **Lottie renderer** — `velato`/`vello` or `rlottie`, whose mixed component licences need review ([OV-016](overlays-and-stickers.md#ov-016-lottie-style-vector-animations--later)).
- **Sticker and chart crates** — `png` for APNG writing ([OV-047](overlays-and-stickers.md#ov-047-animated-stickers-from-numbered-frames--later)), `zip` for sticker packs ([OV-048](overlays-and-stickers.md#ov-048-export-stickers-as-a-shareable-pack--later)), and `csv` and `calamine` for spreadsheet import ([OV-052](overlays-and-stickers.md#ov-052-chart-data-table-editing-and-spreadsheet-import--later)) are marked "verify at adoption".

### Models & smart tools

- **Weight licences marked "verify"** in ai-tools.md's licence table (Silero VAD, Robust Video Matting, BiRefNet, LaMa, Real-ESRGAN, Demucs/Open-Unmix, DeepFilterNet, DDColor, the MediaPipe model cards including hair segmentation, YuNet, EfficientDet-Lite) must be confirmed before a model enters the manifest ([AI-002](ai-tools.md#ai-002-local-model-manager--ai-privacy-settings--v1), [AI-020](ai-tools.md#ai-020-face-retouch--later), [AI-033](ai-tools.md#ai-033-makeup--hair-color-tints--later), [FX-020](effects-transitions-animations.md#fx-020-person-tracking-effects--later), [FX-021](effects-transitions-animations.md#fx-021-face-tracking-effects--later)). [FX-029](effects-transitions-animations.md#fx-029-photo-parallax--later) also asks for the depth models' licences (MiDaS small, Depth Anything V2 Small), model cards included, to be verified before they are offered.
- **RIFE in-between fractions** — check that the chosen model version supports arbitrary fractions, not only fixed 2×/4× steps ([AI-018](ai-tools.md#ai-018-ai-frame-interpolation-for-smooth-slow-motion--later)).
- **Speaker diarization** — no permissively licensed local model has been chosen, so the automatic speaker detection planned for [TX-042](text-and-captions.md#tx-042-speaker-labels--later) has no engine yet (see ai-tools.md *Out of scope*).

### Keymap & spec alignment

- **Plain `S` splits in the app today** (R5 build), but [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp) gives `S` to hover skim ([TL-014](timeline-editing.md#tl-014-hover-skim-preview--v1)). The toolbar round's slice T1 removes the alias and leaves Split on `Ctrl+B` / `⌘B`; confirm before T1 ships, since users of the current build may rely on it.
- **Bindings still waiting for [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp)** — [AU-036](audio.md#au-036-keyboard-volume-steps--v1)'s volume steps (`Alt+=` / `Alt+-`, with `Shift` for 0.1 dB), [AU-025](audio.md#au-025-beat-detection-and-beat-markers--v1)'s tap key (`Alt+B`), [TL-017](timeline-editing.md#tl-017-select-forward--backward-from-playhead--v1)'s select forward/backward (`]` / `[` proposed), [TL-022](timeline-editing.md#tl-022-slip--roll-edits--later)'s slip tool (`Y`), [TL-046](timeline-editing.md#tl-046-reveal-source--match-frame--later)'s match frame (`F` in timeline scope), [TL-050](timeline-editing.md#tl-050-sub-frame-edit-points-for-audio-only-clips--later)'s sub-frame toggle, the layer commands of [OV-002](overlays-and-stickers.md#ov-002-stack-several-items-as-layers-at-the-playhead--v1) and [OV-007](overlays-and-stickers.md#ov-007-layer-order-commands--v1), and the newer commands that their specs leave unbound by default: [TL-052](timeline-editing.md#tl-052-join-clips-heal-a-through-edit--later)'s *Join clips*, [TR-045](clip-transform-and-speed.md#tr-045-align-and-distribute-on-the-canvas--v1)'s align and distribute, [AU-041](audio.md#au-041-beforeafter-compare-for-audio-processing--v1)'s compare, [EX-054](preview-and-export.md#ex-054-clip-startend-jumps-and-locate-in-timeline--v1)'s clip start/end jumps and [EX-055](preview-and-export.md#ex-055-player-rulers-and-guide-lines--v1)'s rulers and guides, plus [OV-049](overlays-and-stickers.md#ov-049-custom-path-shapes-drawn-with-a-pen-tool--later)'s proposed `Esc` / `Enter` while drawing a path. [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp) must either bind each one or list it as unbound by default, so its collision test covers them.
- **Default-transition key** — [FX-009](effects-transitions-animations.md#fx-009-default-transition-and-quick-add--v1) still suggests `Ctrl/Cmd+T` for *Add default transition*, but [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp) gives `Ctrl+T` to *Add text* and the transition `Ctrl+Shift+T`. Add the row to [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp)'s superseded-proposals table and update [FX-009](effects-transitions-animations.md#fx-009-default-transition-and-quick-add--v1).
- **Time display formats** — [TL-002](timeline-editing.md#tl-002-timecode-display--ruler--mvp) promises `HH:MM:SS.mmm` and frame-count options, while [UI-019](workspace-and-shortcuts.md#ui-019-general-settings-startup-time-display-tooltips--v1) defines Timecode, Seconds (`12.40 s`) and Frames (`298 f`). Pick one set.
- **Track tab reference** — [UI-008](workspace-and-shortcuts.md#ui-008-inspector-panel-with-context-dependent-tabs--mvp)'s Track tab cites [TL-010](timeline-editing.md#tl-010-track-height-collapse--naming--v1) for "name & color"; track color is [TL-049](timeline-editing.md#tl-049-track-color--v1).
- **Multiple timelines** — [PM-107](project-and-media.md#pm-107-multiple-timelines-in-one-project--later) (later) specifies several timelines per project, while timeline-editing.md's *Out of scope* and [TL-053](timeline-editing.md#tl-053-batch-edits-scopes-skipped-items--one-history-entry--v1)'s scopes keep one top-level sequence per project, citing the *Deferred by decision* list below. Decide whether PM-107 stays; if it does, update [TL-003](timeline-editing.md#tl-003-sequence--timeline-duration--mvp), [TL-043](timeline-editing.md#tl-043-undo--redo--mvp) and TL's *Out of scope* (as PM-107 itself asks) and drop the item from that list.
- **Relighting and depth** — color-and-filters.md still says ML relighting is in AI's *Out of scope*, but [AI-034](ai-tools.md#ai-034-virtual-relighting--later) now specifies virtual relighting; and [FX-029](effects-transitions-animations.md#fx-029-photo-parallax--later) says ai-tools.md has no depth-map analysis kind, although AI-034 stores depth maps as AI-001 records. Point CL's *Out of scope* and FX-029 at AI-034.

### Assistant

- **Adopt the `assistant` crate (needed by M-AI1).** [llm-editor.md](../architecture/llm-editor.md) §1 proposes `freecut-assistant`, depending on `timeline`, `project` and `media` without FFmpeg (for the cancel token only); only `app` depends on it. CLAUDE.md's crate list and dependency sentence and the architecture README's §1.1, §1.2 and §2.1 need the matching rows when it is created (architecture README C45).
- **The hosted-provider exception to *Local only*.** The convention above and ai-tools.md's *Out of scope* now name the opt-in assistant as the single exception (AS-002, AS-004); the owners confirm.
- **Three pure additions to the timeline (M-AI2).** `PlanChain` with `OpKind::AssistantEdit`, `edit::summarize` (which TL-044's history panel could reuse) and `edit::REQUEST_KINDS` for the coverage test; TL's owner signs off ([AS-008](assistant.md#as-008-accept-adjust-or-discard--v1), llm-editor.md §8).
- **Where the per-project "never use a remote assistant" switch lives** — preferences or the project file, so that it travels with a shared project ([AS-004](assistant.md#as-004-what-leaves-the-machine-disclosure-and-consent--v1)).
- **Panel and accept keys** for [UI-031](workspace-and-shortcuts.md#ui-031-default-keyboard-shortcut-map--mvp), and the "ask" row in [UI-034](workspace-and-shortcuts.md#ui-034-command-palette--v1)'s command palette.
- **Recommended local models** are chosen from M-AI1's live evaluations (llm-editor.md §7.4, §10).
- **Second-line translation through the assistant provider** (architecture C51, [AI-013](ai-tools.md#ai-013-caption-translation--bilingual-captions--later), [TX-057](text-and-captions.md#tx-057-captions-drawer--v1)): opt-in, caption text only, AS-004 disclosure each run. It widens where the provider is used beyond the assistant panel; the AS and AI owners confirm before captions slice CS4 builds it.

### Cloud generation

- **A second exception to *Local only* (needed before M-CG1).** [cloud-generation.md](cloud-generation.md) proposes opt-in cloud generation with the user's own Higgsfield account. The conventions above, ai-tools.md's *Out of scope* ("any cloud processing, account, credits or online model service") and the *Deferred by decision* list below ("every cloud, account or online-store feature") all need the owners' decision; architecture C55 records the design.
- **Adopt the `cloud` crate.** [cloud-generation.md](../architecture/cloud-generation.md) §2 recommends `freecut-cloud` (depends on `timeline` and on `media` without FFmpeg; only `app` depends on it; networking behind its `http` feature) over putting it in `assistant` or in `app`. CLAUDE.md's crate list, dependency sentence and one convention line need the maintainer's approval of the exact lines in §2.4 before H1.
- **How the user brings their own token.** Higgsfield's MCP server documents browser sign-in (OAuth), not a pasteable key; CG-001 makes browser sign-in the main path and keeps *Paste an access token* as an advanced path. The REST API key pair (separate billing) is not offered until Higgsfield confirms a desktop app may store a user's own key ([CG](cloud-generation.md#out-of-scope-and-why)).
- **Ask Higgsfield before release:** native-app registration with loopback redirects, use by a third-party open-source desktop client, naming, and the REST key question. Nothing public allows or forbids it today.
- **Dub, lip sync and voice change** ([CG-021](cloud-generation.md#cg-021-dub-to-another-language--later), [CG-022](cloud-generation.md#cg-022-lip-sync-to-an-audio-clip--later), [CG-023](cloud-generation.md#cg-023-voice-change--later)) conflict with ai-tools.md's *Out of scope*, which excludes them for misuse and licence reasons; the AI owner decides whether the cloud versions, with a consent tick and preset voices only, are acceptable.
- **SDR to HDR** ([CG-020](cloud-generation.md#cg-020-sdr-to-hdr--later)) stays hidden while HDR is deferred.
- **New `GeneratedMediaKind::Cloud` and `TaskOwner::Cloud`** variants for PM-041 and PM-073, PM-041's provenance fields on `MediaOrigin::Generated` (the code carries only `kind` today; the recipe is stored as JSON text so `timeline` needs no JSON library; architecture §9), and the pure `source_section` function in `timeline` (CG-014); the PM and TL owners sign off.
- **Actions with no cost estimate.** The service's dubbing and voice-change tools give no estimate before running. CG-021 and CG-023 are built only if the service adds one or the owners accept a named "cost shown only afterwards" exception ([CG-005](cloud-generation.md#cg-005-cost-preflight-and-run-confirmation--v1)); every v1 action has an estimate.
- **Shipping default.** The runtime switch is off by default (CG-001). The owners also decide whether release builds include the `cloud` feature at all before H2's acceptance and Higgsfield's answer on third-party use.
- **Higgsfield's provenance marks and developer terms.** Ask Higgsfield whether export dropping file-level provenance metadata from its outputs is acceptable, and whether an open-source client acting on each user's own sign-in has developer obligations of its own (architecture §15).
- **Where the per-project "never use cloud generation" switch lives** — the same question as the assistant's (AS-004).

### Deferred by decision (not open)

HDR working space and HDR export (EX-033 stays reserved; revisit only jointly in CL, EX and PM; CL-041 only tunes the HDR-to-SDR conversion of sources), surround mixing and export, persisting undo history across sessions, several top-level sequences per project (TL's decision, now challenged by PM-107; see *Multiple timelines* above), multicam angle editing, third-party plugin hosting, voice cloning, and every cloud, account or online-store feature are out of scope on purpose (the user-configured provider of the opt-in assistant, AS-002, is not a FreeCut service; opt-in cloud generation with the user's own account, CG, is proposed as a second exception and is listed under *Cloud generation* above). Each spec's *Out of scope (and why)* section gives the reasons.
