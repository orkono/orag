// The setup panel's "this computer" card and the list of answer models
// (step 82). Drawn from `tiers.js` view models with textContent only; a
// language change draws both again from the same `/v1/models` answer.
import { $, el } from "./dom.js";
import { choiceView } from "./tiers.js";

let models = null;
let chosen = null;
let onChange = () => {};

function factRow({ label, value }) {
  const row = el("div", undefined, "setup-fact");
  row.append(el("dt", label), el("dd", value));
  return row;
}

function drawMachine(view) {
  $("setup-machine-facts").replaceChildren(...view.machine.map(factRow));
  $("setup-machine").hidden = false;
}

/** An element id for a model id (which may hold dots). */
const inputId = (id) => `setup-tier-${id.replace(/[^\w-]/g, "_")}`;

function radio(option) {
  const input = el("input");
  input.type = "radio";
  input.setAttribute("name", "setup-tier");
  input.value = option.id;
  input.id = inputId(option.id);
  input.disabled = !option.eligible;
  input.checked = option.eligible && option.id === chosen;
  input.addEventListener("change", () => {
    if (!input.checked) return;
    chosen = option.id;
    draw();
    $(inputId(chosen))?.focus(); // the redraw replaced the focused radio
    onChange(chosen);
  });
  return input;
}

function tierItem(option) {
  const head = el("span", undefined, "setup-tier-head");
  head.append(el("span", option.name, "setup-tier-name"), el("span", option.badge, `setup-badge verdict-${option.verdict}`));
  const body = el("span", undefined, "setup-tier-body");
  body.append(head, el("span", option.id, "setup-tier-id"), el("span", option.details, "setup-tier-details"));
  if (option.speed) body.append(el("span", option.speed, "setup-tier-speed"));
  if (option.reason) body.append(el("span", option.reason, "setup-tier-reason"));
  if (option.belowRecommended) body.append(el("span", option.belowRecommended, "setup-tier-warning warning"));
  if (option.lowMemory && option.id === chosen) body.append(el("span", option.lowMemory, "setup-tier-warning warning"));
  const item = el("label", undefined, option.eligible ? "setup-tier" : "setup-tier is-disabled");
  item.htmlFor = inputId(option.id);
  item.append(radio(option), body);
  return item;
}

function drawList(view) {
  $("setup-tier-list").replaceChildren(...view.options.map(tierItem));
  $("setup-choice-warnings").replaceChildren(...view.warnings.map((text) => el("p", text, "warning")));
  $("setup-tiers").hidden = !view.showList;
  const note = $("setup-choice-note");
  note.textContent = view.note ?? "";
  note.hidden = !view.note;
  const unsupported = $("setup-unsupported");
  unsupported.textContent = view.unsupported?.text ?? "";
  unsupported.hidden = !view.unsupported;
}

/** Draws the card and the list; returns the view (null: no choice offered). */
export function draw() {
  const view = choiceView(models, chosen);
  if (!view) return null;
  chosen = view.selected;
  drawMachine(view);
  drawList(view);
  return view;
}

/**
 * Shows the choice of `/v1/models` (`answer`); `changed(id)` hears every
 * pick. Returns the first view, or null when the server offers no choice.
 */
export function initChoice(answer, changed = () => {}) {
  models = answer;
  chosen = null;
  onChange = changed;
  return draw();
}

/** The picked generator id, or null (no choice offered, or none eligible). */
export const chosenGenerator = () => chosen;

/** Locks the list while a download runs. */
export function lockChoice(locked) {
  for (const input of $("setup-tier-list").querySelectorAll("input")) {
    if (locked) input.disabled = true;
  }
  if (!locked) draw();
}
