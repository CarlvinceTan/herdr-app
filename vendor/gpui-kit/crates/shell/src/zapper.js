// Herdr's zapper, for pages in a browser pane: the element under the cursor is outlined in the
// accent colour and a click removes it. A small dock at the bottom if the window undoes, redoes
// and saves what was removed for this site.
(() => {
  if (window.__herdrZapper) window.__herdrZapper.stop();

  const ACCENT = "#3b82f6";
  const DOCK = "herdr-zapper";
  const storeKey = `herdr.zapper.${location.hostname || ""}`;

  const saved = () => {
    try {
      const list = JSON.parse(localStorage.getItem(storeKey) || "[]");
      return Array.isArray(list) ? list.filter((item) => typeof item === "string") : [];
    } catch {
      return [];
    }
  };

  const applySaved = () => {
    const selectors = saved();
    if (selectors.length === 0) return;
    const style = document.createElement("style");
    style.id = "herdr-zapper-saved";
    style.textContent = `${selectors.join(",\n")}{display:none!important;}`;
    const add = () => (document.head || document.documentElement).appendChild(style);
    if (document.head || document.documentElement) add();
    else addEventListener("DOMContentLoaded", add, { once: true });
  };
  applySaved();

  const style = document.createElement("style");
  style.textContent = `
#${DOCK} { position: fixed; left: 50%; bottom: 22px; transform: translateX(-50%); z-index: 2147483647;
  display: flex; align-items: center; gap: 4px; padding: 6px; border-radius: 10px;
  background: rgba(28,30,34,.94); border: 1px solid rgba(255,255,255,.12);
  box-shadow: 0 8px 24px rgba(0,0,0,.4); font: 13px -apple-system, system-ui, sans-serif;
  color: #ddd; cursor: grab; user-select: none; touch-action: none; }
#${DOCK}.dragging { cursor: grabbing; }
#${DOCK} button { all: unset; display: flex; align-items: center; justify-content: center;
  width: 30px; height: 30px; border-radius: 7px; cursor: pointer; color: #ddd; }
#${DOCK} button:hover { background: rgba(255,255,255,.12); }
#${DOCK} button:disabled { opacity: .35; cursor: default; background: none; }
#${DOCK} svg { width: 16px; height: 16px; }
.herdr-zapper-highlight { position: fixed; z-index: 2147483646; pointer-events: none;
  border: 2px solid ${ACCENT}; background: rgba(59,130,246,.26); border-radius: 4px; }
`;
  (document.head || document.documentElement).appendChild(style);

  const icon = (paths) => {
    const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    svg.setAttribute("viewBox", "0 0 24 24");
    svg.setAttribute("fill", "none");
    svg.setAttribute("stroke", "currentColor");
    svg.setAttribute("stroke-width", "2");
    svg.setAttribute("stroke-linecap", "round");
    svg.setAttribute("stroke-linejoin", "round");
    for (const d of paths) {
      const path = document.createElementNS("http://www.w3.org/2000/svg", "path");
      path.setAttribute("d", d);
      svg.appendChild(path);
    }
    return svg;
  };
  const button = (act, title, paths) => {
    const element = document.createElement("button");
    element.dataset.act = act;
    element.title = title;
    if (act !== "save") element.disabled = true;
    element.appendChild(icon(paths));
    return element;
  };
  const dock = document.createElement("div");
  dock.id = DOCK;
  dock.append(
    button("undo", "Undo", ["M9 14 4 9l5-5", "M4 9h10a6 6 0 0 1 0 12h-3"]),
    button("redo", "Redo", ["m15 14 5-5-5-5", "M20 9H10a6 6 0 0 0 0 12h3"]),
    button("save", "Save", ["M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2Z", "M17 21v-8H7v8", "M7 3v5h8"]),
  );
  (document.body || document.documentElement).appendChild(dock);

  const box = document.createElement("div");
  box.className = "herdr-zapper-highlight";
  box.style.display = "none";
  (document.body || document.documentElement).appendChild(box);

  const undo = [];
  const redo = [];
  let target = null;
  let drag = null;
  let moved = false;
  let suppressClick = false;

  const isUi = (node) => node === dock || dock.contains(node) || node === box;
  const sync = () => {
    dock.querySelector('[data-act="undo"]').disabled = undo.length === 0;
    dock.querySelector('[data-act="redo"]').disabled = redo.length === 0;
  };

  const cssPath = (element) => {
    if (!element || element.nodeType !== 1) return "";
    if (element.id) {
      try {
        const id = CSS.escape(element.id);
        if (document.querySelectorAll(`#${id}`).length === 1) return `#${id}`;
      } catch {
      }
    }
    const parts = [];
    let node = element;
    for (let depth = 0; node && node.nodeType === 1 && depth < 5; depth += 1) {
      let part = node.tagName.toLowerCase();
      if (node.id) {
        part += `#${CSS.escape(node.id)}`;
        parts.unshift(part);
        break;
      }
      for (const name of [...node.classList].filter(Boolean).slice(0, 2)) {
        part += `.${CSS.escape(name)}`;
      }
      const parent = node.parentElement;
      if (parent) {
        const same = [...parent.children].filter((child) => child.tagName === node.tagName);
        if (same.length > 1) part += `:nth-of-type(${same.indexOf(node) + 1})`;
      }
      parts.unshift(part);
      node = node.parentElement;
    }
    return parts.join(" > ");
  };

  const outline = (element) => {
    target = element && element.isConnected ? element : null;
    if (!target) {
      box.style.display = "none";
      return;
    }
    const rect = target.getBoundingClientRect();
    box.style.display = "block";
    box.style.left = `${rect.left}px`;
    box.style.top = `${rect.top}px`;
    box.style.width = `${rect.width}px`;
    box.style.height = `${rect.height}px`;
  };

  const pick = (x, y) => {
    for (const element of document.elementsFromPoint(x, y)) {
      if (isUi(element)) continue;
      if (element === document.documentElement || element === document.body) continue;
      return element;
    }
    return null;
  };

  const hover = (event) => {
    if (isUi(event.target)) return;
    outline(pick(event.clientX, event.clientY));
  };

  const hide = (element) => {
    if (!element || !element.parentNode) return;
    undo.push({ element, parent: element.parentNode, next: element.nextSibling, display: element.style.display });
    element.style.setProperty("display", "none", "important");
    redo.length = 0;
    outline(null);
    sync();
  };

  const restore = (entry) => {
    entry.parent.insertBefore(entry.element, entry.next);
    entry.element.style.display = entry.display;
  };

  const click = (event) => {
    if (isUi(event.target)) return;
    event.preventDefault();
    event.stopPropagation();
    if (target) hide(target);
  };

  const key = (event) => {
    if (event.key !== "Escape") return;
    event.preventDefault();
    stop();
  };

  const scroll = () => outline(target);

  dock.addEventListener("click", (event) => {
    if (suppressClick) {
      suppressClick = false;
      event.stopPropagation();
      return;
    }
    const button = event.target.closest("button");
    if (!button) return;
    event.stopPropagation();
    if (button.dataset.act === "undo") {
      const entry = undo.pop();
      if (entry) {
        restore(entry);
        redo.push(entry);
        sync();
      }
    } else if (button.dataset.act === "redo") {
      const entry = redo.pop();
      if (entry) {
        entry.element.style.setProperty("display", "none", "important");
        undo.push(entry);
        sync();
      }
    } else if (button.dataset.act === "save") {
      const selectors = undo.map((entry) => cssPath(entry.element)).filter(Boolean);
      try {
        localStorage.setItem(storeKey, JSON.stringify([...new Set([...saved(), ...selectors])]));
      } catch {
      }
      stop();
    }
  });

  // The whole dock drags: a press that moves repositions it, a press that doesn't clicks a button.
  const dragMove = (event) => {
    if (!drag) return;
    if (!moved && Math.hypot(event.clientX - drag.sx, event.clientY - drag.sy) < 3) return;
    moved = true;
    dock.style.left = `${event.clientX - drag.dx}px`;
    dock.style.top = `${event.clientY - drag.dy}px`;
    dock.style.bottom = "auto";
    dock.style.transform = "none";
  };
  const dragEnd = () => {
    if (moved) suppressClick = true;
    drag = null;
    moved = false;
    dock.classList.remove("dragging");
    removeEventListener("pointermove", dragMove);
    removeEventListener("pointerup", dragEnd);
  };
  dock.addEventListener("pointerdown", (event) => {
    const rect = dock.getBoundingClientRect();
    drag = { dx: event.clientX - rect.left, dy: event.clientY - rect.top, sx: event.clientX, sy: event.clientY };
    moved = false;
    dock.classList.add("dragging");
    addEventListener("pointermove", dragMove);
    addEventListener("pointerup", dragEnd);
  });

  const stop = () => {
    removeEventListener("mousemove", hover, true);
    removeEventListener("click", click, true);
    removeEventListener("keydown", key, true);
    removeEventListener("scroll", scroll, true);
    dock.remove();
    box.remove();
    style.remove();
    window.__herdrZapper = undefined;
  };

  addEventListener("mousemove", hover, true);
  addEventListener("click", click, true);
  addEventListener("keydown", key, true);
  addEventListener("scroll", scroll, true);
  window.__herdrZapper = { stop };
})();
