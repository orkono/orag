// The setup page's generator choice (step 82): the machine facts and the
// tier options of `GET /v1/models` as plain view models, in the current
// language. No DOM access, so `node --test` covers it (`test/tiers.test.mjs`).
// The server decides every verdict; this file only presents them.
import { byteText } from "./format.js";
import { t } from "./i18n.js";

const VERDICTS = new Set(["recommended", "fits", "too_big_ram", "too_big_disk"]);
const ELIGIBLE = new Set(["recommended", "fits"]);
const UNSUPPORTED = new Set(["cpu_unsupported", "no_tier_fits"]);

const isCount = (value) => typeof value === "number" && Number.isFinite(value) && value >= 0;
const known = (value) => (isCount(value) ? byteText(value) : t("setup.machine.unknown"));

function secondsText(seconds) {
  return String(Math.round(seconds * 10) / 10).replace(".", t("format.decimal"));
}

/** The "Bu bilgisayar" card as label/value rows; rows the server could not read are left out. */
export function machineRows(machine) {
  if (!machine || typeof machine !== "object") return [];
  const rows = [];
  const memory = isCount(machine.available_ram_bytes)
    ? t("setup.machine.memoryValue", { total: known(machine.total_ram_bytes), available: byteText(machine.available_ram_bytes) })
    : known(machine.total_ram_bytes);
  rows.push({ label: t("setup.machine.memory"), value: memory });
  if (machine.cpu_model) rows.push({ label: t("setup.machine.cpu"), value: String(machine.cpu_model) });
  if (isCount(machine.logical_cores)) {
    const cores = isCount(machine.physical_cores) && machine.physical_cores !== machine.logical_cores
      ? t("setup.machine.coresValue", { physical: machine.physical_cores, logical: machine.logical_cores })
      : String(machine.logical_cores);
    rows.push({ label: t("setup.machine.cores"), value: cores });
  }
  if (machine.metal) rows.push({ label: t("setup.machine.gpu"), value: t("setup.machine.metal") });
  else if (machine.gpu?.name) {
    const memoryText = isCount(machine.gpu.memory_bytes) ? ` (${byteText(machine.gpu.memory_bytes)})` : "";
    rows.push({ label: t("setup.machine.gpu"), value: `${machine.gpu.name}${memoryText}` });
  }
  rows.push({ label: t("setup.machine.disk"), value: known(machine.free_disk_bytes) });
  return rows;
}

/** Bytes a download of `option` still fetches: nothing for an installed pack, less after an interrupted run. */
export function optionDownloadBytes(option) {
  if (option?.installed) return 0;
  if (isCount(option?.download_bytes)) return option.download_bytes;
  return isCount(option?.bytes) ? option.bytes : 0;
}

/** The start button's total: the embedder still missing plus the chosen tier. */
export function choiceTotal(download, selectedId) {
  const option = (download?.options ?? []).find((o) => o.id === selectedId);
  if (!option) return null;
  return (isCount(download.embedding_bytes) ? download.embedding_bytes : 0) + optionDownloadBytes(option);
}

function speedText(option, machine) {
  const metal = option.speed?.metal_p50_answer_s;
  const cpu = option.speed?.cpu_p50_answer_s;
  if (machine?.metal && isCount(metal)) return t("setup.tier.speedMetal", { seconds: secondsText(metal) });
  if (!machine?.metal && isCount(cpu)) return t("setup.tier.speedCpu", { seconds: secondsText(cpu) });
  return null;
}

function reasonText(option, machine, download) {
  if (option.verdict === "too_big_ram") {
    return t("setup.reason.too_big_ram", { total: known(machine?.total_ram_bytes), need: byteText(option.need_ram_bytes) });
  }
  if (option.verdict === "too_big_disk") {
    const need = (isCount(download?.embedding_bytes) ? download.embedding_bytes : 0) + optionDownloadBytes(option);
    return t("setup.reason.too_big_disk", { free: known(machine?.free_disk_bytes), need: byteText(need) });
  }
  return null;
}

/** One radio of the list. */
export function optionView(option, machine, download, supported) {
  const verdict = VERDICTS.has(option.verdict) ? option.verdict : "too_big_ram";
  const eligible = supported && ELIGIBLE.has(verdict);
  const warnings = Array.isArray(option.warnings) ? option.warnings : [];
  return {
    id: String(option.id),
    name: t(option.label_key || "setup.tier.default", { id: String(option.id) }),
    verdict,
    badge: t(`setup.verdict.${verdict}`),
    details: option.installed
      ? t("setup.tier.installedDetails", { ram: byteText(option.need_ram_bytes) })
      : t("setup.tier.details", { size: byteText(option.bytes), ram: byteText(option.need_ram_bytes) }),
    speed: speedText(option, machine),
    reason: reasonText(option, machine, download),
    eligible,
    lowMemory: eligible && warnings.includes("low_available_memory")
      ? t("setup.warning.low_available_memory", { available: known(machine?.available_ram_bytes), need: byteText(option.need_ram_bytes) })
      : null,
  };
}

function unsupportedText(support, options) {
  if (support.code === "cpu_unsupported") {
    return t("setup.unsupported.cpu_unsupported", { features: featureList(support) });
  }
  const needs = options.map((o) => o.need_ram_bytes).filter(isCount);
  return t("setup.unsupported.no_tier_fits", { need: byteText(needs.length ? Math.min(...needs) : null) });
}

const featureList = (support) => (support.missing_cpu_features ?? []).map((f) => String(f).toUpperCase()).join(", ");

/** The machine-level warnings the page knows, in the current language. */
function warningTexts(download, support) {
  const codes = Array.isArray(download.warnings) ? download.warnings : [];
  return codes.flatMap((code) => {
    if (code === "single_option") return [t("setup.warning.single_option")];
    if (code === "slow_cpu") return [t("setup.warning.slow_cpu", { features: featureList(support) })];
    return [];
  });
}

/**
 * The model ids the press installs: the embedder if missing and the picked
 * generator if not installed; null without a pick.
 */
export function neededIds(models, chosen) {
  const option = (models?.download?.options ?? []).find((o) => o.id === chosen);
  if (!option) return null;
  const ids = [];
  if (models.embedding && !models.embedding.installed) ids.push(String(models.embedding.id));
  if (!option.installed) ids.push(String(option.id));
  return ids;
}

/**
 * The whole choice for `/v1/models`, or null when the server offers none
 * (an older server, or a catalog without tiers): the page then downloads
 * the configured models as before. `chosen` keeps the user's pick across a
 * redraw while it is still eligible.
 */
export function choiceView(models, chosen) {
  const download = models?.download;
  const options = Array.isArray(download?.options) ? download.options : [];
  if (!options.length) return null;
  const support = download.support ?? { status: "supported", code: null };
  const unsupported = support.status === "unsupported";
  const machine = models.machine ?? null;
  const views = options.map((option) => optionView(option, machine, download, !unsupported));
  const pick = views.find((v) => v.id === chosen && v.eligible) ?? views.find((v) => v.id === download.selected && v.eligible) ?? null;
  return {
    machine: machineRows(machine),
    options: views,
    selected: pick?.id ?? null,
    unsupported: unsupported ? { code: UNSUPPORTED.has(support.code) ? support.code : "no_tier_fits", text: unsupportedText(support, options) } : null,
    showList: support.code !== "cpu_unsupported",
    warnings: unsupported ? [] : warningTexts(download, support),
    total: pick ? choiceTotal(download, pick.id) : null,
    note: download.configured_downloadable === false && models.generation?.id
      ? t("setup.configuredNotDownloadable", { id: String(models.generation.id) })
      : null,
  };
}
