import assert from "node:assert/strict";
import test from "node:test";
import {
  flattenAnthropicMessages,
  flattenChatMessages,
  flattenResponsesInput,
  promptFromRequest,
} from "./prompt.mjs";

test("flattens OpenAI chat messages", () => {
  assert.equal(
    flattenChatMessages([
      { role: "system", content: "stay brief" },
      { role: "user", content: [{ type: "text", text: "hello" }] },
    ]),
    "system: stay brief\n\nuser: hello"
  );
});

test("flattens Responses and Anthropic payloads", () => {
  assert.equal(flattenResponsesInput("plain"), "plain");
  assert.equal(
    flattenAnthropicMessages([{ role: "user", content: [{ type: "text", text: "hi" }] }]),
    "user: hi"
  );
});

test("selects prompt by request path", () => {
  assert.equal(
    promptFromRequest(new URL("http://sidecar/v1/chat/completions"), {
      messages: [{ role: "user", content: "ping" }],
    }),
    "user: ping"
  );
  assert.equal(
    promptFromRequest(new URL("http://sidecar/v1/messages"), {
      system: "s",
      messages: [{ role: "user", content: "hi" }],
    }),
    "system: s\n\nuser: hi"
  );
});
