interface SseMessage {
  event: string;
  id: string | null;
  data: string;
}

export function parseSseBuffer(buffer: string): { messages: SseMessage[]; remainder: string } {
  const normalized = buffer.replaceAll("\r\n", "\n").replaceAll("\r", "\n");
  const boundary = normalized.lastIndexOf("\n\n");
  if (boundary < 0) return { messages: [], remainder: normalized };
  const complete = normalized.slice(0, boundary);
  const remainder = normalized.slice(boundary + 2);
  const messages = complete
    .split("\n\n")
    .map(parseSseMessage)
    .filter((message): message is SseMessage => message !== null);
  return { messages, remainder };
}

function parseSseMessage(raw: string): SseMessage | null {
  let event = "message";
  let id: string | null = null;
  const data: string[] = [];
  for (const line of raw.split("\n")) {
    if (!line || line.startsWith(":")) continue;
    const separator = line.indexOf(":");
    const field = separator < 0 ? line : line.slice(0, separator);
    const value = separator < 0 ? "" : line.slice(separator + 1).trimStart();
    if (field === "event") event = value;
    else if (field === "id") id = value;
    else if (field === "data") data.push(value);
  }
  if (data.length === 0) return null;
  return { event, id, data: data.join("\n") };
}
