# Audio
> Status: draft · Owns: audio gain and loudness units (dB, LUFS, pan, cents), the clip audio attribute set (`ClipAudio`: volume, fades, mute, pan, channel map, cleanup, echo reduction, voice clarity, isolation, voice effects, pitch, keep pitch, ducking), the audio signal chain and master bus, the master and track audio slots (`Sequence.audio`, `Track.audio`), the project mute and solo, the "All streams mixed" stream choice, audio-tool availability rules and the silent-source flag, the before/after compare bypass, the gain overlay drawn on waveforms, keyboard volume steps, the Keep pitch flag and time-stretch engine, beat detection and beat markers' audio side (detection job, density, sensitivity, tempo readout, tap key), voice isolation and the "Split into stems" command, audio crossfades between audio clips, ducking envelopes, the audio auto-fix assistants, voiceover recording, audio-based sync of separate recordings, the music and sound-effect library panels, audio-only export options · Crates: freecut-timeline, freecut-media, freecut-audio, freecut-project, freecut-app (ML inference for AU-020/AU-021 runs in freecut-analysis, owned by ai-tools.md)

## Summary
This area covers everything a FreeCut user hears. You can put music, sound effects and voiceovers on audio tracks, use the audio inside video clips, or split that audio off as a separate clip. Each clip has volume, fades, mute and keyframed volume. On top of that you can clean up speech (loudness normalization, noise and echo reduction, voice polish), compare the result with the original, apply voice effects, change pitch, use beat markers to cut on, record narration, and automatically duck music under speech. Every result is mixed into one stereo master that plays in real time and exports either with the video or as an audio-only file. All of it runs locally. FreeCut has no online music store. Libraries hold only the user's own files and assets that are open-licensed or made by the FreeCut project.

### Shared definitions (owned here)
- **Gain** is stored as an integer number of hundredths of a decibel (`GainDb(i32)`), with a sentinel value for −∞, meaning silence. Floats are used only inside `freecut-audio` DSP and at the UI edge. This keeps undo comparisons, serialization and tests exact, and makes 0.1 dB steps exact (10 units).
- **Non-time scalars:** only time has to be rational (CLAUDE.md). Under the shared rule recorded in clip-transform-and-speed.md's conventions, each owning spec picks its own type for non-time values, and animated values go through TR-027's generic `Animatable<T: Interpolate>`. AU's choices are the integer types in this list. `GainDb` implements `Interpolate` in the dB domain, switching to linear amplitude on a segment that touches −∞ (AU-008). `Pan` interpolates linearly and rounds when evaluated. This settles the earlier open question: beyond this rule, there is no project-wide convention for non-time scalars.
- **Pan** is an integer from −100 (fully left) to +100 (fully right). 0 is centre.
- **Pitch** is an integer number of cents (100 cents = 1 semitone).
- **Loudness** uses LUFS/LKFS per ITU-R BS.1770 / EBU R128, stored in tenths of a LU (`i16`). **True peak** is in dBTP.
- **Strength** for cleanup and effect controls is an integer from 0 to 100 %.
- **Time** is not defined here. Positions, durations, fade lengths and envelope points use the `Tick` type from timeline-editing.md (TL-001). Clip-local time `t` and source time `s`, and the map between them, come from clip-transform-and-speed.md (TR time map).

### Signal chain (per clip, in order)
Source-time stages (cacheable per media, stream, source range and settings):
1. Decode the clip's selected audio stream, or all of its streams mixed (AU-038), in `freecut-media`; or the cached voice or background stem when an isolation mode is set (AU-021). Stem clips made by "Split into stems…" (AU-021) decode their stem file here like any other media. All stem files are AI-023's output.
2. Downmix to stereo and apply the channel map (AU-014).
3. Voice clarity, when set (AU-020): plays the processed audio that AI-024's engine wrote to the processed-audio cache.
4. Noise reduction (AU-018), echo reduction (AU-037), then voice polish (AU-019).

Retime stage:
5. Reverse, time-stretch or varispeed. The time map is TR's (TR-036), read through TR-038's contract, and the Keep pitch flag is AU-023's. Pitch shift (AU-024) and the pitch part of voice effects (AU-022) are applied in the same pass.
6. Resample to the project sample rate (AU-010).

Timeline-time stages:
7. Voice effect (AU-022), then the filter stack (AU-034, later).
8. Loudness normalization gain (AU-017).
9. Clip volume, or its keyframe envelope (AU-005, AU-008).
10. Ducking envelope (AU-028).
11. Fades (AU-006), audio crossfades (AU-029) and the transition crossfade of FX-007.
12. Clip mute (AU-007) and clip enable (TL-036).
13. Clip pan (AU-014).
14. Track stage: volume and pan (AU-013), then track mute (TL-008, AU-011) and solo (AU-012).
15. Master stage: master volume and safety limiter (AU-010), then the project mute (AU-039).
16. Send the result to the output device or the export encoder.

Preview-only bypass: while Compare (AU-041) is active, steps 3, 4 and 7 are skipped for the compared clips, and step 1 decodes the original instead of an isolation stem. Export never applies it. Which stages a clip can use at all is AU-040's table.

## Features

### AU-001 Audio tracks and audio clips — mvp
**What the user sees:** Audio tracks sit below the main track and the video tracks (track model: TL-004). Each audio clip is a coloured bar showing its name and waveform (TL-042). Dropping an audio file from the media library onto the timeline creates an audio clip. Dropping it below the last track creates a new audio track (TL-006). Clip colour is chosen by origin (music, sound effect, voiceover, extracted, other) so different kinds of audio are easy to tell apart. A user colour label (TL-045, later) overrides it. Colour is cosmetic only.
**Inputs / controls:**
- Drag and drop from the media library (mvp), or from the audio panels (AU-030, AU-031; v1).
- An "add at playhead" button on library items.
- Track header controls: name (TL-010), mute and hide (TL-008, mvp; mixer side AU-011), solo (AU-012, v1) and lock (TL-009, v1).
- The number of tracks follows TL-004's limit (100 tracks per sequence). Performance target (FreeCut default): at least 32 overlapping clips mixed in real time at 48 kHz on a mid-range laptop.
- **Accepted audio formats** are PM-036's: everything FreeCut's FFmpeg build decodes, including MP3, WAV, AAC/M4A, FLAC, Ogg Vorbis and Opus. The observed reference app accepts only MP3 and WAV in its audio importer; FreeCut does not narrow the list.
- **Import from the audio panel** (AU-030's "Import audio…" button) accepts only files with an audio stream. Other files are skipped, and PM-037's import report gives the count ("2 files skipped: no sound"), worded for one file or several.
  - FreeCut default (app): the file dialog offers audio files and video containers. A video with sound picked here is imported and its sound extracted at once (AU-004), as one undo step, so every picked file shows up in the Audio panel; the video item itself appears in Media only. The report reads "Imported 2 files. 1 file skipped: no sound". Files already in the project are imported again, as from Media, until PM-038 exists. The first new item is highlighted in the Audio panel.
**Behavior / output:**
- Audio clips use the same move, trim, split, delete, ripple and snap operations as every other clip (timeline-editing.md). Positions are rational ticks (TL-001).
- Audio clips live only on audio tracks. Dropping an audio file on a video track redirects it to the nearest audio track, or a new one (TL-004, TL-006).
- Edit points land on the sequence frame grid (TL-001). Sub-frame trimming of audio (useful for cutting out a breath or click) is not offered. It is an open request to TL for a later version.
**Edge cases:**
- **File with several audio streams:** the default stream is used. Stream selection lives in the clip's media reference (TL-011). The stream picker, the import notice and "Add stream as clip" come in v1 (AU-038).
- **Media offline:** the clip shows TL-011's offline pattern and plays silence. Relinking follows project-and-media.md.
- **Inexact duration** (for example VBR MP3 without a seek header, or some Ogg files): `freecut-media` does a background exact-length scan. If the real length is shorter than estimated, the clip's out-point is clamped by a non-undoable model correction, which is logged.
- **Audio-only project (no video at all):** allowed. Preview shows the empty canvas, and export can be audio-only (AU-032).
- **Overlap on one track:** two clips cannot overlap on the same track (TL-004). Overlapping audio on different tracks is summed.
- **Trimming past the end of the stream:** prevented by the source-range limit (TL-011).
- **Disabled clip (TL-036):** silent, whatever its audio settings.
- **Silent source** (digital silence, or a peak that never passes AU-040's threshold): imported and placed normally. The inspector says "No sound in this source", and analysis-based tools refuse it with that reason (AU-040).
- **File already in the project:** PM-038's duplicate detection applies. Importing from the audio panel reports how many files were already there instead of adding copies.
**Undo:** Uses the timeline ops (insert, move, trim, split, delete). Their inverses are defined in timeline-editing.md.
**Model notes:**
- `freecut-timeline`: `TrackKind::Audio` (TL-004) and the `ClipAudio` property bag, which is shared with the embedded audio of video clips (AU-002) and stored with the clip's other property bags (TL-011). `ClipAudio` holds:
  - `gain: Animatable<GainDb>` (static volume plus optional keyframes, TR-027; AU-005, AU-008) and `pan: Animatable<Pan>` (AU-014).
  - `muted`, `fade_in`, `fade_out`, `channel_map`.
  - `isolation` (AU-021), `normalize` (AU-017), `denoise` (AU-018), `dereverb` (AU-037), `polish` (AU-019) and `clarity: Option<Strength>` (AU-020).
  - `voice_fx` (AU-022), `filters` (later, AU-034), `pitch` and `preserve_formants` (AU-024), `preserve_pitch` (the Keep pitch flag, AU-023, default on).
  - `duck` (AU-028) and `origin` (the colour category).
- These live outside `ClipAudio`: stream selection (TL-011), the retime itself (`Retime`, TR-036) and beat markers. Beat markers are TL-037 clip markers with origin Beat, produced by AU-025. There is no `ClipAudio.beats` field and no separate beat-marker type.
- `freecut-media`: probes each stream for codec, channel count and layout, sample rate, duration and start offset.
- `freecut-project`: serializes all of the above.

### AU-002 Audio inside video clips — mvp
**What the user sees:** A video clip whose source has sound plays that sound. Selecting it shows an Audio section in the inspector with the same controls an audio clip has. The clip shows a waveform band under its thumbnails (TL-042).
**Inputs / controls:** The full `ClipAudio` set: volume, fades and mute in the mvp, and the rest in v1.
**Behavior / output:** The embedded audio follows the video clip through every trim, split, move, speed change and reverse. If the source has no audio stream, the Audio section is disabled and says the source has no sound.
- FreeCut default (model): relinking or correcting a media item brings its video clips' embedded sound in line with the new file in the same undo step: no sound becomes active (default settings) when the new file has an audio stream, and active becomes none when it has none. Detached sound stays detached.
**Edge cases:**
- **Audio and video streams with different start times**, including encoder priming delay and container edit lists: `freecut-media` aligns by presentation timestamps so sync matches a reference player. Tests should use fixture files with a known offset.
- **Audio shorter than the video:** silence plays after the audio ends.
- **Audio longer than the video:** the audio is cut at the clip boundary.
- **Variable-frame-rate phone footage:** audio is never stretched to match video frame timestamps. It plays at its own clock. Handling VFR video is project-and-media.md's job.
- **Two or more audio streams** (dual-mic cameras, screen recordings with separate system and mic audio): the mvp plays the default stream only. In v1, AU-038 adds the import notice, the stream picker (inspector and context menu, with "All streams mixed") and "Add stream as clip". Detaching creates one clip per enabled stream (TL-030).
- **Still images, titles, stickers:** these have no `ClipAudio`.
**Undo:** The same per-attribute ops as audio clips.
**Model notes:** Embedded audio uses TL-030's `EmbeddedAudio::{Active(ClipAudio), Detached(LinkId), None}`. It is `None` when the source has no audio stream. The mixer reads only `Active` embedded audio (TL-030).

### AU-003 Detached audio: settings hand-off and transitions — v1
**What the user sees:** The Detach audio and Restore audio commands, where the new clip is placed, and the link badge are all defined in timeline-editing.md (TL-030). AU defines what the sound does: after detaching, the new audio clip sounds exactly as the embedded audio did, and after restoring, the video clip sounds as the detached clip did.
**Inputs / controls:** None beyond TL-030 (command, shortcut, multi-selection).
**Behavior / output:**
- The whole `ClipAudio` set moves to the new clip (TL-030's moved properties): volume, keyframes, fades, mute, pan, channel map, cleanup (noise reduction, echo reduction, voice polish), voice clarity, isolation, voice effect, pitch, Keep pitch (AU-023), normalization and ducking.
- A short toast confirms the detach and names the track the new clip went to (toast style: UI-040). Observed in the reference app: a confirmation toast after the split, with the audio inspector then editing the new clip; which clip ends up selected stays TL-030's rule.
- The new clip carries its own copy of the video clip's retime (speed, curve, reverse; TR), so the two stay in sync.
- Analysis caches (AU-017, AU-018) are keyed by media, stream, source range and settings, so they stay valid and nothing is analysed again.
- Nothing is re-encoded and no file is written.
- On Restore, the settings of the first linked clip move back to the video clip (TL-030).
**Edge cases:**
- **Video transition with audio crossfade on the clip's edge (FX-007):** the detached clip leaves that transition. Its stored fade on that edge is set to the transition's duration with the Fast (equal-power) curve (AU-006), so the sound does not change. When both neighbours are detached, their clips overlap on separate audio tracks and the two fades sum to the same crossfade. Restore puts the previous fade values back.
- **Several audio streams:** one clip per enabled stream (TL-030). Each gets a copy of the settings, and Restore takes them from the first.
- **Detached clip edited later** (volume, effects): Restore carries those edits back, after TL-030's confirmation.
- **Ducking (AU-028):** the duck envelope moves with the settings. If the detached clip now sits on a track that is not a selected speech source, the duck envelopes of other clips show "needs update".
- **Embedded audio was muted:** the new clip is muted too.
- **Restore while a detached clip still exists:** TL-030 removes the linked clips (after its confirmation), so the sound is never doubled. Observed alternative: the reference app's restore brings back only the video clip's own sound and leaves the separated clip on the timeline unchanged.
**Undo:** Part of TL-030's `DetachAudio { video, created_audio_clips, moved_props }` and `RestoreAudio`. The transition-fade adjustment is recorded in the same entry.
**Model notes:** A pure hand-off function in `freecut-timeline` (for example `detach_audio_props(&ClipAudio, edge_transitions) -> ClipAudio`), unit-tested with and without edge transitions.
- FreeCut default (until FX-007's transitions exist): the whole `ClipAudio` moves unchanged on Detach, and on Restore the settings come from the first linked clip, with the fade-out taken from the linked clip that ends last (TL-030's defaults). If no linked clip is left, Restore gives back default settings.

### AU-004 Pull the sound out of a video file in the media library — v1
**What the user sees:** A context-menu command on a video item in the media library adds an audio-only item named after the file, such as "clip.mp4 (audio)". The item shows up in the library and in the Project section of the audio panel. A separate "Save audio as file…" command writes a real audio file to disk.
**Inputs / controls:**
- Choose the stream if there are several.
- For saving: format and quality, using the same choices as AU-032.
**Behavior / output:**
- The extracted item is a virtual media entry that points to the original file's audio stream. It is instant and makes no copy. It behaves exactly like an imported audio file.
- "Save audio as file…" runs in a background job using the audio-only export encoder.
- Works on any media item with an audio stream, whatever its container (the observed reference flow lists MP3, M4A, MP4 and MOV inputs). There is no usage counter or quota: one observed reading of the reference app puts extraction behind a free-use quota, which FreeCut drops (see Out of scope).
- The new item carries an "Extracted" badge in the media panel and the audio panel, so it is easy to tell apart from imported files.
**Edge cases:**
- **Source offline:** the extracted item is offline too, and the two relink together.
- **Video without sound:** the command is disabled with the reason "This video has no sound" (UI-030's disabled-reason rule). The observed reference app accepts the click and then says there is nothing to extract.
- **Extracting the same stream twice:** the existing item is reused and selected. No duplicate is created.
- **Original video item removed from the library while the extracted item is still in use:** the file stays referenced (project-and-media.md rules).
- **Save cancelled mid-way:** the partial file is deleted.
**Undo:** Adding the library item is undoable (a media op from project-and-media.md). Writing a file to disk is file I/O, not an edit, so undo never deletes the saved file.
**Model notes:** Project-and-media.md owns media items. The extracted item is PM-041's virtual source `MediaSource::AudioOf { source: MediaId, stream }`. `freecut-media` handles probing and encoding.
- FreeCut defaults (model): the extracted item is named after the original's display name, or its file name when it has none, plus " (audio)". It keeps its own copy of the original's path and fingerprint, and its probe data holds only its one stream on the original's media timeline (same length and stream start), so source ranges line up with the video. Without a chosen stream, the first audio stream is used. Extracting from an extracted item gives back that item. Relinking (or correcting) the original updates every extracted item in the same undo step, following each item's stream by its place among the file's audio streams (so a `.mov` whose sound moved from stream 1 to 2 behind a timecode track still matches); an extracted item with no matching stream in the new file is left as it was, to be relinked on its own, and the edit reports it. Relinked on its own, an extracted item whose stream index the new file lacks takes the new file's first audio stream that no other item extracted from the same original plays.
- FreeCut default (app): *Extract audio* is in a video tile's menu in Media (UI-035). Afterwards the Audio tab opens with the Extracted chip on and the item highlighted; extracting again highlights the existing item, says "The sound of clip.mp4 is already in Audio" and adds no undo step. The "Extracted" badge shows on the tile in Media and on the row in Audio. "Save audio as file…" and the stream choice are not built yet.

### AU-005 Clip volume — mvp
**What the user sees:** In the inspector's audio section, a Volume slider with a numeric field in decibels (for example "0.0 dB"). Hovering shows the equivalent percentage. A Reset button returns every audio attribute of the clip to its default.
**Inputs / controls:**
- **Range:** −60.0 dB to +20.0 dB, plus a −∞ (silent) detent at the bottom. **Default 0.0 dB.** (FreeCut default. Public material does not confirm the reference editor's ceiling.)
- The slider is dB-scaled, with finer resolution near 0 dB. Step is 0.1 dB.
- The field accepts dB ("-6", "+3.5") or a percentage ("50%" becomes −6.02 dB, "200%" becomes +6.02 dB).
- Arrow keys change the value by ±1 dB, or ±0.1 dB with Shift. This works only while the field has focus. Double-clicking the slider resets it to 0 dB.
- To change the volume of selected timeline clips from the keyboard without focusing the field, use the volume step commands (AU-036).
- **Multi-select:** typing a value sets it on every selected clip. Alt-drag applies a relative offset instead (FreeCut default).
**Behavior / output:** The gain is applied at step 9 of the signal chain and is the same in preview and export. While you drag during playback, the change is smoothed over about 10 ms to avoid zipper noise. Reference: public tutorials for the reference editor suggest leaving background music around −20 to −30 dB under speech, which suits a dB readout.
**Edge cases:**
- **Positive gain pushes peaks above 0 dBFS:** those waveform regions are tinted in a warning colour (AU-009, v1). The master limiter catches the overs (AU-010).
- **−∞ versus mute:** both are silent, but mute keeps the stored level (AU-007).
- **Clip has volume keyframes (v1):** the field shows the envelope value at the playhead. Editing it creates or updates a keyframe at the playhead (keyframe behaviour from clip-transform-and-speed.md).
- **Selection includes clips without audio:** those are ignored.
- **Clips on locked tracks:** they cannot be selected (TL-009), so they are never changed.
- **Typed value out of range:** it is clamped and the field flashes.
**Undo:** `SetClipGain { clips, old: Vec<GainDb>, new }`. A continuous drag merges into one undo entry (coalescing rule, TL-043). A multi-select change is one entry. Reset is `ResetClipAudio { clip, old: ClipAudio }`, which restores the whole previous attribute set. Keyboard steps use AU-036's `StepClipGain`.
**Model notes:** `GainDb(i32)`, held as the static value of `ClipAudio.gain: Animatable<GainDb>`. Validation clamps happen inside the op. Conversion to linear `f32` happens only in `freecut-audio` and at the UI edge.
- mvp model (FreeCut default): `ClipAudio.gain` is a plain `GainDb` until TR-027's `Animatable` lands. `GainDb` clamps on construction, so every stored value is −∞ or within −60.0 to +20.0 dB. One `SetClipAudio` request sets one attribute (volume, mute, fade-in or fade-out) on every selected clip that plays its own sound, as one undo entry; other clips are skipped, a selection with none is refused, and an edit that changes nothing adds no entry.
- mvp inspector (FreeCut defaults, app): the slider's bottom 2 % of travel is the −∞ detent; from there to three quarters of the travel the level rises from −60 dB to 0 dB on a squared curve (most travel near 0 dB), and the last quarter is linear up to +20 dB. Slider values land on 0.1 dB steps; typed values keep 0.01 dB, so "50%" stays −6.02 dB, and "−inf" or "0%" means silence. The field commits on Enter or when it loses focus. While it has focus, Arrow Up/Down step ±1 dB and Shift+Arrow ±0.1 dB, each step its own undo entry; stepping up from −∞ starts at −60 dB, and stepping down stops there. A slider or field drag is one undo entry (TL-043 gesture), including the value set by the frame that releases it. Reset sets volume, mute and both fades back to their defaults as one entry. The mvp section edits a single clip; with several clips selected it offers only the mute toggle (and TL-030's detach or restore).

### AU-006 Fade in / fade out — mvp
**What the user sees:** Fade-in and fade-out fields, in seconds with one decimal, in the inspector. On the timeline clip, two small handles sit at the top corners. Dragging a handle inward draws a ramp over the waveform (AU-009).
**Inputs / controls:**
- **Length:** 0.0 s up to the clip's duration. **Default 0.0 s.** The UI snaps lengths to frames, and a modifier allows sub-frame lengths (a fade length is a clip attribute, not an edit point, so TL-001's grid does not bind it). The rule fade-in + fade-out ≤ clip duration is enforced while editing: the slider stops at the limit and shows it.
- **Curve (v1):** Linear amplitude (default), Smooth (S-curve, ½ − ½·cos(π·p)), Fast (equal-power quarter sine, sin(π·p/2); quick at first) or Slow (1 − cos(π·p/2); gradual at first). `p` runs from 0 to 1 across the fade, and a fade-out mirrors the fade-in shape.
**Behavior / output:**
- The fade-in ramps from silence at the clip start to full level after its length. The fade-out does the reverse at the clip end.
- Fades multiply with volume, keyframes, ducking and FX-007's transition crossfade.
- Fade lengths are measured on the timeline, in output time. A 2 s fade stays 2 s at any clip speed (FreeCut default).
- Reference: in the reference desktop editor, the fade controls sit in the clip's basic audio properties, are set in seconds, and shape only the clip's sound, not its picture. Its observed UI labels show separate fade-in and fade-out duration controls but no range or default, so the length limit and the fade-in + fade-out ≤ duration rule above are FreeCut's.
**Edge cases:**
- **Clip later trimmed shorter than its fades:** the stored values are kept. Render uses clamped effective values, `in_eff = min(in, dur)` and `out_eff = min(out, dur − in_eff)`. Lengthening the clip again restores the original fades without any data change.
- **Split:** the left piece keeps the fade-in and the right piece keeps the fade-out. The new inner edges have no fade.
- **Fade on an edge that has an audio crossfade (AU-029):** the crossfade wins while it exists. The stored fade value is kept and becomes active again if the crossfade is removed.
- **Reversed clip:** the fade-in stays at the clip's timeline start.
- **1-frame clip:** fades are allowed but clamped as above.
- **Fades on a video clip's embedded audio:** supported. Picture fades are separate (effects-transitions-animations.md).
**Undo:** `SetFade { clip, edge: In | Out, old, new }`.
**Model notes:** `Fade { len: Tick, curve: FadeCurve }` in `ClipAudio`, with clamping helpers in `freecut-timeline` and evaluation in `freecut-audio`.
- mvp model (FreeCut default): until curves arrive in v1, `ClipAudio` holds the lengths only (`fade_in`, `fade_out: Tick`). Negative lengths are refused. Lengths are stored as given, not snapped to frames and not limited to the clip, and `ClipAudio::effective_fades` applies the clamping above.
- mvp inspector (FreeCut defaults, app): the fade fields step in 0.1 s and round typed values to 0.1 s. While editing, each field stops at the clip's duration minus the other edge's stored fade, so it can't push the pair past the clip; a stored value already past that limit (after a trim) is shown as stored and isn't changed until edited. Frame snapping and the sub-frame modifier come with the on-clip handles.
- v1 model (FreeCut defaults): each edge keeps its curve next to its length, `ClipAudio.fade_in_curve` and `fade_out_curve: FadeCurve { Linear (default), Smooth, Fast, Slow }`, set by `SetClipAudio`'s `FadeInCurve` and `FadeOutCurve` like the lengths (one undo entry for the selection, no entry when nothing changes). A split keeps each curve on the edge that keeps its fade; the new inner edges get no fade and the default curve. Restore takes the fade-out and its curve from the linked clip that ends last (TL-030).
- v1 app (FreeCut defaults): each fade field in the inspector has a curve dropdown beside it (Linear, Smooth, Fast, Slow); choosing one is its own undo entry. The section's Reset puts both curves back to Linear with the other defaults.

### AU-007 Clip mute — mvp
**What the user sees:** A speaker toggle in the inspector's audio header, and a speaker icon that appears when hovering over a clip. A muted clip's waveform turns grey and shows a "muted" badge.
**Inputs / controls:** Toggle. Works on the whole selection. The shortcut is owned by workspace-and-shortcuts.md.
**Behavior / output:** Silences the clip without touching its volume. Unmuting brings back exactly the previous level, keyframes and fades. This is the non-destructive form of the clip volume control. Muting every video track at once, and muting the whole project, are AU-039. Reference: one public guide notes that pulling the reference editor's volume slider down to zero discards the earlier level, while a mute toggle keeps it. The observed UI labels show a per-clip mute of a video clip's original sound, with a marker on the muted clip.
**Edge cases:**
- Muting a video clip silences only its sound.
- A muted clip on a soloed track stays silent.
- Mute is disabled on video clips whose audio is detached.
- A mixed selection toggles every clip to the same new state: if any clip is unmuted, all become muted.
- A disabled clip (TL-036) is silent whatever its mute state, and the mute state is kept.
**Undo:** `SetClipMuted { clips, old_states, new }`. The batch is one entry.
**Model notes:** `ClipAudio.muted: bool`.
- mvp UI (FreeCut defaults, app): the toggle sits in the inspector's Audio section for one clip, as a Mute audio / Unmute audio button for several, and in the clip context menu (UI-035). A selection where no clip plays its own sound shows it disabled, with "This clip has no sound", or, for a video whose sound was detached, a pointer to mute the audio clip instead. The muted clip's waveform turns grey and the clip shows a "muted" badge.

### AU-008 Volume keyframes and the on-clip volume line — v1
**What the user sees:**
- A keyframe diamond next to Volume in the inspector, with previous/next keyframe arrows.
- On the clip, the volume line (AU-009) shows dots for keyframes.
- Dragging the line between two dots moves that whole segment up or down. With no keyframes, it moves the static volume.
- Alt/Option-clicking the line adds a keyframe. Dots can be dragged in time and level. Delete removes the selected dots.
**Inputs / controls:**
- Level range is the same as AU-005.
- Time placement, interpolation choices (linear default, hold, eased) and keyframe selection are defined in clip-transform-and-speed.md (TR-027 and related).
- Gain interpolates in the dB domain. A segment that touches −∞ interpolates in linear amplitude, so it doesn't jump.
**Behavior / output:**
- When at least one keyframe exists, the envelope replaces the static volume. Fades and ducking still multiply on top.
- This is the manual way to duck music under speech (AU-028 automates it).
- Reference: public guides for the reference desktop editor describe manual ducking with separate tracks and volume keyframes.
**Edge cases:**
- **Keyframes outside the trimmed range:** kept, as TR-027 specifies. They still shape the envelope at the clip edge, and they become visible again when the clip is extended.
- **Split:** follows TR-027. Keyframes are source-anchored, both pieces keep all of the keyframes, and no boundary keyframe is inserted at the cut. Because each piece evaluates the same source-anchored envelope, the audible result is unchanged.
- **Speed change:** keyframes are source-anchored (TR-027), so a dip stays on the same words when the clip is retimed.
- **One keyframe:** acts as a constant.
- **Very dense keyframes** (more than one per 10 ms): the renderer smooths between them to avoid clicks.
- **Dragging a dot past its neighbour:** it clamps and cannot reorder.
- **Locked track:** the clip can't be selected (TL-009). The line is visible but cannot be edited.
**Undo:** The add, move, delete and set-value keyframe ops from clip-transform-and-speed.md. Dragging a segment is one entry.
**Model notes:** `ClipAudio.gain: Animatable<GainDb>` (TR-027's generic container). `GainDb` implements TR's `Interpolate` trait with the dB-domain and −∞ rules above. Split and trim use TR-027's keyframe helpers, called from TL-020 and TL-023, so AU adds no split logic of its own. Evaluation happens in `freecut-audio` at sample resolution.
- v1 engine (FreeCut default, `freecut-audio`): when a clip's keyframes change while it plays (the volume line dragged, a key typed), the mixer holds the volume heard at that sample (static volume times envelope, a crossfade in progress included) and crossfades from it to the new volume over 10 ms. This works all the way down to −60 dB and −∞, so dragging the line to the bottom during playback never clicks, and a new change during a crossfade starts from what is heard. Seeks and clip starts play the new envelope exactly.
- v1 model (FreeCut defaults, `freecut-timeline`): `ClipAudio.gain` is TR-027's `Animatable<GainDb>` with linear and hold segments so far. `ClipAudio::gain_at(source_time)` is the read-only value for read-outs and the volume line; on a segment touching −∞ it returns the nearer key (silence before the segment's midpoint, the other key from it on, where the true linear-amplitude ramp is within 6.02 dB), and `freecut-audio` evaluates such segments exactly from `Animatable::segment_at`. Between finite keys it interpolates in dB, rounded to 0.01 dB, halves to even. The `EditGainKeys { seq, clips, edit }` request applies one `GainKeyEdit` to every listed clip with sound as one undo entry: `Set { at, gain }` adds a key or sets the one there (`gain: None` takes the clip's current volume there, so the first key captures the static volume), `Interp`, `Move { from, to }` (clamped strictly between the neighbours, never reordering), `Remove(times)` and `Clear`. When the last key goes, the static volume becomes the earliest removed key's value (`Clear`: the first key's). Clips without the key an edit needs are skipped, and the edit is refused (`NoKeyframe`) when none has it. A static `SetClipAudio` volume skips keyframed clips and is refused (`VolumeKeyframed`) when every clip with sound has keys; AU-036's step that shifts the whole envelope comes later. Key times are the listed clips' source times; the app converts the playhead with `Clip::source_time_at`. Keys are unique by exact tick; the architecture's ±1-tick snap and 3-tick spacing come with TR-028's UI.
- v1 app (FreeCut defaults, `freecut-app`): the inspector's Volume row has the volume slider, a keyframe diamond and ⏴ / ⏵ arrows, with the dB field under them. The diamond is hollow, filled while the playhead sits on a key; a click adds a key at the playhead's source time with the volume there (`Set { gain: None }`, so the first key captures the static volume) or removes the key it sits on. It is disabled, with the reason, while the playhead is off the clip (the clip's end counts as on it). The arrows move the playhead to the previous / next key inside the clip's trimmed range. With keys, the slider and field show the value at the playhead (at the nearer clip edge when it is off the clip) and editing them sets or creates the key there (TR-027), one undo entry per drag; off the clip they are disabled. Under them, "N keyframes · value at the playhead" and a "Remove all" button (`Clear`). The audio section's Reset also removes keyframes, as part of its one undo entry. On the timeline, the volume line and dots of a *selected* clip on an unlocked track can be edited (so a press on an unselected clip still moves it): dragging the line moves the static volume, or the segment under the pointer (the two keys around it; before the first or after the last key, that key alone), by the same travel as the volume slider; Alt/Option-click on the line adds a key on the nearest frame with the line's value there, selected; a click on a dot selects it (Shift-click adds or removes it), and a dot drags in time (frame-snapped, kept inside the clip and strictly between its neighbours by the model) and in level at once. Each drag is one undo entry, and a tooltip next to the pointer shows the level being set. Delete / Backspace removes the selected dots instead of the clip; any other click clears the dot selection. Dots hit within 7 px and the line within 3–6 px (by band height), so both stay usable on small tracks; the line needs a band of at least 6 px.

### AU-009 Gain overlay on waveforms — mvp
**What the user sees:**
- The waveforms themselves (on audio clips and the audio band of video clips) come from timeline-editing.md (TL-042) and show the source signal.
- On top of them, AU draws a thin volume line and shaded fade ramps at the clip ends, so you can see what you will hear.
- A muted clip's waveform turns grey (AU-007).
- v1: keyframe dots (AU-008), the duck dip (AU-028), and a warning tint on regions where gain pushes peaks past 0 dBFS.
**Inputs / controls:**
- TL-042's "Show waveforms" preference, track height (TL-010) and timeline zoom (TL-041).
- Display options (v1, FreeCut defaults): linear amplitude (default) or dB scale; stereo shown as combined peaks (default) or split left/right lanes.
- **Waveform height** (v1, Preferences next to TL-042's "Show waveforms"): 30 %, 60 % or 100 % of the clip's waveform band. Default 100 % (FreeCut default). A lower setting leaves room to read the volume line and keyframe dots on short tracks. The observed reference app offers the same three steps.
**Behavior / output:**
- The waveform is mapped through the clip's retime map (TR), so it is squeezed or stretched by speed and mirrored by reverse.
- The overlay line is the gain envelope (volume × keyframes × fades × duck), evaluated from the model at one value per pixel column. It needs no decoding.
- Cleanup and effects are not reflected in the drawing (FreeCut default).
**Edge cases:**
- **Clip at very high speed (100×):** peak bins are aggregated, never under-sampled, so short transients still show.
- **Silent clip:** a flat line, not a placeholder.
- **Media offline or peaks not ready:** TL-042's placeholder. The overlay line is still drawn, because it comes from the model.
- **Peaks can't be computed** (the audio stream fails to decode, or its codec is unsupported): the band shows a "Waveform unavailable" hatch, distinct from the offline pattern, with the reason in its tooltip. The overlay line is still drawn. If playback can't decode the stream either, the clip plays silence and PM-037's error names the file.
- **Volume at −∞:** the line sits on the clip's bottom edge.
- **Multi-stream file:** the waveform of the selected stream is shown.
**Undo:** n/a (view state and derived display).
**Model notes:** Peak data comes from TL-042's `PeakRequest` in `freecut-media`. `freecut-timeline` provides a pure `gain_envelope_at(clip, t) -> GainDb` helper (v1 form: `Clip::source_time_at(t)` then `ClipAudio::gain_at(s)`, the volume and keyframes; fades and duck are applied on top by the app). `freecut-app` converts it to pixels (floats only there) and never decodes on the UI thread.
- v1 app (FreeCut defaults, `freecut-app`): every clip that plays its own sound draws a thin yellow line over its waveform band (below the name on audio clips, in the sound strip of video clips): volume and keyframes times the fades with their curves, one point per two pixel columns, on the volume slider's scale stood upright (0 dB a quarter of the way down, +20 dB at the top, −∞ on the bottom edge). A segment touching −∞ is drawn in linear amplitude, as it plays. The fade ramps are shaded above the line. A muted clip gets a grey band and a grey line. Keyframe dots inside the trimmed range sit on the line; selected ones are larger and orange. The line is drawn while peaks are pending, offline or failed. The waveform is still the media's default stream's (the selected stream's waveform comes later).

### AU-010 Mixing engine, master bus and playback output — mvp
**What the user sees:**
- Sound plays in sync with the preview.
- A master volume control sits next to the player. The master meter beside it arrives in v1 (AU-015).
- An audio-device picker and buffer-size setting are in Preferences.
- A banner appears if no output device is available.
**Inputs / controls:**
- **Master volume:** −∞, then −60.0 to +12.0 dB. Default 0.0 dB.
- **Safety limiter:** on by default. Ceiling −1.0 dBFS sample peak. About 5 ms look-ahead.
- **Output device:** defaults to the system default.
- **Buffer:** 256 to 4096 frames. Default 1024 frames (about 21 ms at 48 kHz) (FreeCut default).
- **Project sample rate:** owned by project-and-media.md. 48 kHz is the FreeCut default, and 44.1 kHz is allowed.
**Behavior / output:**
- Every audible clip is processed through the per-clip chain, then the track stage, then summed on a 32-bit float stereo master bus.
- Preview and export use the same mixing code. Export renders offline and faster than real time.
- Sources are resampled to the project rate (high-quality sinc resampler) and downmixed to stereo. More than 2 channels uses standard ITU downmix coefficients: centre and surrounds at −3 dB, LFE dropped. Mono sources are duplicated to both channels at unity gain.
  - v1 decode (FreeCut defaults, `freecut-media`): the stereo matrix is FreeCut's own, set on libswresample (whose default lowers mono by 3 dB and normalises downmixes). Speakers come from the stream's channel layout, else FFmpeg's default layout for the count: front left/right (also left/right of centre, wide, Lt/Rt, binaural) at unity on their side; centre and other centred channels at −3 dB on both; surrounds and height channels at −3 dB on their side; LFE dropped; no normalisation, so the limiter catches any overs. A one-channel stream is mono whatever its label. An unlabelled stream (no channel layout, such as a plain-PCM multitrack WAV from a field recorder) follows FFmpeg's default layout for its count except that nothing is dropped: the channel that layout calls LFE (the 3rd of 3, the 4th of 6 or 8) may be a real track, so it goes to both sides at −3 dB like an unknown channel. Media info shows such streams as "N ch" (no layout name).
- The equal-power crossfade law (AU-029) ships with the mvp mixer, because FX-007 (mvp) uses it for embedded audio under video transitions.
- Sample positions come exactly from rational time: `sample = floor(ticks × sample_rate / ticks_per_second)`, computed in `i128` so long timelines can't overflow. TL-001's tick rate divides evenly by every common sample rate, so the result is exact.
- During playback, the audio output clock is the master clock for A/V sync, with the device's reported output latency compensated. The player itself belongs to preview-and-export.md.
- The real-time callback never blocks, allocates or decodes. Decoding happens on worker threads in `freecut-media`, which read ahead into lock-free ring buffers.
**Edge cases:**
- **Buffer underrun:** silence is output for the missing block and an underrun counter increments. Audio is never repeated.
- **Output device unplugged during playback:** FreeCut switches to the system default and continues.
- **No device at all:** the video still plays, driven by a system clock, and a banner explains why and asks the user to connect one.
- **Chosen device busy** (held exclusively by another app) **or access denied by the OS:** FreeCut falls back to the system default device. The banner names the cause and suggests closing the other app or choosing another device, with a "Choose device…" button that opens UI-029. If no device opens, the no-device rule applies.
- **Device error or timeout while opening:** retried once after 1 s (FreeCut default), then handled like an unplugged device, with the banner saying the device didn't respond.
- **Nothing audible although playback runs:** FreeCut neither reads nor changes the system volume, because there is no portable way to do so through `cpal`. The banner's help text lists what to check: system volume, preview mute (EX-019), project mute (AU-039), track mute and solo. Observed alternative: the reference app detects a very low system volume and offers to raise it.
- **Device runs at a different rate from the project:** the output is resampled at the device boundary.
- **Seek or scrub:** buffers flush and refill. Target start latency is under 150 ms.
- **Overs:** if every track sums above full scale, the limiter prevents hard clipping. With the limiter off, the export clips hard and, in v1, the clip LED lights up (AU-015).
- **Offline media:** contributes silence.
- **Empty timeline:** silence.
**Undo:** Master volume and limiter on/off are saved and undoable (`SetMasterGain`, `SetLimiter`). Device and buffer settings are app preferences, not project edits.
**Model notes:**
- `freecut-audio` owns the mixer graph, DSP, limiter and the `cpal` output, depending on `freecut-media` (decoding) and `freecut-timeline` (the model).
- The mixer reads the sequence through TL's read-only `MixView` (TL-008): for any time range, which clips and attributes are active.
  - mvp form (FreeCut default): `freecut_timeline::mix::mix_sources(doc, seq, window)` lists, by track order then start, every clip overlapping the window that plays media with `Active` sound settings, isn't disabled or muted and isn't on a muted track, with the media item, its audio stream (the item's first), placement, source start and `ClipAudio`. Hidden tracks still sound (TL-008: hiding is a picture flag). A clip at −∞ volume is still listed, so the mixer can ramp to it.
  - v1 app wiring (FreeCut defaults, UI-018, UI-029): the app reads its preferences on the job pool at startup and then opens the output device they name (`EngineConfig { device, buffer_frames }`), off the window thread. A change in Preferences goes to `AudioEngine::reconfigure` at once; with no engine (no device opened), the app tries `AudioEngine::start` again with the new choice. A banner under the menu bar (and on Home) shows, from `AudioEngine::status` and `last_error`: no device at all ("No audio output device — playing without sound. Connect a device, or choose another one."), a lost device that couldn't be reopened, or a chosen device that isn't playing — `DeviceIssue`'s sentence, then "; playing on the system default (“name”) instead." (or "; trying it again…" while it waits for its retry), then advice by cause (close the other app; allow access in the system settings; connect it; choose another device). It has a "Choose device…" button that opens Preferences, and ✕ hides it until its text changes. The cause or the chosen device's name shows on hover.
  - mvp app wiring (FreeCut defaults): the app opens the default output device at startup, off the window thread. If that fails, the status line says "No audio output device — playing without sound", the player's speaker button is disabled with the cause on hover, and playback runs on the system clock. Otherwise playback is clocked by the time being heard (EX-002) and stops at the end of the sequence. The engine plays `freecut_audio::DocSchedule` over the root sequence; it gets a new schedule after every commit, undo and redo (so a track mute is heard within the ~200 ms read-ahead, AU-011), and a new file table (`FileOpener::from_paths`) only when the project's media paths or offline set change, so open readers survive edits. Moving the playhead during playback continues playing from the new time; frame steps and go-to-start/end pause. Opening another project stops playback and drops the old project's sound. The speaker button next to the transport is EX-019's preview mute: app state, never saved, and applied after the limiter. Beside it, a "Master" field shows the root sequence's master volume in dB; it is typed or dragged like AU-005's Volume field (dB, percent or "−inf"), values above +12 dB become +12 dB, a drag is one undo entry, and the engine gets the new level with the next schedule. Extracted audio items (AU-004) that aren't resolved yet use their parent's resolved file when both point at the same file.
  - mvp engine (FreeCut defaults, `freecut-audio`): `DocSchedule` turns each listed clip into one mixer input keyed by its clip id, so a clip keeps its open reader across moves and trims; it reads the clip's own library item at the listed stream. Volume converts as 10^(dB/20), with −∞ as 0. Fades are passed as stored and clamped by the mixer per AU-006; a fade-in's gain is taken at each sample's start and a fade-out's at its end, so the first and last samples of the ramps are exactly 0. Offline media gets no reader and plays silence. A feeder thread mixes about 200 ms ahead in 10 ms blocks into a lock-free ring; the device callback only takes frames from it. Preview mixes at the device's own rate (sources are resampled straight to it) rather than resampling the mix at the device boundary; export will mix at the project rate with the same mixer. Master gain is applied before the limiter, so a change is heard within the read-ahead. A lost device is reopened at once and again after 1 s; if both fail, the position keeps running on the system clock and the next play tries again. A new schedule during playback flushes nothing and restarts nothing: the queued audio plays out and the change is heard after the read-ahead (about 200 ms), with clip volume, clip mute and track mute changes ramped over 10 ms (AU-005, AU-011); only play and seek flush. The feeder never opens a file: a prefetch thread opens and positions the readers of the next 2 s of the schedule ahead of time; a clip whose reader is late plays silence and ramps in when it arrives, a new file table keeps each old reader playing until its replacement is ready, and play or seek waits at most 100 ms for the readers under the playhead. Silence while a reopened device waits for audio counts as an underrun; only the wait after play or seek doesn't. A device whose default sample format FreeCut can't write (24-bit packed, unsigned 32-bit, DSD) is opened in one it can, preferring F32, then I16, then I32.
  - v1 engine (FreeCut defaults, `freecut-audio`): the feeder's read-ahead is at least two device buffers plus a block (about 200 ms otherwise), so a narrowband output (8 or 16 kHz Bluetooth hands-free) with a 4096-frame buffer never runs dry; the buffer is the size the device was opened with, else the configured one. A clip whose source changes during playback (another stream picked, AU-038) keeps its old reader playing until the new stream's reader takes over at its position: a cut, never a gap. Device names are read with the device's description, and a device that can't say its name (one being unplugged) is skipped when listing or looking one up, instead of panicking the feeder thread.
- mvp master model (FreeCut defaults): `Sequence.master: MasterAudio { gain: GainDb }` (named `master` because `audio` already holds the audio track list) changes only through `SetMasterGain { seq, gain }`, which lowers gains above +12 dB to +12 dB and adds no entry when nothing changes. The file stores it as the sequence's `master: { "gain": … }`, spelled like a clip gain and left out at the defaults, so files that never changed it are written exactly as before; a master gain above +12 dB is refused on open. The limiter is always on until `SetLimiter` and its toggle land; the project mute and auto-fix records come with AU-039 and AU-042.
- Master settings live in the typed property-bag slot that TL-003 adds to the sequence, `Sequence.audio: MasterAudio { gain: GainDb, limiter: bool, muted: bool, auto_fix: Vec<AutoFixRecord> }`. `muted` is the project mute (AU-039) and `auto_fix` holds AU-042's records. AU defines the type, and it changes only through undoable ops (`SetMasterGain`, `SetLimiter`, `SetMasterMute`, and AU-042's apply and remove ops), which TL-043 records. `freecut-project` serializes it. A new sequence, including a compound clip's nested sequence (TL-040), starts at 0 dB with the limiter on, unmuted and with no auto-fix records.

### AU-011 Track mute in the mixer — mvp
**What the user sees:** The speaker toggle on audio-track and video-track headers. It is defined in timeline-editing.md (TL-008, mvp) and appears on the mvp track header (TL-004, UI-035). A muted track is silent in preview and export (EX-002 applies the same rule). On a video track it silences all embedded audio on that track. Muting the main track is FreeCut's way of silencing the original sound of every main-track clip at once, which the observed reference app offers as a separate toggle. AU-039 adds one command for every video track.
**Inputs / controls:** As TL-008 (click, and Alt-click to mute every other audio-bearing track). The shortcut is owned by workspace-and-shortcuts.md.
**Behavior / output:**
- The mixer skips muted tracks at step 14 of the chain, without changing any clip.
- Read-ahead decoding pauses for clips on muted tracks, to save CPU, and restarts when the track is unmuted.
- Toggling during playback ramps over about 10 ms to avoid clicks.
**Edge cases:**
- **Muted and soloed track:** stays silent.
- **Muted speech track used as a ducking source (AU-028):** still analysed (FreeCut default). The analysis respects clip mute only.
- **Locked track:** whether the toggle stays usable is TL's rule (TL-008, TL-009).
- **Deleting a muted track and then undoing:** the mute state comes back with the track (TL).
**Undo:** TL-008's `SetTrackFlags { track, before, after }`. It is persisted and undoable, and it ships in the mvp.
**Model notes:** The flag is `TrackFlags.muted` (TL-004, TL-008). It is not part of AU's `Track.audio` slot. `freecut-audio` reads it through `MixView`.

### AU-012 Track solo — v1
**What the user sees:** An "S" toggle on audio-bearing track headers and mixer strips. When any track is soloed, only soloed tracks are heard.
**Inputs / controls:** A click solos only that track. Shift-click adds or removes it from the soloed set.
**Behavior / output:**
- Solo is a monitoring aid. It affects preview playback only.
- Export ignores solo by default. If solo is active, the export dialog shows a notice with an "Export as heard" option (FreeCut default; coordinate with preview-and-export.md).
- Solo on a video track covers its embedded audio.
**Edge cases:**
- **Soloed track that is also muted:** it is silent.
- **Removing the soloed track:** solo state is cleared.
- **Scrubbing:** follows solo.
**Undo:** n/a (view state). The solo set is kept in the project's UI state, so it survives reopening, but is not on the undo stack.
**Model notes:** The solo set lives in app view state. `freecut-audio` takes it as a playback parameter.
- v1 engine (FreeCut defaults, `freecut-audio`): `DocSchedule::with_solo(Option<BTreeSet<TrackId>>)` keeps only the soloed tracks' sources; a muted track stays silent because the mix view never lists it. An empty set, or one naming no track of the sequence (a soloed track that was removed), counts as nothing soloed, so a stale solo set never silences the timeline. `Schedule::export_inputs` is the view without solo, and `OfflineMix` renders it unless `set_export_as_heard(true)` ("Export as heard").
- v1 app (FreeCut defaults, `freecut-app`): the "S" toggle sits between M and L on video and audio track headers, and on each mixer strip. A click on the only soloed track un-solos it; a click on any other track solos only that one; Shift-click adds or removes it. The set is `freecut_project::UiState.solo`, held by the open project: changing it adds no undo entry but marks the project modified (as EX-021's export settings do), so the next save keeps it. After every commit, undo and redo, ids that are no longer sound-carrying tracks are dropped, so an undo that brings a removed track back doesn't bring its solo back. Playback gets it through `DocSchedule::with_solo`, with the schedule rebuilt whenever the set changes. While any track is soloed, the export dialog's audio section shows "Solo is on (track names). The export includes every track." and an "Export as heard (only the soloed tracks)" checkbox, off each time the dialog opens and not saved with the export settings; when ticked, the export job passes the set to `DocSchedule::with_solo` and calls `OfflineMix::set_export_as_heard(true)`.

### AU-013 Track volume, pan and mixer panel — v1
**What the user sees:** A dockable mixer panel (the layout is workspace-and-shortcuts.md's job). It has one strip per audio-bearing track plus a master strip. Each strip shows a fader, pan knob, mute (TL-008), solo, a meter and the track name. Track headers also show a compact volume readout.
**Inputs / controls:**
- **Fader:** −∞, then −60.0 to +12.0 dB. Default 0.0 dB.
- **Pan:** −100 to +100. Default 0.
- Double-click resets either control. Typed values are accepted.
**Behavior / output:**
- The track stage comes after all clip processing.
- Pan uses a balance law: at centre both channels are at unity. Moving towards one side lowers the other channel, reaching silence at ±100. This applies to mono-origin material too. (FreeCut default; a constant-power option comes later.)
**Edge cases:**
- **Video track with all its audio detached:** the strip still exists but shows as idle.
- **New track:** starts at 0 dB and centre pan.
- **Track keyframes (automation):** later, not v1.
**Undo:** `SetTrackGain { track, old, new }` and `SetTrackPan { track, old, new }`. A drag is one entry (TL-043 coalescing).
**Model notes:** `TrackAudio { gain: GainDb, pan: Pan }` in `freecut-timeline` is stored in the typed property-bag slot that TL-004 adds to every track, `Track.audio`. AU defines the type and its ops. The slot holds defaults on track kinds that carry no sound (Text, Sticker, Effect, Adjustment) and is ignored there. Mute and hide stay in TL's `TrackFlags` (TL-008).
- v1 model (FreeCut defaults): the slot is `Track.mix: TrackAudio` (named `mix` because `audio` reads as the sequence's audio track list), `Pan(i8)` clamped to −100..=100. `SetTrackGain { seq, track, gain }` lowers gains above +12 dB to +12 dB; `SetTrackPan { seq, track, pan }`. Each adds no entry when nothing changes, is refused (`TrackHasNoSound`) on Text, Sticker, Effect and Adjustment tracks, and, like mute (TL-008), works on locked tracks, because a lock protects clips. Removing a track and undoing brings its stage back. `Track::from_parts` starts at the defaults and a loader sets `mix` afterward. `mix_sources` hands each source its track's stage (`MixSource.track_audio`), so the audio crate applies it at chain step 14. Solo is never model state (AU-012).
- v1 app (FreeCut defaults, `freecut-app`): View › Mixer shows the mixer as a resizable panel above the timeline. Strips follow the timeline's order: video tracks top to bottom, then audio tracks top to bottom; text, sticker, effect and adjustment tracks have none. Each strip shows the track name, "idle" when no clip on it plays sound, a vertical fader with a dB field under it (typed values accept dB, a percentage or "−inf", like AU-005's Volume field; above +12 dB becomes +12 dB), a pan slider with a field reading "L30", "C" or "R30" (typed "L30", "R 30", "−30" or "30"), and M and S buttons. The fader puts −60 dB at 2 % of its travel, 0 dB at 75 % and +12 dB at the top, quadratic below 0 dB like the clip Volume slider, on a 0.1 dB grid; the bottom 2 % is −∞. Double-click resets the fader to 0 dB and the pan to centre. A drag is one undo entry; M is TL-008's track mute. The master strip has the fader only and edits `Sequence.master` (`SetMasterGain`). The meter waits for AU-015. Audio track headers always show the track volume ("−6.0 dB", with "L30"/"R30" after it when panned); video track headers show it once it differs from 0 dB and centre.

### AU-014 Clip pan and channel mapping — v1
**What the user sees:** In the inspector's audio section, a Pan slider and a Channels dropdown.
**Inputs / controls:**
- **Pan:** −100 to +100. Default 0. Same balance law as AU-013.
- **Channels:** Stereo (default), Mono mix (L+R averaged, sent to both sides), Left only (left sent to both sides, which fills the right side from the left), Right only (right sent to both sides), Fill from the side with sound (auto), Swap L/R.
- **Multi-select:** the chosen mode is set on every selected clip with stereo sound. Mono clips in the selection are skipped (AU-040's batch rule).
**Behavior / output:**
- Multichannel sources are first downmixed to stereo (AU-010), then the channel mode applies.
- The typical fix this enables is a camera recording where one lavalier mic is on the left channel only.
- **Fill from the side with sound:** a quick background analysis (an AI-001 job, cached per media and stream like beat analysis) measures each channel's RMS level over the whole stream. A side counts as silent when it is at least 30 dB below the other side and below −50 dBFS (FreeCut defaults). The side with sound is then sent to both sides. If both sides have sound, the clip plays as Stereo and the dropdown reads "Auto: both sides have sound". If both are silent, it plays as Stereo. Until the analysis is ready, the clip plays as Stereo with "Analysing…".
- Reference: the observed UI labels show filling one channel from the other in both directions, an automatic choice of the side with sound, and a left / right / both routing choice for stereo sources. FreeCut's Left only, Right only, auto fill and Stereo modes cover them.
**Edge cases:**
- **Mono source:** the Channels dropdown is disabled with the reason "Source is mono" (the observed reference app also blocks the option for mono). A channel map that arrives on a mono clip through Paste attributes (TL-034) or detach (AU-003) is stored but has no effect.
- **Out-of-phase stereo in Mono mix:** partly cancels. No automatic fix (a phase warning comes later).
- **Auto fill on a recording where one side only drops out now and then:** the analysis looks at the whole stream, so both sides count as having sound and nothing changes. The user picks Left only or Right only by hand.
- **Media relinked or changed on disk:** the auto-fill analysis is redone (AI-001's stale rule), and the decision may change.
**Undo:** `SetClipPan` and `SetChannelMap { clips, old: Vec<ChannelMap>, new }`. Pan keyframes use TR-027's keyframe ops. The auto-fill decision is a derived cache, not on the undo stack.
**Model notes:** `ClipAudio.pan: Animatable<Pan>`. Pan is one of the audio properties registered on TR-027's keyframe engine, so it can take keyframes like volume (AU-008), with the same source anchoring and split rule. `ChannelMap { Stereo, MonoMix, LeftOnly, RightOnly, AutoFill, Swap }` enum in `freecut-timeline`. The channel-level analysis and the matrix maths live in `freecut-audio`. The analysis result is a small AI-001 `AnalysisRecord` stored in the project file. Its kind, ChannelBalance, is a request to AI-001, which owns the list of job kinds.

### AU-015 Level meters and loudness readout — v1
**What the user sees:**
- A stereo master meter beside the preview. It shows a peak bar and an RMS bar on a −60 to 0 dBFS scale, a peak-hold tick, and a clip LED.
- Per-track meters in the mixer.
- A small loudness readout: momentary, short-term and integrated LUFS since playback started, plus true peak, with a reset button.
- A limiter activity indicator.
**Inputs / controls:**
- Click the clip LED to reset it.
- Click the integrated readout to reset it.
- Meter ballistics (FreeCut defaults): peak hold 1.5 s, fall 20 dB/s. The clip LED lights at ≥ −0.1 dBFS measured before the limiter and stays lit until reset.
**Behavior / output:** Meters read the live playback stream. Loudness is computed per EBU R128 using the `ebur128` crate.
**Edge cases:**
- Integrated loudness over a span shorter than 400 ms shows "—".
- Meters freeze at their last values when playback is paused, and decay after 1 s.
- Soloing changes what the meters show.
**Undo:** n/a (view state).
**Model notes:** `freecut-audio` publishes meter data via a lock-free snapshot that the UI polls. `freecut-app` draws it.

### AU-016 Audio scrubbing — v1
**What the user sees:** Dragging the playhead or a clip edge plays short snippets of sound at the cursor. Shuttle playback at non-1× rates (preview-and-export.md) plays sped-up or slowed-down sound.
**Inputs / controls:** An on/off toggle (default **on**, FreeCut default). The shortcut is owned by workspace-and-shortcuts.md.
**Behavior / output:**
- Each scrub step plays a 40 to 80 ms grain at the playhead position, with short crossfades between grains.
- Shuttle at 2× to 8× uses the time-stretch engine (AU-023), keeping pitch.
- Scrubbing uses the full clip chain only where cached results exist. Otherwise it plays the source with volume, fades and mute only, to stay responsive.
**Edge cases:**
- Scrubbing over offline media is silent.
- Very fast drags drop grains rather than queue them.
- Solo and mute are respected.
**Undo:** n/a (view state).
**Model notes:** `freecut-audio` provides a scrub voice that uses the same decode prefetch.

### AU-017 Loudness normalization — v1
**What the user sees:** An inspector toggle, "Normalize to target", with a target dropdown. After analysis, the clip shows its measured loudness and the gain being applied, for example "−27.4 LUFS → +11.4 dB". A small badge marks normalized clips on the timeline.
**Inputs / controls:**
- **Target presets:** −12 LUFS (loud, short-form social video), −14 LUFS (streaming and long-form platforms), **−16 LUFS (default, FreeCut default)**, −23 LUFS (broadcast, EBU R128), or custom from −40 to −5 LUFS in 0.1 steps. Observed: the reference app offers −23 LUFS (its default), −14 and −12 LUFS presets. FreeCut keeps −16 as its default because most FreeCut exports go to online platforms, where −23 plays noticeably quiet. −23 stays one click away.
- **Several clips with different targets selected:** the target field shows "Mixed" (the observed reference app shows the same state). Picking a value sets it on all of them.
- **True-peak ceiling:** −1.0 dBTP (fixed in v1).
- **Mode for multi-selection:** Each clip independently (default), or As a group, which measures the selection together and applies one shared gain so their balance is preserved.
**Behavior / output:**
- A background job measures the integrated loudness and true peak of the clip's processed signal through step 7 of the chain (after cleanup, retime and voice effect).
- The applied gain is `target − measured`, then reduced if needed so that `true_peak + gain ≤ ceiling`, then clamped to the −30 to +24 dB range.
- The gain is applied at step 8, ahead of the user's volume. Volume still works as a trim on top.
- The source file is never modified.
- Reference: the reference editor offers a per-clip normalization toggle in its audio controls, and its public pages state a fixed target of −23 LUFS.
**Edge cases:**
- **Analysis not finished:** preview plays without the gain and shows "Analyzing…". Export waits for analysis before it starts (with progress).
- **Silent or near-silent clip** (integrated loudness below −70 LUFS, the gate floor): no gain is applied, and the toggle shows "Too quiet to normalize". For a source that AU-040 already flags as silent, the toggle is disabled with that reason before any analysis runs (the observed reference app disables the option with an explanation).
- **Compound clip (TL-040):** not available in v1 (AU-040). The hint suggests normalizing the clips inside. The observed reference app refuses compound clips too.
- **Progress:** while several clips are analysed, AI-001's progress strip shows on each one, and the inspector reads "Analyzing 2 of 5…". When done, each clip shows the target it was set to.
- **Target cannot be reached without breaking the peak ceiling:** the clip is set to the highest gain that keeps the ceiling, and a notice shows the shortfall.
- **Clip trimmed, retimed, or its cleanup changed:** the measurement is invalidated and recomputed automatically.
- **Very short clip** (under 400 ms): the measurement falls back to short-term loudness.
- **Normalization combined with ducking or keyframes:** the measurement covers the clip's own signal only, not envelopes.
**Undo:** `SetNormalize { clip, old: Option<NormalizeSettings>, new }`. The measured value is a derived cache, so it is not on the undo stack.
**Model notes:**
- The model stores only the toggle, target and mode.
- `freecut-audio` runs the analysis with `ebur128` (MIT) on frames decoded by `freecut-media`.
- The cache key is media fingerprint + stream + source range + upstream settings hash.
- `freecut-project` may persist the cached measurement so reopening doesn't trigger re-analysis.

### AU-018 Noise reduction — v1
**What the user sees:** An inspector toggle, "Noise reduction", with a Strength slider. A hint says it is designed for steady background noise under speech.
**Inputs / controls:** On/off (default off). **Strength 0 to 100 %, default 60 %** (FreeCut default). Strength sets the maximum suppression: 0 % is a bypass and 100 % allows up to 30 dB of reduction (FreeCut default mapping).
**Behavior / output:**
- Suppresses steady background noise such as fan or air-conditioning hum, hiss and room tone.
- This is classic signal processing written by FreeCut in Rust, with no ML model: short-time spectral analysis, a per-band noise-floor estimate that follows the quietest recent level (minimum-statistics style), and a smoothed per-band suppression gain (Wiener-style). All of these are published textbook methods.
- Runs in source time (chain step 4), so its output can be cached per media range and settings.
- Runs in real time. The analysis window latency (about 20 to 40 ms) is compensated so lip sync is unaffected.
- This control never uses an ML model, and it has no ML mode. For hard cases the separate Voice clarity control (AU-020) runs before this stage, using AI-024's engine. Room echo is AU-037's job.
- The before/after comparison is AU-041's Compare button.
- Multi-select: the toggle and strength apply to every selected clip with sound, as one entry (AU-040's batch rule).
- Reference: public pages for the reference editor describe a one-click noise toggle in the clip audio controls, with an intensity control on some versions. Its observed UI labels show the noise removal running as a background job (waiting, processing, done, cancelled, or failed with retry) with a before/after player, and a success toast. FreeCut processes in real time instead, so there is no wait in the normal case. Only the pre-render fallback below runs as a job, with those same states.
**Edge cases:**
- **Applied to music:** music is mostly left alone, but high strength can soften sustained notes or cymbals. The hint suggests lower strength.
- **Changing noise** (wind, traffic, clatter): only partly reduced. This is a documented limitation, and the hint points to Voice clarity (AU-020).
- **High strength artifacts** ("musical noise", a faint warbling): limited by gain smoothing. The hint suggests lowering strength.
- **Noise floor at the clip start:** the tracker needs about 1 s to settle, so it pre-rolls on up to 2 s of handle media before the in-point when available.
- **Stacked with voice polish:** voices can sound dull. The UI suggests trying each control alone first.
- **Clip shorter than one analysis window, or silent input:** passes through unchanged.
- **Real-time budget exceeded on a slow CPU with many processed clips:** the engine pre-renders those clips to the processed-audio cache in the background and plays the cache. Export always renders at full quality. The pre-render shows in PM-073's task list as Waiting, Processing n %, Done, Cancelled or Failed (with Retry). Until it is done, the clip plays unprocessed with AI-001's badge.
- **Pre-render cancelled or failed:** the setting stays on and the clip keeps playing unprocessed in preview. Export still renders the effect in its own offline pass, so it never needs the cache.
**Undo:** `SetNoiseReduction { clips, old: Vec<Option<Strength>>, new: Option<Strength> }`. A strength drag coalesces (TL-043).
**Model notes:**
- The model holds `ClipAudio.denoise: Option<Strength>`.
- The DSP lives in `freecut-audio` and is tested with fixture signals (tones plus noise at known levels).
- The processed-audio cache (a proxy-like rendered WAV keyed by media, range and settings) is written by `freecut-media`, with its location under project-and-media.md's cache rules.

### AU-019 Voice polish — v1
**What the user sees:** An inspector toggle, "Voice polish", with a Strength slider. The goal is clearer, fuller, more present speech.
**Inputs / controls:** On/off (default off). **Strength 0 to 100 %, default 60 %** (FreeCut default).
**Behavior / output:** A fixed DSP chain, scaled by strength:
- High-pass at about 80 Hz to remove rumble.
- A plosive softener: a fast ducker on the band below about 200 Hz that triggers on sudden low-frequency bursts, taming "p" and "b" pops without thinning the voice.
- A mouth-click reducer: very short (under about 5 ms) broadband spikes that stand out from their surroundings are detected and smoothed over by interpolation.
- A gentle downward expander to lower breaths and room tone between phrases.
- A presence lift of about +1 to +4 dB around 3 to 5 kHz.
- A de-esser around 5 to 8 kHz.
- A 3:1 compressor with automatic threshold.
- Make-up gain matched to the input loudness, so toggling the effect does not simply sound louder (FreeCut default).

The ML option is Voice clarity (AU-020, using AI-024's engine). Echo is handled by AU-037, not here. Reference: the reference editor places a voice-enhancing toggle with an intensity control next to its noise and normalization controls, and some sources say it requires a paid tier. Its observed UI labels describe that enhancement as removing echo, plosive pops, mouth clicks and other noise, processed in a server queue. FreeCut covers the same jobs locally and free: plosives and clicks here, echo in AU-037, noise in AU-018.
**Edge cases:**
- **Applied to music or sound effects:** allowed, but the hint discourages it.
- **Already heavily compressed source:** low strength is recommended. No automatic detection.
- **Clipping after the presence boost:** prevented by the internal ceiling.
- **Mono or stereo:** processed with linked detection, so the stereo image is preserved.
- **Sung vocals or percussive speech:** the plosive softener can dull hard consonants or kick-like sounds. Low strength scales it down along with the rest of the chain.
- **Clicks that are part of the content** (a pen click, a finger snap): can be softened. Compare (AU-041) lets the user check.
**Undo:** `SetVoicePolish { clips, old: Vec<Option<Strength>>, new: Option<Strength> }`. A strength drag coalesces (TL-043).
**Model notes:** `ClipAudio.polish: Option<Strength>`. The DSP is written in `freecut-audio`, in Rust, and tested with fixture signals (sine sweeps and noise bursts).

### AU-020 Voice clarity (ML speech enhancement) — later
**What the user sees:** A "Voice clarity" switch with a Strength slider in the inspector's audio section, below Noise reduction and Voice polish. It is meant for speech recorded in noisy or echoey rooms, which classic noise reduction (AU-018) cannot fix. The same switch appears in the voiceover recording panel (AU-027), so new takes can get it automatically. While the clip is being processed, it shows a progress strip. Afterwards it shows a small "processed" badge (the strip and badge styles are AI-024's).
**Inputs / controls:**
- **Voice clarity:** on/off, default off.
- **Strength:** 0 to 100 %, **default 70 %** (FreeCut default). AI-024 defines how strength maps to the model's maximum attenuation. Strength 0 is a bypass.
- Multi-select: the switch and strength apply to every selected clip with sound.
**Behavior / output:**
- AU owns the control and the stored value. AI-024 owns only the engine: the DeepFilterNet model, the strength mapping, the install prompt and the cache keys.
- Turning the switch on, or changing strength, asks AI-024's engine to process the clip's source range (with AI-001's handles) in a background job on AI-001's scheduler. The result is written to the processed-audio cache that AU-018 also uses.
- At chain step 3, AU plays the cached processed audio in place of the decoded signal. Noise reduction, voice polish, retime and every later stage then run on top as usual.
- There is no real-time mode. Until the processed audio is ready, preview plays the unprocessed audio with a "Processing…" badge. Export waits for it, with progress.
- With the switch on and strength 0, no job runs and the stage is bypassed.
- Reference: public pages for the reference editor describe a one-click voice-enhancing option among its clip audio controls. Its observed UI labels show it as a server-queued, paid feature with an intensity slider. FreeCut's version runs locally, and the model is an optional install.
**Edge cases:**
- **Model not installed:** the switch is still shown. Turning it on opens AI-002's install prompt (raised by AI-024). Cancelling leaves the switch off and changes nothing.
- **Heavy reverb:** limited improvement.
- **Music or singing input:** damaged, as AI-024 warns. The hint says so.
- **Clip retimed:** nothing to redo, because the processing happens in source time, before the retime stage.
- **Clip trimmed longer:** only the new part is processed (AI-001), and the badge shows until it is done.
- **Combined with an isolation mode (AU-021):** the chosen stem from step 1 is the input to this stage.
- **Cached audio deleted or evicted:** rebuilt on demand, with the badge shown meanwhile.
- **Detach or restore (AU-003):** the setting moves with `ClipAudio`. The cache is keyed by media and source range, so it stays valid.
- **Reversed clip:** allowed, and reversing needs no new processing, because the processing happens in source time before the retime stage. Observed alternative: the reference app drops its enhancement when a clip is reversed, after a confirmation, and restores earlier voice effects.
- **Compound clip, clip without sound, offline or silent source:** unavailable with AU-040's reasons.
- **Job states** (AI-001 and PM-073): queued, with the place in the queue; processing n %; re-processing n % after an edit extends the source range; done. A failed job offers Retry, and the clip keeps playing unprocessed. There is no limit on clip length or number of clips. Jobs simply queue.
**Undo:** `SetVoiceClarity { clips, old: Vec<Option<Strength>>, new: Option<Strength> }`. A strength drag coalesces into one entry (TL-043). Cache files are not edits and are never deleted by undo.
**Model notes:** `ClipAudio.clarity: Option<Strength>` (None = off) in `freecut-timeline`. Inference runs in `freecut-analysis` (AI-024's engine), the cache file is written by `freecut-media` under project-and-media.md's cache rules, and `freecut-audio` reads it at chain step 3. AU-018 stays model-free.

### AU-021 Voice isolation and "Split into stems" — later
**What the user sees:** Two commands on clips with sound (audio clips, and video clips whose embedded audio is active). AU owns both of them.
- **Voice isolation**, an inspector dropdown that switches the clip in place and creates no new clips:
  - **Off** (default).
  - **Voice only:** plays just the voice, without music and background sound.
  - **Background only:** plays the backing without the voice, as in karaoke.
- **Split into stems…**, in the clip's context menu and in the audio section. A small dialog picks **2 stems** (voice and background) or **4 stems** (vocals, drums, bass, other). After separation, each stem appears as its own clip on a new audio track under the source, linked to the source. The source's own sound is muted, never deleted.
**Inputs / controls:**
- Isolation mode dropdown.
- Split dialog:
  - **Stem set:** 2 (default) or 4. Quality uses AI-023's choice (Standard or High; four stems always use High). A 6-stem set (vocals, drums, bass, guitar, piano, other) is a request to AI-023, since six-source models exist in the Demucs family. The dialog offers it only when AI-023's engine provides it.
  - **Stems to place:** one checkbox per stem of the chosen set, all ticked by default.
  - **Keep the rest as one clip:** default on. When some stems are unticked, their sum is placed as one more clip named "Rest", so nothing in the original is lost. When every stem is ticked, the option is hidden.
  - **Place empty stems too:** default off (see the "stem with no content" edge case).
- Progress and Cancel for the separation job, in AI-001's task list and on the clip. Cancelling a job that has run for more than 5 s asks for confirmation first, because the progress is lost (FreeCut default).
**Behavior / output:**
- **Engine:** AI-023 owns the separation model, the job, the quality setting and the stem files. Stems cover the whole media file, once per audio stream. They are written as FLAC files and registered as PM-041 derived media (`GeneratedKind::Stem`). Every clip that uses the same media and stream shares them. Inference runs in `freecut-analysis`.
- **Voice isolation:** choosing a mode starts AI-023's two-stem job if those stems don't exist yet. Once they exist, chain step 1 decodes the chosen stem instead of the source.
- **Split into stems** runs AI-023's job for the chosen stem set, or reuses existing stems. When every stem is ready, it applies one compound edit:
  - **Tracks:** one new audio track per stem (TL-006). For an audio clip they go directly below the source clip's track. For a video clip they go at the top of the audio band. Each track is named after the clip and its stem, for example "interview · Voice".
  - **Clips:** one stem clip per new track, with the same timeline range and source range as the source clip, pointing at the stem's media item.
  - **Settings:** each stem clip gets a copy of the source's retime (speed, curve and reverse; TR) and of its `ClipAudio`, with three exceptions (FreeCut default). Isolation is set to Off. Normalization is off, since each stem has its own loudness. Cleanup (noise reduction, echo reduction, voice polish, voice clarity) is copied to the voice or vocals stem only.
  - **Link:** the stem clips share one `LinkId` with the source clip (TL-030's link mechanism), so they show the link badge and TL-030's out-of-sync offset if one is moved without the others. TL-030's Restore audio does not apply, because the source's audio was never detached.
  - **Source:** the source clip's audio is muted (`ClipAudio.muted`, AU-007; for a video clip, its active embedded audio). It is never deleted or detached, so unmuting it brings the original back for comparison.
  - **Unticked stems:** with "Keep the rest" on, AI-023 is asked for one extra Rest stem, the sum of the unticked stems. It is written as another stem file, and producing it is a request to AI-023, which owns the stem files. With the option off, unticked stems are not placed, but their files exist and can be added later from the media panel.
- Stem clips are ordinary audio clips. Their stem file is decoded at chain step 1.
- **Splitting again:** "Split into stems…" also works on a stem clip, including Rest. Its stem file is ordinary derived media, so AI-023 runs on that file and the new stems go on tracks under it. For example, a Rest clip can be split into drums and bass.
- History labels name the result, such as "Voice only", "Background only" or "Split into 4 stems" (TL-043's labels).
- Reference: the reference editor offers keeping or removing vocals in its audio controls, and its public tool pages describe separating vocals from the accompaniment. Its observed UI labels also show per-instrument separation (vocals, all instruments, bass, drums, guitar, piano, strings, wind, synthesizer, other, plus the remaining audio), placing stems at the source position, separating again from what is left, a report for each requested stem that isn't present, and partial failures. Those run on its server behind a paywall. FreeCut's stem sets follow what the local engine can do.
**Edge cases:**
- **Separation not finished (isolation):** the clip plays the original audio with a warning badge. Export waits.
- **Separation not finished (split):** the timeline doesn't change until every stem exists. Cancelling the job leaves everything unchanged.
- **Trimming or extending a clip later:** never needs new separation, because the stems cover the whole file.
- **Media longer than 1 h:** AI-023's time and disk estimate is shown first, and the job starts only after the user confirms (FreeCut default).
- **Track limit (TL-004) would be exceeded by the new tracks:** the command is refused with a message before any job starts.
- **Source clip already uses an isolation mode:** the split works from the original source. The source keeps its mode (it is muted anyway).
- **Source already muted:** it stays muted, and undo restores exactly the previous state.
- **Several clips selected:** each one is split, and all of it is one undo entry, applied after the last job finishes. Selected clips that overlap in time get separate sets of tracks. There is no cap on the number of clips. Jobs queue on AI-001's scheduler (the observed reference app caps the batch size).
- **Some jobs in a batch fail:** the clips whose separation succeeded are split, as one undo entry. The failed ones are listed in a toast and in PM-073 with Retry (AU-040's batch rule). Retrying makes a separate entry.
- **Stem with no content:** a stem whose integrated loudness is below −60 LUFS (FreeCut default) is reported by name, for example "No drums found", and is not placed unless "Place empty stems too" is ticked. Its file is kept for PM-074's cleanup.
- **Clip length:** there is no fixed maximum. The 1 h estimate rule above applies (the observed reference app refuses clips over a length cap).
- **Reversed or retimed clip:** allowed, because the stems cover the whole file in source time and each stem clip copies the retime. The observed reference app refuses reversed clips.
- **Compound clip, image, freeze-frame still:** unavailable with AU-040's reasons.
- **Project opened in an older FreeCut that predates stems:** PM-021's newer-format rule applies.
- **Stem file deleted later:** isolation plays the original with the badge, and stem clips show the missing-media state. Both offer PM-041's Regenerate, which reruns AI-023.
- **Model not installed:** both commands are still shown. Using one opens AI-002's install prompt (raised by AI-023). Cancelling leaves everything unchanged.
- **Speech-only input with Voice only:** behaves like strong noise reduction.
- **Locked track:** the source can't be selected (TL-009), so the command is unavailable. New tracks are created unlocked.
**Undo:**
- `SetIsolation { clip, old, new }`.
- `SplitIntoStems { sources, source_muted_before, created_tracks, created_clips, created_media_items, link }` is one compound entry. Undo removes the stem clips, the new tracks and the stem media items, and restores the source's mute state.
- Undo never deletes stem files (PM-041). Cache cleanup does.
**Model notes:**
- `ClipAudio.isolation: IsolationMode { Off, VoiceOnly, BackgroundOnly }`, `StemSet { Two, Four, Six }` (Six only once AI-023 offers it) and the split options `SplitStemsOptions { set, place: Vec<StemKind>, keep_rest: bool, place_empty: bool }` live in `freecut-timeline`.
- A pure planner, `plan_split_into_stems(seq, clips, stems: &[(StemKind, MediaId)]) -> Result<ChangeSet, EditError>`, is unit-tested for track placement, settings copy, linking, source mute, unticked stems with and without the Rest clip, and skipped empty stems.
- Stems are produced by AI-023's engine in `freecut-analysis` and written by `freecut-media`. `freecut-audio` only decodes them, at chain step 1.

### AU-022 Voice effects (voice changer) — v1
**What the user sees:** A "Voice effects" grid in the inspector's audio section. Each tile has a generic name and a short preview sound. One effect can be active per clip, with a "None" tile first. When an effect is selected, its parameters appear below the grid.
- **Tabs above the grid** (FreeCut's own grouping): All, Pitch & character, Rooms & echoes, Devices & radio, Modulation, Recent, and My voices (the user's saved presets).
- An **Apply to** menu beside the grid, and a toast after applying that says how many clips changed.
**Inputs / controls:** Every effect has **Strength 0 to 100 % (default 100 %)** and one to three specific parameters. The set below is FreeCut's own. The names describe the processing and are not taken from any other product.

| Effect (FreeCut name) | Tab | What it does | Extra parameter(s) |
|---|---|---|---|
| Pitch up | Pitch & character | Pitch raised with formant shift | Amount +1 to +12 semitones (default +5); timbre −50 to +50 (default 0) |
| Pitch down | Pitch & character | Pitch lowered with formant shift | Amount −1 to −12 semitones (default −5); timbre −50 to +50 (default 0) |
| Squeaky character | Pitch & character | Large pitch and formant rise | Timbre −50 to +50 (default 0) |
| Booming character | Pitch & character | Large pitch and formant drop | Timbre −50 to +50 (default 0) |
| Ring-mod machine | Pitch & character | Ring modulation plus pitch flattening | Tone 30 to 300 Hz (default 80) |
| Phone line | Devices & radio | Narrow band-pass (about 300 to 3400 Hz) plus light saturation | — |
| Loudspeaker | Devices & radio | Mid-range band-pass plus overdrive | Drive 0 to 100 % |
| Crackly radio | Devices & radio | Band-limited, warm saturation plus generated crackle | Crackle (noise amount) 0 to 100 % |
| Lo-fi | Devices & radio | Bit-depth and sample-rate reduction | Bits 4 to 12 (default 8); downsample 1× to 16× (default 4×) |
| Feedback delay | Rooms & echoes | Repeating echo | Delay 50 to 1000 ms (default 250), feedback 0 to 90 % (default 35) |
| Hall reverb | Rooms & echoes | Algorithmic reverb | Room size 0 to 100 %, mix 0 to 100 % |
| Muffled underwater | Rooms & echoes | Low-pass plus slow chorus | Depth 0 to 100 % |
| Vibrato | Modulation | Pitch wobble | Rate 1 to 12 Hz, depth 0 to 100 % |
| Doubler | Modulation | Chorus or doubling | Voices 2 to 8 (default 2); width 0 to 100 % |
| Tremolo | Modulation | Amplitude modulation | Rate 1 to 20 Hz, depth 0 to 100 % |
| Rotating | Modulation | The voice circles slowly around the listener (auto-pan with a slight level and tone change at the sides) | Rate 0.05 to 2 Hz (default 0.25); width 0 to 100 % (default 100); centre −100 to +100 (default 0) |

All defaults not stated above are 50 % (FreeCut default).
- **Timbre** shifts the formants on their own, on top of the effect's built-in formant shift: −50 is darker and larger, +50 brighter and smaller, and 0 keeps the effect's own character.
- **Apply to:** Selected clips (default), All clips with sound on the main track, All clips with sound on other tracks, or All clips with sound. The scopes cover audio clips and video clips whose embedded audio is active. They skip clips on locked tracks and ineligible clips (AU-040).
- **My voices:** "Save as my voice…" stores the current effect, strength and parameters under a name (1 to 40 characters, unique, FreeCut default). Presets can be renamed and deleted. Applying one copies its values into the clip, with no lasting link. They are stored in the app data directory, not the project, so they are available in every project, and they can be exported and imported as small files.
**Behavior / output:**
- Applied at step 7 of the chain. The pitch part of pitch-based effects runs in the retime pass at step 5 (shared with AU-023 and AU-024), so the audio is never stretched twice.
- Real-time in preview. Every effect is local DSP, so applying it never waits on a job and never fails for clips that pass AU-040.
- A standalone pitch change without any effect is AU-024.
- Reference: the reference editor's voice changer mixes three kinds of tools: DSP-style filters, synthetic character voices, and a speech-to-singing conversion. Only the DSP-filter kind is in scope here (see Out of scope). Its observed UI labels show voice tabs (female, male, young, mature, trending, free and the user's own), an apply scope of all, main-track or overlay clips, and adjustable pitch, timbre, strength, depth, frequency, noise amount, room size, surround rate, width, voice count and centre position. FreeCut's tabs group effects by the kind of processing instead, and the parameter list above covers those adjustable quantities with its own effects.
**Edge cases:**
- **Delay or reverb tail at the clip end:** the tail is cut at the clip boundary by default. A "let tail ring" option extends the audio past the clip end, as a render-only extension that doesn't change the clip's length (later).
- **Effect on a clip with pitch shift (AU-024):** the semitone values add up.
- **Effect on a muted clip:** no audible change.
- **Extreme settings** (feedback 90 %, long delay, 8 voices at full width): bounded so the output can't run away. The Doubler's voices are summed with a 1/√n gain.
- **Rotating on a mono-origin clip:** works, because the clip is already stereo after chain step 2. With clip pan (AU-014) set, the pan offsets the effect's centre.
- **Apply to with some ineligible clips:** AU-040's batch rule applies, and the toast says how many were skipped and why.
- **Clip length:** no minimum or maximum. The observed reference app limits clip length for its model-based voices, which FreeCut does not have.
- **Reversed clip:** allowed, because the effect runs in timeline time (the observed reference app refuses reversed clips).
- **My voices preset made by a newer FreeCut with an unknown effect kind:** the preset shows as unavailable and is never silently changed.
**Undo:** `SetVoiceEffect { clips, old: Vec<Option<VoiceEffect>>, new: Option<VoiceEffect> }`. One entry per apply, including the Apply to scopes. Parameter drags are coalesced. Saving, renaming and deleting My voices presets are app preferences, not project edits (as CL-026's user looks).
**Model notes:** `VoiceEffect { kind: VoiceEffectKind, strength, params: VoiceEffectParams }` is in `freecut-timeline`. Parameters are stored as integers (cents, ms, Hz × 100, %, voices, bits). A pure planner `plan_apply_voice_effect(seq, scope, effect) -> ChangeSet` resolves the Apply to scopes and is unit-tested. The DSP is in `freecut-audio`, and the preview sounds are FreeCut-recorded CC0 assets. The My voices store lives in `freecut-app` (JSON in the app data directory).

### AU-023 Keep pitch and the time-stretch engine — v1
**What the user sees:** A **Keep pitch** checkbox, on by default, on every clip with sound. It sits in the inspector's Speed tab under TR's speed controls (TR-037, TR-039, TR-040) and is mirrored in the Audio section. With it on, sped-up or slowed-down speech keeps its natural pitch. With it off, pitch rises and falls with speed, like a tape. Speed, speed curves and reverse themselves are defined in clip-transform-and-speed.md (TR-036, TR-037, TR-039, TR-040, TR-043). AU owns this toggle and makes the resulting audio sound right.
**Inputs / controls:**
- **Keep pitch:** on/off, **default on** (FreeCut default). Public tutorials describe a similar pitch option beside the reference editor's speed control.
- **Multi-select:** sets every selected clip with sound to the same new state. If any of them is off, all of them turn on.
- The speed values, and the quality hint shown at extreme speeds, come from TR's retime model (TR-036, TR-037).
**Behavior / output:**
- **Keep pitch on:** a time-stretch engine changes duration without changing pitch. It supports continuously changing ratios for speed curves.
- **Keep pitch off:** the audio is resampled (varispeed), so pitch shifts by `12·log2(speed)` semitones.
- **Reverse** (AU's rule, which TR-038 refers to): the source samples for each span are read backwards first, then stretched or varispeeded by the same rule as forward playback. A reversed clip at 1× therefore sounds backwards at its original pitch, whatever the setting.
- The engine reads the time map only through TR-038's contract (`source_time_at`, `local_rate`, `reverse`), once per processing block. It never works the mapping out from the UI speed value. Clip duration and timing come entirely from TR's retime map, and the audio stage only follows it.
- Pitch shift (AU-024) and the pitch part of voice effects (AU-022) are applied in the same pass.
- Resampling to the project rate happens after this stage (chain step 6).
- At constant 1× the flag has no audible effect, but it is still stored, so it applies as soon as the speed changes.
**Edge cases:**
- **Ratios outside the engine's best range** (roughly 0.25× to 4×): quality drops, but the audio still plays. There is no automatic mute (FreeCut default).
- **Near-zero speed segments of a curve:** the stretch ratio is clamped to the engine's limit, and the section is silent below 0.05× (FreeCut default).
- **Freeze-frame stills (TL-031):** they have no time map and no audio (TR-038).
- **Beat markers and keyframes:** shown and evaluated through the retime map.
- **Normalization:** measured after the stretch (AU-017).
- **Shuttle playback (AU-016):** uses the same engine and always keeps pitch, whatever the clip's setting.
- **Detach and restore (AU-003, TL-030):** the flag moves with `ClipAudio`. The detached clip carries its own copy of the retime, so it sounds the same.
- **Video clip whose audio is detached, or that has no sound:** the checkbox is disabled. On a detached clip it lives on the detached audio clip instead.
**Undo:** `SetPreservePitch { clips, old: Vec<bool>, new: bool }`, one entry per click, including multi-select. The speed ops belong to TR (TR-037 to TR-043).
**Model notes:**
- The flag is `ClipAudio.preserve_pitch: bool` (default true) in `freecut-timeline`. It is not part of TR's `Retime`.
- TR-038 defines only the time-map contract that the engine consumes.
- The engine lives in `freecut-audio`. Candidates: our own pure-Rust WSOLA or phase-vocoder implementation (TR's suggestion), signalsmith-stretch (MIT), or Rubber Band (GPL-2.0-or-later, compatible with FreeCut's GPL-3.0-or-later, with a variable-ratio real-time mode). The engine choice is an open question.

### AU-024 Pitch shift — v1
**What the user sees:** A Pitch slider in the audio section, labelled in semitones, with a fine-tune field in cents, and a "Preserve formants" checkbox.
**Inputs / controls:**
- **Range: −12 to +12 semitones** (FreeCut default). The slider steps one semitone; the field accepts cents (−1200 to +1200). Default 0.
- **Preserve formants:** default off.
**Behavior / output:**
- Changes pitch without changing duration, in the same retime pass as AU-023.
- With Preserve formants on, voices stay natural-sounding instead of gaining a cartoonish quality.
- Typical uses: changing the key of a song, or a subtle voice disguise.
**Edge cases:**
- **±12 semitones with formants off:** strong artifacts, which is expected.
- **Pitch shift combined with Keep pitch off (AU-023):** the speed-based shift and the manual shift add up.
- **Pitch shift on a reversed clip:** allowed.
**Undo:** `SetPitch { clip, old_cents, new_cents }` and `SetPreserveFormants`.
**Model notes:** `ClipAudio.pitch: Cents(i32)` and `ClipAudio.preserve_formants: bool`.

### AU-025 Beat detection and beat markers — v1
**What the user sees:**
- **Detect beats** works on any clip with sound. It is a button in the inspector's audio section, an item in the clip's context menu, and a beat button in the timeline toolbar that runs on the selected clips.
- After the background analysis, small tick marks appear along the clip at each beat. These are the auto markers. Beats added with the tap key are manual markers and use a slightly different tick style, so the two kinds are easy to tell apart.
- The audio section shows the tempo readout, for example "≈ 122 BPM", or "Variable tempo" when there is no steady pulse. Clicking the readout opens AI-005's Beat grid popover, which only refines the grid.
- The context menu offers **Clear auto beats** and **Clear all beats**.
- Beat markers are snap targets (TL-028), and they are the input for auto beat sync (AI-006).
**Inputs / controls (FreeCut defaults):**
- **Density:**
  - Every beat (default).
  - Every 2nd beat.
  - Downbeats only: the first beat of each bar, using AI-005's meter (4/4 unless refined).
  - Every onset: a marker on every detected note or hit above the sensitivity threshold, not only on the steady pulse, so cuts can follow the melody or the drum pattern. Onsets closer than 100 ms are merged (FreeCut default).
  - Observed: the reference app offers two automatic densities, read either as main beats versus a denser pattern, or as beats versus following the melody. FreeCut's Every beat and Every onset cover both readings.
- **Sensitivity:** 0 to 100, default 50. Higher values accept weaker onsets and let the tracker follow tempo drift more freely.
- Changing density or sensitivity redraws the auto markers at once from the cached analysis, without decoding again.
- **Tap key:** during playback, pressing it drops a manual beat marker where the beat was heard, using AU-026's latency compensation. The proposed binding is `Alt+B` (macOS `⌥B`). It is to be listed in UI-031, whose table wins on conflicts.
- **Add beat at playhead:** when playback is stopped, the same key (and the context-menu item "Add beat at playhead") drops a manual marker exactly at the playhead, with no latency compensation. This is the precise way to place or fix a single beat while stepping frame by frame. The observed reference app offers the same manual add at the playhead.
- **Detect on import:** AI-001's "Analyze on import" switch for beat detection on audio-only files, off by default.
**Behavior / output:**
- **Job:** detection runs as a background job on AI-001's scheduler, so it gets the task list, throttling during playback and the export wait.
- **Input:** the clip's selected stream, decoded by `freecut-media`, downmixed to mono and resampled to 22.05 kHz (FreeCut default) before any clip processing. That way, markers don't move when the user changes volume, cleanup or effects.
- **Range:** the analysis covers the whole media file, once per stream. Every clip that uses that file shares it, and trims never trigger new analysis.
- **Method:** FreeCut's own implementation of published textbook methods.
  - An onset-strength envelope from spectral flux.
  - A tempo estimate from the envelope's autocorrelation, searched between 60 and 200 BPM.
  - A dynamic-programming beat tracker that weighs onset strength against a steady beat period.
  - A downbeat phase chosen as the bar position with the strongest low-frequency onsets.
  - A tempo-stability score decides when the readout says "Variable tempo".
- **Result:** a beat-analysis record, AI-001's `AnalysisRecord` with kind Beats. It holds the tempo, the stability score, the beat and onset times in source time and the detection settings. It is small, so it is stored in the project file. The onset envelope is a regenerable cache file (one of AI-001's large results). Density, sensitivity and AI-005's refinement re-run the tracker on that envelope. AI-005 stores its `BeatGridParams` on this same record.
- **Markers:** auto beats are written as TL-037 clip markers with origin Beat, stored in source time at tick precision and flagged auto. They cover the whole media stream. Markers outside the trimmed range are hidden, not deleted (TL-037).
  - Re-detecting replaces the clip's auto markers. Manual markers are never touched.
  - An auto marker within 2 frames of a manual one is dropped, so the manual one wins (FreeCut default).
  - Because markers are stored in source time, they follow moves and trims and divide on split (TL-037). They are drawn through the retime map (TR-036), so they stay on the beat at any speed and in reverse.
- **Tap target:** the single selected clip with sound, if the playhead is inside it. Otherwise, the clip with sound under the playhead on the topmost audio track that has one, preferring music-origin clips. If no clip qualifies, nothing is added and the status bar says why. Beats are always clip markers, never timeline markers.
- Reference: in the reference desktop editor, a beat-marking button above the timeline (or a context-menu item) places dots on each beat. Public guides also describe pressing a key on each heard beat to add markers by hand, and note that the automatic dots often need manual fixes. The density and sensitivity controls are FreeCut's own.
**Edge cases:**
- **Muted clip:** can still be analysed.
- **Clip with an isolation mode (AU-021):** analysis uses the original source, not the stem (FreeCut default).
- **Media offline:** detection is disabled, and existing markers stay.
- **Video clip with embedded audio:** analysed the same way, using its selected stream.
- **No steady pulse:** the readout says "Variable tempo". Markers are still placed at the tracked beats, and AI-005's tempo controls are disabled.
- **Silent or near-silent media, or media shorter than about 2 s:** no markers are placed, and a message explains why. A source that AU-040 flags as silent has Detect beats disabled with that reason.
- **No beats found in audible media** (for example speech or ambience with no pulse): no auto markers, and the message suggests raising sensitivity or using Every onset.
- **Detection job fails** (decode error, cancelled): existing markers are unchanged, and PM-073's row and the inspector offer Retry.
- **Clip with a speed curve (TR-039, TR-040):** allowed. Markers are drawn through the time map, so they bunch up and spread out with the curve. The observed reference app reports a conflict between beat markers and speed curves on the same clip. FreeCut keeps both, because markers are stored in source time.
- **Cutting video to the beats:** AI-006 (fit clips to beats) and AI-029 (auto montage) own this. When no clip has beat markers, they offer to run this detection first. The observed reference app's drop zone, which lays several media files out on the beats, maps to AI-029.
- **User drags an auto marker:** it becomes a manual marker, so detecting again does not undo the correction. Deleted auto markers return if detection is run again.
- **Media file changed on disk:** AI-001's stale rule applies. The markers stay until detection is run again.
- **Locked track:** the clip can't be selected (TL-009), and the tap key skips locked tracks.
- **Detach (TL-030):** markers stay on the clip they were made on. Copying them to the detached clip is an open request to TL.
- **Several clips of the same media:** they share one analysis record. Changing density or sensitivity on one of them rewrites the auto markers of every clip of that media and stream, in one entry.
**Undo:**
- **Detect, density and sensitivity:** `SetAutoBeatMarkers { clips, before: Vec<Vec<Marker>>, after: Vec<Vec<Marker>>, params_before, params_after }`. It is one entry, and a slider drag coalesces (TL-043). AI-005's refinement wraps this same op. The analysis job itself is not an edit.
- **Tap key and Add beat at playhead:** TL-037's `AddMarker` with origin Beat (manual). A run of taps coalesces under TL-043's keyboard-repeat rule.
- **Clear auto beats and Clear all beats:** TL-037's `DeleteMarkers`.
**Model notes:**
- Beats are stored only as TL-037 clip markers: origin Beat, source time, and an auto or manual flag (`MarkerOrigin::Beat { auto: bool }`). There is no `ClipAudio.beats` field and no separate beat-marker type.
- The beat-analysis record payload is `BeatAnalysis { tempo: Option<Rational>, stability: u8, beats: Vec<Tick /* source time */>, onsets: Vec<Tick /* source time */>, detect: BeatDetectParams { density, sensitivity }, grid: Option<BeatGridParams> /* AI-005 */ }`. It is persisted by `freecut-project`.
- The onset, tempo and tracker code is classic DSP with no model weights. It lives in `freecut-audio` and runs as an AI-001 `AnalysisJob` orchestrated by `freecut-app`, with decoding in `freecut-media`.
- `BeatDensity { EveryBeat, EverySecond, Downbeats, Onsets }`. Every onset reads the onset list, which the beat-analysis record keeps next to the tracked beats (`onsets: Vec<Tick>` in source time).
- A pure planner, `plan_auto_beat_markers(clip, &BeatAnalysis) -> ChangeSet`, lives in `freecut-timeline`. It is unit-tested for density filtering (including the 100 ms onset merge), the manual-wins rule and source-time placement.

### AU-026 Tap-beat latency compensation — v1
**What the user sees:** When the user taps beat markers during playback with AU-025's tap key, the markers land where the beat was heard, not a little late. A Preferences field lets the user fine-tune the offset.
**Inputs / controls:**
- **Output latency offset:** −200 to +200 ms. Default 0 ms (FreeCut default).
- A tap-along calibration helper that plays a click track and measures the average tap error (later).
**Behavior / output:**
- Marker time = playhead time at the key press − (device-reported output latency + buffer duration + the user offset). The result is mapped into the target clip's source time (TR-036) and stored at tick precision, like auto beats (AU-025). Snapping to it rounds to the frame grid (TL-028).
- The same latency value is shared with A/V sync (AU-010).
**Edge cases:**
- **Tap before the compensated time reaches the clip start:** clamped to the clip start.
- **Output device changed mid-playback:** the latency is read again.
- **Bluetooth headphones** (often 150 to 300 ms of latency): fully compensated when the device reports it. Otherwise the user offset covers it.
- **No output device** (system-clock playback, AU-010): zero latency is assumed.
**Undo:** n/a for the preference. The marker edits use AU-025's tap op (TL-037's `AddMarker`, origin Beat, manual).
**Model notes:** `freecut-audio` exposes the current output latency. `freecut-app` computes the marker time. The offset is an app preference, not project data.

### AU-027 Voiceover recording — v1
**What the user sees:** A microphone button in the timeline toolbar opens a recording panel with:
- An input device dropdown.
- An input level slider with a live meter.
- Toggles that apply "Noise reduction" (AU-018), "Reduce echo" (AU-037), "Voice polish" (AU-019) and, later, "Voice clarity" (AU-020) to the new recording.
- A voice effect picker (AU-022, default None) for the new recording.
- A "Mute timeline while recording" toggle.
- A countdown choice.
- A large Record/Stop button, and a "Redo last take" button after a take.
- A time readout: the elapsed take time and, while the playhead is inside the timeline, the time left until the timeline's end.
- A **Takes** list of the recordings made in this project (newest first), with play, "Show on timeline" and "Remove from project". It has an empty state that says how to start recording. The same recordings appear under AU-030's Recordings filter.

During recording, a red clip grows on the timeline in real time, with a live waveform, starting at the playhead. The panel shows its state: Ready, Waiting (no usable input), Counting down, Recording, or Stopping.
**Inputs / controls:**
- **Countdown:** off, 3 s (default) or 5 s.
- **Input level:** −20 to +20 dB software gain. Default 0 dB.
- **Mute timeline while recording:** default off (FreeCut default). A warning appears if input and output appear to be the same laptop's built-in mic and speakers, because playback would bleed into the take.
- **Channels:** Mono (default) or Stereo.
- **Voice effect:** any AU-022 effect or a My voices preset. Default None.
- Record or stop with the panel button. The keyboard shortcut is owned by workspace-and-shortcuts.md.
- **Minimum take length:** 0.3 s (FreeCut default). Shorter takes are treated as accidental clicks (see Edge cases).
**Behavior / output:**
- After the countdown, the timeline plays from the playhead (unless muted) while the capture thread writes to disk.
- The file is WAV, 24-bit, at the project rate if the device supports it, otherwise at the device's native rate. It is written to the project's recordings folder (location owned by project-and-media.md) with a timestamped name. A take that would pass the 4 GB WAV limit (about 4 hours of 24-bit stereo at 48 kHz) switches to RF64 on the fly, so long takes never stop for that reason.
- On stop, the file is imported as a media item and a clip is placed at the start position on the first audio track free over the recorded span. If none is free, a new track is created. FreeCut never overwrites existing clips.
- The toggles and the voice effect in the panel just set the matching `ClipAudio` attributes on the new clip, so they stay non-destructive and can be changed later.
- **Redo last take:** removes the last take made in this panel session from the timeline and starts a new take at the same start position, after the countdown. The removal and the new take form one undo entry. The replaced file stays on disk and in the Takes list until the user removes it.
- **Remove from project** in the Takes list uses PM-058. The file stays on disk, and PM-074's cleanup offers to move unused recordings to the trash.
- Input latency (reported by the device, plus a manual offset preference of −500 to +500 ms, default 0) is compensated so the clip lines up with what you were watching.
- A toast confirms when a microphone is connected while the panel is open, and the device dropdown refreshes.
- Reference: the reference desktop editor's recorder offers a device choice, an input level, cleanup toggles and an option to silence the project while recording. Its observed UI labels also show a waiting state, an optional countdown, re-recording the last take, a recordings list with delete and voice effects, a remaining-time readout, and a minimum and maximum take length with auto-stop.
**Edge cases:**
- **No microphone found:** Record is disabled with "No microphone found". The panel stays in Waiting and enables Record as soon as a device appears.
- **Microphone permission missing or denied (macOS or Windows privacy settings):** the first Record asks the OS for permission. If it is denied, the panel explains how to grant it and offers a button that opens the system's privacy settings page for the microphone. Nothing is recorded.
- **Microphone in use by another app** (exclusive access) **or fails to open:** the panel names the problem and suggests closing the other app or choosing another input. Nothing is recorded.
- **Device unplugged mid-take:** recording stops, the audio captured so far is kept and placed, and a notice appears. Observed alternative: the reference app pauses the take and continues it when the microphone comes back. FreeCut stops instead, because a reconnected device may return at another rate or as a different device. The playhead is left at the end of the take, so the next take continues from there.
- **Zero samples captured, or a take shorter than 0.3 s:** no clip is created, the file is removed (it holds no usable audio), and a notice says "Take too short".
- **Disk nearly full:** a warning appears below 500 MB free, and recording stops cleanly at 50 MB free. The captured part is kept and placed, and the notice asks the user to free space before recording again.
- **Recording past the end of the timeline:** allowed, and the timeline grows. Once the playhead passes the end, the time-left readout disappears. Observed alternative: the reference app limits a take to the time left in the video, and auto-stops at a maximum length. FreeCut has no maximum other than disk space.
- **Locked audio tracks:** placement skips them.
- **Several takes at the same position:** each take goes on the next free track, giving stacked takes.
- **Redo last take after the last take was moved or edited:** the button still replaces that take. If the take was deleted, the button is disabled.
- **Device rate differs from the project:** recorded at the native rate and resampled during playback (AU-010).
- **App crash during recording:** the WAV header is fixed on next launch and the file is offered for import (FreeCut default).
- **Project mute (AU-039) on:** recording works, and the panel shows that the timeline is muted.
- **Camera recording:** PM-043's job, not this panel's.
**Undo:** `InsertRecordedClip { media_item, clip, created_track? }` is one entry that removes the clip, track and media item on undo. Redo last take is one compound entry: the removal of the old take's clip plus the new `InsertRecordedClip`. The recorded files stay on disk and can be reimported. Undo never deletes user audio.
**Model notes:**
- Capture uses `cpal` input in `freecut-audio`, on a dedicated thread writing through a ring buffer to a background file writer. The WAV is written by FreeCut's own pure-Rust writer (`freecut_audio::record::wav`, architecture C53), with no FFmpeg; it switches to RF64 on the fly and refreshes the header sizes every second. `hound` (Apache-2.0) is only a dev-dependency for fixtures. Nothing touches the UI thread.
- `freecut-timeline` provides the free-track search, the insert op and the redo-take planner.
- `freecut-app` builds the panel, the Takes list (a filtered view of media items with origin Recording, PM-041) and the permission prompts. Device listing runs off the UI thread, as in UI-029.

**Toolbar round additions (2026-10-13).** These extend the controls and behavior above; all are FreeCut defaults, free and local.
- **Where it opens:** the microphone button in the timeline toolbar and *Add ▸ Record voiceover…* (UI-011) open the recorder as a non-modal floating panel anchored above the toolbar (the UI concept's panel shell). The timeline, player and inspector stay usable while it is open, except that edits are refused during Counting down and Recording (the status line says "Stop recording to edit"; FreeCut text), because the take's start position must not move under it. Closing the panel while recording asks "Stop and keep this take?" with *Keep* (stop and place, the default button), *Discard* (as Cancel below) and *Go back*.
- **Place take on:** *First free audio track* (default; the rule above) or *A new audio track* (always creates one at the top of the audio band, TL-006). The choice is remembered per user.
- **Pre-roll:** 0 (default), 1, 2 or 3 s. Playback starts that much before the start position so the speaker hears the lead-in. Capture runs from the beginning of the pre-roll; the pre-roll audio stays in the file as a handle, the clip starts at the start position with its in-point after the handle, and extending the clip's head (TL-020) reveals any words spoken early. The countdown, if any, runs before the pre-roll.
- **Use In/Out range (punch range):** shown when TL-038's In and Out marks are both set; off by default. When on, the start position is the In mark (not the playhead) and the take stops by itself at the Out mark, plus 0.5 s of handle kept in the file beyond the clip's end (FreeCut default). The time readout shows the time left to the Out mark instead of to the timeline's end.
- **Hear yourself (monitoring):** off by default. When on, the microphone is played back through the current output device on its own path (never into the mix, never into the file), at the input level set in the panel. The panel shows the measured round-trip delay ("Monitoring delay about 18 ms"; FreeCut text). With the built-in speaker/microphone warning above active, switching it on asks first, because it would feed back; a hint recommends headphones.
- **Level meter:** a horizontal meter from −60 to 0 dBFS with green below −12, yellow to −3 and red above; a peak-hold tick held 1.5 s; and a clip light that latches red when any sample after the input-level gain reaches −0.1 dBFS, cleared by clicking it or by starting a new take. The meter is live from the moment a device is chosen (Ready state), so the level can be set before recording. While recording, a latched clip light adds "Input clipped. Lower the level for the next take." under the meter (FreeCut text).
- **Cancel:** `Esc` or the *Cancel* button. During Counting down (or pre-roll before the start position is reached): back to Ready, nothing written. During Recording: the take stops and is **not placed**; the file is finalized and kept in the Recordings folder and the Takes list (marked "Not placed"), with no history entry, and a toast offers *Place* (which places it as if it had just stopped, as one `InsertRecordedClip`). This keeps the rule that FreeCut never deletes a user's audio without asking; PM-074's cleanup offers unused recordings for the trash later. Takes under the 0.3 s minimum are removed as above.
- **States** (the panel shows each): No device → *Waiting*; device open → *Ready*; *Counting down*; *Pre-roll*; *Recording*; *Stopping* (draining the ring and finalizing the file, normally under 200 ms). Every state change happens on the recorder's own threads and is reported to the panel by message; the UI thread only reads atomics and messages.
- **Panel keys** (while the panel has focus; UI-030's focused-panel rule): `Enter` starts or stops a take, `Esc` cancels as above. `Space` keeps its global meaning (play/pause) and is ignored during Counting down, Pre-roll and Recording. The global *Record voiceover* command stays unbound by default (UI-031).
- **Edge cases added:** the In/Out range shorter than the 0.3 s minimum → *Use In/Out range* is disabled with the reason; the playhead moved during Ready → the start position follows it until Record is pressed, then is fixed; a sample-rate change of the device mid-take → treated like an unplug (stop, keep, place, notice); monitoring with a device that has no output path at the chosen rate → monitoring is switched off with a notice and the take continues.

**Build contract (toolbar round, slice T6; types in audio-engine.md §12):**
- `freecut-audio::record`. The capture-independent parts (state machine, writer, meters, `SyntheticInput`) build in every configuration; `CpalInput` sits behind a new default feature `record` that, like `output`, pulls in `cpal`, so `cargo test -p freecut-audio --no-default-features` still runs the recorder tests:
  - `fn list_inputs() -> Result<Vec<InputDeviceInfo>, RecordError>`, called on a worker; `InputDeviceInfo { id, name, is_default, rates: Vec<u32>, max_channels: u16 }`.
  - `trait CaptureSource: Send { fn start(&mut self, sink: CaptureSink) -> Result<(), RecordError>; fn stop(&mut self); fn latency(&self) -> Duration; }` with `CpalInput` and, for tests, `SyntheticInput { signal, rate, channels, fail_after: Option<u64> }`.
  - `Recorder::open(cfg: RecordConfig, source: Box<dyn CaptureSource>) -> Result<Recorder, RecordError>`; `RecordConfig { path, rate, channels: Channels::{Mono, Stereo}, gain: GainDb, preroll_frames, stop_after_frames: Option<u64>, min_free_bytes, monitor: bool }`; methods `arm()`, `start()`, `stop() -> Result<TakeInfo, RecordError>`, `cancel() -> Result<TakeInfo, RecordError>`, `meter() -> MeterReading { peak_db, rms_db, held_peak_db, clipped }` (atomics), `state() -> RecorderState`, `events() -> Receiver<RecorderEvent>`.
  - `TakeInfo { path, frames, rate, channels, preroll_frames, input_latency, started_at: ClockStamp, ended_by: EndReason::{User, Cancelled, RangeEnd, DeviceLost, DiskLow, Overrun, RateChanged} }`.
  - `wav::Writer`: 24-bit PCM, RIFF switching to RF64 past 4 GiB, header sizes refreshed every second so a crash leaves a readable file; the `<file>.recording` marker of PM-043 sits beside the file until it is finalized.
- `freecut-timeline`: `EditRequest::InsertRecordedClip { seq, media: NewMedia, at: Tick, handle_in: Tick, target: RecordTarget::{FirstFree, NewTrack}, replace: Option<ClipId> }` (one entry: library item, clip, any new track, and the removal of `replace` for *Redo last take*; `NewMedia` gains PM-041's `origin`, here `MediaOrigin::Generated { kind: Voiceover, … }`, if it has not by then); `fn recorded_start(start: Tick, input_latency: Duration, user_offset_ms: i32, rate) -> Tick`, a pure, rounded placement helper; `fn first_free_audio_track(seq, range) -> Option<TrackId>` skipping locked tracks.
- `freecut-project`: the Recordings folder path and a startup scan that finds `.recording` markers, repairs the header and offers the file for import (the crash edge case above).
- Tests without a microphone: synthetic capture → file → read back sample-exact (mono and stereo, gain applied); RF64 switch with an injected small threshold; header refresh survives a simulated crash (file truncated mid-write is still readable up to the last refresh); `stop_after_frames` ends at the Out mark; `DeviceLost` from `fail_after` keeps the captured part; `recorded_start` rounding; `InsertRecordedClip` apply → undo → redo for each target and for `replace`; the recorder state machine's transitions, including cancel from every state.
- Build notes (model, 2026-10-13):
  - `NewMedia` is unchanged. `InsertRecordedClip` gives the new item `MediaOrigin::Generated { kind: GeneratedMediaKind::Voiceover }` itself. The model calls PM-041's kind enum `GeneratedMediaKind`, because `GeneratedKind` already names generated clip content.
  - `recorded_start(start, input_latency: Duration, user_offset_ms, rate: SampleRate)` returns the take's sample-exact start: the nearest sample, with halves going earlier. The result may be negative.
  - `InsertRecordedClip` puts `at` on the frame grid and moves the in-point by the same amount, so the sound stays where `at` put it. Any part before 0 is dropped.
  - `RecordTarget::FirstFree` falls back to a new audio track at the bottom, which gives stacked takes. `NewTrack` always adds one at the top of the audio tracks.

- Build notes (app, T6c, 2026-10-14):
  - The recorder runs on an app-side worker (`freecut_app::recorder::RecorderHost`) that owns `freecut_audio::record::Recorder`. It opens inputs, lists them, creates the Recordings folder and cancels takes there. It sends the panel messages: state changes, finished takes, and the meter about 30 times a second. The UI thread never waits on the microphone or the disk. Stop uses the recorder's non-blocking stop and the finished-take event.
  - Takes go to `<Name> Media/Recordings` and fall back to `Recordings/<Name> <hash>` in app data, or `Recordings/Unsaved` for a project that was never saved (`freecut_project::recordings`). The time in a take's name is UTC, because the workspace has no time-zone data yet.
  - The clip's position is worked out from the take's first-sample time (`TakeInfo::started_at`, which already allows for the input latency) and the moment the timeline started playing: `from + (first sample − playback start) + pre-roll`. Without playback (an empty timeline, or a start past the end) it is the start position. The user's latency offset is then added on the sample grid (`recorded_start` with no further latency). `InsertRecordedClip` puts it on the frame grid.
  - The pre-roll never reaches before 0: before the start position there is at most the start position's own time. The countdown counts 3, 2, 1.
  - Edits, undo and redo are refused with "Stop recording to edit" from Record until Stop or Cancel. `Space` is ignored while a take runs. `Enter` starts or stops a take while the panel is open. `Esc` cancels a take, and closes the panel when no take runs.
  - A cancelled take is kept on disk and listed as *Not placed*. A toast offers *Place*, which places it where it was meant to start, as one entry. A take under 0.3 s is removed on a worker, and "Take too short" is shown.
  - When a project opens, a worker repairs interrupted takes in both folders and offers them as *Recovered* in the Takes list (placed at the playhead). A toast says so.
  - *Hear yourself* hands the recorder's feed to the output engine. If the output runs at another rate, the panel says monitoring is off. The measured monitoring delay is not shown yet.
  - Not built yet: the punch range (TL-038's In/Out marks don't exist), the cleanup toggles and voice effect (AU-018/019/020/022/037), the built-in speaker warning, preview playback in the Takes list, the "microphone connected" toast, and AU-030's Recordings filter.
  - Without the app's `record` feature (`cargo build -p freecut-app --no-default-features`), the panel opens in its Waiting state and says the build has no microphone support.
  - macOS packaging must give the app bundle an `NSMicrophoneUsageDescription`, or the system won't ask for permission. A denied permission shows the explanation and *Open privacy settings*.

### AU-028 Automatic ducking — v1
**What the user sees:** With one or more music clips (or a music track) selected, an "Auto-duck" section in the inspector lets you choose which tracks contain speech and press "Apply". The music clips get a visible dip curve drawn under each spoken passage, separate from their volume line. Further buttons let you update or remove the dips. A "needs update" badge appears if the speech clips change after the dips were applied.
**Inputs / controls (FreeCut defaults):**
- **Speech source:** a multi-select of tracks. Default: every audio track that holds voiceover clips, plus video tracks with un-detached audio.
- **Duck amount:** −3 to −30 dB. Default −10 dB.
- **Detection threshold:** −60 to −10 dBFS. Default −35 dBFS, measured on a 100 Hz to 4 kHz band-passed copy of the speech signal.
- **Fade down:** 0.05 to 2.0 s. Default 0.25 s, starting *before* the speech so the first word isn't masked.
- **Fade up:** 0.1 to 3.0 s. Default 0.6 s.
- **Bridge gaps shorter than:** 0 to 3.0 s. Default 0.5 s. Short pauses between sentences don't make the music pump back up.
- Reference: a public guide from the reference editor's maker recommends starting with a dip of roughly 6 to 12 dB, an attack of a few tens of milliseconds and a release of a few hundred. Public sources confirm automatic speech-triggered ducking on the mobile app. For the desktop app they recommend manual volume keyframes and do not confirm an automatic option.
**Behavior / output:**
- `freecut-audio` analyses the mixed speech sources (after their own volume and mute) over the music clips' spans in the background.
- It finds the active regions and generates a duck envelope for each target clip.
- The envelope is stored in the clip's own `duck` layer. It multiplies with the user's volume and keyframes but never overwrites them.
**Edge cases:**
- **Target and source on the same track or in the same clip:** disallowed, with a message.
- **Target clip moved after ducking:** the envelope is stored in clip-local time, so it moves with the clip and is flagged as needing an update.
- **No speech detected:** no envelope is created, and a message suggests lowering the threshold.
- **Overlapping music clips on different tracks:** each gets its own envelope.
- **Speech source muted or soloed out:** still analysed. Analysis ignores the monitoring state and track mute but respects clip mute.
- **Very long timeline:** analysed progressively, with a progress bar and Cancel. Cancel leaves no change.
- **Live sidechain mode** (dynamic, no regeneration needed): later.
**Undo:** `SetDuckEnvelope { clips, old: Vec<Option<DuckEnvelope>>, new }` covers Apply, Update and Remove, each as one entry.
**Model notes:** `DuckEnvelope { points: Vec<(Tick, GainDb)>, params: DuckParams, source_hash: u64 }` in `freecut-timeline`, with points in clip-local time. The hash drives the needs-update badge. Detection lives in `freecut-audio`. An optional ML voice-activity detector (for example Silero VAD, MIT) comes later, run in `freecut-analysis` and installed through AI-002's model manager.

### AU-029 Audio crossfades — v1
**What the user sees:** When two audio clips touch on the same audio track, hovering over the cut shows a crossfade control. Clicking it, or dragging the fade handles of both clips across the cut, creates a crossfade, drawn as two overlapping ramps. Selecting the crossfade shows its duration and curve in the inspector.
**Inputs / controls:**
- **Duration:** 1 frame up to the available handles. Default 0.5 s (FreeCut default).
- **Alignment:** centred on the cut (default), ending at the cut, or starting at the cut.
- **Curve:** Equal power (default; best for unrelated material) or Equal gain (best for two parts of the same take).
**Behavior / output:**
- The outgoing clip continues past its out-point, and the incoming clip starts before its in-point, using media handles. Clip positions and timeline length do not change.
- Video transitions (FX-002) are only allowed between video or image clips, so AU owns this separate audio crossfade object for audio tracks.
- Embedded audio under a video transition is handled by FX-007 (mvp), which uses this spec's equal-power law. That law therefore ships with the mvp mixer (AU-010).
**Edge cases:**
- **Not enough handle on one side:** the crossfade shifts towards the side that has handle, or shortens. The inspector shows the effective length.
- **No handles at all:** falls back to a "dip": the outgoing clip fades out and the incoming clip fades in, each over half the duration.
- **Clips not adjacent (gap between them):** the command is unavailable. Use fades instead.
- **One clip muted:** the crossfade still shapes the other clip.
- **Different sample rates or channel counts:** handled by the mixer (AU-010).
- **Trimming, moving or splitting either clip:** the same op clamps the duration to the new handles. If the clips stop touching or the duration falls below 1 frame, the op removes the crossfade and records it, so undo restores it (the same approach as FX-002).
- **Locked track:** no add, remove or edit (TL-009).
**Undo:** `AddAudioCrossfade { track, left_clip, crossfade }` and `RemoveAudioCrossfade` (which stores the full object), plus `SetAudioCrossfade { old, new }`. A drag is one entry.
**Model notes:** In `freecut-timeline`, an audio track keeps `crossfades: BTreeMap<ClipId /* left clip */, AudioCrossfade { duration: Tick, alignment, curve }>`. `freecut-audio` implements the equal-power and equal-gain laws. The stored fades of the involved edges are kept but inactive (AU-006).

### AU-030 Music library panel — v1
**What the user sees:** An Audio panel with these sections:
- **Project audio:** everything audio-capable in the project, including extracted audio and recordings, with filter chips All, Music, Sound effects, Recordings and Extracted.
- **Music:** FreeCut's optional starter pack plus any folders the user adds.
- **Recent:** the last 50 tracks and effects added to any project (FreeCut default), newest first.
- **Favourites.**

Each row shows title, artist, duration, tempo (when analysed), a licence badge ("Your file", "CC0", "CC BY 4.0", ...) and a play button. Clicking the row's mini-waveform seeks the preview. A "+" adds the track at the playhead, or you can drag it to the timeline. There is a search box and sort options. An "Import audio…" button adds files to the project (AU-001's audio-only import).
**Inputs / controls:**
- Add folder or remove folder (user folders are scanned recursively).
- Search by title, artist, album, genre or tags, read from file metadata.
- Sort by name, duration, tempo or date added.
- **Licence filter:** Any (default), "Allows commercial use" (CC0, CC BY and other permissive licences in the pack manifest) or "Your files". The user's own files have unknown licences, so the commercial filter leaves them out unless the user tags a folder as "My own work".
- Star a track to add it to Favourites.
- A volume control for previews.
- **Trim before adding:** dragging across a row's mini-waveform marks an in and out range, shown as a highlight. "+" and drag then add only that range. Esc clears it.
- **Multi-select:** Shift or Ctrl/Cmd-click selects several rows. The "+" button then reads "Add 3", and the items are placed one after another from the playhead on one free audio track.
**Behavior / output:**
- Folder scanning and metadata reading run in a background job with a file watcher.
- Adding a track imports it into the project as a media item (project-and-media.md) and places a clip on the first free audio track at the playhead. A trimmed add sets the clip's source range to the marked range.
- For tracks whose licence requires credit (CC BY), a "Credits" view lists ready-to-copy attribution lines for every such track used in the project. A clip from such a track shows a small attribution badge (artist and title) in the inspector, so the requirement isn't forgotten.
- No network access: there is no online catalogue, no "trending" list, no download step and no account. Favourites and Recent work offline and need no sign-in.
- Reference: the observed reference app's audio panel has all, music, sound-effect and recordings tabs, search by song, artist or effect name, a recently used list, favourites, preview, click-to-trim before adding, a count of selected items, a commercial-use licence filter and an attribution label on clips. Its server catalogue, trending sort, similar-sound suggestions, brand music and sign-in favourites are cloud features (see Out of scope). "More like this", matching by tags, tempo and duration on the local index, is a later option.
**Edge cases:**
- **Folder removed or unmounted:** its entries turn grey. Clips that already use those files follow the missing-media rules.
- **Huge folders** (more than 50 000 files): scanning is incremental and the panel stays responsive.
- **Unsupported or corrupt file:** listed with a warning icon and cannot be added.
- **Duplicate files in two folders:** both are listed. The project imports each path once (PM-038).
- **Preview playback while the timeline is playing:** the timeline pauses (FreeCut default).
- **File without metadata:** the title falls back to the filename.
- **Search with no results, or a licence filter with no matching tracks:** an empty state says so and offers to clear the filter.
- **Trim range shorter than 1 frame:** ignored. The whole track is added.
- **Multi-add longer than the remaining timeline:** allowed, and the timeline grows.
- **Recent entry whose file is gone:** shown grey. Clicking offers to remove it from Recent.
**Undo:** Adding to the timeline is undoable (media import plus clip insert, as one entry; a multi-add is one entry). Folder lists, favourites, Recent, licence tags and the scan cache are app preferences, not project edits.
**Model notes:** The library index lives in `freecut-app` (app data directory, never the project file). Metadata and probing come from `freecut-media`. The starter pack's licence manifest ships as a JSON file alongside the assets. The multi-add planner (sequential placement on one free track) is a pure function in `freecut-timeline`.
- mvp form (FreeCut default, `ui::audio`): this build has no starter pack and no user folders, so the Audio tab holds only **Project audio**. The left sub-list has "Import audio…" (AU-001) and the Project audio entry. Project audio lists every library item with sound and no picture: imported audio files and extracted sound (AU-004). Videos are not listed; their sound gets here through Extract audio. The chips are All, Imported and Extracted (Music, Sound effects and Recordings wait for the pack, AU-031 and AU-027). The search matches every typed word in the item's name, ignoring case. Rows are in library order and show a ♪ glyph in the clip colour, the name (display name, else file name), "m:ss · Your file" or "m:ss" with an "Extracted" badge, an "Offline" badge when the file is missing, and a mini waveform of the whole item once its peaks are cached (TL-042's cache). "+", a double-click or the row menu's *Add at playhead* places the item on the first free audio track at the playhead (`InsertTarget::FirstFree`); the new clip is selected and the playhead stays. Rows drag onto the timeline like Media tiles. Empty states: "No audio in this project yet" with Import audio…; "No imported audio yet" or "No extracted audio yet" with Show all; and, for a search with no matches, a message with Clear search. A chip with nothing of its kind wins over the search, since clearing the search wouldn't help. Not built yet: row preview playback, artist and tempo columns, sort, licence filter, favourites, Recent, trim before adding and multi-select.

### AU-031 Sound effects library — v1
**What the user sees:** A Sound effects section in the same Audio panel, with generic categories: whooshes and transitions, impacts and hits, clicks and pops, cartoon and comedy, risers and drones, notifications and UI, nature and ambience, crowd and reactions, household and foley, animals. It has the same preview, search, favourites and "+" behaviour as AU-030. User folders can be added as extra effect collections.
**Inputs / controls:** As in AU-030, plus a category filter.
**Behavior / output:**
- Adding an effect places a clip at the playhead on the first free audio track, with the "sound effect" colour.
- The bundled effects are recorded or synthesised by the FreeCut project, or curated from CC0 sources, with the licence shown on each item.
**Edge cases:**
- **Effect longer than the remaining timeline:** allowed, and the timeline grows.
- **Very short effects** (under 1 frame): kept at a minimum of 1 frame in length (TL-011).
- **Pack not installed** (for distributions that strip assets): the section shows only user folders.
**Undo:** Adding is undoable, as in AU-030.
**Model notes:** As in AU-030. Categories come from the pack manifest, not hard-coded names.

### AU-032 Audio-only export — v1
**What the user sees:** The export dialog (preview-and-export.md) has an "Audio only" output type. Choosing it hides the video settings and shows the audio format settings below. The "Save audio as file…" command in AU-004 uses the same panel.
**Inputs / controls:**
- **Format:** WAV (PCM 16-bit, 24-bit or 32-bit float), FLAC (16 or 24-bit), MP3 (CBR 128, 192, 256 or 320 kbps; **default 320**) or AAC in .m4a (128, 192, 256 or 320 kbps; default 256). **Default format: MP3** (FreeCut default). Opus comes later.
- **Sample rate:** 44.1 or 48 kHz. Default is the project rate.
- **Channels:** Stereo (default) or Mono (L+R averaged).
- **Range:** whole timeline (default) or the in/out range (TL-038).
- Reference: public guides for the reference desktop editor describe an audio-only export with MP3, WAV and AAC (and possibly FLAC) and common bitrates from 128 to 320 kbps.
**Behavior / output:**
- Renders the master bus offline through the same mixer (AU-010) and encodes with FFmpeg in `freecut-media` (libmp3lame, the native AAC encoder, FLAC, PCM).
- Runs in a background job with progress and Cancel.
- Metadata tags (title, artist) can be filled in optionally.
**Edge cases:**
- **Solo active:** ignored unless "Export as heard" is ticked (AU-012).
- **Timeline that is silent or has no audio:** export is allowed after a warning.
- **Analysis-dependent features not ready** (normalization, ML clarity, isolation, ducking needing an update): the job waits for their analysis, or warns about stale ducking.
- **Cancel or failure:** the partial file is deleted, and the error names the failing clip if decoding failed.
- **Target file exists:** you are asked to confirm overwrite.
- **Long timelines in WAV over 4 GB:** RF64/WAV64 is used automatically, or the user is told to choose FLAC.
- **Mono export of out-of-phase stereo:** possible cancellation, with no automatic fix.
**Undo:** n/a (output, not an edit).
**Model notes:** Export settings types live in `freecut-project` (export presets are owned by preview-and-export.md). Encoding is in `freecut-media`, offline rendering in `freecut-audio`.

### AU-033 Stems and separate-audio export — later
**What the user sees:** Export options to:
- Write the mixed audio as a separate file alongside the exported video.
- Export one audio file per track ("stems").
- Export grouped stems (dialogue, music, effects), grouped by clip origin tag.
**Inputs / controls:** The same format controls as AU-032, plus the stem selection.
**Behavior / output:** Each stem is the track's output after the track stage and before the master limiter. Files are named `<project>_<track>.<ext>`.
**Edge cases:**
- Empty tracks are skipped.
- The master limiter is not applied to stems (a notice explains this).
- Solo rules follow AU-012.
- Two tracks with the same name: a numeric suffix keeps the file names unique.
**Undo:** n/a (output, not an edit).
**Model notes:** An extension of AU-032's job in `freecut-audio` and `freecut-media`.

### AU-034 Basic audio filters (EQ, filters, dynamics) — later
**What the user sees:** A per-clip "Audio filters" stack, separate from voice effects, offering:
- A 3-band EQ: low shelf, mid peak and high shelf.
- High-pass and low-pass filters.
- A compressor.
- A noise gate.
- Reverb and delay.

Filters can be reordered and bypassed.
**Inputs / controls (FreeCut defaults):**
- **3-band EQ:** gain ±15 dB per band (default 0 dB). Low shelf 20 to 500 Hz (default 100 Hz); mid peak 200 Hz to 8 kHz (default 1 kHz) with Q 0.3 to 10 (default 1.0); high shelf 2 to 20 kHz (default 8 kHz).
- **High-pass / low-pass:** cutoff 20 Hz to 20 kHz (defaults 80 Hz and 12 kHz), slope 12 or 24 dB/octave (default 12).
- **Compressor:** threshold −60 to 0 dBFS (default −18), ratio 1:1 to 20:1 (default 3:1), attack 0.1 to 100 ms (default 10), release 10 to 1000 ms (default 150), make-up 0 to +24 dB (default 0).
- **Noise gate:** threshold −80 to −20 dBFS (default −50), range 0 to −80 dB (default −40), attack 0.1 to 50 ms (default 1), hold 0 to 500 ms (default 50), release 10 to 1000 ms (default 200).
- **Reverb:** decay 0.1 to 10 s (default 1.5), mix 0 to 100 % (default 25). **Delay:** 1 to 2000 ms (default 300), feedback 0 to 90 % (default 30), mix 0 to 100 % (default 25).
- Every filter has a bypass toggle. Parameters are stored as integers (Hz, tenths of a dB, ms, %).
**Behavior / output:** Inserted at step 7 of the chain, after the voice effect. Real-time.
**Edge cases:**
- An empty stack has no cost.
- Extreme high-Q EQ settings are bounded for stability.
- Reverb or delay tails are cut at the clip end, as in AU-022.
- Third-party plugin hosting (VST3, CLAP, LV2) is out of scope.
**Undo:** Add, remove, reorder and set-parameter ops on the filter stack. Parameter drags are coalesced.
**Model notes:** `ClipAudio.filters: Vec<AudioFilter>` in `freecut-timeline`. DSP in `freecut-audio`.

### AU-035 Waveform-based sync of external audio — later
**What the user sees:** Select clips that share a recorded moment, then choose "Sync by sound". The clips move so their sound lines up. An option mutes the camera sound afterwards. A toast reports the result with counts, for example "Synced 4 of 5 clips · 1 had no match".
- **One reference, many followers:** the reference is the clip the others line up to. With one audio clip and several video clips selected, the audio clip is the reference and the video clips move to it, as when several cameras filmed one recorder's take. With one video clip and several audio clips, the video clip is the reference and the audio clips move. With any other mix, a small dialog asks which clip is the reference.
**Inputs / controls:**
- Reference clip: chosen automatically as above, or picked in the dialog.
- Search window ±60 s (default), adjustable up to ±10 min.
- "Mute camera audio after sync" (default on). It mutes the embedded sound of the video clips (AU-007), whether they moved or were the reference.
**Behavior / output:** For each follower, the loudness envelopes of the follower and the reference are cross-correlated in the background. The follower then moves by the best offset, rounded to the frame grid (TL-001). The leftover error is at most half a frame, which is within normal lip-sync tolerance. Sub-frame placement depends on the open TL request in AU-001.
- Followers are processed one by one, with progress ("Syncing 2 of 5") and Cancel.
- Multicam angle editing is out of scope (timeline-editing.md), but this same sync is what a future multicam feature would use to line up angles.
- Reference: the observed reference app lines video clips up with a reference audio track by their sound, uses the same sync for multicam, and reports success, cancel and failure with per-clip counts, including partial success.
**Edge cases:**
- **Low confidence for one follower (no clear peak):** that follower doesn't move and is listed as "no match". The others still sync.
- **Long recordings with clock drift:** synced at the start only, with a warning if drift is detected (a drift fix comes later).
- **Clips on locked tracks:** they can't be selected (TL-009), so they are never moved.
- **Target position would overlap another clip on the follower's track:** the clip moves to the first free track of its kind instead (TL rules).
- **Cancel:** followers already synced stay synced (as one entry). The rest are left alone, and the toast says so.
- **Follower or reference without sound, or silent:** skipped with AU-040's reason.
- **Follower linked to detached audio (TL-030):** only the selected clip moves. The out-of-sync badge shows the new offset, and TL-030's Resync can bring the partner along.
**Undo:** One compound entry per run. It holds TL's move ops (TL-018, including any track change) for every follower that synced and, optionally, AU-007's `SetClipMuted` on the camera clips.
**Model notes:** AU owns this feature: audio-based sync and auto-alignment of separately recorded audio. Timeline-editing.md keeps only multicam angle editing out of scope, and points here for sync. The correlation runs in `freecut-audio` as a background job on AI-001's scheduler, one job per follower. It returns offsets in rational ticks with a confidence value each, and AU only emits ordinary TL move ops, with no sync-specific timeline state. A pure planner, `plan_sync_moves(seq, reference, results) -> ChangeSet`, applies the successful offsets and is unit-tested for partial results.

### AU-036 Keyboard volume steps — v1
**What the user sees:** With one or more clips selected, a key press raises or lowers their volume without opening the inspector. Each press changes the level by 1 dB, or by 0.1 dB with Shift added. A short status toast shows the result:
- One clip: its new level, for example "Volume −4.0 dB".
- Several clips: the total step and the count, for example "Volume +3.0 dB · 4 clips".
- It adds "at limit" when a clip hit the range end, and "(muted)" when the clip is muted.

The inspector's Volume field and the gain overlay (AU-009) update live. The four commands are also in the command palette and in the menus that workspace-and-shortcuts.md lays out.
**Inputs / controls:**
- **Commands:** Volume up 1 dB, Volume down 1 dB, Volume up 0.1 dB, Volume down 0.1 dB. The steps are fixed in v1 (FreeCut default) and match the arrow-key steps of AU-005's field.
- **Proposed default bindings** (Editor scope), to be added to UI-031's table, which wins on conflicts:

  | Command | Windows / Linux | macOS |
  |---|---|---|
  | Volume up / down 1 dB | `Alt+=` / `Alt+-` | `⌥=` / `⌥-` |
  | Volume up / down 0.1 dB | `Alt+Shift+=` / `Alt+Shift+-` | `⌥⇧=` / `⌥⇧-` |

  Keys are matched by physical key (UI-030). When a text field has focus, the key goes to the field (UI-030's field-first rule). Inside AU-005's Volume field, its own arrow keys apply.
- Reference: third-party shortcut lists for the reference desktop editor report a modifier plus punctuation-key pair that nudges the selected clip's volume in 0.1 dB steps, and its observed UI labels confirm volume up and down commands of 0.1 dB. FreeCut picks different keys, because UI-031 already binds `,` and `.` to frame nudges and `Ctrl+,` to Preferences.
**Behavior / output:**
- **Targets:** every selected clip with sound, meaning audio clips and video clips whose embedded audio is `Active` (TL-030). Group expansion applies (TL-015, TL-039).
- **Relative step:** each target's level changes by the same step, so the balance between clips is kept (like AU-005's Alt-drag). It is integer arithmetic on `GainDb`: 1 dB = 100 units and 0.1 dB = 10 units, so steps are exact.
- **Range:** AU-005's range, −60.0 to +20.0 dB plus −∞.
  - A result above +20.0 dB is clamped to +20.0 dB.
  - A result below −60.0 dB becomes −∞ (silent).
  - A static level at −∞ counts as starting one step below −60.0 dB, using the size of the run's first step. So the first up press lands exactly on −60.0 dB.
- **Clips with volume keyframes (AU-008):** the step shifts the static value and every keyframe value by the same amount, including keys outside the trimmed range. The envelope keeps its shape. Each key follows the range rules above, except that keys already at −∞ stay at −∞. This differs on purpose from typing in AU-005's field, which edits the key at the playhead, because the playhead may be outside the selected clips.
- **Coalescing (TL-043):** the four commands count as one command for TL-043's keyboard-repeat rule. Presses and key auto-repeat on the same set of clips within 1 s of each other build a single history entry, such as "Volume +3.0 dB (2 clips)".
  - Within one coalesced run, each value is computed from the clip's original value plus the run's total step, with the range rules above applied once at the end. It is never clamped press by press, so going past a limit and coming back restores the earlier balance exactly.
  - A different selection, another command, or a pause longer than 1 s starts a new entry.
- **During playback:** the change is heard at once, with AU-005's 10 ms smoothing.
- The step edits only the user volume. Normalization gain (AU-017), ducking (AU-028) and fades multiply on top as usual.
**Edge cases:**
- **No selection, or no selected clip has sound:** nothing changes, and the status bar says "Select clips with sound". The command never falls back to the clip under the playhead.
- **Images, text and stickers in the selection:** ignored.
- **Video clip whose audio is detached:** ignored. Its detached audio clip is stepped only if it is also selected, because TL-030 does not auto-select linked partners.
- **Locked tracks:** their clips can't be selected (TL-009), so they never change.
- **Muted or disabled clip (AU-007, TL-036):** the stored level changes, and the clip stays silent.
- **Some clips reach a limit while others don't:** each is clamped on its own, and the toast says "at limit". Undo restores every original value.
- **Clip with normalization on:** no new analysis is needed, because the measurement happens before the volume stage (AU-017).
- **Stepped clip is a speech source for ducking (AU-028):** the speech level changes, so the ducked clips show "needs update".
- **Key held down:** each auto-repeat is one step. The whole hold is one history entry.
- **Inspector Volume field focused:** these chords type into the field, or do nothing there, per UI-030. AU-005's arrows are the field's own steps.
**Undo:** `StepClipGain { clips, total_step: GainDb, before: Vec<Animatable<GainDb>>, after: Vec<Animatable<GainDb>> }`. Coalescing merges consecutive steps on the same clip set: it keeps the first `before`, adds up `total_step`, and recomputes `after` from `before`. Undo restores `before` exactly. Redo reapplies `after`.
**Model notes:**
- A pure planner, `plan_gain_step(seq, clips, step: GainDb, base: Option<&StepClipGain>) -> Result<ChangeSet, EditError>`, lives in `freecut-timeline`. It is unit-tested for:
  - Clamping at +20 dB, results below −60 dB becoming −∞, and the first up step from −∞ landing on −60 dB.
  - Shifting a keyframed envelope, with keys at −∞ left alone.
  - Ignoring clips without sound.
  - A run that goes past the ceiling and back, which must restore the balance.
  - 0.1 dB steps being exact.
- The commands, the toast and the menu entries live in `freecut-app::commands`. The binding rows belong to UI-031's `default_keymap()`.

### AU-037 Echo reduction — v1
**What the user sees:** An inspector toggle, "Reduce echo", with a Strength slider, between Noise reduction and Voice polish. A hint says it is meant for speech recorded in rooms with hard walls, and that it shortens the room's tail rather than removing it completely. After the first playback or analysis, a small readout shows the estimated room echo, for example "Room echo ≈ 0.7 s".
**Inputs / controls:**
- On/off, default off.
- **Strength 0 to 100 %, default 50 %** (FreeCut default). Strength sets the maximum suppression of the late echo: 0 % is a bypass, and 100 % allows up to 20 dB of reduction (FreeCut default mapping).
- Multi-select: the toggle and strength apply to every selected clip with sound (AU-040's batch rule).
**Behavior / output:**
- Classic signal processing written by FreeCut in Rust, with no ML model, built from published textbook methods:
  - short-time spectral analysis;
  - a blind estimate of the room's decay time (T60), taken from the slope of the free decays after speech stops;
  - a statistical model of late reverberation, an exponential decay of past spectral power delayed by about 50 ms, so the direct sound and early reflections are kept;
  - a smoothed per-band suppression gain whose floor is set by strength.
- Runs in source time at chain step 4, after noise reduction and before voice polish, so its output can be cached per media range and settings, like AU-018's.
- Real time, with the same latency compensation and pre-render fallback as AU-018.
- For heavy echo, Voice clarity (AU-020) is the ML option. A stronger multi-channel linear-prediction dereverberation mode (WPE-style) is a later option.
- Reference: the observed reference app offers a per-clip echo and reverb reduction option for voice recordings, and lists echo removal as part of its paid voice enhancement. FreeCut's control, name and processing are its own.
**Edge cases:**
- **Clip shorter than about 2 s, or no pauses to measure:** the decay estimate falls back to 0.5 s (FreeCut default) and the readout shows "≈ 0.5 s (assumed)".
- **Little or no echo** (estimated T60 below 0.15 s, as with outdoor or close-mic speech): the stage passes the audio through, and the readout says "No echo found".
- **Music or intended reverb:** note tails are shortened. The hint discourages it.
- **High strength:** speech can sound dry or choppy between words. Gain smoothing and the strength floor limit this, and the hint suggests lowering strength.
- **Stacked with noise reduction:** both run. The combined suppression in any band is limited to 35 dB (FreeCut default) to avoid a hollow sound.
- **Reversed or retimed clip:** works unchanged, because it runs in source time.
- **Detach or restore (AU-003):** the setting moves with `ClipAudio`.
- **Real-time budget exceeded:** pre-rendered in the background, as in AU-018.
**Undo:** `SetEchoReduction { clips, old: Vec<Option<Strength>>, new: Option<Strength> }`. A strength drag coalesces (TL-043). The T60 estimate is a derived cache, not on the undo stack.
**Model notes:** `ClipAudio.dereverb: Option<Strength>` (None = off) in `freecut-timeline`. The DSP lives in `freecut-audio` and is tested with fixture signals: dry speech convolved with synthetic exponential-decay impulse responses of known T60, checking the decay-time reduction and that a dry input passes through unchanged. Output goes to AU-018's processed-audio cache.

### AU-038 Multi-stream sources: stream notice, picker and stream as clip — v1
**What the user sees:**
- When an imported file has more than one audio stream, a notice says how many were found and which one plays by default. Typical files are dual-mic camera clips, screen recordings with separate system and microphone sound, and films with several language tracks. When several files are imported at once, the notice is a line in PM-037's import report instead of a separate toast.
- A **Sound source** picker in the clip's Audio section and in its context menu lists each stream (number, language and title from the file, channel layout) plus **All streams mixed**.
- A context-menu command, **Add stream as clip ▸ <stream>**, puts one stream on its own audio track under the clip.
**Inputs / controls:**
- **Picker:** stream 1 to n, or All streams mixed. Default: the media item's stream from PM-040 (Auto means the stream the file flags as default, otherwise the first).
- **Add stream as clip:** one stream per command. With several video clips selected, it adds the same stream number for each clip that has it.
**Behavior / output:**
- The picker sets the clip's stream selection (TL-011), overriding PM-040's media-level default for this clip only. Changing it swaps the decoded stream at chain step 1. Every other `ClipAudio` setting stays.
- **All streams mixed:** each stream is downmixed to stereo (AU-010), and the streams are summed with a gain of 1/√n (about −3 dB each for two streams; FreeCut default), so the level stays close to a single stream's.
  - v1 engine (FreeCut defaults, `freecut-audio`): each stream is its own mixer input with its own reader. A clip playing one stream is keyed by its clip id (so switching streams keeps the slot and ramps); each stream of an "All streams mixed" clip is keyed by `stream_input_key(clip, stream)`, which sets the top bit and puts the container index (modulo 512) above the 54 clip-id bits, so it never equals a clip id and stays stable across edits.
  - FreeCut default (engine, 2026-10-12): switching a playing clip to **All streams mixed** never dips its level. Every mixer input names its clip (`MixInput::clip`), so the mixer knows a clip's streams. The stream that was playing keeps its reader and its level while the other streams' readers, opened about 50 ms ahead of the mix, are on their way; from the sample they join at, it ramps to 1/√n as they ramp in from silence, over the usual 10 ms. A stream that can't be opened holds nothing back. Switching back to one stream ramps at once, the other streams ramping out.
- v1 app (FreeCut defaults, `freecut-app`): the import notice is a status-line sentence: "<file> has N audio streams; the default one (stream k) plays. Choose another in the inspector". Streams are named "Stream n · language · title · layout", with "mono", "stereo" or "N ch" when the file names no layout. The Audio section's **Sound source** dropdown and the clip menu's **Sound source ▸** list every stream plus "All streams mixed" (from two streams); the file's default stream is offered as "follow the default" (`sel: None`), every other one by container index. The menu's choice goes to every selected clip (`SetAudioStream`, one undo entry). Both are disabled with the reason on a video whose sound is detached ("its audio clips play the stream") and on a locked track. **Add stream as clip ▸** appears when a selected video clip's file has several streams; each entry is dry-run planned so it says what will happen: the stream the clip plays reads "(playing): Detach audio" and runs Detach audio (disabled with the reason when the sound is already detached); a stream already on an audio clip of the video's link is disabled ("This stream already plays on its own audio clip"), so it is never doubled; with several videos selected, each one that has that stream number gets its clip, all in one undo entry, and the new clips are selected.
- v1 app follow-up (FreeCut defaults, `freecut-app`, 2026-10-12): the timeline waveform follows the stream the clip plays: peaks are cached per (media item, container stream) and a clip asks for those of `played_streams(sel)`, so picking another stream redraws from that stream. **All streams mixed** draws the first stream it plays (no mixed peaks are computed); the Audio tab row shows the file's default stream. "Add stream as clip" relies on the model's "already plays" rule alone (timeline overlap, current selections, muted clips included): an entry is refused only when the dry-run plan refuses it. When the stream plays through the video's own or detached sound the entry offers Detach audio (or says the detached clip plays it); when an audio clip added from that stream plays it, the entry reads "This stream already plays on its own audio clip".
- FreeCut default (model and app, 2026-10-12): the Sound source picker never doubles a stream either. A clip can't switch to a stream (or to **All streams mixed**, which plays them all) that another clip of its link already plays at the same time: a stream added as a clip, or a piece of detached sound. `SetAudioStream` skips such clips like the others it skips and is refused only when no listed clip can change, with `StreamPlaysElsewhere { clip, stream, by }` naming the clip that already plays it (after `UnknownStream`, before `SoundDetached`); keeping the current choice is never refused. The inspector dropdown and the menu's **Sound source ▸** show those entries disabled, "Already playing on its own clip" on hover (from the read-only `stream_doubled_by` helper; with several clips selected, only when none of them could take it). Add stream as clip refuses (`SourceOutOfRange`) to reuse an extracted item that was relinked on its own to a file too short for the video's source range.
- **Add stream as clip** creates an audio clip whose media is PM-041's virtual `MediaSource::AudioOf { source, stream }`, the same item AU-004 creates (an existing item for that stream is reused). It has the same timeline range, source range and a copy of the retime, and goes on the first free audio track below (or a new track, TL-006). It joins the video clip's link (TL-030's `LinkId`), so the out-of-sync badge works. The video clip keeps playing its own selected stream.
- Analysis caches (normalization, beats, stems, clarity, channel auto-fill) are keyed by stream, so a stream change reuses or starts the right analysis.
- Reference: observed in the reference app: a notice when a file with several audio streams is detected, a context-menu stream switch, and separating a stream onto its own track.
**Edge cases:**
- **Add stream as clip with the stream the clip already plays:** the command says that stream is already playing and offers Detach audio (TL-030) instead, so the sound is not doubled.
- **Streams with different sample rates or channel layouts:** handled by the mixer (AU-010).
- **Streams of different lengths:** silence after the shorter one ends, as in AU-002.
- **Media relinked to a file with fewer streams:** the clip falls back to the default stream with a warning. Clips made with Add stream as clip whose stream is gone show as offline (TL-011).
- **Only one audio stream:** the picker shows just that stream, and Add stream as clip is hidden, because Detach audio covers it.
- **Audio-only files with several streams** (for example multitrack MKA files): the same picker appears on the audio clip.
- **Video clip with detached audio:** the picker is disabled with AU-040's reason. The stream belongs to the detached clips.
- **Locked track:** unavailable (TL-009).
**Undo:** `SetAudioStream { clips, old: Vec<StreamSel>, new: StreamSel }`. Add stream as clip is one compound entry (media item if new, clip insert, optional new track, link). The import notice is not an edit.
**Model notes:** `StreamSel { Index(u16), AllMixed }` is the stream selection on the clip's media reference (TL-011). AU defines the `AllMixed` variant, which PM-040 refers to. Stream metadata comes from PM-035's probe in `freecut-media`. The mixing maths lives in `freecut-audio`. A pure planner, `plan_stream_as_clip(seq, clip, stream) -> Result<ChangeSet, EditError>`, is in `freecut-timeline` and unit-tested for track placement, linking and the "already playing" refusal.
- v1 model (FreeCut defaults, `freecut-timeline`): `StreamSel::Index` holds the container index as `u32`, like every other stream reference; the selection is `ClipContent::Media.stream: Option<StreamSel>`, `None` meaning the media default. `AudioStreamInfo` gains `default`, `language`, `title` and `layout` from the probe, and `MediaInfo::default_audio()` is the stream flagged default, else the first. `MediaInfo::played_streams(sel)` falls back to the default stream for an index the (relinked) file no longer has; the selection itself is kept. `ImportMedia` reports `Notice::MultipleAudioStreams { media, count, default_stream }` per item with more than one stream. `SetAudioStream { seq, clips, sel }` (`sel: None` goes back to the default) skips clips without sound, video clips whose sound is detached, and clips whose media lacks the chosen index; when none is left it is refused with the most specific reason (`UnknownStream`, then `SoundDetached`, then `NoClipAudio`). Detaching copies the video's selection to the new audio clip. `AddStreamAsClip { seq, clip, stream }` works on one video clip: refused for a clip that isn't video (`NotVideo`), without sound, for an unknown stream, for media with a single stream (`SingleStream`) and, after that check, for a stream the clip already plays, also through detached sound or "All streams mixed" (`StreamAlreadyPlaying`). FreeCut default (model, the "never doubled" guarantee): a stream "already plays" when the video's own active sound plays it (its selection, "All streams mixed" included), or when an audio clip of the video's link (or of its detached link) that overlaps the video in time plays it: a stream added as a clip before (its `AudioOf` item), or a piece of the detached sound by that clip's *current* selection (the video's own selection is frozen while its sound is detached, so it isn't read). A linked clip elsewhere in time, or a deleted one, doesn't count. The app's dry-run plan therefore covers every case; its own link check is only for wording. The new clip reads the stream's extracted item (AU-004's, reused when it exists), copies the video's range, source range and enabled state, starts with default sound settings and is selected. It goes on the first free unlocked audio track below every track holding a clip of the video's link (from the top when none does), else on a new audio track at the bottom of the audio band. It joins the video's link; a video without one gets a new link, and a detached video whose link dissolved reuses its detached link id. Restore (TL-030) removes every audio clip of the link, streams added as clips included, and takes the settings from the clips that read the video's own file. `mix_sources` lists an "All streams mixed" clip once per stream with `mixed = n`, and the audio crate applies the 1/√n gain.

### AU-039 Mute all video sound and project mute — v1
**What the user sees:**
- **Mute video sound:** a command in the Audio menu and the main track header's context menu that mutes the embedded sound of every video track at once. When every video track is already muted, it reads "Unmute video sound". A toast confirms the new state, for example "Video sound muted (3 tracks)".
- **Project mute:** a toggle in the Audio menu and next to the master volume (AU-010). When it is on, the whole project is silent in preview and in export, the master strip shows "Muted", and a badge stays on the player's master control. Turning it on asks for confirmation, because it also silences exports.
**Inputs / controls:**
- **Mute video sound:** a command. It targets every video track (the main track and overlay video tracks) in the open sequence.
- **Project mute:** on/off, default off. The confirmation reads "Mute the whole project? Exports will have no sound until you unmute." and has a "Don't ask again" box, stored as an app preference.
**Behavior / output:**
- **Mute video sound** sets TL-008's `muted` flag on each video track. If any video track is unmuted, all become muted. Otherwise all become unmuted (the same rule as AU-007's mixed selection). Audio tracks are untouched, so detached sound and stem clips keep playing. New video tracks created later start unmuted (TL-006), because this is a one-time command, not a mode.
- **Project mute** sets `MasterAudio.muted` (AU-010) and silences the master stage (chain step 15) with a 10 ms ramp. Clip, track and master settings are untouched, so unmuting brings everything back exactly.
- **Export with project mute on:** export preflight (preview-and-export.md) warns "Project is muted — the export will be silent", with "Unmute and export" and "Export silent". Audio-only export (AU-032) does the same.
- Unlike preview mute (EX-019), project mute is saved with the project, is undoable and affects export.
- Reference: the observed UI labels show a toggle that mutes or unmutes the original sound of every main-track clip, with a confirmation toast for each state, and a project-wide mute that warns before it turns on. FreeCut's main-track mute (AU-011) and Mute video sound cover the first; project mute covers the second.
**Edge cases:**
- **No video tracks with sound:** Mute video sound is disabled with "No video sound to mute".
- **Locked video tracks (TL-009):** whether their mute flag can change is TL-008's and TL-009's rule. If TL forbids it, those tracks are skipped and the toast says how many.
- **Compound clips (TL-040):** Mute video sound acts on the open sequence only, and nested sequences keep their own flags. Project mute is stored on the root sequence and silences the whole mix, nested compound clips included.
- **Project mute with solo active:** silent.
- **Meters (AU-015):** they read after the mute, so they show silence and the master strip's "Muted" label explains why.
- **Scrubbing and voiceover recording while project mute is on:** scrubbing is silent. Recording still captures the microphone (AU-027), and the panel shows that the timeline is muted.
**Undo:** Mute video sound is one compound entry of TL-008's `SetTrackFlags`, one per changed track, labelled "Mute video sound" or "Unmute video sound". Project mute is `SetMasterMute { old, new }`.
**Model notes:** `MasterAudio.muted: bool` (default false) in `freecut-timeline` (AU-010). A pure planner, `plan_mute_video_sound(seq) -> ChangeSet`, is unit-tested for the mixed-state rule and for skipping tracks TL refuses. `freecut-audio` applies the master mute after the limiter. `freecut-app` owns the confirmation, the toast and the preflight hook.

### AU-040 Audio tool availability and batch results — v1
**What the user sees:** Every audio tool that can't work on a clip says why, in the same words everywhere. The control is disabled with the reason in its tooltip, and the command is greyed in menus with the same reason (UI-030's disabled-reason rule). When a tool is applied to several clips and some can't take it, the others are processed and a toast reports both, for example "Noise reduction on 4 clips · 2 skipped (compound clips)".
**Inputs / controls:** None of its own. It governs every per-clip tool in this spec.
**Behavior / output:**
- **Availability table** (FreeCut defaults):

| Clip kind or state | Level tools: volume, fades, mute, pan, keyframes, voice effects (AU-022), filters (AU-034), keyboard steps, ducking target (AU-028) | Source tools: channel auto-fill, noise and echo reduction, voice polish, clarity, isolation, stems, normalization, beat detection, sync, ducking source | Reason shown when unavailable |
|---|---|---|---|
| Audio clip, video clip with active embedded audio, stem clip, recording, speech clip (AI-025), starter-pack track | Yes | Yes | — |
| Reversed or retimed clip | Yes | Yes. Source tools work in source time, before the retime stage. | — |
| Compound clip (TL-040) | Yes | No | "Works on single sources. Open the compound clip to apply it to the clips inside." |
| Video clip whose sound is detached | No | No | "The sound is on the detached clip", with a "Select it" link |
| Media offline | Settings can be edited | Jobs can't start | "Media offline" |
| Silent source (see below) | Yes | No for analysis tools (normalization, beats, isolation, stems, sync, ducking source, auto-fill). Yes for DSP toggles, which have no audible effect. | "No sound in this source" |
| Model not installed (clarity, isolation, stems) | — | Shown; using it opens AI-002's install prompt | — |
| Images, text, stickers, freeze-frame stills | No Audio section | No | — |

- **Silent flag:** a stream whose absolute peak never exceeds −80 dBFS (FreeCut default) is flagged silent. It is computed from the peak data that the waveform job already builds (PM-072, AU-009), so it costs no extra decoding. It is a derived fact about the media, not an edit, and it is redone when the media changes on disk.
- **No audio in the project:** a command that needs a source when nothing qualifies (for example Detect beats from the toolbar with no clip with sound) says so in the status bar, and suggests importing a file with sound or extracting it from a video (AU-004).
- **Batch rule:** with several clips selected, the eligible clips get the change as one undo entry, and ineligible ones are named in the toast and in PM-073's row details. For background jobs (normalization analysis, clarity, isolation, stems, sync), the successful part is applied as one entry when the batch ends, and failed clips are listed with Retry (AU-021, AU-035). Retrying makes a separate entry.
- **Job presentation:** AU's jobs report through AI-001 and PM-073 and use their states: Waiting (with the queue position), Running n %, Re-running after an edit invalidated the result, Done, Cancelled (no change) and Failed (with Retry). AU defines no other states. Until a job is done, preview plays the unprocessed audio with AI-001's badge, and export waits.
- **No usage limits:** there are no counters, quotas, caps on clips per batch or caps on clip length. Long jobs show AU-021's time estimate instead.
- Reference: for the reference app's AI audio tools (isolation, separation, enhancement, voice effects, normalization), the observed UI labels show refusals for compound, reversed, template, built-in-library and generated-avatar clips and for photos, caps on clip length and batch size, and a waiting, percentage, done, cancelled, failed and partial-failure job cycle. FreeCut keeps the pattern of refusing with a reason, but only for the cases in the table. FreeCut has no cloud templates or avatar clips, and its library tracks are open-licensed, so they are processed like any other file.
**Edge cases:**
- **Selection with no eligible clip:** nothing changes, and the toast gives the first reason.
- **A clip loses eligibility after a setting was applied** (its media goes offline, or is relinked to a silent file): the stored setting is kept, the stage is bypassed, and the inspector shows the reason. Relinking to a file with sound brings the setting back into effect.
- **Paste attributes (TL-034) onto ineligible clips:** those attributes are skipped, with the same toast.
- **Mixed selection in the inspector:** controls show the state of the eligible clips. A control that no selected clip can use is disabled with the reason.
**Undo:** n/a for the rules themselves. Each tool keeps its own op, and a batch is one entry of that op.
**Model notes:** A pure function, `audio_tool_availability(clip, tool, facts: &MediaFacts) -> Result<(), Unavailable>`, lives in `freecut-timeline`. `MediaFacts` (offline, silent, model installed) is passed in by `freecut-app`, so the timeline crate stays free of I/O. `Unavailable { Compound, Detached, NoSound, Offline, Silent, ModelMissing }` has one reason string each in the string catalog (UI-044). The function is unit-tested as a table, one case per row above. A compound clip's own `ClipAudio` applies to its nested mix, and only the level tools' fields take effect there. The silent flag is computed in `freecut-media` from the peak pyramid.

### AU-041 Before/after compare for audio processing — v1
**What the user sees:** A **Compare** button in the header of the inspector's Audio section. It is enabled when the selected clip has at least one processing setting on: noise or echo reduction, voice polish, clarity, isolation, a voice effect or filters. Holding the button, or clicking it to latch it, plays the clip without that processing. Releasing it, or clicking again, returns to the processed sound. While it is active, a "Before" label shows on the player.
**Inputs / controls:**
- **Press and hold:** compare while held. **Click:** latch on or off. Esc, selecting another clip or making any edit turns it off.
- **Scope:** This clip (default) or All clips, which bypasses processing on every clip to judge the whole mix.
- No default shortcut. The command is in the command palette, and UI-031 may bind it.
**Behavior / output:**
- While active, preview skips chain steps 3, 4 and 7 for the compared clips (clarity, noise and echo reduction, voice polish, voice effect and filters), and step 1 decodes the original instead of an isolation stem. Volume, normalization gain, keyframes, ducking, fades, pan, retime and mutes still apply, so before and after play at the same level and timing.
- Compare doesn't start playback. It changes what plays, and it works while playing, looping or scrubbing.
- The switch is crossfaded over 20 ms (FreeCut default). The processed and bypassed paths share one decode, so they stay in sync.
- Preview only: never saved, never applied to export, never on the undo stack.
- Reference: the observed reference app offers a before/after preview for its noise removal and voice isolation.
**Edge cases:**
- **Processing not ready yet** (a job is running): before and after sound the same, and the button shows the job's progress.
- **Stem clips from Split into stems:** the stem is their media, so Compare doesn't bring back the original mix. It only bypasses the stem clip's own processing. Unmuting the source clip is the way to hear the original (AU-021).
- **Real-time load:** bypassing only reduces load.
- **Export starts while Compare is latched:** export is unaffected, and Compare turns off.
- **Solo (AU-012):** respected. Comparing a clip on a track that isn't soloed is silent, and the button's tooltip says why.
**Undo:** n/a (view state).
**Model notes:** A preview-only bypass set (clip ids, or All) passed from `freecut-app` to `freecut-audio`, like the solo set (AU-012). There are no model fields.

### AU-042 Audio auto-fix assistants — later
**What the user sees:** Two commands in the Audio menu. Each analyses the project first, then opens a dialog that lists the proposed change for every clip, with a checkbox per clip, before anything is applied.
- **Clearer speech:** finds clips with speech, measures their background noise and room echo, and proposes noise reduction (AU-018), echo reduction (AU-037) and, when its model is installed, voice clarity (AU-020), each with a strength.
- **Even loudness:** finds clips that are much louder or quieter than the rest. It proposes loudness normalization (AU-017) to one target for speech clips, and a lower target for music and sound effects, so music sits under speech.
- After applying, **Remove auto-fix** in the Audio menu undoes only what the assistant changed, after a confirmation.
**Inputs / controls (FreeCut defaults):**
- **Scope:** the whole timeline (default) or the selected clips.
- **Clearer speech:** Gentle, **Normal** or Strong, which scales the proposed strengths.
- **Even loudness:** speech target (AU-017's presets, default −16 LUFS) and a music offset from 0 to −20 LU (default −6 LU).
**Behavior / output:**
- The analysis runs as AI-001 jobs, one per clip, with progress and Cancel. It uses classic measurements only, with no model weights:
  - speech or music, from published speech/music discrimination features (4 Hz modulation energy, variance of spectral flux and the share of low-energy frames). If AI-009 transcripts exist, they confirm speech.
  - the noise floor, from AU-018's tracker, and the room decay, from AU-037's estimate.
  - integrated loudness, from AU-017's measurement.
- **Proposals:**
  - noise reduction when the speech-to-noise ratio is below 25 dB, with a strength that grows as the ratio falls;
  - echo reduction when the estimated decay is above 0.4 s;
  - voice clarity only when its model is installed and the speech-to-noise ratio is below 10 dB;
  - normalization for every speech clip more than 3 LU away from the target, and for music and effects more than 3 LU away from target + offset.
- **Apply** sets ordinary `ClipAudio` attributes in one undo entry. It also stores an `AutoFixRecord` with an id, the assistant's kind and, for each changed attribute, the value before and the value it set.
- **Remove auto-fix** restores the earlier value of each recorded attribute that still holds the value the assistant set. Attributes the user has changed since are left alone, and the dialog lists them. It is one entry.
- **Running an assistant again** replaces its previous record. It first reverts its earlier changes under the same rule, so repeated runs never stack.
- Reference: the observed UI labels describe two one-click assistants. One analyses all clips for voice, background and noise, then applies enhancement and noise reduction. The other finds overly loud or quiet sections, then applies normalization and noise reduction. Both can be removed as a unit, after a confirmation. FreeCut's version shows its proposals before applying them, and runs locally.
**Edge cases:**
- **No speech found:** Clearer speech says so and proposes nothing.
- **Silent, offline or compound clips:** skipped with AU-040's reasons.
- **A fixed clip is deleted, then the auto-fix is removed:** the deleted clip is ignored. Undoing the delete later brings it back with the assistant's values, and the record still covers it.
- **Clip split after apply:** both pieces carry the assistant's values. The piece that keeps the original clip's identity under TL-023 is reverted by Remove auto-fix. The other piece is a new clip that the record doesn't cover, so it keeps the values, and the dialog lists it as left alone (FreeCut default; following split lineage is a possible later refinement).
- **Long timelines:** analysis progress is shown with Cancel. Cancelling leaves the project unchanged.
- **Clips whose values already match the proposal:** listed as "No change needed" and left out of the record.
**Undo:** `ApplyAutoFix { record, changes }` and `RemoveAutoFix { record, restored }`, one entry each. A rerun is one entry that holds both the revert and the new apply.
**Model notes:** `AutoFixRecord { id, kind: AutoFixKind { ClearerSpeech, EvenLoudness }, entries: Vec<(ClipId, AttrPath, before, set)> }` is stored in AU's `Sequence.audio` slot (`MasterAudio.auto_fix`, AU-010) and serialized by `freecut-project`. Pure planners, `plan_apply_auto_fix` and `plan_remove_auto_fix`, live in `freecut-timeline` and are unit-tested for the "only untouched values" rule and for split clips. The measurements and the speech/music classifier live in `freecut-audio` and run as AI-001 jobs.

## Interactions with other areas
- **Timeline & core editing (TL, timeline-editing.md):**
  - **TL owns** time (TL-001), tracks and the track limit (TL-004), clips and stream selection (TL-011), selection, snapping (TL-028), grouping, markers (TL-037), lock (TL-009, v1), clip enable (TL-036), embedded-audio detach and restore (TL-030), waveform generation and display (TL-042), and the semantics of undo and coalescing (TL-043).
  - **Track mute and hide (TL-008)** is mvp: two persisted, undoable flags changed with `SetTrackFlags`. AU-011 (mvp) depends on it.
  - **AU's typed slots:** TL-003 and TL-004 add typed property-bag slots, `Sequence.audio` (AU's `MasterAudio`, AU-010) and `Track.audio` (AU's `TrackAudio`, AU-013). AU defines the types and the undoable ops that change them (`SetMasterGain`, `SetLimiter`, `SetMasterMute`, the auto-fix ops of AU-042, `SetTrackGain`, `SetTrackPan`).
  - **Mute video sound (AU-039)** only emits TL-008's `SetTrackFlags` on video tracks, so its rules on locked tracks are TL's.
  - **Stream selection (TL-011):** AU-038 sets it per clip and adds the `AllMixed` variant. Its "Add stream as clip" links the new clip with TL-030's `LinkId`.
  - **Compound clips (TL-040):** their `ClipAudio` acts on the nested mix, and source tools are unavailable on them (AU-040).
  - **Other AU additions:** `ClipAudio`, the audio crossfade object (AU-029), the settings hand-off on detach (AU-003) and the gain overlay on waveforms (AU-009).
  - **Split (TL-023):** keyframes follow TR-027 (all keys on both halves, no boundary keys). AU-008 adds nothing of its own.
  - **Beat markers:** AU owns beat detection (AU-025). Its results are stored only as TL-037 clip markers with origin Beat, in source time, with an auto or manual flag. TL's Interactions should list beat detection under AU-025.
  - **Coalescing:** TL-043's keyboard-repeat rule covers AU-036's volume steps.
  - **Sync of separate recordings:** AU-035 (later) owns audio-based sync and auto-alignment of separately recorded audio, and emits only TL move ops. TL's Out of scope keeps only multicam angle editing and points sync to AU-035.
  - **Stem splitting:** "Split into stems…" (AU-021) creates tracks (TL-006) and links the stem clips with TL-030's `LinkId`.
  - **Restore audio (TL-030):** TL removes the linked clips on restore. AU-003 records the observed alternative (keeping the separated clip), which FreeCut does not follow.
  - **Open requests to TL:** sub-frame audio trimming (AU-001), and copying beat markers to the detached clip (AU-025).
- **Transform, speed & keyframes (TR, clip-transform-and-speed.md):**
  - **TR owns** speed values, curves, reverse, freeze frames, the time map (TR-036) and its audio contract (TR-038).
  - **TR also owns** the keyframe container `Animatable<T: Interpolate>` (TR-027), with interpolation and source anchoring. On split, both halves keep all keys and no boundary key is inserted.
  - **Scalar types:** TR's shared conventions record the rule that only time must be rational, and that each owning spec picks its own non-time scalar types.
  - **AU owns** the Keep pitch toggle (AU-023, `ClipAudio.preserve_pitch`, default on, `SetPreservePitch`), which moves on detach (AU-003). It also owns the time-stretch and varispeed engine, its rule for reversed clips, and the `Interpolate` implementations for `GainDb` and `Pan` used for volume and pan keyframes (AU-008, AU-014).
- **Projects & media (PM, project-and-media.md):**
  - **PM owns** import and media items, including the extracted-audio source in AU-004 (`MediaSource::AudioOf`, also used by AU-038's Add stream as clip) and the PM-041 derived media for recordings (AU-027) and stems (AU-021, produced by AI-023's engine).
  - **PM also owns** the project sample-rate setting, the cache, generated-media and recordings folder locations, missing-media handling, relinking and Regenerate.
  - **Import:** accepted formats (PM-036), the import report (PM-037, which carries AU-001's "no sound" skips and AU-038's multi-stream notice), duplicate detection (PM-038), stream metadata (PM-035) and the media-level stream default (PM-040, which refers to AU-038's "All streams mixed").
  - **Peak data (PM-072):** AU-040's silent flag is computed from it, with no extra decoding.
  - **Format versions (PM-021):** projects that use newer audio features (stems, auto-fix records) follow PM-021's newer-version rule when opened in an older FreeCut.
  - **Takes list (AU-027)** removes recordings through PM-058. Cleanup of unused recordings and stem files is PM-074's.
- **Effects, transitions & animations (FX, effects-transitions-animations.md):**
  - **FX owns** the video transition object (FX-002) and audio under video transitions (FX-007). AU supplies the equal-power law that FX-007 uses, and decides what happens to a transition's audio on detach (AU-003).
  - **Effect keyframes on split** follow TR-027, as AU-008 does.
  - Video transitions are not allowed between audio clips, so AU owns audio-track crossfades (AU-029).
- **Preview & export (EX, preview-and-export.md):**
  - **EX owns** the player, shuttle and the export dialog. EX-002 (mvp) applies TL-008's hidden and muted rules.
  - AU supplies the audio clock (AU-010), scrubbing (AU-016), audio-only and stem export options (AU-032, AU-033), and the rule for solo during export.
  - **Project mute (AU-039)** asks export preflight for a "Project is muted" warning. Preview mute (EX-019) is separate: it is not saved and never affects export.
  - **Compare (AU-041)** is preview-only and never reaches the export render.
- **Smart / AI tools (AI, ai-tools.md):**
  - **Beats:**
    - AI owns beat-grid refinement (AI-005). Its `BeatGridParams` are stored on AU-025's beat-analysis record.
    - AI also owns auto beat sync (AI-006), which reads TL-037 beat markers, and auto montage (AI-029), which covers laying several media files out on the beats.
    - Two small requests to AI-001: an onset list on AU-025's Beats record (for the Every onset density), and a ChannelBalance job kind for AU-014's channel auto-fill.
  - **Engines:**
    - AI owns the separation engine (AI-023): model, job, quality, and FLAC stems as PM-041 derived media. AU-021 requests two additions: an optional 6-stem set (vocals, drums, bass, guitar, piano, other) and a Rest stem that sums the stems the user did not tick.
    - AI also owns the speech-enhancement engine (AI-024): DeepFilterNet, the strength mapping, the install prompt and the cache keys.
  - **Other AI features:** text-to-speech (AI-025), per-media transcription (AI-009, AI-010) and silence and filler-word removal (AI-003, AI-012).
  - **Infrastructure:** the analysis scheduler and cache (AI-001), the model manager (AI-002), and the proposed `freecut-analysis` crate, where all ML inference runs.
  - **AU owns the user-facing side:**
    - Beat detection, density and sensitivity, the tempo readout and the tap key (AU-025), with tap latency compensation (AU-026).
    - Voice isolation and "Split into stems…" (AU-021).
    - Voice clarity (AU-020).
    - The chain steps where processed audio enters: step 1 for stems, step 3 for clarity.
    - Noise reduction (AU-018) and echo reduction (AU-037), which stay model-free and have no ML or RNNoise mode.
    - Audio-tool availability and batch results (AU-040), and the auto-fix assistants (AU-042), which run their measurements as AI-001 jobs and may read AI-009 transcripts to confirm speech.
  - **Job states:** AU uses AI-001's and PM-073's states and badges (AU-040) and defines none of its own.
- **Text & captions (TX, text-and-captions.md):** captions generated from speech are TX's job, built on AI's per-media transcripts (AI-009, AI-010). AU only provides the audio.
- **Overlays & templates (OV, overlays-and-stickers.md):** templates that include music must use FreeCut's own or open-licensed tracks (AU-030 manifest).
- **Workspace & shortcuts (UI, workspace-and-shortcuts.md):**
  - **UI owns** the inspector layout (the Audio section, and where the Keep pitch checkbox sits in the Speed tab), the dockable mixer panel, the mvp track header (UI-035) and all keyboard bindings.
  - **Bindings for UI-031:** UI-031 should add AU-036's volume-step chords (`Alt+=` / `Alt+-`, with `Shift` for 0.1 dB steps) and AU-025's tap key (`Alt+B`, which also adds a beat at the playhead when stopped). Its table wins on conflicts.
  - **Commands for the registry (UI-030):** Mute video sound, Project mute (AU-039), Compare (AU-041), Clearer speech, Even loudness and Remove auto-fix (AU-042). None has a default key.
  - **Disabled reasons and toasts:** AU-040's reason strings use UI-030's disabled-reason rule and the string catalog (UI-044). Batch and mute toasts use UI-040.
  - **Preferences:** waveform height (AU-009) sits next to TL-042's "Show waveforms". Output-device errors (AU-010) open UI-029's device picker.

## Out of scope (and why)
- **Online music, sound-effect or "trending sounds" libraries, and the reference editor's licensed music catalogue:** FreeCut has no cloud, no accounts and no server. Users import their own files, plus an optional bundled open-licensed pack (AU-030, AU-031). This covers the observed server-side pieces: download on first use, trending and popularity sorting, server "similar sounds" suggestions, brand-kit music, favourites that need sign-in or come from a linked social account, songs hidden for copyright reasons, partner-platform music, and batch fetches of songs, effects or lyrics. Local substitutes: Recent, Favourites and the licence filter (AU-030) work offline, and a local "More like this" is a later option.
- **Copyright-check, content-ID scanning and the commercial-use scan:** these need a remote fingerprint database or a vendor catalogue. Local substitute: every library item carries its licence in the pack manifest, AU-030 has a licence filter and a Credits view, and user files are labelled "Your file".
- **Usage quotas, free-use counters, paywalls and credits on audio tools:** the observed reference app gates noise reduction, voice isolation, stem separation, voice enhancement, model-based voice changing and, by one reading, audio extraction behind counters or a subscription, and can block export when they run out. FreeCut is free and local, so every tool is always available, with no counters or caps (AU-040).
- **Server-side processing queues with wait estimates:** FreeCut runs its jobs locally on AI-001's scheduler. They use the same progress, cancel and retry pattern (AU-040), with no network.
- **Synthetic character voices, speech-to-speech conversion, voice cloning, voice swap, overdub and speech-to-singing conversion:** these are cloud model features in the reference editor, and they raise consent concerns. AU-022 offers only DSP voice effects, with user presets instead of persona voices. Text-to-speech, if any, belongs to ai-tools.md (AI-025). Translated dubbing and lip sync are excluded there too.
- **Generated music, lyric re-singing and generated sound effects for a clip:** cloud generative features. Users bring their own audio or use the SFX library (AU-031).
- **Importing audio from a pasted link, or from a social account's sounds:** needs network access to third-party services. Users download a file themselves (where its licence allows) and import it locally.
- **Direct publishing to social platforms with platform music:** no accounts or network.
- **Template soundtrack locks** (template authors allowing or blocking replacement of the music) **and publishing rules for templates with copyrighted sound:** template publishing is a marketplace feature. Local user templates are OV's, and their music must follow AU-030's licence rules.
- **Refusals tied to the reference app's own clip types** (generated avatar clips, cloud templates, partner music): FreeCut has none of these types. AU-040 lists the refusals FreeCut does make.
- **Changing the system volume:** an OS setting, not the editor's job. AU-010 only points to it in its no-sound help text.
- **Object-based spatial audio and binaural rendering** (per-object position and size, distance modes, a binaural monitoring mode): the observed reference build only has empty placeholders for it, and FreeCut mixes to stereo. A local binaural renderer (HRTF convolution with open SOFA sets) could be a later spec of its own.
- **Surround (5.1/7.1) mixing and export:** FreeCut mixes to stereo. Surround sources are downmixed (AU-010). This may be revisited later.
- **Third-party audio plugin hosting (VST3, AU, CLAP, LV2) and MIDI:** large scope that is unusual for this class of editor.
- **Multitrack destructive audio editing (sample-level pencil, spectral repair):** belongs in a DAW, not here.

## Clean-room notes
- **Research basis:** this spec relies on public help pages, the vendor's public marketing and tutorial pages, third-party tutorials and reviews, and public documentation of open-source editors and libraries. It also uses a paraphrased inventory of the reference app's English UI labels, taken from the bundled English localization file of a local install (CapCut desktop 9.5.0; see Provenance in the [README](README.md)). Nothing was decompiled, disassembled or traced, and no assets, strings, project files or network traffic were copied. Everything taken from the labels is restated in FreeCut's own words, as behavior, ranges and option lists.
- **Names:** every feature, UI label, preset and category name here is FreeCut's own and generic (for example "Pitch up", "Phone line", "Voice polish", "whooshes and transitions"). Do not reuse the reference editor's preset, filter, effect or UI label names in code, UI strings or assets.
- **Music:** no music from the reference editor or its partners. Optionally ship a small starter pack of **CC0** tracks, or **CC BY 4.0** tracks with automatic attribution (AU-030). Each file carries its licence in the pack manifest. Licences with non-commercial or no-derivatives terms (CC BY-NC, BY-ND) are excluded because they conflict with free redistribution in a GPL app.
- **Sound effects:** recorded or synthesised by FreeCut contributors and released as CC0, or curated from CC0 sources (for example CC0 game-audio packs, or CC0-tagged items from community sound archives), with each file's source recorded in the manifest. Do not bundle royalty-free packs whose terms forbid redistribution.
- **Voice-effect preview sounds:** recorded by contributors and released as CC0.
- **Voice-effect grouping and parameters:** the tabs (Pitch & character, Rooms & echoes, Devices & radio, Modulation) and the effect names are FreeCut's own. The reference app's voice names, persona categories and tab labels are not reused.
- **Models and DSP:**
  - Noise reduction (AU-018), echo reduction (AU-037), voice polish (AU-019, including its plosive and click stages), voice effects (AU-022) and filters (AU-034): FreeCut's own Rust implementations of published signal-processing methods. No model weights.
  - Audio auto-fix (AU-042) and channel auto-fill (AU-014): classic measurements (speech/music discrimination features, noise floor, decay time, channel RMS) written by FreeCut. No model weights.
  - Voice clarity (AU-020) and voice/background separation (AU-021): this spec picks no models. AI-024's engine uses DeepFilterNet (MIT/Apache-2.0 code), and AI-023's engine uses a Demucs-family or Open-Unmix model. Code and weight licences are checked there, and models load through AI-002's model manager. Noise reduction (AU-018) uses no model at all: no RNNoise or other weights.
  - Loudness: `ebur128`, MIT.
  - Time-stretch: our own WSOLA or phase vocoder, signalsmith-stretch (MIT), or Rubber Band (GPL-2.0-or-later; compatible with GPL-3.0-or-later, but it forbids Apple App Store distribution without a commercial licence).
  - Beat tracking: AU-025, FreeCut's own Rust implementation of published methods (spectral-flux onset strength, autocorrelation tempo estimate, dynamic-programming beat tracking). No model weights.
  - Voice activity: our own energy detector, with Silero VAD (MIT) as a later option.
  - Recording: FreeCut's own WAV/RF64 writer (architecture C53); `hound` (Apache-2.0) only for test fixtures.
  - Encoders: FFmpeg with libmp3lame (LGPL) and the native AAC and FLAC encoders.
- **UI artwork:** speaker, solo and record icons, meters and waveform styling are drawn by FreeCut, or taken from an open icon set with a compatible licence. Nothing is copied from the reference editor.
- **Keyboard bindings:** the volume-step chords (AU-036) and the tap key (AU-025) are FreeCut's own choices. They are not copied from the reference editor's keymap, which third-party lists report uses different keys.

## Sources
- Reference app UI observation — CapCut desktop 9.5.0 English UI labels (local install), paraphrased; no code, assets or strings copied.
- How to edit audio / video sound editor overview (vendor) — https://www.capcut.com/resource/video-sound-editor
- How to change your voice on web, app and PC (vendor) — https://www.capcut.com/resource/how-to-change-your-voice
- Audio ducking for clear dialogue (vendor guide) — https://www.capcut.com/create/audio-ducking-for-clear-dialogue-in-video
- Audio ducking for commentary (vendor guide) — https://www.capcut.com/create/audio-ducking-in-capcut-for-clear-commentary
- Loudness normalization tool page (vendor) — https://www.capcut.com/tools/loudness-normalization
- MP3 normalizer guide (vendor) — https://www.capcut.com/resource/mp3-normalizer
- Audacity normalize guide mentioning the desktop normalization target (vendor) — https://www.capcut.com/resource/audacity-normalize
- Noise reduction guidance (vendor) — https://www.capcut.com/resource/noise-reduction-guidance
- Music speed changer (vendor) — https://www.capcut.com/tools/music-speed-changer
- Vocal remover tool page (vendor) — https://www.capcut.com/tools/vocal-remover
- Extract audio from video guide (vendor) — https://www.capcut.com/resource/extract-audio-from-video
- Why is the audio not playing (vendor help center) — https://www.capcut.com/help/audio-not-playing
- How to use Auto Cut (vendor help center) — https://www.capcut.com/help/how-to-use-auto-cut
- CapCut for PC & Mac beginner tutorial (Primal Video) — https://primalvideo.com/guides/capcut-for-pc-mac-tutorial/
- How to fade audio in/out (MacMyths) — https://macmyths.com/how-to-fade-audio-in-out-in-capcut/
- Add music and sync it with cuts on desktop (NotJustOk) — https://notjustok.com/article/how-to-add-music-and-sync-it-with-cuts-in-capcut-desktop/
- How to add music and beat-match (TourBox) — https://www.tourboxtech.com/en/news/how-to-add-music-to-capcut.html
- How to export only audio (Miracamp) — https://www.miracamp.com/learn/capcut/how-to-only-export-audio
- Set a clip to mute on mobile and desktop (Hollyland) — https://store.hollyland.com/blogs/creator-hub/set-a-clip-to-mute-in-capcut
- Noise reduction troubleshooting (SimpleClean) — https://simpleclean.app/blog/capcut-noise-reduction-not-working
- How to speed up a video, including the keep-pitch option (Movavi) — https://www.movavi.com/learning-portal/how-to-speed-up-a-video-on-capcut
- Top audio ducking tools roundup (NokiaMob) — https://nokiamob.net/2026/05/15/top-5-audio-ducking-tools-for-perfect-voice-music-balance/
- Shotcut Gain/Volume audio filter (Shotcut forum) — https://forum.shotcut.org/t/gain-volume-audio-filter/12906
- Shotcut linear audio fade discussion (Shotcut forum) — https://forum.shotcut.org/t/linear-audio-fade-way-to-quickly-set-up/26960
- Kdenlive Audio Tools manual (KDE) — https://docs.kdenlive.org/en/effects_and_filters/audio.html
- OpenShot user guide, clip properties (OpenShot) — https://forum.openshot.org/static/files/user-guide/clips.html
- DaVinci Resolve normalize audio levels guide (Beginners Approach) — https://beginnersapproach.com/davinci-resolve-normalize-balance-audio-levels/
- Resolve Normalize Audio Levels discussion (Blackmagic forum) — https://forum.blackmagicdesign.com/viewtopic.php?p=421692
- ebur128 crate documentation (docs.rs) — https://docs.rs/ebur128
- nnnoiseless crate (docs.rs) — https://docs.rs/crate/nnnoiseless
- Rubber Band Library licensing (Breakfast Quay) — https://breakfastquay.com/rubberband/license.html
- DeepFilterNet LADSPA package with licence metadata (Arch Linux) — https://archlinux.org/packages/extra/x86_64/deepfilternet-ladspa/
- CapCut keyboard shortcut cheat sheet, including clip volume nudges (FastShortcuts) — https://fastshortcuts.com/shortcuts/capcut/
- 50 CapCut keyboard shortcuts, including 0.1 dB volume steps (TutorialTactic) — https://tutorialtactic.com/blog/capcut-shortcuts/
- CapCut keyboard shortcuts list (Skillademia) — https://www.skillademia.com/shortcuts/capcut-shortcuts/
- Final Cut Pro keyboard shortcuts, relative and absolute volume adjustment (Apple Support) — https://support.apple.com/guide/final-cut-pro/keyboard-shortcuts-ver90ba5929/mac
