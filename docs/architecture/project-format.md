# Project file format, versioning & interchange
> Status: draft · Crates: freecut-project (format, migrations, I/O, interchange), freecut-timeline (in-memory model types it maps to), freecut-app (save coordinator, autosave scheduler, dialogs)

This document designs how a FreeCut project is written to disk, read back, upgraded across
versions, protected against crashes and partial writes, kept portable across folders, volumes
and operating systems, and exchanged with other editors through OpenTimelineIO (and later
FCPXML/EDL). It is written from the behavior specs in `docs/features/` and from a source-level
study of six open-source editors and frameworks. Nothing here is modeled on any proprietary
project format.

---

## What FreeCut needs

Requirements distilled from `docs/features/project-and-media.md` (PM), `timeline-editing.md`
(TL), `clip-transform-and-speed.md` (TR), `color-and-filters.md` (CL),
`effects-transitions-animations.md` (FX), `overlays-and-stickers.md` (OV),
`preview-and-export.md` (EX), `audio.md` (AU), `ai-tools.md` (AI) and the paraphrased behavior
inventory (`scratchpad/inventory/project-and-media.md` §4–5, §8.6, §11).

### File format

| # | Requirement | Source |
|---|---|---|
| F1 | One UTF-8 JSON document per project (`<Name>.freecut`), no binary payloads; caches, proxies, peaks, thumbnails live elsewhere. | PM-021 |
| F2 | Envelope `{format, format_version, app_version, header, settings, library, sequences, cover, …}`; `header` duplicates id, name, created/modified (RFC 3339 UTC), duration, canvas, frame rate for fast listing. | PM-021, PM-001 |
| F3 | All time values are integer ticks (1/705,600,000 s); no floats for time anywhere in the file. | TL-001, PM-021 |
| F4 | A sequence rate whose frame duration is not a whole number of ticks is refused on load, never guessed. | TL-001, PM-013 |
| F5 | Deterministic output: stable key order, 2-space indent, trailing newline, clean VCS diffs. | PM-021 |
| F6 | View state (zoom, scroll, panel sort/filter, track heights, source marks) is **not** in the project; it lives in app data `view/<project-id>.json` so browsing never dirties the project. | PM-021 (conflicts noted below) |
| F7 | Non-UTF-8 Unix paths stored as `{ "unix_bytes": "<base64>" }`; Windows extended-length and UNC paths preserved. | PM-021, PM-100 |
| F8 | Enums serialized as stable kebab-case strings. | OV (blend modes), general |
| F9 | Optional sections omitted when at default (color stack sections). Fields added later deserialize with defaults so older files load. | CL-001, TR-001 |
| F10 | Unknown effect / transition kinds are kept verbatim, bypassed or rendered as fallback, flagged in the UI, never dropped. Unknown keys inside effect params from newer files are preserved on save. | FX-002, FX-012 |
| F11 | Persisted per-project data owned by other areas: master audio (AU), last-used export settings (EX), cover frame `Option<Tick>` (PM-019), transcripts with user corrections (AI-011), generated-media provenance `MediaOrigin` (PM-041). | AU, EX, PM-019, AI, PM-041 |

### Versioning & migration

| # | Requirement | Source |
|---|---|---|
| V1 | `format_version` is an integer starting at 1, bumped for every schema change. | PM-021 |
| V2 | Each bump ships a pure `migrate_vN(serde_json::Value) -> Result<Value, MigrationError>`; chained on load; each has fixture tests (old file in, expected new file out). Migration works offline. | PM-021, inventory §5.1 |
| V3 | After migration, the first save keeps the pre-migration file once as `<name>.freecut.v<N>.bak`; the user sees "Project upgraded…". | PM-021 |
| V4 | A file with a higher `format_version` opens read-only; Save and Save a Copy are disabled because unknown data would be lost. | PM-021, PM-001 ("Newer version" badge) |
| V5 | Unknown fields at the current version (e.g. from a development build) are kept in memory as an opaque map, written back, and logged. | PM-021 |
| V6 | Other versioned serde formats reuse the same machinery: look files (CL-026 `LookFile`), templates (OV-030/031), export presets and queue (EX), registry and presets (PM-017, PM-020). | CL, OV, EX, PM |
| V7 | TR's single-mask → mask-list change is an example of a structural migration the chain must express. | TR (masks) |

### Lifecycle, safety and portability

| # | Requirement | Source |
|---|---|---|
| L1 | Save: cheap immutable snapshot on the UI thread; serialization and I/O on a background thread; write `<file>.tmp`, flush, fsync, atomic rename. At most one save in flight, one queued. | PM-012 |
| L2 | Save As recomputes relative paths and moves the lock; Save a Copy writes a new project id. Saving moves TL's saved position (dirty flag) but never clears undo history. | PM-012, TL-043 |
| L3 | Disk full / permission denied: target untouched, temp removed, project stays dirty, retry. File changed on disk since last load/save → Overwrite / Save As / Cancel. Non-atomic network rename → copy old to `<file>.prev` first. | PM-012 |
| L4 | Open = parse → migrate → validate/repair → build model → background path resolution and media checks. Repairs are reported; a repaired project opens dirty with an empty history; the original is kept once as `.bak-YYYYMMDD`. | PM-013, TL-004 (main-track repair), TL-040 (`validate()`) |
| L5 | Single-writer lock `<file>.lock` `{pid, hostname, user, app_version, started, refreshed}`, refreshed every 60 s, stale after 2 min or when the process is gone; read-only open offered. | PM-013 |
| L6 | Autosave (v1): 2 s debounce, ≥ every 30 s while editing, also on focus loss / before export / close; only committed ops; exponential backoff on failures; slow-save adaptation. | PM-014 |
| L7 | Crash recovery (v1): `recovery/<project-id>.freecut` in app data while the project file itself is not being written; prompt on next launch; corrupt snapshot → newest backup. | PM-015 |
| L8 | Rolling backups (v1): app data `backups/<project-id>/<UTC>.freecut`, newest 20 + one per day for 14 days, 500 MB cap with newest 3 per project kept, `prune` pure and tested. | PM-016 |
| L9 | Stable ids: TL ids are stable `u64`s from persisted counters so ids are never reused after undo/redo; the project id is a UUID; a copied project (same id, different path) gets a fresh id on open. | TL-006, TL-023, PM-001 |
| L10 | Portable paths: each reference stores the absolute path and a relative path (≤ 8 `..`, same volume only), resolution order depends on whether the project moved; fix-ups are runtime state, not edits; relative paths use `/`. Cross-OS (Windows path on macOS) falls back to relative then relink. | PM-100, PM-091 |
| L11 | Fingerprint `{size, xxh3 of first+last 1 MiB, duration}` identifies content for relink, dedupe, cache keys. | PM-038 |
| L12 | Packaging: copy of the project with a new id, media/assets collected, paths rewritten relative, verified by size + fingerprint. | PM-101, PM-042 |

### Interchange

| # | Requirement | Source |
|---|---|---|
| I1 | OTIO export/import (later): media references (paths and ranges), tracks, clips, gaps, markers; effects/text/transforms not mapped and reported; import creates a new project or new tracks; rate mismatches converted exactly through ticks; missing media → offline items; unsupported schema features skipped and listed. OTIO serde mapping written ourselves from the public schema, in `freecut-project::interop`. | PM-103 |
| I2 | The reference editor imports XML/FCPXML/AAF timelines; FreeCut plans OTIO first, FCPXML and EDL later. | inventory §8.6 |
| I3 | Cross-project paste reuses project clip serialization. | TL-032 |

### Spec conflicts this design has to resolve

1. **View state location.** PM-021 puts view state in app data, but TL-010 says track heights are "persisted in the project's UI section", TL-038 says range marks are "persisted in the project", and EX refers to `freecut-project::ui::PlayerUiState`. Resolution here: the *types* live in `freecut-project::view`, the *data* lives in the app-data sidecar of PM-021, keyed by stable ids. The project file never contains view state.
2. **`assets` key.** PM-021 lists an `"assets"` top-level section; PM-042 says the asset index "is computed on demand, never stored". Resolution: no `assets` section (asset references live at their use sites, e.g. `LutRef` inside a color stack); the key is reserved.
3. **Id counters.** TL-006/TL-023 describe a per-sequence id counter. Compound-clip create/break-apart (TL-040) moves clips between sequences, so per-sequence counters can collide. Resolution: one document-wide monotonic allocator (see [Stable ids](#stable-ids)).
4. **Unresolvable clips.** PM-013 says repair "drops unresolvable clips". This design proposes a lossless repair instead (placeholder offline media item) and lists it as an open question.
5. **Analysis data.** AI-001 says `AnalysisRecord` is "persisted by `freecut-project`". Regenerable analysis belongs in the evictable cache (PM-074) keyed by fingerprint; only user-edited data (transcripts) belongs in the project document.

---

## How the reference editors do it

All paths below are relative to each repository root under `scratchpad/oss/<repo>/`.

### OpenTimelineIO (Apache-2.0)

**Model and format.** Every serialized object carries `"OTIO_SCHEMA": "<Name>.<version>"`; there
is deliberately **no file-level version** because any object (a Clip, a RationalTime) can be a
top-level document (`docs/tutorials/otio-file-format-specification.md`). Time is
`RationalTime { value: double, rate: double }` (`src/opentime/rationalTime.h`), ranges are
`TimeRange { start_time, duration }`. Tracks are *sequential* (items are laid end to end, gaps
are explicit `Gap` items); a `Stack` composites its children; transitions overlap neighbours
without changing the track's length (`docs/tutorials/time-ranges.md`,
`docs/tutorials/otio-timeline-structure.md`). A clip's `source_range` is expressed in the
*media's* time frame, which includes the media's start timecode carried by
`available_range.start_time` ("wedding.mov starts at 01:00:00:00").

**Schema evolution.** `src/opentimelineio/typeRegistry.cpp` registers per-schema upgrade
functions (Marker 1→2 renames `range`→`marked_range`; Marker 2→3 turns a color name into a
`Color` object; Clip 1→2 wraps `media_reference` into a `media_references` map with an
`active_media_reference_key` of `"DEFAULT_MEDIA"`) and downgrade functions for writing older
versions. `src/opentimelineio/CORE_VERSION_MAP.cpp` maps release labels ("0.15.0", …) to the
schema versions of that release so a newer library can write files an older one reads
(`docs/tutorials/versioning-schemas.md`). Adding a field with a correct default needs no bump
("the parser ignores values that aren't in the schema").

**Forward compatibility.** Unknown schema names become `UnknownSchema` objects that keep the
original name, version and data and are written back unchanged
(`src/opentimelineio/unknownSchema.h`, `typeRegistry.cpp::_instance_from_schema`). Unknown keys
inside a *known* schema land in `_dynamic_fields` and are re-serialized
(`src/opentimelineio/serializableObject.cpp`, `read_from`). But a *known* schema at a *higher*
version than the library supports is a hard error (`SCHEMA_VERSION_UNSUPPORTED` in
`_instance_from_schema`).

**Pitfalls found in the source.**

1. *Inclusive upgrade guard.* The upgrade loop applies every function whose target version
   lies in `[file_version, current]`:
   ```cpp
   // src/opentimelineio/typeRegistry.cpp (Apache-2.0)
   for (const auto& e: type_record->upgrade_functions)
   {
       if (schema_version <= e.first
           && e.first <= type_record->schema_version)
   ```
   As written, an "upgrade *to* N" function also runs on a dict that is already at version N.
   For a `Marker.2` file read by a Marker-3 library, the 1→2 function reads the absent `range`
   key (`operator[]` inserts an empty value) and overwrites `marked_range` with it; the typed
   read of `marked_range` should then fail with a type mismatch
   (`deserialization.cpp::_fetch`). The tests read `Marker.1` (`tests/test_marker.py`) and
   `Marker.3` samples (`tests/sample_data/*.otio`) but no `Marker.2` file, so the intermediate
   path is untested. Lesson for FreeCut: step exactly once per version (`from == v`), and test
   migration from **every** historical version, not only the oldest.
2. *Float time.* `value` and `rate` are doubles; NTSC rates are stored as the double of
   `24000/1001` etc. (`src/opentime/rationalTime.cpp`, `smpte_timecode_rates`), and other
   tools write `23.976` or `29.97`. `RationalTime::from_frames` truncates with `int(frame)`.
   Any importer must snap rates and round values deliberately.
3. *Non-standard JSON.* OTIO writes and reads `NaN`, `Infinity`, `-Infinity` (`Inf` on read)
   via RapidJSON's `kWriteNanAndInfFlag` / `kParseNanAndInfFlag`
   (`src/opentimelineio/serialization.cpp`, `deserialization.cpp`), and metadata may contain
   integers beyond 64 bits (`tests/sample_data/big_int.otio`). A strict JSON parser rejects
   such files.
4. *Newer-version refusal.* Because readers refuse newer schema versions, a writer that always
   emits the newest schemas breaks older readers. Writing the **oldest** schema version that
   carries the data is the most compatible choice.
5. *File URLs.* `bundle.cpp::file_from_url` percent-decodes the path and handles
   `file://C:/…`, `file://host/share` (UNC) and `file://localhost/…`, while the Python
   `url_utils.py::url_from_filepath` writes **unquoted** URLs (`urllib.parse.unquote(...)`).
   A filename containing a literal `%` followed by two hex digits does not round-trip.
   `percent_decode` also uses `std::stoi` on the two following characters, which throws on
   malformed escapes.
6. *Instancing.* Repeated objects are normally written as copies; with the compile-time option
   `OTIO_INSTANCING_SUPPORT`, `OTIO_REF_ID` / `SerializableObjectRef.1` references appear
   instead (`serialization.cpp`). Importers should tolerate both.

**Bundles.** `.otiod` (directory) and `.otioz` (zip with `version.txt`) hold `content.otio` plus a
flat `media/` folder with unique basenames (`docs/tutorials/otio-filebundles.md`,
`bundle.cpp::register_bundle_file`, which treats case-insensitive name collisions from
different folders as an error).

### Kdenlive (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)

**Format.** MLT XML with Kdenlive properties (`kdenlive:docproperties.*`,
`kdenlive:sequenceproperties.*`); five format generations are documented in
`dev-docs/fileformat.md`, including generation 4, which "fixed a long standing issue with the
decimal separator (comma/point)" and is not backward compatible.

**Versioning.** `src/doc/kdenlivedoc.cpp` defines `DOCUMENTVERSION = 1.1` as a **double** plus
an integer `DOCUMENTPATCHVERSION` for "minor corrections that do not break opening with
previous versions". `src/doc/documentvalidator.cpp`:
- parses the version with locale fallbacks (document locale, `LC_NUMERIC`, then replacing `,`
  by `.`), because old files wrote `0,88`;
- refuses newer versions (`version > currentVersion`) and versions 0.5 and 0.7;
- upgrades in one ~1,700-line `upgrade()` function made of `if (version <= 0.81)` …
  `if (version < 1.04)` blocks (mixing `<=` and `<` on floats), mutating the DOM in place and
  marking `upgraded="1"`;
- runs `convertSubtitles()` for `version <= 1.1`, i.e. on every current-version load too, so
  that step must be idempotent;
- applies **semantic** fixes that are not schema changes: `upgradePatchVersion` writes an
  explicit `rotate_anchor="0 0"` into old `qtblend` filters because MLT changed the default
  rotation centre; MLT-version-dependent fixes remove `mute_on_pause` for MLT ≤ 7.15;
- repairs locale-damaged numbers with regexes in `upgradeTo100`.

**Open/save.** `KdenliveDoc::Open` can recover a corrupted XML file by deleting the character at
the parser's error position (for files from Kdenlive 0.9.4), and calls
`QDir::setCurrent(...)` so relative paths resolve, a process-wide side effect.
`saveSceneList` copies the previous file to app data `.backup/` (`backupLastSavedVersion`),
writes a visible `<name>_backupN.kdenlive` next to the project when the document was upgraded
or modified on open, then writes through `QSaveFile` (temp + rename).

**Backup retention bug.** `cleanupBackupFiles` buckets backups by age:
```cpp
// src/doc/kdenlivedoc.cpp (GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL)
if (d.secsTo(resultList.at(i).lastModified()) < 3600) {
    // files created in the last hour
```
`QDateTime::secsTo` returns a negative number when the argument is earlier, so as written every
past backup lands in the "last hour" bucket and the day/week buckets stay empty. The thinning
loop that follows (`step = count / 10; … removeAt(i); --i;`) is also hard to reason about.
Lesson: retention must be a pure function with table tests and a fixed clock.

**Autosave and crash detection.** `ProjectManager` (`src/project/projectmanager.cpp`) uses
`KAutoSaveFile` keyed by an MD5 of the project file name inside the project folder; recovery is
offered when the autosave file's mtime is newer than the project's. `slotAutoSave` skips while
an operation is in progress and refuses to write a scene without `"<track "` ("In some
unexplained cases, the MLT playlist is corrupted and all tracks are deleted"). The actual write
in `KdenliveDoc::slotAutoSave` truncates and rewrites the file in place (`resize(0)` + `write`),
which is not atomic. Crash detection is a single empty `kdenlivelock` file in the temp folder
(`src/core.cpp`), without pid or host.

**Missing media and assets.** `src/doc/documentchecker.cpp` recomputes a "root replacement" when
the saved `root` no longer exists, relocates resources by common-ancestor heuristics
(`relocateResource`), recovers clips that were saved as `qtext` "INVALID" placeholders when
their source was missing at save time, and marks effects or transitions that are not installed
as `MissingStatus::Remove`, after which `removeAssetsById` deletes them from the document (a
`renamedEffects` table handles known renames). Media identity is an MD5 of the first and last
1 MB plus the file size (`src/bin/projectclip.cpp::calculateHash`).

### MLT XML module (LGPL-2.1-or-later)

`src/modules/xml/consumer_xml.c` writes `LC_NUMERIC`, the MLT `version` and an optional `root`
on `<mlt>`, and converts absolute paths to relative ones by a **string-prefix match of every
property value** against `root`. Only descendants of `root` become relative (no `..`), and
the comparison is case-sensitive. `src/modules/xml/common.c::mlt_xml_prefix_size` exists
because some `resource` values pack extra data before the path (`timewarp` writes
`speed:path`, where the speed may use `,` as decimal separator). On load,
`src/modules/xml/producer_xml.c::qualify_property` prepends `root` unless the value looks
absolute, has a drive letter, or starts with a whitelisted scheme; `is_known_prefix` compares
`strncmp(whitelist[i], resource, n)` with `n` = length of the value's prefix, so any short
token before `:` that is a prefix of a whitelisted scheme name also counts as a scheme. In/out
points can be written as clock strings `%02d:%02d:%06.3f` and parsed back with
`floor(fps*h*3600) + floor(fps*m*60) + lrint(fps*s)`, with explicit "floating point error"
fix-ups (`src/framework/mlt_property.c::time_clock_to_frames`, `time_clock_from_frames`).
Everything-is-a-string property bags preserve unknown properties for free, but they force
heuristics for paths versus URLs, packed values and locale-formatted numbers.

### Shotcut (GPL-3.0-or-later)

- **Save.** `src/mltcontroller.cpp::Controller::saveXML` sets project properties on the live
  service, temporarily clears `ignore_points`, rewires the service's consumer for the XML
  consumer, and writes through `QSaveFile` with `setDirectWriteFallback(true)`, which writes
  the target directly when a temp file can't be created (atomicity traded for working in odd
  folders). `ProxyManager::filterXML` (`src/proxymanager.cpp`) then rewrites the serialized XML
  stream to swap proxy paths back to originals and make paths relative.
- **Autosave.** `src/autosavefile.cpp` names the recovery file `appdata/autosave/<md5 of full
  path>.mlt`, so renaming or moving a project loses its autosave association, and all untitled
  projects share one name. `MainWindow::onAutosaveTimeout` runs `doAutosave` on a
  `QtConcurrent` worker that calls `saveXML` on the **live** timeline model while the UI thread
  keeps editing (`src/mainwindow.cpp`); the playlist branch of `MainWindow::saveXML` even
  mutates the producer's in/out during the save and restores it afterwards.
- **Backups.** `MainWindow::backup` copies the project next to itself as
  `<name> <ISO mtime>.mlt`; `backupPeriodically` runs on a user-chosen period.
- **Load checks.** `src/mltxmlchecker.cpp` rewrites the file into a temp copy before loading:
  it normalizes `LC_NUMERIC` to `C` and fixes decimal separators (`checkNumericString`),
  refuses MLT major versions above 7, repairs a bug of one release
  (`fixVersion1701WindowsPathBug` strips the leading `/` of `/C:/…` paths), breaks
  self-inclusion (`checkIncludesSelf`), collects unlinked files and replaces them, and swaps
  GPU-only services for CPU ones.
- **Media hash.** `src/util.cpp::Util::getFileHash` is "intentionally copied from Kdenlive"
  (MD5 of first + last 1 MB); `Util::getHash` caches it in the document property
  `shotcut:hash` and only computes it when empty, so a changed file keeps its old hash.

### Olive (GPL-3.0-or-later)

- **Versioning by parallel loaders.** `app/node/project/serializer/serializer.cpp` stores a
  `YYMMDD` integer version and keeps one complete serializer class per format version
  (`serializer210528.cpp` … `serializer230220.cpp`, about 4,000 lines). Each old loader reads
  directly into the *current* in-memory model, so every model change must be reflected in
  every old loader; the 0.1 loader is left as a `FIXME` and those projects are reported as
  "too old". Saving always uses the newest serializer. Files may be compressed with an `OVEC`
  magic + `qCompress`.
- **Non-atomic replace.** Save writes a temp file and then calls:
  ```cpp
  // app/common/filefunctions.cpp (GPL-3.0-or-later)
  if (QFileInfo::exists(to) && !QFile::remove(to)) {
    ...
  }
  if (!QFile::rename(from, to)) {
  ```
  Between `remove` and `rename` no file exists at the project path; a crash there leaves only a
  `<name>.tmp0.ove` file. There is no fsync.
- **Memory addresses as ids.** `serializer230220.cpp` writes `ptr` =
  `reinterpret_cast<quintptr>(node)` and, when pasting, casts the number back to a pointer and
  checks `project->nodes().contains(existing)`. Ids change every session, and an address freed
  and reused by another node can match by accident.
- **Autorecovery.** `app/core.cpp::SaveAutorecovery` writes timestamped files under
  `autorecovery/<project UUID>/`, keeps an index of unrecovered projects and a `realname.txt`,
  and caps the count. Keying by project UUID survives renames. `SaveProjectInternal` runs on the
  main thread; its comment acknowledges "a brief (but often unnoticeable) pause in the GUI".
- **Moved projects.** `Core::ValidateFootageInLoadedProject` stores absolute footage paths plus
  the project's saved URL; when footage is missing and the project moved, it re-derives a
  relative path from the old project folder and applies it to the new one. A
  `StripWindowsDriveLetter` "HACK" removes `C:` on non-Windows hosts. `QFileInfo::exists` runs
  on the UI thread for every footage item.
- **Exact time.** Keyframe times are written as rational strings (`rational::fromString` in
  `app/node/keyframe.cpp`).
- **OTIO import/export.** `app/task/project/loadotio/loadotio.cpp` dereferences
  `source_range()->` without checking that the optional is set (a valid OTIO clip may have a
  null `source_range`), converts through `rational::fromDouble(...to_seconds())`, ignores
  `available_range.start_time`, uses `target_url` directly as a file name, and turns unknown
  item types (including nested stacks) into gaps. `saveotio.cpp` writes clips at the sequence
  rate but gaps through `toRationalTime()` without a rate.

### libopenshot (LGPL-3.0-or-later)

- **Float seconds.** `src/ClipBase.cpp` serializes `position`, `start`, `end` as doubles in
  seconds; frame alignment depends on later rounding.
- **Enums as integers.** `src/Clip.cpp::SetJsonValue` and `src/Point.cpp` cast
  `root["gravity"].asInt()` etc. straight into C++ enums, so reordering an enum or reading an
  out-of-range value corrupts meaning silently.
- **Default-by-absence.** `Clip::SetJsonValue` maps a missing `reader_orientation_mode` to
  `LegacyClipTransform` ("Older project files predate reader-applied orientation metadata"),
  i.e. the deserializer, not a migration, encodes the legacy semantics.
- **Regex path rewriting.** The `Timeline(path, convert_absolute_paths)` constructor in
  `src/Timeline.cpp` finds paths with the regex `"(image|path)":.*?"(.*?)"` over the raw JSON
  text and splices canonical absolute paths back in, with symbolic roots `@assets` and
  `@transitions`. Escaped quotes break the match, and the spliced path is not JSON-escaped (a
  Windows path with `\` produces invalid escapes).
- **Ids.** `src/IdGenerator.h` generates random 8-character `[A-Z0-9]` ids with no collision
  check. Edits travel as JSON diffs keyed by paths (`Timeline::ApplyJsonDiff`).
- Project-file versioning for OpenShot lives in the Python front end, not in this library, so
  it is not covered here.

### Comparison

| | OTIO | Kdenlive | Shotcut / MLT | Olive | libopenshot | **FreeCut (proposed)** |
|---|---|---|---|---|---|---|
| Container | JSON | MLT XML | MLT XML | XML (optionally zlib) | JSON | JSON (UTF-8, canonical) |
| Version | per object `Name.N` | double + patch int | MLT version + app title | `YYMMDD` int | in front end | integer `format_version` + `write_compat` |
| Upgrade | per-type fn chain on dicts | monolithic DOM mutation | load-time XML rewrite | one loader per version | field presence | pure `Value → Value` steps, one per version |
| Newer files | refuse newer known schema | refuse | refuse MLT > 7 | refuse | n/a | read-only (or read-write if `write_compat` allows) |
| Unknown data | kept (`UnknownSchema`, dynamic fields) | unknown effects **removed** | strings kept | dropped | ignored | kept per entity, written back |
| Time | f64 value + f64 rate | int frames, clock strings | int frames, clock strings | rational strings | f64 seconds | i64 ticks |
| Ids | none / ref ids | int + UUID | none | memory addresses | random strings | document-wide `u64` allocator + project UUID |
| Atomic save | n/a (library) | QSaveFile | QSaveFile (direct fallback) | remove + rename | n/a | temp + fsync + rename + dir fsync, no fallback without backup |
| Autosave | n/a | in-place rewrite | worker on live model | UI thread | n/a | Arc snapshot, worker, atomic |
| Recovery key | n/a | md5(file name) | md5(full path) | project UUID | n/a | project UUID + revision |
| Relative paths | URL strings | `root` string prefix | `root` string prefix | derived at load | regex on text | typed `StoredPath {abs, rel}` |

---

## Recommended FreeCut design

### Design principles (what the references taught)

1. **Typed tree, never text surgery.** All path, version and unit handling happens on parsed
   values (`serde_json::Value` during migration, typed DTOs afterwards). No regex or string
   prefix rewriting of serialized output (MLT `root`, libopenshot regex, Shotcut `filterXML`,
   Kdenlive decimal repair).
2. **Integers for versions and time; locale-free numbers.** `format_version` is a `u32`;
   time is ticks; JSON numbers via `serde_json`/`ryu` never depend on locale (Kdenlive's
   generation-4 break, MLT `LC_NUMERIC`).
3. **One meaning per field.** Legacy semantics are written explicitly by migrations, not
   inferred from a missing key in the deserializer (libopenshot `reader_orientation_mode`,
   Kdenlive `rotate_anchor` patch).
4. **Exactly-once, pure, tested migrations from every version** (OTIO inclusive-bound issue,
   Olive's parallel loaders, Kdenlive's monolith).
5. **Never destroy data the build doesn't understand** (Kdenlive removes missing effects; OTIO
   keeps unknowns). Unknown fields and kinds survive a load/save cycle.
6. **Serialize a snapshot, not the live model; never on the UI thread** (Shotcut worker on live
   model, Olive and Kdenlive on the UI thread).
7. **Crash-safe replacement:** write temp in the same directory, fsync, rename over, fsync the
   directory, keep the old file until the new one is durable (Olive's remove-then-rename,
   Kdenlive's in-place autosave).
8. **Stable, persistent, non-reused ids** (Olive pointers, libopenshot random strings).
9. **Identity of recovery and backups by project UUID + revision, not path or mtime** (Shotcut
   and Kdenlive path hashes and mtime comparisons).
10. **Interchange through one intermediate representation**, exact rational conversion and a
    report of everything that was approximated or skipped.

### Module layout (`crates/freecut-project`)

```
src/
  lib.rs              pub API: open, save, read_header, Opened, ProjectSnapshot, errors
  format/
    mod.rs            FORMAT_MAGIC, CURRENT_FORMAT_VERSION, WRITE_COMPAT, OLDEST_READABLE
    dto.rs            serde DTOs for the current version only (#[non_exhaustive] not needed: private)
    map.rs            DTO <-> freecut_timeline::Document mapping
    extra.rs          Extra (unknown-field map), UnknownFields side table, EntityKey
    canonical.rs      canonical writer (formatter, finite-float and tick-range guards)
    header.rs         fast prefix reader for ProjectHeader
  migrate/
    mod.rs            registry + runner (pure)
    v1_to_v2.rs ...   one file per step, each with fixtures
    quirks.rs         writer-app-version quirk fixes (pure)
  validate.rs         load-time validation and repair (pure) -> RepairNote list
  paths/
    repr.rs           PathRepr/StoredPath <-> JSON and <-> native PathBuf
    relative.rs       pure relative-path computation (style- and volume-aware)
    resolve.rs        resolver over the FileSystem trait
  io/
    atomic.rs         atomic_replace(), FileStamp, conflict check
    fs.rs             FileSystem trait, RealFs; MemFs behind cfg(test) / feature "test-util"
  lock.rs             acquire / inspect / refresh / release
  backup.rs           snapshot, list, prune (pure policy + thin I/O)
  recovery.rs         write / find / assess / discard recovery snapshots
  view.rs             ViewState sidecar types and I/O (app data)
  package.rs          PackagePlan::build (pure) + executor steps
  interop/
    ir.rs             InterchangeTimeline (format-neutral cut model) + InterchangeReport
    to_doc.rs         IR -> ImportPlan (pure); from_doc.rs: Document -> IR (pure)
    otio/             read.rs, write.rs, json_lenient.rs, schema.rs, time.rs, url.rs
    fcpxml/ edl/      later
tests/                golden/, fixtures/format/vN/, fixtures/otio/, crash_harness.rs
```

Dependencies: `freecut-project → freecut-timeline` only (no `freecut-media`, no GPU, no egui).
Fingerprints and probe-based relink validation stay in `freecut-media` (PM-038, PM-091); the
project crate only stores and compares the values. All public functions are synchronous,
blocking and `Send`; the app calls them from worker threads. No global state, no
current-directory changes (Kdenlive's `QDir::setCurrent`), no `unsafe`.

### On-disk layout and envelope

```json
{
  "format": "freecut-project",
  "format_version": 1,
  "write_compat": 1,
  "app_version": "0.1.0",
  "header": {
    "id": "6f1c0a9e-3b5d-4c1e-9a77-2f0d5c1b8e42",
    "name": "My Trip",
    "created": "2026-10-11T14:32:05.120Z",
    "modified": "2026-10-11T15:02:44.871Z",
    "revision": 37,
    "saved_dir": { "abs": "/Users/me/Movies/FreeCut", "style": "posix" },
    "duration": 42336000000,
    "canvas": { "width": 1080, "height": 1920 },
    "frame_rate": { "num": 30, "den": 1 }
  },
  "settings": { "...": "PM-003..PM-010 values" },
  "ids": { "next": 58 },
  "library": {
    "bins": { "...": "..." },
    "items": {
      "12": {
        "name": "IMG_0412.MOV",
        "source": {
          "kind": "file",
          "path": { "abs": "/Users/me/Footage/IMG_0412.MOV", "style": "posix",
                    "rel": "../../Footage/IMG_0412.MOV" }
        },
        "fingerprint": { "size": 734003200, "head_tail": "9f3a1c0e5b7d2a41",
                         "duration": 42336000000 },
        "storage": "linked",
        "origin": { "kind": "imported" },
        "info": { "...": "probe data (PM-035), ticks only" }
      }
    }
  },
  "sequences": {
    "root": 3,
    "items": {
      "3": {
        "flags": { "main_magnet": true, "...": "..." },
        "master": { "gain": -600 },
        "tracks": [
          { "id": 4, "kind": "video", "main": true, "name": null,
            "flags": { "muted": false, "hidden": false, "locked": false },
            "clips": [
              { "id": 20, "start": 0, "duration": 42336000000, "enabled": true,
                "content": { "kind": "media", "media": 12, "streams": { "video": 0, "audio": 0 },
                             "source": { "start": 0, "duration": 42336000000 } } }
            ] }
        ]
      }
    }
  },
  "export": null,
  "cover": null
}
```

- **Key order** is the DTO field order: envelope fields, then `header` (so `read_header` finds
  it in the first few KiB), then the bulky sections. Maps keyed by id are `BTreeMap<u64, _>`,
  serialized as decimal string keys in numeric order. Arrays have a defined order (clips by
  start then id; keyframes by time).
- **Nested sequences are flat** (`sequences.items`), referenced by id from compound clips
  (TL-040). JSON depth stays bounded regardless of nesting, well under `serde_json`'s
  default recursion limit of 128.
- **Bytes:** UTF-8 without BOM on write (a BOM is accepted on read), `\n` line endings on every
  OS, 2-space indent, trailing newline.
- **Where things live**

  | Data | Location |
  |---|---|
  | Project document | `<Name>.freecut` |
  | Lock metadata | `<Name>.freecut.lock` (next to the project) |
  | Local liveness lock | app data `locks/<xxh3(canonical project path)>.lck` (OS lock held) |
  | Pre-migration copy | `<Name>.freecut.v<N>.bak` (once) |
  | Pre-repair copy | `<Name>.freecut.bak-YYYYMMDD` (once) |
  | Recovery snapshot | app data `recovery/<project-id>.freecut` |
  | Backups | app data `backups/<project-id>/<YYYYMMDDTHHMMSSZ>.freecut` |
  | View state | app data `view/<project-id>.json` |
  | Covers | app data `covers/<project-id>.webp` |

### Value encoding rules

| Value | JSON form | Rule |
|---|---|---|
| `Tick` | integer | Must satisfy `|t| ≤ 2^53` (≈148 days at 705.6 M ticks/s) so any JSON tool reads it exactly; larger values are a validation error. |
| `FrameRate`, speeds | `{ "num": u32, "den": u32 }` | Reduced form; rates must pass TL-001's tick-exact check. |
| `TimeRange` | `{ "start": t, "duration": d }` | `duration > 0` except where a spec allows 0 (markers). |
| ids | integer (`u64`, `< 2^53`) | Map keys as decimal strings. |
| Hashes, fingerprints | lowercase hex string | Never JSON numbers (64-bit hashes exceed 2^53). |
| Project id | UUID string (hyphenated, lowercase) | |
| Timestamps | RFC 3339 UTC, millisecond precision, `Z` | Fixed width for deterministic output. |
| Enums | kebab-case string | `#[serde(rename_all = "kebab-case")]`; never integer discriminants. |
| Effect / transition kinds | namespaced string (`"freecut.dissolve"`) | Unknown kinds kept verbatim (FX-002, FX-012). |
| Floats (transform, easing) | JSON number via `ryu` | Must be finite; `serde_json` would silently write `NaN`/`inf` as `null`, so the writer checks first and fails with `SaveError::NonFinite { path }`. Parsed with `serde_json`'s `float_roundtrip` feature so save → load → save is bit-identical. |
| Colors | `"#RRGGBBAA"` uppercase | |
| Paths | `StoredPath` (see [Media paths](#media-paths)) | |
| Optional sections | omitted when default | `skip_serializing_if` + `#[serde(default)]` only where "absent" and "default" mean the same thing in every version (principle 3). |

Enforcement: a `clippy.toml` `disallowed-types` entry for `std::collections::HashMap` /
`HashSet` (DTOs must use `BTreeMap`/`BTreeSet`/sorted `Vec`), and the golden and property tests
described below.

### Stable ids

- **Project id:** UUID v4, created at New, regenerated by Save a Copy, Duplicate, Package,
  "Open backup as copy" and when PM-001 detects a second file with the same id.
- **Entity ids** (`MediaId`, `BinId`, `SeqId`, `TrackId`, `ClipId`, `MarkerId`, `GroupId`,
  `EffectId`, …): typed `u64` newtypes allocated by **one** document-wide monotonic counter,
  persisted as `ids.next`. One counter makes every id unique across kinds and sequences, so
  compound-clip create/break-apart and cross-sequence moves never collide, and the
  unknown-field side table can key by id alone.
- **Never reused:** undo restores old ids, redo restores the same new ids (the allocator is
  only advanced by planning a new op, never rewound), so histories, view-state keys and cache
  keys stay valid (TL-006).
- **On load:** if `ids.next ≤ max(existing id)`, it is bumped to `max + 1` (repair note).
  Duplicate ids are a validation problem: the later occurrence gets a fresh id and references
  that can be attributed unambiguously are rewritten; otherwise the project opens read-only
  with a report.
- **Merging documents** (cross-project paste TL-032, OTIO import, template instantiation
  OV-031): every incoming id is remapped through `IdRemap` before insertion. Ids are never
  memory addresses (Olive) or unchecked random strings (libopenshot).

```rust
// freecut-timeline (pure). Sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ClipId(u64);

#[derive(Clone, Debug)]
pub struct IdAllocator { next: u64 }

impl IdAllocator {
    pub const MAX: u64 = 1 << 53; // JSON-safe bound
    pub fn alloc<T: From<u64>>(&mut self) -> Result<T, EditError> {
        if self.next >= Self::MAX {
            return Err(EditError::IdSpaceExhausted);
        }
        let id = self.next;
        self.next += 1;
        Ok(T::from(id))
    }
}
```

### Schema types, mapping and unknown data

`freecut-project::format::dto` holds serde structs for the **current version only**; they are
private to the crate. `format::map` converts between DTOs and `freecut_timeline::Document`
(the in-memory model, which stays free of serde attributes and file concerns). The extra layer
costs some code, but it means a model refactor cannot silently change the file format, and the
golden/schema tests below catch any DTO change.

**Unknown fields.** Every DTO for an "extensible record" (envelope, settings, media item, bin,
sequence, track, clip, effect instance, transition, marker, keyframe track) ends with:

```rust
/// Keys this build does not know, preserved verbatim and written back.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Extra(BTreeMap<String, serde_json::Value>);

#[derive(Serialize, Deserialize)]
struct TrackDto {
    id: u64,
    kind: TrackKindDto,
    #[serde(default)]
    main: bool,
    name: Option<String>,
    flags: TrackFlagsDto,
    clips: Vec<ClipDto>,
    #[serde(flatten)]
    extra: Extra,
}
```

On load, the mapper moves each non-empty `extra` into a side table returned next to the model:

```rust
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityKey { Root, Settings, Media(MediaId), Bin(BinId), Sequence(SeqId),
                     Track(TrackId), Clip(ClipId), Effect(ClipId, EffectId), Marker(MarkerId) }

#[derive(Clone, Debug, Default)]
pub struct UnknownFields(BTreeMap<EntityKey, Extra>);
```

On save, extras are merged back for entities that still exist; entities that were deleted take
their unknown fields with them, and new entities (split results, duplicates) start without
any. This keeps `freecut-timeline` and the undo stack unaware of foreign data, and the table is
immutable after load (an `Arc` in the snapshot). A warning is logged once per open listing the
JSON paths of unknown keys.

`#[serde(flatten)]` caveats, accepted deliberately: it routes the struct through serde's
internal buffering (slower; fine for project sizes), it cannot be combined with
`deny_unknown_fields` (not wanted here), and it is not compatible with `serde_json`'s
`arbitrary_precision` or `RawValue` features (neither is enabled).

**Unknown kinds.** Effect and transition kinds are data (strings) in the model already, so an
unknown kind is simply carried along (FX-002, FX-012). For tagged enums that the model must
understand (clip content, track kind), an unknown `"kind"` is replaced during a pre-pass by
`{"kind": "unavailable", "original": {…}}`, mapped to a model placeholder
(`ClipContent::Unavailable { kind }`) that keeps its timeline range, renders an
"unavailable item" slate and is written back with its original object. Such a document opens
**read-only** unless `write_compat` allows editing (next section), because edit semantics for
an unknown item are undefined.

### Versioning and migrations

```rust
pub const FORMAT_MAGIC: &str = "freecut-project";
pub const CURRENT_FORMAT_VERSION: u32 = 1;
/// Oldest format version whose builds may safely edit and save files written at
/// CURRENT_FORMAT_VERSION. Equal to CURRENT unless a bump is purely additive and independent.
pub const WRITE_COMPAT: u32 = 1;
pub const OLDEST_READABLE: u32 = 1;

pub type MigrateFn = fn(serde_json::Value) -> Result<serde_json::Value, MigrationError>;

/// MIGRATIONS[i] upgrades version i + 1 to i + 2.
const MIGRATIONS: &[MigrateFn] = &[ /* v1_to_v2::migrate, … */ ];
const _: () = assert!(MIGRATIONS.len() + 1 == CURRENT_FORMAT_VERSION as usize);

pub fn migrate_to_current(mut doc: Value, from: u32) -> Result<Migrated, MigrationError> {
    if from < OLDEST_READABLE {
        return Err(MigrationError::TooOld { found: from, oldest: OLDEST_READABLE });
    }
    let mut v = from;
    while v < CURRENT_FORMAT_VERSION {
        let step = MIGRATIONS
            .get((v - 1) as usize)
            .ok_or(MigrationError::MissingStep { from: v })?;
        doc = step(doc).map_err(|e| e.at_step(v))?;
        v += 1;
        set_format_version(&mut doc, v)?; // the runner, not the step, owns the version field
    }
    Ok(Migrated { doc, from, to: v })
}
```

Rules:

1. **One step per version, applied exactly once** (`from == v`), so steps need not be
   idempotent and never see an already-upgraded document (the OTIO pitfall).
2. **Steps are pure** functions of the JSON value: no clock, no file system, no access to the
   render or media engines (Kdenlive's MLT-dependent fixes made migrations
   environment-dependent). If a step needs a constant table (an effect rename map), the table
   is frozen inside that step's module.
3. **Semantic default changes are migrations.** When a later version changes what an absent or
   default value means (e.g. a rotation anchor default), the step writes the old meaning
   explicitly into every affected object. The DTO field then has a single meaning.
4. **Renames of effect/transition kinds** are migration steps that rewrite the kind string
   (Kdenlive needed a runtime `renamedEffects` table and still dropped what it couldn't map).
5. **`write_compat` (refines PM-021 V4).** Opening a file with `format_version >
   CURRENT_FORMAT_VERSION`:
   - if `write_compat ≤ CURRENT_FORMAT_VERSION`: open read-write with a banner ("made with a
     newer FreeCut; some newer features are preserved but not shown"); unknown keys are
     preserved through `Extra`;
   - otherwise: open **read-only** (PM-021) if the typed parse succeeds, else fail with
     `OpenError::TooNew`.
   Each version bump sets `WRITE_COMPAT` deliberately; the default is "equal to the new
   version" (conservative). The written file always records the *writer's* `WRITE_COMPAT`.
6. **Writer quirks.** `migrate::quirks` holds fixes keyed by the writer's `app_version` (a
   `semver::VersionReq` → pure fix), applied before migrations. It is empty at launch; it
   exists so that a buggy release can be repaired later without a format bump (Shotcut's
   `fixVersion1701WindowsPathBug`).
7. **Fixtures are frozen at bump time.** When version N is superseded, the test suite saves a
   set of documents produced by the N writer into `tests/fixtures/format/vN/` (see test
   strategy). Fixtures are never regenerated afterwards.
8. **No downgrade writer in the mvp.** Saving for an older FreeCut is not offered; a later
   "Save a copy for FreeCut x.y" would add per-step `downgrade` functions like OTIO's.

**Implementation note (2026-10-12): format versions so far.** 1: the first format. 2: clip
sound and media sources (`v1_to_v2`). 3 (`v2_to_v3`): text clips get their own content,
`{"type": "text", "text", "style": {"font": {"family"}, "size", "fill": "#RRGGBBAA", "bold",
"italic"}, "layout": {"align", "box_width", "line_spacing"}, "origin"}`, with lengths in
1/100 reference px and ratios per-mille; a version-2 `{"type": "generated", "kind": "text"}`
becomes the factory default ("Your text", frozen in the step). Clips gain `transform`
(`position {x, y}` in millionths of the canvas, `scale` in basis points, `rotation` in
1/100 degree, `flip_h`, `flip_v`, `opacity` per-mille), left out while it is the identity,
which is what a missing key means in every version. The envelope gains `export` (the last
export settings, EX-021) after `document`, `null` until the first export. Out-of-range
transform and text values in a hand-edited file are clamped on open and reported as a repair
(`OpenWarning::ValuesClamped`), the way an edit clamps them. FreeCut default: this covers any
integer that fits 64 bits, including ones beyond the model's own width (a negative `opacity`
or `line_spacing`, a `scale` beyond `i32`), which are saturated first; fractions, strings and
numbers beyond 64 bits are refused as a damaged file with their JSON path.

**Implementation note (2026-10-12): format 4** (`v3_to_v4`, the v1 sound features). An
`"active"` clip sound gains `gain_keys` (volume keyframes, AU-008: `[{"at": ticks of source
time, "gain": hundredths or "-inf", "interp": "linear" | "hold"}]`, strictly increasing; `gain`
stays the static volume) and `fade_in_curve` / `fade_out_curve` (`"linear"`, `"smooth"`,
`"fast"`, `"slow"`, AU-006). Media clip content gains `stream` (AU-038): `null` for the
media's default stream, `{"type": "index", "index": n}` or `{"type": "all-mixed"}`. Each
audio stream of a media item gains `default`, `language`, `title` and `layout` from the probe.
The step writes the format-3 meaning into old files: `[]`, `"linear"`, `null`, `false`.
Tracks gain `mix` (AU-013: `{"gain", "pan"}`, pan in percent −100…100) and the envelope gains
`ui` after `export` (`{"solo": [track ids]}`, AU-012); both are left out while at their
defaults, which is what a missing key means in every version. Hand-edited values: keys out of
time order are sorted (`OpenWarning::KeyframesSorted`, a repair), two keys at one time are
refused, gains, track gains above +12 dB, pans outside −100…100 and a non-default `mix` on a
track without sound are refused with their JSON path (like clip gains always were). Unknown
keys on a keyframe are kept by the keyframe's time (`audio.gain_keys@<ticks>` in `Extras`), so
they stay with their keyframe when an earlier one is added or removed, and go when it is moved
or deleted. Load-time repairs that rebuild a track (dissolving lonely groups and links,
clearing stray links) keep its `mix`. A `stream`
index the media lacks is kept, as after a relink, and reported without a repair
(`OpenWarning::MissingStreams`). **View state in the file (refines F6):** AU-012 says the solo
set is "kept in the project's UI state" and survives reopening, so it is the first UI-state slot
in the project file (`freecut_project::UiState`, `Opened::ui`, `SaveRequest::ui`). It is not
part of the `Document` and never undoable; open and save keep only ids of sound-carrying tracks
of the document, so a removed track's solo is cleared. Whether a solo change alone marks the
project unsaved is the app's call; the next save writes whatever the set holds.

**Implementation note (2026-10-12): format 5** (`v4_to_v5`, text round 2: TX-005, TX-008 to
TX-016, TX-054, TX-055). On `"type": "text"` content, `style.fill` becomes a tagged object:
`{"type": "solid", "color"}`, `{"type": "linear-gradient", "angle", "stops", "extent"}` or
`{"type": "radial-gradient", "center": {"x", "y"}, "spread", "stops", "extent"}`, with angles in
tenths of a degree (`0..3600`), centre and spread per-mille, `extent` `"block"` or `"line"`,
and `stops` a list of 2–8 `{"at": per-mille, "color": "#RRGGBBAA"}`. The step wraps a version-4
colour string as a solid fill. `style` gains `underline`, `strike`, `case` (`"as-typed"`,
`"upper"`, `"lower"`, `"title"`, `"sentence"`) and `letter_spacing` (per-mille of the size);
`layout` gains `justify` and `v_anchor` (`"top"`, `"middle"`, `"bottom"`). The step writes the
version-4 meaning (`false`, `false`, `"as-typed"`, `0`, `false`, `"middle"`), which is also what
a missing key reads as. The optional layers `outline`, `shadow`, `glow`, `background`,
`bubble` and `curve` are objects with `"on"` and all their settings (a box's or bubble's
`border` and a bubble's `tail` are nested layers with their own `"on"`; the tail tip is in
hundredths of a reference px from the text box centre, other lengths per-mille of the font
size). A layer is left out while it is the factory default (off, default settings), which
is what a missing key means and what every version-4 text had, so the step writes nothing for
them; a layer switched off with changed settings, or one carrying unknown keys, is written
with `"on": false` (TX-054). Hand-edited values: numbers are clamped into their ranges (beyond
64 bits they are refused), angles wrapped, stops past 100 % clamped and out-of-order stops
sorted, and a clip with both `background` and `bubble` on opens with the bubble (TX-054's
edge case); each is a repair (`OpenWarning::ValuesClamped`). A colour that is not
`#RRGGBBAA`, a fill or bubble kind that does not exist and a gradient without 2 to 8 stops are
refused with their JSON path. Unknown keys are kept on every format-5 object, including
`fill.center`, the borders, the tail and the curve. Gradient stops have no identity of their
own, so a stop's unknown keys travel with it through the sort and are filed under its index in
the sorted list, the order the writer uses: opening and saving a file whose stops were out of
order keeps each stop's keys on the same colour.

**Implementation note (2026-10-13): format 6** (`v5_to_v6`, S2 keyframes and animations:
TR-027, TR-031, TX-021, FX-018, FX-019, TX-022). Every clip may carry `keys`, after
`transform`: `{"origin": ticks, "tracks": [{"prop": "transform.position", "keys": [{"at":
ticks, "value": …, "interp": "ease-in-out-cubic"}]}]}`. `prop` is the property's stable path
(`transform.position|scale|rotation|opacity`, the animatable `text.*` paths,
`text.fill.stops.N.color`, `text.reveal`); `at` is content time (source time for media, clip
time plus `origin` otherwise); `value` is spelled like the static value it animates: an
integer in the same fixed-point unit (scale in basis points, rotation in 1/100°, ratios
per-mille, lengths in 1/100 reference px, fill and shadow angles in 1/10°), a `"#RRGGBBAA"`
colour, or `{"x", "y"}` for positions (millionths of the canvas) and tail tips (1/100 reference
px). Tracks are written in the model's property order. `keys` is left out while a clip has no
keys and origin 0; an origin without keys (the right piece of a split text) is written, so the
document round-trips exactly, and reads the same as 0 for everything a user sees. Every clip
may also carry `animations`, after `keys`: `{"entrance", "exit", "continuous"}`, each `null` or
`{"preset", "duration", "easing", "direction", "unit", "overlap", "order", "seed", "pace",
"intensity", "travel", "phase"}` (ticks for times, per-mille for overlap and intensity,
kebab-case names, `null` for "the preset's default"), left out while every slot is empty. A slot
may also carry `"caret": true` (Typewriter's Caret, TX-056), written only when on; absent reads as
off, so format 6 files without it need no migration, and an older build keeps the key as an
unknown key of the slot. A Loop slot of a split clip also carries `"seams": {"start", "end",
"ramp"}` (ticks of loop time, `null` for a missing seam; TX-022), written only after a split
and read as none when absent, likewise without a migration. A
preset id this build has no manifest for is kept as chosen and plays as no animation (FX-018).
`interp` names, shared by volume keys, prop keys, easing overrides and preset manifests:
`linear`, `hold`, `ease-<in|out|in-out>-<sine|quad|cubic|expo>`, `overshoot-out`,
`overshoot-in`, `elastic-out`, `bounce-out`; volume keys, which format 5 limited to `linear`
and `hold`, take all of them. Both sections are new and absent means the format-5 state, so
the step writes nothing. Hand-edited values: key values are clamped as an edit would clamp
them (angles wrapped), overlap and intensity capped at 1000, Pace held within 0.02–1 s, a
`hold` easing override dropped, an In or Out raised to 0.1 s, a Loop's cycle held within
0.1–10 s, a seed saturated into 0–4294967295 and a seam ramp held within 0–250 ms
(`OpenWarning::ValuesClamped`); keys out of time order are
sorted (`OpenWarning::KeyframesSorted`). Refused with their JSON path: a property that takes no
keys or that the clip has no place for (text keys on a video clip, any keys on an audio clip),
a property listed twice or with no keys, two keys at one time, a value of the wrong shape,
animations on an audio clip, a Loop beside In or Out on a video, image or generated clip
(FX-019's exclusive policy), a length that isn't positive and an empty preset id. Unknown keys:
on `keys` and `animations` and each slot by path, on a key track by its property
(`keys[text.size]`), on a key by property and time (`keys[text.size]@<ticks>`, and
`…@<ticks>.value` for a point value), so they follow the key like a volume key's. A slot's
unknown keys stay with the slot when another preset is chosen in it.

**Colour stacks (format 10, CL-001, slice K1).** Video, image and adjustment clips may carry
`color`, after `attach`: `{"bypass": true, "amount": 0–100, "off": ["light", …], "light":
{"exposure", "brightness", "contrast", "contrast_pivot", "highlights", "shadows", "whites",
"blacks", "tone_lift"}, "color": {"temperature", "tint", "hue", "saturation", "vibrance"}}`.
Every part at its default is left out (bypass off, Amount 100, every section on, a slider at
0), and `color` itself is left out for the default stack, which is what a missing key means in
every version; a stack that reads back to the defaults is stored as none. `off` lists switched-off
sections by CL-043's names in display order. Sliders are integers −100..=100 and Amount
0..=100; numbers beyond are clamped (`OpenWarning::ValuesClamped`). Refused with their path: a
`color` on a clip that can't carry one (text, sound, stickers, effects) and an unknown section
name. Colour keyframes are key tracks named `color.<slider>` (`color.exposure`, `color.amount`)
with integer values, clamped like the static ones, and only on clips that take a stack. Unknown
keys are kept on `color`, `color.light` and `color.color` by path. Step 9 → 10 writes nothing.

**HSL, Curves and Wheels (format 11, CL-013 to CL-015, slice K2).** `color` gains `"hsl":
{"red": {"hue", "saturation", "lightness"}, …}` (the eight bands red, orange, yellow, green,
cyan, blue, purple, magenta; each level −100..=100), `"curves": {"luma", "red", "green",
"blue"}`, each a list of `[x, y]` points in 0..=10000 left to right with the endpoints
included, and `"wheels": {"strength": 0–100, "shadows" | "midtones" | "highlights" |
"offset": {"x", "y", "luma"}}` (pucks on the disk x² + y² ≤ 1000², luminance −100..=100).
Every part at its default is left out: a band at 0, a curve that is exactly the diagonal
`[[0, 0], [10000, 10000]]`, a wheel with its puck centred and its luminance 0, Strength 100.
Repairs counted as `ValuesClamped`: levels beyond their range, curve coordinates outside
0..=10000, and pucks off the disk (pulled onto the rim along their direction). A list of points
that breaks CL-014's rules (fewer than 2 or more than 16, x not at least 100 apart, the left
endpoint past 2500 or the right one before 7500) is refused at its path
(`….color.curves.red`). Keyframes: `color.hsl.<band>.<channel>`, `color.wheels.<wheel>.luma`,
`color.wheels.strength` (integers) and `color.wheels.<wheel>.puck` with `{"x", "y"}` values,
pulled onto the disk on load; curves take no keys. Unknown keys are kept on `color.hsl`, each
band, `color.curves`, `color.wheels` and each wheel by path. Step 10 → 11 writes nothing.

**The LUT slot and skin protection (format 12, CL-017, CL-030, slice K3).** `color` gains
`"lut": {"hash": "<32 hex digits>", "file": "<original name>", "intensity": 0–100, "interp":
"trilinear", "skin": {"on": true, "amount": 0–100}}` and `color.color` gains `"skin": {"on":
true, "amount": 0–100}`. The hash is the XXH3-128 of the LUT file's bytes (big-endian, so its
hex reads as the 128-bit number); the file itself lives in the user's LUT library or FreeCut's
bundled set, never in the project. Left out at the defaults: no LUT, Intensity 100,
`tetrahedral`, skin protection off at Amount 70. Repairs counted as `ValuesClamped`: Intensity
and skin Amounts beyond 0..=100. Refused at their path: a `hash` that isn't 32 hex digits
(`….color.lut.hash`) and an unknown `interp`. Keyframes: `color.lut.intensity`,
`color.skin_amount` (the Color scope) and `color.lut.skin_amount`, integers clamped like the
static values. Unknown keys are kept on `color.lut`, `color.lut.skin` and `color.color.skin` by
path. Step 11 → 12 writes nothing.

**Auto and Match (format 13, CL-019, CL-020, slice K5).** `color` gains `"auto": {"offsets":
{"exposure", "contrast", "whites", "blacks", "temperature", "tint", "vibrance",
"saturation"}, "style": "vivid", "source": "playhead", "strength": 0–100}` (each offset
−100..=100, left out at 0; Natural, the whole clip and Strength 100 left out) and `"match":
{"gain": [y, cb, cr], "offset": [y, cb, cr], "intensity": 0–100, "reference": {"clip": <id>,
"at": <ticks>, "ungraded": true} | {"media": <id>, "at": <ticks>}}`. Gains (5000..=20000) and
offsets (−10000..=10000) are ×10⁴ of Rec.709 `Y'`, `Cb` and `Cr` of code values (CL-001 ›
*K5 as built*) and are always written; Intensity 100 and `ungraded: false` are left out. `at`
is the reference frame's content time (source time for media). A reference may name a clip
that no longer exists; the transform stays. Repairs counted as `ValuesClamped`: offsets,
Strength, Intensity, gains and match offsets beyond their ranges. Refused at their path: an
unknown `style` or `source`, and a `reference` that names neither or both of `clip` and
`media`. The sections' switches are the existing `off` names `auto` and `match`. Keyframes:
`color.auto.strength` and `color.match.intensity`, integers, each only while its result
exists. Unknown keys are kept on `color.auto`, `color.auto.offsets`, `color.match` and
`color.match.reference` by path. Step 12 → 13 writes nothing. Saved looks never hold either.

**Color masks (format 14, CL-031, slice K6).** `color` gains `"masks": [{"id": 1–65535,
"shape": "linear" | "band" | "ellipse" | "rectangle", "center": [x, y], "size": [w, h],
"linked": bool, "rotation": <1/100 °>, "feather": 0–1000, "invert": true, "roundness":
0–1000, "width": 0–2000, "expand": −100..=100, "mode": "add" | "subtract" | "intersect"}]` in
combine order. Positions and sizes are ten-thousandths of the item's shorter side (CL-001 ›
*K6 as built*); every value is left out at its shape's default (centre `[0, 0]`, size
`[5000, 5000]` or `[6000, 4000]` for a rectangle, linked only for an ellipse, no rotation,
feather or roundness, band width 500, not inverted, Expand 0, `add`), so a default mask is
`{"id", "shape"}`. Repairs counted as `ValuesClamped`: centre beyond ±30000, size beyond
0..=40000, rotation beyond ±36000, feather and roundness beyond 1000, band width beyond 2000,
Expand beyond ±100. Refused at their path: more than 8 masks (`….color.masks[8]`), an id that
is 0, beyond 65535 or used twice, an unknown `shape` or `mode`, and `roundness` on a shape
other than a rectangle or `width` on one other than a band. The section's switch is the
existing `off` name `masks`. Keyframes: `color.masks.<id>.center` and `.size` (`{x, y}`),
`.rotation`, `.feather`, `.roundness`, `.band_width` and `.expand` (integers), clamped like the
static values; a key of a mask the stack doesn't hold is kept as stored. Unknown keys are kept on
`color.masks.<index>` by path. Step 13 → 14 writes nothing. Saved looks never hold masks.

**Implementation note (2026-10-14): the LUT library index.** `<app data>/luts/index.json`
(`freecut_project::luts`) is app state, not a project file: `{"version": 1, "luts": [{"hash",
"name", "file", "kind": "cube" | "3dl", "size", "identity"?, "imported", "hidden"?}]}` in import
order, next to one copy per file named `<hash>.<kind>`. A newer `version` or a damaged file is
an error, never an empty library; copies and the index are written atomically.

**Implementation note (2026-10-14): look files (CL-026, slice K4).** `<app data>/looks/<id>.fclook`
(`freecut_project::look`) is app state, not a project file: `{"format": "freecut-look",
"look_version": 1, "name", "tags": [], "created": <unix seconds>, "sections": ["light", …],
"color": {…}, "luts": [{"hash", "file"}]}`. `color` is this format's colour object (the clip
`color` above, defaults left out, read with the same clamping and refusals) holding only the
listed sections; `luts` is written for other readers and not read back. Only Light, Color, LUT,
Filter, HSL, Curves and Wheels are kept. A newer `look_version` is refused with its number and
the file is listed as unavailable, never rewritten; files are written atomically under a
random id, so a display name can be any text. Version 1 is the only one, so there is no
`migrate::Chain` step yet; the first change adds one.

**Implementation note (2026-10-13): preset manifest files.** FX-030's manifests are not
project files and do not use the envelope or `migrate::Chain`: each file carries a `"schema"`
integer (1 today), a file with a newer schema is ignored with a log line, and unknown keys are
refused, so a format change bumps `schema` (see `freecut_project::presets` for the format and
FX-030 for the decision). The bundled files in `resources/animations/` and
`resources/text-animations/` are compiled in with `include_str!` from the `presets::BUNDLED`
table; a test checks that the table lists exactly the `.json` files of both directories.

Other versioned formats (look files, templates, export presets, the export queue, the recent
registry, presets) reuse `migrate::Chain<T>`: the same runner parameterized by magic string,
version constants and step table.

### Open pipeline

`open(path) -> Result<Opened, OpenError>` runs on a worker thread:

1. **Read** the file (size limit 256 MiB, `OpenError::TooLarge` beyond). Empty or all-zero
   content → `Damaged { reason: Empty }` (what a lost write looks like on some file systems).
2. **Decode:** strip a UTF-8 BOM; invalid UTF-8 → `InvalidUtf8 { offset }`.
3. **Parse** to `serde_json::Value`; syntax errors → `Damaged { line, column }` (truncation
   always yields a syntax error because the top level is an object). The UI then offers the
   newest backup (PM-013, PM-016).
4. **Envelope:** `format` must equal `FORMAT_MAGIC`, else `NotAProject` (with a hint when the
   bytes look like OTIO JSON or MLT XML). Read `format_version`, `write_compat`, `app_version`.
5. **Quirks**, then **migrate** (above). `recovery` metadata, if present, is lifted out.
6. **Typed decode** with `serde_path_to_error`, so schema errors name the JSON path
   (`sequences.items.3.tracks[1].clips[7].source.duration: expected i64`).
7. **Map and validate** (`validate.rs`, pure). Load-specific checks and repairs, each recorded
   as a `RepairNote`:
   - frame rate not tick-exact → **refuse** (`OpenError::Invalid`), never guess (TL-001);
   - ticks outside `±2^53`, negative durations, `i64` overflow in derived sums → refuse or
     clamp per TL's checked math;
   - main track missing or not lowest → promote lowest `Video` track or create one (TL-004);
   - compound references: unknown `SeqId` → placeholder; cycles (DFS) or a nested sequence
     referenced twice → deep-copy or break (TL-040), never infinite recursion (Shotcut's
     `checkIncludesSelf`);
   - clips overlapping on one track outside a transition → the later clip moves to a new
     track above (lossless) rather than being trimmed;
   - clip referencing an unknown `MediaId` → **placeholder offline media item** carrying the
     clip's known duration (proposed; PM-013 currently says "drop"), so timing, effects and
     keyframes survive and the user can relink;
   - id allocator and duplicate ids (see [Stable ids](#stable-ids));
   - `header` disagreeing with `settings` → `settings` wins, warning only;
   - *added in review (README C37, §5.11):* effect parameters stored by an older effect
     `version` are migrated here with the pure `freecut_timeline::effects::EffectCatalog::migrate`,
     while mapping DTOs, so nothing has to mutate the document after `Editor::open`.
     Unknown effect ids keep their parameters untouched for the unknown-field round trip.
8. Return `Opened { document, unknown, mode, warnings, provenance }`. If any repair happened,
   the app opens the document dirty with an empty history and the original is copied once to
   `<name>.freecut.bak-YYYYMMDD` before the first overwrite (PM-013).
9. **Afterwards, in the background (app):** lock, path resolution, fingerprint/changed checks,
   thumbnail and peak jobs (PM-090, PM-093, PM-073). Any document change these produce (for
   example an exact-length correction) is an ordinary undoable commit (README §5.11).

```rust
pub struct Opened {
    pub document: freecut_timeline::Document,
    pub unknown: UnknownFields,
    pub mode: OpenMode,                // ReadWrite | ReadOnly(ReadOnlyReason)
    pub warnings: Vec<OpenWarning>,    // Migrated, Repaired, UnknownFields, NewerCompatible, QuirkFixed…
    pub provenance: Provenance,        // stamp, migrated_from, repaired, writer app_version, original bytes hash
    pub recovery: Option<RecoveryInfo>,
}

pub fn read_header(path: &Path) -> Result<ProjectHeader, OpenError>;
```

`read_header` (PM-001 start screen, registry rebuild) reads only the first 64 KiB and drives a
`serde::de::DeserializeSeed` over the top-level object that stores `format`,
`format_version` and `header` into an out-parameter and stops; the "incomplete input" error
that follows is ignored once the header was captured. If the header is not within the prefix
(hand-reordered keys), it falls back to a full parse. The app caches headers in the registry by
`(len, mtime)` so a rescan only touches changed files.

### Save pipeline

```rust
pub struct ProjectSnapshot {
    pub doc: Arc<freecut_timeline::Document>,   // O(1) structural clone taken on the UI thread
    pub unknown: Arc<UnknownFields>,
    pub generation: u64,                        // history position to mark as saved (TL-043)
    pub resolved: Arc<BTreeMap<MediaId, PathBuf>>, // runtime relocations to persist (PM-100)
}

pub enum SaveKind { Save, SaveAs, SaveCopy, Recovery }

pub struct SaveTarget<'a> {
    pub path: &'a Path,
    pub kind: SaveKind,
    pub expected: Option<&'a FileStamp>,   // what we last loaded/saved at this path
    pub provenance: &'a Provenance,        // for the one-time .v<N>.bak / .bak-YYYYMMDD copies
}

pub struct Saved { pub stamp: FileStamp, pub generation: u64, pub revision: u64, pub bytes: Arc<[u8]> }

pub fn save(snap: &ProjectSnapshot, target: &SaveTarget<'_>, clock: &dyn Clock)
    -> Result<Saved, SaveError>;
```

Steps (worker thread; the app's save coordinator keeps one in flight and one queued, PM-012):

1. **Validate before writing.** Run `Document::validate()` on the snapshot. An invalid document
   is a bug: it is **not** written over the project file; it goes to the recovery location
   instead and the error is reported. This replaces Kdenlive's `"<track "` heuristic with a
   real invariant check.
2. **Map and serialize** to canonical bytes: header `modified = clock.now()`,
   `revision += 1`, `saved_dir` = target folder, relative paths recomputed for the target
   folder (Save As), absolute paths refreshed from `resolved` (PM-100). Non-finite floats and
   out-of-range ticks fail here with the JSON path.
3. **Conflict check** (`Save` only): read the current file, compare `(len, xxh3)` with
   `expected`. A different hash → `SaveError::ChangedOnDisk` (Overwrite / Save As / Cancel). An
   mtime-only change with the same hash (a sync tool touching the file) is not a conflict.
4. **One-time copies:** if the document was migrated or repaired on open and this is the first
   overwrite, copy the original bytes to `.v<N>.bak` / `.bak-YYYYMMDD` with `create_new`
   (never overwriting an existing copy; append ` (2)` on collision).
5. **Atomic replace** (below).
6. **Verify:** re-read the file and compare its hash with the buffer. Cheap for project-sized
   files, and it catches broken network shares and sync filters before the user relies on
   the save.
7. **Report** `Saved` to the UI thread, which moves the history's saved position to
   `generation` (if it is still in history) and updates the lock and registry. Backups
   (PM-016) are written from `Saved.bytes`, without re-reading the file.

```rust
pub fn atomic_replace(target: &Path, bytes: &[u8]) -> Result<FileStamp, WriteError> {
    // A symlinked project file is written at its real location; replacing the link
    // itself with a regular file would silently break the user's layout.
    let target = real_target(target)?;
    let dir = target.parent().ok_or(WriteError::NoParent)?;
    let old_perms = std::fs::metadata(&target).ok().map(|m| m.permissions());

    // Same directory => same volume => rename is atomic.
    let mut tmp = tempfile::Builder::new()
        .prefix(&tmp_prefix(&target))      // ".My Trip.freecut."
        .suffix(".tmp")
        .tempfile_in(dir)?;
    tmp.write_all(bytes)?;
    if let Some(p) = old_perms {
        // tempfile creates 0o600 on Unix; keep a shared project group-readable.
        tmp.as_file().set_permissions(p)?;
    }
    tmp.as_file().sync_all()?;             // std uses F_FULLFSYNC on Apple platforms
    persist_with_retry(tmp, &target)?;     // rename over target; retries Windows sharing violations
    sync_dir(dir)?;                        // Unix: fsync the directory entry; Windows: no-op
    FileStamp::from_written(&target, bytes)
}
```

Details and edge cases:

- **Temp naming and cleanup.** The temp file lives next to the target with a dot prefix;
  `tempfile` removes it on every error path (drop). Left-over `.*.freecut.*.tmp` files from a
  power loss are ignored by the project scanner and removed when the same project is next
  saved.
- **Windows.** `std::fs::rename` / `tempfile`'s `persist` replace an existing file
  (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). Antivirus, indexers and sync clients briefly
  open the target and cause sharing violations, so `persist_with_retry` retries 5 times with
  100 ms back-off before failing.
- **No silent non-atomic fallback.** If rename-over fails with an error that indicates the file
  system can't replace atomically (some SMB/NFS setups), the writer copies the old file to
  `<file>.prev` first and then writes the target directly (PM-012), and tells the caller it
  used the fallback. It never does a direct write without that copy (contrast Shotcut's
  `setDirectWriteFallback(true)` and Olive's remove-then-rename).
- **Disk full / permission denied:** the target is untouched, the temp is gone, the error maps
  to `SaveError::Io { path, kind }` with a user-facing reason (PM-012, inventory §4.3: no
  permission, read-only folder, no space, path too long, invalid path, disconnected disk,
  unknown).
- **Long Windows paths:** native paths longer than `MAX_PATH` are passed in verbatim
  (`\\?\`) form; the temp name adds at most ~20 characters, checked up front.
- **Metadata not preserved:** macOS Finder tags, extended attributes and Windows ACLs other
  than the basic permission bits are not carried over to the new inode (documented
  limitation).

```rust
#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error("cannot write {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("cannot serialize the project at {json_path}: {detail}")]
    Serialize { json_path: String, detail: String },
    #[error("value at {json_path} is not a finite number")]
    NonFinite { json_path: String },
    #[error("the file changed on disk since it was opened or last saved")]
    ChangedOnDisk { expected: FileStamp, found: FileStamp },
    #[error("the project is open read-only: {0}")]
    ReadOnly(ReadOnlyReason),
    #[error("refusing to save an invalid document: {0}")]
    InvalidDocument(#[from] freecut_timeline::ValidationError),
}

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("cannot read {path}: {source}")]
    Io { path: PathBuf, #[source] source: std::io::Error },
    #[error("not a FreeCut project")]
    NotAProject { hint: Option<ForeignFormat> },
    #[error("the project file is damaged ({reason})")]
    Damaged { reason: DamageReason },              // Empty, Syntax { line, column }, InvalidUtf8 { offset }
    #[error("made with a newer FreeCut (format {found}; this build reads up to {supported})")]
    TooNew { found: u32, supported: u32 },
    #[error(transparent)]
    Migration(#[from] MigrationError),             // TooOld, MissingStep, StepFailed { from, detail }
    #[error("unexpected data at {json_path}: {detail}")]
    Schema { json_path: String, detail: String },
    #[error(transparent)]
    Invalid(#[from] freecut_timeline::ValidationError),
    #[error("project file is {len} bytes; the limit is {limit}")]
    TooLarge { len: u64, limit: u64 },
}
```

### Single-writer lock

Two pieces, because a lock must answer "is a FreeCut process alive on *this* machine?"
reliably and "is someone on *another* machine editing this file?" approximately:

1. **Lock metadata** `<file>.lock` next to the project, written atomically:
   `{ pid, hostname, user, app_version, session, started, refreshed }` (PM-013), refreshed every
   60 s from a worker.
2. **Local liveness lock:** app data `locks/<xxh3(canonical path)>.lck`, held with
   `std::fs::File::try_lock` (stable since Rust 1.89) for the whole editing session. The OS
   releases it when the process dies, so no pid probing is needed and pid reuse can't produce
   a false "still open". It lives in app data on a local disk because advisory locks on
   network shares are unreliable and because Windows `LockFileEx` would make a locked
   metadata file unreadable to the second instance.

```rust
pub enum LockStatus {
    Free,
    Ours,
    LiveHere(LockInfo),        // same host, liveness lock held by another process
    LiveElsewhere(LockInfo),   // other host, refreshed within 2 min
    Stale(LockInfo),           // same host and liveness lock free, or other host and old
}
pub fn inspect(project: &Path, local: &LocalLockDir, now: Timestamp) -> Result<LockStatus, LockError>;
pub fn acquire(project: &Path, local: &LocalLockDir, mode: Takeover) -> Result<ProjectLock, LockError>;
impl ProjectLock {
    pub fn refresh(&mut self, now: Timestamp) -> Result<(), LockError>;
    pub fn move_to(&mut self, new_project: &Path) -> Result<(), LockError>; // Save As (PM-012)
    pub fn release(self) -> Result<(), LockError>;
}
```

Clock skew between hosts: if `refreshed` lies in the future or within the skew window, the
dialog re-reads the lock after 70 s; a live holder will have changed `refreshed`. Kdenlive's
single global `kdenlivelock` without pid/host (`src/core.cpp`) shows why per-project, per-host
data matters.

### Autosave and crash recovery

The scheduler (PM-014) is a pure state machine in `freecut-app`, tested with a fake clock. Its
contract with this crate is just `save()` (project file) or `save()` with
`SaveKind::Recovery` (recovery snapshot):

```rust
// freecut-app (pure). Inputs and outputs only.
pub enum AutosaveEvent { Committed { generation: u64 }, FocusLost, BeforeExport, Close,
                         SaveFinished { ok: bool, took: Duration }, Tick(Instant) }
pub enum AutosaveAction { None, SaveProject, WriteRecovery }
```

- Only committed history entries trigger saves; an in-progress drag is not in the model yet
  (TL-043 coalescing). Kdenlive's "skip while an operation is in progress" is thereby
  structural rather than a runtime check.
- The snapshot is an `Arc` clone of the immutable document taken on the UI thread; nothing
  touches the live model from the worker (contrast Shotcut's worker reading the live MLT
  model).

**Recovery snapshot** (PM-015): same JSON as a project file plus an envelope field that the
normal loader lifts out and that is never written into project files:

```json
"recovery": { "project_path": { "abs": "...", "style": "posix" },
              "base_revision": 37, "written": "2026-10-11T15:04:10.002Z",
              "session": "b0e7…", "app_version": "0.1.0" }
```

It is written atomically to `recovery/<project-id>.freecut` and deleted on clean close.
Keying by **project UUID** survives renames and moves (Olive) rather than hashing the path
(Shotcut, Kdenlive). The decision on the next launch uses revisions, not mtimes:

| Project file | Snapshot `base_revision` vs file `header.revision` | Offer |
|---|---|---|
| exists, lock stale | equal | **Recover** / Open last saved / Discard |
| exists | file revision greater | snapshot superseded → moved to backups, no prompt |
| exists | file revision smaller | file was replaced by an older copy → offer with a warning |
| missing | – | "Save recovered project as…" |
| snapshot corrupt | – | fall back to the newest backup (PM-016) |

"Discard" moves the snapshot into the backups folder for 7 days (PM-015). Undo history is not
persisted, so a recovered project opens dirty with an empty history.

### Rolling backups

`backup::snapshot(project_id, bytes, now)` writes `backups/<project-id>/<YYYYMMDDTHHMMSSZ>.freecut`
atomically with `create_new` (a collision appends `-2`); names come from the current UTC time,
not from the project's mtime (Kdenlive and Shotcut both name backups by mtime, which collides
and can silently replace a previous backup).

```rust
pub struct Policy {
    pub keep_newest: usize,        // 20
    pub daily_days: u32,           // 14: newest snapshot of each local calendar day
    pub max_total_bytes: u64,      // 500 MB across all projects
    pub always_keep_per_project: usize, // 3
    pub trashed_project_days: u32, // 30
    pub utc_offset: time::UtcOffset,   // "day" is the user's local day; passed in, not read
}
pub struct BackupEntry { pub project: Uuid, pub taken: OffsetDateTime, pub len: u64, pub id: BackupId }

/// Pure: returns the entries to delete. Never returns any of the newest
/// `always_keep_per_project` entries of a project.
pub fn prune(policy: &Policy, now: OffsetDateTime, entries: &[BackupEntry]) -> Vec<BackupId>;
```

### Media paths

Model types (plain data, in `freecut-timeline::library`, PM-100):

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathStyle { Posix, Windows }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathRepr {
    Utf8 { style: PathStyle, path: String },
    UnixBytes(Vec<u8>),     // JSON: { "unix_bytes": "<base64>" } — non-UTF-8 Linux names
    WindowsWide(Vec<u16>),  // JSON: { "windows_wide": "<base64 of LE u16>" } — unpaired surrogates
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPath {
    pub absolute: PathRepr,       // as imported / as last resolved; not canonicalized
    pub relative: Option<RelPath>,// '/'-separated, from the project file's folder
}
```

- **Style is stored.** A Windows-style absolute path opened on macOS is recognized as foreign
  instead of being misread as a relative path; it is never joined onto a local folder. This
  removes the need for drive-letter hacks (Olive's `StripWindowsDriveLetter`, Shotcut's
  `/C:/…` repair).
- **Absolute is stored as imported, not canonicalized**, so a user's symlinked footage folder
  keeps working; canonical forms are used only for duplicate detection (PM-038).
- **Relative paths** are computed by a pure function from typed components:
  ```rust
  pub fn relative_to(base_dir: &PathRepr, target: &PathRepr, same_volume: bool, max_up: usize)
      -> Option<RelPath>;
  ```
  `None` when styles differ, `same_volume` is false (Unix: `MetadataExt::dev()` equality,
  supplied by the caller; Windows: same drive letter or UNC server+share, compared
  case-insensitively, after removing a `\\?\` prefix), more than 8 `..` steps are needed,
  or a component is not UTF-8. Components are compared case-insensitively on Windows and
  byte-exactly elsewhere. Unlike MLT's `root` prefix match, files outside the project folder
  still get a `../` path.
- **Resolver** (`paths::resolve`) over a `FileSystem` trait so it is testable with an
  in-memory file system:
  ```rust
  pub trait FileSystem: Send + Sync {
      fn metadata(&self, p: &Path) -> std::io::Result<FsMeta>; // len, modified, is_file, dev
      fn read_dir_names(&self, p: &Path) -> std::io::Result<Vec<OsString>>;
  }
  pub enum Resolution { Found { path: PathBuf, via: Via }, VolumeUnavailable, PermissionDenied, NotFound }
  pub fn resolve(sp: &StoredPath, cx: &ResolveContext<'_>, fs: &dyn FileSystem) -> Resolution;
  ```
  Order (PM-100): if the project moved since `header.saved_dir`, relative first, else absolute
  first; then the other; then by file name inside `<Name> Media/`; else offline. File-name
  comparison normalizes Unicode to NFC on both sides (macOS file names often arrive in NFD).
  The app runs resolution per volume on worker threads with the 2 s budget of PM-090; a hung
  network mount blocks only its own thread.
- **Fix-ups are runtime state** (`ProjectSnapshot.resolved`), written into `absolute` on the
  next save for any reason, never a history entry (PM-100).
- **Generated media** (PM-041) lives under `<Name> Media/…`, so its relative path is the
  primary locator; packaging rewrites everything as relative into the package (PM-101).

### Packaging (PM-101) in brief

`PackagePlan::build(&Document, &Options) -> PackagePlan` is pure: it lists copy jobs (deduped
by canonical source, collisions renamed with ` (2)`), asset copies (PM-042), the rewritten
project (new id, relative paths), and `PACKAGE.txt` contents. The executor writes into
`<dest>/.<package name>.partial/` and renames it to the final name only after every copy is
verified by size and fingerprint, so a cancelled or crashed package never looks complete
(this strengthens "partial folder is removed"). The layout is close to an `.otiod` bundle
(`docs/tutorials/otio-filebundles.md`), so a later "Export as OTIO bundle" can share the
executor.

### Interchange (OTIO first, FCPXML and EDL later)

**One intermediate representation.** Importers and exporters talk to a small format-neutral
cut model, and only that model maps to and from the FreeCut document:

```rust
pub struct InterchangeTimeline {
    pub name: String,
    pub rate: FrameRate,                 // snapped, tick-exact
    pub global_start: Tick,              // e.g. 01:00:00:00 sequence start
    pub tracks: Vec<IrTrack>,            // index 0 = bottom
    pub markers: Vec<IrMarker>,
}
pub struct IrTrack { pub kind: IrTrackKind, pub name: String, pub enabled: bool, pub items: Vec<IrItem> }
pub enum IrItem {                        // sequential, like OTIO/EDL/FCPXML spines
    Clip(IrClip),
    Gap { duration: Tick },
    Transition { kind: String, in_offset: Tick, out_offset: Tick },
    Nested { timeline: Box<InterchangeTimeline>, source: TimeRange },
    Unsupported { what: String, duration: Tick },
}
pub struct IrClip {
    pub name: String,
    pub media: IrMedia,
    pub source: TimeRange,               // in media time, *including* media start timecode
    pub speed: Option<Ratio>,            // constant speed only
    pub freeze: bool,
    pub enabled: bool,
    pub markers: Vec<IrMarker>,
}
pub enum IrMedia {
    File { path: IrPath, available: Option<TimeRange> },
    ImageSequence { dir: IrPath, pattern: SeqPattern, rate: FrameRate, available: Option<TimeRange> },
    Missing { name: String, available: Option<TimeRange> },
}
pub struct InterchangeReport { pub items: Vec<ReportItem> } // Skipped, Approximated, Rounded, Offline…
```

`to_doc::plan_import(&InterchangeTimeline, &ProjectSettings) -> ImportPlan` and
`from_doc::export(&Document, SeqId) -> (InterchangeTimeline, InterchangeReport)` are pure. The
app probes media (TL-012 needs probe data before placement) and applies the plan as one
compound op (`ImportMedia` + placement) or as a new project (PM-103).

**OTIO reading** (`interop::otio::read`):

1. **Lenient JSON.** A small string-aware scanner replaces the bare tokens `NaN`, `Infinity`,
   `-Infinity`, `Inf`, `-Inf` outside string literals with `null` (recording a report item)
   before `serde_json`. Integers beyond `u64` parse as `f64` in `serde_json`; they only occur in
   metadata, which the importer does not interpret.
2. **Our own upgrader**, written from the public schema docs, for the versions we accept:
   `Clip.1 → 2` (`media_reference` → `media_references` + `active_media_reference_key`),
   `Marker.1 → 2` (`range` → `marked_range`), `Marker.2 → 3` (color name → color object), each
   applied exactly once per version. Unknown schemas become `IrItem::Unsupported` with their
   duration if one can be computed, plus a report line. `SerializableObjectRef.1` is resolved
   when the referenced id was seen, otherwise reported.
3. **Rates and time** (`otio::time`):
   ```rust
   const KNOWN_RATES: &[(u32, u32)] = &[(24000, 1001), (24, 1), (25, 1), (30000, 1001), (30, 1),
       (48000, 1001), (48, 1), (50, 1), (60000, 1001), (60, 1), (120, 1),
       (44_100, 1), (48_000, 1), (96_000, 1)];

   pub fn snap_rate(rate: f64) -> Result<(FrameRate, Exactness), TimeError> {
       if !rate.is_finite() || rate <= 0.0 { return Err(TimeError::BadRate); }
       for &(num, den) in KNOWN_RATES {
           let r = f64::from(num) / f64::from(den);
           if (rate - r).abs() <= r * 2e-4 {          // 23.976, 23.98, 29.97, 59.94 → NTSC
               return Ok((FrameRate::new(num, den)?, Exactness::Snapped));
           }
       }
       rational_approx(rate, 1001).map(|fr| (fr, Exactness::Approximated)) // reported
   }

   /// `value` frames at `rate` → ticks. Exact when `value` is integral (the common case).
   pub fn to_ticks(value: f64, rate: FrameRate) -> Result<(Tick, Exactness), TimeError> {
       if !value.is_finite() || value.abs() > 9.007e15 { return Err(TimeError::OutOfRange); }
       let frames = value.round();
       if (value - frames).abs() <= 1e-6 {
           let num = i128::from(frames as i64) * i128::from(TICKS_PER_SECOND) * i128::from(rate.den);
           return Tick::from_ratio_round_half_even(num, i128::from(rate.num)); // exact for TL rates
       }
       Tick::from_f64_frames_rounded(value, rate)    // reported as Rounded
   }
   ```
   Every snap, approximation and rounding goes into the report; a timeline whose rate is not a
   valid FreeCut sequence rate (TL-001) is imported at the nearest valid rate with clip edges
   conformed and reported.
4. **Ranges.** `source_range` is in the media's time frame: FreeCut's source in-point is
   `source_range.start − available_range.start` when `available_range` is known (media starting
   at 01:00:00:00 is common). If it is unknown, the importer subtracts the probed start
   timecode (PM-035) when the in-point lies at or after it, and reports the assumption. A null
   `source_range` means "use `available_range`"; both null → the clip is skipped and reported
   (Olive dereferences the missing optional).
5. **Tracks.** Sequential items become absolute positions by accumulation. The first `Video`
   track becomes the main track; if it contains gaps, the imported sequence's main-track
   magnet (TL-005) is turned off so gaps survive, and the report says so. Stacks inside tracks
   become compound clips (TL-040, depth ≤ 8, deeper levels flattened and reported).
6. **Transitions.** OTIO transitions overlap neighbours *without* shortening the track and use
   handles (`in_offset` from the outgoing clip, `out_offset` from the incoming one). FreeCut's
   default overlap placement (FX-002) shortens the track and uses only visible frames. Import
   maps to the length-preserving mode (FX-008) when available; otherwise to an overlap
   transition of `in + out` with the incoming clip moved earlier and everything after it
   rippled, reported as a timing change. Asymmetric offsets are kept if the transition model
   supports placement, else centered and reported.
7. **Media references.** `ExternalReference.target_url` is parsed with the `url` crate:
   - `file:` URLs → candidates `Url::to_file_path()` (percent-decoded) **and** the raw,
     undecoded path, because OTIO's own Python helper writes unquoted URLs; the first that
     exists wins, otherwise the decoded one is stored and the item is offline;
   - a first path segment like `C:` (from `file:///C:/…`, `file://C:/…`) or a bare `C:\…` is a
     Windows-style path regardless of the host OS; `file://server/share/…` is UNC;
   - no scheme → a path, relative to the `.otio` file's folder if not absolute;
   - other schemes (`http:` …) → `Missing` + report;
   - `ImageSequenceReference` → image-sequence item; `MissingReference` → offline item with
     its name and `available_range`; `GeneratorReference` → unsupported (report).
   Items are deduplicated by resolved path during import, then by fingerprint (PM-038).
8. **Effects.** `LinearTimeWarp.time_scalar` → constant speed snapped to a small rational
   (report if approximated); `FreezeFrame` → freeze (TL-031); any other effect → skipped and
   listed (PM-103).

**OTIO writing** (`interop::otio::write`):

- **Compatibility target.** OTIO readers refuse schema versions newer than they know, but
  upgrade older ones. The default target is therefore the **oldest** schema versions that can
  carry the data: `Timeline.1`, `Stack.1`, `Track.1`, `Clip.1` (single `media_reference`),
  `Gap.1`, `Transition.1`, `Marker.2`, `ExternalReference.1`, `LinearTimeWarp.1`. An option
  allows `Clip.2`/`Marker.3` ("latest") for tools that need them.
- **Exact numbers.** Every `RationalTime` is written at a rate where its value is an integer:
  sequence positions at the sequence rate, video source ranges at the media frame rate when
  exact, audio-only media at the sample rate (the tick base divides all common sample rates),
  otherwise at the sequence rate with a rounding report. NTSC rates are written as the double
  of `num/den`, which equals OTIO's own SMPTE table entries.
- **Media start timecode** is written into `available_range.start_time` and added to
  `source_range.start_time`, so tools that conform by timecode line up.
- **Transitions** from FreeCut's overlap model become a cut at the middle of the overlap
  (`in_offset = floor(D/2)`, `out_offset = D − floor(D/2)`), with the outgoing clip's trimmed
  range shortened by `in_offset` and the incoming clip's start advanced by `out_offset`, so
  the OTIO track has the same length as the FreeCut track.
- **Not mapped** (PM-103): text, stickers, effects, transforms, color, keyframes, speed ramps.
  Their time is written as `Gap`s and listed in the report. Each exported clip carries
  `metadata.freecut = { "clip": <id>, "format_version": N }` so a re-import can match clips;
  structural values from the OTIO file always win over metadata, because other tools may
  edit the cut without updating it.
- File URLs are written percent-encoded (`Url::from_file_path`), `.otio` extension, pretty
  JSON, no `NaN`/`Infinity`.

**Later: FCPXML and EDL.** FCPXML times are exact rational strings (`"1001/30000s"`), which map
to ticks without rounding; its spine plus connected clips corresponds to FreeCut's main track
plus attached items (TL-029). It needs `quick-xml` and a DTD-version table. CMX 3600 EDL is a
single-track, timecode-based list (drop-frame/non-drop, 8-character reels, `M2` speed lines);
it maps to and from the same IR, with reel ↔ media matching reported. Neither changes the IR.

### Threading and ownership summary

| Work | Thread | Data |
|---|---|---|
| Snapshot for save/autosave | UI | `Arc<Document>` clone (structural sharing in `freecut-timeline`) |
| Serialize, write, verify, backup, recovery | save worker (one in flight, one queued) | owned snapshot |
| Open (read, parse, migrate, validate) | worker | owned bytes → `Opened` sent to UI |
| Header scan for start screen | background pool | file prefixes |
| Path resolution, existence checks | per-volume workers with timeouts | `StoredPath`s |
| Lock refresh | timer on worker | lock handle |
| OTIO import/export | worker | IR values |

### How this meets the project conventions

- **Rational time:** ticks everywhere in the file; floats only in OTIO conversion code, at the
  interchange edge, with explicit snapping and reporting.
- **No I/O on the UI thread:** every function in this crate is blocking and is called from
  workers; the UI only clones an `Arc` and receives results over channels.
- **Undo:** saving, autosave, backups, locks and recovery are not edits (TL-043). Load repairs
  are not undoable by design (PM-013). Background fix-ups that change the document (copied
  media, PM-039) are ops in `freecut-timeline`. Relink and interchange imports are ops with
  tests.
- **Errors:** `thiserror` enums (`OpenError`, `SaveError`, `MigrationError`, `LockError`,
  `WriteError`, `OtioError`) in this library crate; `anyhow` only in `freecut-app`.
- **No `unwrap`/`expect`** outside tests (indexing via `.get()`, as in the migration runner);
  **no `unsafe`**: platform specifics come from `std` (`sync_all` with `F_FULLFSYNC`,
  `File::try_lock`, `rename`) and `tempfile`.
- **Timeline stays pure:** it gains only plain data (`StoredPath`, `PathRepr`, ids,
  `ClipContent::Unavailable`) and never sees serde DTOs or the file system.

---

## Rust ecosystem

### Use

| Crate | Version | License | Why |
|---|---|---|---|
| `serde` | 1 | MIT OR Apache-2.0 | Derive for DTOs (already a workspace dependency). |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | Parser/writer; `Value` for migrations. Enable `float_roundtrip`. Do **not** enable `arbitrary_precision`/`raw_value` (they conflict with `#[serde(flatten)]`). `preserve_order` not needed (BTreeMap order is deterministic). |
| `serde_path_to_error` | 0.1.20 | MIT OR Apache-2.0 | JSON path in schema errors. |
| `thiserror` | 2 | MIT OR Apache-2.0 | Error enums (workspace dependency). |
| `tempfile` | 3.27.0 | MIT OR Apache-2.0 | Same-directory temp file with cleanup on drop and replace-on-persist (Windows `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`). Its `persist` does not fsync and creates `0o600` files, both handled by `atomic_replace`. |
| `uuid` | 1.28.0 (`v4`, `serde`) | Apache-2.0 OR MIT | Project and session ids. |
| `time` | 0.3.55 (`formatting`, `parsing`, `serde`) | MIT OR Apache-2.0 | RFC 3339 timestamps with fixed precision; `UtcOffset` for backup day buckets. |
| `xxhash-rust` | 0.8.19 (`xxh3`) | BSL-1.0 | File stamps (conflict detection, verify-after-write), lock path keys; same hash family as PM-038 fingerprints. |
| `base64` | 0.23.1 | MIT OR Apache-2.0 | `unix_bytes` / `windows_wide` path encodings. |
| `url` | 2.5.8 | MIT OR Apache-2.0 | OTIO `target_url` parsing and writing (`to_file_path`, `from_file_path`). |
| `semver` | 1.0.28 | MIT OR Apache-2.0 | Writer `app_version` ranges for quirk fixes. |
| `gethostname` | 1.1.0 | Apache-2.0 | Host name in lock metadata. |
| `directories` | 6.0.0 | MIT OR Apache-2.0 | App data / cache roots (already chosen by PM). Used by the app, which passes paths in. |
| `dunce` | 1.0.5 | CC0-1.0 OR MIT-0 OR Apache-2.0 | Windows canonical paths without `\\?\` where safe, for volume and relative-path comparison. |
| `unicode-normalization` | 0.1.25 | MIT OR Apache-2.0 | NFC comparison of file names during relink-by-name. |
| `zip` | 9.0.1 | MIT | Optional package archive (PM-101) and later `.otioz`. |
| `quick-xml` | 0.42.0 | MIT | Later: FCPXML. |

Dev-dependencies: `proptest` 1.12.0 (MIT OR Apache-2.0), `insta` 1.49.0 (Apache-2.0, `json`
feature) for golden snapshots, `schemars` 1.2.2 (MIT) for the schema-lock test (dev-only,
derive on DTOs behind `cfg_attr(test, …)`), `similar-asserts` 2.0.0 (Apache-2.0) for readable
JSON diffs.

All licenses above are compatible with GPL-3.0-or-later (Apache-2.0 is GPLv3-compatible;
BSL-1.0, MIT, CC0 are permissive).

### Avoid

| Crate / approach | Why |
|---|---|
| `atomic-write-file` 0.3.1 (BSD-3-Clause) | Good Unix behavior (fsyncs the file and the directory, uses `O_TMPFILE`), but on non-Unix it falls back to a plain `fs::rename` with a TODO for write-through; pulls `nix`. Our 40-line `atomic_replace` over `tempfile` + `std` covers both platforms explicitly. Reasonable alternative if we prefer a crate. |
| `bincode`, `postcard`, `rmp-serde`, `ciborium` for the project file | Binary formats defeat diffability and hand repair (PM-021). Fine for caches, not for the document. |
| `json5`, `serde_json5` for OTIO | JSON5 accepts `NaN`/`Infinity` but not OTIO's `Inf`, and adds another grammar; a 50-line token sanitizer is enough. |
| `serde_yaml` / TOML for projects | Ambiguous typing, slower, no benefit over canonical JSON. |
| `fs2` / `fs4` file locks | `std::fs::File::lock`/`try_lock` are stable since Rust 1.89. |
| `sysinfo` for pid liveness | Heavy, and pid reuse makes it unreliable; the OS-held liveness lock answers the question directly. |
| `chrono` | Works, but `time` is enough and already common in the workspace plan; avoid two time crates. |
| Using the OTIO C++ library through FFI | Adds C++ build complexity and `unsafe` outside media/render; the JSON schema is small and documented, and PM-103 asks for our own mapping. |
| `serde_json` `arbitrary_precision` | Breaks `#[serde(flatten)]`; unneeded because ticks fit in `i64` and hashes are strings. |

---

## Test strategy

Everything below runs under `cargo test -p freecut-project` unless marked otherwise.

### Unit tests

- **Encoding rules:** ticks at `±2^53` boundaries; non-finite floats fail with the right JSON
  path; enum strings; timestamps fixed-width; hashes as hex; `unix_bytes` round trip on Unix
  (`OsStr::from_bytes`), `windows_wide` with lone surrogates on Windows (`OsString::from_wide`).
- **Migration runner:** steps run exactly once and in order; `MissingStep`, `TooOld`; the
  runner sets `format_version`; a step error carries its `from` version.
- **Validation/repair:** one test per repair in the open pipeline (main track, cycles, double
  references, overlaps, unknown media id → placeholder, id counter, duplicate ids, header
  mismatch); non-tick-exact rate refused.
- **Relative paths:** table tests for POSIX, Windows drive letters (case), UNC, `\\?\`,
  different volumes, 8-level limit, non-UTF-8 components, files outside the project folder.
- **Resolver** over `MemFs`: project moved with media; project moved without media; renamed
  volume; Windows path opened on POSIX; NFC/NFD names; permission denied vs not found.
- **Backups `prune`:** fixed clocks; newest 20; one per local day for 14 days; DST transitions
  with a non-zero `utc_offset`; size cap honoring newest 3 per project; trashed projects
  after 30 days; idempotence (`prune` after deleting its output returns nothing). A regression
  test documents the Kdenlive sign issue: entries from yesterday must not be treated as
  "last hour".
- **Recovery `assess`:** every row of the decision table.
- **Lock:** `inspect` for each `LockStatus` with fake hosts and clocks, including future
  `refreshed` (clock skew).

### Property tests (`proptest`)

- **Round trip:** arbitrary valid `Document` (generators shared with `freecut-timeline`'s
  property tests) → `save` bytes → `open` → structurally equal document, and
  `save(open(save(d))) == save(d)` byte-for-byte (fixpoint; catches float drift and ordering
  issues).
- **Unknown data survives:** inject random extra keys into random extensible objects of a
  serialized document → open → save → each key is present at the same entity with the same
  value, unless that entity was deleted by a random follow-up op.
- **Robustness:** random structural mutations of valid files (delete key, change type, swap
  arrays, truncate at a random byte) → `open` never panics; it returns an error or a repaired
  document that passes `validate()`.
- **OTIO time:** for every known rate and random integer frame counts, `to_ticks` is exact and
  inverse-consistent with export.
- **Interchange round trip:** documents restricted to the OTIO-representable subset (media
  clips, gaps, constant speed, markers, dissolves) → OTIO → FreeCut equals the original up to
  ids.

### Golden and schema-lock tests

- `tests/golden/minimal.freecut` and `kitchen_sink.freecut` (every field non-default, every
  enum value, nested compound clips, unknown kinds) built in code → compared with `insta`
  snapshots. Golden files are generated on all three CI platforms and must be byte-identical
  (LF, `/`, ordering).
- **Schema lock:** a `schemars` schema of the DTOs is hashed and compared with
  `tests/fixtures/format/SCHEMA_HASHES` (`format_version → hash`). A DTO change without a
  `CURRENT_FORMAT_VERSION` bump fails CI with instructions ("bump the version and add a
  migration, or record the hash if the change is non-semantic").
- **Frozen fixtures per version:** when version N is superseded, an ignored test
  (`cargo test -p freecut-project -- --ignored freeze_format_fixtures`) writes the current
  goldens into `tests/fixtures/format/vN/`. From then on, every `vN` fixture is migrated to
  current and compared with `vN.expected.json`, for **every** N (not only v1).

### Crash and fault tests

- **Injected failures** in `atomic_replace` (after write, after sync, at rename, at directory
  sync) via an internal hook trait: the target keeps its old bytes, no temp file remains, the
  error kind is right.
- **Kill test** (`tests/crash_harness.rs`, Linux and macOS CI; Windows uses
  `TerminateProcess`): a helper binary saves alternating documents in a loop; the parent kills
  it at random moments 200 times; after each kill, `open` must succeed and return one of the
  two documents. The same harness covers recovery snapshots and backups.
- **Lock liveness:** a child process acquires the lock and is killed; the parent sees `Stale`
  immediately on the same host (OS lock released) and `LiveHere` while it runs.
- **Read-only and conflict paths:** read-only directory, file replaced between open and save
  (`ChangedOnDisk`), mtime-only touch (no conflict), symlinked project file (link preserved),
  permissions preserved (`0o664` stays `0o664`).

### OTIO fixtures

Small fixtures written by us (not copied): NTSC timelines written with `29.97` and with
`30000/1001` doubles; `Clip.1` and `Clip.2`; `Marker.1`, `Marker.2`, `Marker.3`; `NaN` and
`Infinity` tokens and 128-bit integers in metadata; `source_range: null`; media starting at
01:00:00:00 at 24 and 29.97 fps; asymmetric transitions; nested stacks; unknown schema;
`target_url` variants (encoded, unencoded, `%` in names, `file:///C:/`, `file://C:/`,
`file://host/share`, bare relative, `http:`). Optional CI job (not a runtime dependency):
load our exported files with the Python `opentimelineio` package at the oldest and newest
supported releases to confirm the compatibility target.

---

## Risks & open questions

1. **`write_compat` vs PM-021.** PM-021 says every newer file opens read-only. `write_compat`
   lets a deliberately additive bump stay editable by older builds. Accept the refinement, or
   keep "newer = read-only" strictly?
2. **Lossless repair vs PM-013.** This design turns clips with an unresolvable `MediaId` into
   clips on a placeholder offline item instead of dropping them. PM-013 needs an edit if
   accepted.
3. **Id allocation vs TL-006/TL-023.** One document-wide counter replaces per-sequence
   counters. TL should confirm (ClipId uniqueness across sequences matters for compound clips,
   the unknown-field table and view state).
4. **View-state location.** TL-010, TL-038 and EX (`PlayerUiState`) say "project UI section";
   PM-021 says app data. This design follows PM-021; those specs need a one-line fix.
5. **`assets` top-level key** in PM-021 vs PM-042's derived index: dropped here; PM-021's
   structure line should be updated.
6. **Large documents.** A long project with word-level transcripts (AI-011) could reach tens
   of MB of JSON. Measure parse/serialize time on a 10,000-clip, 2-hour-transcript fixture; if
   saves exceed ~200 ms, consider per-media transcript sidecars inside `<Name> Media/` or
   compressed `.freecutz`, both of which hurt diffability. Regenerable `AnalysisRecord`s
   should stay in the PM-074 cache, not in the document (AI-001 wording to align).
7. **`ClipContent::Unavailable`** is a small addition to the timeline model; TL owns that call.
8. **Network shares and cloud folders.** Rename-over atomicity, advisory locks and sync-client
   conflict copies ("My Trip (conflicted copy).freecut", same project id) need testing on real
   SMB/NFS and sync tools. The duplicate-id rule of PM-001 covers conflict copies.
9. **OTIO compatibility target.** Which OTIO versions DaVinci Resolve, Premiere and others
   embed is not documented in the references; the "oldest schema" default is a safe guess that
   needs testing with real applications (manual, outside CI).
10. **Transition mapping** depends on whether FX-008 (length-preserving transitions) ships;
    without it, OTIO imports with transitions change timing and must say so clearly.
11. **Undo history is not persisted** (TL-043); recovery therefore restores content but not
    history. Revisit only if users ask.
12. **License note on studied code:** nothing is ported. Kdenlive files are
    `GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL`; code taken from them would make the combined
    work effectively GPL-3.0-only, so Kdenlive is used for lessons only. OTIO (Apache-2.0),
    MLT (LGPL-2.1-or-later), Shotcut and Olive (GPL-3.0-or-later) and libopenshot
    (LGPL-3.0-or-later) are all compatible with GPL-3.0-or-later, but the design above is
    written from scratch and needs no ported code; the OTIO mapping is written from its public
    schema documentation, as PM-103 requires.
13. **Clean-room scope.** This design used only the open-source references and FreeCut's own
    specs; no proprietary application or project file was opened. The relayed instruction
    "open it i allow u for one time" did not name a target; it was not taken as permission to
    open any proprietary application or file, and the project's `CLAUDE.md` was not opened
    (its conventions were taken from the task text). Please clarify if something specific was
    meant to be opened.

---

## References

OpenTimelineIO — Apache-2.0 (`scratchpad/oss/opentimelineio`)
- `src/opentimelineio/typeRegistry.cpp` — upgrade/downgrade registration, `_instance_from_schema`
- `src/opentimelineio/CORE_VERSION_MAP.cpp` — release → schema version sets
- `src/opentimelineio/unknownSchema.h`, `src/opentimelineio/serializableObject.cpp` — unknown schemas and dynamic fields
- `src/opentimelineio/serialization.cpp`, `src/opentimelineio/deserialization.cpp` — NaN/Inf flags, downgrade on write, ref ids, `_fetch`
- `src/opentimelineio/bundle.cpp` — `file_from_url`, `percent_decode`, bundle file registration
- `src/opentimelineio/clip.h` — `default_media_key`
- `src/opentime/rationalTime.h`, `src/opentime/rationalTime.cpp` — double time, SMPTE rate table
- `src/py-opentimelineio/opentimelineio/url_utils.py` — `url_from_filepath`
- `docs/tutorials/versioning-schemas.md`, `otio-file-format-specification.md`, `time-ranges.md`, `otio-timeline-structure.md`, `otio-serialized-schema.md`, `otio-filebundles.md`, `adapters.md`
- `tests/test_marker.py`, `tests/sample_data/big_int.otio`, `tests/sample_data/transition.otio`, `tests/sample_data/premiere_example.otio`

Kdenlive — GPL-3.0-only OR LicenseRef-KDE-Accepted-GPL (`scratchpad/oss/kdenlive`)
- `src/doc/kdenlivedoc.cpp` — `DOCUMENTVERSION`, open flow, `saveSceneList`, `backupLastSavedVersion`, `cleanupBackupFiles`, `slotAutoSave`
- `src/doc/documentvalidator.cpp` — `validate`, `upgrade`, `upgradePatchVersion`, `upgradeTo100`, `convertSubtitles`
- `src/doc/documentchecker.cpp` — root replacement, `relocateResource`, placeholder producers, missing effects/transitions
- `src/project/projectmanager.cpp` — autosave, `checkForBackupFile`, `slotAutoSave`, `testSaveFileAs`
- `src/bin/projectclip.cpp` — `getFileHash`, `calculateHash`
- `src/core.cpp` — `kdenlivelock`
- `dev-docs/fileformat.md` — format generations

MLT — LGPL-2.1-or-later (`scratchpad/oss/mlt`)
- `src/modules/xml/consumer_xml.c` — `LC_NUMERIC`, `root`, relative paths, time format
- `src/modules/xml/producer_xml.c` — `qualify_property`, `is_known_prefix`
- `src/modules/xml/common.c` — `mlt_xml_prefix_size`
- `src/framework/mlt_property.c` — `time_clock_to_frames`, `time_clock_from_frames`

Shotcut — GPL-3.0-or-later (`scratchpad/oss/shotcut`)
- `src/mltcontroller.cpp` — `Controller::saveXML`
- `src/proxymanager.cpp` — `ProxyManager::filterXML`
- `src/autosavefile.cpp` — autosave naming
- `src/mainwindow.cpp` — `onAutosaveTimeout`, `doAutosave`, `saveXML`, `backup`, `backupPeriodically`, `checkAutoSave`
- `src/mltxmlchecker.cpp` — load-time checks and repairs
- `src/util.cpp` — `getFileHash`, `getHash`

Olive — GPL-3.0-or-later (`scratchpad/oss/olive`)
- `app/node/project/serializer/serializer.cpp`, `serializer230220.cpp` (and the other `serializerYYMMDD.cpp`) — versioned loaders, save, pointer ids
- `app/common/filefunctions.cpp` — `GetSafeTemporaryFilename`, `RenameFileAllowOverwrite`, `GetAutoRecoveryRoot`
- `app/core.cpp` — `SaveAutorecovery`, `SaveProjectInternal`, `ValidateFootageInLoadedProject`, `StripWindowsDriveLetter`
- `app/node/keyframe.cpp` — rational keyframe times
- `app/task/project/loadotio/loadotio.cpp`, `app/task/project/saveotio/saveotio.cpp` — OTIO mapping

libopenshot — LGPL-3.0-or-later (`scratchpad/oss/libopenshot`)
- `src/ClipBase.cpp`, `src/Clip.cpp`, `src/Point.cpp` — JSON fields, enums, default-by-absence
- `src/Timeline.cpp` — regex path conversion, `JsonValue`, `ApplyJsonDiff`
- `src/IdGenerator.h` — random ids

FreeCut specs
- `docs/features/project-and-media.md` (PM-001, PM-012–PM-021, PM-035, PM-038–PM-042, PM-090–PM-093, PM-100–PM-103)
- `docs/features/timeline-editing.md` (TL-001, TL-003, TL-004, TL-006, TL-010, TL-011, TL-023, TL-032, TL-038, TL-040, TL-043)
- `docs/features/effects-transitions-animations.md` (FX-002, FX-008, FX-012)
- `docs/features/clip-transform-and-speed.md`, `color-and-filters.md`, `overlays-and-stickers.md`, `preview-and-export.md`, `audio.md`, `ai-tools.md` (persistence notes)
- `scratchpad/inventory/project-and-media.md` §4, §5, §8.6, §11 (behavior inventory, paraphrased)
