# Review round 2: feature/higgsfield, Phase A fixes (`freecut-cloud`)

- Reviewed range: `8636c30..feature/higgsfield`. R1 and R2 were already checked as fixed at
  8636c30. This round covers M1-M4, SEC-1..9, F4, F7, F8, F10, Q3-Q7, Q9, Q10 and the CI step.
  c3303d3 (F8) was amended from 04a1742. Neither commit was ever pushed, so no published
  history was rewritten.
- Previous review: `docs/handoff/higgsfield-review-phase-a.md`. Its M5-M12 map to SEC-1,
  SEC-2, SEC-3, F4, F8, Q3, Q4 and Q6, and this file uses those ids.
- Method: three verification passes (money safety, security, quality) against the code, the
  design doc (`docs/architecture/cloud-generation.md` §4.5, §4.7, §5, §10) and the ureq 3.4.2
  and rustls sources. Everything ran offline. The only listeners were loopback probes in a
  scratch crate, and the worktree was not changed.
- Checks run (all offline, target dir in the worktree):
  - `cargo test -p freecut-cloud`: 101 unit, 8 catalogue_and_golden, 35 mcp_fake,
    20 oauth_flow, 1 doctest. All pass.
  - `--features http` and `--features http,test-support`: 108 unit plus 6 loopback_http and
    the same suites. All pass.
  - `cargo clippy -p freecut-cloud --all-targets -D warnings` with no features, `http`,
    `test-support` and `http,test-support`: clean.
  - `cargo fmt -p freecut-cloud --check`: clean.
  - `cargo test -p freecut-app --test deps_guard` **does not compile** on this branch (see
    the blocker below). A copy of deps_guard.rs run from a scratch crate against the
    worktree passes 8/8.

## Verdict: approve Phase A with changes

The fixes are real and nearly all are tested. Paid calls are no longer retried or resent on
any path. RPC errors and unreadable cost answers now fail closed. Cancel is gated after TLS.
The credential store is locked and crash-safe. Debug output is redacted. There is a single
pinned rustls with no bundled roots.

What is left falls into three groups:

- **Ledger loss.** A few paths still drop job ids, so a run the user paid for may never show
  up in FreeCut. None of these paths resends anything, so the owner rule holds. But §4.7
  rule 4 (record every started run) does not hold yet.
- **One token-rotation gap.** A keychain that locks during a refresh can bring back a
  superseded refresh token.
- **Build blocker.** `freecut-app` does not compile on this branch. It inherits timeline
  types from 2bad49c that the app does not handle. Because of that, the deps guards and the
  workspace CI steps cannot run.

Merge only after the rebase is green. Fix R3-R9 before Phase B wires any paid call or the
app's sign-in. The free-only record tool can start once the rebase is green (next steps).

## Item table

| Id | Topic | Verdict | Left over (details below) |
|---|---|---|---|
| R1 | initialize/exchange recursion | fixed (no regression) | - |
| R2 | call class taken from the allow list | fixed (no regression) | - |
| M1 (F5) | cost checks fail closed | partially fixed | R5, R6 |
| M2 (F2) | JSON-RPC errors on paid calls | fixed | id scan on the Rpc arm and on plain-text tool errors: R5 |
| M3 (F3) | cancel gate, late job ids | partially fixed | R4. A cancel during connect is reported MaybeSent (safe, doc wrong): recommended |
| M4 (F6) | at-most-once property | fixed | widen the strategy with the R3-R5 tests |
| SEC-1 (M5) | disconnect/sign-in vs refresh, store locks | fixed | nit: tokens dropped this way are not revoked |
| SEC-2 (M6) | keychain set/delete leave nothing behind | fixed | nit: stale refresh parts after a failed `clear_slot` |
| SEC-3 (M7) | keep rotated tokens the store refuses | partially fixed | R7 |
| SEC-4..6 | redacted Debug | fixed | nits: `IdClaims`, `Meta`, the session-id header, `email` in `SECRET_FIELDS` |
| SEC-7 | exact issuer comparison | fixed | - |
| SEC-8 | clear copies of secrets | fixed | nits: form serializer growth, proxy password lifetime |
| SEC-9 | strip invisible characters from service text | partially fixed | recommended: TAG block, two raw error strings |
| F4 (M8) | SSE resumption | partially fixed | R3. No overall time cap: recommended |
| F7 | send-state assumption written down | fixed | wording is not strictly true (soft ICMP errors): recommended |
| F8 (M9) | `notifications/cancelled` | fixed | Phase B notes |
| F10 | non-message events, DELETE header, allow.rs doc | fixed | - |
| Q3 (M10) | no bundled root certificates | fixed | the guard runs only once the app builds |
| Q4 (M11) | licence records | fixed | - |
| Q5 | rustls pinned in the workspace | fixed | - |
| Q6 (M12) | proxy only from HTTPS_PROXY, never loopback | fixed | nits: other loopback spellings, `https_only=false` with a proxy |
| Q7 | send-state table covers every variant | fixed | - |
| Q9 | no sockets outside http.rs | partially fixed | recommended: also ban the `ureq` and `rustls` tokens |
| Q10 | ScriptedProvider behind test-support | fixed | recommended: a guard that keeps test-support out of normal deps |
| CI | cloud clippy and test step | partially fixed | R8, plus the build blocker |
| CLAUDE.md conventions | forbid(unsafe), thiserror, no unwrap | fixed | - |

## Blocker: freecut-app does not compile on this branch

- The errors are E0004 non-exhaustive matches in `crates/freecut-app/src/prop_keys.rs:194,236`,
  `session.rs:604` and `ui/look.rs:10,31`.
- They come from 2bad49c, which has timeline types the app does not handle yet:
  `ColorParamId::WheelStrength/Hsl`, `ColorOp::Curve/Puck`, `PropPath::ColorPuck` and
  `PropValue::Puck`. This round did not cause them.
- Consequences:
  - `cargo clippy --workspace` and `cargo test --workspace` fail in CI before the cloud step
    runs.
  - The deps_guard tests (Q3, Q9) never run.
- The fix is next step 1 below.

## Required follow-ups

These are correctness, money and security gaps only. Each one needs a test that fails
before its fix.

### R3. The SSE parser commits `Last-Event-ID` too early, so a cut stream loses its answer (F4)

- **Where:** `crates/freecut-cloud/src/mcp/sse.rs`, the `id` arm in `end_line`. Also
  `next_stream()`, which keeps `last_id`.
- **Problem:**
  - The parser stores the id as soon as it reads the `id:` line, not when the event is
    dispatched.
  - So if the stream is cut inside the response event, after its `id:` line, FreeCut resumes
    *after* that event. A compliant server never sends it again.
  - Probe: after the input `id: ev-1 … data:\n\nid: ev-2\ndata: {partial`, the parser
    reports `last_event_id() == Some("ev-2")`.
- **Impact:**
  - A paid call ends as OutcomeUnknown and its job ids are lost.
  - A cost check ends as PossiblyStarted and blocks that tool until restart.
- **Fix:**
  - Follow WHATWG: keep a pending id buffer and commit it only when the event is dispatched.
    That includes an event with empty data.
  - Add a fake-server test with this exact cut. It should assert that the resumed stream
    replays the event and the answer arrives.

### R4. Answers the caller abandons without a cancel lose their job ids (M3)

- **Where:** `crates/freecut-cloud/src/http.rs` `run()`, and `read_answer` / `read_stream`
  in `mcp/mod.rs`.
- **Problem:**
  - Sometimes the caller stops reading because of a Protocol error, not a cancel: bad JSON in
    an event, an oversized event, a broken stream, a failed resume, or an unreadable JSON
    body.
  - The helper still reads the remaining bytes, but it only calls the late sink when
    `abandoned` or `cancelled` is set. Here neither is, so those bytes are dropped.
  - The bytes the caller already read are never scanned for job ids either.
- **Fix:**
  - Set `abandoned` whenever a `ChannelReader` that has a late sink is dropped before EOF.
  - On Protocol errors from a paid call or a cost check, run `uuids_in_text` over the bytes
    already read. Return any ids with the OutcomeUnknown.
  - Test with Garbage and CutStream faults that carry a job id.

### R5. Refusal paths drop job ids (M1, M2)

- **Where:** `provider/higgsfield/mod.rs` `quote()`, and `error.rs` `submit_disposition`
  (both the Tool and Rpc arms). Also `job_ids_in`.
- **Problem:**
  - A cost check whose tool error names job ids becomes `PossiblyStarted`. That variant has
    no `job_ids` field, so the ids are dropped.
  - The Rpc arm never scans `e.data`. An error with a "rejecting" code is `Rejected` even
    when it names a job.
  - The Tool arm only scans structured content or text that parses as JSON. A plain-text
    `isError` that mentions a job UUID becomes `Rejected`. Since MCP 2025-11-25 this is the
    main refusal path.
  - `job_ids_in` scans only `output.value()`, not every text item.
- **Fix:**
  - Use one id scan everywhere: every content item (structured and text, JSON or not), plus
    `e.data` and the raw message.
  - If it finds ids, the result is never `Rejected` or `NotSent`. For a cost check, return
    `StartedRun { job_ids }`, or add `job_ids` to `PossiblyStarted`. For a paid call, return
    OutcomeUnknown with the ids.
  - Add these cases to the M4 property strategy: tool `isError` with and without ids, in JSON
    and in plain text, and an Rpc error whose `data` carries an id.

### R6. The preflight guard must be atomic and live as long as the app (M1)

- **Where:** `guard.rs`, and `quote()` in the provider.
- **Problem:**
  - Calling `is_blocked()` and then `check_cost()` is not atomic, so two concurrent checks of
    the same tool can both go out. §15 item 14 says a cost check can spend money.
  - The guard lives inside each `HiggsfieldProvider`, so recreating the provider (reconnect,
    account switch) clears "blocked until restart".
- **Fix:**
  - Reserve the tool under one lock before sending (states: in flight, blocked). A second
    check while one is in flight is refused with `PreflightStopped`.
  - Release the reservation only on a recognised answer, NotSent or Rejected.
  - Have the app own the guard (for example an `Arc<PreflightGuard>` passed to the provider)
    so it outlives provider instances.
  - Test two threads checking the same tool: exactly one request reaches the fake.

### R7. The keychain can lock during a refresh and bring back a superseded refresh token (SEC-3)

- **Where:** `oauth/mod.rs:914` and `oauth/mod.rs:966-967`.
- **Problem:**
  - After the refresh network call, the code re-reads the store with `?`.
  - If the keychain is locked or unreadable at that point (for example, macOS locks it on
    sleep or screen lock), the code returns `SignInUnavailable` before the renewed tokens
    reach `unsaved`.
  - After unlock, the stored credential still holds R1, and the next refresh sends it. That
    breaks M7's rule, and on a server with reuse detection it can revoke the whole token
    family.
  - Separately, `disconnect` clears `unsaved` before `store.delete`. If the delete fails on a
    locked store, the newer in-memory tokens are gone and the stale stored ones come back.
- **Fix:**
  - Detect "deleted or replaced during the refresh" with an in-memory write generation. Every
    write bumps it under `writing`. Do not re-read the store for this. At minimum, keep the
    renewed tokens in `unsaved` when the get fails.
  - In `disconnect`, clear `unsaved` only after the delete succeeds.
  - Test with a store whose reads and writes both fail during the refresh call. Assert that
    R1 is never sent again in this process.
  - Doc: reword the §5.6 note that the superseded token "is never sent again". That holds
    only while FreeCut runs. After a quit with unsaved tokens, it is sent on the next start.

### R8. The Windows CI step hides a clippy failure

- **Where:** `.github/workflows/ci.yml`, the cloud step.
- **Problem:** on `windows-latest` the default shell is pwsh. In a multi-line `run: |`, only
  the last command's exit code fails the step, so a clippy failure there is hidden.
- **Fix:** add `shell: bash` to that step, or split clippy and test into two steps.

### R9. Phase B obligations (make them acceptance criteria, not code changes now)

- **Drain the late queue.** Late job ids sit only in an in-memory queue, and they can arrive
  up to 120 s after a cancel (longer across resumes). The Phase B worker must drain it on
  every tick and before quit, and write the ids to the ledger. Draining once right after the
  cancel is not enough.
- **Explain blocked checks.** Cost checks use pooled connections with no cancel gate, so
  cancelling a check in flight ends as PossiblyStarted and blocks that tool until restart.
  The UI must say so. If that is too harsh in practice, route checks through the gated
  agent.
- **Keep `notify_cancelled` off the UI thread.** It blocks, and `tokens.bearer()` may refresh
  over the network under a token that cannot be cancelled.

## Recommended, not blocking

- **M3:** a cancel during connect is reported MaybeSent, because the wait loop checks the
  token before `head_rx`. This is safe, but it contradicts the http.rs module doc and sends
  an unneeded `notifications/cancelled`. Either share a CAS state between the gate and the
  caller (Connecting, then Writing or Abandoned), or correct the doc. Add a test that goes
  through `send()`.
- **F4:** put a cap on the total time across resumes. The worst case today is about 6 × 120 s.
- **F8:** document that a timeout without a cancel sends no `notifications/cancelled` (MCP
  says SHOULD), if that is deliberate.
- **F7:** reword the §4.5 assumption. Linux and BSD can report EHOSTUNREACH or ENETUNREACH on
  an established connection. Optionally, a pass-through connector could record that the
  connection is up.
- **SEC-9:**
  - Drop the TAG block (U+E0001, U+E0020-E007F), U+1BCA0-1BCA3 and U+1D173-1D17A in
    `quoted()`.
  - Pass `CallbackError::Denied.error` (`oauth/mod.rs:210`) and `TokenError::Other.code`
    (`oauth/token.rs:114`) through `quoted()`.
- **SEC-4..6:**
  - Give `IdClaims` (`oauth/token.rs:123`) and `Meta` (`secret.rs:360`) a redacted Debug.
  - Add `email` to `redact::SECRET_FIELDS`.
  - Consider hiding `mcp-session-id` in the HttpRequest Debug output.
- **SEC-1:** on a best-effort basis, revoke rotated tokens that `disconnect` drops.
- **SEC-2:** when `refresh_parts == 0`, remove every refresh part in the target slot.
- **SEC-8:**
  - Use `String::with_capacity` for the form serializer (`transport.rs:145`).
  - Mention in §5.6 that the proxy password is kept for the app's lifetime.
- **Q6:**
  - Have `proxy_for` return None when `!https_only`.
  - Optionally bypass all of 127.0.0.0/8 and `*.localhost`.
- **Q9:**
  - Also ban the `ureq` and `rustls` tokens outside http.rs.
  - Optionally catch `net::` inside grouped `use std::{..}` imports, and
    `std::os::unix::net`.
- **Q10:** once the app depends on freecut-cloud, add a deps_guard check that its normal
  dependency never enables `test-support`. Consider gating `pub mod fake` the same way.
- **Docs:** update §5.6 for the two-slot `access.<slot>.<n>` scheme and for delete clearing
  both slots.
- **Test hygiene:** `cancelling_before_sending_sends_nothing` uses the real service URL. Use
  a loopback or unroutable URL instead.

## Next steps for the branch agent

### 1. Rebase onto main at 339c149 and get it green

- Rebase `feature/higgsfield` onto `339c149` ("Colour K2–K3: HSL, curves, wheels, LUTs and
  skin protection"). Main is green there, and it has the app-side colour work this branch is
  missing.
- Resolve the conflicts by keeping both sides:
  - `CLAUDE.md`: keep main's text and the branch's cloud and crate rules.
  - Root `Cargo.toml` and `Cargo.lock`: keep both sets of members and workspace
    dependencies, keep the `rustls =0.23.45` pin, then let `cargo build --offline` update the lock (do not regenerate it from
    scratch). Confirm there is still exactly
    one rustls.
  - `crates/freecut-app/tests/deps_guard.rs`: keep main's guards and the cloud guards (Q3,
    Q9).
  - `docs/architecture/README.md`: keep both sides' sections and table rows.
- Rebase onto the commit, not onto main's working tree. The main checkout has uncommitted
  K4 work belonging to another agent. Do not touch it, and build only in the worktree's own
  target dir.
- Re-run every check:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo clippy -p freecut-cloud --all-targets --features http,test-support -- -D warnings`
  - `cargo test -p freecut-cloud --features http`
  - `cargo test -p freecut-app --test deps_guard`, which must now actually run.
- Do not push until all of these pass.

### 2. Owner decision on models: a reviewed allow list

- **Decision:** v1 shows only a reviewed allow list of plain image and video generation
  models.
- **Hidden:** pipeline and internal models (`ad_multiplier*`, `hf_mult_*`, `3d_jutsu_*`,
  `cinematic_studio_soul_*`) and every model nobody has reviewed. `image_auto` stays hidden
  too, because the model and price are not known before the check.
- **Why this is a change:** today `catalogue.rs` (`HIDDEN_IDS` / `HIDDEN_PREFIXES` /
  `hidden_reason`, around l.424-462) is a deny list. A model the service adds tomorrow is
  shown by default, which is the opposite of what the previous review asked for.
- **Implementation:**
  - Add a `REVIEWED_MODELS` list in the Higgsfield provider (for example in
    `provider/higgsfield/map.rs` or a new `reviewed.rs`). Each entry gives the model id and
    its kind (image or video).
  - Build the list from the recorded catalogue fixture (`tests/fixtures/higgsfield/`). Have
    the owner confirm it, and say in a comment who reviewed it and when.
  - Every catalogue model not on the list is hidden with `HiddenReason::NotReviewed`. Keep
    the existing specific reasons (voice, identity training, styles, pipeline-only) where
    they apply, so the UI can explain them.
- **Tests (in `tests/catalogue_and_golden.rs`):**
  - Every visible model in the fixture is on the list.
  - The named pipeline prefixes and `image_auto` are hidden.
  - A synthetic, previously unseen model id (image or video) is hidden as NotReviewed.
  - Every list entry is an image or video generation model.
- Update `cloud-generation.md` §6.2 to describe the allow list.

### 3. Phase B may start once the rebase is green

- **First item: `--example record`.** It makes free calls only: discovery, initialize,
  `tools/list`, `models_explore`, `ping` and the session DELETE. Everything goes through
  `allow::lookup` and Free-class tools.
- **No paid or cost calls.** The record tool must not call `get_cost` or any paid tool.
- **The owner signs in once**, interactively. The agent never handles the owner's
  credentials.
- **Scrub fixtures before committing them:** tokens, the e-mail, the client id, and any job
  or media ids.
- **Order of work:**
  - Land R3-R8 before Phase B wires any paid call, cost check or the app's sign-in UI.
  - Treat R9 as acceptance criteria for the Phase B worker.
  - The models allow list (step 2) must land before the catalogue reaches the UI.
