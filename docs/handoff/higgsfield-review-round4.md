# Review round 4 (feature/higgsfield, cbf9a2f..45e4bb1): Approve with changes

**Verdict: Approve with changes.** Phase B may start now, in parallel with the fixes below.
R34 comes first, in its own commit. No blocker and no major survived the second pass.

- **Reviewed commits:** the 30 commits in `cbf9a2f..45e4bb1` in `../FreeCut-higgsfield`, one
  per item (R10-R33, N1-N7). N7 has no commit of its own; it is folded into cec7cf7 and
  d2b60c9. Line numbers below are valid at 45e4bb1.
- **Review files:** `higgsfield-review-round3.md` (R10-R27, N1-N3) and
  `higgsfield-review-round3-record.md` (R28-R33, N4-N7) in this folder.
- **Method:**
  - Four lanes: the money path (late ids, job-id scan, SSE resume); tokens and guards; the
    record tool; docs and the H2 gate.
  - A second pass then tried to refute every open finding. Only findings that survived are
    listed below, with the severity from that pass. Where the two votes split, the reason is
    given.
  - Everything ran offline. There was no paid call, no cost check, no network, no `cargo run`
    and no run of `record`. The recording's values were never read; the committed blob got
    one count-only scan.

## Owner decision

The owner accepted R12's rule: **a 4xx body that names a job is Outcome unknown** (fail
closed). This review only checked that the rule is applied everywhere and documented in §10.

- §10 documents it (`docs/architecture/cloud-generation.md:848`, "4xx as well as 5xx").
- The code applies it to every 4xx that reaches the generic status arm. A 409 is tested.
- It does not apply it to 401, 403 or a session 404. Those bodies are never read. That is
  R35.

## Summary

All 30 commits do what their messages say, and each comes with a test. What holds:

- **Late job ids.** The late queue is app-owned (R10). Every late result carries its run and
  call class (R15). A paid call or cost check needs a `RunTag` before anything is sent.
- **The job-id scan.** It reads every byte of an answer (R12). It finds UUIDs glued to
  hyphens or hex letters (R13). It never counts the request's own ids, or the nil and max
  UUIDs, on failures (R14).
- **SSE resume.** A broken stream that has an event id is resumed with a GET, never a
  re-POST (R16).
- **Tokens.** There is one TokenManager per provider id (R17). Refused writes are retried in
  `bearer()`, and `persist_unsaved()` exists for quit (R18).
- **Models and guards.** An unreviewed model is refused before any request (R19). The
  test-support guard sees app features, and `fake` is gated (R20).
- **The record tool.** All of R21-R30 and N2-N7 are in.
- **Docs.** R11, R31, R32, R33 and N1 are in.

I found no path that retries or resends a paid call. No path retries a cost check that may
have been sent, and nothing secret is written.

What is left:

- **R34 (fix first).** The R10 commit also committed the unreviewed recording. N2's ignore
  rule therefore has no effect on it.
- **Money path, latent.** These all fail closed and need to be in before cost checks reach
  the UI and before paid submit:
  - 401, 403 and session-404 bodies are not scanned (R35);
  - a cancel after a broken stream drops ids (R37);
  - SSE `id:` lines count as job ids (R38);
  - successful paid answers keep the request's own ids (R39);
  - the UUID scan skips windows that overlap a match (R40).
- **Guards and tokens.** CI never builds the shipping configuration (R41).
  `persist_unsaved()` can return Ok while a refresh is in flight (R42).
- **The record tool.**
  - The scrubber drops the blank line that ends each SSE event (R36). This matters for the
    recording the owner is about to make.
  - The final secret check can miss nested or lower-case forms (R43).
- **Docs.** Signatures and record-tool text are out of date (R44, R45).
- **Nits:** R46-R57.

## Item table

| Id | Topic | Status | Commit | Left over (details below) |
|---|---|---|---|---|
| R10 | late queue is app-owned | fixed | 4b1999e | The same commit added the unreviewed recording: R34 |
| R11 | H2 drain obligation names a queue the worker can reach | fixed | c48cfd6 | Stale signatures and rustdoc: R44, R47 |
| R12 | every byte of an answer is scanned for job ids | partially fixed | eaf5774 | 401, 403 and session 404 unscanned: R35. SSE framing scanned: R38 |
| R13 | UUIDs glued to hyphens or hex letters are found | fixed | cec7cf7 | Overlapping windows: R40 |
| R14 | the request's own UUIDs, nil and max are not jobs | fixed (failures, cost answers, late answers) | 279b059 | Successful paid answers: R39 |
| R15 | a late answer carries the run and the call class | fixed | b5d40f9 | - |
| R16 | a broken stream with an event id is resumed | fixed | 72fc8ab | R37, R46 |
| R17 | one TokenManager per provider id per run | fixed | 1cef1b3 | - |
| R18 | refused writes retried in `bearer()`; `persist_unsaved()` | fixed | afbeabe | R42, R51 |
| R19 | test: an unreviewed model is refused before any request | fixed | b1523db | - |
| R20 | test-support guard sees app features; `fake` gated | fixed | f4466cf | R41, R50 |
| R21 | e-mails and JWTs scrubbed anywhere; SSE JSON stays valid | fixed | 232f468 | SSE framing lost: R36 |
| R22 | every held secret replaced, then a final check | fixed | fa5f3f8 | R43, R57 |
| R23 | only scalar values under secret names are redacted | fixed | 99d5e4e | R52 |
| R24 | the account's allowance is faked | fixed | 69025a0 | - |
| R25 | the gate allows exactly the calls the tool makes | fixed | a3c708b | R53 |
| R26 | the sign-in listener survives bad connections | fixed | 9a933f8 | R54 |
| R27 | the Windows browser opener gets the whole URL | fixed | 21d4a03 | R55 |
| R28 | UUIDs right after escapes ending in hex are scrubbed | fixed | d2b60c9 | - |
| R29 | object keys scrubbed, never collapsed | fixed | bc241ae | - |
| R30 | a repeated header is recorded with its first value | fixed | 7e50171 | - |
| R31 | test: a stateless server's 404 is an ordinary error | fixed | daea61d | - |
| R32 | §15.1 no longer contradicts its signed-in rows | fixed | 74d538d | - |
| R33 | "Still unknown" names everything still unseen | fixed | 5d56418 | Optional additions: R49 |
| N1 | a cached catalogue gets the reviewed list | fixed | 45e4bb1 | - |
| N2 | record hygiene (DELETE text, ignore rule, quoted server text) | partially fixed | 53e996d | The ignore rule does nothing, because the file is tracked: R34 |
| N3 | buffered answers are zeroed once read | fixed | 9659c34 | R56 |
| N4 | `get_cost` refused however it is written | fixed | 0ef0bd6 | - |
| N5 | unknown arguments refused before anything is built | fixed | df4e6ac | - |
| N6 | the one-off pass re-runs today's scrubber | fixed | 772099a | Shares R36. The file is rewritten in place, not atomically |
| N7 | dashes checked before the hex scan | fixed | cec7cf7, d2b60c9 (no own commit) | - |

## Remaining issues

### R34 (minor, fix first). The unreviewed recording is committed, so N2's ignore rule does nothing

- **Where:**
  - `crates/freecut-cloud/tests/fixtures/higgsfield/recorded/free-calls.jsonl`. Commit
    4b1999e (R10) added it as blob 228572e, 1,339,381 bytes.
  - `.gitignore:9`.
  - `crates/freecut-cloud/tests/fixtures/higgsfield/README.md:55-56`.
- **Problem:**
  - The R10 commit added the recording, and its message does not mention it. No other commit
    touches the file.
  - Round 3 said twice not to commit it (`higgsfield-review-round3.md:545`,
    `higgsfield-review-round3-record.md:285`).
  - The file is still tracked at 45e4bb1 (`git ls-files`). Git does not apply an ignore rule
    to a tracked file, so `git check-ignore` exits 1.
  - The N2 commit message says "`git add -A` cannot commit one by accident". For this file
    that is false.
  - The README says the file "is committed only once the reviewer approves it". It is
    already committed.
  - A count-only scan of the blob found:
    - 32 `<redacted>` and 2 `user@example.test`;
    - 0 `eyJ`, 0 `Bearer`, 0 `Set-Cookie`, and no real e-mail.

    It is the over-redacted file from before R21-R24, and it holds no secret.
  - The next `record` run overwrites a tracked file. `git commit -a` would then commit an
    unreviewed recording.
  - Only the local `feature/higgsfield` contains 4b1999e. Nothing was pushed.
- **Why minor:** there is no secret and no money path. But it breaks an explicit review
  instruction, and it makes N2 ineffective.
- **Fix:**
  1. Now, in its own commit:
     `git rm --cached crates/freecut-cloud/tests/fixtures/higgsfield/recorded/free-calls.jsonl`.
     `--cached` leaves the working copy alone, so the owner's run is not touched. After this
     commit the ignore rule works, and README:55-56 is true again.
  2. Until the new recording has been reviewed, do not run any of these on that path:
     `git add`, `git commit -a`, `git stash`, `git checkout`, `git restore`. Do not rebase
     either.
  3. The old blob is still in 4b1999e. The branch is unpushed, so it can be dropped from
     history before merge. Do that only if the owner asks, and only after the new recording
     has been reviewed and kept safe.
  4. The approved recording goes in later with `git add -f`.

### R35 (minor). R12's rule skips 401, 403 and session-404 bodies

- **Where:**
  - In `crates/freecut-cloud/src/mcp/mod.rs`:
    - `:1095-1101`, the status mapping;
    - `:958-981`, `exchange`;
    - `:346-372`, `McpError::job_ids`, and its doc at `:340-345`.
  - `src/error.rs:304-307`.
  - §10 in `cloud-generation.md:836`, `:845` and `:848`.
- **Problem:**
  - 401 becomes `Unauthorized`, 403 becomes `Forbidden`, and a 404 with a session becomes
    `SessionExpired`. None of them reads the body. Only the catch-all arm calls `excerpt()`.
  - A paid call is not resent: `exchange` turns a 401 or a session 404 into `NotAccepted`.
    These variants carry no ids, so `submit_disposition` returns Rejected ("Press Run
    again").
  - For a cost check, `quote()` releases the tool (`provider/higgsfield/mod.rs:308-317`).
  - The unread body is not lost. Dropping it marks the read as abandoned, and the late sink
    queues any answer that has new ids (`mcp/mod.rs:1451-1472`). So the caller hears
    Rejected while the LateQueue holds a job id for the same run.
  - §10 (`:848`) says any failure whose answer names a job id is Outcome unknown, never
    Rejected, "4xx as well as 5xx".
    - The 401 and session-404 rows (`:836`, `:845`) do not say their bodies go unread.
    - 403 has no exception at all.
    - The `job_ids` doc also says "4xx or 5xx".
  - `every_byte_of_an_answer_is_scanned_for_job_ids` (`tests/mcp_fake.rs:1678`) covers only
    409 and 5xx.
- **Why minor:**
  - Nothing is resent.
  - Auth and session refusals come before any tool runs.
  - Higgsfield is stateless, so a session 404 cannot happen there.
  - The late copy keeps the ids.
- **Fix:** option (a) matches the owner's rule.
  - (a) For the Paid and Preflight classes, read up to `MAX_ERROR_SCAN_BYTES` of a 401, 403
    or session-404 body, and keep the ids.
    - When `new_ids` finds any, return an error that carries them, for example `Http` with
      `job_ids`. The result is then Outcome unknown, or StartedRun for a cost check.
    - Never resend.
    - Tests:
      - a paid 401 and a paid session 404 whose body names JOB end as
        `OutcomeUnknown { job_ids: [JOB] }`, with exactly one `generate_image`;
      - a cost check's 403 that names JOB is StartedRun and blocks the tool.
  - (b) Otherwise, write in §10 and in the `job_ids` doc that these three bodies are not
    read, and why. Then the worker must also accept late ids for a run it marked Rejected.

### R36 (minor). The record scrubber drops the blank line that ends each SSE event

- **Where:** `crates/freecut-cloud/examples/record.rs:897-909` (`Scrubber::string`).
- **Problem:**
  - A string with a `data:` line is rebuilt with `s.lines()...join("\n")`. `lines()` drops
    the last line ending and turns `\r\n` into `\n`.
  - `event: message\ndata: X\n\n` becomes `event: message\ndata: X\n`. A second pass
    (`--filter-headers`, N6) gives `...data: X`.
  - FreeCut's `SseParser` dispatches an event only at a blank line. Its test
    `an_unterminated_event_is_not_dispatched` checks this. So replaying a recorded SSE
    answer through the transport would drop the response.
  - The tests check only `data:` payloads (`data_lines`, `record.rs:1173`), so none catches
    this.
  - It affects the recording the owner is about to make. A later fix cannot restore the lost
    framing.
- **Why minor:** nothing leaks and no paid path is involved. Nothing replays recorded SSE
  framing yet.
- **Fix:**
  - Walk the body with `split_inclusive('\n')`. Scrub the content before each terminator,
    then put the original terminator back.
  - Tests:
    - an SSE body with nothing to scrub comes back byte for byte, after one pass and after
      two;
    - a scrubbed body still gives exactly one event through `SseParser`.

### R37 (minor). A cancel after a broken event stream drops the ids already read

- **Where:**
  - In `crates/freecut-cloud/src/mcp/mod.rs`:
    - `:306`, where `Cancelled` has no ids;
    - `:383-398`, where `with_answer` and `with_job_ids` skip `Cancelled`;
    - `:1202`, `:1234-1238` and `:1350-1352`, the points that return `Cancelled`.
  - `src/http.rs:477`.
- **Problem:**
  - Since R16, a broken stream that has an event id is resumed. `ChannelReader` records
    Finished when it hands over the error (`http.rs:477`), so no late copy of that stream
    is queued.
  - The user may cancel during the resume wait (`:1234-1238`) or the resume GET
    (`:1350-1352`).
    - The call then ends as `Cancelled { MaybeSent }`, and `with_answer` drops the ids in
      `read`.
    - The result is Outcome unknown with no ids. The ids are in neither the result nor the
      LateQueue.
  - The same can happen without an event id, when a cancel and a transport error land in the
    same wait slice.
    - `http.rs:477` records Finished. Then the `Err(_) if cancel.is_cancelled()` arm
      (`:1202`) returns Cancelled.
    - Before 72fc8ab, the drop recorded Abandoned and the copy was queued.
    - The helper's own cancel check (`http.rs:426-430`) usually wins this race.
  - Round 3 dismissed the clean-EOF form of the first case. R16 widened it to cut streams,
    and the second case is new in 72fc8ab.
- **Why minor:**
  - Nothing is resent, and the result still fails closed.
  - Only ledger ids are lost, in a rare timing window.
  - The first case needs event ids, and Higgsfield sends none.
- **Fix:**
  - Carry ids on `Cancelled`, for example `Cancelled { send, job_ids }`. Handle it in
    `with_job_ids` and `job_ids()`.
  - In `ChannelReader`, record Abandoned, not Finished, when the cancel token has fired.
  - Test: a cut stream with `id: ev-1` and a progress event naming JOB is cancelled during
    the resume wait. It ends as Outcome unknown with JOB.

### R38 (minor). SSE framing lines are scanned as job ids

- **Where:** in `crates/freecut-cloud/src/mcp/mod.rs`:
  - `:1243-1245`, where raw chunks go into `read`;
  - `:1144`, `ids(&read)`;
  - `:384`, `with_answer`;
  - `:1517`, `late_result`.
- **Problem:**
  - The R12 scan reads the raw SSE bytes, framing included. An `id:` line that holds a UUID
    counts as a job id. The scan has no token boundaries, so `id: <uuid>_<n>` counts too.
  - `new_ids` removes only the request's own UUIDs.
  - Some servers put a UUID in their event ids; the reference SDK's event store does. On such
    a server:
    - every readable cost answer becomes StartedRun and blocks the tool until restart
      (`provider/higgsfield/mod.rs:321-326`);
    - every paid refusal becomes Outcome unknown with a false id;
    - successes carry false ids.
  - The tests use event ids like `ev-1`, so none catches this.
- **Why minor:** it fails closed, and Higgsfield sends no `id:` lines (§15.1). It is latent,
  but it hits exactly the servers R16 was written for.
- **Fix:**
  - Scan only event data: every `data:` field the parser saw, dispatched or not, plus an
    unterminated tail.
  - Or drop the lines that start with `id:`, `event:`, `retry:` or `:` before the scan.
  - Do the same in `late_result` for event-stream bodies.
  - Tests:
    - an SSE cost answer with UUID event ids is a Quote;
    - a refusal on such a stream stays Rejected;
    - the existing progress-event case still finds its job.

### R39 (minor). A successful paid answer still counts the request's own UUIDs

- **Where:**
  - `crates/freecut-cloud/src/mcp/mod.rs:515-526` (`ToolOutput::job_ids`) and `:825-840`
    (`call_paid`).
  - The H2 obligations at `cloud-generation.md:975-1003`.
- **Problem:**
  - R14 filters three kinds of answer:
    - failures (`error.rs:278`);
    - cost answers (`provider/higgsfield/mod.rs:323`);
    - late answers (`mcp/mod.rs:1458`).
  - A successful paid answer that arrives in time is not filtered. `ToolOutput::job_ids()`
    cannot see the arguments, and `call_paid` returns the output as it is.
  - So the same answer drops a media id if it arrives late, and keeps it if it arrives in
    time.
  - A Phase B `submit()` that uses the obvious method would record an echoed `medias[].value`
    as a job. The poller would then poll a media id.
  - R14 asked for this before Phase B adds media references. No H2 obligation names it.
- **Why minor:** no code reads job ids from a successful paid answer yet.
- **Fix:**
  - Add `ToolOutput::new_job_ids(sent)`, or have `call_paid` return ids already filtered by
    `guard::new_ids`.
  - Add the rule to the H2 obligations.
  - Test: a successful paid answer that echoes a media id lists only the new job.

### R40 (minor). The UUID scan skips windows that overlap a match

- **Where:**
  - `crates/freecut-cloud/src/guard.rs:139-153` (`uuid_spans`, with `i += UUID_LEN` at
    `:146`).
  - Used by `uuids_in_text` (`:126`) and by the record scrubber (`examples/record.rs:955`).
- **Problem:**
  - After a match, the scan jumps 36 bytes.
  - Take a 28-35 byte UUID-like prefix glued to a real id, such as
    `ab12cd34-0000-1111-2222-3333` followed by a UUID. The scan finds one false window and
    never finds the real id.
  - The doc of `uuids_in_text` (`:122-125`) says "A window is tried at every byte". R13
    asked for that, and the code does not do it.
  - The ledger would get a wrong id. The scrubber would leave the last 28 characters of the
    real id in a fixture.
- **Why minor:** the input shape is contrived, and any match still fails closed. The votes
  split between nit and minor. It stays minor because R13 asked for every byte and the doc
  claims it.
- **Fix:**
  - In `uuids_in_text`, test every byte, also after a match.
  - In the scrubber, replace the union of overlapping windows.
  - Add the case above to `uuids_glued_to_hyphens_or_hex_letters_are_found` (`guard.rs:413`)
    and to the record test.

### R41 (minor). CI never compiles `freecut-cloud` without test-support

- **Where:**
  - `crates/freecut-cloud/Cargo.toml:48`, a self dev-dependency with `test-support`.
  - `.github/workflows/ci.yml:44-51`.
- **Problem:**
  - The workspace uses resolver 3. Any build of tests or all targets unifies the
    dev-dependency's features into the crate.
    - With dev edges, `cargo tree` shows `freecut-cloud [default,test-support]`.
    - With `-e normal` it shows `[default]`.
  - This round's runs confirm it: `--features http` and `--features http,test-support` built
    the same test binaries, with the same hashes.
  - Every CI step that builds the crate either builds dev targets or names the feature. So
    the shipping configuration is never compiled or linted. That is the one with `fake` and
    `provider::scripted` gated out (`lib.rs:41`, `provider/mod.rs:6`).
  - Before f4466cf, the workspace steps did build that configuration.
  - Once the app depends on freecut-cloud, app code that names `fake` or `scripted` would
    pass CI. It would also pass `deps_guard`, which reads the dependency graph, not the code.
    Only a release `cargo build -p freecut-app` would fail.
- **Why minor:**
  - It fails closed, as a build error.
  - The app does not depend on the crate yet.
  - This review did not compile the shipping configuration, because `cargo check` was
    outside its allowed commands.
- **Fix:**
  - Add a CI step that builds without dev targets:
    `cargo clippy -p freecut-cloud --features http -- -D warnings` (no `--all-targets`).
  - Once the app depends on the crate, add
    `cargo clippy -p freecut-app --all-features -- -D warnings`.
  - Or replace the self dev-dependency with `required-features = ["test-support"]` on the
    cloud test targets.
  - Land this with Phase B step 2.

### R42 (minor). `persist_unsaved()` returns Ok while a refresh is in flight

- **Where:**
  - In `crates/freecut-cloud/src/oauth/mod.rs`: `:839-849` (`persist_unsaved`) and
    `:916-968` (`refresh`).
  - The obligation at `cloud-generation.md:999-1000`.
- **Problem:**
  - `persist_unsaved` takes only `writing`. `refresh()` holds `refreshing` across up to 30 s
    of network calls (`CallLimits::OAUTH`).
  - Suppose a refresh has sent rt-N, and the worker quits.
    - `persist_unsaved` returns Ok: either nothing is unsaved, or it saves rt-N again.
    - The process exits, rt-N+1 is lost, and the next start sends rt-N.
  - FreeCut can cause this itself. `close()` and `notify_cancelled` call `bearer()` with a
    token that cannot be cancelled, and that can start a refresh during quit.
  - The obligation says to warn the user only when `persist_unsaved` fails. Here they are
    told nothing.
  - This differs from round 3's dismissed "refresh whose answer is lost". There, the network
    loses the answer. Here, FreeCut drops it and then reports a successful save.
- **Why minor:** at worst the user signs in again. No money is involved, and the worker is
  not written yet.
- **Fix:**
  - In `persist_unsaved`, `try_lock` `refreshing`, and return a distinct "refresh in flight"
    error. The worker can then wait for a bounded time, or warn.
  - Or change the obligation: call it only after in-flight helper calls, `close()` included,
    have returned.
  - Test: `persist_unsaved` does not return Ok while a refresh holds `refreshing`.

### R43 (minor). The final secret check can miss a secret that contains another

- **Where:** in `crates/freecut-cloud/examples/record.rs`:
  - `:727-745`, `replace_known`;
  - `:542-549`, `leaks`;
  - `:490-504`, `Known::forms`;
  - `:510`, `known_secrets`.
- **Problem:**
  - Secrets are replaced one after another, in list order: client id, state, verifier, code,
    access, refresh, ID token, e-mail, sub. Forms are sorted longest first only within one
    secret.
  - An earlier, shorter secret may sit inside a later, longer one. It is then replaced first,
    and the rest of the longer secret is written. `leaks()` still passes, because it looks
    only for whole forms.
  - Percent forms come only from `byte_serialize`, which writes upper-case hex. A server
    echo in lower case, such as `%2b`, is neither replaced nor detected.
- **Why minor:**
  - Clerk values are random base64url, opaque strings or JWTs, so nesting is not expected.
  - Secret-named fields are redacted anyway.
  - Still, it weakens R22's promise that the file is not written while any secret remains.
- **Fix:**
  - Before replacing, build one list of (form, label) pairs across all secrets, sorted
    longest first.
  - In `leaks()`, compare percent forms without regard to case.
  - Also in `leaks()`, fail on any 12-byte window of a secret that is 16 bytes or longer.
  - Test: a client id planted inside the access token.

### R44 (minor, docs). §6.1, §14 H1 and §15.2 show signatures the code no longer has

- **Where:** `docs/architecture/cloud-generation.md:529-531`, `:934`, `:502` and `:1156`.
- **Problem:**
  - `:529-531` says two things that are no longer true:
    - That the cost check goes through `check_cost(tool, &QuoteRequest, cancel)`. The code
      has `check_cost(tool, &QuoteRequest, run, cancel)` (`mcp/mod.rs:849`).
    - That paid tools go through `call_tool`. `call_tool` refuses tools that are not free
      with `NeedsRun` (`:807-817`). Paid tools use `call_paid(tool, arguments, run, cancel)`
      (`:825`).
  - `:934`, the H1 contracts, lists only `McpClient::{initialize, list_tools, call_tool}`.
  - `:502`: the planned `submit` has no `run: &RunTag`, but `:962-964` says every `quote` and
    `submit` carries one.
  - `:1156` says that `quote` "takes the `Operation` alone".
  - The `guard` and `late` parameters of `HiggsfieldProvider::new` and `with_endpoint` are
    not named anywhere.
- **Fix:**
  - `:529-531`: "...through `McpClient::check_cost(tool, &QuoteRequest, run, cancel)`...;
    free tools go through `McpClient::call_tool`, and paid tools only through
    `McpClient::call_paid(tool, arguments, run, cancel)`, each with an
    `&'static AllowedTool` (§6.3)".
  - `:934`: `McpClient::{initialize, list_tools, call_tool, call_paid, check_cost}`. Add
    `HiggsfieldProvider::new(..., guard, late)`.
  - `:502`: add `run: &RunTag` to `submit`.
  - `:1156`: "`quote` takes the `Operation` and the caller's `RunTag`".

### R45 (minor, docs). §13 and §15.2 still describe the record tool as first planned

- **Where:** `cloud-generation.md:910` (the §13 "Recorded fixtures" row) and `:1239-1241`
  (the §15.2 "Next" bullet).
- **Problem:**
  - `:910` says `--example record` produces the `get_cost` answers and the paid image and
    video runs.
    - The tool refuses any body that names `get_cost`, and every call except a
      `models_explore` list (R25, N4).
    - The `get_cost` fixtures came from the probes (fixtures README).
    - A reader of §13 could extend `record` to make paid runs.
  - `:1239-1241` says Phase B starts with the record run that records the open transport
    facts. That run has happened (the signed-in rows of §15.1). The rest of "Still unknown"
    needs paid runs.
- **Fix:**
  - §13: say that the `get_cost` answers came from the probes. Say that paid-run recordings
    need a separate tool the owner approves. `record` cannot make them, on purpose.
  - §15.2: replace "Next" with what is left: the owner's re-run after R21-R24 and R36, its
    review, then the Phase B order.

### Nits

- **R46. Resuming after the client's own timeout restarts the total limit**
  (`mcp/mod.rs:1210`, `:1328`).
  - The R16 arm resumes after any read error that is not a cancel, the ureq global timeout
    included. Each GET reuses `resume.limits`.
  - So a paid call can take about 6 x 120 s plus the delays, instead of the 120 s in §4.5.
  - There is no money risk, cancel still works, and Higgsfield sends no event ids.
  - Fix: keep one deadline across resumes, or do not resume after a timeout. State the bound
    in §4.1 (`cloud-generation.md:239-243`).
- **R47. Stale `LateToolResult` rustdoc** (`mcp/mod.rs:401-406`).
  - It still says the worker drains `McpClient::take_late_results`.
  - Write: "the app's `LateQueue` (`LateQueue::take`) (R9, R10)".
- **R48. §4.4 understates where late ids go** (`cloud-generation.md:296-298`).
  - It says only "job ids from a paid submission are still recorded (§7.2)". Cost checks have
    the late route too, and §7.2 says nothing about late answers.
  - Write: "job ids from a paid call or a cost check go to the app's `LateQueue`, tagged with
    the caller's `RunTag` and call class, and the worker writes them to the ledger (§14 H2
    obligations, §15.2)".
- **R49. "Still unknown" could name more** (`cloud-generation.md:1134-1141`). Optional
  additions:
  - the `media_upload` and `media_confirm` answers, and the presigned upload (they need an
    approved upload, in H2);
  - the refresh-token lifetime.
- **R50. The deps guard has no positive control** (`crates/freecut-app/tests/deps_guard.rs:346-360`).
  - If `--format` were dropped, no line would carry a feature list, and the guard would pass
    without checking anything.
  - Repeated lines end in `] (*)`, so the last feature parses as `x] (*)`.
  - Fix: assert that the `freecut-audio` line lists `record`, which is R20's own proof case.
    Strip a trailing ` (*)`, or pass `--no-dedupe`.
- **R51. `bearer()`'s `try_lock` gives up for good on a poisoned mutex**
  (`oauth/mod.rs:1005-1006`).
  - Every other lock in the file ignores poison.
  - Fix: take the guard on `TryLockError::Poisoned(p)` with `p.into_inner()`, and skip only
    on `WouldBlock`.
- **R52. Numbers under secret field names are redacted** (`record.rs:853-858`).
  - `code` is a secret field. So a JSON-RPC `{"error":{"code":-32601}}` is recorded as
    `"<redacted>"`, and `jsonrpc::parse` would read 0 on replay.
  - R23 asked for strings or numbers, so keep that rule.
  - Fix: keep a numeric `code` inside an `error` object, or inside any object that has a
    `message` sibling. Test: a recorded -32602 keeps its code.
- **R53. The record gate keeps the last of a duplicate key** (`record.rs:576-620`).
  - Hand-made bytes with two `name` keys can pass as `models_explore`, while a server that
    keeps the first key sees `generate_image`.
  - FreeCut never writes duplicate keys.
  - Optional fix: at ENDPOINT, refuse a body unless `serde_json::to_vec(&parsed) == body`.
- **R54. The callback listener's timeout applies to each read** (`record.rs:384-392`).
  - A local process that sends one byte every 4.9 s can hold the listener for hours, past
    `SIGN_IN_WAIT`.
  - Fix: give each connection a total deadline of 10 s. Before each read, set the read
    timeout to the time left. Bound the 64 KiB drain by time too.
- **R55. `plain_endpoint` runs after `oauth.register`** (`record.rs:119-120`).
  - A refused endpoint has already cost one more Clerk client.
  - Fix: call it right after `discover`.
- **R56. `ZeroingBody::read` zeroes too early** (`record.rs:708-715`).
  - It wipes the data whenever `n == 0`. That includes a read with an empty buffer while
    bytes remain.
  - Every current reader passes a non-empty buffer.
  - Fix: zero only when `self.pos == self.data.len()`, and on drop.
- **R57. The fixtures README scrub rules miss R22**
  (`tests/fixtures/higgsfield/README.md:47-56`).
  - They do not say that every secret held in memory is replaced wherever it appears, or
    that the file is not written if any remains.
  - Fix: copy the sentence from `record.rs:18-23`.

## Checked and dismissed

The second pass rejected these. Do not change code for them.

- **`bearer()` writes to the store and warns on every call while the store refuses**
  (`oauth/mod.rs:1005-1009`, `:827`).
  - R18 asked for exactly this, and §10 documents it.
  - `credential()` already reads the store on every call. A keychain that prompts would
    prompt anyway, so this is a design point for the Phase B keychain store.
  - Optional polish: warn once, then log at debug.
- **`HiggsfieldProvider::with_endpoint` is public and not gated.**
  - `McpClient::new` is public too, and takes any endpoint.
  - The app chooses the endpoint, and R10's fix names `with_endpoint`.
  - Gating it would not change what app code can do.
- **R53's "any `result` reply passes".** R25 asked for this, and a reply cannot start a tool.
- Round 3's dismissed list still stands. R37 is not that item reopened (see R37).

## Gates

Everything ran offline, in `../FreeCut-higgsfield` at 45e4bb1 with its own `target/`. This
pass re-ran each command, and the counts match the lanes.

- **`cargo test -p freecut-cloud --features http`:** 226 pass, 0 fail.
  - 113 unit, 13 catalogue_and_golden, 8 loopback_http, 47 mcp_fake, 23 oauth_flow, 21 record
    example tests and 1 doctest.
  - Round 3 had 202. The new tests are 1 unit, 1 catalogue, 1 loopback, 6 mcp_fake,
    2 oauth_flow and 13 record.
- **`cargo test -p freecut-cloud --features http,test-support`:** the same 226 pass, from the
  same test binaries (see R41).
- **`cargo test -p freecut-cloud --features http --example record`:** 21 passed, 0 failed.
- **`cargo clippy -p freecut-cloud --all-targets --features http,test-support -- -D warnings`:**
  clean.
- **`cargo test -p freecut-app --test deps_guard`:** 10 passed, 0 failed.
- **`cargo fmt --all --check`:** clean (one lane).
- **The R17, R18, R19 and N1 tests:** stable over 3 runs in a row (one lane).
- **The guard's own `cargo tree` command, run by hand** (read-only, `--offline --locked`): it
  shows `freecut-audio ... [media,output,record]`. So R20's proof case is now visible to the
  guard.
- **Not run:**
  - the workspace clippy and test;
  - a build without test-support (that is R41);
  - `cargo run`, and any run of `record`.

## Is the record tool safe for the owner to run now?

**Yes, on macOS, for money and for secrets.**

- **What it can send:**
  - discovery, one registration and the code exchange;
  - `initialize`, `notifications/initialized` and `tools/list`;
  - `models_explore` with `action: "list"`.

  Everything else is refused before sending (R25, N4): `get_cost` in any spelling, every
  other tool and action, and every other method. Unknown arguments stop the tool before
  anything is built (N5).
- **What it writes:**
  - only allowed headers;
  - e-mails, JWTs and UUIDs replaced wherever they appear, in object keys too;
  - the allowance replaced with fake values.

  Every secret the tool holds is replaced in raw, percent-encoded and JSON-escaped form, and
  the file is not written if one remains (R22; R43 is a latent gap). Buffered token answers
  are zeroed once read (N3).
- Each run registers one more Clerk client. After this run there will be three.
- **Two caveats:**
  1. The output file is still tracked (R34). After the run, `git status` shows it as
     modified. Until the review is done, nobody runs `git add`, `git commit -a`,
     `git stash`, `git checkout` or `git restore` on it, or a rebase. `git rm --cached` is
     safe.
  2. R36: each SSE body in the new file loses its final blank line. This affects fidelity
     only. For a faithful fixture, run after R36 lands. If the owner has already run it, the
     file can still be reviewed, and the review decides whether they record again.
- At review time the working copy matched HEAD (`git status` was clean).

## Message for the agent

Round 4 of the Higgsfield review is done. The verdict is **approve with changes**. The full
notes are in `docs/handoff/higgsfield-review-round4.md`.

Thank you. All 30 commits do what they say, and each has a test. R10, R11, R13-R33, N1 and
N3-N7 are fixed. The owner accepted your R12 rule: a 4xx body that names a job is Outcome
unknown. R12 and N2 are only partly done. Phase B may start now.

Do this first, in its own commit:

1. **R34.** The R10 commit added `recorded/free-calls.jsonl`. The file is still tracked, so
   the `.gitignore` rule does nothing. Run `git rm --cached` on that one path, and commit.
   Do not touch the working copy, because the owner may be recording into it. Do not rewrite
   history unless the owner asks.

Then fix these. Each fix needs a test that fails before the fix.

Money path:

2. **R35.** On paid calls and cost checks, read and scan 401, 403 and session-404 bodies for
   job ids, like any other 4xx. A body that names a job is Outcome unknown, never Rejected.
   If you choose not to do this, say so in §10.
3. **R37.** A cancel after a broken event stream drops the ids already read. Carry ids on
   `Cancelled`. In `ChannelReader`, record Abandoned, not Finished, when the cancel has
   fired.
4. **R38.** Do not count SSE `id:` lines as job ids. Scan only event data, both in the answer
   path and in `late_result`.
5. **R39.** Filter a successful paid answer with `guard::new_ids` too. Add this to the H2
   obligations.
6. **R40.** In `uuids_in_text`, try a window at every byte, also after a match.

Tokens and CI:

7. **R41.** Add a CI step that builds freecut-cloud without dev targets:
   `cargo clippy -p freecut-cloud --features http -- -D warnings`.
8. **R42.** `persist_unsaved()` must not return Ok while a refresh is in flight. Return a
   distinct error, or change the obligation.

Record tool:

9. **R36.** Keep SSE line endings in `Scrubber::string`, using `split_inclusive('\n')`. An
   SSE body with nothing to scrub must come back byte for byte.
10. **R43.** Replace all known secrets longest first, across all of them. Make `leaks()`
    catch partial remains and lower-case percent forms.

Docs:

11. **R44.** Fix the old signatures in §6.1 (`:502`, `:529-531`), §14 H1 (`:934`) and §15.2
    (`:1156`).
12. **R45.** Fix the §13 "Recorded fixtures" row and the §15.2 "Next" bullet.
13. Fix the nits R46-R57 when convenient.

About the recording:

- The owner will run `record` once more on macOS, and they sign in. It is best to land R36
  first.
- The new file comes back for review. Commit it only after the review, with `git add -f`.
- Until then, do not run `git add`, `git commit -a`, `git stash`, `git checkout` or
  `git restore` on that path. Do not rebase either.

Phase B may start now, in parallel with these fixes. Follow this order:

1. The record tool is done. The new recording is committed only after review.
2. Land R41 before the app depends on freecut-cloud (R20 is already in). Then add the app's
   `cloud` feature. It stays off until the user connects.
3. Next come the sign-in UI (Preferences > Cloud generation), the keychain `SecretStore`, the
   loopback listener and the worker thread. Use one PreflightGuard, one TokenManager and one
   LateQueue per app run (R6, R17, R10).
4. Then the Generate tab and the run sheet with cost checks. The catalogue reaches the UI
   only through the reviewed list (N1). R35, R37, R38 and R40 must be in first.
5. Then paid submit, the Runs page and ledger, import, and Place. R9, R11, R15, R18, R39 and
   R42 are the acceptance criteria for this worker.
6. Put `model: cloud provenance` in one separate commit. Do not change
   `CURRENT_FORMAT_VERSION`.

The money rules have not changed:

- Never retry or resend a paid call.
- Anything that may have reached the server without a readable answer is Outcome unknown.
- Send cost checks only with `get_cost: true`, and never retry one that may have been sent.
- No test or tool makes a paid call or a cost check against the real service.
- Tokens never go into logs, files or fixtures.

Never push, and never merge. The owner does that. Run fmt, clippy and the cloud tests before
you report.
