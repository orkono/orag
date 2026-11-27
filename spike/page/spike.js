const BASE = "http://127.0.0.1:47613";
const results = [];

async function step(name, fn) {
  const r = { name, origin: location.origin };
  try {
    const res = await fn();
    r.ok = res.ok;
    r.status = res.status;
    r.retryAfter = res.headers.get("Retry-After");
    r.extra = res.extra;
    if (res.body && !res.extra) {
      try { r.bodyReadable = (await res.text()).length > 0; } catch (e) { r.bodyReadable = false; }
    }
  } catch (e) {
    r.ok = false;
    r.error = String(e);
  }
  results.push(r);
}

async function readEvents(res, stopAfter, ctrl) {
  const reader = res.body.getReader();
  const dec = new TextDecoder();
  let text = "", n = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    text += dec.decode(value, { stream: true });
    n = (text.match(/\n\n/g) || []).length;
    if (stopAfter && n >= stopAfter) { ctrl.abort(); break; }
  }
  return n;
}

async function stream(stopAfter) {
  const ctrl = new AbortController();
  const res = await fetch(BASE + "/stream", { method: "POST", signal: ctrl.signal });
  let n;
  try { n = await readEvents(res, stopAfter, ctrl); }
  catch (e) { if (!(stopAfter && e.name === "AbortError")) throw e; n = stopAfter; }
  res.extra = { events: n };
  return res;
}

(async () => {
  const jsonGet = () => fetch(BASE + "/json", { headers: { Accept: "application/json" } });
  await step("a GET /json", jsonGet);
  await step("b POST /json", () => fetch(BASE + "/json", {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ x: 1 }) }));
  await step("c POST /upload", () => {
    const fd = new FormData();
    fd.append("file", new Blob(["hello"], { type: "text/plain" }), "a.txt");
    return fetch(BASE + "/upload", { method: "POST", body: fd });
  });
  await step("d DELETE /item/1", () => fetch(BASE + "/item/1", { method: "DELETE" }));
  await step("e POST /stream", () => stream(0));
  await step("f1 stream aborted after 2", () => stream(2));
  await step("f2 GET /json again", jsonGet);
  const report = { origin: location.origin, ua: navigator.userAgent, results };
  document.getElementById("out").textContent = JSON.stringify(report, null, 1);
  try {
    await fetch(BASE + "/report", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(report) });
  } catch (e) { document.title = "report failed " + e; }
})();
