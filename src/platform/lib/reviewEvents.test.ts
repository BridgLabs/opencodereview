import { describe, expect, it } from "vitest";
import { parseSseBuffer } from "./reviewEvents";

describe("parseSseBuffer", () => {
  it("keeps incomplete events for the next chunk", () => {
    const first = parseSseBuffer('event: review-progress\ndata: {"runId":');
    expect(first.messages).toEqual([]);
    const second = parseSseBuffer(`${first.remainder}7}\n\n`);
    expect(second.remainder).toBe("");
    expect(second.messages).toEqual([{ event: "review-progress", id: null, data: '{"runId":7}' }]);
  });

  it("parses ids, comments, CRLF, and multiple data lines", () => {
    const parsed = parseSseBuffer(
      ": heartbeat\r\nevent: review-progress\r\nid: 4-8\r\ndata: first\r\ndata: second\r\n\r\n",
    );
    expect(parsed.messages).toEqual([{ event: "review-progress", id: "4-8", data: "first\nsecond" }]);
  });
});
