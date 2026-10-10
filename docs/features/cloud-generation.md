# Cloud generation (Higgsfield)

> Status: draft · 2026-10-13 · Owns: the connection to a user-owned generation service account (sign-in, pasted token, keychain storage, disconnect); the generation model catalogue and its filters; the run confirmation sheet (cost preflight, what is sent, explicit spend); the **Generate** tab of the asset shelf; cloud job states, the unknown-outcome rule and the cloud job ledger; the source-range and upload rules for AI edit actions on clips; placement of generated results; spending limits; the assistant's generation tools (CG-024). Not owned here: media items and the generated-media store (PM-041), the task list (PM-073 / UI-040), undo semantics (TL-043), clip replacement (TL-035), the assistant's proposal flow (AS-008, AS-013), local smart tools (AI) · Crates: freecut-cloud (proposed, see [cloud-generation.md](../architecture/cloud-generation.md)), freecut-timeline, freecut-media, freecut-project, freecut-assistant (CG-024 only), freecut-app

## Summary

A user who has their own Higgsfield account can connect it to FreeCut and generate images and video from a prompt, or send a clip section or a frame to be edited, extended, upscaled, reframed, dubbed and so on. Results come back as ordinary generated media items with a full record of how they were made, and can be placed on the timeline as one undoable edit. The feature is **off by default** and FreeCut runs no service of its own. The user connects their own account, every paid run shows its cost and exactly what will leave the computer, and nothing is spent without an explicit click. Higgsfield is a third-party service and FreeCut is not affiliated with it. Local tools (ai-tools.md) stay the default and the free choice wherever one exists.

**How Higgsfield can be reached (checked 2026-10-13).** Higgsfield has two separate products:

- **The MCP server** at `https://mcp.higgsfield.ai/mcp`. It uses the user's Higgsfield plan credits and needs a paid plan. Its documented sign-in is OAuth in a browser window; Higgsfield's help pages say no API key is needed.
- **The REST API** at `https://api.higgsfield.ai`. It uses a separate key pair and a prepaid dollar balance. Its docs tell developers not to call it from client applications.

The user asked to "bring their own token". As published, the MCP server has no token to paste: the user signs in, and FreeCut stores the tokens the sign-in returns. So CG-001 offers **Sign in with Higgsfield** as the main path and **Paste an access token** as an advanced path for users whose service gives them one. The REST key pair is not offered until Higgsfield confirms that a desktop app may store a user's own key (see Out of scope and the README's Open questions).

## Features

### CG-001 Connect a Higgsfield account — v1
**What the user sees:** Preferences ▸ **Cloud generation** (a new UI-018 section):
- **Cloud generation:** on / off. Default **off**. While off, no Generate tab, no AI edit menu entries and no network requests exist.
- **Service:** *Higgsfield (third-party service)*. It is the only entry in v1, and the list is there so other services can be added later (architecture §6).
- A status line: *Not connected*, *Connected* (with the account e-mail when the sign-in returns one), *Sign-in expired* or *Can't reach the service*.
- **Connect your Higgsfield account** opens a short FreeCut sheet:
  - **Sign in with Higgsfield (recommended):** FreeCut opens the system browser at Higgsfield's own sign-in page. FreeCut never shows a sign-in form, never sees the password and uses no embedded web view. The sheet names the address the browser should show (the host from the service's sign-in metadata, for example `mcp.higgsfield.ai`), so the user can check they are on Higgsfield's own page. After the user approves, the browser shows a plain FreeCut page that says the tab can be closed, and the sheet reports *Connected*.
  - **Use a code instead:** shown only when the service offers device sign-in (architecture §5.4). FreeCut shows a short code and the service's address; the user enters the code in any browser.
  - **Advanced ▸ Paste an access token:** a password field. A pasted token cannot be renewed by FreeCut, so when it expires the user must paste a new one or sign in.
  - Text under the buttons, in FreeCut's words: Higgsfield is a separate company and FreeCut is not affiliated with or endorsed by it. Generations use the credits of the user's Higgsfield plan, and Higgsfield needs a paid plan for this kind of connection. The user is responsible for what they generate under Higgsfield's terms. Links: *Higgsfield terms*, *Higgsfield privacy policy* (shown as text with a copy button and an *Open in browser* button; never loaded inside FreeCut).
- **Test connection** connects, lists the service's tools and loads the model catalogue (CG-003). It reports success, the number of image, video and audio models, and the round-trip time. It spends nothing.
- **Show balance** (optional, on click only) asks the service for the account's remaining credits and shows them in the section. FreeCut never reads the balance in the background.
- **Disconnect** (CG-002).
**Inputs / controls:** The on/off switch; the three connection methods; Test connection; Show balance; Disconnect.
**Behavior / output:**
- Sign-in uses OAuth 2.1 with PKCE in the system browser and a one-time loopback address on `127.0.0.1` (architecture §5). FreeCut registers itself with the service the first time.
- Connecting runs the MCP handshake and checks that every tool FreeCut uses is offered (architecture §4.6). An action whose tool is missing is shown disabled with "Not offered by the service right now".
- Turning the switch off keeps the account but stops all cloud activity, including polling (CG-009). Turning it on again resumes polling.
**Edge cases:**
- The browser never returns (the user closes it): after 5 minutes (FreeCut default) the sheet says sign-in timed out, with **Try again**. The loopback address is closed.
- The service refuses FreeCut's loopback sign-in or its registration: the sheet says so, offers the code method if available and the pasted token otherwise, and shows the service's message as quoted text.
- No paid plan: the service's refusal is shown as "This connection needs a paid Higgsfield plan" when the service says so, otherwise its own message.
- Pasted text that is not a plausible token (empty, contains spaces or line breaks): refused before anything is sent.
- Offline: *Can't reach the service*, with **Try again**. Nothing else in FreeCut is affected.
- Several FreeCut windows (UI-057): one account and one connection for the whole app.
- A second FreeCut process on the same computer: only the process that holds the cloud ledger's lock (architecture §7.2) connects, polls and submits; the other shows "Cloud generation is running in another FreeCut window" and its Run buttons are disabled.
**Undo:** n/a (preferences).
**Model notes:** `CloudPrefs { enabled, provider, client_registration }` (no secrets) in the preferences (UI-018). OAuth, the MCP client and the catalogue live in `freecut-cloud`; the browser launch, the loopback listener and the section UI live in `freecut-app` (architecture §5).

### CG-002 Credentials in the OS keychain & disconnect — v1
**What the user sees:** Under the status line: "Sign-in stored in <macOS Keychain / Windows Credential Manager / the Secret Service>". **Disconnect** asks "Disconnect your Higgsfield account from FreeCut?" with the note that runs in progress will finish on the service but FreeCut can only download their results after reconnecting.
**Inputs / controls:** Disconnect; **Keep sign-in for this session only** (offered always, and the only choice when no credential store exists).
**Behavior / output:**
- Access token, refresh token, expiry, the FreeCut client id and the account e-mail shown in the status line are stored in the OS credential store under the service name "FreeCut cloud generation" and the provider id (split over several entries where the store limits entry size, architecture §5.6). They are never written to `settings.json`, project files, the cloud ledger (CG-009), logs, crash reports (UI-023), diagnostic bundles (UI-063) or settings exports (UI-050).
- Tokens are read on the cloud worker thread only, right before a request. FreeCut renews an access token with the refresh token when less than 60 s remain (5 minutes before a paid run) or when the service answers *unauthorized*. A free request is then sent once more; a paid one is not (CG-010).
- Disconnect asks the service to revoke the tokens when it offers revocation, then deletes them from the credential store. It also tells the user that the service may list FreeCut among connected apps, where it can be removed too, and that a sign-in they think has leaked should be disconnected here and reported to Higgsfield (its terms ask account holders to report suspected leaks).
**Edge cases:**
- The credential store is locked: the OS prompt appears; if dismissed, the request fails with "Sign-in not available" and **Retry**.
- Renewal fails (refresh token expired or revoked): status *Sign-in expired*; running jobs pause polling (CG-009) until the user signs in again. Nothing is deleted automatically.
- Signing in with a different account than before: jobs recorded under the old account (CG-009) are listed as "From another account" and are not polled with the new one.
- Settings import never imports tokens; a settings export says only whether an account was connected.
**Undo:** n/a.
**Model notes:** A `SecretStore` trait in `freecut-cloud`, implemented in `freecut-app` by the same keychain type that implements the assistant's `SecretStore` (AS-003); tests use an in-memory store.

### CG-003 Model catalogue & model picker — v1
**What the user sees:** In the Generate tab (CG-006, CG-007) and in each AI edit action (CG-015 to CG-023), a **Model** picker. Each entry shows the model's display name, the company behind it as the service reports it (for example "by ByteDance"), what it takes (text, start frame, end frame, reference images, reference video, reference audio), what it makes (image, video, audio), its durations and largest resolution, and an *Audio* badge when it can make sound. A **Refresh models** command sits in the picker's menu.
**Inputs / controls:** The picker, with a search field. A *Recommended* group at the top: FreeCut's default model per task (as of 2026-10-13: `gpt_image_2_5` for images, `seedance_2_5` for video and for extension), shown only while the service still lists it.
**Behavior / output:**
- The catalogue is read from the service's free model listing (architecture §6.2) when the account connects, on Refresh, and when the cached copy is older than 24 h (FreeCut default) and the Generate tab opens. It is cached in app data and works offline for browsing.
- The picker is filtered by **output type** (the tab or action) and by **inputs**: once the user adds a start frame or a reference video, models that cannot take that input are hidden, and a line says how many were hidden and why.
- Every option control (durations, aspect ratios, resolutions, quality, audio, model-specific settings) is built from the catalogue entry, so new models need no FreeCut update.
- **Hidden models:** models the service marks for its own pipelines only (its game-pipeline music, sound-effect and speech models), models that need setup FreeCut doesn't offer (identity training, marketing styles chosen in the service's own UI, legacy preset recipes), and models that take web addresses instead of media. The list is in architecture §6.2.
**Edge cases:**
- A model in a saved run (CG-009) or a recipe (CG-011) is no longer listed: *Run again* says the model is gone and offers the picker.
- The catalogue fails to load and there is no cache: the tab shows the error and **Retry**; actions are disabled.
- Two entries with the same display name: the model id is shown in brackets.
**Undo:** n/a.
**Model notes:** `Catalogue`, `ModelInfo`, `ParamSpec`, `DurationSpec` and the filters are pure code in `freecut-cloud` (architecture §6.2), tested with a recorded listing.

### CG-004 What leaves the machine: disclosure and data notice — v1
**What the user sees:**
- **Once, when the account is connected**, a FreeCut notice in plain words:
  - What FreeCut sends: only what each run's sheet lists (below), and only when the user presses Run.
  - What Higgsfield says about data in its terms and privacy policy, as read on 2026-10-13: it may use uploaded inputs and generated outputs to train models; generated outputs are kept for at least 7 days; removed content may stay up to 30 days plus backups; data is shared with the AI model companies that run each model and may be processed in the US and elsewhere. The notice links to the current policy and says the policy may have changed.
- **On every run**, the run sheet (CG-005) has a **Sends** list, for example:
  - "Prompt text (212 characters) and the settings above"
  - "A 12.4 s section of *Interview.mp4* (0:03.20–0:15.60), re-encoded as 1080p H.264 MP4 **without sound**, about 18 MB"
  - "One still frame at 0:42 of the timeline, as a 1920×1080 PNG"
  - "To Higgsfield, and through it to <model company> (the company that runs this model)"
  - It also lists what is never sent: the project file, other clips, file names and paths, location and camera metadata, anything else on the computer.
- A per-project switch in Project settings, **Never use cloud generation in this project**, hides the Generate tab's Run and the AI edit actions for that project.
**Inputs / controls:** The notice's **I understand**; the per-project switch; **Show last request** in the Runs list menu (CG-009), which shows the exact JSON arguments of the last tool call (no tokens) with **Copy**.
**Behavior / output:**
- Uploads carry generic file names (`freecut-input-1.mp4`), so the user's file names never reach the service. Transcoded sections and frames have all container metadata removed (architecture §8.1).
- FreeCut sends the service's optional analytics field empty and adds no identifiers of its own. No telemetry.
- The Sends list is built from the same `UploadPlan` that drives the upload, so the sheet and the upload cannot differ (architecture §8.1).
**Edge cases:**
- A run that needs sound (dub, lip sync, voice change) says "with sound" in bold.
- "As seen in the timeline" mode (CG-014, later) says that effects, titles and stickers on that clip are part of the upload.
- The per-project switch is on and the user opens a recipe from that project in another project: the switch of the current project applies.
**Undo:** n/a.
**Model notes:** `UploadPlan` and `Disclosure` are pure types in `freecut-cloud`; the consent record (the account notice) is kept in the preferences.

### CG-005 Cost preflight and run confirmation — v1
**What the user sees:** Pressing **Generate…** (CG-006, CG-007) or confirming an AI edit (CG-015 onward) opens the **run sheet**, a FreeCut dialog with:
- A title written by FreeCut from the run ("Generate 2 images with GPT Image 2.5", "Extend *Beach.mp4* forward by 5 s with Seedance 2.5").
- **Cost:** the service's estimate for the whole run (all variants), fetched before the sheet is shown, for example "About 24 credits from your Higgsfield plan". Every v1 action has an estimate. An action whose service tool gives no estimate (architecture §6.3) is routed through a catalogue model that does, or is **not offered**; the only exception would be a later action the owners explicitly accept with a "cost shown only afterwards" warning (README Open questions), and then the sheet says "The service does not show the cost before running" and needs a tick on **I understand the cost is shown only afterwards** before Run is enabled.
- **Sends** (CG-004).
- The prompt and settings as they will be sent, read-only (go back to change them).
- For voice and likeness actions (dub, lip sync, voice change, and edits of footage with people), a required tick box: **I have the rights and consent needed for the people, voices and material in this run.**
- Buttons: **Run** (labelled with the estimate, "Run · about 24 credits") and **Cancel**. There is no "don't ask again".
**Inputs / controls:** Run, Cancel, the tick boxes.
**Behavior / output:**
- The estimate comes from the service's own cost preflight for the exact arguments, which submits nothing. Changing any setting invalidates it, and the sheet fetches a new one.
- The preflight uses the same service tool as the paid run, with one flag that asks for the price only. FreeCut guards that flag in code (architecture §4.7): it refuses to send a preflight without it, sends a preflight only when the tool's live schema still offers it, never retries a preflight that may have reached the service, and treats a preflight answer that contains job ids as a run that started: the run is recorded in the ledger and polled, the user is told "Higgsfield started a run while FreeCut only asked for the price", and cost checks for that tool stop until FreeCut restarts.
- When the estimate cannot be shown to cover every variant (the answer gives one figure and the service does not say what it covers), Variants is limited to 1 for that model.
- An estimate older than 5 minutes (FreeCut default) is fetched again when Run is pressed; if it went up, the sheet shows the new figure and asks again.
- Run starts the job (CG-009). Uploads come first, then the submission. The sheet closes at once; progress is in the task list.
- Credits are the service's unit; FreeCut never converts them to money.
**Edge cases:**
- The preflight fails (offline, sign-in expired): the sheet shows the reason and Run is disabled.
- The preflight answers with a service question (CG-026) instead of a price: the question is shown first.
- The user presses Run twice quickly: one run (the button is disabled after the first click).
- A run whose estimate is over the user's warning threshold (CG-025): the sheet shows the warning and needs a second click.
**Undo:** n/a (spending credits cannot be undone; the media it produces can be removed, CG-011).
**Model notes:** `Quote` and `RunSheet` (pure state) in `freecut-cloud`; the dialog in `freecut-app`. Run sheets opened for the assistant (CG-024) are labelled "Requested by the assistant".

### CG-006 Generate tab: images — v1
**What the user sees:** A new asset shelf tab **Generate** (UI-004), shown only while cloud generation is on and connected. Its drawer list has **Image**, **Video** and **Runs**. The Image page has:
- **Prompt** (multi-line, up to 4,000 characters, FreeCut default, or the model's own limit when lower).
- **Model** (CG-003).
- **Aspect ratio:** defaults to the project canvas (PM-003), snapped to the nearest ratio the model offers, with a note when it had to snap ("Project 4:5 → 3:4"). Other ratios from the model are listed.
- **Resolution** and **Quality** when the model has them; defaults are the model's defaults.
- **References** (CG-008): image slots for the roles the model accepts. With a reference image the page edits that image by prompt; with a mask slot (models that offer one) only the masked area changes.
- **Variants:** 1–4 (default 1). Each variant is a separate result of the same request.
- **Advanced:** other model settings from the catalogue (seed, transparent background and so on).
- **Check cost** (shows the estimate inline without opening the sheet) and **Generate…** (CG-005).
**Inputs / controls:** As above.
**Behavior / output:** A finished run adds its images to the library (CG-011) and to the Runs list. Place them with CG-012.
**Edge cases:**
- Empty prompt with no reference: Generate is disabled.
- The model drops the requested ratio or count: the service reports an adjustment (CG-026), shown on the run's card and recorded.
- The project is a 9:16 story and the model has no 9:16: the nearest ratio is chosen and the note says so.
**Undo:** n/a here (results: CG-011, CG-012).
**Model notes:** `Operation::Image` in `freecut-cloud`; the page in `freecut-app`.

### CG-007 Generate tab: video from a prompt or frames — v1
**What the user sees:** The Video page has the same Prompt, Model, Aspect ratio and Variants controls as CG-006, plus:
- **Duration:** a slider or list from the model's allowed durations, in seconds. Default 5 s (FreeCut default), snapped to the model.
- **Resolution:** the model's list; default 720p (FreeCut default, to keep costs down) when offered.
- **Sound:** on / off for models that can make sound; default is the model's default.
- **Start frame** and **End frame** slots (CG-008) for models that take them. With only a start frame the clip begins on it; with both the clip moves from one to the other.
- **Fill a gap** (shortcut): with a gap or a cut selected on the timeline, the start frame is set to the last frame before it and the end frame to the first frame after it, and the duration to the gap's length (snapped).
**Inputs / controls:** As above.
**Behavior / output:** Like CG-006. A video run's result is a video file, with sound when the model made sound.
**Edge cases:**
- Duration not allowed by the model: the slider cannot reach it; the service may still adjust it (CG-026).
- Fill a gap where the gap is shorter than the model's minimum: the duration is the minimum and the note says the result will be trimmed when placed.
- A start frame from a 4:3 clip in a 16:9 project: the aspect ratio follows the project; the model decides how to fill.
**Undo:** n/a here.
**Model notes:** `Operation::Video { mode: TextToVideo | FramesToVideo, .. }`.

### CG-008 References from the library, timeline and current frame — v1
**What the user sees:** Each reference slot accepts:
- **A library item:** drag from the Media tab or click the slot's **Choose…** (library picker). An image item is used as is; for a video item the slot asks for a frame (default: its poster frame, PM-057) or, for video slots, a range.
- **A timeline clip:** drag a clip onto the slot. Image slots take the frame of that clip under the playhead (or its first frame when the playhead is outside it); video slots take its used range (CG-014's rules).
- **Current frame:** a button that takes the frame the viewer shows at the playhead, rendered by the same compositor as export (FX-001), titles and effects included.
- Each filled slot shows a thumbnail, its source ("Frame 0:42 of the timeline"), and ✕.
**Inputs / controls:** Drag and drop, Choose…, Current frame, ✕.
**Behavior / output:** References are prepared and uploaded only when Run is pressed (CG-014's upload rules). A reference that already went up in this session for the same account is reused (architecture §8.2), which saves time but not credits.
**Edge cases:**
- Offline media in a slot: the slot shows *Offline* and Generate is disabled.
- A reference that is a generated result of an earlier run (CG-011): the service's own result is reused without uploading.
- More references than the model takes: the extra slot is disabled with the model's limit.
- A sticker, text or adjustment clip dropped on an image slot: only *Current frame* can capture it; the drop shows a hint.
**Undo:** n/a.
**Model notes:** `InputRef { Library(MediaId, FrameOrRange), TimelineFrame(Tick), Clip(ClipId), PriorResult(LocalJobId, usize) }` in `freecut-cloud`; frame rendering and transcoding are app jobs using `freecut-render` and `freecut-media`.

### CG-009 Cloud jobs in the task list and the Runs list — v1
**What the user sees:**
- Each run is a row in the app's single task list (PM-073, UI-040), owner *Cloud*: "Uploading 18 MB" with a progress bar, then "Waiting for Higgsfield" with elapsed time (the service reports no percentage), then "Downloading", then done. Row action **Stop waiting**.
- The Generate tab's **Runs** page lists every run of this project, newest first: thumbnail, prompt excerpt, model, state, cost, time. States: *Queued here* (waiting for a free slot, CG-025), *Uploading*, *Submitting*, *Running*, *Downloading*, *Ready*, *Added*, *Failed* (with the reason; *Failed: not sent* when nothing left the computer), *Not accepted* (CG-010), *Outcome unknown* (CG-010), *Stopped*, *Expired*.
- Run actions: **Place** (CG-012), **Show in library**, **Copy prompt**, **Use as reference** and, for results, **Upscale…** and other follow-up actions that can take a prior result, **Run again…** (always opens a new run sheet), **Remove from list** (the media stays).
**Inputs / controls:** The row and card actions.
**Behavior / output:**
- Every run is written to the cloud ledger in app data **before** it is submitted and again as soon as the service answers with job ids (architecture §7). The ledger, not the project, keeps runs alive: closing the project or quitting FreeCut does not lose a run (CG-013).
- FreeCut polls the service for running jobs while the app is open and cloud generation is on, whether or not their project is open, at the interval the service asks for (at least 2 s, at most 60 s).
- **Stop waiting** stops polling and downloading for that run. The service offers no way to cancel a submitted generation, so the confirmation says: "Higgsfield will still finish this run and may still charge for it. You can check it again later." The run becomes *Stopped*, with **Check again**.
- Uploads that have not finished when the user stops are cancelled for real; nothing is submitted.
**Edge cases:**
- A run fails on the service: *Failed* with the service's reason (architecture §10). Whether credits were charged is reported only as the service reports it.
- Results not downloaded within 7 days of finishing: *Expired*, because the service keeps outputs for at least 7 days and promises nothing after. **Check again** still tries.
- Sign-in expired while jobs run: polling pauses and the rows say "Paused: sign in again" (CG-002).
- Offline: polling backs off and resumes when the network returns; rows say "Waiting for network".
**Undo:** n/a (jobs are not document edits; importing and placing are, CG-011, CG-012).
**Model notes:** `CloudJob` state machine and `LedgerEntry` DTOs in `freecut-cloud`; the ledger file is written by `freecut-app` with the same atomic replace as project saves; rows feed `TaskRow { owner: TaskOwner::Cloud, .. }` (PM-073).

### CG-010 Unknown outcomes are never resubmitted — v1
**What the user sees:** When FreeCut sent a paid request but got no answer (the connection dropped or timed out after the request left), the run shows **Outcome unknown**: "FreeCut sent this run but didn't hear back. It may be running on Higgsfield and may have used credits." Actions: **Check again** (asks the service for the run's status by any job id FreeCut has, or looks for it among the account's recent results when the service offers that) and **Run again…**, which opens a new run sheet with a warning that this may pay twice.
**Inputs / controls:** Check again; Run again….
**Behavior / output:**
- FreeCut **never** resubmits a paid request on its own, after any error, in any code path. Automatic retries exist only for free, read-only requests (the catalogue, cost preflight, status polling) and for uploads to an address the service handed out (architecture §4.5).
- A request that provably never left the computer (the connection could not be opened) is not an unknown outcome: the run is *Failed: not sent* and **Run again…** carries no double-payment warning.
- A request the service **refused before running it** (sign-in expired, MCP session expired, too many runs at once) is not resent either, even after FreeCut renews the sign-in or the session. The run becomes *Not accepted* with the reason, and **Run again…** opens the run sheet with a fresh estimate and no double-payment warning. To make this rare, FreeCut renews a sign-in that will expire within 5 minutes and checks the session with the fresh estimate just before it submits.
- If the cloud ledger cannot be written before submitting (disk full, permissions), nothing is submitted: *Failed: not sent*, with the reason.
- An answer FreeCut cannot read after a paid request was sent is always *Outcome unknown*, never *Failed*.
- A run whose service request returns job ids after the user pressed Stop waiting keeps those ids in the ledger.
**Edge cases:**
- FreeCut crashes between sending a run and recording its job ids: on the next start the ledger entry is still *Submitting*, which becomes *Outcome unknown*.
- Several variants: when some job ids came back and others did not, the known ones continue and the run says how many are unknown.
**Undo:** n/a.
**Model notes:** The rule is enforced in one place, `freecut-cloud`'s submit path, which has no retry wrapper; a test asserts exactly one submission for every scripted failure (architecture §13).

### CG-011 Results as generated media with provenance — v1
**What the user sees:** Each result is a media item in the library's **Generated** bin (PM-041) with an *AI-generated* badge and a cloud mark. **Media info** (PM-059) has a **How it was made** section: service and model, prompt, settings as the service used them (including adjustments), inputs (with links to the source items and the source range), the run date, the cost estimate, and the file's checksum. **Copy recipe** copies it as JSON.
**Inputs / controls:** Media info; Copy recipe; **Generate again…** (opens a run sheet with the same recipe; results will differ).
**Behavior / output:**
- Downloads go into the project's generated-media store, `<Name> Media/Generated/Cloud/`, named "<date> <model> <short run id>.<ext>". The checksum is computed while downloading; a file is used only after it is complete and FreeCut can open it (PM-035).
- The item records `MediaOrigin::Generated { kind: GeneratedMediaKind::Cloud, produced_by: "higgsfield:<model id>", recipe, parent, source_range }` (PM-041; in code PM-041's kind enum is `GeneratedMediaKind`, since `GeneratedKind` names generated clip content). The recipe is kept as JSON text, so `freecut-timeline` needs no JSON library (architecture §9). `parent` is the source media item for clip actions; `source_range` is the source range that was uploaded.
- When the run was started from this project and the project is open, results are added to the library as soon as they are downloaded, in one undo entry per run, "Add generated media" (a background change, architecture README C42).
**Edge cases:**
- The file is missing later: the relink dialog (PM-091) offers **Generate again…**, which goes through a run sheet and costs credits.
- The result cannot be opened (unexpected format): the run is *Failed: unsupported result*; the downloaded file is kept in the inbox (CG-013) and *Show file* reveals it.
- The project folder is read-only: PM-041's app-data fallback applies.
- Several variants: one media item per variant, named with its number.
**Undo:** Undoing "Add generated media" removes the items but never deletes their files (PM-041). PM-074's **Clean up project…** offers unused generated files for the Trash.
**Model notes:** `GeneratedMediaKind::Cloud` (a new variant, PM-041) plus the provenance fields PM-041 describes, which today's `MediaOrigin::Generated` does not have yet (it carries only `kind`); adding them is a timeline model and format change that the PM and TL owners sign off (architecture §9). The recipe builder is pure (`freecut-cloud`, architecture §9).

### CG-012 Placing results on the timeline — v1
**What the user sees:** A result can be placed by dragging it from the library like any media, with **Place** on its run card, or automatically when the run was started with **When ready** set to a placement. The choices:
- **Library only** (default for the Generate tab).
- **Insert at playhead** (main track, TL-012's rules) or **Add above at playhead** (overlay track).
- For AI edit actions on a clip (CG-015 onward): **Place above the original** (default: a new clip on the track above, covering the same timeline range, so the original stays underneath and can be hidden or compared), **Replace the original** (the clip's media is swapped by TL-035's rules, keeping its position, length and every edit on it), and for extensions **Insert after** / **Insert before** (CG-016).
**Inputs / controls:** The *When ready* dropdown in the action's sheet; Place on the run card.
**Behavior / output:**
- Automatic placement waits until no drag or slider gesture is in progress and is planned against the document as it is when the result arrives. Import and placement are **one** undo entry, "Add & place generated media".
- If the original clip has been deleted, moved to another place or changed so the placement no longer fits, the result is only added to the library and a toast says why, with **Place…**.
**Edge cases:**
- A result longer or shorter than the original range: *Place above* uses the result's own length from the range start; *Replace* follows TL-035 (shorter media shortens the clip and ripples on the magnetic main track; longer media uses its first part).
- The track above is occupied in that range: a new track is created (TL-006).
- The original is on a locked track (TL-009): automatic placement falls back to the library with a toast.
**Undo:** One entry; undo removes the placed clip and the added items, never the files.
**Model notes:** Placement reuses existing `EditRequest`s (add media, insert, place on track, replace media) combined into one compound plan; no new timeline op, only the `GeneratedMediaKind` variant and the provenance fields (CG-011).

### CG-013 Results that finish while the project is closed — v1
**What the user sees:** When the user reopens a project whose runs finished while it was closed, a bar above the timeline says "3 cloud results are ready" with **Add to project**, **Review…** and **Later**. FreeCut's Home screen shows a small badge on such projects.
**Inputs / controls:** The bar's buttons; Review lists the runs with per-run Add and Discard.
**Behavior / output:**
- While a project is closed, FreeCut still polls and downloads its runs (CG-009). Files go straight into that project's generated-media store when its folder is reachable and writable, otherwise into the cloud inbox in app data, `<app data>/FreeCut/cloud/inbox/<run id>/`.
- **Add to project** imports them as one undo entry. Automatic placement is not applied after reopening; the bar offers **Place** per result instead, because the timeline may have changed.
- **Discard** removes the run from the list. Files already in the project's store stay and are offered by **Clean up project…** (PM-074); inbox files are moved to the Trash.
**Edge cases:**
- The project was moved or renamed: the ledger matches it by project id (PM-021); inbox files are moved into its store on Add.
- The project was deleted: its runs stay in the ledger under *Projects not found* in Preferences ▸ Cloud generation ▸ **Unclaimed results**, where they can be revealed or discarded. FreeCut deletes nothing on its own.
**Undo:** Add to project is one entry (as CG-011).
**Model notes:** Inbox paths are computed by `freecut-project`; the matching is by `ProjectId` in the ledger.

### CG-014 Source ranges and uploads for clip actions — v1
**What the user sees:** Right-click a clip ▸ **AI edit (cloud)** ▸ the actions of CG-015 to CG-023, also in the inspector's **AI** section. Every action's sheet shows the section that will be sent on a small strip of the clip, with handles to change it within the action's limits.
**Inputs / controls:** The section handles; **Send** *Source* (default) or *As seen in the timeline* (later); **With sound** where the action allows a choice.
**Behavior / output — the rules every clip action follows:**
- **Clip kinds:** video clips and image clips whose media is a file. Text, sticker, shape, color, adjustment and caption clips have no AI edit menu. Compound clips (TL-040) need *As seen in the timeline*.
- **Section:** the clip's used source range, from its in-point to its out-point in **source media time**: what the clip shows, at the media's own speed, before speed changes, reverse, transforms, color and effects. In/out marks (TL-038) inside the clip narrow it.
- **As seen in the timeline** (later): the clip's timeline range is rendered alone through FX-001 (its own effects, transform, color and speed; no other tracks) and sent instead. The result then replaces the rendered look, so the sheet says that the clip's own effects are baked in and switched off when *Replace the original* is used.
- **Limits:** each action has a shortest and longest section from the model (for example at most 15 s, 30 s or 60 s). A longer clip is **never cut silently**: the sheet starts with a section at the playhead (or at the clip start) of the longest allowed length and the user can move it. A clip shorter than the minimum has the action disabled with the limit in the tooltip.
- **Frame accuracy:** the section is cut on exact frames from the source (TL-001's rounding), and its length in seconds is computed from ticks; only the request's duration field is a number of seconds.
- **Upload:** the section is re-encoded as an H.264 MP4 of at most 1920 px on the long side (FreeCut default; smaller when the source is smaller), keeping the source frame rate, with AAC sound only when the action needs it or *With sound* is on; frames as PNG; audio-only inputs as 48 kHz WAV. Metadata is removed and files have generic names (CG-004). Prepared files live in the media cache (PM-074), so retries reuse them.
- **One clip at a time** in v1 (FreeCut default). With several clips selected, the actions are disabled with "Select one clip".
**Edge cases:**
- Offline media, or proxy-only editing with the original missing (PM-081): disabled, "Needs the original file".
- Variable-frame-rate phone video: the section is cut by timestamps and re-encoded at the stream's nominal rate; the sheet shows the rounded length.
- HDR source (CL-041): sent as SDR after FreeCut's tone mapping, and the sheet says so (HDR upload is out of scope while HDR stays deferred).
- The clip's source is itself a cloud result: the service's own result is used instead of uploading (CG-008).
**Undo:** n/a here (placement: CG-012).
**Model notes:** `source_section(clip, limits, playhead) -> SourceSection` is a pure function in `freecut-timeline` (it needs only the clip and ticks); `UploadPlan` in `freecut-cloud`; transcoding in `freecut-media`.

### CG-015 Edit a clip with a prompt — v1
**What the user sees:** **Edit with a prompt…**: a prompt ("make it night", "replace the red car with a bicycle"), optional reference images, the model (video-editing models of CG-003; default `seedance_2_5` in its edit mode as of 2026-10-13), the section (CG-014) and *When ready* (CG-012, default *Place above the original*). For image clips the same action uses an image model with the image as reference.
**Inputs / controls:** As above, plus the model's own settings.
**Behavior / output:** Upload, run sheet, job, result as CG-011 and CG-012. The result covers the sent section.
**Edge cases:**
- The model uses only the first part of what it gets (one model edits at most the first 15 s): the section is limited to that length up front, never cut by the service.
- The model returns a different length than sent: placed by CG-012's rules, and the card says by how much.
**Undo:** CG-012.
**Model notes:** `Operation::Video { mode: EditByPrompt, .. }` or `Operation::Image` with a reference.

### CG-016 Extend a clip forward or backward — v1
**What the user sees:** **Extend forward…** / **Extend backward…**: how many seconds to add (from the model's durations; default 5 s), an optional prompt for what happens next, the model (default `seedance_2_5` in its extension mode), and *When ready*: **Insert after** (forward) or **Insert before** (backward), default; or *Library only*.
**Inputs / controls:** As above. **Context sent:** the last (forward) or first (backward) 5 s of the clip's used range (FreeCut default; adjustable within the model's limits).
**Behavior / output:**
- The result is placed directly after (or before) the clip on the same track. On the magnetic main track this ripples later clips (TL-005); on an overlay track it needs free room, otherwise it goes on a new track above at that position.
- The result follows the source's aspect ratio (the model decides; the project's ratio is not sent).
- If the service returns the context section together with the new part, FreeCut trims the context off when placing, so the joined clip continues without repeating frames. Which of the two the service does is confirmed with a recorded run before this ships (architecture §15).
**Edge cases:**
- The clip ends at its media's end and the user extends forward: allowed (that is the point); the new clip is separate media, not a longer source.
- Extending a clip whose out-point is before the media's end: the sheet points out that the source already has more footage and offers **Trim out instead** (TL-020), which is free.
**Undo:** CG-012.
**Model notes:** `Operation::Video { mode: Extend { direction }, .. }`.

### CG-017 Remove background (cloud) — later
**What the user sees:** **Remove background (cloud)…** for a video or image clip. The sheet first offers the local tool: "FreeCut can also do this on your computer, free (AI-014)". *When ready*: Place above (default) or Replace.
**Behavior / output:** The whole used range (CG-014) is sent without sound. A result with transparency is used as a video with alpha (OV-014); a result delivered as a separate matte is applied as the clip's matte. Which one the service returns is confirmed with a recorded run first; until then the action is not shown.
**Edge cases:** Long clips: limited by the model. The service's dedicated background tool lists no cost estimate, so FreeCut runs this through the catalogue's background-removal model (`video_background_remover` for video, with a still-image model for images) with the normal estimate; if a free estimate for that model cannot be confirmed in H3b, the action is not offered (CG-005).
**Undo:** CG-012.
**Model notes:** `Operation::RemoveBackground`, mapped to `generate_video` / `generate_image` with the catalogue model (architecture §6.3).

### CG-018 Upscale (cloud) — later
**What the user sees:** **Upscale (cloud)…** with the target (for video: 1080p, 2K or 4K and frame rate where the model offers it; for images: 2K or 4K), and a pointer to the local tool (AI-019). Results that are already in the service (prior results) are upscaled without uploading.
**Behavior / output:** The used range is sent (CG-014); FreeCut supplies the source's pixel size where the service requires it. The dedicated video upscale tool gives no cost estimate, so video upscaling runs through the catalogue's upscale models (`bytedance_video_upscale`, `topaz_video`) with the normal estimate, and is not offered if that cannot be confirmed in H3b (CG-005). Higher frame rates may cost more; the estimate shows it.
**Edge cases:** The target is not larger than the source: disabled, "Already at this size".
**Undo:** CG-012 (default *Replace the original*, since the picture is the same).
**Model notes:** `Operation::Upscale`.

### CG-019 Reframe to the project aspect (generative) — later
**What the user sees:** **Reframe with new picture area (cloud)…** when the clip's aspect differs from the canvas: the service generates the missing picture area to fill the project's ratio instead of cropping. The sheet first offers FreeCut's own crop-based reframe (AI-016), which is free.
**Behavior / output:** Up to 60 s per run (sections over 15 s need a resolution choice, which the sheet asks for). Target ratio: the project canvas, snapped to the service's list.
**Edge cases:** The project ratio is not offered: nearest ratio, with a note.
**Undo:** CG-012 (default *Replace the original*).
**Model notes:** `Operation::Reframe`.

### CG-020 SDR to HDR — later
**What the user sees:** **Convert to HDR (cloud)…**, producing an HDR version of a clip.
**Behavior / output:** The service keeps size and frame rate and returns a ProRes MOV (or H.265). **Blocked:** FreeCut has no HDR working space and no HDR export (deferred by decision; CL-041 tone-maps HDR sources to SDR), so an HDR result would look the same as its SDR source in the project. The action is not shown until CL, EX and PM reopen HDR.
**Undo:** CG-012.
**Model notes:** `Operation::SdrToHdr`.

### CG-021 Dub to another language — later
**What the user sees:** **Dub (cloud)…** with a target language (the service's list: 18 languages as of 2026-10-13) and the consent tick box (CG-005). *When ready*: Replace the sound (default: the clip's own audio is muted, AU's clip mute, and the dubbed result is placed above with its sound) or Library only.
**Behavior / output:** The used range is sent **with sound**. The result is a new video with translated speech.
**Edge cases:** No speech: the service's error is shown. Music under the speech may be changed by the service; the sheet says so.
**Undo:** CG-012; muting the original audio is part of the same entry.
**Model notes:** `Operation::Dub`. Needs the owners' decision on ai-tools.md's *Out of scope* (README Open questions). The service's dubbing tool lists no cost estimate, so this action also needs the owners to accept CG-005's "cost shown only afterwards" exception, or a cost estimate from the service; without either it is not built.

### CG-022 Lip sync to an audio clip — later
**What the user sees:** **Lip sync to audio (cloud)…**: the video clip and an audio source, chosen from the audio clip overlapping it on the timeline (default), another timeline clip or a library item; what to do when lengths differ (the model's options, for example loop, cut off or fill with silence); the consent tick box.
**Behavior / output:** The video section goes without sound, the audio section as WAV. The result is placed above with the chosen audio as its sound.
**Edge cases:** Audio longer than the model allows: the section is limited up front (CG-014).
**Undo:** CG-012.
**Model notes:** `Operation::LipSync`. Same owner decision as CG-021.

### CG-023 Voice change — later
**What the user sees:** **Change voice (cloud)…** with a voice picker. The picker lists the service's **preset voices** only, each with a preview the user can play on click. Custom or cloned voices on the user's account are not listed (voice cloning stays out of scope). The consent tick box is required.
**Behavior / output:** The used range is sent with sound; the result replaces the clip's sound (*Replace the original*, default) or goes to the library.
**Edge cases:** The voice list is unavailable: the action is disabled with the reason.
**Undo:** CG-012.
**Model notes:** `Operation::VoiceChange`. Same owner decisions as CG-021, including the cost-estimate one. Voice previews are downloaded only when the user presses play. The service's voice list also returns the account's own custom voices; FreeCut drops them on receipt and keeps nothing of them.

### CG-024 Generation tools for the assistant — later
**What the user sees:** With the assistant (AS) set up and cloud generation connected, the user can ask "make a 5-second establishing shot of a beach at sunset and put it before the first clip" or "extend this clip by 3 seconds". The assistant prepares the run, and FreeCut opens the **same run sheet** (CG-005) with the assistant's prompt in an editable field, the cost and the Sends list. Nothing is spent unless the user presses Run. When the result is ready, its placement appears as an ordinary assistant proposal (AS-007, AS-008).
**Inputs / controls:** The run sheet; the proposal card.
**Behavior / output:**
- The assistant sees three tools: a read-only list of models and their options, *generate media*, and *AI edit a clip*. The two run tools are **actions** (AS-013): they only ask FreeCut to open a run sheet.
- The assistant never sends media to its own provider because of these tools: it sees model names, settings, run states and the resulting media ids only.
- The assistant cannot answer service questions (CG-026), tick consent boxes, change limits or lift the per-project switch.
- At most 4 run sheets per assistant turn (FreeCut default, an AS-018 limit).
**Edge cases:** The user cancels the sheet: the assistant is told "cancelled by the user". Outcome unknown: the assistant is told and may not ask for a new run in the same turn.
**Undo:** Placement follows AS-008 (one entry when accepted).
**Model notes:** Tool specs in `freecut-assistant` (action tools, llm-editor.md §3.5); the app routes them to CG-005; the assistant crate does not depend on `freecut-cloud`.

### CG-025 Spending limits — v1
**What the user sees:** Preferences ▸ Cloud generation ▸ **Limits**:
- **Warn when one run is estimated above** N credits: off by default; when on, the run sheet shows a warning and needs a second click.
- **Runs at the same time:** 1–4, default 2 (FreeCut default). Further runs wait as *Queued here* before anything is submitted, and their estimate is fetched again when their turn comes; if it went up, they ask again.
- A **This session** line in the Runs page: number of runs and the sum of their estimates (counted on this computer only).
**Inputs / controls:** As above.
**Behavior / output:** Every paid run needs its own confirmation; no setting turns that off. Higgsfield itself has no spending cap for this connection, so FreeCut says so next to the limits.
**Edge cases:** A limit lowered while runs are queued: applies to the queue at once; running jobs continue.
**Undo:** n/a.
**Model notes:** `Limits` in `freecut-cloud`, enforced by the job scheduler.

### CG-026 Service questions, adjustments and follow-up steps — v1
**What the user sees:**
- **Free-trial generations:** when the service answers a run with a choice between the account's free-trial or unlimited allowance and paid credits, FreeCut shows a dialog with both choices and **no default**: **Use a free-trial generation** / **Pay with credits** / Cancel. FreeCut never picks one by itself.
- **Adjustments:** when the service changes a setting (for example 7 s rounded to 8 s, or variants capped at 1), the run card lists each change, and the recipe records what was actually used.
- **Follow-up steps:** when the service asks for another step before it can run, FreeCut does it only if it is a free, read-only step on its allow list; a paid step opens a new run sheet; anything else fails the run with the service's message.
**Inputs / controls:** The dialog buttons.
**Behavior / output:**
- When the question comes back from the cost check, the dialog appears before the run sheet, and the sheet then shows the estimate for the chosen way of paying; Run stays the only button that submits.
- When it comes back from the submission itself (the service submitted nothing), the dialog shows the run's estimate on **Pay with credits** ("Pay about 24 credits"). Either answer is the user's explicit click for that run, and FreeCut sends the same settings once more with the answer. The answer applies to that run only.
**Edge cases:**
- Free-trial use limits variants to 1 (as the service states); the dialog says so.
- Higgsfield's help pages say free generations apply on its own website only, while the tool descriptions describe this question; the dialog is built against a recorded answer and may never appear (architecture §15).
**Undo:** n/a.
**Model notes:** `ServiceQuestion` and `Adjustment` types in `freecut-cloud`; the provider mapping turns the service's fields into them (architecture §6.4).

## Interactions with other areas

- **Projects & media (PM):** results are PM-041 items with the new `GeneratedMediaKind::Cloud`; files go to the generated-media store; prepared uploads are PM-074 cache entries; cloud runs are PM-073 task rows (`TaskOwner::Cloud`); Media info (PM-059) shows the recipe; relink (PM-091) offers Generate again.
- **Timeline (TL):** placement uses existing requests: insert (TL-012), add on a track (TL-006), replace media (TL-035), with ripple rules (TL-005) and locked tracks (TL-009); import and placement are one entry (TL-043). The pure `source_section` function is added to `freecut-timeline`.
- **Smart tools (AI):** local tools stay first and free; the sheets of CG-017, CG-018 and CG-019 point to AI-014, AI-019 and AI-016. AI-026's local generator extension point is unchanged: it still refuses remote hosts; cloud generation is this separate, opt-in area.
- **Assistant (AS):** CG-024 adds action tools; confirmations stay FreeCut UI (AS-013, llm-editor.md §6.2). The assistant's own provider and the generation service are different services with separate consent.
- **Workspace (UI):** the Generate tab joins UI-004's shelf; Preferences gains the Cloud generation section (UI-018); task rows and toasts are UI-040's; strings go through UI-044; the AI edit submenu joins the clip context menu (UI-035).
- **Color (CL) and Export (EX):** HDR results wait for the HDR decision (CG-020). Export is unchanged: generated media exports like any media, with no watermark (EX-040).

## Out of scope (and why)

- **A FreeCut-run service, proxy or account.** FreeCut connects the user's own account directly to the service; there is nothing in between to run, pay for or trust.
- **The REST API key pair (pay-as-you-go balance).** It matches "paste a key" most literally, but Higgsfield's docs tell developers not to call the API from client apps and its terms forbid embedding keys in client-side code. Whether a desktop app that stores the user's own key in the OS keychain is acceptable needs Higgsfield's confirmation. The provider abstraction (architecture §6) lets it be added later as a second provider.
- **Music and sound effects.** The service restricts its music, sound-effect and game speech models to its own game pipeline. FreeCut offers none of them; AU-030 and AU-031's own libraries cover music and effects.
- **The service's presets, workflows and studios** (viral presets, marketing and ad studios, AI influencers, websites, 3D scenes, social posting, video analysis). They are separate products inside the service, often multi-step and paid, and FreeCut does its own editing.
- **Batch submission tools.** They cannot preflight cost, so every variant goes through the single-run tools.
- **Service tools that give no cost estimate**, unless the owners accept CG-005's "cost shown only afterwards" exception for a named action.
- **Creating or cloning voices, and training identities** on the service. Voice cloning is deferred by decision (README); identity training needs face data FreeCut does not handle.
- **Using free-trial generations without asking**, and any setting that skips the run sheet.
- **Webhooks.** A desktop app cannot receive them; FreeCut polls.
- **Higgsfield's logo, colors or marks**, and any wording that suggests endorsement. FreeCut names the service in plain text only ("Connect your Higgsfield account").
- **Telemetry.** None.

## Clean-room notes

- Higgsfield is a third-party service, not the reference editor. The integration follows its public help pages, terms, API docs and the public MCP and OAuth discovery documents, plus the tool schemas and the free model listing of the user's own connected MCP server, read on 2026-10-13 without generating anything. No Higgsfield SDK, CLI or skill code is copied; FreeCut talks to the documented protocols directly.
- The reference editor has its own cloud generation features; they were not studied for this spec, and none of its services, models, prompts, presets or assets are used.
- All UI wording, defaults, run states, the run sheet and the recipe format are FreeCut's own. Model names and company names are shown as the service reports them, nominatively.

## Sources

- What is Higgsfield MCP (Higgsfield help center) — https://higgsfield.ai/creator-hub/help-center/integrations/what-is-higgsfield-mcp
- How do I connect Higgsfield to an AI agent (Higgsfield help center) — https://higgsfield.ai/creator-hub/help-center/integrations/how-do-i-connect-higgsfield-to-ai-agent
- Official Higgsfield platforms (Higgsfield help center) — https://higgsfield.ai/creator-hub/help-center/getting-started/official-higgsfield-platforms
- What is the Higgsfield API (Higgsfield help center) — https://higgsfield.ai/creator-hub/help-center/integrations/what-is-the-higgsfield-api
- Higgsfield terms of use — https://higgsfield.ai/terms-of-use-agreement
- Higgsfield privacy policy — https://higgsfield.ai/privacy-policy
- Higgsfield API authentication — https://docs.higgsfield.ai/docs/authentication.md
- Higgsfield API requests and statuses — https://docs.higgsfield.ai/docs/concepts/requests.md
- Higgsfield API billing and retention — https://docs.higgsfield.ai/docs/concepts/billing-and-retention.md
- Higgsfield API file uploads — https://docs.higgsfield.ai/docs/concepts/file-uploads.md
- Higgsfield MCP protected-resource metadata — https://mcp.higgsfield.ai/.well-known/oauth-protected-resource
- Higgsfield MCP authorization-server metadata — https://mcp.higgsfield.ai/.well-known/oauth-authorization-server
- Model Context Protocol specification (transports, lifecycle, tools, authorization) — https://modelcontextprotocol.io/specification
