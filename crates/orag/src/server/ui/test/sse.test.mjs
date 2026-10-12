// The SSE reader, with in-memory streams (Node has ReadableStream and
// TextDecoderStream built in).
import { test } from "node:test";
import assert from "node:assert/strict";
import { sseEvents } from "../sse.js";

const bytes = (text) => new TextEncoder().encode(text);

test("records are split and parsed", async () => {
  const body = new ReadableStream({
    start(c) {
      c.enqueue(bytes('event: token\ndata: {"text":"a"}\n\nevent: done\r\n'));
      c.enqueue(bytes('data: {"abstained":false}\r\n\r\n'));
      c.close();
    },
  });
  const seen = [];
  for await (const record of sseEvents(body)) seen.push(record);
  assert.deepEqual(seen, [
    { event: "token", data: { text: "a" } },
    { event: "done", data: { abstained: false } },
  ]);
});

test("a body that fails mid-stream is a lost connection", async () => {
  const body = new ReadableStream({
    pull(c) {
      c.enqueue(bytes('event: token\ndata: {"text":"a"}\n\n'));
      c.error(new TypeError("network error"));
    },
  });
  await assert.rejects(
    (async () => { for await (const _ of sseEvents(body)); })(),
    (err) => err.code === "lost",
  );
});
