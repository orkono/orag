// The graphics card line in the sidebar footer and the last answer's time
// (v0.6 step 87). Pure text: `test/gpu.test.mjs` covers it; status.js puts
// it on the page. The state comes from `GET /v1/version`'s `gpu`, which the
// service computes from where the answer model was loaded.
import { t } from "./i18n.js";

const REASONS = new Set([
  "loader-missing",
  "no-eligible-device",
  "probe-timeout",
  "start-timeout",
  "start-failed",
  "crashed-twice",
  "fit_failed",
  "fit_zero_layers",
  "alloc_failed",
  "no_eligible_device",
]);

/**
 * `{text, retry}` for the footer, or null when there is nothing to say: the
 * CPU `orag` run directly (or `gpu = "off"`), and Apple Silicon, where Metal
 * always holds the models.
 */
export function gpuLine(gpu) {
  if (!gpu || gpu.backend === "metal") return null;
  if (gpu.in_use) {
    const device = gpu.device || t("gpu.unnamed");
    const state = gpu.full
      ? t("gpu.inUse")
      : t("gpu.partial", { on: gpu.layers_on_gpu ?? "?", total: gpu.layers_total ?? "?" });
    return { text: t("gpu.line", { device, state }), retry: false };
  }
  if (!gpu.reason) return null;
  const text = REASONS.has(gpu.reason)
    ? t("gpu.notUsedWhy", { why: t(`gpu.reason.${gpu.reason}`) })
    : t("gpu.notUsedCode", { code: gpu.reason });
  return { text, retry: Boolean(gpu.retry) };
}

/** "Son yanıt: 3,2 sn · ekran kartı", or null without a time. */
export function answerTimeText(ms, backend) {
  if (typeof ms !== "number" || !Number.isFinite(ms) || ms < 0) return null;
  const seconds = ms < 10000 ? Math.round(ms / 100) / 10 : Math.round(ms / 1000);
  const where = t(`gpu.where.${backend === "vulkan" || backend === "metal" ? backend : "cpu"}`);
  return t("gpu.lastAnswer", { seconds: String(seconds).replace(".", t("format.decimal")), where });
}

/** Where an answer ran, from the `gpu` state at the time: kept with its time. */
export function whereOf(gpu) {
  return gpu?.in_use && (gpu.backend === "vulkan" || gpu.backend === "metal") ? gpu.backend : "cpu";
}

/** True when a `/v1/version` `gpu` differs from the one shown before. */
export function gpuChanged(before, after) {
  return JSON.stringify(before ?? null) !== JSON.stringify(after ?? null);
}
