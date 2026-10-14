# Review: feature/higgsfield, Phase A (`freecut-cloud`)

- Reviewed range: `2bad49c..c5d34b1` (e7d6a71 CLAUDE.md, d2c2665 crate, c5d34b1 docs §15).
- Also read: the follow-up commits on the branch tip (85cda36, 09408c6, 9540591, b118cfa,
  70fad79). Where a finding was fixed there, it says so.
- Method: three review passes (security, money safety, quality), and each finding was then
  checked against the code and the docs. Nothing was sent over the network. Line numbers are
  for the branch tip.

## Verdict: changes required

The crate is in good shape overall. The layering is clean, the secret types redact themselves,
the send-state table leans to the safe side, there are fake-server and loopback tests, and the
deps guard is in place. Two major issues must be fixed before Phase B wires the app. One can
crash the process. The other leaves the rule that paid calls are never resent to caller
discipline. The money-safety minors (M1-M4) should be fixed in the same pass, because Phase B
is the first phase to send paid calls.

## Required changes

### R1. `initialize` and `exchange` can recurse without limit (crash plus request flood)

- **Where:** `crates/freecut-cloud/src/mcp/mod.rs:405-414`, together with the `SessionExpired`
  arm at `:570`.
- **Why:**
  - `initialize` stores the new session id, then sends `notifications/initialized` through
    `exchange(.., CallClass::Free, ..)`.
  - If that notification gets a 404, `send_once` maps it to `SessionExpired`.
  - The `SessionExpired` arm only skips `out.method == "initialize"`, so for the notification
    it calls `self.initialize()` again.
  - Each nested call starts with `reinitialized = false`, so nothing stops the loop.
  - The result: one initialize and one notification per level until the stack overflows,
    which aborts the whole app. A load balancer without sticky sessions is enough to trigger
    it. No paid call goes out, because the handshake never finishes.
- **Fix:**
  - Send `notifications/initialized` without the session-expiry recovery. A 404 there means
    the handshake failed.
  - Alternatively, pass an `in_handshake` flag or depth so `initialize` never re-enters itself.
  - Add a `FakeService` fault that answers 404 to the notification. Assert one handshake
    attempt and a `NotSent` error for the call that triggered it.

### R2. The transport takes the call class from the caller, not from the allow list

Covers Q1 and F9.

- **Where:**
  - `crates/freecut-cloud/src/mcp/mod.rs:484`: `pub fn call_tool(name, arguments, class, limits, cancel)`.
  - `crates/freecut-cloud/src/provider/higgsfield/mod.rs:98`: `pub fn mcp(&self) -> &McpClient`.
- **Why:**
  - `class` alone decides three things: retries on 5xx/429 (`may_retry`), the resend after a
    401 or session 404 (`resends_after_refusal`), and whether the §4.7 preflight guard runs.
  - Any tool name is accepted, including unlisted ones such as `execute_preset`.
  - So `generate_image` sent as `CallClass::Free` would be retried and resent. That breaks
    §4.5 ("any call whose classification is unclear is treated as paid"), §6.3 ("denied by
    construction") and the owner's rule that paid calls are never resent.
  - The tests already take this path (`tests/mcp_fake.rs:562, 584, 724`). Only the provider's
    private `call_free` consults `allow::lookup`.
- **Fix:**
  - Have `call_tool` take a `&'static AllowedTool` (or a typed tool enum), and read `class`
    and `limits` from the allow list. This keeps the H1 contract in §14, which lists
    `call_tool` as public, while making the class impossible to choose. An unlisted tool
    cannot be expressed at all.
  - Remove `HiggsfieldProvider::mcp()`, or narrow it to `cfg(any(test, feature = "test-support"))`.
  - Add a test that no API path sends a tool marked Paid with a retrying class.

## Minor items

### Money safety: fix before the first paid call in Phase B

- **M1. Started-run guard fails open** (F5): `provider/higgsfield/mod.rs:280`.
  - Today a tool is blocked only when the answer contains a UUID-shaped job id. An unknown
    answer shape, or a MaybeSent preflight failure, comes back as `Protocol` with no block, so
    the next Check cost can start another paid run.
  - Fix: fail closed. Any preflight answer that is not a recognised `cost` / `unlim_choice` /
    `recovery_tool` shape, and any MaybeSent preflight failure, should block that tool's checks
    until restart and report "possibly started". Keep the UUID scan only for collecting ledger
    ids.
- **M2. JSON-RPC server faults on a paid submission become `Rejected`** (F2):
  `error.rs:234-255`.
  - Map -32603, -32001 and any code the classifier does not recognise to `OutcomeUnknown`.
  - Keep `Rejected` for -32600/-32601/-32602 and for refusals the service classifier
    recognises (credits, plan, content, model, busy, invalid).
  - Update §10's table, which says "Anything else → Failed". This is a gap in the spec, not
    only in the code.
- **M3. Cancel does not stop the helper thread, and late job ids are lost** (F3):
  `http.rs:253-278`.
  - Give the helper thread the `CancelToken`, and check it just before `agent.run`. A connector
    layer that checks it after connect would also work.
  - For paid calls, add a late-result channel so ids that arrive after cancel still reach the
    ledger, as §4.4 and §7.2 require.
- **M4. The "sent at most once" property is close to tautological** (F6):
  `tests/mcp_fake.rs:732`.
  - Add `submissions == 1 => disposition != NotSent`.
  - Add fake faults that record the request and then fail, one for each error kind, including
    Rpc -32603.
  - Once the provider's submit path exists, drive the property through it.

### Credentials (CG-002)

- **M5. Disconnect can race a refresh, and concurrent `set` calls can mix parts** (SEC-1).
  - Where: `oauth/mod.rs:846`, where `disconnect` never takes `refreshing`; `store_sign_in`
    and `store_pasted` at `:755` and `:777`; `secret.rs:519-571`, where `set` has no writer
    lock.
  - The window is real, because §4 abandons cancelled helpers and lets them run on.
  - Fix: take one credential mutex in `disconnect`, `store_sign_in`, `store_pasted` and
    `refresh`. After the network refresh, do not write if the credential was deleted or
    replaced in the meantime. Add a test that runs disconnect during a refresh.
- **M6. `delete` and `set` leave token parts behind** (SEC-2): `secret.rs:521`, `:574`.
  - `delete` should also sweep generation G+1. A crash between writing the parts and writing
    `meta` leaves the newest tokens there.
  - `unwrap_or(None)` must not treat an unreadable `meta` as empty. Propagate `Unavailable`.
    On `Corrupt`, sweep a bounded range of generations.
  - Add a test that runs delete after the existing crash test.
- **M7. A failed keychain write after a refresh discards the rotated tokens** (SEC-3):
  `oauth/mod.rs` around `:830-836`.
  - Keep the renewed credential in memory, use its access token for the current request, and
    retry the store write before the next refresh. Never send the superseded refresh token.

### Protocol

- **M8. No SSE resumption** (F4): `mcp/mod.rs:732-746`.
  - FreeCut offers 2025-11-25 but treats EOF after a priming event as a broken stream.
  - Either implement GET plus `Last-Event-ID`, honour `retry`, and cap the attempts (a GET does
    not resubmit), or offer only 2025-06-18 until that is done.
  - Record the choice in §4.1 or §15.2.
- **M9. No `notifications/cancelled` on a local cancel** (F8), which §4.4 requires.
  - Send it best effort, on a fresh token, with the CLOSE limits.

### Dependencies, licences, CI

- **M10. webpki-roots** (Q3): fixed in 85cda36.
  - Still to do: add `webpki-roots` to deps_guard's banned list for `--features http`.
- **M11. Licence records** (Q4): partly fixed in 70fad79.
  - Add ISC to README §4.3. ring, rustls-webpki and untrusted use it.
  - Add rows for `getrandom` and the direct `rustls` dependency to the §4 table.
  - Fix §2.3: the `http` feature no longer controls `sha2`.
- **M12. Proxy variables** (Q6): `http.rs:68`.
  - `Proxy::try_from_env()` also reads ALL_PROXY and HTTP_PROXY, and does not exclude loopback.
  - Add `TransportConfig::proxy_from_env`, set it to false in `tests/loopback_http.rs`, and
    correct the comments in `Cargo.toml` and `http.rs`.

### Nits

- **SEC-4 / Q8: debug output shows secrets.**
  - `MemoryEntryStore` derives `Debug` and prints raw token parts (`secret.rs:588`). So does
    `ChunkedSecretStore`, which delegates to it. Write a manual `Debug` that shows names and
    counts only.
- **SEC-5:** `AuthorizeRequest`'s derived `Debug` prints `state` as part of `url`
  (`oauth/mod.rs:83`). Redact the query.
- **SEC-6:** `OAuthCredential`'s `Debug` prints `account_label`, which is the account e-mail
  (`secret.rs:169`). Redact it.
- **SEC-7:** issuer checks use `same_url`, which ignores trailing slashes
  (`discovery.rs:325`, `oauth/mod.rs:195, 789, 794`). RFC 8414 and RFC 9207 require an exact
  match. Keep the trailing-slash tolerance only for `resource`.
- **SEC-8:** some copies of secrets are never cleared: the bearer header, the form bodies, the
  `split_chars` chunks and the `read_parts` buffer. Say in §5.6 that clearing memory is best
  effort, and close the cheap cases.
- **SEC-9:** `quoted()` (`error.rs:157`) should also strip bidi and other `Cf` characters
  (U+200E/F, U+202A-202E, U+2066-2069).
- **F7:** `send_state` decides `NotSent` from `io::ErrorKind` whatever the phase. Note the
  assumption in §4.5, or record the phase in which the error happened.
- **F10:**
  - `read_stream` should skip SSE event types other than `message`.
  - The session `DELETE` is missing `MCP-Protocol-Version`.
  - The allow.rs module doc (l.7-8) needs rewording: the development rule (never call upload
    tools) is separate from `CallClass`.
- **Q5:** put `rustls` in `[workspace.dependencies]` and pin it exactly (0.23.45, matching
  ureq). Today it is `"0.23"` in the crate manifest.
- **Q7:** §15.2 says "a test lists every 3.4.2 variant". It does not list `Error::Http`,
  `Protocol`, `Pem`, `BodyStalled` or `Timeout::Await100`. Add them, or reword the sentence.
- **Q9:** deps_guard should also scan the source: no `std::net`, `TcpStream`, `UdpSocket` or
  `TcpListener` outside `http.rs`.
- **Q10:** §2.2 and §6.1 promise a `ScriptedProvider` that does not exist yet. Add it before
  the Phase B app tests, behind the same test-support feature.
- **Rejected:** Q2 (`media_upload` classed Free). It matches §4.5 and §6.3. Only the allow.rs
  comment needs rewording (see F10).

## Answers to the open questions

1. **webpki-roots licence.**
   - Avoid it. The platform verifier is enough, and 85cda36 already does this correctly
     (`rustls-no-provider` with an explicit ring provider).
   - Keep the test that the first HTTPS request does not panic for lack of a crypto provider,
     and ban `webpki-roots` in deps_guard (M10).
   - `webpki-root-certs` only appears for wasm32 and is never built, so recording it in §15.2
     is enough. If either crate ever ships, accept CDLA-Permissive-2.0 and record it in
     `assets/ASSETS.md` or the README §4 licence notes at that point.
2. **CI.**
   - Yes. b118cfa adds clippy and tests with `--features http` to `.github/workflows/ci.yml`,
     which is right.
   - The loopback tests depend on M12. A runner that sets proxy variables would break them, so
     add `proxy_from_env: false` before you rely on the job.
3. **Device sign-in.**
   - Yes: request the device grant only when the server advertises it, and keep it as a
     fallback. 09408c6 does this.
   - Browser sign-in stays the main path. Show *Use a code instead* only when the echoed
     grants include the device grant.
   - The re-registration fallback creates a second client registration. That is acceptable,
     but persist the registration that wins so FreeCut does not register again on every
     launch.
4. **Record tool first in Phase B.**
   - Yes, make `--example record` the first Phase B item. The owner signs in once, and the
     tool makes free calls only: discovery, initialize, tools/list, `models_explore`, `ping`
     and DELETE.
   - Two conditions:
     - (a) Land R2 first, so the example can only call tools the allow list marks Free.
     - (b) Do not include `get_cost` checks until M1 is fixed. The free cost answers are
       already recorded, and a check that misbehaves can spend.
   - Scrub tokens, the e-mail, the client id and any job or media ids from the fixtures before
     they are committed.
5. **Newly listed models.**
   - Agree with 9540591: hide `ad_multiplier*`, `hf_mult_*`, `3d_jutsu_*`,
     `cinematic_studio_soul_*` and `image_auto` as `HiddenReason::NotReviewed`.
   - `image_auto` is not a plain image model. The service picks the model per prompt, so the
     model and price are not known before the check. Keep it hidden.
   - Hiding `dubbing`, `voice_change` and `sync_so` matches the decision not to build voice
     tools.
   - Keep the rule that a model id not reviewed is hidden by default (allow list, not deny
     list), so the next catalogue change cannot expose new pipeline models.
6. **The four §15.2 deviations.**
   - **Server requests (§4.1): accept.** FreeCut declares `capabilities: {}`
     (`mcp/mod.rs:378`), so answering `ping` with `{}` and every other server request with
     -32601 is exactly what MCP specifies.
   - **Keychain entries (§5.6): accept the scheme** (generation-named parts, `meta` written
     last). It only delivers "old or new set, never a mix" once M5 (writer lock) and M6
     (sweep G+1 and corrupt meta on delete) are fixed. Until then the claim holds for crashes
     only.
   - **Contracts (§6.1): accept.** These are building `connect` from a `TokenSource`,
     `quote(Operation)` with `RoleRef` inputs, and `QuoteOutcome::StartedRun`. `StartedRun`
     is the right way to carry §4.7 rule 4. Update §6.1's signatures in the body text, not
     only in §15.2, and add `ScriptedProvider` (Q10) against the new trait.
   - **No resend of cost checks after a refusal (§4.5, §4.7): accept.** It is stricter than
     the spec and consistent with "preflight can spend" (§15 item 14). It only holds if the
     class cannot be chosen by the caller, which is R2.
