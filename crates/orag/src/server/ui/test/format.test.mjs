// Pure page logic, run with `node --test crates/orag/src/server/ui/test/`
// (Node's built-in runner: no dependencies, no build step).
import { test } from "node:test";
import assert from "node:assert/strict";
import { answerNotice, finishNotice, pageLabel, retryAfterMs, warningText } from "../format.js";

test("page labels", () => {
  assert.equal(pageLabel(null), "");
  assert.equal(pageLabel([3, 3]), "s. 3");
  assert.equal(pageLabel([12, 13]), "s. 12–13");
  assert.equal(pageLabel([1]), "");
});

test("warnings get a Turkish label with their pages and reason", () => {
  assert.equal(
    warningText("ocr_required: page(s) 3-5, 9 have no extractable text (scanned?) and were not indexed"),
    "Taranmış sayfalar okunamadı (s. 3–5, 9)",
  );
  assert.equal(
    warningText(
      "suspicious_layout: page(s) 2-3 may be garbled (words split into single letters); answers from them can be wrong",
    ),
    "Bazı sayfaların metni bozuk olabilir (s. 2–3; harflere bölünmüş kelimeler)",
  );
  assert.equal(warningText("no_text: the document has no extractable text"), "Metin bulunamadı");
});

test("unknown warning codes fall back to the raw text", () => {
  assert.equal(warningText("something_new: x"), "something_new: x");
  // Object.prototype keys are not labels.
  assert.equal(warningText("constructor: x"), "constructor: x");
});

test("finish notices", () => {
  assert.equal(finishNotice("stop"), "");
  assert.equal(finishNotice(undefined), "");
  assert.match(finishNotice("length"), /kesildi/);
  assert.match(finishNotice("repetition"), /tekrar/);
  assert.equal(finishNotice("toString"), "");
});

test("Retry-After is read in seconds, with a default and a cap", () => {
  const headers = (value) => new Headers(value === undefined ? {} : { "Retry-After": value });
  assert.equal(retryAfterMs(headers("5")), 5000);
  assert.equal(retryAfterMs(headers()), 5000);
  assert.equal(retryAfterMs(headers("soon")), 5000);
  assert.equal(retryAfterMs(headers("0")), 1000);
  assert.equal(retryAfterMs(headers("3600")), 30000);
});

test("the notice under a finished answer", () => {
  const done = (fields) => ({ abstained: false, uncited: false, finish_reason: "stop", ...fields });
  assert.equal(answerNotice(done({})), "");
  assert.equal(answerNotice(done({ abstained: true })), "Belgelerde bu sorunun yanıtı bulunamadı.");
  assert.equal(
    answerNotice(done({ uncited: true })),
    "Bu yanıt hiçbir kaynağa atıf yapmıyor; belgelerden doğrulayın.",
  );
  assert.equal(
    answerNotice(done({ uncited: true, finish_reason: "length" })),
    "Yanıt uzunluk sınırında kesildi. Bu yanıt hiçbir kaynağa atıf yapmıyor; belgelerden doğrulayın.",
  );
  // A server from before 0.3.0-alpha.21 sends no `uncited`.
  assert.equal(answerNotice({ abstained: false, finish_reason: "stop" }), "");
});
