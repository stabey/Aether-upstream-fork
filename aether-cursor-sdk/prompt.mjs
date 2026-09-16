export function flattenChatMessages(messages) {
  if (!Array.isArray(messages)) return "";
  return messages
    .map((message) => {
      const role = String(message?.role || "user").trim() || "user";
      return `${role}: ${textFromContent(message?.content)}`;
    })
    .filter((line) => !line.endsWith(": "))
    .join("\n\n");
}

export function flattenResponsesInput(input) {
  if (typeof input === "string") return input;
  if (!Array.isArray(input)) return textFromContent(input);
  return input
    .map((item) => {
      if (typeof item === "string") return item;
      const role = String(item?.role || item?.type || "user").trim();
      return `${role}: ${textFromContent(item?.content ?? item?.text)}`;
    })
    .join("\n\n");
}

export function flattenAnthropicMessages(messages) {
  if (!Array.isArray(messages)) return "";
  return messages
    .map((message) => {
      const role = String(message?.role || "user").trim() || "user";
      return `${role}: ${textFromContent(message?.content)}`;
    })
    .join("\n\n");
}

export function textFromContent(content) {
  if (typeof content === "string") return content;
  if (!Array.isArray(content)) return content == null ? "" : JSON.stringify(content);
  return content
    .map((part) => {
      if (typeof part === "string") return part;
      if (part?.type === "text" && typeof part.text === "string") return part.text;
      if (typeof part?.text === "string") return part.text;
      return "";
    })
    .filter(Boolean)
    .join("\n");
}

export function promptFromRequest(url, body) {
  if (url.pathname.endsWith("/messages")) {
    const system = typeof body.system === "string" ? `system: ${body.system}\n\n` : "";
    return `${system}${flattenAnthropicMessages(body.messages)}`.trim();
  }
  if (url.pathname.endsWith("/responses") || url.pathname.endsWith("/responses/compact")) {
    return flattenResponsesInput(body.input ?? body.instructions).trim();
  }
  return flattenChatMessages(body.messages).trim();
}

export function openaiChatChunk(id, model, delta, finishReason = null) {
  return {
    id,
    object: "chat.completion.chunk",
    created: Math.floor(Date.now() / 1000),
    model,
    choices: [
      {
        index: 0,
        delta,
        finish_reason: finishReason,
      },
    ],
  };
}
