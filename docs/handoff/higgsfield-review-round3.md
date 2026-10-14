# Review round 3: feature/higgsfield, round-2 fixes and the record tool (`freecut-cloud`)

- Reviewed commits:
  - R3 450ff0d, R4 b95ce32 and b358f99, R5 b23b5a3, R6 28b7189, R7 2a055e0, R8 3c2db56,
    R9 911d75b.
  - Reviewed models 5373f03 (the owner confirmed the list in 50038bf).
  - Test-support guard 7cbf81f.
  - `--example record` b5f315d.
  - Base after the rebase: 339c149 on main.
- The branch kept moving during the review. The record follow-ups d413faa, f185baf, f4d3f36,
  a664030, c0d46d3 and cbf9a2f were checked too. Apart from `examples/record.rs`, docs and
  tests, the code under review did not change after b5f315d. Line numbers below are valid at
  cbf9a2f.
- Previous rounds: `higgsfield-review-phase-a.md` and `higgsfield-review-round2.md` in this
  folder.
- Method:
  - Four review lanes: MCP, SSE and late job ids; guard and keychain; CI, docs, models and
    the deps guard; the record tool.
  - A second pass then tried to refute every open finding. Only findings that survived are
    listed below, with the severity from that pass.
  - Everything ran offline. There was no paid call, no cost check, no network, no
    `cargo run` and no run of `record`.
- Checks run (offline, the worktree's own target dir):
  - `cargo test -p freecut-cloud --features http` at cbf9a2f: 112 unit, 12
    catalogue_and_golden, 7 loopback_http, 41 mcp_fake, 21 oauth_flow, 8 record example tests
    and 1 doctest. All pass. It was also green at b5f315d and at the record commits in
    between.
  - `cargo clippy -p freecut-cloud --all-targets --features http -- -D warnings`: clean.
  - The R6 and R7 race tests passed 5 runs in a row.
  - Not run in this round: the workspace fmt, clippy and test, and
    `cargo test -p freecut-app --test deps_guard`. They were outside this review's allowed
    commands. The guard's own `cargo tree` command was run by hand, read-only.
  - The untracked recording `tests/fixtures/higgsfield/recorded/free-calls.jsonl` was scanned
    by scripts that print counts and names only, never values.

## Verdict: approve with changes

R3, R4, R6, R7 and R8 are fixed and tested:

- The SSE parser commits an event id only when the event is dispatched.
- Answers the caller gives up on keep their job ids.
- The preflight guard is atomic, fails closed, and is owned by the app.
- A keychain that locks during a refresh no longer brings back the old token.
- CI fails on any Windows clippy error.

The reviewed model list denies by default, and the owner confirmed it. Over the full
recording, exactly the 12 reviewed models are visible.

**The record tool is safe for the owner to run on macOS.**

- It sent only free calls: 2 discovery GETs, register, token, initialize,
  `notifications/initialized`, `tools/list` and 3 `models_explore` list calls.
- `get_cost` never appears in a request.
- Headers are now an allow list.
- The current recording holds no tokens, cookies, e-mails or real ids.
- The first recording did keep Set-Cookie and Cloudflare and Clerk headers. d413faa and
  f4d3f36 fixed that.

I found no path that retries or resends a paid call. What is left:

- **One major: the late queue.** Late job ids go into a queue that each `McpClient` owns. The
  provider hides that client, so nothing outside a test can drain the queue, and the queue
  dies with the provider. R9's "drain on every tick" cannot be met. This must close before the
  H2 worker is wired.
- **Job-id scan gaps (minor).** The one scan misses some places and some id shapes. It also
  counts the request's own media ids as jobs. A late result cannot be tied to its run.
- **Token lifetime (minor).** R7 holds only while one TokenManager lives. Refused writes wait
  for the next refresh.
- **Guards and tests (minor).** The test-support guard misses features turned on through the
  app's own features table. No provider-level test proves that an unreviewed model is
  refused.
- **Record tool (minor).** It has scrubber gaps, no final secret check, over-redacted schemas
  and a loose refusal gate. None of these leaked anything into the current file.

Phase B may start once R10-R27 are in. The nits can follow.

## Item table

| Id | Topic | Verdict | Left over (details below) |
|---|---|---|---|
| R3 | SSE id counts only once its event is dispatched | fixed | Optional: add `SseResumable` and `SseCutInsideAnswer` to the M4 proptest. A cut over real chunked HTTP is R16 |
| R4 | abandoned answers keep their job ids | fixed | Optional: `the_first_verdict_stands` also passes without b358f99's `run()` change, so add a `run()`-level test. The JSON path drops partial bytes before the scan (in R12) |
| R5 | one job-id scan; a refusal that names a job is never Rejected | partially fixed | R12 (minor) |
| R6 | atomic preflight guard owned by the app | fixed | - |
| R7 | keychain lock during a refresh | fixed | R17, R18 (minor, both about lifetime) |
| R8 | CI runs every step with bash; cloud step split | fixed | - |
| R9 | Phase B obligations as H2 acceptance criteria | partially fixed | R11 (minor). The code side is R10 (major) |
| MODELS | reviewed allow list of image and video models | fixed | R19 (minor), N1 (nit) |
| TSGUARD | the app's normal build enables no test-support | partially fixed | R20 (minor) |
| RECORD | `--example record`, free calls only | fixed | R21-R27 (minor), N2 and N3 (nits). Header leak fixed by d413faa and f4d3f36. Batch and non-JSON bodies refused since f185baf |

## Remaining issues

### R10 (major). Late job ids cannot be drained through the provider, and they die with it

- **Where:**
  - `mcp/mod.rs:513` (the `late` field) and `:1270` (`late_route`).
  - `provider/higgsfield/mod.rs:57` (the private `mcp` field).
  - `provider/mod.rs:306` (`GenerationProvider`).
- **Problem:**
  - Each `McpClient` owns its late queue. The sink that helper threads hold keeps its own Arc
    of it.
  - `HiggsfieldProvider` keeps its client private and has no drain method.
    `GenerationProvider` has none either. Only tests that hold a bare client call
    `take_late_results`.
  - Cost checks already use this path. Every non-Free request gets a late route
    (`mcp/mod.rs:887`).
  - The app may drop or replace a provider (disconnect, reconnect, account switch). A helper
    that is still reading then pushes its ids into a queue nobody can read. That can happen
    up to 60 s (cost check) or 120 s (paid call) after the cancel, plus the 30 s verdict
    wait.
  - R6 made the guard app-owned for this same reason. The late queue stayed per client.
- **Impact:** job ids that should reach the ledger are lost (§4.7 rule 4). Nothing is resent.
  The app does not use this path yet, so this blocks H2. It is not a live loss.
- **Fix:**
  - Make the queue app-owned, like the guard. Pass an `Arc<LateQueue>` into
    `HiggsfieldProvider::new` / `with_endpoint` and on to `McpClient::new`.
  - At minimum, add `take_late_results` to `GenerationProvider`, for both Higgsfield and
    Scripted.
  - Test: cancel a cost check whose late answer names a UUID, then drop the provider. Assert
    that the id still reaches the app's queue.
- **Severity note:** one verification pass rated this minor, because nothing reads the queue
  yet. It stays major here, because the current API makes R9 impossible to meet.

### R11 (minor). The R9 criteria name a drain the worker cannot reach

- **Where:** `docs/architecture/cloud-generation.md:953` (the H2 contracts line) and
  `:966-981` (the R9 criteria).
- **Problem:**
  - The first criterion drains `McpClient::take_late_results`. The worker holds a provider,
    not a client.
  - The contracts line (`GenerationProvider::{prepare_upload, confirm_upload, submit, poll}`)
    has no drain.
  - The criterion says "every tick and before quitting". It does not say "before a provider
    is dropped or replaced".
- **Fix:**
  - Add the drain, or the app-owned queue from R10, to the contracts line.
  - Extend the obligation: drain before a provider is dropped or replaced, or own the queue
    in the app.
  - Use the R10 test as the acceptance test.

### R12 (minor). The job-id scan is narrower than "every item" (R5)

- **Where:** `mcp/mod.rs:1399`, `:1412`, `:352`, `:1360` and `:1095`.
- **Problem:**
  1. `parse_tool_result` keeps only `type == "text"` items, and it drops a `structuredContent`
     that is not an object. `resource`, `resource_link` and `_meta` are never scanned. The raw
     answer at `:763` is used only for Protocol errors.
  2. Only HTTP status 500 and above is scanned. A 4xx body that names a job is Rejected.
  3. The 5xx excerpt goes through `quoted()`, which cuts it to 500 characters. An id after
     that is lost. No late copy exists for a body under 2 KiB.
  4. In an SSE answer, only the response message is scanned. An earlier event that names the
     job, such as `notifications/progress`, is never looked at. A refusal after it is
     Rejected. A cost answer that has a cost is a Quote, which releases the tool.
  5. From R4: in the JSON path, `read_body(...).map_err(body_error)?` drops the partial bytes
     on TooLarge or I/O errors before the scan. Only the late copy has those ids.
- **Why minor:** the recorded Higgsfield answers use only text items and object
  `structuredContent`, with no progress events. Point 2 follows the current doc. Nothing is
  resent.
- **Fix:**
  - For every outcome of a paid call or a cost check, scan the raw answer bytes: the JSON
    `bytes`, the SSE `read` buffer, and the partial bytes on error.
  - Scan the raw 5xx excerpt before `quoted()`.
  - Decide what to do with 4xx bodies, and make the doc match.
  - Tests:
    - a `resource_link` item naming a job in an `isError` result;
    - a 4xx body and a long 5xx body naming a job;
    - an SSE progress event naming a job, before a refusal and before a cost answer.

### R13 (minor). The UUID scan misses ids glued to hyphens or hex letters

- **Where:** `guard.rs:120-139` (`uuids_in_text`).
- **Problem:**
  - It splits on any character that is not a hex digit or `-`, then needs exactly five
    groups.
  - So `job-<uuid>`, `request-<uuid>`, `id-<uuid>` and `<uuid>-0.png` are all missed.
  - This is the only scan, on every path. On a tool error or a readable cost answer, a missed
    id makes the result Rejected or a Quote, and the tool is released.
  - a664030 fixed exactly this in the record scrubber, with a sliding 36-byte window. The
    guard scan was not changed to match.
- **Fix:**
  - Look for an 8-4-4-4-12 window at every byte position, with no token boundaries.
    Over-detection fails closed.
  - Add unit cases for the four shapes above.

### R14 (minor). Any UUID in a refusal counts as a job id

- **Where:** `error.rs:271-280`, and `quote()` in `provider/higgsfield/mod.rs` (the
  `check_cost` error arm and the `job_ids()` check after it).
- **Problem:**
  - Media ids in the request are UUIDs, because `RemoteMediaId::parse` requires one.
  - The service echoes argument values in its refusals. The recorded fixture has
    `unknown model "..."`.
  - So a refusal like "media <uuid> not found" becomes OutcomeUnknown for a paid call, with
    the media id written to the ledger as a job. For a cost check it becomes StartedRun,
    which blocks the tool until restart.
  - The nil and max UUIDs are accepted too. They appear in the zod pattern in the tool
    schemas. FreeCut does not send the fields that use that pattern, so this part is
    hardening only.
- **Why minor:** money stays safe, because the result fails closed. The cost is a false
  "outcome unknown", a polluted ledger, a poller polling media ids, and blocked tools.
- **Fix:**
  - Before Phase B adds media references, leave out the request's own UUIDs (or report them
    apart from new job ids), and skip nil and max.
  - Test: an `isError` or -32602 refusal that echoes an argument's media id stays Rejected
    for a paid call. For a cost check, it releases the tool.

### R15 (minor). A late result cannot be tied to the run that caused it

- **Where:** `mcp/mod.rs:379-394` (`LateToolResult`), `:702` (`call_tool`), `:718`
  (`check_cost`) and `:867-870` (`send_once`).
- **Problem:**
  - A late result carries the tool name, the JSON-RPC request id and the job ids.
  - The caller never learns the request id. It is allocated inside `send_once`, and again on
    each retry.
  - The result does not say whether it came from a cost check or a paid run.
  - Almost every operation maps to `generate_image` or `generate_video`, and two runs can be
    in flight. So the worker cannot tell which ledger entry a late id belongs to.
- **Fix:**
  - Let callers pass a correlation handle, such as a run id, to `call_tool` and `check_cost`.
  - Carry that handle and the call class (Paid or Preflight) into `LateToolResult`.
  - Add an R9 acceptance test: with two runs of one tool in flight, a late id lands on the
    right run.

### R16 (minor). A cut chunked SSE stream is never resumed

- **Where:** `mcp/mod.rs:1047` (`read_stream`).
- **Problem:**
  - Only a clean EOF leads to a resume.
  - ureq 3.4.2 reports a chunked or content-length body that is cut before its end as an
    `UnexpectedEof` error. A reset gives the same error. `read_stream` turns it into a
    Protocol error and does not resume.
  - So over real HTTP/1.1, R3 only helps when the server ends the stream cleanly.
  - The fake and loopback tests use only clean EOF or close-delimited streams.
- **Why minor:** Higgsfield sends no `id:` lines (§15.1), so there is never anything to
  resume, and the result is the safe Outcome unknown. It matters for a future server with
  event ids. MCP 2025-11-25 says a client should resume after network failures too.
- **Fix:**
  - When a last event id exists, treat a non-cancel read error like EOF and resume. A GET
    never resends the request.
  - In `http.rs`, have `ChannelReader` record Finished when it hands over the terminal
    error. Bytes the caller fully read are then not queued again as a late copy.
  - Test: a loopback server that sends chunked SSE, primes `id: ev-1`, drops the socket
    inside the response event, then serves the replay on GET. Assert one POST, one GET, and
    that the answer arrives with its job id.

### R17 (minor). R7 holds only while one TokenManager lives

- **Where:** `oauth/mod.rs:726-771` (the `TokenManager` fields and `new`),
  `provider/higgsfield/mod.rs:71-90`, and `cloud-generation.md` §5.6 and `:965-981`.
- **Problem:**
  - `refreshing`, the `writing` counter and `unsaved` all belong to one instance.
  - Nothing says the app must keep only one.
  - If the Phase B app builds a second one (reconnect, account switch, or the sign-in and
    disconnect UI):
    - The new instance reads R1 from the store and sends it. That is the R7 failure again.
    - Two instances can refresh R1 at the same time, which can trip reuse detection.
    - A sign-in on the other instance does not bump this one's counter. A refresh in flight
      here can then overwrite the new sign-in (SEC-1), and its old `unsaved` tokens shadow
      the new account.
- **Fix:**
  - On `TokenManager::new` and in §5.6, write that the app keeps one TokenManager per
    provider id for its whole run. The app passes that same `Arc<dyn TokenSource>` to every
    provider and to the sign-in and disconnect UI.
  - Add this to the R9 obligations, next to the PreflightGuard line.
  - Test: after a refused store write, recreate the provider. Assert that rt-2 is sent and
    rt-1 never is.
  - Fix the struct doc at `:722-723` ("nothing is cached between requests except the server
    metadata"). Since `unsaved` was added, that is no longer true.

### R18 (minor). Refused token writes wait for the next refresh

- **Where:** `oauth/mod.rs:812` (`flush_unsaved`). Its only caller is `:895`.
- **Problem:**
  - Tokens the store refused are written again only at the next refresh or 401. That can be
    most of an access-token lifetime after the keychain unlocks.
  - A quit in that window means the next start sends the superseded R1.
  - No public method lets the worker save the tokens before quitting.
- **Why minor:** the doc states this limit honestly, and the trigger is rare. The worst case
  is that the user signs in again. No money is involved.
- **Fix:**
  - In `bearer()`, when `unsaved` is set, `try_lock` `writing` and flush. Ignore a failure.
  - Add a public `TokenManager::persist_unsaved()`. List it in the R9 obligations, to be
    called before quit.
  - Test: a refused write, then unlock, then one `bearer()` call with no refresh. The store
    then holds at-2/rt-2.

### R19 (minor). No test proves that an unreviewed model is refused before any request

- **Where:** `provider/higgsfield/mod.rs:208-210` and `:236-241`, and `tests/mcp_fake.rs`.
- **Problem:**
  - The code is right. `catalogue()` applies the reviewed list, and `quote()` filters on
    `is_visible()` before the guard and `check_cost`.
  - But every provider-level quote test uses reviewed models. Deleting either line fails no
    test, and an unreviewed model would then reach `get_cost`.
  - The message "is not in the service's model list" is wrong for a model the service lists
    but FreeCut has not reviewed.
- **Fix:**
  - Add a mcp_fake test that quotes `nano_banana_2_shots` (in the fixture, not reviewed) and
    a synthetic new video id. Assert `ModelUnavailable`, and that
    `calls_to("generate_image")` and `calls_to("generate_video")` stay empty.
  - Reword the message, for example "{id} is not offered in FreeCut (not listed or not
    reviewed)".

### R20 (minor). The test-support guard misses features enabled through the app's own features table (TSGUARD)

- **Where:** `crates/freecut-app/tests/deps_guard.rs:317-351`, and
  `crates/freecut-cloud/src/lib.rs:40`.
- **Problem:**
  - The guard reads the lines of `cargo tree -e normal,features`. A dependency feature that
    the app's own `[features]` turns on never shows up as a line.
  - Proof in the real workspace: `freecut-audio`'s `record` feature comes from the app's
    `default = ["record"]`. It is enabled, but the guard's output has no line for it.
  - A scratch probe confirmed that `default = ["x"]`, `x = ["freecut-cloud/test-support"]`
    passes the guard.
  - Non-default app features and other targets' dependency tables are missed too.
  - `pub mod fake` is still ungated, so the fake server would ship in the app.
- **Why minor:** the app does not depend on freecut-cloud yet, so nothing ships with it
  today.
- **Fix:**
  - Use `cargo tree -p freecut-app -e normal --all-features --target all --prefix none
    --format "{p} [{f}]" --offline`. Fail when a line that starts with `freecut-` has
    `test-support` in its feature list. Keep the sanity check.
  - Repeat the mutation check with an app `[features]` entry, and record it in the commit.
  - Gate `pub mod fake` behind `cfg(any(test, feature = "test-support"))`.
  - This must land before the app depends on freecut-cloud.

### R21 (minor). The record scrubber matches e-mails and JWTs only as whole words

- **Where:** `examples/record.rs:519` and `:563-573`.
- **Problem:**
  - An e-mail inside a URL, or `%40`-encoded, is kept.
  - A JWT is kept when it follows `=` or `;`, is followed by `.`, sits in brackets, or has 5
    parts.
  - The service answers only with SSE. There, JSON inside `content[].text` arrives escaped.
    Replacing an escaped e-mail drops the escaping backslash, so the data line is no longer
    valid JSON.
  - The test runs only in the fake's JSON mode.
  - `email` is still not in `SECRET_FIELDS`, which round 2 asked for.
- **Fix:**
  - Scan for e-mails and JWTs anywhere in the text, as `uuids()` does, and replace only the
    match.
  - Better: scrub each SSE data line as parsed JSON, including `content[].text` parsed as
    JSON, then serialise it again.
  - Also run `free_calls_are_recorded_and_scrubbed` with `StreamMode::Sse { chunk: 64 }`.
    Assert that the e-mail and the UUID are gone and every data line still parses.
  - Add `email` to `SECRET_FIELDS`.

### R22 (minor). The record tool has no final check for the secrets it holds

- **Where:** `examples/record.rs:118` and `:128`.
- **Problem:**
  - Only the client id is replaced by its value. Everything else relies on field names and
    word shapes.
  - The owner cannot spot an opaque refresh token, auth code or PKCE verifier by reading a
    1.3 MB file.
  - The current file is clean.
- **Fix:**
  - Before writing, collect every secret held in memory:
    - a copy of the access token, taken before it moves into `StaticToken`;
    - the refresh and ID tokens;
    - the auth code, the PKCE verifier and the state;
    - the e-mail and `sub` from the ID token claims.
  - Replace each one in raw, percent-encoded and JSON-escaped form.
  - Then refuse to write, and exit non-zero, if any raw value is still in the output.
  - Test: plant each value in a body, a header and an error string.

### R23 (minor). `redact::json` turns whole schema objects into "<redacted>"

- **Where:** `src/redact.rs:36-37`, used by `body_json` in `record.rs`.
- **Problem:**
  - Any value under a key named `code` or `token` is replaced, even a JSON Schema object.
  - In the recorded `tools/list`, 28 schema nodes became strings. Examples: 13 times
    `lease/properties/token`, several `error/properties/code`, and the `code` input of two
    scene_builder tools.
  - The e-mail pass also changed a word in two public tool descriptions to
    `user@example.test`.
  - The allow-listed tools are intact, but the file is not a faithful record.
- **Fix:**
  - In the recorder, redact a secret key only when its value is a string or a number, and
    recurse into objects and arrays. Or apply `SECRET_FIELDS` only to the OAuth exchanges.
  - Test: a schema property named `code` survives.

### R24 (minor). The recording keeps the account's allowance

- **Where:** `examples/record.rs:433` (`body_json`).
- **Problem:**
  - Each `models_explore` answer carries `unlim.available`, `unlim.remaining` and
    `unlim.expires_at` for the signed-in account.
  - The values appear three times: in `structuredContent`, inside `content[0].text`, and as
    prose in a second text block. All are recorded verbatim.
  - Today they are `false` and nulls. An account with an allowance would record its count
    and expiry, and the design says fixtures hold no private account data.
- **Fix:**
  - Replace the `unlim` object with fixed fake values in every copy, and the allowance prose
    with fixed text.
  - List this in the README's scrub rules.

### R25 (minor). The record refusal gate allows more than the tool needs

- **Where:** `examples/record.rs:323-347`.
- **Problem:**
  - It checks only URLs under ENDPOINT.
  - It lets every method other than `tools/call` through.
  - It allows every Free tool. That includes `media_upload`, which creates upload slots and
    is banned during development. It also includes `balance`, whose answer would be
    recorded, and `jobs_wait`.
  - `main` cannot reach any of these today, so this is defence in depth only.
- **Fix:**
  - Allow exactly `initialize`, `notifications/initialized`, `tools/list`, `tools/call` with
    `name == "models_explore"` and `action == "list"`, and replies that have no method.
    Refuse everything else.
  - Apply the `get_cost` and `tools/call` checks to every URL.

### R26 (minor). The callback listener ends the run on the first bad connection

- **Where:** `examples/record.rs:224-266`, and the call at `:104`.
- **Problem:**
  - State checking is sound, and no token or code can leak. This is about availability only.
  - Any of these ends the whole sign-in:
    - a socket that stays idle for 5 s (a browser's spare connection, or a port scan);
    - non-UTF-8 input;
    - an accept error;
    - one `/callback` request with the wrong state.
  - `read_line` has no length cap.
  - Phase B's app sign-in should not copy this pattern.
- **Fix:**
  - On read errors, timeouts, InvalidData, ConnectionAborted, Interrupted, NotTheCallback and
    StateMismatch, answer 400 and keep waiting.
  - End the wait only on a result whose state matches.
  - Read the request line through `(&stream).take(8 * 1024)`.
  - Drop the listener as soon as the code arrives.

### R27 (minor). On Windows, the browser opener splits the sign-in URL at `&`

- **Where:** `examples/record.rs:207-220` (`open_in_browser`).
- **Problem:**
  - `cmd /C start "" <url>` gets the URL unquoted, because it has no spaces. cmd treats each
    `&` as a command separator.
  - The browser gets only `...?response_type=code`, and the other pairs run as commands.
  - The owner is on macOS, so their run was not affected.
  - An authorization endpoint that already has a query is not refused.
- **Fix:**
  - Use `rundll32 url.dll,FileProtocolHandler <url>` or `explorer.exe <url>`, or escape `&`
    as `^&` with `raw_arg`.
  - Refuse an authorization endpoint that already carries a query.

### Nits

- **N1. Only the provider applies the reviewed list** (`catalogue.rs:634`, `:666`, `:683`).
  - `Catalogue::from_pages`, `from_listings` and `parse_listing` are public, and what they
    build shows unreviewed models.
  - A future disk cache (§6.2) or submit path must apply the list too. `quote()` still
    refuses them, so this is UI only.
  - Add a provider-owned `catalogue_from_cache` with a test, or one line in the H2
    obligations.
- **N2. Record hygiene.**
  - `record.rs:11` and line 41 of the fixtures README still promise a session DELETE. The
    server is stateless, and `close()` sends nothing.
  - `tests/fixtures/higgsfield/recorded/` is untracked and not ignored, so `git add -A` would
    commit it before review. Add it to `.gitignore` until it is approved, then use
    `git add -f`.
  - The server name and version are printed raw (`record.rs:144-150`). Pass them through
    `quoted()`.
  - Each run registers another Clerk client. Two exist now.
- **N3. Unzeroed secret copies in the record tool.**
  - `Recorder::send` buffers the whole token answer, then builds an unredacted
    `serde_json::Value` from it. Neither is zeroed.
  - Production streams this answer, so this copy is extra.
  - Zero the buffered token answer once the caller has read it. The other copies are best
    effort, as §5.6 says.

## Checked and dismissed

These were raised, then refuted by the second pass. Do not change code for them.

- **A cancel during a resume delay ends as Outcome unknown without ids.** §4.4 allows this.
  Nothing is resent, and Higgsfield sends no event ids. One doc sentence would be enough.
- **An in-flight check and a blocked tool both return `PreflightStopped`.** The round-2 brief
  asked for exactly this. `PreflightGuard::is_blocked` tells the two apart.
- **`McpClient::check_cost` can be called without a reservation.** `McpClient` is the
  low-level transport. Only `quote()` calls it, and `quote()` reserves first.
- **The NotSent and Rejected release paths are untested.** An unsettled reservation blocks
  on drop, so the existing asserts prove the release. The session-404 test also checks again
  after a Rejected.
- **`disconnect` may revoke the in-memory tokens before a failed delete.** That is the
  existing revoke-then-delete design. rt-1 is never sent.
- **A refresh whose answer is lost sends the same refresh token again.** Every OAuth client
  does this, and the §15.2 note does not claim otherwise.

## Message for the agent

Round 3 of the Higgsfield review is done. The verdict is **approve with changes**. The full
notes are in `docs/handoff/higgsfield-review-round3.md`.

R3, R4, R6, R7 and R8 are fixed. The reviewed model list and the record tool are approved.
R5, R9 and the test-support guard are partly done. Thank you for the tests. They are good.

Fix the items below. Each fix needs a test that fails before the fix.

Late job ids and the money path:

1. **R10 (major).** Make the late queue app-owned, like the guard. Pass an `Arc` into the
   provider and the client, or at least add `take_late_results` to `GenerationProvider`.
   Test: cancel a cost check, drop the provider, and the late id still arrives.
2. **R11.** In §14 H2, add the drain to the contracts line. Say to drain before a provider is
   dropped or replaced.
3. **R12.** Scan the raw answer bytes for job ids on every outcome of a paid call or cost
   check. That covers non-text items, 4xx bodies, the 5xx excerpt before `quoted()`, earlier
   SSE events, and partial bytes on error.
4. **R13.** Find UUIDs at any byte position, like the record scrubber does now.
   `job-<uuid>` and `<uuid>-0.png` must be found.
5. **R14.** Do not count the request's own UUIDs (or nil and max) as job ids.
6. **R15.** Let callers pass a run id, and carry it and the call class into
   `LateToolResult`.
7. **R16.** When a last event id exists, resume after a read error too, not only after a
   clean EOF.

Tokens:

8. **R17.** Document one TokenManager per provider for the whole app run, and add it to the
   R9 obligations. Fix the outdated struct doc.
9. **R18.** Retry refused token writes in `bearer()`, and add a public `persist_unsaved()`
   for quit.

Models and guards:

10. **R19.** Add a provider test: quoting an unreviewed model sends nothing and returns
    `ModelUnavailable`. Fix the message wording.
11. **R20.** Use `cargo tree ... --all-features --target all --format "{p} [{f}]"` in the
    test-support guard. Gate `pub mod fake` behind test-support.

Record tool:

12. **R21-R27.**
    - Scrub e-mails and JWTs anywhere, and keep SSE JSON valid.
    - Add a final check that no secret value from memory is written.
    - Redact only scalar values.
    - Fake the `unlim` allowance.
    - Allow only the calls `main` makes.
    - Keep the callback listener waiting after a bad connection.
    - Fix the Windows browser opener.
13. **Nits N1-N3** when convenient. Ignore `recorded/` in git until it is approved.

Do not commit the current recording. It holds no secrets, but 28 schema nodes are redacted
and two descriptions were changed. After R21-R24, ask the owner to run `record` once more on
macOS. The owner signs in. Send the new file for review.

Phase B may start once these fixes are in. Follow this order (from the round-2 brief and
`higgsfield-branch.md`):

1. The record tool is the first item, and it is done. The new recording is committed only
   after review.
2. Land R20 before the app depends on freecut-cloud. Then add the app's `cloud` feature. It
   stays off until the user connects.
3. Next come the sign-in UI (Preferences > Cloud generation), the keychain `SecretStore`, the
   loopback listener and the worker thread. Use one PreflightGuard, one TokenManager and one
   late queue per app run (R6, R17, R10).
4. Then the Generate tab and the run sheet with cost checks. The catalogue reaches the UI only
   through the reviewed list.
5. Then paid submit, the Runs page and ledger, import, and Place. R9, R11, R15 and R18 are the
   acceptance criteria for this worker.
6. Put `model: cloud provenance` in one separate commit. Do not change
   `CURRENT_FORMAT_VERSION`.

The money rules have not changed:

- Never retry or resend a paid call.
- Anything that may have reached the server without a readable answer is Outcome unknown.
- Send cost checks only with `get_cost: true`, and never retry one that may have been sent.
- Tokens never go into logs, files or fixtures.
- No test or tool makes a paid call or a cost check against the real service.
