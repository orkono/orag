// Server-sent events over a fetch body (EventSource cannot POST).

/**
 * Splits an SSE byte stream into `{event, data}` records. Stopping early (a
 * final event, an error) cancels the stream, which frees the answer slot.
 */
export async function* sseEvents(body) {
  const reader = body.pipeThrough(new TextDecoderStream()).getReader();
  try {
    yield* sseRecords(reader);
  } finally {
    reader.cancel().catch(() => {});
  }
}

async function* sseRecords(reader) {
  let buffer = "";
  for (;;) {
    const { value, done } = await reader.read();
    if (done) return;
    // A trailing "\r" waits for the next chunk: it may be half of "\r\n".
    buffer = (buffer + value).replace(/\r\n|\r(?!$)/g, "\n");
    let end;
    while ((end = buffer.indexOf("\n\n")) !== -1) {
      const block = buffer.slice(0, end);
      buffer = buffer.slice(end + 2);
      let name = "message";
      const data = [];
      for (const line of block.split("\n")) {
        if (line.startsWith("event:")) name = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
      }
      if (data.length > 0) yield { event: name, data: JSON.parse(data.join("\n")) };
    }
  }
}
