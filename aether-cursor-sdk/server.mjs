# Aether in-image Cursor SDK sidecar.
#
# Codex talks HTTP in-process because ChatGPT already exposes an HTTP API.
# Cursor's official harness is Node `@cursor/sdk`. This process is that SDK;
# `aether-gateway` calls it over loopback / Compose DNS.

import http from "node:http";
import { mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Agent, Cursor } from "@cursor/sdk";
import {
  openaiChatChunk,
  promptFromRequest,
} from "./prompt.mjs";

const HOST = process.env.HOST?.trim() || "127.0.0.1";
const PORT = Number.parseInt(process.env.PORT || "8792", 10);
const WORKSPACE =
  process.env.AETHER_CURSOR_SDK_WORKSPACE?.trim() ||
  join(tmpdir(), "aether-cursor-sdk-workspace");
const MAX_JSON_BYTES = 2 * 1024 * 1024;
const RUN_TIMEOUT_MS = Number.parseInt(process.env.AETHER_CURSOR_SDK_RUN_TIMEOUT_MS || "180000", 10);

mkdirSync(WORKSPACE, { recursive: true });

function bearerToken(request) {
  const apiKey = header(request, "x-api-key");
  if (apiKey) return apiKey.trim();
  const authorization = header(request, "authorization") || "";
  const match = /^Bearer\s+(\S+)/i.exec(authorization);
  return match?.[1];
}

function header(request, name) {
  const value = request.headers[name] || request.headers[name.toLowerCase()];
  return typeof value === "string" ? value : Array.isArray(value) ? value[0] : "";
}

function openaiChatCompletion(id, model, text, usage) {
  return {
    id,
    object: "chat.completion",
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [
      {
        index: 0,
        message: { role: "assistant", content: text },
        finish_reason: "stop",
      },
    ],
    usage: usage ?? { prompt_tokens: 0, completion_tokens: 0, total_tokens: 0 },
  };
}

function asText(event) {
  if (!event || typeof event !== "object") return "";
  if (typeof event.text === "string") return event.text;
  const content = event.message?.content;
  if (!Array.isArray(content)) return "";
  return content
    .filter((block) => block.type === "text" && typeof block.text === "string")
    .map((block) => block.text)
    .join("");
}

async function runAgent(apiKey, model, prompt) {
  const agent = await Agent.create({
    apiKey,
    model: { id: model || "composer-2.5" },
    local: { cwd: WORKSPACE },
  });
  try {
    const run = await agent.send({ text: prompt });
    return { agent, run };
  } catch (error) {
    await agent.close?.();
    throw error;
  }
}

async function collectRun(run) {
  let text = "";
  let usage;
  for await (const event of run.stream()) {
    if (event?.type === "assistant") text += asText(event);
    if (event?.type === "usage" && event.usage) usage = event.usage;
  }
  const result = await run.wait().catch(() => null);
  if (!text && typeof result?.result === "string") text = result.result;
  return {
    text,
    usage: usage || result?.usage,
    status: result?.status,
    error: result?.error,
  };
}

function writeJson(response, payload, status = 200) {
  const body = JSON.stringify(payload);
  response.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  response.end(body);
}

function writeSse(response, payload) {
  response.write(`data: ${JSON.stringify(payload)}\n\n`);
}

async function readJson(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > MAX_JSON_BYTES) {
      const error = new Error("request body too large");
      error.status = 413;
      throw error;
    }
    chunks.push(chunk);
  }
  if (chunks.length === 0) return {};
  return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}

async function handleRequest(request, response) {
  const url = new URL(request.url || "/", `http://${request.headers.host || `${HOST}:${PORT}`}`);

  if (request.method === "GET" && (url.pathname === "/healthz" || url.pathname === "/health")) {
    writeJson(response, { ok: true });
    return;
  }

  if (request.method === "GET" && (url.pathname === "/v1/models" || url.pathname === "/models")) {
    const apiKey = bearerToken(request);
    if (!apiKey) {
      writeJson(response, { error: { message: "Missing Cursor API key", type: "authentication_error" } }, 401);
      return;
    }
    const models = await Cursor.models.list({ apiKey });
    const data = (Array.isArray(models) ? models : models?.models || []).map((model) => ({
      id: model.id || model,
      object: "model",
      owned_by: "cursor",
      display_name: model.displayName || model.id,
    }));
    writeJson(response, { object: "list", data });
    return;
  }

  const isChat = url.pathname === "/v1/chat/completions" || url.pathname === "/chat/completions";
  const isResponses =
    url.pathname === "/v1/responses" ||
    url.pathname === "/responses" ||
    url.pathname === "/v1/responses/compact";
  const isMessages = url.pathname === "/v1/messages" || url.pathname === "/messages";
  if (request.method !== "POST" || (!isChat && !isResponses && !isMessages)) {
    writeJson(response, { error: { message: "Not found", type: "invalid_request_error" } }, 404);
    return;
  }

  const apiKey = bearerToken(request);
  if (!apiKey) {
    writeJson(response, { error: { message: "Missing Cursor API key", type: "authentication_error" } }, 401);
    return;
  }

  const body = await readJson(request);
  const model = String(body.model || "composer-2.5");
  const prompt = promptFromRequest(url, body);
  if (!prompt) {
    writeJson(response, { error: { message: "Empty prompt", type: "invalid_request_error" } }, 400);
    return;
  }

  const id = `cursor-${crypto.randomUUID()}`;
  const stream = body.stream === true;
  const { agent, run } = await Promise.race([
    runAgent(apiKey, model, prompt),
    new Promise((_, reject) => {
      setTimeout(() => {
        const error = new Error("Cursor SDK run timed out");
        error.status = 504;
        reject(error);
      }, RUN_TIMEOUT_MS);
    }),
  ]);

  try {
    if (stream && isChat) {
      response.writeHead(200, {
        "content-type": "text/event-stream",
        "cache-control": "no-cache",
        connection: "keep-alive",
      });
      writeSse(response, openaiChatChunk(id, model, { role: "assistant" }));
      for await (const event of run.stream()) {
        if (event?.type === "assistant") {
          const text = asText(event);
          if (text) writeSse(response, openaiChatChunk(id, model, { content: text }));
        }
      }
      writeSse(response, openaiChatChunk(id, model, {}, "stop"));
      response.write("data: [DONE]\n\n");
      response.end();
      await run.wait().catch(() => {});
      return;
    }

    const collected = await collectRun(run);
    if (collected.error?.message) {
      writeJson(
        response,
        { error: { message: collected.error.message, type: "api_error" } },
        collected.status === "cancelled" ? 499 : 502
      );
      return;
    }

    if (isMessages) {
      writeJson(response, {
        id,
        type: "message",
        role: "assistant",
        model,
        content: [{ type: "text", text: collected.text }],
        stop_reason: "end_turn",
      });
      return;
    }

    if (isResponses) {
      writeJson(response, {
        id,
        object: "response",
        status: "completed",
        model,
        output: [{ type: "message", role: "assistant", content: [{ type: "output_text", text: collected.text }] }],
      });
      return;
    }

    writeJson(response, openaiChatCompletion(id, model, collected.text, collected.usage));
  } finally {
    await agent.close?.();
  }
}

function startServer() {
  const server = http.createServer((request, response) => {
    handleRequest(request, response).catch((error) => {
      const status = Number(error.status) || 500;
      if (!response.headersSent) {
        writeJson(
          response,
          { error: { message: error.message || "Cursor SDK sidecar failed", type: "api_error" } },
          status
        );
      } else {
        response.end();
      }
    });
  });
  server.listen(PORT, HOST, () => {
    console.log(`Aether Cursor SDK sidecar listening on http://${HOST}:${PORT}`);
  });
  return server;
}

if (import.meta.url === `file://${process.argv[1]}`) {
  startServer();
}
