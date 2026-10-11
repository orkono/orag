// "Nasıl kullanılır?": what ORAG is and how to use it. Opens by itself on
// the first visit, and from the help buttons afterwards.
import { $ } from "./dom.js";

const SEEN_KEY = "orag.helpSeen";

function seen() {
  try {
    return localStorage.getItem(SEEN_KEY) === "1";
  } catch {
    return true; // storage blocked: do not open it on every visit
  }
}

function markSeen() {
  try {
    localStorage.setItem(SEEN_KEY, "1");
  } catch {
    // Not remembered; it simply opens again next time.
  }
}

export function openHelp() {
  const dialog = $("help-dialog");
  if (dialog.open) return;
  dialog.showModal();
  // Read from the top; focus the button without scrolling down to it.
  $("help-close").focus({ preventScroll: true });
  dialog.scrollTop = 0;
}

export function initHelp() {
  for (const id of ["help-open", "welcome-help"]) $(id).addEventListener("click", openHelp);
  $("help-dialog").addEventListener("close", markSeen);
  // A click on the backdrop (outside the content) closes it too.
  $("help-dialog").addEventListener("click", (event) => {
    if (event.target === event.currentTarget) event.currentTarget.close();
  });
  if (!seen()) openHelp();
}
