// Shows the sidecar status the app hands over (src/app.rs) through
// window.oragStatus(status), called again after every load of this page.
// Every value is set with textContent, so service output is never parsed as HTML.
// Turkish or English like the web page (step 77): the page opens in Turkish
// unless English was chosen (localStorage "orag.lang"); the app also hands it
// the language last chosen on the service page (window.oragLanguage).
"use strict";

const STORAGE_KEY = "orag.lang";
/** When the choice was made (ms since 1970). */
const AT_KEY = "orag.lang.at";
const STATES = ["loading", "restarting", "failed"];

const MESSAGES = {
  tr: {
    loading: "Hizmet başlatılıyor…",
    restarting: "Hizmet yeniden başlatılıyor…",
    failed: "ORAG başlatılamadı.",
    hint: "Uygulamayı kapatıp yeniden açmayı deneyin. Sorun sürerse yukarıdaki satırları sorun bildirimine ekleyin.",
    language: "Dil",
  },
  en: {
    loading: "Starting the service…",
    restarting: "Restarting the service…",
    failed: "ORAG could not start.",
    hint: "Try closing the app and opening it again. If the problem persists, add the lines above to your bug report.",
    language: "Language",
  },
};

const isLanguage = (lang) => typeof lang === "string" && Object.hasOwn(MESSAGES, lang);
const timeOf = (value) => (/^\d{1,15}$/.test(String(value ?? "")) ? Number(value) : null);

/** The stored choice ({language, at}), or null. */
function storedChoice() {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (isLanguage(stored)) return { language: stored, at: timeOf(localStorage.getItem(AT_KEY)) };
  } catch {
    // Storage blocked.
  }
  return null;
}

/**
 * Which choice stands, as on the service page (i18n.js chooseLanguage) and
 * in the app (prefs.rs Choice::prefer): a timed choice beats an untimed one,
 * the later time wins (a tie goes to the hand-over), and an untimed
 * hand-over, the app's default, never overrides a stored choice.
 */
function choose(handed, stored) {
  if (!handed || !stored) return handed || stored || { language: "tr", at: null };
  if (handed.at === null) return stored;
  if (stored.at === null) return handed;
  return handed.at >= stored.at ? handed : stored;
}

let chosen = choose(null, storedChoice());
let language = chosen.language;
let lastStatus = null;

const text = (key) => MESSAGES[language][key];

function render(status) {
  lastStatus = status;
  const state = STATES.includes(status && status.state) ? status.state : "loading";
  const main = document.getElementById("main");
  main.className = state;
  document.getElementById("status").textContent = text(state);
  const message = document.getElementById("message");
  const reason = state === "restarting" ? status.reason : state === "failed" ? status.message : "";
  message.textContent = reason || "";
  message.hidden = !reason;
  const details = document.getElementById("details");
  const lines = state === "failed" && Array.isArray(status.details) ? status.details : [];
  details.textContent = lines.join("\n");
  details.hidden = lines.length === 0;
  const hint = document.getElementById("hint");
  hint.textContent = text("hint");
  hint.hidden = state !== "failed";
}

function applyLanguage() {
  document.documentElement.lang = language;
  const select = document.getElementById("language-select");
  select.value = language;
  select.setAttribute("aria-label", text("language"));
  render(lastStatus);
}

/** Shows, stores and writes into the address the choice that stands. */
function adopt(choice) {
  chosen = choice;
  language = choice.language;
  try {
    localStorage.setItem(STORAGE_KEY, choice.language);
    if (choice.at === null) localStorage.removeItem(AT_KEY);
    else localStorage.setItem(AT_KEY, String(choice.at));
  } catch {
    // Not remembered; the page still switches now.
  }
  // The app reads the choice back from the address (src/app.rs) and hands it
  // to the service page, whose own storage is on another origin.
  const at = choice.at === null ? "" : `&at=${choice.at}`;
  try {
    history.replaceState(history.state, "", `${location.pathname}${location.search}#lang=${choice.language}${at}`);
  } catch {
    // The app keeps the language it handed over.
  }
  applyLanguage();
}

/** A choice made on this page's menu: now, but always after the standing choice. */
function chooseOnPage(lang) {
  if (!isLanguage(lang)) return;
  const now = Date.now();
  const at = chosen && chosen.at !== null && chosen.at >= now ? chosen.at + 1 : now;
  adopt({ language: lang, at });
}

/** The app hands over the choice it knows (`at`: ms since 1970, or null). */
function handOver(lang, at) {
  if (!isLanguage(lang)) return;
  adopt(choose({ language: lang, at: timeOf(at) }, chosen));
}

document.getElementById("language-select").addEventListener("change", (event) => chooseOnPage(event.target.value));
applyLanguage();

window.oragStatus = render;
// The app hands over the language last chosen on either page after each load.
window.oragLanguage = handOver;
