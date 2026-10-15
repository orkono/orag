// Shows the sidecar status the app hands over (src/app.rs): through
// window.oragStatus(status), or in the URL fragment on a fresh load. Every
// value is set with textContent, so service output is never parsed as HTML.
"use strict";

const TEXT = {
  loading: "Hizmet başlatılıyor…",
  restarting: "Hizmet yeniden başlatılıyor…",
  failed: "ORAG başlatılamadı.",
};

function render(status) {
  const state = TEXT[status && status.state] ? status.state : "loading";
  const main = document.getElementById("main");
  main.className = state;
  document.getElementById("status").textContent = TEXT[state];
  const message = document.getElementById("message");
  const text = state === "restarting" ? status.reason : state === "failed" ? status.message : "";
  message.textContent = text || "";
  message.hidden = !text;
  const details = document.getElementById("details");
  const lines = state === "failed" && Array.isArray(status.details) ? status.details : [];
  details.textContent = lines.join("\n");
  details.hidden = lines.length === 0;
  document.getElementById("hint").hidden = state !== "failed";
}

window.oragStatus = render;

(function fromFragment() {
  const fragment = window.location.hash.slice(1);
  if (!fragment) return;
  try {
    render(JSON.parse(decodeURIComponent(fragment)));
  } catch (_) {
    render({ state: "loading" });
  }
})();
