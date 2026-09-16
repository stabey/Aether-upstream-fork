# Cursor provider behavior

Codex is the mature in-Aether provider: the Rust gateway talks HTTP in-process
because ChatGPT Codex already exposes an HTTP Responses API. Cursor is different.
The official Agent harness is Node [`@cursor/sdk`](https://www.npmjs.com/package/@cursor/sdk).
It cannot be linked into `aether-gateway`.

## Can the Rust process embed `@cursor/sdk`?

No, not in this deployment:

- `@cursor/sdk` is a Node package (Agent.create / send / stream).
- Production `Dockerfile.app` is distroless/static: only `aether-gateway`.
- Compose runs the app container `read_only` with `noexec` `/tmp`, so the
  gateway cannot spawn Node the way Windsurf optionally starts a local LS.

## Same-image interaction (this is the supported path)

Follow `aether-vscodex`: keep Node next to the gateway, not inside the binary.

```text
Client → aether-gateway (Rust, same as Codex) → aether-cursor-sdk → @cursor/sdk → Cursor
```

| Process | Image | Role |
| --- | --- | --- |
| `aether-gateway` | distroless | Pool, routing, conversion, Bearer Cursor keys |
| `cursor-sdk` | `node:22` | Official SDK Agent harness |

```bash
docker compose -f docker-compose.yml -f docker-compose.cursor-sdk.yml up -d
```

The overlay sets `AETHER_CURSOR_SDK_INTERNAL_URL=http://cursor-sdk:8792/v1`.
Loopback template URLs (`http://127.0.0.1:8792/v1`) are rewritten to that
internal URL at request time, the same way Aether talks to vscodex.

Local without Docker: `cd aether-cursor-sdk && npm ci && npm start`, then create
a `cursor` provider (default base URL is already the sidecar).

You can still point the provider at an external
[cursor-sdk2api](https://github.com/Sunnyender-org/cursor-sdk2api) or
[Cursor2API](https://github.com/NGLSG/Cursor2API) gateway. Explicit custom
base URLs are preserved.

## Codex-style Aether behavior

- Same-format HTTP, not a dedicated protobuf client.
- Native formats: `openai:chat`, `openai:responses`, `claude:messages`.
  Other clients use Aether's existing conversion matrix.
- Keys are Cursor User API Keys (`crsr_…`), forwarded as Bearer (Codex's
  API-key channel, not ChatGPT OAuth).
- Live `GET /v1/models` against the sidecar, with SDK identity headers
  (`x-cursor-client-type: sdk`).
- Format conversion is on by default.

## Setup

1. Start the sidecar (Compose overlay or local `npm start`).
2. Create provider type `cursor`.
3. Add Cursor User API Keys as provider keys.
4. Fetch models from the sidecar, or use presets (`composer-2.5`,
   `composer-2.5-fast`, `claude-sonnet-4-6`, `claude-opus-4-6`, `grok-4.6`).

## Out of scope

- Linking Node into the Rust binary
- Re-implementing Cursor2API's private Connect+protobuf chat in Rust
- Cursor dashboard quota RPC (use Cursor / sidecar for account health)
- Full tool-bridge parity with cursor-sdk2api (v1 flattens the turn to
  Agent.send text)

## Operational notes

Follow Cursor's terms. This is an integration surface, not a bypass of
account limits. If the sidecar is down, Aether returns ordinary upstream
connection errors.
