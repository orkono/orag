// The GPU bench's entry (v0.6 step 88): a link in the help panel and the
// one-button view at `?bench=1`. The button navigates to `orag-bench://run`,
// which the desktop app cancels and acts on (src/bench/channel.rs): it stops
// the service and shows its own page with the progress.
import { $ } from "./dom.js";
import { versionInfo } from "./api.js";

export const BENCH_RUN_URL = "orag-bench://run";

const desktop = () => globalThis.__ORAG_DESKTOP__ === true;

/** In the desktop app, on a pre-release, and not on Metal. */
export function benchLinkShown(info, inDesktop) {
  const backend = info?.gpu?.backend;
  return Boolean(inDesktop && /-/.test(info?.version ?? "") && backend && backend !== "metal");
}

export function benchRequested(search, inDesktop) {
  return Boolean(inDesktop) && new URLSearchParams(search).get("bench") === "1";
}

export function initBench() {
  document.addEventListener("orag:version", ({ detail }) => {
    $("bench-link").hidden = !benchLinkShown(detail, desktop());
  });
  if (!benchRequested(location.search, desktop())) return;
  $("bench-start").addEventListener("click", (event) => {
    event.currentTarget.disabled = true;
    location.href = BENCH_RUN_URL;
  });
  const dialog = $("bench-dialog");
  versionInfo()
    .then((info) => {
      if (benchLinkShown(info, true)) dialog.showModal();
    })
    .catch(() => {});
}
