# Cloud generation: Higgsfield through MCP

> Status: draft · 2026-10-13 · Crates: freecut-cloud (proposed, new), freecut-timeline (one `GeneratedKind` variant, one pure function), freecut-media (upload transcodes, probe), freecut-project (store and inbox paths), freecut-assistant (CG-024 action tools only), freecut-app (UI, cloud worker, keychain, browser and loopback sign-in, ledger file) · Features: CG-001…CG-026 ([cloud-generation.md](../features/cloud-generation.md)) · Decision: README §6, C55

The user wants to connect their own Higgsfield account and generate or edit images and video
from inside FreeCut. This document fixes how: which Higgsfield product FreeCut talks to and how
it signs in, where the code lives, the MCP client, the job and ledger model that keeps paid work
from being lost or paid twice, the upload and download legs, provenance, error mapping, testing
without a network, and the build slices H1–H4.

Everything here is opt-in and off by default. FreeCut runs no server. The rules of the features
README (*Local only*) and of the assistant (llm-editor.md) still hold; this is a second, separate
opt-in exception that the owners must confirm (§15).

---

## 0. Principles

| # | Principle | Consequence |
|---|---|---|
| Q1 | **The user pays, so the user decides.** | Every paid call follows a cost preflight and an explicit Run click on FreeCut's own sheet (CG-005). No setting skips it, and the assistant cannot press it. |
| Q2 | **Never resubmit an unknown outcome.** | Paid submissions have no retry wrapper anywhere. A request that may have reached the service and got no answer becomes *Outcome unknown* (CG-010). Only free, read-only calls and presigned uploads are retried. |
| Q3 | **Send the minimum, say exactly what.** | Only the chosen section or frame, re-encoded, metadata stripped, generic file names. The disclosure is generated from the same `UploadPlan` that drives the upload. |
| Q4 | **Typed operations, not a proxy of the server.** | FreeCut maps its own `Operation` enum onto an allow list of tools. The server offers 100+ tools, many of which spend money or publish; FreeCut never exposes the raw list to anyone, including the assistant. |
| Q5 | **Sans-I/O core, offline tests.** | Protocol, OAuth, catalogue, job states, recipe and error mapping are pure and tested with recorded exchanges. CI never touches the network; the only sockets in tests are on `127.0.0.1`. |
| Q6 | **Project conventions hold.** | Time is `Tick` until the request's seconds field; edits go through the undo stack; no network, JSON or file I/O on the UI thread; `thiserror` in the crate, `anyhow` only in the app; `#![forbid(unsafe_code)]`. |
| Q7 | **Nominative naming, the service's own sign-in.** | "Connect your Higgsfield account", no logo, an explicit "not affiliated" line; sign-in happens in the system browser on Higgsfield's pages, never in a FreeCut form or web view. |

---

## 1. What Higgsfield offers, and what FreeCut uses

Facts checked on 2026-10-13 from public pages, the public OAuth discovery documents, and the
tool schemas and free model listing of the user's own connected MCP server. Nothing was
generated, uploaded or charged; no private account data was read.

| | MCP server | REST API |
|---|---|---|
| Endpoint | `https://mcp.higgsfield.ai/mcp` ("the only official connector URL") | `https://api.higgsfield.ai` |
| Auth | OAuth 2.1 (protected-resource metadata; authorization server `https://mcp.higgsfield.ai` with `/oauth2/authorize`, `/oauth2/token`, `/oauth2/register`; PKCE S256; grants authorization_code, refresh_token, device_code; scopes `openid email offline_access`; upstream identity provider Clerk). Help center: "No API key needed". | `Authorization: Key <id>:<secret>`, created in the API console |
| Billing | The user's Higgsfield **plan credits**; needs a paid plan; no spending cap | Prepaid USD balance, separate account |
| Cost preflight | `get_cost: true` on the generate tools | `POST /estimate/{model}` |
| Jobs | Submit returns job ids; `jobs_wait` long-polls up to 12 jobs, ≤ 15 s | `request_id`, `/requests/{id}/status`, `/cancel` |
| Client guidance | "Any MCP-compatible agent can use the same server URL" | "Do not call the API directly from browser or mobile application code" |
| Model ids | `seedance_2_5`, `gpt_image_2_5`, … (catalogue via `models_explore`) | path-style, e.g. `higgsfield-ai/soul/v2/standard` |

An unauthenticated request to `/mcp` answers `401` with
`WWW-Authenticate: Bearer resource_metadata=".../.well-known/oauth-protected-resource/mcp"`,
which is the MCP authorization flow. Higgsfield's pages do not name the transport; a public
third-party guide connects it as HTTP, and the behaviour matches Streamable HTTP. Treat it as
Streamable HTTP and confirm in H1 (§15).

**Decision.**

1. FreeCut talks to the **MCP server**: it is the product Higgsfield offers to third-party
   agents, it bills the plan the user already has, and its tool set covers editing (video edit,
   extension, background removal, upscale, reframe, dubbing, lip sync).
2. **Sign-in** is OAuth in the system browser (authorization code + PKCE with a loopback
   redirect, RFC 8252), with device code as a fallback when offered. This is how a user "brings
   their own token" to an MCP server that has no token to paste.
3. **Paste an access token** is kept as an advanced path (CG-001): a bearer token the user
   obtained elsewhere, stored the same way, never renewed by FreeCut. Higgsfield documents no
   personal access token for the MCP server, so this path may never be needed; it costs one
   `Credential` variant.
4. The **REST key pair** is not built until Higgsfield confirms in writing that a desktop app may
   store a user's own key (Terms §11.3 forbids embedding keys in "client-side" code). The
   provider abstraction (§6) keeps the door open.

### 1.1 Terms of use checklist

From Higgsfield's terms (updated 2026-07-26) as read on 2026-10-13. Section numbers come from a
summarised reading and are re-checked against the page before release (§15).

| Terms | FreeCut's design |
|---|---|
| Developer Access covers API, MCP and CLI (§11, §1.5) | FreeCut is a client acting on the user's own grant; the user is the account holder. |
| Keys must not be shared or embedded in client-side or public code; store securely (§11.3–11.4) | FreeCut ships **no** keys or client secrets (public OAuth client, `token_endpoint_auth_method: none`). The user's tokens live only in the OS keychain. |
| No resale, sublicensing or pass-through without added value (§11.5, §5.2) | FreeCut charges nothing and runs no service; generation is one step inside an editor. |
| Provide privacy notices and obtain consents for end users (§11.5–11.6) | CG-004's account notice and per-run Sends list; per-project opt-out. |
| Do not present outputs as human-made; legal AI disclosures (§11.7) | Every result is badged *AI-generated* and carries its recipe (CG-011). Labels required by publishing platforms remain the user's job; FreeCut adds no watermark (EX-040). |
| No implied endorsement through name or logo without written consent (§11.8) | Plain-text naming, no logo or colors, "not affiliated" line (CG-001). |
| The user is responsible for connected agents and tools (§11.12–11.13) | Stated in the connect sheet. |
| Rights and consent for likeness and voice; content rules (§4.2(b), §5) | Required consent tick for dub, lip sync, voice change and edits of footage with people (CG-005). |
| Higgsfield may train on inputs and outputs; outputs kept ≥ 7 days | Stated in the account notice (CG-004); FreeCut downloads promptly and marks runs *Expired* after 7 days. |
| Report suspected key or credential leaks within 24 h (§11.4) | The obligation is the account holder's; Disconnect revokes the tokens and the sheet says to report a leak to Higgsfield (CG-002). |
| Outputs may carry provenance watermarks or metadata (§6.4) | FreeCut keeps each downloaded file byte-for-byte as received. Export re-encodes, as for all media, which can drop file-level metadata; whether the terms forbid that is **unknown** (§15). |
| Higgsfield has no desktop app, and its name inside another product may mean a copy (official-platforms page) | The connect sheet says FreeCut is not Higgsfield's app and names the sign-in host the browser should show (CG-001); sign-in happens only on Higgsfield's own pages. |
| Does FreeCut's project count as a "developer" with §11 obligations of its own? | **Unknown**: FreeCut ships no keys, runs no service and acts only on each user's own grant; ask Higgsfield (§15). |

---

## 2. Where it sits

### 2.1 Options

| Option | For | Against |
|---|---|---|
| (a) Inside the planned `freecut-assistant`, behind its `http` feature | Reuses its `ureq`, `SecretStore` and test tooling; one networking crate | Generation is not language-model work and has its own consent, billing and job lifetime. llm-editor.md §6.1 states that the assistant contacts **only** its configured provider; putting a second service in the same crate blurs that safety invariant. A build without the assistant (`--no-default-features` on its app feature) would lose generation too. The assistant milestones (M-AI1) would have to land first. |
| (b) **A new crate `freecut-cloud`** | Clear owner for OAuth, MCP, catalogue, job ledger and provider mappings; sans-I/O core testable without egui or a network; its own `http` feature and dependency guard; the assistant reaches it only through the app, as an action; buildable and shippable before the assistant | An eighth crate (after assistant and analysis); CLAUDE.md must be updated by the owner |
| (c) `freecut-app` only | No new crate, no CLAUDE.md change | The job state machine, OAuth and error mapping are exactly the code that must be unit-tested hardest (they guard money); the app has no clean seam for that, and `anyhow` there loses typed errors. Extracting later costs more than starting separate. |

**Recommendation: (b), `freecut-cloud`.** If the owner prefers not to add a crate yet, use (c)
with the same module layout under `freecut-app/src/cloud/` (`mcp`, `oauth`, `catalogue`,
`job`, `provider`) and no egui imports in those modules, so they can move into a crate
unchanged.

### 2.2 The crate

| Crate | Responsibility | Owns | Must never |
|---|---|---|---|
| `freecut-cloud` | Talking to user-connected generation services | MCP client (JSON-RPC 2.0 over Streamable HTTP, SSE parsing, sessions, cancellation); OAuth client (discovery, registration, PKCE, device code, refresh, revoke); `Credential` and `SecretStore` trait; `GenerationProvider` trait and the Higgsfield mapping; tool allow list; `Catalogue` and filters; `Operation`, `Quote`, `ServiceQuestion`, `Adjustment`; `UploadPlan` and `Disclosure`; `CloudJob` state machine; ledger DTOs; recipe builder; error classifier; `Limits`; `ScriptedProvider` and the fake MCP server for tests | Depend on egui, render, audio, project, assistant or app; open sockets unless built with `http`; read or write project files; decode media; run on the UI thread; use `unsafe` |

`freecut-app` owns the Preferences section, the Generate tab and run sheet, the cloud worker
thread, the keychain implementation, launching the browser, the loopback redirect listener, the
ledger file, transcode and render jobs for uploads, and committing imports and placements.

### 2.3 Dependency direction (rule update)

```
   freecut-app ──────────────► freecut-cloud ──► freecut-timeline (Tick, TimeRange, MediaId,
        │                            │                             GeneratedMediaKind for recipes)
        │                            └─────────► freecut-media (default-features = false:
        ▼                                                         CancelToken only, no FFmpeg)
   (everything)
```

| Crate | May depend on |
|---|---|
| `freecut-cloud` | timeline; media **with `default-features = false` only** (for `freecut_media::cancel::CancelToken`, README C18) |
| `freecut-app` | everything, now including cloud |

Forbidden: anything → cloud except app; cloud → render, audio, project, assistant, analysis,
app. The assistant does **not** depend on cloud: CG-024's tools are action tools that the app
routes (§12). The dependency guard gains the edge list and the check that `cargo tree -p
freecut-cloud --features http` contains no `ffmpeg-sys-next`, `wgpu` or `egui`.

Feature flags:

| Crate | Feature | Enables | Default |
|---|---|---|---|
| `freecut-cloud` | `http` | `ureq` (blocking, rustls; the same version as the assistant and C49's downloader), `sha2`, the real transport and OAuth endpoints | on in `freecut-app`'s dependency; **off** for `cargo test -p freecut-cloud` |
| `freecut-app` | `cloud` | the Preferences section, Generate tab, worker | on (runtime switch still **off** by default, CG-001); a distributor may build without it |

New third-party crates, licences confirmed at adoption: `ureq`, `sha2`, `base64` and `rand`
(PKCE verifier and state; or `getrandom` alone), `url`, `uuid` (local run ids),
`keyring` (shared with AS-003), `open` or the platform's URL opener for the browser. All are
MIT/Apache-2.0 today. No async runtime (README §2.1).

### 2.4 Proposed CLAUDE.md lines (owner approval required)

CLAUDE.md lists six crates today; the assistant (C45) and analysis (C48) additions are also
pending. The exact lines for cloud, written to sit with them:

In **Architecture**, after the `freecut-app` bullet:

```
- `freecut-cloud` — opt-in generation through the user's own account on a
  third-party service (Higgsfield first): MCP client, OAuth sign-in, model
  catalogue, cloud job ledger. Off by default; networking only behind its
  `http` feature. No FreeCut server, no telemetry.
```

Replace the dependency sentence with (the assistant and analysis parts as C45 and C48 propose):

```
Dependency direction: `app` → everything; `render`/`audio` → `media` +
`timeline`; `cloud` → `timeline` + `media` (without FFmpeg, for the cancel
token only); `timeline` depends on nothing internal. Nothing but `app`
depends on `cloud`.
```

In **Conventions**, one new bullet:

```
- Paid cloud calls are never retried or resent automatically; every one
  needs the user's confirmation after a cost preflight
  (docs/features/cloud-generation.md, architecture §4.5–§4.7).
```

---

## 3. Components and threads

```
 UI thread (app)                          Cloud worker (app thread, 1 per app)
 ───────────────                          ───────────────────────────────────
 Preferences ▸ Cloud generation           CloudService (freecut-cloud, sans-I/O core)
 Generate tab, run sheet  ── CloudCmd ──►  ├─ McpClient ─► HttpTransport (ureq, http feature)
 Runs page, task rows   ◄─ CloudEvent ──   ├─ OAuth: refresh / revoke; SecretStore (keychain)
 Editor (sole owner)        (crossbeam)    ├─ Scheduler: Limits, queued runs, one poller
 import + placement commits                ├─ Ledger (DTOs; file written via app's atomic save)
                                           └─ transfer requests ─► PM-073 I/O pool (2 workers):
 App job pool (PM-073)                          presigned PUT with progress, result download
  ├─ upload prep: freecut-media transcode       with SHA-256 and resume
  │  (section → MP4/WAV), CPU compositor
  │  (current frame → PNG)
  └─ probe of downloaded results (PM-035)
 Loopback listener thread (sign-in only, 127.0.0.1, one request, ≤ 5 min)
```

Thread table row, added to README §2.1 when H1 starts:

| Thread | Crate | Count | Priority | Owns | Talks through | Never |
|---|---|---|---|---|---|---|
| Cloud worker | app (driving cloud) | 1 per app while cloud generation is on; parked when idle and no job is running | normal | `CloudService`, the MCP session, tokens in memory for the duration of a request, the scheduler and poller | crossbeam `CloudCmd` in, `CloudEvent` out (bounded, 64); one `CancelToken` per run; transfers posted to PM-073's I/O pool | touch the `Editor` or the project file, decode, render, block the UI thread |

Rules:

1. The UI thread never waits on the network, the keychain or JSON. The run sheet shows a
   spinner until `CloudEvent::Quote` arrives.
2. One worker per app, not per project: runs outlive projects (CG-013), and one account needs
   one MCP session and one poller.
3. Uploads and downloads are plain HTTPS to presigned or result URLs, run on PM-073's I/O pool
   so they share its throttling and pause rules; they are not MCP traffic and never carry the
   bearer token.
4. Import and placement are commits by the UI thread from a `CloudEvent::ResultReady`, planned
   against the current document (CG-012), never from the worker.

---

## 4. MCP client

### 4.1 Transport: Streamable HTTP

- Every JSON-RPC message is a `POST` to the endpoint with `Content-Type: application/json`,
  `Accept: application/json, text/event-stream`, `Authorization: Bearer <access token>`, and
  after initialization `MCP-Protocol-Version: <negotiated>` and `Mcp-Session-Id: <id>` when
  the server issued one.
- The response is either a single JSON body or an SSE stream (`text/event-stream`). The SSE
  parser accepts events split across reads, `data:` lines joined with `\n`, comments and `id:`
  fields; the event whose JSON-RPC `id` matches the request is the response. Other messages in
  the stream are handled as below.
- **Server-to-client messages.** FreeCut declares no client capabilities (no sampling, roots or
  elicitation). Server requests are answered with JSON-RPC error `-32601`; `notifications/progress`
  and `notifications/message` are logged at `debug`; anything else is ignored.
- FreeCut does not open the optional standalone `GET` stream; it polls with `jobs_wait`. It does
  use `GET` with `Last-Event-ID` to **resume** an answer's stream that the server ended after an
  event id but before the response (MCP 2025-11-25 polling): after the server's `retry` delay
  (1 s if none, at most 30 s), at most 5 times. A `GET` never resends the request; if resuming
  fails, the answer is lost, so a paid call is *Outcome unknown* and a free call fails (review
  F4).
- **Sessions.** The `Mcp-Session-Id` from the `initialize` response is kept in memory for the
  worker's lifetime. HTTP `404` on a request carrying a session id means the session expired:
  FreeCut re-initializes once. A **free** call (§4.5's table) is then sent again. A **paid**
  submission is never sent again on its own, even though the MCP specification implies the
  request was not processed: a `404` can also come from a proxy or a server fault, and Q2 allows
  no exception. The run becomes *Not accepted* (CG-010) and the user presses Run again on a
  fresh sheet. To keep this rare, the re-quote that precedes every submission (§7.1) runs on the
  same session moments before it, so an expired session is found by a free call.
  On disconnect and on quit, FreeCut sends `DELETE` with the session id (best effort, 2 s).

### 4.2 Lifecycle

1. `initialize { protocolVersion, capabilities: {}, clientInfo: { name: "FreeCut", version } }`.
   FreeCut offers the newest MCP revision it implements (2025-11-25 at the time of writing; at
   least 2025-06-18, which added `structuredContent` and the protocol-version header) and accepts
   the server's choice if it is a revision FreeCut supports, otherwise fails with "unsupported
   service version".
2. The server's `instructions` text is addressed to chat agents. FreeCut keeps it for the debug
   view only and never acts on it.
3. `notifications/initialized`.
4. `tools/list`, following `nextCursor` until done. The result is checked against the allow list
   (§6.3); each allowed tool's `inputSchema` is stored for argument validation (§4.6).

### 4.3 Calling tools

`tools/call { name, arguments }` returns `{ content: [...], structuredContent?, isError? }`.
FreeCut reads `structuredContent` when present, otherwise parses the first `text` content item
as JSON, otherwise treats the text as a message. `isError: true` and JSON-RPC errors both go to
the error classifier (§10). Every value from the server is untrusted data: it is parsed into
FreeCut's own types, and text shown to the user is shown as quoted plain text.

### 4.4 Cancellation

- A local cancel of an in-flight request sends `notifications/cancelled { requestId, reason }`
  and stops reading. The server may still have acted: for a paid submission the run becomes
  *Outcome unknown* (CG-010) unless the answer arrives before the socket closes, in which case
  its job ids are recorded.
- The service has no cancel for submitted generation jobs (none in the tool list as of
  2026-10-13). **Stop waiting** stops polling and downloads only, and says so (CG-009). If a
  provider offers a cancel, `GenerationProvider::cancel_remote` reports it and the row gains
  **Cancel on the service**.
- `ureq` 3.4 (README §4.2) has per-phase timeouts (resolve, connect, send request, send body,
  receive response, receive body) but no per-read idle timeout, so a blocked read cannot be woken
  to check a `CancelToken`. Cancellation therefore works in two ways: streaming loops (SSE
  parsing, uploads through the counting reader, downloads) check the token between chunks; and a
  call that is blocked waiting is **abandoned**: the worker runs each call on a short-lived helper
  thread, waits on the call's `crossbeam-channel` result with `recv_timeout(250 ms)` and checks
  `CancelToken::is_cancelled()` between waits (or selects on the token's `receiver()` once C18's
  version of it exists; today's `freecut_media::cancel::CancelToken` has no receiver), and on
  cancel stops waiting; the helper ends on its own phase timeouts and its result is
  discarded, except that job ids from a paid submission are still recorded (§7.2). llm-editor.md
  §2 rule 2 assumes a 1 s socket read timeout with the same crate; the same correction applies
  there (§15).
- After each `CloudEvent` the worker calls `egui::Context::request_repaint()` (through the
  app's handle), so the UI updates without polling.

### 4.5 Timeouts and retries

| Call | Connect | Whole call | Automatic retry |
|---|---|---|---|
| `initialize`, `tools/list` | 10 s | 30 s | 3 tries, 1 s / 2 s / 4 s ± 25 % jitter, on connect errors, 5xx, 429 |
| `models_explore` (catalogue) | 10 s | 30 s | as above |
| cost preflight (`get_cost: true`) | 10 s | 60 s | **only while `NotSent`** (§4.7): it shares its tool with the paid call, so a preflight that may have reached the service is never repeated |
| `jobs_wait` (server waits ≤ 15 s) | 10 s | 45 s | unlimited while the run is active, with back-off to 60 s while offline |
| `media_upload`, `media_confirm` | 10 s | 60 s | `media_upload` as above (it only creates upload slots); `media_confirm` once, then *Failed* |
| presigned `PUT` | 10 s | no total; 60 s without progress | 2 more tries to the same URL while it is valid |
| result download | 10 s | no total; 60 s without progress | resume with `Range` up to 5 times |
| **paid submission** (any generate or edit tool without `get_cost`) | 10 s | 120 s | **never**; connect failure before the body was written → *Failed: not sent*; anything after → *Outcome unknown* |

`ureq` does not report how much of a request was written, so the transport derives
`SendState::NotSent | MaybeSent` from the error alone, and it is the only input to the
unknown-outcome decision. `NotSent` only for errors that can happen before the first request
byte: `Error::HostNotFound`, `Error::ConnectionFailed`, `Error::Timeout(Resolve | Connect)`
(connect includes the TLS handshake), `Error::Rustls` carrying a certificate error
(`InvalidCertificate`, `NoCertificatesPresented`; other rustls errors can occur mid-stream and
count as `MaybeSent`), and `Error::Io`
whose kind is `ConnectionRefused`, `NetworkUnreachable`, `HostUnreachable` or
`AddrNotAvailable`. Everything else is `MaybeSent`. The `io::ErrorKind` rule assumes those four
kinds come only from `connect(2)`; once a connection is up, a broken one reports a reset, a
broken pipe or a timeout, which are `MaybeSent` (review F7). The cancel gate's refusal
(review M3) is the one other `NotSent` case. A test pins this table against ureq's error
enum, so a ureq upgrade that adds variants fails the build until they are classified.

Further transport rules for paid calls:

- **Paid by default.** The allow list (§6.3) marks the free tools; every other call, and any
  call whose classification is unclear, is treated as paid (no retry, unknown-outcome rules).
- **Fresh connection.** Paid submissions use an agent with `max_idle_connections(0)`, so a stale
  pooled socket cannot turn an ordinary refusal into *Outcome unknown*. `ureq` does not retry
  `POST` on its own; a test with the loopback server asserts exactly one request per call.
- **Proxy.** `HTTPS_PROXY` is honoured as in llm-editor.md §5.6; TLS uses rustls with the
  platform's certificate roots (`RootCerts::PlatformVerifier`, ureq's `platform-verifier`
  feature), as the assistant does.

### 4.6 Schema drift

Tool names, model ids and descriptions change on the server without a changelog. FreeCut:

- builds arguments only from its own `Operation` mapping (§6.4);
- validates them against the live `inputSchema` from `tools/list` (required fields, types,
  enums, numeric ranges; a small validator in `freecut-cloud`, no full JSON Schema crate unless
  review prefers one);
- disables an action whose tool is missing or whose arguments no longer validate, with "Not
  offered by the service right now", instead of guessing;
- builds option controls from the catalogue (§6.2), so new models and options need no release.

### 4.7 The preflight guard

The cost preflight and the paid run are the same tool call, differing only in `get_cost: true`.
A dropped or renamed flag would turn a free check (and its retries) into paid runs. So:

1. The quote path takes a `QuoteRequest` type whose only constructor sets the flag; after
   serialization, the transport checks that the outgoing JSON has `get_cost: true` at the
   expected place and refuses to send otherwise (a bug, not a user error).
2. A preflight is sent only if the tool's live `inputSchema` (§4.6) still lists `get_cost` as a
   boolean; otherwise the action has no estimate and CG-005's rules apply.
3. Preflights are retried only while `NotSent` (§4.5).
4. A preflight answer that contains job ids is treated as a started run: a ledger entry is
   written in `Running` with those ids, it is polled and imported like any run, the user is told,
   and preflights for that tool stop until restart.
5. Tests: golden JSON for every quote; a fake server that ignores `get_cost` must produce exactly
   one submission per Check cost and the "started run" path, never more.

---

## 5. Sign-in (OAuth 2.1)

### 5.1 Discovery

From the `401`'s `resource_metadata` URL (RFC 9728) to the authorization server metadata
(RFC 8414). Only the MCP endpoint URL is built in; every other endpoint comes from metadata,
and FreeCut checks that the metadata's `issuer` and `resource` match the expected origin
(`https://mcp.higgsfield.ai`). The protected-resource metadata also lists the upstream
`clerk.higgsfield.ai` and a device-auth host; FreeCut uses the server that the
`authorization_servers` list and the authorization server's own metadata point to, and re-checks
the hints at H1.

### 5.2 Client registration

- **Dynamic Client Registration** (RFC 7591) at `/oauth2/register`, once per installation:
  `client_name: "FreeCut"`, `redirect_uris: ["http://127.0.0.1/callback"]` (loopback; the port is
  chosen per sign-in as RFC 8252 §7.3 allows), `grant_types: ["authorization_code",
  "refresh_token"]`, `token_endpoint_auth_method: "none"`, `application_type: "native"`. The
  returned `client_id` is not secret and is kept in the preferences (it is also stored with the
  tokens so a mismatch is detected).
- **Client ID metadata document** (the metadata says it is supported) is the alternative if
  registration rejects loopback redirects: the `client_id` is an HTTPS URL of a static JSON file
  published with FreeCut's website. That is a static file, not a service, but it is still a
  hosting dependency the owner must accept (§15).
- No client secret exists anywhere.

### 5.3 Authorization code with PKCE

1. The worker creates a 64-byte random verifier, its S256 challenge and a random `state`.
2. The app binds `127.0.0.1:0` (IPv4 loopback only), builds the authorize URL (`response_type=code`,
   `client_id`, `redirect_uri=http://127.0.0.1:<port>/callback`, `scope=openid email
   offline_access`, `code_challenge`, `code_challenge_method=S256`, `state`, `resource=
   https://mcp.higgsfield.ai/mcp` per RFC 8707) and opens it in the system browser.
3. The listener accepts exactly one request to `/callback`, checks `state`, answers with a
   static FreeCut HTML page (no scripts, no remote content) and closes. 5 minutes timeout.
4. The worker exchanges the code at the token endpoint with the verifier.
5. Tokens go to the `SecretStore`; the e-mail, if the ID token or user-info endpoint gives one,
   is shown in Preferences only.

FreeCut never embeds a web view, never sees the password and never stores cookies.

### 5.4 Device code (fallback)

Offered only when the authorization server metadata advertises a
`device_authorization_endpoint` (RFC 8628). FreeCut shows `user_code` and
`verification_uri`, offers to open the latter, and polls the token endpoint at the given
`interval`, honoring `slow_down`. As of 2026-10-13 the MCP authorization server lists the
device-code grant but its separate device host has no discovery document, so this path is
confirmed or dropped in H1.

### 5.5 Pasted token

`Credential::Bearer { token }` with no refresh and unknown expiry. A `401` turns the status into
*Sign-in expired* with "paste a new token or sign in". The token is validated for shape only
(non-empty, no whitespace, at most 8 KB) before storage.

### 5.6 Storage, refresh, revoke

```rust
pub enum Credential {
    OAuth { access: Secret, refresh: Option<Secret>, expires_at: Option<UnixSeconds>,
            scope: String, client_id: String, subject_hash: [u8; 16] },
    Bearer { token: Secret },
}
pub trait SecretStore: Send + Sync {
    fn get(&self, provider: ProviderId) -> Result<Option<Credential>, SecretError>;
    fn set(&self, provider: ProviderId, cred: &Credential) -> Result<(), SecretError>;
    fn delete(&self, provider: ProviderId) -> Result<(), SecretError>;
}
```

- Keychain service "FreeCut cloud generation", account = provider id plus a part suffix.
  Windows Credential Manager limits one credential to 2,560 bytes (and `keyring` stores the
  secret as UTF-16 there, about 1,280 characters), which a JWT access token, a refresh token and
  an ID token together can exceed; a pasted token may be up to 8 KB. So access token, refresh
  token and metadata are separate entries (`higgsfield/access`, `higgsfield/refresh`,
  `higgsfield/meta`), and an entry over 1,200 characters is split into numbered parts with a
  count in `meta`; all parts are written before `meta` is replaced, so a crash leaves the old or
  the new set, never a mix. The same `SecretStore` implementation serves AS-003.
  `Secret` zeroizes on drop and has a redacting `Debug`. Clearing memory is **best effort**
  (review SEC-8): FreeCut clears its own copies (secrets, keychain parts and their assembly
  buffer, request bodies after sending, token and device answers after parsing, the
  `Authorization` value it builds), but not copies made inside `ureq`, `http` or rustls, nor
  buffers a growing `Vec` left behind while reading an answer.
- Refresh when fewer than 60 s remain, and before any paid submission when fewer than 5 minutes
  remain. A `401` on a free call triggers one refresh and one resend; a `401` on a paid
  submission triggers the refresh but **not** a resend (the run becomes *Not accepted*, CG-010).
  Rotated refresh tokens replace the stored ones atomically. `invalid_grant` → *Sign-in
  expired*; nothing is deleted.
- `subject_hash` (first 16 bytes of SHA-256 of the token's `sub`) tags ledger entries with the
  account (CG-002) without storing the e-mail.
- Disconnect: revoke at the revocation endpoint if the metadata lists one, then delete.
- The HTTP logger redacts `Authorization`, `code`, `code_verifier`, `refresh_token` and
  `access_token` before formatting anything.

---

## 6. Providers, operations and the Higgsfield mapping

### 6.1 Provider-neutral interface

```rust
pub trait GenerationProvider: Send + Sync + std::fmt::Debug {
    fn id(&self) -> ProviderId;                       // "higgsfield"
    fn connect(&self, cred: &Credential, cancel: &CancelToken) -> Result<ServiceInfo, CloudError>;
    fn catalogue(&self, cancel: &CancelToken) -> Result<Catalogue, CloudError>;
    fn quote(&self, op: &Operation, inputs: &[RemoteInput], cancel: &CancelToken)
        -> Result<QuoteOutcome, CloudError>;          // Quote | Question | NotAvailable
    fn prepare_upload(&self, files: &[UploadSpec], cancel: &CancelToken)
        -> Result<Vec<UploadSlot>, CloudError>;       // presigned URL + headers + remote id
    fn confirm_upload(&self, slots: &[UploadSlot], cancel: &CancelToken)
        -> Result<Vec<RemoteInput>, CloudError>;
    fn submit(&self, op: &Operation, inputs: &[RemoteInput], answer: Option<&ServiceAnswer>,
              cancel: &CancelToken) -> Result<SubmitOutcome, SubmitError>;
    fn poll(&self, jobs: &[RemoteJobId], cancel: &CancelToken)
        -> Result<PollResult, CloudError>;            // states, result URLs, next poll delay
    fn cancel_remote(&self, job: &RemoteJobId) -> CancelSupport;  // Higgsfield: Unsupported
}

pub enum SubmitOutcome {
    Submitted { jobs: Vec<RemoteJobId>, adjustments: Vec<Adjustment> },
    Question(ServiceQuestion),      // e.g. UseAllowance { cap_variants_to: Option<u8> }
    FollowUp(FollowUp),             // the service asks for another step (CG-026)
}
pub enum SubmitError {
    NotSent(CloudError),            // safe to offer Run again without warning
    OutcomeUnknown(CloudError),     // CG-010
    Rejected(CloudError),           // the service answered with an error
}
```

`ScriptedProvider` (always built) implements the trait from a script for app and assistant
tests. A second service (for example the REST key pair, or another vendor) is a second
implementation plus its own catalogue mapping; the UI, job model, ledger, recipe and disclosure
do not change.

### 6.2 Catalogue

- Source: `models_explore` with `action: "list"` for each output type (image, video, audio);
  each listing returned `has_more: false` on 2026-10-13 (about 46 image, 50 video and 9 audio
  models). The listing is free and carries parameters, aspect ratios, durations (range or set),
  input roles and an allowance block; it carries **no prices**.
- Parsed into `ModelInfo { id, display_name, company, outputs, roles: Vec<Role>, durations:
  DurationSpec, aspects, resolutions, audio: AudioSupport, params: Vec<ParamSpec>,
  input_limits }`. Roles are the real signal of what a model takes (`start_image`, `end_image`,
  `image_references`, `video_references`, `audio_references`, `mask`, `input_video`,
  `input_audio`); the listing's per-media `type` field says "image" even for video inputs.
- Cached as `<app data>/FreeCut/cloud/catalogue-<provider>.json` with its fetch time.
- **Hidden** (a list in code, plus any model whose description marks it as restricted to the
  service's own pipelines): `sonilo_music`, `mirelo_text_to_audio`, `inworld_text_to_speech`
  (game pipeline only), `higgsfield_preset` (legacy recipes), `ms_image` and the other marketing
  studio models (styles chosen in the service's UI), the identity-trained `soul_*` family
  (needs identity training), `autosprite`, `clipify` (web addresses as input),
  `marketing_studio_v2_reference2video`.
- **Defaults** (FreeCut's, re-checked per release): image `gpt_image_2_5`; video
  `seedance_2_5` (text-to-video, frames-to-video, edit and extension modes); lip sync `sync_so`.

### 6.3 Tool allow list

| Tool | Use | Paid | Cost preflight |
|---|---|---|---|
| `models_explore` (`list`, `get`) | catalogue | no | n/a |
| `media_upload`, `media_confirm` | upload slots, confirmation | no | n/a |
| `generate_image` | CG-006, CG-015 (images) | yes | `get_cost` |
| `generate_video` | CG-007, CG-015, CG-016, CG-020 (`topaz_hyperion_2_5`), CG-022 (`sync_so`); CG-017 (`video_background_remover`) and CG-018 video (`bytedance_video_upscale`, `topaz_video`) instead of the dedicated tools below | yes | `get_cost` (for the CG-017/CG-018 models, confirmed with a free call in H3b before they ship) |
| `jobs_wait` | polling | no | n/a |
| `upscale_image` | CG-018 images | yes | `get_cost` |
| `reframe` | CG-019 | yes | `get_cost` (needs `duration_seconds` and resolution over 15 s) |
| `dubbing` | CG-021 | yes | not listed → **gated**: built only if the service adds an estimate or the owners accept the "cost shown afterwards" exception (CG-005) |
| `voice_change` | CG-023 | yes | as `dubbing` |
| `list_voices` | CG-023 voice picker, preset voices only, on opening the picker; custom voices in the answer are dropped | no | n/a |
| `balance` | CG-001 *Show balance*, on click only | no | n/a |

`upscale_video` and `remove_background` (no estimate) are **not** on the list; the catalogue
models above do the same work through `generate_video`. The "Paid" column is the transport's
input (§4.5): a tool is free only if this table says so.

Everything else is denied by construction (the mapping has no code path to it), including the
batch tools (no preflight), `execute_preset`, presets and workflows, projects, folders and
preferences, voice creation, websites, publishing, 3D, sandboxes and analytics. `media_import_url`
is not used: FreeCut uploads its own prepared files. The optional `context` (analytics) argument
is never sent; `folder_id` is never sent (FreeCut creates no service-side projects).

### 6.4 Operation mapping

```rust
pub enum Operation {
    Image { model: ModelId, prompt: String, refs: Vec<RoleRef>, aspect: Aspect,
            variants: u8, params: ParamValues },
    Video { model: ModelId, mode: VideoMode, prompt: String, refs: Vec<RoleRef>,
            aspect: Option<Aspect>, duration: Option<WholeSeconds>, resolution: Option<String>,
            sound: Option<bool>, variants: u8, params: ParamValues },
    RemoveBackground { input: RoleRef, media: MediaKind },
    Upscale { input: RoleRef, media: MediaKind, target: UpscaleTarget, source_px: (u32, u32) },
    Reframe { input: RoleRef, aspect: Aspect, section_secs: WholeSeconds, resolution: Option<String> },
    SdrToHdr { input: RoleRef, output: HdrOutput },
    Dub { input: RoleRef, language: LanguageCode },
    LipSync { video: RoleRef, audio: RoleRef, length_mode: String },
    VoiceChange { input: RoleRef, voice: VoiceId },
}
pub enum VideoMode { TextToVideo, FramesToVideo, EditByPrompt, Extend { forward: bool } }
```

The Higgsfield module (`provider/higgsfield/map.rs`) turns each into `(tool, arguments)`:
`params.model`, `params.prompt`, `params.medias: [{ value: <media or job uuid>, role }]`
(never URLs), `aspect_ratio`, `count` (variants), `duration` (whole seconds), model-specific
fields at the top level of `params` (for example `mode: "video_extension"`,
`extension_mode: "forward"` for `seedance_2_5`), and `get_cost: true` for the quote. Golden JSON
tests pin every mapping.

**Service fields handled** (from the tool descriptions; exact JSON shapes are recorded in H1/H2):

- `unlim_choice` → `ServiceQuestion::UseAllowance`: nothing was submitted; CG-026's dialog;
  re-call with `use_unlim: true | false`. FreeCut never sets `use_unlim` without that answer.
- `adjustments` → `Vec<Adjustment>`: shown and recorded; the recipe keeps both requested and
  effective values.
- `recovery_tool` → `FollowUp`: executed only if it names an allow-listed free tool; a paid tool
  opens a new run sheet; anything else fails the run with the message. (The description tells
  chat agents to call it "immediately"; FreeCut is not a chat agent and keeps Q1.)

**Time at the edge.** Durations are `Tick` in FreeCut and become whole seconds only in
`map.rs`: `snap_duration(ticks, &DurationSpec) -> WholeSeconds` rounds to the nearest allowed
value with exact integer math (ties go up), and the UI shows the snapped value before Run.

---

## 7. Jobs and the ledger

### 7.1 Run state machine

```
Draft ─► Quoting ─► AwaitingConfirmation ─► QueuedHere ─► PreparingInputs ─► Uploading
   ▲          │               │                                  │              │
   └──────────┴─ Cancel ──────┘                         Stopped ◄─┴──────────────┤
                                                                                  ▼
             ┌────────────────────── Submitting ◄─── (re-quote if older than 5 min)
             ▼                ▼                ▼                   ▼
        NeedsAnswer     Failed(NotSent)   OutcomeUnknown       Running ──► Downloading ──► Ready
         (CG-026)                         (CG-010)               │  ▲          │            │
                                                                 ▼  │          ▼            ▼
                                                   Failed(service)  Stopped   Failed     Added
                                                   Expired (> 7 days)          (unsupported result)
```

`Submitting` also leads to `NotAccepted` (the service refused before running: `401`, session
`404`, concurrency limit; CG-010), which, like `Failed(NotSent)`, returns to
`AwaitingConfirmation` only through **Run again** with a fresh quote.

The machine is a pure `CloudJob::handle(Input) -> Vec<Output>` in `freecut-cloud`;
`Outcome unknown` can only be entered from `Submitting`, and no transition leads from any state
to `Submitting` without a `RunConfirmed` input from the UI for that attempt (a re-quote that
rose returns to `AwaitingConfirmation`; an unchanged or lower one keeps the earlier
confirmation).

### 7.2 The ledger

- File: `<app data>/FreeCut/cloud/ledger.json`, schema-versioned, written by the app with the
  atomic replace that project saves use (temp file, fsync, rename), from the cloud worker,
  never the UI thread.
- **Write points:** after confirmation (`QueuedHere`), **before** the submission is sent
  (`Submitting`), immediately when job ids arrive (`Running`), and on every later state change.
  A crash between the second and third write leaves `Submitting`, which the next start turns into
  `OutcomeUnknown`. If the write before submitting fails, nothing is sent (*Failed: not sent*).
- **One writer.** The worker holds an exclusive lock file (`ledger.lock`, OS advisory lock)
  while cloud generation is on. A second FreeCut process that cannot take it runs no worker, no
  poller and no submissions, and says so (CG-001); this keeps two pollers or two submitters from
  racing on one account.
- Entry: local run id (UUIDv4), provider id, account `subject_hash`, `ProjectId` and the last
  known project path, the `Operation` (prompt and settings), inputs as source references
  (media fingerprint, PM-038; source range in ticks; rendered or not) plus upload checksums and
  remote ids, quote, remote job ids with their variant index, state and timestamps, adjustments,
  downloaded files (path, size, SHA-256), and import status.
- Holds no tokens, no e-mail, no file contents. It may hold prompts, so it is listed in the
  diagnostic bundle (UI-063) as "excluded".
- Entries in a final state older than 90 days (FreeCut default) are pruned at start-up; entries
  still `Ready` (downloaded, not added) are kept until added or discarded.

### 7.3 Polling

One poller per provider on the cloud worker batches up to 12 running jobs per `jobs_wait` call
with `timeout_seconds: 15`, then waits the returned `poll_after_seconds` clamped to 2–60 s.
It runs while the app is open and cloud generation is on, regardless of which project is open.
On start-up, jobs younger than 7 days resume; older ones become `Expired` (outputs are kept
"at least seven days"), and **Check again** still tries. Jobs from another account are skipped.

### 7.4 Scheduling and limits

`Limits { max_concurrent_runs: 1..=4 (default 2), warn_above: Option<Credits> }`. A confirmed
run waits in `QueuedHere` until a slot frees, then is re-quoted if its quote is older than
5 minutes; a higher price returns it to `AwaitingConfirmation`. `Credits` is a fixed-point
`i64` of thousandths parsed from the service's decimal text; no floats.

---

## 8. Media legs

### 8.1 Preparing uploads

`UploadPlan` (pure, `freecut-cloud`) lists each input with its source, section, format, sound
flag and size estimate; the run sheet's Sends list (CG-004) is rendered from it, and the app's
jobs execute exactly it.

| Input | Prepared by | Format |
|---|---|---|
| Video section (CG-014) | `freecut-media` transcode job | H.264 High, long side ≤ 1920 px (or the source's), source frame rate, yuv420p, MP4 with `faststart`; AAC 48 kHz stereo only when sound is sent; HDR sources tone-mapped to SDR (CL-041) |
| Still from a library image | `freecut-media` | PNG (JPEG at quality 92 if the PNG would exceed 20 MB) |
| Frame of a clip | `freecut-media` decode at the frame | PNG |
| Current frame of the timeline | CPU compositor (`freecut_render::cpu::Compositor`, README C44) at canvas size | PNG |
| Audio section | `freecut-media` | WAV, 48 kHz, 16-bit |

- All container metadata is dropped (FFmpeg `map_metadata -1`, no creation time, no GPS, no
  encoder string beyond the muxer's default); file names are `freecut-input-<n>.<ext>`.
- Prepared files are PM-074 cache entries keyed by fingerprint + section + preset, so retries
  and variants reuse them, and the cache rules delete them.
- Exact section cuts: decode from the preceding keyframe and drop frames before the section's
  first tick (frame-accurate trim, no stream copy).

### 8.2 Uploading

1. `media_upload { files: [{ filename, content_type }] }` → presigned `upload_url`, headers and
   a media id per file (up to 20).
2. `PUT` each file to its URL on PM-073's I/O pool: the exact headers returned, **no**
   `Authorization`, HTTPS only, the body streamed from disk through a counting reader that
   reports progress to the task row.
3. Only when every `PUT` returned `200`: `media_confirm { type, media_ids }`.
4. Remote media ids are remembered per (account, fingerprint, section, preset) for 24 h
   (FreeCut default; the service's retention of uploads is not documented). A "media not
   found" answer drops the memory and uploads once more.
5. **Chaining:** a finished job id is accepted wherever a media id is, so follow-up actions on a
   result (Upscale, Use as reference) send the job id and upload nothing.

### 8.3 Downloading results

- Result URLs come from `jobs_wait`. They are fetched without credentials, HTTPS only, following
  at most 5 redirects that stay on HTTPS, with a size cap of 8 GB (FreeCut default).
- "60 s without progress" (§4.5) is approximated with ureq's phase timeouts: `timeout_recv_response`
  60 s and `timeout_recv_body` set from `Content-Length` at a floor of 256 KB/s plus 60 s (no
  length: 30 minutes); a timed-out download resumes with `Range`. The same holds for presigned
  `PUT`s with `timeout_send_body`. The same approach serves C49's downloader.
- Target: the project's generated-media store `<Name> Media/Generated/Cloud/` when the project
  folder is known and writable, otherwise `<app data>/FreeCut/cloud/inbox/<run id>/` (CG-013).
  Paths are computed by `freecut-project`.
- Written as `<name>.part`, SHA-256 computed while streaming, resumed with `Range` +
  `If-Range` when the server supports it, renamed into place when complete, then probed
  (PM-035). A file that does not probe as image, video or audio is *Failed: unsupported result*
  and stays in the inbox for inspection.
- Results are downloaded as soon as a job is terminal, because the service keeps them only for a
  limited time.

---

## 9. Provenance (PM-041)

Each result becomes a media item with:

```rust
MediaOrigin::Generated {
    kind: GeneratedMediaKind::Cloud,            // new variant (PM-041 lets producers add one)
    parent: Option<MediaId>,                    // the source item of a clip action
    source_range: Option<TimeRange>,            // the uploaded section, in source ticks
    recipe: Option<Arc<str>>,                   // canonical JSON text, below
    produced_by: Option<Arc<str>>,              // "higgsfield:seedance_2_5"
}
```

**Model change (H2, PM and TL owners sign off).** In code today
(`freecut-timeline/src/model/library.rs`) PM-041's kind enum is `GeneratedMediaKind`
(`Voiceover`, `Tts`; `GeneratedKind` is the clip-content enum) and `MediaOrigin::Generated`
carries only `kind`; `MediaOrigin` is `Copy`. H2 adds the `Cloud` variant and PM-041's
provenance fields, which makes `MediaOrigin` `Clone` but not `Copy`. PM-041's model note types
the recipe as `serde_json::Value`, but `freecut-timeline` has no serde dependency, so the
recipe is stored as JSON text (`Arc<str>`): `freecut-cloud` builds and parses it,
`freecut-project` persists it as a string, and the timeline only carries it. The project format
takes the next free version with an additive migration (C54 rule 2).

Recipe (built by a pure function in `freecut-cloud`, versioned):

```json
{
  "v": 1,
  "provider": "higgsfield",
  "service": "https://mcp.higgsfield.ai/mcp",
  "tool": "generate_video",
  "model": "seedance_2_5",
  "operation": "extend_forward",
  "prompt": "the wave breaks and the camera rises",
  "requested": { "duration": 5, "resolution": "720p" },
  "effective": { "duration": 5, "resolution": "720p", "mode": "video_extension", "extension_mode": "forward" },
  "adjustments": [],
  "inputs": [
    { "role": "video_references", "media": "media:12", "fingerprint": "…",
      "range_ticks": [7056000000, 10584000000], "rendered": false,
      "upload": { "preset": "h264-1080-nosound", "sha256": "…" } }
  ],
  "run": "4f1c…", "job": "9a7e…", "variant": 0,
  "cost": { "credits": "24.000", "preflight": true },
  "sha256": "…", "created": "2026-10-13T14:02:11Z", "freecut": "0.1.0"
}
```

The recipe holds no file paths, no account data and no URLs with signatures. *Generate again*
(CG-011) builds a new `Operation` from it and goes through the run sheet. `parent` is set for
single-source clip actions; multi-reference generations list their inputs in the recipe only.

---

## 10. Errors

The classifier is a pure function `classify(&Transport, &JsonRpcError | &ToolError) ->
CloudError`, tested against recorded and hand-written responses. The MCP server's structured
error codes are not documented, so matching uses structured fields first, then known message
patterns, then falls back to `ServiceError` with the message.

| Signal | `CloudError` | User text (FreeCut's) | Run state | Retry |
|---|---|---|---|---|
| DNS, connect or TLS failure before sending | `Offline` | "Can't reach Higgsfield. Check your connection." | read-only: waits; paid: *Failed: not sent* | read-only automatic; paid by the user |
| Timeout or reset after sending | `NoAnswer` | "Sent, but no answer from Higgsfield." | paid: *Outcome unknown* | never automatic |
| `401` / `invalid_token` | `SignInExpired` (after one refresh) | "Your Higgsfield sign-in expired. Sign in again." | polling paused | after sign-in |
| `403` / `insufficient_scope` | `NotPermitted` | "Higgsfield did not allow this for FreeCut's sign-in." | *Failed* | by the user |
| Not enough credits | `InsufficientCredits` | "Not enough credits on your Higgsfield plan." + *Open Higgsfield* (system browser) | *Failed* | by the user |
| No paid plan | `PlanRequired` | "This connection needs a paid Higgsfield plan." | *Failed* | — |
| Content refused (moderation, `nsfw`) | `ContentRefused` | "Higgsfield declined this request under its content rules." | *Failed* | by the user, after changes |
| Model blocked or unavailable | `ModelUnavailable` | "This model is not available right now." | *Failed* | by the user |
| Concurrency or rate limit | `Busy` | "Higgsfield is busy for your account. Try again in a moment." | read-only: back-off; paid: *Failed: not accepted* (the service refused it) | read-only automatic |
| Argument validation | `InvalidRequest` | the service's message, quoted | *Failed* | — (FreeCut's own validation should prevent it) |
| Tool missing | `NotOffered` | "Not offered by the service right now." | action disabled | after Refresh |
| `401` or session `404` on a paid submission | `NotAccepted` | "Higgsfield did not accept the run (sign-in or session renewed). Press Run again." | *Not accepted* (CG-010) | by the user, on a fresh sheet |
| Unexpected response shape | `Protocol` | "Unexpected answer from Higgsfield." + details in the log | free calls: *Failed*; a paid submission: **always** *Outcome unknown* | — |
| JSON-RPC error on a paid submission | classified as above (credits, content, …) | the classified text | *Rejected* (*Failed*) only for codes that prove the method never ran: -32700, -32600, -32601, -32602; any other code (-32603, -32000..-32099, …) is *Outcome unknown*, reason kept (review M2) | by the user |
| Anything else | `ServiceError { message }` | the message, quoted | *Failed*. On a paid submission: a 4xx answer or a tool error result is *Rejected* (a tool error that names job ids is *Outcome unknown*); a 5xx is *Outcome unknown* | by the user |

Whether credits were charged for a failed run is reported only when the service says so.
Messages from the service are shown as plain quoted text; no links in them are made clickable.

---

## 11. Privacy and safety

- Nothing is contacted while cloud generation is off. When on, requests happen only on a user
  action (Connect, Test, Refresh models, opening the Generate tab with a catalogue older than
  24 h, opening a run sheet or changing its settings (free cost check), Check cost, Run, Check
  again, Show balance, opening the voice picker) or for runs the user started (polling,
  downloads, session `DELETE` on quit). No telemetry, no update checks, no analytics field.
- Disclosure (CG-004) is generated from `UploadPlan`; a test asserts that the arguments and
  uploads of a run contain nothing the plan did not list (no `/` path separators, no original
  file names, no metadata atoms in the prepared files).
- Server text (tool descriptions, `instructions`, error messages) is data. It is never shown as
  instructions and never forwarded to the assistant's model as instructions (CG-024 sends only
  FreeCut-built summaries).
- The loopback listener binds `127.0.0.1` only, accepts one request with the right `state`, and
  closes.
- Logs at `info`: run ids, tool names, timings, states, error classes. Request and response
  bodies only at `trace` behind *Log cloud requests* (off), with secrets redacted and prompts
  shown only in that mode. Those trace lines go to a separate `cloud-trace.log` that crash
  reports (UI-023) and diagnostic bundles (UI-063) never include; the switch turns itself off
  when FreeCut quits.

---

## 12. Assistant integration (CG-024)

- `freecut-assistant` gains three tool specs: `list_generation_models` (query; the app answers
  from the cached catalogue), `generate_media` and `ai_edit_clip` (actions, `Output::Action` as
  in llm-editor.md §3.5). Their arguments are FreeCut's `Operation` fields in the assistant's
  conventions (ids `clip:41`, time strings, AS-006).
- The app turns an action into the CG-005 run sheet with the prompt editable. The assistant
  receives `confirmed`, `cancelled`, `outcome_unknown`, `failed: <class>` or, later,
  `ready: media:<id>`; placement after a result is a normal proposal (AS-007, AS-008).
- No path exists for the assistant to submit, answer CG-026's questions, tick consent, change
  limits or read the ledger. AS-018 gains "at most 4 run sheets per turn".
- The assistant crate does not depend on `freecut-cloud`. If a later feature needs MCP
  elsewhere (llm-editor.md §10 item 6 considers exposing FreeCut itself as an MCP server; a
  client for other servers is not planned), the `mcp` module moves into its own small crate
  then; today only cloud uses it.
- Consistency edits made with this design: AS-013 and AS-019 name CG-024's run-sheet actions as
  the one route by which an assistant request can lead to a service contact (on the user's Run
  click); AS-018 and llm-editor.md §4.9 gain "run sheets per turn: 4"; llm-editor.md §3.5,
  §6.1 and §6.2 list the two actions and the run sheet as their confirmation.

---

## 13. Testing

| Layer | What | How |
|---|---|---|
| Pure units | SSE parser (split chunks, multi-line data, comments); JSON-RPC framing; catalogue parsing and filters; `snap_duration`; `Operation` → arguments (golden JSON per operation); error classifier; PKCE and authorize URL; recipe builder; `UploadPlan` → disclosure; `CloudJob` transitions including crash points; ledger DTO migrations; `source_section` in `freecut-timeline` (limits, in/out marks, speed and reverse ignored, frame rounding) | `cargo test -p freecut-cloud` with no `http` feature, and `-p freecut-timeline` |
| Fake MCP server | `FakeMcp` implements the `HttpTransport` trait in memory: scripted JSON-RPC exchanges, session ids, `404` session expiry, SSE responses, `401` then refresh, timeouts before and after the body is written | sans network |
| Unknown-outcome property | For every scripted failure point of a paid submission (connect errors, each ureq timeout phase, reset after write, `401`, session `404`, concurrency refusal, unreadable answer, ledger write failure, crash after each write point), at most one submission is observed by the fake server, and the run ends in `NotSent`, `NotAccepted`, `OutcomeUnknown` or `Rejected`, never resubmitted | property test over the failure points |
| Preflight guard | quote JSON always carries `get_cost: true`; no quote when the live schema lacks it; a server that ignores the flag yields one submission per Check cost and the "started run" path; preflights not retried after `MaybeSent` | units + fake server (§4.7) |
| Send-state table | every `ureq::Error` variant maps to `NotSent` or `MaybeSent` as §4.5 lists | exhaustive match test |
| Loopback integration | presigned `PUT` with progress and retry; download with `Range` resume and checksum; OAuth loopback callback with right and wrong `state` | `std::net::TcpListener` on `127.0.0.1:0` (feature `http`) |
| Recorded fixtures | `initialize`, `tools/list`, `models_explore` listings, `get_cost` answers (all free), then one cheapest image run and one shortest video run of each operation as it is built (paid; the owner records them with their own account and explicit consent) | `crates/freecut-cloud/tests/fixtures/higgsfield/*.jsonl`, produced by `cargo run -p freecut-cloud --features http --example record`; a scrubber removes tokens, e-mails, subject ids, signed URL queries and job ids are replaced by stable fakes; reviewed before commit |
| Contract check (manual, not CI) | compares live `tools/list` schemas and the catalogue with the fixtures and reports drift | `--example check-contract`, free calls only |
| App | Generate tab, run sheet, Runs page and placement driven by `ScriptedProvider`; import and placement are one undo entry (apply → undo → redo test) | `freecut-app` tests |
| Guard | dependency edges; no networking crate in `cargo tree -p freecut-cloud` without `http` | workspace guard test |

CI runs no network: fixtures and loopback only.

---

## 14. Build slices

Each slice leaves FreeCut buildable, tested and usable; fmt, clippy and tests pass on all three
platforms. Slice → milestone mapping: H1 = M-CG1, H2 = M-CG2, H3 = M-CG3, H4 = M-CG4 (features
README).

### H1 — Client, account, catalogue and cost preflight

- **Gate:** the owner approves §2.4's CLAUDE.md lines (or chooses option (c)) and confirms the
  second *Local only* exception (§15 item 1).
- **Crates:** `freecut-cloud` (new: `mcp`, `oauth`, `secret`, `catalogue`, `provider`,
  `provider/higgsfield` for `models_explore` and `get_cost`, `error`); `freecut-app`
  (`cloud` feature, Preferences ▸ Cloud generation, keychain `SecretStore`, browser launch,
  loopback listener, cloud worker, a Generate tab with the Image and Video forms and **Check
  cost** but no Run).
- **Contracts:** `McpClient::{initialize, list_tools, call_tool}`, `HttpTransport` trait,
  `Credential`, `SecretStore`, `GenerationProvider::{connect, catalogue, quote}`, `Catalogue`,
  `ModelInfo`, `Operation::{Image, Video}`, `Quote`, `CloudCmd`/`CloudEvent`, `CloudError`.
- **Tests:** SSE and JSON-RPC units; OAuth URL and PKCE; fake-server handshake, session expiry
  and refresh; catalogue parse from the recorded listing; operation → `get_cost` golden JSON;
  loopback sign-in callback; "no request while off" (a counting transport sees zero calls with
  the switch off); preflight guard (§4.7); send-state table; keychain split and reassembly of
  oversized tokens (in-memory store with a 2,560-byte entry limit); dependency guard.
- **Acceptance:** with the switch on, a user connects through the browser (or pastes a token),
  Test connection shows the model counts, the Generate tab lists models filtered by type, the
  aspect ratio defaults to the project canvas, and Check cost shows the service's estimate for
  the current settings without submitting anything; Disconnect removes the keychain entry; CI is
  green offline. H1 also confirms the transport, the DCR redirect rules and device code (§15)
  and records the free fixtures.

### H2 — Generate tab: images and video, jobs, import

- **Crates:** `freecut-cloud` (submit, upload slots, `jobs_wait` poller, `CloudJob`, ledger DTOs,
  recipe, limits, service questions); `freecut-media` (still and frame to PNG, image transcode);
  `freecut-render` (none new: the CPU compositor renders *Current frame*); `freecut-project`
  (`Generated/Cloud/` and inbox paths; the recipe and provenance fields in the project format,
  next free format version); `freecut-timeline` (`GeneratedMediaKind::Cloud` and PM-041's
  provenance fields on `MediaOrigin::Generated`, §9; the compound "Add & place generated media"
  built from existing requests); `freecut-app` (run sheet,
  Runs page, `TaskOwner::Cloud` rows, ledger file, CG-013 bar, CG-026 dialog).
- **Features:** CG-004 to CG-013, CG-025, CG-026.
- **Contracts:** `GenerationProvider::{prepare_upload, confirm_upload, submit, poll}`,
  `SubmitOutcome`, `SubmitError`, `LedgerEntry` v1, `UploadPlan`, recipe v1,
  `TaskOwner::Cloud`.
- **Tests:** unknown-outcome property test; ledger crash points (kill after each write point,
  restart, expected state); poller batching and `poll_after_seconds` clamping; upload
  confirm-only-after-all-200; download resume and checksum; recipe golden JSON; import +
  placement apply → undo → redo; Sends list equals the prepared uploads.
- **Acceptance:** a user generates two image variants from a prompt, edits a library image with a
  prompt, makes a 5 s video from text in the project's aspect ratio and one from the current
  frame; each run shows its cost and what is sent and starts only on Run; results land in the
  Generated bin with *How it was made*; Place puts one on the timeline and Undo removes it (the
  file stays); quitting during a run and restarting resumes and imports it; a submission that
  times out after sending shows *Outcome unknown* and is never sent again on its own.

### H3 — AI edit actions on clips

- **Crates:** `freecut-timeline` (pure `source_section`); `freecut-media` (frame-accurate
  section transcode with metadata stripped, WAV sections); `freecut-cloud` (operation mappings
  per tool, consent flags); `freecut-app` (AI edit submenu and inspector section, section strip
  in the sheet, *When ready* placements).
- **Order inside H3:**
  - **H3a (v1):** CG-014, CG-015, CG-016. One recorded run of the extension mode settles whether
    the service returns the context section (CG-016).
  - **H3b (later):** CG-017, CG-018, CG-019, each shown only after a recorded run confirms its
    output format (alpha video or matte; upscale containers) and a free `get_cost` call
    confirms that its catalogue model gives an estimate (§6.3).
  - **H3c (later):** CG-021, CG-022, CG-023, after the owners' decision on ai-tools.md's
    *Out of scope* (§15 item 4); CG-021 and CG-023 also need a cost estimate or the owners'
    acceptance of "cost shown afterwards" (§15 item 13).
  - **H3d (later):** CG-020, blocked until HDR is reopened.
- **Contracts:** `SourceSection { media, range, limited_by }`, `Operation::{RemoveBackground,
  Upscale, Reframe, SdrToHdr, Dub, LipSync, VoiceChange}`, `Video { mode: EditByPrompt |
  Extend }`.
- **Tests:** `source_section` table tests (long clip limited to the model maximum around the
  playhead; marks; reversed and sped-up clips use source time; too-short clips disabled);
  prepared files have no metadata and exact frame counts; extension placement on the magnetic
  main track (ripple) and on overlay tracks (new track when occupied); Replace follows TL-035
  for shorter and longer results.
- **Acceptance (H3a):** a user right-clicks a 40 s clip, chooses *Edit with a prompt*, sees the
  30 s section around the playhead and what will be sent, runs it, and gets the edited section
  above the original; *Extend forward* adds 5 s after a main-track clip and ripples the rest; both
  undo in one step.

### H4 — Assistant tools

- **Prerequisites:** M-AI3 (action tools, AS-013), H2, H3a.
- **Crates:** `freecut-assistant` (tool specs and argument DTOs; AS-018 limit); `freecut-app`
  (routing actions to the run sheet, results back to the session).
- **Feature:** CG-024.
- **Tests:** a scripted model that calls `generate_media` ten times in a turn produces at most 4
  sheet requests and zero submissions without a Run click; cancelled sheets return `cancelled`;
  the assistant's provider request contains no upload, URL or token (policy test, llm-editor.md
  §5.6).
- **Acceptance:** "make a 5-second shot of a beach at sunset and put it before the first clip"
  opens a run sheet with the assistant's prompt, cost and Sends list; after Run and completion,
  the placement appears as a proposal and Accept adds it as one undo entry.

---

## 15. Risks and open questions

1. **Second exception to *Local only*** (features README conventions, ai-tools.md *Out of
   scope*, the *Deferred by decision* list). The owners confirm or reject cloud generation as a
   second opt-in exception before H1.
2. **CLAUDE.md** gains `freecut-cloud` only with the owner's approval of §2.4 (or option (c)).
3. **Third-party client permission.** Nothing public allows or forbids a third-party desktop app
   on a user's own grant; the server advertises dynamic registration, which suggests arbitrary
   MCP clients are expected, and its auth hints name specific clients. Ask Higgsfield before
   release: client registration for a native app with loopback redirects, naming, and whether the
   REST key pair may be stored by a desktop app.
4. **Dub, lip sync and voice change** conflict with ai-tools.md's *Out of scope* (written for
   local models and impersonation risk). The AI owner decides whether the cloud versions, with
   the consent tick and preset voices only, are acceptable.
5. **Unconfirmed protocol details:** the transport, session lifetime, token lifetimes, whether
   DCR accepts loopback redirects, the device-code host, and the exact JSON of submissions,
   `get_cost`, `jobs_wait`, `unlim_choice`, `adjustments` and `recovery_tool`. H1 and H2 record
   them; the mapping is built against recordings, not guesses.
6. **Output formats** of background removal (alpha or matte), dubbing, voice change, upscale and
   the extension mode (whether the context section is included). Each action ships only after a
   recorded run.
7. **No server-side cancel** for submitted jobs; credits may be spent after Stop waiting. The UI
   says so; nothing more can be done client-side.
8. **Upload size limits** are undocumented for `media_upload`; sections are capped by model
   limits and the 1920 px preset, and a service refusal is shown as "too large".
9. **Terms section numbers** are from a summarised reading; re-check the live page and quote
   nothing until then.
10. **Client ID metadata document** (if DCR fails) needs a static HTTPS file on FreeCut's
    website: the owner decides whether that is acceptable.
11. **Where the per-project "never use cloud generation" switch lives** (preferences or the
    project file), the same question as AS-004's.
12. **Catalogue drift.** Model ids churn (several generations of the same family are live at
    once); defaults are re-checked each release, and saved recipes name models that may vanish.
13. **Actions without a cost estimate.** `dubbing` and `voice_change` list no `get_cost`.
    Either the service adds one, or the owners accept a named exception where the sheet says
    "cost shown only afterwards" and needs a tick; otherwise CG-021 and CG-023 are not built.
    (CG-017 and CG-018 avoid the problem through catalogue models, §6.3.)
14. **Preflight that spends.** The free cost check and the paid run share one tool; §4.7's guard
    makes a misbehaving flag cost at most one run per Check cost and tells the user. Confirm in
    H1 with recorded free calls that `get_cost` answers carry no job ids.
15. **Free-trial allowance on the MCP.** The help center says unlimited plans and free
    generations apply on higgsfield.ai only; the tool descriptions describe `use_unlim` and
    `unlim_choice`, and the catalogue reports an allowance block per model. CG-026 is built
    against a recorded answer only.
16. **Higgsfield's provenance marks.** Outputs may carry watermarks or metadata (terms §6.4).
    FreeCut keeps the downloaded file unchanged, but export re-encodes and can drop file-level
    metadata. Ask Higgsfield whether that conflicts with its terms; FreeCut does not remove
    visible watermarks.
17. **Developer obligations.** Whether an open-source client acting only on each user's own
    grant is a "developer" under terms §11 (privacy notices, end-user responsibility) is
    unclear; CG-004's notice is written as if it were. Part of the question in item 3.
18. **Blocking HTTP and cancellation.** `ureq` 3 has no per-read idle timeout; §4.4 abandons
    blocked calls instead. llm-editor.md §2 rule 2 relies on a 1 s socket read timeout with the
    same crate and should adopt the same approach (assistant owner).
19. **Shipping default.** The design keeps the runtime switch off by default and the `cloud`
    build feature on. The owners decide whether release builds include the feature at all in
    the first release that has it (for example, on only after H2's acceptance and Higgsfield's
    answer to item 3).

### 15.1 Confirmed with free calls (H1, 2026-10-14)

Only discovery, one client registration, `models_explore` and `get_cost: true` were called.
Recordings: `crates/freecut-cloud/tests/fixtures/higgsfield/` (its README lists each source).

| Question (item 5) | Finding |
|---|---|
| Authorization server | **Changed since 2026-10-13.** The protected-resource metadata now lists `https://clerk.higgsfield.ai` and `https://fnf-device-auth.higgsfield.ai`, not `mcp.higgsfield.ai` (whose own metadata is still served but no longer referenced). Clerk's metadata: `/oauth/authorize`, `/oauth/token`, `/oauth/token/revoke`, `/oauth/device_authorization`, `/oauth/register`, PKCE S256, `authorization_response_iss_parameter_supported: true`. §5.1's origin check therefore becomes: every sign-in endpoint is HTTPS on `higgsfield.ai` or a subdomain, the resource equals the MCP endpoint, and the issuer equals the server it was fetched for. The callback's `iss` is checked (RFC 9207). A non-standard `higgsfield_auth_hints` block names example clients; FreeCut ignores it. |
| DCR with a loopback redirect | **Accepted.** §5.2's request → `201`, public client (`token_endpoint_auth_method: none`), redirect `http://127.0.0.1/callback` echoed. `client_id_issued_at` is in milliseconds (RFC 7591 says seconds); FreeCut does not read it. One FreeCut registration now exists at Higgsfield's Clerk; its id is in no file. No client-metadata document is needed (item 10 closes). |
| Loopback port | The authorize endpoint accepts `http://127.0.0.1:<any port>/callback` for that registration and continues to Higgsfield's sign-in page; an unregistered host gets `400 invalid_request`. |
| Device code | Clerk advertises `device_authorization_endpoint`; `fnf-device-auth.higgsfield.ai` has no discovery document (404). Not exercised. FreeCut now requests the device grant at registration when it is advertised, falling back to a browser-only registration if refused (§15.2). |
| Transport | Unauthenticated `POST`/`GET /mcp` → `401`, JSON body, `WWW-Authenticate: Bearer resource_metadata=…/oauth-protected-resource/mcp, scope="openid email offline_access"`. The authenticated framing (JSON or SSE answers, `Mcp-Session-Id`, negotiated revision) is **not yet observed**; the client handles all of them. |
| `models_explore` | `{items, has_more, unlim: {available, remaining, expires_at}}`. Items: `id, name, provider_name, description, output_type, parameters[], medias[{name, type, max?, required?, roles[]}], aspect_ratios, tags, supports_unlim?` and durations as a top-level `durations` set, a top-level `duration_range`, or a `duration` parameter (`options` or `min`/`max`, one model with `min: -1`, one without `max`). Parameters: `required: "optional" | "required"`, types `string`, `number`, `bool`, `string_array`. One media group has no `roles`. Counts: 45 image, 57 video, 8 audio, each `has_more: false`; pages use `after` with `next_page_token`. New actions `search` and `recommend`, output type `3d`. |
| `get_cost` answers | `{"cost": {"credits": 0.25, "credits_exact": 0.25}}` (image) and `{"cost": {"credits": 35, "credits_exact": 35}}` (video): JSON numbers, **no job ids** (item 14). The image check with `count: 2` answered the same figure as `count: 1`, so an estimate is taken to cover one variant and CG-005's rule limits Variants to 1. `get_cost` sits inside `params`, and `params` is `anyOf` object or JSON string. New optional fields `use_free_gens` and `preset_session_id` exist; FreeCut never sends them. |
| Errors | An unknown model answers the tool error `Invalid request: unknown model "…". Use models_explore…` (classified as model unavailable). |

**Still unknown:** the authenticated transport framing; session and token lifetimes
(`expires_in`), refresh-token rotation and revocation behaviour; whether Clerk's tokens carry the
`resource` audience the MCP server expects; whether Clerk sends `iss` on the redirect as it
advertises; device sign-in; the shapes of `unlim_choice`, `adjustments` and `recovery_tool`. The
first need one signed-in FreeCut session (free calls, the owner signs in); the last need runs.

### 15.2 Implementation notes (H1 core, `freecut-cloud`)

Deviations from the sections above, all **accepted by owner** (2026-10-14):

- **Server requests (§4.1).** `ping` gets the empty result MCP requires; every other server
  request gets `-32601`. *Accepted by owner.*
- **Keychain entries (§5.6).** Each write uses a new generation and replaces `meta` last, which
  is what makes "old or new set, never a mix" hold. *Accepted by owner.* Parts are named
  `<provider>/access.<slot>.<n>` with slot = generation mod 2, so there are only two sets to
  clear: a write clears the other slot, a delete clears both, and a damaged `meta` is swept the
  same way; an unreadable store is an error, never "nothing stored" (review SEC-2). Writes are
  exclusive (SEC-1).
- **Contracts (§6.1).** A provider is built with a `TokenSource` (the `TokenManager` supplies
  tokens), so `connect` takes no `Credential`; `quote` takes the `Operation` alone (inputs are
  `RoleRef`s with service media ids). `QuoteOutcome` gains `StartedRun` (§4.7 rule 4).
  *Accepted by owner.*
- **Cost checks after a refusal (§4.5, §4.7).** A cost check answered `401` or session `404` is
  not resent: the sign-in or session is renewed and the user presses Check cost again.
  *Accepted by owner.*

Other notes:

- **Send-state table (§4.5).** `ureq::Error` is `#[non_exhaustive]`, so no match can make a new
  variant fail the build. Unlisted variants fall to `MaybeSent` (the safe side), `ureq` is pinned
  `=3.4.2`, and a test lists every 3.4.2 variant.
- **TLS.** `ureq` is built with `rustls-no-provider` and `platform-verifier`; the transport hands
  it ring as the crypto provider (`rustls` is a direct optional dependency for that and for its
  certificate errors). Certificates come only from the platform verifier, so `webpki-roots`
  (bundled roots, CDLA-Permissive-2.0) is not a dependency. `webpki-root-certs` (also
  CDLA-Permissive-2.0) stays in `Cargo.lock` only as `rustls-platform-verifier`'s wasm32
  dependency; no FreeCut target builds it.
- **A failed handshake before a call** reports that call as not sent; a session `404` on a paid
  call or preflight is *not accepted* even if opening the new session then fails. A `404` to
  `initialize` or `notifications/initialized` ends the handshake: it never starts another one
  (review R1; before the fix it recursed until the stack overflowed).
- **The call class comes from the allow list (review R2).** `McpClient::call_tool` takes an
  `&'static AllowedTool`, which only `freecut-cloud`'s allow list can create, and reads the
  class and timeouts from it; an unlisted tool cannot be named. Cost checks go only through
  `McpClient::check_cost(tool, &QuoteRequest)`, for tools that offer one. The provider does not
  expose its MCP client (only `server_info()` for the debug view). §14's H1 contract
  `McpClient::{initialize, list_tools, call_tool}` gains `check_cost`.
- **Cost checks fail closed (§4.7, review M1).** A check whose answer is lost, unreadable or of
  an unknown shape (anything but `cost`, `unlim_choice` or `recovery_tool`) is
  `QuoteOutcome::PossiblyStarted`: that tool's checks stop until restart, and the caller records
  it like a run with an unknown outcome. "Lost" uses the paid-call rule: whatever
  `submit_disposition` would call *Outcome unknown*. Refusals stay errors. UUIDs in an answer
  still give `StartedRun` with those ids for the ledger.
- **Cancelling paid calls (§4.4, review M3).** A paid request gets an agent of its own (fresh
  connection) whose connector chain ends in a cancel gate: after TCP and TLS, right before the
  request is written, the gate checks the token, and a cancel there is *not sent*. Paid calls and
  cost checks also carry a late sink: if the caller's token fired before it had the whole answer,
  the helper thread reads the answer to its end and the MCP client queues it as a
  `LateToolResult` with every UUID in it (`McpClient::take_late_results`); the cloud worker
  drains the queue into the ledger after a cancel. The queue is in memory: until the ledger
  exists (H2), a late answer is kept only while FreeCut runs.
- **Refreshed tokens the keychain refuses (§5.6, review SEC-3)** are kept in memory and used:
  the stored refresh token may already be dead after rotation and is never sent again. The
  write is retried before the next refresh; a disconnect or a new sign-in drops them.
- **Stream resumption (§4.1, review F4).** Implemented rather than offering only 2025-06-18:
  `GET` + `Last-Event-ID`, the server's `retry`, at most 5 resumes, the same late sink as the
  request, the answer's own session id during `initialize`.
- **Device sign-in (§5.2, §5.4).** Registration asks for the device grant only when the server
  advertises a device endpoint and grant; if that registration is refused, FreeCut registers
  again without it (browser sign-in stays the main path) and reports whether *Use a code
  instead* is available.
- **Pure helpers always built.** PKCE (`sha2`, `base64`, `getrandom`) and URL handling (`url`) are
  not behind `http`; only `ureq` and `rustls` are.
- **Catalogue hiding** adds `dubbing`, `voice_change` and `sync_so` (listed as video models; the
  voice tools are not built), and, until the owner reviews them, `ad_multiplier*`, `hf_mult_*`,
  `3d_jutsu_*`, `cinematic_studio_soul_*` and `image_auto` (it routes each prompt to a model of
  the service's choosing, so model and price are not known up front). On the 2026-10-14
  catalogue this leaves 31 image, 45 video and 5 audio models.
- **Next.** Phase B starts with `cargo run -p freecut-cloud --features http --example record`
  (§13): the owner signs in once and it records the transport facts still open in §15.1, free
  calls only.

---

## Sources

- Model Context Protocol specification: transports, lifecycle, tools, cancellation, authorization — https://modelcontextprotocol.io/specification
- RFC 9728 OAuth 2.0 Protected Resource Metadata — https://www.rfc-editor.org/rfc/rfc9728
- RFC 8414 OAuth 2.0 Authorization Server Metadata — https://www.rfc-editor.org/rfc/rfc8414
- RFC 7591 OAuth 2.0 Dynamic Client Registration — https://www.rfc-editor.org/rfc/rfc7591
- RFC 7636 PKCE — https://www.rfc-editor.org/rfc/rfc7636
- RFC 8252 OAuth 2.0 for Native Apps — https://www.rfc-editor.org/rfc/rfc8252
- RFC 8628 OAuth 2.0 Device Authorization Grant — https://www.rfc-editor.org/rfc/rfc8628
- RFC 8707 Resource Indicators for OAuth 2.0 — https://www.rfc-editor.org/rfc/rfc8707
- Higgsfield MCP protected-resource and authorization-server metadata — https://mcp.higgsfield.ai/.well-known/oauth-protected-resource ; https://mcp.higgsfield.ai/.well-known/oauth-authorization-server
- Higgsfield help center: What is Higgsfield MCP; How do I connect Higgsfield to an AI agent; Official Higgsfield platforms; What is the Higgsfield API — https://higgsfield.ai/creator-hub/help-center/integrations/what-is-higgsfield-mcp ; https://higgsfield.ai/creator-hub/help-center/integrations/how-do-i-connect-higgsfield-to-ai-agent ; https://higgsfield.ai/creator-hub/help-center/getting-started/official-higgsfield-platforms ; https://higgsfield.ai/creator-hub/help-center/integrations/what-is-the-higgsfield-api
- Higgsfield terms of use and privacy policy — https://higgsfield.ai/terms-of-use-agreement ; https://higgsfield.ai/privacy-policy
- Higgsfield API docs: authentication, requests, polling, errors, billing and retention, file uploads — https://docs.higgsfield.ai/docs/authentication.md ; https://docs.higgsfield.ai/docs/concepts/requests.md ; https://docs.higgsfield.ai/docs/concepts/polling.md ; https://docs.higgsfield.ai/docs/concepts/errors.md ; https://docs.higgsfield.ai/docs/concepts/billing-and-retention.md ; https://docs.higgsfield.ai/docs/concepts/file-uploads.md
- Higgsfield MCP tool schemas and `models_explore` listings of the user's own connected server, read 2026-10-13 (no paid or private calls)
- `keyring`, `ureq` and `sha2` documentation on docs.rs (licences to confirm at adoption)
