// The page's own confirmation dialog and short notifications (toasts).
import { $, el, messageEl } from "./dom.js";
import { errorDetail, errorSentenceKey } from "./api.js";
import { icon } from "./icons.js";
import { setAttrText, t } from "./i18n.js";

const TOAST_MS = 4500;

/**
 * Asks before an action; resolves to true only for the confirm button.
 * Escape or the cancel button resolve to false. For a destructive action the
 * focus starts on cancel, so Enter never deletes by accident. The texts are
 * already translated (the open dialog blocks the language menu).
 */
export function confirmAction({ title, message, confirmLabel = t("common.confirm"), danger = false }) {
  const dialog = $("confirm-dialog");
  // One question at a time: a second one while the first is open is refused,
  // so a single click can never confirm two actions.
  if (dialog.open) return Promise.resolve(false);
  $("confirm-title").textContent = title;
  $("confirm-message").textContent = message;
  const ok = $("confirm-ok");
  ok.textContent = confirmLabel;
  ok.classList.toggle("danger-fill", danger);
  dialog.returnValue = "";
  dialog.showModal();
  (danger ? dialog.querySelector("button[value=cancel]") : ok).focus();
  return new Promise((resolve) => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "ok"), { once: true });
  });
}

/**
 * A short message in the corner; `kind` is "ok" or "error". `text` and
 * `detail` are `{ key, params }` messages, which follow a language change
 * while the toast is up, or plain text (a server's own words).
 */
export function toast(text, kind = "ok", detail = "") {
  const item = el("div", undefined, `toast ${kind}`);
  item.setAttribute("role", kind === "error" ? "alert" : "status");
  const body = el("div", undefined, "toast-body");
  body.append(messageEl("strong", text));
  if (detail) body.append(messageEl("span", detail, "toast-detail"));
  const close = el("button", undefined, "icon-button small");
  close.type = "button";
  setAttrText(close, "aria-label", "common.close");
  close.append(icon("x"));
  item.append(icon(kind === "error" ? "alert" : "check"), body, close);
  const remove = () => {
    item.classList.add("leaving");
    setTimeout(() => item.remove(), 200);
  };
  close.addEventListener("click", remove);
  setTimeout(remove, kind === "error" ? TOAST_MS * 2 : TOAST_MS);
  $("toasts").append(item);
}

/** An API error as a toast: the sentence (`fallback` and `byCode` are keys),
 * with the API message below. */
export function toastError(err, fallback, byCode) {
  toast({ key: errorSentenceKey(err, fallback, byCode) }, "error", errorDetail(err));
}
