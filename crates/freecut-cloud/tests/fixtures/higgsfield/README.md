# Higgsfield fixtures

Recorded 2026-10-14 for slice H1 of `docs/architecture/cloud-generation.md`
(§13, §15). **Free calls only**: public discovery documents, one dynamic
client registration, `models_explore` listings, and `get_cost: true` cost
checks. Nothing was generated, uploaded or charged; no job ids exist.

| File | Source | Changes |
|---|---|---|
| `www-authenticate.txt` | `POST /mcp` without a token → `401` | none |
| `oauth-protected-resource-mcp.json` | `GET https://mcp.higgsfield.ai/.well-known/oauth-protected-resource/mcp` | none |
| `oauth-authorization-server-clerk.json` | `GET https://clerk.higgsfield.ai/.well-known/oauth-authorization-server` | none |
| `oauth-authorization-server-mcp.json` | `GET https://mcp.higgsfield.ai/.well-known/oauth-authorization-server` (no longer listed by the resource metadata) | none |
| `oauth-register-clerk-201.json` | `POST https://clerk.higgsfield.ai/oauth/register` with FreeCut's §5.2 body → `201` | `client_id` replaced by `FAKE_CLIENT_ID` |
| `models_explore-list-{image,video,audio}.json` | `models_explore {action: list, type, limit: 100}` through the user's connected MCP client | excerpts: image 11 of 45 items, video 17 of 57 (entries verbatim), audio 8 of 8; some long descriptions and option lists shortened in image and audio |
| `models_explore-get-gpt_image_2_5.json` | `models_explore {action: get}` | none |
| `get_cost-generate_image.json` | `generate_image {model: gpt_image_2_5, prompt, aspect_ratio: 1:1, get_cost: true}`; the same answer for `count: 2` | none |
| `get_cost-generate_video.json` | `generate_video {model: seedance_2_5, mode: t2v, 16:9, 5 s, 720p, get_cost: true}` | none |
| `tool-error-unknown-model.txt` | `generate_image {model: freecut_no_such_model, get_cost: true}` → tool error | none |
| `tools-list-schemas.json` | input schemas of four tools as the connected client presented them | descriptions removed; transcribed, not a raw `tools/list` capture |

The MCP replies came through a connected client, not FreeCut's own
transport, so they show the tool results' JSON, not the HTTP framing
(content type, SSE or not, session headers). Those are recorded once a
signed-in FreeCut client exists (see §15 of the architecture doc).

Tokens, e-mails, subject ids and signed URLs: none are present.

## Recorded by FreeCut itself (`recorded/`)

`recorded/free-calls.jsonl` is written by

```text
cargo run -p freecut-cloud --features http --example record
```

which the owner runs once and signs in to in the browser. It records
FreeCut's own HTTP exchanges, so it shows what the excerpts above cannot:
content types, SSE or JSON answers, session and protocol headers. Free calls
only: discovery, one client registration, the code exchange, `initialize`,
`tools/list`, the `models_explore` listings and the session `DELETE`. The
tool's transport refuses any paid tool and anything carrying `get_cost`
before it is sent.

Before writing, the tool keeps only an allow list of headers (`content-type`,
`cache-control`, `date`, `mcp-session-id`, `mcp-protocol-version`,
`www-authenticate`, `x-higgsfield-build`; no `Authorization`, cookies, trace
ids or CDN headers), replaces secret fields (tokens, codes, client id) and the
client id wherever else it appears,
and replaces e-mail addresses, JWTs and UUIDs with stable fakes; session ids
become `fake-session-N` and signed URLs lose their query. Review the file
before committing it. Recorded once by the owner on 2026-10-14 and filtered
with `--filter-headers`; it is committed only once the reviewer approves it.

