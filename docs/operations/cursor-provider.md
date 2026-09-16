# Cursor provider behavior

The `cursor` fixed provider integrates Cursor subscription models into Aether
through an external OpenAI / Anthropic-compatible SDK gateway. Aether does
**not** embed `@cursor/sdk` or reverse-engineer Cursor's private Agent
transport.

This design follows the same split used by:

- [cursor-sdk2api](https://github.com/Sunnyender-org/cursor-sdk2api)
- [Cursor2API](https://github.com/NGLSG/Cursor2API)

and matches how those projects integrate with gateways such as new-api: the
upper layer speaks standard HTTP APIs; the sidecar owns the official Cursor
Agent harness.

## Architecture

```text
Client  →  Aether (Rust)  →  cursor-sdk2api / Cursor2API  →  @cursor/sdk  →  Cursor
```

| Layer | Responsibility |
| --- | --- |
| Aether | Multi-tenant auth, key pool, routing, format conversion, usage |
| Sidecar | `@cursor/sdk` Agent create/send/stream, tool continuation |
| Credential | Cursor User API Key (`crsr_…`) stored as an Aether provider key |

## Setup

1. Deploy a sidecar (recommended: cursor-sdk2api) with `AUTH_MODE=byok`.
2. In Aether admin, create a provider with type `cursor`.
3. Default base URL is `http://127.0.0.1:8080/v1`. Override for Docker, for
   example `http://cursor-sdk2api:8080/v1`.
4. Add one or more Cursor User API Keys as provider keys (Bearer).
5. Associate models. Presets include `composer-2.5`, `composer-2.5-fast`,
   `claude-sonnet-4-6`, `claude-opus-4-6`, and `grok-4.6`. Prefer live
   `GET /v1/models` from the sidecar when available.

## Endpoints

The fixed template exposes:

| API format | Upstream path (relative to base `/v1`) |
| --- | --- |
| `openai:chat` | `/chat/completions` |
| `openai:responses` | `/responses` |
| `claude:messages` | `/messages` |

Cross-format clients still go through Aether's existing conversion matrix.
Format conversion is enabled by default.

## Auth and pooling

- Keys are **key-managed**, not OAuth accounts.
- Each Cursor User API Key is a pool member. Aether schedules them like other
  Bearer providers.
- Run the sidecar in BYOK mode so the Bearer token Aether forwards is the
  Cursor key itself.
- Managed-mode sidecars (one gateway key + internal Cursor pool) also work, but
  then Aether only sees a single upstream key and loses per-Cursor-key
  scheduling visibility.

## Out of scope (v1)

- Embedding `@cursor/sdk` inside the Rust gateway process
- Re-implementing Connect+protobuf Composer chat from Cursor2API's worker
- Cursor dashboard quota RPC refresh (use the sidecar console / Cursor
  dashboard for now)
- Cookie / CLI session scraping

## Operational notes

- Follow Cursor's terms of use. This provider is an integration surface, not a
  bypass of Cursor account limits.
- Tool-heavy clients (Claude Code, Codex, Grok Build) depend on the sidecar's
  tool bridge fidelity. Validate against the sidecar's own smoke tests before
  production traffic.
- If the sidecar is down, Aether surfaces ordinary upstream connection errors;
  there is no in-process Cursor fallback.
