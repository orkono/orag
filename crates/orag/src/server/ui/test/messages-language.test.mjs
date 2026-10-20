// Messages already on screen follow a language change (step 77 review): toasts
// (P3-3) and the detail line of client-side errors (P3-4).
import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { installPage } from "./fake-dom.mjs";

const doc = installPage();
const { setLanguage, t } = await import("../i18n.js");
const { toast, toastError } = await import("../dialog.js");
const $ = (id) => doc.getElementById(id);

function lastToast() {
  const items = $("toasts").children;
  return items[items.length - 1];
}

test("a visible toast changes language", () => {
  mock.timers.enable({ apis: ["setTimeout"] });
  try {
    toastError({ code: "busy", message: "HTTP 429: busy (busy)" }, "documents.deleteFailed");
    const error = lastToast();
    toast({ key: "common.deleted", params: { name: "a.txt" } });
    const done = lastToast();
    toast({ key: "collections.reindexQueued", params: { count: { key: "collections.count", params: { count: 1 } } } });
    const queued = lastToast();
    assert.match(error.textContent, /Sunucu şu an meşgul/);
    assert.match(done.textContent, /"a\.txt" silindi\./);
    assert.match(queued.textContent, /1 belge yeniden indeksleniyor\./);

    setLanguage("en");
    assert.match(error.textContent, /The server is busy/);
    assert.match(error.textContent, /HTTP 429: busy \(busy\)/, "server detail stays as it is");
    assert.match(done.textContent, /"a\.txt" deleted\./);
    assert.match(queued.textContent, /Reindexing 1 document\./);
  } finally {
    setLanguage("tr");
    mock.timers.reset();
  }
});

test("a client-side error's own sentence follows the language; server detail does not", async () => {
  const { clientError, errorBox } = await import("../api.js");
  const gone = clientError("collections.gone", { name: "HR" });
  assert.equal(gone.messageKey, "collections.gone");
  assert.equal(gone.message, '"HR" artık yok.');
  const box = errorBox(gone, "collections.deleteFailed");
  const server = errorBox({ code: "internal", message: "HTTP 500: disk on fire (internal)" }, "documents.loadFailed");
  // The heading is not repeated as the detail.
  const lost = errorBox(clientError("ask.lost", undefined, "lost"), "ask.failed", { lost: "ask.lost" });
  doc.body.append(box, server, lost); // on screen, as the page shows them
  assert.equal(box.textContent, 'Koleksiyon silinemedi."HR" artık yok.');
  assert.equal(lost.textContent, "Bağlantı kesildi; yanıt tamamlanmadı.");
  mock.timers.enable({ apis: ["setTimeout"] });
  try {
    toastError(clientError("collections.gone", { name: "HR" }), "collections.deleteFailed");
    const shown = lastToast();
    setLanguage("en");
    assert.equal(box.textContent, 'Could not delete the collection."HR" no longer exists.');
    assert.equal(lost.textContent, "The connection dropped; the answer is incomplete.");
    assert.equal(server.textContent, "Something went wrong on the server; see the server log for details.HTTP 500: disk on fire (internal)");
    assert.match(shown.textContent, /"HR" no longer exists\./);
  } finally {
    setLanguage("tr");
    mock.timers.reset();
  }
});

test("t() fills a placeholder with a message in the current language", () => {
  const count = { key: "collections.count", params: { count: 3 } };
  assert.equal(t("collections.reindexQueued", { count }), "3 belge yeniden indeksleniyor.");
  setLanguage("en");
  assert.equal(t("collections.reindexQueued", { count }), "Reindexing 3 documents.");
  setLanguage("tr");
});
