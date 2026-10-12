// Line icons drawn as SVG elements (never parsed from markup). An element
// with `data-icon="name"` in the page gets its icon when the page starts.

const SVG_NS = "http://www.w3.org/2000/svg";

/** 24×24 outline paths, drawn with the current text colour. */
const PATHS = {
  alert: "M12 3.5 21.5 20h-19ZM12 10v4.5M12 17.5v.01",
  check: "m5 12.5 4.5 4.5L19 7.5",
  copy: "M9 9h11v11H9ZM5 15H4V4h11v1",
  file: "M6 3h8l4 4v14H6ZM14 3v4h4M9 13h6M9 17h6",
  folder: "M3 6.5h6l2 2h10V19H3Z",
  help: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18ZM9.6 9.2a2.5 2.5 0 1 1 3.3 2.4c-.6.3-.9.8-.9 1.4v.5M12 16.5v.01",
  lock: "M6 11h12v10H6ZM8.5 11V7.5a3.5 3.5 0 0 1 7 0V11",
  plus: "M12 5v14M5 12h14",
  refresh: "M19.5 10.5A7.5 7.5 0 0 0 6 6.6L4.5 8.2M4.5 4v4.2h4.2M4.5 13.5A7.5 7.5 0 0 0 18 17.4l1.5-1.6M19.5 20v-4.2h-4.2",
  send: "M4 12h15M13 6l6 6-6 6",
  sparkle: "M12 3.5 13.8 9 19.5 10.5 13.8 12 12 17.5 10.2 12 4.5 10.5 10.2 9ZM18.5 16.5l.7 1.8 1.8.7-1.8.7-.7 1.8-.7-1.8-1.8-.7 1.8-.7Z",
  stop: "M7.5 7.5h9v9h-9Z",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
  upload: "M12 15.5V4M7.5 8.5 12 4l4.5 4.5M4.5 15v5h15v-5",
  x: "M6 6l12 12M18 6 6 18",
};

export function icon(name) {
  const svg = document.createElementNS(SVG_NS, "svg");
  svg.setAttribute("viewBox", "0 0 24 24");
  svg.setAttribute("aria-hidden", "true");
  svg.setAttribute("class", "icon");
  const path = document.createElementNS(SVG_NS, "path");
  path.setAttribute("d", PATHS[name] || PATHS.file);
  svg.append(path);
  return svg;
}

/** Puts each `[data-icon]` element's icon in front of its content. */
export function applyIcons(root = document) {
  for (const target of root.querySelectorAll("[data-icon]")) {
    if (!target.querySelector(":scope > svg.icon")) target.prepend(icon(target.dataset.icon));
  }
}
