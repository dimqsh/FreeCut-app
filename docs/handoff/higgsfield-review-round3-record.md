# Review round 3, addendum: the record follow-ups (50038bf..cbf9a2f)

**Verdict: approve with changes.** The money rule holds: no paid call and no `get_cost` can
leave the record tool on any path it can take today. Nothing private leaked into the fixture.
There is no blocker and no major. This round adds six minors (R28-R33) and four nits
(N4-N7). R25 and N2 from the main round-3 review are still open.

- **Fixture audit.** After the one-off pass, the orchestrator audited
  `crates/freecut-cloud/tests/fixtures/higgsfield/recorded/free-calls.jsonl`:
  - only allowed headers remain;
  - the tokens, the auth code, the PKCE verifier and the client id are redacted;
  - there are no cookies, no country and no trace ids.

  This review read only the file's structure (header names and keys), never its values.
- **Scope.** This review covers commits 50038bf, d413faa, f185baf, f4d3f36, a664030, c0d46d3
  and cbf9a2f in `../FreeCut-higgsfield`, with b5f315d as the base. Line numbers are valid at
  cbf9a2f. It adds to `higgsfield-review-round3.md`, which owns R10-R27 and N1-N3.
- **Method.** Three lanes reviewed the work: the refusal gate, scrubbing and headers, and the
  stateless docs. A second pass then tried to refute every finding. Only findings that
  survived are listed, with the severity from that pass. Everything ran offline: no network,
  no `cargo run`, and no run of `record`.

## Commits

| Commit | What it does | Result | Items |
|---|---|---|---|
| 50038bf | Owner confirmed the reviewed model list | OK. Doc text only. `REVIEWED_MODELS` is byte-identical to b5f315d | - |
| d413faa | Header allow list, default deny | OK. Applied to requests and responses, case-insensitive, and the bearer token never reaches `headers` | R30 (minor) |
| f185baf | Refuse any body that is not one JSON-RPC object | OK for its goal. Batches, BOM, non-UTF-8 and gzip bodies are refused | R25 still open, N4 |
| f4d3f36 | `--filter-headers` one-off pass | OK. Returns before any transport or OAuth client is built, and prints header names only | N5, N6 |
| a664030 | UUIDs replaced wherever they sit, nil and max kept | Issue. No panic path, but the ids it misses are the URL-context case the commit set out to cover | R28, R29 (minor), N7 |
| c0d46d3 | §15.1 findings: stateless server, protocol, SSE, tokens | Issue. The §15.1 table now contradicts itself, and "Still unknown" is incomplete | R32, R33 (minor), N2 |
| cbf9a2f | Test `a_stateless_server_needs_no_session` | OK. Fails if `close()` sends a DELETE. But the doc's 404 claim is not tested | R31 (minor) |

Also checked and fine:

- Redirects are never followed (`max_redirects(0)` on both agents), so a redirect cannot carry
  a body somewhere else.
- No file under `src/` changed except `reviewed.rs`, so session support is intact for a server
  that does issue ids.

## Issues

### R28 (minor). The UUID scrubber keeps an id that follows a percent or `\b`/`\f` escape

- **Where:** `crates/freecut-cloud/examples/record.rs:542`, in `Scrubber::uuids`.
- **Problem:**
  - `free_before` refuses a match when the byte before it is a hex digit.
  - A percent escape (`%2F`, `%3D`, `%22`) and a JSON `\b` or `\f` escape both end in a hex
    digit.
  - A copy of the code, run in memory, left these ids in place:
    - `next=%2Fjobs%2F<id>`
    - `id%3D<id>`
    - `%22<id>%22`
    - `https://x/_next/image?url=https%3A%2F%2Fcdn.example%2F<id>.webp&w=640`
    - `\b<id>` and `\f<id>`
  - `word()` keeps a query that has no signature marker, so nothing else catches these ids.
  - `free-calls.jsonl` has no job ids and is unaffected. A Phase B recording with job or media
    ids would leak them.
- **Fix:**
  - Treat a hex byte as a boundary when it closes an escape:
    `(i >= 3 && bytes[i - 3] == b'%')`, `(i >= 2 && bytes[i - 2] == b'\\')`, and
    `(i >= 6 && &bytes[i - 6..i - 4] == b"\\u")`.
  - These are byte checks, so they are safe at char boundaries. If they replace too much,
    that is the safe direction.
  - Another option is to also scan a percent-decoded copy.
  - Add `next=%2Fjobs%2F{id}`, `id%3D{id}`, `%22{id}%22` and `\b{id}` to
    `uuids_are_replaced_anywhere_but_nil_and_max_stay`. `{id}%22` already passes.

### R29 (minor). Object keys are never scrubbed

- **Where:** `record.rs:478-492`, in `Scrubber::value`. The key is copied as-is at `:490`.
- **Problem:**
  - Objects are rebuilt as `(k.clone(), v)`, so only values go through `text()`.
  - A UUID or e-mail used as a map key in a JSON answer would be written unchanged.
  - Event-stream bodies are stored as strings, so keys inside them are covered.
  - Four of the 10 recorded answers are JSON objects. None of their keys holds an id today.
- **Fix:**
  - Match `k == "mcp-session-id"` on the original key, then write `self.text(k)` as the key.
  - Every e-mail becomes `user@example.test`, so two e-mail keys in one object would collapse
    into one. Give each key its own fake, or refuse to write on a collision.
  - Test with a UUID key and two e-mail keys.

### R30 (minor). A repeated allowed header is recorded with its last value, but FreeCut acts on the first

- **Where:**
  - `record.rs:421-428` (`headers_json`) keeps the last value.
  - `crates/freecut-cloud/src/transport.rs:231-236` (`HttpResponse::header`, which uses
    `.find()`) reads the first.
- **Problem:**
  - `http.rs` passes each value of a repeated header as its own pair.
  - Collecting the pairs into a `serde_json::Map` keeps the last one.
  - So with several `www-authenticate` challenges, or a split `cache-control`, the fixture
    shows a different value from the one the client used.
  - This loses fidelity only. Nothing leaks. The collect predates d413faa.
- **Fix:**
  - Keep the first value with `map.entry(k).or_insert(v)`, or record repeats as an array.
  - Add a doubled `www-authenticate` to the canned answer in
    `only_allowed_headers_are_recorded`.

### R31 (minor). The stateless "404 is an ordinary error" claim is not tested

- **Where:**
  - `crates/freecut-cloud/tests/mcp_fake.rs:1326`.
  - The code is `crates/freecut-cloud/src/mcp/mod.rs:958`
    (`404 if session.is_some() => SessionExpired`).
  - The claim is in `docs/architecture/cloud-generation.md:1104`.
- **Problem:**
  - `set_sessions(false)` appears only in the new test, and that test injects no fault.
  - Every `Fault::Status(404)` in the suite runs with a session.
  - So changing the arm to a plain `404 =>` passes every test. On the real stateless server,
    a free call would then re-initialize and be sent again. A paid call would report "session
    renewed, press Run again" instead of `Rejected`.
  - The code is correct today, and a paid call still goes out at most once.
- **Fix:**
  - Add a sibling test with `fake.set_sessions(false)`.
  - Push `Fault::Status(404, ..)` before a `models_explore` call. Assert
    `McpError::Http { status: 404 }`, no second `initialize`, and that the call went out once.
  - Push it before a submit. Assert `SubmitError::Rejected` and exactly one `generate_image`.
  - Optionally add `sessions in any::<bool>()` to the at-most-once proptest.

### R32 (minor). The §15.1 table contradicts its new rows

- **Where:** `docs/architecture/cloud-generation.md`.
  - **Line 1099 (Transport row).** It still says the authenticated framing (JSON or SSE,
    `Mcp-Session-Id`, the negotiated revision) is "not yet observed". Rows 1103-1105 now
    record all three.
  - **Line 1090 (intro).** It says only "discovery, one client registration,
    `models_explore` and `get_cost: true` were called". Row 1103 adds:
    - a second registration;
    - the code exchange;
    - `initialize` and `notifications/initialized`;
    - `tools/list`.
  - **Line 1096 (DCR row).** It says "One FreeCut registration now exists". Row 1103 says
    two exist.
- **Fix:**
  - Line 1099: replace the sentence with a pointer to the signed-in rows.
  - Line 1090: say that only free calls were made, in two sessions: the probes and the
    owner's signed-in recording.
  - Line 1096: change "One" to "two", or point to row 1103.

### R33 (minor). "Still unknown" drops two known unknowns

- **Where:**
  - `cloud-generation.md:1109-1111`.
  - Item 5 of §15, at `:1043-1046`, still lists them.
- **Problem:**
  - Only free calls were recorded, so these answers are still unseen:
    - the paid submission answer, which carries the job ids;
    - the `jobs_wait` answer.
  - The rewritten list leaves both out.
  - It also says "the first two need a longer signed-in session". That fits rotation and
    revocation, but device sign-in needs a device-code sign-in instead.
  - Your report says "the answer shapes of real runs", which is broader than what the doc
    lists.
- **Fix:** rewrite the list as:
  - refresh-token rotation and revocation (need a longer signed-in session);
  - device sign-in (needs a device-code sign-in);
  - the answer shapes of real runs (need paid runs):
    - submissions with job ids (`generate_image` and `generate_video`);
    - `jobs_wait`;
    - `unlim_choice`, `adjustments` and `recovery_tool`.

### R25 (minor, from round 3, still open). The refusal gate allows more than the tool needs

- **Where:** `record.rs:336-346`. This code predates these commits. f185baf only added the
  checks around it.
- **Problem, confirmed again this round:**
  - The gate checks only whether a tool is `CallClass::Free`. So `media_upload`, `balance`
    and `jobs_wait` go out with any arguments, and `models_explore` goes out with any action:
    - `media_upload` creates upload slots, and the development rule bans it
      (`allow.rs:10-11`, branch doc §7).
    - A `balance` answer would put the account's credit figures into the fixture, and the
      scrubber has no rule for them.
  - Only `method == "tools/call"` is inspected:
    - `"method":"Tools/Call"` and `"method":["tools/call"]` naming `generate_image` are sent
      (nit).
    - A spec-compliant server would refuse both, so this is not a live hole.
  - `main` reaches none of these today, so this is defence in depth.
- **Fix:**
  - Allow `tools/call` only for `models_explore` with `action == "list"`.
  - Allow the handshake methods `initialize`, `notifications/initialized`, `tools/list`,
    `ping` and `notifications/cancelled`.
  - Allow a body with no `method` only when it has `id` plus `result` or `error`.
  - Refuse everything else, including a `method` that is not a string.
  - Test that each of these is refused NotSent:
    - `media_upload`, `balance` and `jobs_wait`;
    - `models_explore` with `action: "recommend"`;
    - `"Tools/Call"` and an array `method`.

### Nits

- **N4. The `get_cost` check matches raw text** (`record.rs:15`, `:322`, `:328`).
  - The comment promises to refuse "any request that carries get_cost". The check is a
    substring test on the raw bytes, so an escaped key (`get_cost`) on a free tool gets
    through.
  - No paid tool can pass, because the name check still refuses it. FreeCut's serializer
    never writes that escape.
  - Fix: reword to "any body whose text names get_cost", or walk the parsed value (parsing
    string-typed `params` and `arguments` too) and refuse any key or string that contains it.
- **N2 (from round 3, still open). The session DELETE text.**
  - `record.rs:11` and `tests/fixtures/higgsfield/README.md:41` still list "the session
    `DELETE`" among the recorded calls.
  - c0d46d3 edited that same README paragraph but kept the claim. The recording has no
    DELETE, and `close()` sends none without a session.
  - Fix: write "a session `DELETE` only if the server issues a session; Higgsfield's issues
    none (§15.1)".
- **N5. `record` ignores unknown arguments** (`record.rs:73-78`).
  - A typo such as `--filter-header f`, or the `--filter-headers=f` form, skips the offline
    branch. The tool then runs discovery and a client registration and opens the sign-in
    page.
  - These are free calls, the browser shows what is happening, and the file is only
    rewritten after a completed sign-in.
  - Fix: reject any argument other than `--print-url`, or `--filter-headers <file>`, with a
    usage message before building the transport.
- **N6. The one-off pass does not re-run the scrubber** (`record.rs:164`, `filter_file`).
  - `free-calls.jsonl` was scrubbed by the b5f315d scrubber, not the a664030 one.
  - The audit found only the nil and max UUIDs, so nothing leaks.
  - Optional: run `Scrubber::default().value(..)` after the header filter, or note in the
    README that the file predates a664030.
- **N7. `uuids()` allocates at every byte** (`record.rs:541`, `guard.rs:132`).
  - `is_uuid` collects `split('-')` into a `Vec` at every byte. A body at the 32 MiB cap
    costs about 33M allocations, which is seconds in a debug build.
  - Fix: check that bytes `i+8`, `i+13`, `i+18` and `i+23` are `-` before calling
    `is_uuid`.

## Tests and lint

All runs were offline, in `../FreeCut-higgsfield` at cbf9a2f, and this pass re-ran them.

- `cargo test -p freecut-cloud --features http`: 202 pass, 0 fail. That is 112 unit, 12
  catalogue_and_golden, 7 loopback_http, 41 mcp_fake, 21 oauth_flow, 8 record example tests
  and 1 doctest. It matches the reported 202.
- `cargo clippy -p freecut-cloud --features http --all-targets -- -D warnings`: clean.
- `cargo fmt -p freecut-cloud --check`: clean.
- **Not run here:** the workspace fmt, clippy and test, and `deps_guard`. They are outside
  this review's allowed commands. The 10/10 deps_guard result is the agent's own report.

## Message for the agent

> Round-3 review of the record follow-ups (50038bf..cbf9a2f) is done: approve with changes.
> The header allow list, the one-object body check, the one-off pass and the stateless test
> do what you said. I found no way for a paid call or a cost check to leave the record tool.
> The owner's audit of free-calls.jsonl after your pass is clean. Please fix these, each
> with a test:
>
> 1. R28: the UUID scrubber keeps an id that comes right after a percent escape
>    (`%2F<id>`, `%3D<id>`, `%22<id>`) or a `\b`/`\f` escape, because the escape ends in a
>    hex digit. In `uuids` (record.rs:542), count the end of such an escape as a boundary.
>    Add those cases to `uuids_are_replaced_anywhere_but_nil_and_max_stay`.
> 2. R29: `Scrubber::value` never scrubs object keys (record.rs:490). Scrub keys too, still
>    matching `mcp-session-id` on the original key, and don't let two keys collapse into
>    one fake.
> 3. R30: a repeated allowed header is recorded with its last value, but FreeCut acts on
>    the first. Keep the first (or record an array) in `headers_json`, and test a doubled
>    `www-authenticate`.
> 4. R31: add a stateless test where the server answers 404. A free call should fail with
>    Http 404 and no second initialize. A paid call should be Rejected with exactly one
>    generate_image.
> 5. R32: fix the §15.1 rows that contradict the new ones. Line 1099 still says the
>    signed-in framing is "not yet observed". Line 1090 lists only the first probes. Line
>    1096 says one registration.
> 6. R33: "Still unknown" should also name the submission answer with job ids and the
>    `jobs_wait` answer. Device sign-in needs a device-code sign-in, not a longer session.
> 7. R25 (from the round-3 review, still open): limit the refusal gate to what the tool
>    sends:
>    - `models_explore` with action `list`;
>    - the handshake methods;
>    - replies that have `id` plus `result` or `error`.
>
>    Refuse `media_upload`, `balance`, `jobs_wait`, any other action, and any `method` that
>    is not exactly one of those strings.
>
> Small ones, any time:
>
> - N4: reword the `get_cost` comment, or check the parsed body.
> - N2: drop the session DELETE from record.rs:11 and the fixtures README line 41.
> - N5: make `record` reject unknown arguments, so a typo in `--filter-headers` cannot start
>   a live sign-in.
> - N6 and N7 (optional): re-scrub in the one-off pass, and check the dashes before calling
>   `is_uuid`.
>
> The Phase B gate from the round-3 review is unchanged. Nothing here adds a blocker, but R28
> and R29 must be in before any recording that holds job or media ids. Run fmt, clippy and
> the cloud tests afterwards. Leave free-calls.jsonl uncommitted until the owner says to
> commit it.
