// Shows the sidecar status the app hands over (src/app.rs) through
// window.oragStatus(status), called again after every load of this page.
// Every value is set with textContent, so service output is never parsed as HTML.
// Turkish or English like the web page (step 77): the page opens in Turkish
// unless English was chosen (localStorage "orag.lang"); the app also hands it
// the language last chosen on the service page (window.oragLanguage).
"use strict";

const STORAGE_KEY = "orag.lang";
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

function storedLanguage() {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (Object.hasOwn(MESSAGES, stored)) return stored;
  } catch {
    // Storage blocked: Turkish.
  }
  return "tr";
}

let language = storedLanguage();
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

function setLanguage(lang) {
  if (!Object.hasOwn(MESSAGES, lang)) return;
  language = lang;
  try {
    localStorage.setItem(STORAGE_KEY, lang);
  } catch {
    // Not remembered; the page still switches now.
  }
  // The app reads the choice back from the address (src/app.rs) and hands it
  // to the service page, whose own storage is on another origin.
  try {
    history.replaceState(history.state, "", `${location.pathname}${location.search}#lang=${lang}`);
  } catch {
    // The app keeps the language it handed over.
  }
  applyLanguage();
}

document.getElementById("language-select").addEventListener("change", (event) => setLanguage(event.target.value));
applyLanguage();

window.oragStatus = render;
// The app hands over the language last chosen on either page after each load.
window.oragLanguage = setLanguage;
