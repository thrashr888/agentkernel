import { describe, it, expect } from "vitest";
import { parseSSE } from "../src/sse.js";

function makeStream(chunks: string[]): ReadableStream<Uint8Array> {
  const encoder = new TextEncoder();
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      if (i < chunks.length) {
        controller.enqueue(encoder.encode(chunks[i++]));
      } else {
        controller.close();
      }
    },
  });
}

describe("parseSSE", () => {
  it("parses a complete SSE stream", async () => {
    const stream = makeStream([
      'event: started\ndata: {"command":["echo","hello"]}\n\n',
      'event: output\ndata: {"data":"hello\\n","stream":"stdout"}\n\n',
      'event: done\ndata: {"exit_code":0,"success":true}\n\n',
    ]);

    const events = [];
    for await (const event of parseSSE(stream)) {
      events.push(event);
    }

    expect(events).toHaveLength(3);
    expect(events[0].type).toBe("started");
    expect(events[1].type).toBe("output");
    expect(events[1].data).toEqual({ data: "hello\n", stream: "stdout" });
    expect(events[2].type).toBe("done");
  });

  it("stops on error event", async () => {
    const stream = makeStream([
      'event: started\ndata: {"command":["fail"]}\n\n',
      'event: error\ndata: {"message":"command failed"}\n\n',
      'event: output\ndata: {"data":"should not see this"}\n\n',
    ]);

    const events = [];
    for await (const event of parseSSE(stream)) {
      events.push(event);
    }

    expect(events).toHaveLength(2);
    expect(events[1].type).toBe("error");
  });

  it("handles chunked data", async () => {
    const stream = makeStream([
      'event: started\ndata: {"co',
      'mmand":["echo"]}\n\nevent: done\n',
      'data: {"exit_code":0}\n\n',
    ]);

    const events = [];
    for await (const event of parseSSE(stream)) {
      events.push(event);
    }

    expect(events).toHaveLength(2);
    expect(events[0].type).toBe("started");
    expect(events[1].type).toBe("done");
  });

  it("ignores unknown event types", async () => {
    const stream = makeStream([
      'event: unknown\ndata: {}\n\n',
      'event: output\ndata: {"data":"hello"}\n\n',
      'event: done\ndata: {"exit_code":0}\n\n',
    ]);

    const events = [];
    for await (const event of parseSSE(stream)) {
      events.push(event);
    }

    expect(events).toHaveLength(2);
    expect(events[0].type).toBe("output");
  });
});
