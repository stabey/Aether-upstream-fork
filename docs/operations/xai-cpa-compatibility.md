# xAI compatibility with CLIProxyAPI

The xAI provider was reviewed against CLIProxyAPI commit
[`7fac6b15bcfe5ea55c18c9eaec8e5b7e6457d974`](https://github.com/router-for-me/CLIProxyAPI/tree/7fac6b15bcfe5ea55c18c9eaec8e5b7e6457d974).
The following rules preserve the provider-specific behavior across Aether's request,
transport, and local video-task layers.

## Responses and tools

- HTTP requests keep CPA's removal of `previous_response_id`. Clients must supply
  conversation history; this change does not add an HTTP response-ID history store.
- Preserve requested `reasoning.encrypted_content`. On a native Responses-to-Responses
  hop, keep provider-owned input items instead of rebuilding them through the canonical
  format. xAI encrypted reasoning may have IDs that do not use OpenAI's `rs` prefix.
  Aether's Gemini signature carriers remain excluded from xAI replay.
- The replay policy is selected from the configured provider type. A model called
  `grok-*` on another provider does not opt into that policy. WebSocket continuation
  metadata retains the selected policy across reconnects.
- A regular client function called `web_search` remains a function. Claude hosted
  search choices are resolved against the original typed tool declaration, including
  declarations with a different name.
- When only `image_generation` is allowed, keep only that tool and retain the requested
  `auto` or `required` mode. For mixed allowed-tool lists, remove the image choice while
  preserving the other allowed entries, as required by xAI's tool-choice schema.

## Images and videos

OAuth media requests default to `https://cli-chat-proxy.grok.com/v1`; API-key or
`using_api=true` requests default to `https://api.x.ai/v1`. Explicit custom gateways
are preserved. Compact remains on the official endpoint. This follows CPA's
[`e4119f8`](https://github.com/router-for-me/CLIProxyAPI/commit/e4119f83b448988a47aad3fdd4d515788fc87c36)
media-routing fix. CLI identity headers are applied to media requests and restored
when a persisted video task's polling transport is reconstructed.

Aether's OpenAI-compatible task parser accepts xAI's `request_id` creation field,
status aliases such as `pending` and `done`, nested `video.url` and `video.duration`,
and failure payloads containing `code` / `error` without a status. Existing OpenAI
`id` takes precedence. The client receives Aether's local task ID; polling uses the
upstream task ID and selected credential. Completed video downloads use the returned
media URL without forwarding provider authentication headers to the media host.

### Public video protocols

The xAI provider supports both of CPA's video surfaces:

| Operation | xAI native | OpenAI compatible |
| --- | --- | --- |
| Create | `POST /v1/videos/generations` | `POST /openai/v1/videos` |
| Edit / extend | `POST /v1/videos/edits`, `POST /v1/videos/extensions` | — |
| Retrieve | `GET /v1/videos/{request_id}` | `GET /openai/v1/videos/{id}` |
| Download | use the returned `video.url` | `GET /openai/v1/videos/{id}/content` |

For xAI, `POST /v1/videos` is a native creation alias, matching CPA. Other
providers retain Aether's existing OpenAI-compatible `/v1/videos` behavior.
xAI callers using OpenAI `seconds` / `size` parameters must use
`/openai/v1/videos`. The adapter maps these to numeric `duration`,
`aspect_ratio`, and `resolution`; it also adapts image references. CPA's defaults
(4 seconds, portrait, 720p), duration clamp (1–15), and input validation apply.
Explicit native requests retain native parameters and additional provider fields.

Default xAI creation targets `/videos/generations` on the selected upstream host.
Explicit custom endpoint paths still take precedence. Native generation, editing,
and extension paths only select xAI provider candidates.

Native creation returns `request_id`; native retrieval preserves `done`, nested
`video.url`, and provider fields such as `respect_moderation`. The identifier is
an opaque Aether task ID so queries remain scoped to the owning user and pinned
to the original upstream task and credential. The explicit `/openai/v1/videos`
surface projects `id`, `completed`, and `video_url`.

The task row records the native client protocol as `xai:video`, while its provider
transport remains `openai:video`. This survives restart without storing request
bodies or credentials. Raw native responses are cached only in memory; after
reconstruction the gateway refreshes from the original provider to recover its
response fields, including for completed tasks. If refreshing is unavailable,
the stored task still provides the native status and media URL projection.

### Runtime configuration

Standalone Rust deployments must set
`AETHER_GATEWAY_VIDEO_TASK_TRUTH_SOURCE_MODE=rust-authoritative` and restart the
gateway to enable video task retrieval, polling, and content downloads. The CLI's
legacy default is `python-sync-report`: creation can return a task ID in that mode,
but the local task read/refresh paths are disabled and may return HTTP 503.

When the gateway also serves the frontend, `/openai/v1/videos` and its subpaths
must bypass the static SPA handler and be mounted as API routes. Otherwise a
successful-looking HTTP 200 response to a video query may contain `text/html`
instead of the task's JSON response. The lifecycle regression includes the static
frontend to cover this production configuration.

## Regression coverage

The format tests cover client and hosted search choices, image-only and mixed tool
restrictions, encrypted reasoning replay, and unchanged OpenAI replay restrictions.
Transport tests cover OAuth/API-key/custom routing and media identity headers.
Video-task tests exercise creation, polling, terminal projection, persistence fields,
content-download planning, and status-less errors using local fixtures. They do not
make paid generation requests.

The HTTP regression exercises all native creation paths and the compatibility
prefix through the public router and candidate planner, then checks polling,
cross-user denial, persistence, and retrieval from a fresh gateway instance.

```sh
cargo test -p aether-ai-formats -p aether-provider-transport -p aether-video-tasks-core --lib
cargo test -p aether-gateway --lib xai
```
