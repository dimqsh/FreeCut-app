# LLM assistant

> Status: draft · Owns: the assistant panel and its conversation; assistant provider settings and API-key storage; the tool surface a language model sees (query, edit, action and macro tools) and its argument conventions (ids, time strings, value units); proposals (step list, change summary, ghost preview, accept / adjust / discard, re-checking after document changes); the disclosure of what leaves the machine; assistant limits; the assistant's evaluation scenarios (edit semantics stay with each owning spec — the assistant only issues their requests; undo semantics: TL-043; batch skip reporting: TL-053; analysis jobs, transcripts and their staleness: AI-001 / AI-009; panel docking: UI-056; the export dialog: EX-021) · Crates: freecut-assistant (proposed, see [llm-editor.md](../architecture/llm-editor.md)), freecut-timeline, freecut-project, freecut-app

## Summary

FreeCut's long-term goal is to be an open-source, LLM-based video editor: the user describes an edit in plain language and a language model turns it into ordinary FreeCut edits. The assistant is **off by default** and **never edits on its own**. A model only *proposes*: every change it wants is planned through the same typed edit requests the timeline, inspector and menus use (`freecut_timeline::edit::EditRequest`), shown as a ghost on the timeline with a FreeCut-written change list, and applied only when the user accepts it — as **one undo entry**. The user chooses the model: a local model behind an OpenAI-compatible server on their own machine (for example Ollama or llama.cpp), or a hosted API such as Anthropic's Messages API with their own key. FreeCut runs no service of its own, has no account and sends no telemetry; AS-004 shows exactly what a hosted model receives, and media files are never sent.

## Features

### AS-001 Assistant panel and conversation — v1
**What the user sees:**
- View ▸ *Panels* ▸ **Assistant** opens an auxiliary panel (UI-056). FreeCut default: the second face of the inspector column, switched at the column top ([ui-concept §11](../design/ui-concept.md#11-assistant-its-place-in-the-ui) owns the presentation); it can be moved to the asset slot. The panel is built around the timeline rather than the chat: proposals appear *on the timeline* (AS-007), and the panel holds the conversation, the active proposal card and the input.
- Top to bottom: a header with a **provider badge** (*Local · model name*, *Hosted · model name* or *Off*) and a privacy dot (green: nothing leaves this computer; amber: a hosted provider is in use); the conversation; the proposal card (AS-007), pinned above the input while a proposal is open; the input box with its context chips (AS-014) and **Send** / **Stop**.
- While the assistant is off (the default), the panel shows two sentences about what it does and a **Set up…** button that opens Preferences ▸ Assistant (AS-002). Nothing is contacted until setup is finished.
- The command palette (UI-034) gains an "ask" row: text that matches no command offers *Ask the assistant: "…"*, which opens the panel with that text sent.
- Replies stream in as plain text with light formatting (paragraphs, lists, bold, inline code). Clip, track and time references in a reply become chips (AS-005). Tool activity shows as one compact, collapsible line per call, written by FreeCut from the call itself (for example "Read 12 clips between 0:30 and 1:10").
**Inputs / controls:** Message box, up to 4,000 characters (FreeCut default); Enter sends, Shift+Enter adds a line break. **Stop** (AS-010). **New conversation** clears the context (AS-020). The panel's own key bindings are proposed to UI-031 (see Open questions in the README).
**Behavior / output:**
- One turn runs at a time per project; Send is disabled while a turn runs or a proposal is waiting for re-checking (AS-009).
- In a turn the model may answer a question (AS-005), propose an edit (AS-006, AS-007), or ask the user a clarifying question (AS-011).
- All panel strings go through the string catalog (UI-044). The model is told the UI language and answers in it; FreeCut's own instructions to the model stay in English.
**Edge cases:**
- No provider configured: the input is disabled and shows **Set up…**.
- Provider unreachable or failing: an error line names the cause (not running, key rejected, rate limited, timed out) with **Retry** and **Settings**; the conversation is kept.
- Message over the limit: Send is disabled and a counter shows the excess.
- The project closes or the app quits during a turn: the turn is cancelled (AS-010) without a prompt; no document change can result.
- Several projects open in separate windows (UI-057, later): each window has its own panel, conversation and proposal.
- Project opened read-only because another FreeCut process holds the lock (PM-013): questions work; **Accept** is disabled with the lock reason.
**Undo:** n/a (the conversation is view state; edits are AS-008's).
**Model notes:** `Conversation`, `Turn`, `Message` and the provider-neutral `Part` types, and the sans-I/O `Session` state machine, live in `freecut-assistant`. The panel lives in `freecut-app` and only draws state received from the assistant worker over a crossbeam channel; nothing in the panel blocks on the network (llm-editor.md §2).

### AS-002 Providers and model choice — v1
**What the user sees:** Preferences ▸ **Assistant** (UI-018):
- **Assistant:** on / off. Default **off**.
- **Provider:**
  - *None* (default).
  - *Local server (OpenAI-compatible)*: base URL, default `http://127.0.0.1:11434/v1` (Ollama's default port), and a model dropdown filled from the server's model list, with free entry.
  - *Anthropic API*: key (AS-003), and a model dropdown with `claude-opus-5-5` (default), `claude-sonnet-5-5` (faster, lower cost) and `claude-haiku-5-5` (fastest), plus free entry for newer ids; **Effort** Low / Medium / High, default Medium.
  - *Other OpenAI-compatible server*: base URL and optional key, for self-hosted or third-party endpoints.
- **Test connection** sends one minimal request and reports success, the model's tool and image support, and the round-trip time.
- **Tool profile:** Full / Compact (AS-006); default Full for hosted providers, Compact for local ones.
**Inputs / controls:** URLs must be `https://`, except loopback addresses (`127.0.0.1`, `::1`, `localhost`), which may use `http://`. Model ids are free text, 1–200 characters.
**Behavior / output:**
- A provider is **local** when its base URL resolves to a loopback address; everything else, including a LAN server, is **remote** and goes through AS-004's disclosure.
- FreeCut probes the provider's capabilities when it is chosen and on **Test connection**: tool calling (required for everything beyond chat), parallel tool calls, image input (AS-016), streaming, and the context size when the server reports it.
- Switching provider or model takes effect at the next turn. The conversation gets a divider line naming the new model; the earlier conversation is re-sent to the new provider only as a FreeCut-written recap (AS-020), under AS-004's rules.
**Edge cases:**
- The local server is not running: Test connection says so and suggests starting it; FreeCut never starts or installs a server itself.
- The chosen model cannot call tools: the panel says the model can chat but cannot read or edit the project, and suggests another model.
- `http://` to a non-loopback host: refused with the reason.
- The server's model list is unavailable: free entry still works.
- A hosted model id is retired by its vendor: the provider's error is shown with a link to the setting.
**Undo:** n/a (preferences).
**Model notes:** `ProviderConfig` (no secrets) is stored with the preferences (UI-018); keys are AS-003's. The `Provider` trait, the capability probe and both wire mappings (Anthropic Messages, OpenAI-compatible chat completions) live in `freecut-assistant` behind its `http` cargo feature (llm-editor.md §5).

### AS-003 API keys in the OS keychain — v1
**What the user sees:** In the provider settings, a key field that shows only the last four characters once saved, with **Replace** and **Remove**. Under it: "Stored in <macOS Keychain / Windows Credential Manager / the Secret Service>".
**Inputs / controls:** Paste a key; **Keep for this session only** (always offered, and the only choice when no credential store is available); **Read from the `ANTHROPIC_API_KEY` environment variable** (off by default).
**Behavior / output:**
- Keys are written to the operating system's credential store under the service name "FreeCut assistant" and the provider id. They are never written to `settings.json`, project files, the conversation sidecar (AS-020), logs, crash reports (UI-023), diagnostic bundles (UI-063) or settings exports (UI-050).
- The key is read from the store on the assistant worker thread only when a request is about to be sent.
**Edge cases:**
- The credential store is locked: the OS prompt appears; if it is dismissed the request fails with "Key not available" and **Retry**.
- No credential store (for example a Linux session without a Secret Service): the session-only choice is the only one, and the field says why.
- The provider rejects the key (HTTP 401/403): the panel error offers **Check key**; the key is not deleted automatically.
- Settings import (UI-050) never imports keys, and a settings export lists which providers have a key, without the key.
**Undo:** n/a.
**Model notes:** Access goes through a small `SecretStore` trait in `freecut-assistant`, implemented in `freecut-app` with the `keyring` crate; tests use an in-memory store.

### AS-004 What leaves the machine: disclosure and consent — v1
**What the user sees:**
- The first time a remote provider is used for a project, and again whenever the provider's host changes, a FreeCut dialog lists what will be sent:
  - **Always:** the user's messages; the project brief (canvas, frame rate, duration, tracks; clip names, positions and lengths; text-clip content and style; audio and transform values; media file *names*, kinds, durations and formats; selection, playhead and range); FreeCut's instructions and tool descriptions.
  - **Only when a turn needs it, shown as a chip:** transcript and caption excerpts (AS-015), audio level and beat numbers (AS-017), small still frames if allowed (AS-016).
  - **Never:** media files or their audio, full file paths (only file names), keys, other projects, anything from the operating system.
  The choices are **Allow for this project**, **Allow for all projects** and **Cancel**.
- Each turn's context chips (AS-014) show what that turn includes; removing a chip before sending removes that data.
- **Show last request** in the panel menu opens a read-only view of the exact request body sent (the key redacted), with **Copy**.
- A per-project switch, **Never use a remote assistant for this project**, disables remote providers for it; local ones still work.
**Inputs / controls:** The dialog choices; the per-project switch; **Reset assistant permissions** in Preferences.
**Behavior / output:**
- FreeCut sends nothing to anyone except the configured endpoint, and only on a user action: Send, Retry, Test connection, or refreshing the model list. There are no background requests, no usage pings and no telemetry from this feature.
- Local (loopback) providers need no dialog: nothing leaves the computer. The badge still says *Local*.
- Images are sent only as FreeCut-rendered JPEG stills (AS-016), never as media files.
**Edge cases:**
- Cancel in the dialog: the message stays in the input, unsent.
- The per-project switch is on and only a remote provider is configured: the input is disabled with the reason.
- File names that are themselves sensitive: Preferences ▸ Assistant ▸ **Send file names** can be turned off, and media is then labelled `media:<id>` with its kind and length only.
**Undo:** n/a.
**Model notes:** The disclosure list is generated from the same `ContextPolicy` that builds requests (llm-editor.md §5.6), so the dialog cannot drift from what is sent. The consent record is kept in the preferences, keyed by project id and provider host; where the per-project switch should live (preferences or the project file) is an open question.

### AS-005 Questions about the project (read-only) — v1
**What the user sees:** The user asks things like "how long is the edit?", "which clips are muted?", "where does the title say 'Summer'?" or "what's at 1:20?". The model answers using FreeCut's read-only query tools. Clips, tracks and times in the answer are chips: clicking a clip chip selects it and scrolls the timeline to it (TL-015); clicking a time chip moves the playhead (TL-013).
**Inputs / controls:** The message. Query tools available from M-AI1 (llm-editor.md §3.4): project summary, clips in a time range or on a track, one clip's full properties, the media library, the recent undo history labels. Later milestones add transcripts and captions (AS-015), frames (AS-016) and audio levels and beats (AS-017).
**Behavior / output:**
- Queries read the current document, or the proposal under review when the model asks for it (AS-011).
- Large projects are paged: the project brief is capped (24 KB by default, llm-editor.md §4.2) and lists per-track counts when the cap is hit; the model then pages with the range tools.
- Every query result is marked as data (AS-019).
**Edge cases:**
- A chip refers to a clip that has since been deleted: it is disabled and its tooltip says so.
- Empty project: the brief says so, and the model is told it may suggest importing media.
- A query for something FreeCut does not model yet (for example effects before FX lands): the tool list does not offer it, and the model is told which areas are not available in this build.
**Undo:** n/a (clicking a chip changes only the selection or playhead, which are view state).
**Model notes:** Query tools are pure functions over a `Document` snapshot in `freecut-assistant`, plus host queries the app answers on worker threads (frames, levels, transcripts).

### AS-006 Edit tools — v1
**What the user sees:** The model can propose edits. While it plans, the activity lines show each planned step ("Planned: split 2 clips at 0:12.40"). Nothing on the timeline changes until AS-008.
**Inputs / controls (the tool surface the model sees):**
- One edit tool per `EditRequest` kind, grouped as tracks (add, remove, rename/mute/hide/lock, magnetic main track), placement (insert media, still images and text clips), arrangement (move, trim, split, delete with or without ripple), text (content, style, layout), transform (position, scale, rotation, flip, opacity, reset), audio (clip volume, mute, fades, gain keyframes, track volume and pan, master volume, detach/restore/extract, stream choice, resync) and project settings. The full mapping is llm-editor.md §3.2. As other specs add requests (effects, transitions, keyframes, captions, markers…), each gets a tool or a documented exclusion; a test enforces this (llm-editor.md §3.3).
- **Ids:** `clip:41`, `track:3`, `media:7`, as the query tools report them.
- **Times:** a string in seconds (`"12.48s"`), frames (`"300f"`), or timecode (`"00:00:12.480"` or `"00:00:12:12"` with frames). Each is converted exactly (no floating point) and rounded to the sequence's frame grid by TL-001's rule. Results report times in seconds and frames.
- **Values** use the inspector's units: percent, degrees, dB, canvas pixels, `#RRGGBB[AA]` colors; decimals are strings (`"-6.5"`), whole numbers may be JSON integers.
- **Tool profile:** *Full* offers every tool; *Compact* offers the queries plus insert, move, trim, split, delete, text, transform and clip audio, for small local models.
**Behavior / output:**
- Each edit call is planned at once by the same planner the UI uses, with the same rules (magnetic main track, locked tracks, track limits, skip reporting per TL-053). The model receives the planner's result: success with the ids it created and any notices, or the planner's refusal reason, and may try again.
- In M-AI2 a proposal holds **one** edit request (it may still touch many clips, for example "mute all clips on track 2"); a second edit call in the same turn is refused with a hint to combine. AS-011 (M-AI3) lifts this.
**Edge cases:**
- Unknown or stale id: refused with "no such clip" and the model is asked to query first.
- Value out of range: refused with the allowed range; where the planner itself clamps (for example gain above +12 dB, AU-010), the result says what was used.
- An edit that some targets don't support (for example a text style on a video clip): those targets are skipped and listed (TL-053); if none is left, the planner's refusal is returned.
- Malformed arguments (common with small local models): the call fails with the JSON path and the expected shape; after two failed repairs of the same call the turn stops with an error line.
**Undo:** n/a here (planning only; see AS-008).
**Model notes:** Argument DTOs with serde and JSON Schema live in `freecut-assistant` and map by hand to `EditRequest` (the timeline model stays serde-free, README C4). The tool order is fixed so requests stay cacheable (llm-editor.md §3.7).

### AS-007 Proposal preview: ghost timeline and change list — v1
**What the user sees:**
- When a turn ends with planned edits, a **proposal card** appears above the input: the model's one-line title (shown as the model's text), a **change list written by FreeCut** from the planned changes — for example "Split *Interview.mp4* at 0:12.40", "Remove 0:12.40–0:15.00 and close the gap", "Title: size 64 → 96" — totals (clips added, removed and changed; duration before → after), and planner notices (new track created, gaps closed).
- The timeline shows the proposal as a ghost layer. FreeCut default **Overlay**: added clips with a dashed outline in the accent color, removed clips hatched, moved clips with a faint trace of their old place, changed properties marked with a dot. A switch in the timeline toolbar shows *Current*, *Proposed* or *Overlay*.
- **Play proposal** plays the proposed edit from 2 s before its first change, without applying it. Hovering a line in the change list highlights its clips and moves the playhead there.
**Inputs / controls:** The view switch; Play proposal; hover; the AS-008 buttons.
**Behavior / output:**
- The change list is generated from the plan itself (llm-editor.md §4.4), never from the model's description, so the model cannot misdescribe what Accept will do.
- Save, autosave and export always use the current document, never the proposal; while one is open, the export dialog says so.
**Edge cases:**
- The plan changes nothing: the card says no change is needed and has no Accept.
- The user edits the timeline while a proposal is shown: AS-009.
- Playing the proposal needs frames that are not cached: it behaves like normal playback (buffering, offline slates).
- A proposal touching clips that are off screen: the change list lines still scroll the timeline to them.
**Undo:** n/a (preview is view state).
**Model notes:** `ChangeSummary` comes from a pure `freecut_timeline::edit::summarize(before, changes)` that TL-044's history panel can reuse. The ghost draws `Plan::preview()` as gestures already do; Play proposal publishes the preview document as a snapshot flagged as a proposal, which the save and export paths ignore.

### AS-008 Accept, adjust or discard — v1
**What the user sees:** Three buttons on the proposal card:
- **Accept** applies the whole proposal as **one undo entry**, shown in the Edit menu as "Undo Assistant edit". The card turns into an *Applied* line with **Undo**, which is enabled while that entry is the newest one in the history.
- **Adjust**: untick lines in the change list (lines that depend on an unticked line untick too, and the card says which), or keep typing ("make it a bit shorter"); the follow-up continues the same proposal, which stays one entry when accepted.
- **Discard** drops the proposal; nothing changes.
The model is told the outcome (accepted, accepted in part, discarded) at the start of the next turn.
**Inputs / controls:** The buttons; tick boxes per line (from AS-011; a single-request proposal has one line); a key to accept while the panel has focus (proposed to UI-031).
**Behavior / output:**
- Accept commits the proposal through `Editor::commit` on the UI thread. After accepting, the clips the proposal added or changed are selected (at most 100; FreeCut default), unless the plan decides the selection itself (TL-015).
- A partial accept re-plans the remaining lines from the original document before committing (AS-009's checks apply).
**Edge cases:**
- A drag or slider gesture is in progress: Accept waits until it ends.
- The document changed since planning: the commit is refused as stale and AS-009 runs; Accept never applies a plan made against an older document.
- Undoing the applied entry later and redoing it behave like any other entry.
- Undo from the card when newer entries exist: disabled, with a tooltip pointing to Edit ▸ Undo.
**Undo:** One history entry with `OpKind::AssistantEdit` and the number of steps. It holds the concatenated change sets of all steps in order; undo applies their inverse and restores the selection from before Accept; redo reapplies them (the op triple test of README §5.8 applies).
**Model notes:** Multi-step proposals are merged by a proposed `PlanChain` in `freecut-timeline` (llm-editor.md §4.3). The app keeps a map from history entry to conversation turn for the *Applied* line; it is view state.

### AS-009 Proposal out of date — v1
**What the user sees:** If the document changes while a proposal is waiting — the user edits, undoes or redoes, or a background job commits — the card shows "Re-checking after your change…". Then either:
- every line still plans and reads the same: the card is ready again with the note "Re-checked after your edit"; or
- something differs: the changed lines are highlighted and lines that no longer plan show the planner's reason. The user can accept the remaining lines, discard, or press **Ask to update**, which sends the model a note about the change.
**Inputs / controls:** Ask to update; the AS-008 buttons.
**Behavior / output:** FreeCut re-plans the recorded tool calls against the new document on the assistant worker, 300 ms after the last change (FreeCut default; only the latest re-check counts). Clips the proposal itself created are matched by the step and position that created them, not by their old ids.
**Edge cases:**
- A clip the proposal edits was deleted: those lines fail with "no longer in the project".
- The model is still working when the document changes: the turn finishes first, then the re-check runs.
- The user changes the document many times in a row: only the last state is re-checked.
**Undo:** n/a.
**Model notes:** Every `Plan` is stamped with the editor revision it was made against; `Editor::commit` refuses a stale plan (`CommitError::Stale`). Re-planning uses a fresh `Planner` snapshot.

### AS-010 Stop and cancel — v1
**What the user sees:** **Stop** (and Esc while the input has focus and a turn runs) ends the running turn within 1 s. Steps already planned are kept as an *incomplete* proposal, collapsed, which the user may still review and accept or discard (FreeCut default).
**Inputs / controls:** Stop.
**Behavior / output:** The streaming request is aborted, pending query work is cancelled, and local analysis jobs that this turn queued are cancelled unless another request shares them (AI-001's one-job-per-key rule). Cancelling never changes the document.
**Edge cases:**
- Stop while waiting to retry after a rate limit: the retry is dropped.
- A request that stops sending data: after 120 s without data (FreeCut default) the turn fails with a timeout and **Retry**.
- Closing the project or quitting: the turn is cancelled silently and an open proposal is dropped (it was never applied).
**Undo:** n/a.
**Model notes:** One `freecut_media::cancel::CancelToken` per turn (README §2.4); the HTTP reader checks it between stream chunks.

### AS-011 Multi-step proposals — later
**What the user sees:** A request such as "cut the first 10 seconds, add a title 'Day 1' at the start and fade out the music" becomes one proposal with several lines, grouped under the sub-goals the model names ("Trim the start", "Add a title", "Fade the music"). Lines can be unticked one by one (AS-008). The model may also ask a question with choice buttons before it plans ("Fade over 2 s or 5 s?").
**Inputs / controls:** Tick boxes per line and per group; choice buttons on a question card.
**Behavior / output:**
- Each edit call is planned against the document as left by the previous ones, so later steps see the clips earlier steps created. Query tools can read this proposed state.
- At most 200 steps per proposal (AS-018).
- A step the planner refuses is reported to the model, which may fix it; at the end of the turn any step that still fails is left out, and the card says so. Nothing is ever accepted silently.
**Edge cases:**
- Unticking a step that later steps depend on (they edit a clip it created, or a position it shifted): the dependents untick too and are listed.
- After a partial untick, a remaining step no longer plans: it is marked and Accept is disabled until it is unticked or the model updates the proposal.
**Undo:** As AS-008: the whole accepted proposal is one entry.
**Model notes:** `PlanChain` (llm-editor.md §4.3); dependencies come from the ids each step reads and creates.

### AS-012 Macro tools — later
**What the user sees:** Some requests run a ready-made recipe whose result still appears as ordinary proposal lines, for example:
- **Rough cut from the transcript**: keep or drop transcript passages (needs AI-009; reuses AI-011's apply step).
- **Remove pauses** (AI-003's detection and apply rules).
- **Add captions from speech** (AI-010 onto a TX-025 caption track).
- **Cut to the beat** (AI-006 with AU-025's beats).
- **Title card**: a text clip with style and position at a given time (TX-001, TX-004, TR-001; available as soon as AS-011 is).
- **Apply to all matching**: one style, transform or audio change on every clip that matches a filter (TL-053).
**Inputs / controls:** Each macro's parameters, with defaults taken from the owning spec.
**Behavior / output:** A macro is deterministic FreeCut code, not a model: for the same document and parameters it yields the same steps. Macros appear in the tool list only when their prerequisites exist in the build.
**Edge cases:**
- Missing prerequisite data (no transcript yet): the macro returns "needs transcription" and the model can offer to start it; model downloads still need AI-002's consent.
- Stale analysis (AI-001): the macro refuses and says why.
- A macro that finds nothing to do: the card says no change is needed.
**Undo:** One entry per accepted proposal (AS-008).
**Model notes:** Macros are functions `(&Document, analysis data, params) -> Vec<EditRequest>` in `freecut-assistant`, unit-tested with golden step lists; the analysis data comes from the app's AI-001 records.

### AS-013 Actions outside the document — later
**What the user sees:** Some requests are not edits. The model can ask for them, but FreeCut's own UI always takes over:
- **Export**: the export dialog (EX-021) opens pre-filled with the range, preset and file name the model suggested; the user presses Export. Overwriting follows EX-023 and always asks.
- **Import**: only files the user dropped onto the panel or picked in a file dialog that FreeCut opens are imported; the model cannot name other paths.
- **Start a local analysis** (transcription, beats, silence): queued in the task list (PM-073); model downloads ask AI-002's consent.
- **Cloud generation** (later, only when CG is on and connected): CG-024's actions open FreeCut's run sheet (CG-005) with the cost and what is sent; nothing is contacted or spent unless the user presses Run.
**Behavior / output:** The model is told whether each action was confirmed, cancelled or failed. There is no tool for saving, deleting files, opening URLs or contacting any service; saving and autosave stay the user's (UI-021). CG-024's run sheet is the one route by which an assistant request can lead to a service contact, and only through the user's Run click.
**Edge cases:**
- Export requested while a proposal is open: the dialog says it exports the current document.
- The model asks for the same action repeatedly: each one needs its own click.
- A file dropped on the panel that FreeCut cannot import: PM-036's error.
**Undo:** An import is an ordinary undoable edit (PM-030). Export creates no entry.
**Model notes:** Actions are `Output::Action` items of the session state machine; the app shows the confirmation and reports back (llm-editor.md §3.5).

### AS-014 Scope: selection, range and playhead — v1
**What the user sees:** Chips under the input are filled automatically: *Selection: 3 clips*, *Range: 0:10–0:40* (TL-038's in/out marks), *Playhead: 0:42*. Phrases like "this clip" or "here" are resolved against them. A chip can be removed or pinned (kept for later turns). Right-clicking clips ▸ **Ask the assistant…** (UI-035) opens the panel with a selection chip.
**Inputs / controls:** Remove and pin per chip.
**Behavior / output:** Chips update until the message is sent and are frozen with it. A proposal that also changes clips outside the chips' scope says so on the card ("2 steps change clips outside your selection"); FreeCut default: warn, don't block.
**Edge cases:** Nothing selected: no selection chip. No in/out range: no range chip. The selection changes while a turn runs: the turn keeps the frozen scope.
**Undo:** n/a.
**Model notes:** Scope is part of the turn's user message as structured data, built by `freecut-app` from its selection and playhead state.

### AS-015 Transcript- and caption-aware editing — later
**What the user sees:** With transcripts (AI-009), the user can edit by meaning: "remove the part where I talk about pricing", "add a chapter marker at each new topic", "fix the caption that says 'their'". The model reads word-timed transcript excerpts and proposes cuts, caption edits (TX-025–TX-027) and markers (TL-037). A *Transcript excerpt* chip shows when a turn sends transcript text.
**Behavior / output:** Word → time positions come from the transcript's word timings, so cuts land on word boundaries before frame rounding (TL-001). Transcript and caption text are untrusted data (AS-019). Long transcripts are paged and searched (AI-027).
**Edge cases:** Stale transcript (AI-001): the model is told, and the user is offered a re-run. No transcript: the model can offer to start transcription (AS-013). Mixed languages: sent as is.
**Undo:** One entry per accepted proposal (AS-008).
**Model notes:** Transcript access goes through the host query interface; the assistant crate does not depend on `freecut-analysis`.

### AS-016 Frames for the model (vision) — later
**What the user sees:** If **Let the assistant look at frames** is on (Preferences ▸ Assistant; off by default; for hosted providers it is also listed in AS-004's dialog), the model can ask for still frames of the current or proposed edit to check framing, find a shot or judge whether a title is readable. A *Frames: 3* chip and small thumbnails in the activity line show what was sent.
**Inputs / controls:** The toggle; FreeCut defaults: at most 8 frames per turn and 2 per call, 768 px on the long side, JPEG quality 80.
**Behavior / output:** Frames are rendered off the UI thread by the same compositor as preview and export (FX-001), at the requested time, without the FreeCut UI.
**Edge cases:** Offline media renders its slate. A model without image input is not offered the tool. The limit is reached: the call is refused and the model is told.
**Undo:** n/a.
**Model notes:** A host query answered by the app's render job; images travel as JPEG bytes in a tool result.

### AS-017 Sound-aware questions — later
**What the user sees:** The model can answer and act on "is the voice too quiet in the middle?", "where are the long pauses?" or "cut on the beat" using numbers: loudness per window (AU-015's measures), peaks and silent ranges (PM-072, AI-003) and beats (AU-025). No audio is ever sent to a model.
**Behavior / output:** Missing analyses are queued as local jobs; the turn waits up to 60 s for them (FreeCut default), then answers with what it has and says what is pending.
**Edge cases:** Analysis fails: the error is reported to the model as a tool error. Silent sources (AU-040's flag): reported as silent.
**Undo:** n/a (edits that follow are AS-008's).
**Model notes:** Host queries answered by `freecut-audio` and `freecut-media` jobs in the app.

### AS-018 Limits and spending guard — v1
**What the user sees:** Preferences ▸ Assistant ▸ **Limits**, and a token readout per turn (input and output tokens, as the provider reports them; counted locally, never sent anywhere).
**Inputs / controls (FreeCut defaults):** per turn at most 25 model requests, 60 tool calls, 200 planned steps, 8 frames and 4 cloud run sheets (CG-024); request bodies up to 2 MB; optional **Stop a turn after N output tokens**; connect timeout 10 s, first data within 90 s, at most 120 s between data; 2 retries for rate limits and server errors, honoring the provider's retry delay.
**Behavior / output:** When a limit is reached, the turn stops as with AS-010 (planned steps kept as an incomplete proposal) and the line names the limit. One turn at a time per project.
**Edge cases:** Provider rate limit lasting longer than the retries: the turn fails with the provider's message and the suggested wait. A limit lowered during a turn: applies from the next turn.
**Undo:** n/a.
**Model notes:** `Limits` lives in `freecut-assistant`; the session enforces it, so a misbehaving model cannot loop.

### AS-019 Content from media is data, not instructions — v1
**What the user sees:** Usually nothing. The proposal card lists what the model read during the turn ("Read: transcript of *Interview.mp4*, 2 text clips, 3 file names"), so the user can judge where an unexpected proposal came from.
**Behavior / output:**
- Everything that did not come from the user's own chat — file names, media metadata tags, text-clip content, captions, transcripts, marker notes, frame content — is sent inside tool results as quoted data, and FreeCut's instructions tell the model that such data never contains instructions for it.
- The design does not depend on the model obeying: the model can only propose (AS-008), every confirmation is FreeCut UI that model text cannot imitate, there is no tool that deletes files, opens URLs or contacts other services (CG-024's actions only open FreeCut's cloud run sheet, which the user must confirm), and the panel never loads remote content (links in replies are shown as text with a copy button, and no remote images are fetched).
- Control characters are stripped from data, and any single metadata field is cut to 2 KB (FreeCut default).
**Edge cases:** A project received from someone else contains hostile text in a clip or file name: at worst it yields a proposal the user sees and can discard.
**Undo:** n/a.
**Model notes:** Untrusted strings are wrapped by one function in `freecut-assistant` (llm-editor.md §6.3); the evaluation scenarios include injection cases (§7).

### AS-020 Conversation history and storage — v1
**What the user sees:** The conversation lives in memory and ends when the project closes (FreeCut default). Preferences ▸ Assistant ▸ **Keep conversations with the project** (off) saves it next to the project; the panel menu has **New conversation**, **Copy conversation** and **Delete saved conversation**.
**Behavior / output:**
- A saved conversation is a sidecar file `<project name>.freecut-assistant.json` beside the project file, never inside it, so sharing a project does not share chats; packaging (PM-101) leaves it out unless the user ticks it.
- What was sent to a provider is never edited afterward. When a conversation grows past its budget (60 % of the model's context, FreeCut default), FreeCut starts a fresh provider conversation that begins with a FreeCut-written recap and a new project brief; a divider in the panel marks the point. The full history stays visible locally.
**Edge cases:** Sidecar from a newer FreeCut or damaged: ignored with a note, never deleted. Disk full while saving: a toast (UI-040), the conversation stays in memory. Saving runs on the project save worker, never on the UI thread.
**Undo:** n/a.
**Model notes:** The sidecar's serde DTOs and atomic write live in `freecut-project` next to the other sidecar types; the recap builder lives in `freecut-assistant`.

## Interactions with other areas

- **Timeline (TL):** every edit is a `freecut_timeline::edit::EditRequest`, planned into a read-only `Plan` (validated, previewable, stamped) and committed through `Editor::commit` as one history entry (TL-043). Batch skips follow TL-053, availability TL-054, selection TL-015. This spec asks TL for three pure additions: `PlanChain` (multi-step plans merged into one plan), `OpKind::AssistantEdit`, and `edit::summarize` for change lists (llm-editor.md §8).
- **Every area with edit requests (PM, TR, TX, AU, CL, FX, OV):** a new `EditRequest` variant needs an assistant tool or a documented exclusion (llm-editor.md §3.3). The owning spec still defines the behavior; the assistant only describes the arguments.
- **Smart tools (AI):** macros (AS-012) reuse AI-003, AI-006, AI-010 and AI-011 apply steps; transcripts, beats and silence come from AI-001 records; model downloads keep AI-002's consent. ai-tools.md's local-only rule stays as is: no AI tool needs the assistant or the network.
- **Text (TX):** text clips, styles and captions are edited through TX's requests; transcripts and captions are untrusted data.
- **Preview and export (EX):** Play proposal uses the normal player; export goes through EX-021's dialog (AS-013) and always renders the current document.
- **Projects (PM):** the conversation sidecar (AS-020); import only through user-chosen files (PM-030).
- **Workspace (UI):** the panel is a UI-056 auxiliary panel; the palette row extends UI-034; context-menu entry UI-035; settings UI-018; toasts UI-040; strings UI-044; key bindings UI-031.

## Out of scope (and why)

- **A FreeCut-run cloud service, account, credits or bundled model.** FreeCut is a local app; the user brings their own provider. A bundled local model would add gigabytes and license review; users install one through their local server instead.
- **Unattended auto-apply** (edits that apply without Accept). Every model output stays a proposal; this is the core safety property.
- **Sending media files or audio to a model.** Only file names, numbers, text and small rendered stills (AS-016) are sent.
- **Web browsing, stock search or calling other services from the assistant.** No tool exists for it. (CG-024's cloud generation actions only open FreeCut's run sheet; the user's Run click contacts the service.)
- **Generated media from the assistant's own provider** (images, video, music or voices from a language-model vendor). AI-026's external-generator extension point covers user-installed generators; voice cloning stays out of scope (ai-tools.md). Cloud generation through the user's own Higgsfield account is a separate opt-in area ([cloud-generation.md](cloud-generation.md)); CG-024 gives the assistant action tools that only open FreeCut's run sheet.
- **Training or fine-tuning on the user's projects, and telemetry.** Never.

## Clean-room notes

- The reference editor offers a cloud editing agent that needs sign-in. It was observed only as UI labels, paraphrased into a short behavior list (plain-language requests, a task list with confirmations, partial failures, a busy service). FreeCut's assistant differs on purpose: local-first, bring-your-own provider, proposals with a FreeCut-generated change list, and one undo entry.
- FreeCut's instructions to the model, tool names, tool descriptions, macro recipes, defaults and panel design are FreeCut's own. No reference-app prompts, strings, icons or assets are used.
- Provider integrations follow the providers' public API documentation. No vendor SDK code is copied; FreeCut talks to the HTTP APIs directly.

## Sources

- Anthropic, Messages API and tool use — https://platform.claude.com/docs/en/agents-and-tools/tool-use/overview.md
- Anthropic, streaming messages — https://platform.claude.com/docs/en/build-with-claude/streaming.md
- Anthropic, prompt caching — https://platform.claude.com/docs/en/build-with-claude/prompt-caching.md
- Anthropic, models overview — https://platform.claude.com/docs/en/about-claude/models/overview.md
- Ollama, OpenAI compatibility — https://github.com/ollama/ollama/blob/main/docs/openai.md
- llama.cpp HTTP server — https://github.com/ggml-org/llama.cpp/tree/master/tools/server
- JSON Schema specification — https://json-schema.org/specification
- OWASP Top 10 for LLM applications, prompt injection — https://genai.owasp.org/llmrisk/llm01-prompt-injection/
- `keyring` crate documentation — https://docs.rs/keyring
