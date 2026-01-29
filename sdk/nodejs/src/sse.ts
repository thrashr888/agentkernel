import { createParser, type EventSourceMessage } from "eventsource-parser";
import type { StreamEvent, StreamEventType } from "./types.js";

const KNOWN_EVENTS = new Set<string>(["started", "progress", "output", "done", "error"]);

/** Shared mutable state between the pump and the generator. */
interface PumpState {
  events: StreamEvent[];
  resolve: (() => void) | null;
  done: boolean;
}

function pushEvent(state: PumpState, event: StreamEvent): void {
  state.events.push(event);
  if (state.resolve) {
    const cb = state.resolve;
    state.resolve = null;
    cb();
  }
}

/**
 * Parse an SSE response body into an async generator of StreamEvents.
 *
 * Consumes a ReadableStream<Uint8Array> from fetch() and yields typed events
 * until the stream closes or a "done"/"error" event is received.
 */
export async function* parseSSE(
  body: ReadableStream<Uint8Array>,
): AsyncGenerator<StreamEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  const state: PumpState = { events: [], resolve: null, done: false };

  const parser = createParser({
    onEvent(event: EventSourceMessage) {
      const type = event.event ?? "message";
      if (!KNOWN_EVENTS.has(type)) return;

      let data: Record<string, unknown>;
      try {
        data = JSON.parse(event.data);
      } catch {
        data = { raw: event.data };
      }

      pushEvent(state, { type: type as StreamEventType, data });
    },
  });

  // Read the stream in the background
  const pump = (async () => {
    try {
      for (;;) {
        const result = await reader.read();
        if (result.done) break;
        parser.feed(decoder.decode(result.value, { stream: true }));
      }
    } catch (err) {
      pushEvent(state, {
        type: "error",
        data: { message: err instanceof Error ? err.message : String(err) },
      });
    } finally {
      state.done = true;
      if (state.resolve) {
        const cb = state.resolve;
        state.resolve = null;
        cb();
      }
    }
  })();

  try {
    for (;;) {
      // Yield all buffered events
      while (state.events.length > 0) {
        const event = state.events.shift()!;
        yield event;
        if (event.type === "done" || event.type === "error") return;
      }

      // If the stream is done and no more events, exit
      if (state.done) return;

      // Wait for the next event
      await new Promise<void>((r) => {
        state.resolve = r;
      });
    }
  } finally {
    reader.cancel().catch(() => {});
    await pump.catch(() => {});
  }
}
