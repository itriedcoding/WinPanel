// Small DOM/formatting toolkit shared by every view. No framework, no build.

const ICONS = {
  dashboard: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
  server: '<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>',
  templates: '<path d="M12 3l8 4.5-8 4.5-8-4.5L12 3z"/><path d="M4 12l8 4.5 8-4.5"/><path d="M4 16.5L12 21l8-4.5"/>',
  users: '<circle cx="9" cy="8" r="3.2"/><path d="M3.5 20a5.5 5.5 0 0111 0"/><path d="M16 5.2a3.2 3.2 0 010 5.6"/><path d="M17.5 14.4A5.5 5.5 0 0120.5 20"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 00.34 1.87l.06.06a2 2 0 11-2.83 2.83l-.06-.06a1.7 1.7 0 00-1.87-.34 1.7 1.7 0 00-1 1.56V21a2 2 0 11-4 0v-.11a1.7 1.7 0 00-1.11-1.56 1.7 1.7 0 00-1.87.34l-.06.06a2 2 0 11-2.83-2.83l.06-.06a1.7 1.7 0 00.34-1.87 1.7 1.7 0 00-1.56-1H3a2 2 0 110-4h.11a1.7 1.7 0 001.56-1.11 1.7 1.7 0 00-.34-1.87l-.06-.06a2 2 0 112.83-2.83l.06.06a1.7 1.7 0 001.87.34H9a1.7 1.7 0 001-1.56V3a2 2 0 114 0v.11a1.7 1.7 0 001 1.56 1.7 1.7 0 001.87-.34l.06-.06a2 2 0 112.83 2.83l-.06.06a1.7 1.7 0 00-.34 1.87V9a1.7 1.7 0 001.56 1H21a2 2 0 110 4h-.11a1.7 1.7 0 00-1.56 1z"/>',
  play: '<path d="M8 5.5l11 6.5-11 6.5V5.5z"/>',
  stop: '<rect x="6" y="6" width="12" height="12" rx="1.5"/>',
  restart: '<path d="M20 11a8 8 0 10-2.3 6.3"/><path d="M20 4v7h-7"/>',
  trash: '<path d="M4 7h16"/><path d="M9 7V4.5h6V7"/><path d="M6.5 7l.8 12.2a1.6 1.6 0 001.6 1.5h6.2a1.6 1.6 0 001.6-1.5L17.5 7"/><path d="M10 11v6M14 11v6"/>',
  edit: '<path d="M4 20h4l10.5-10.5a2.1 2.1 0 00-3-3L5 17v3z"/><path d="M13.5 6.5l3 3"/>',
  folder: '<path d="M3 6.5A1.5 1.5 0 014.5 5h4l2 2.5h8A1.5 1.5 0 0120 9v9a1.5 1.5 0 01-1.5 1.5h-14A1.5 1.5 0 013 18V6.5z"/>',
  file: '<path d="M6 3.5h7.5L18 8v12.5H6V3.5z"/><path d="M13.5 3.5V8H18"/>',
  upload: '<path d="M12 16V4"/><path d="M7.5 8.5L12 4l4.5 4.5"/><path d="M4 16v3.5h16V16"/>',
  download: '<path d="M12 4v12"/><path d="M7.5 11.5L12 16l4.5-4.5"/><path d="M4 16v3.5h16V16"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M15.8 15.8L20 20"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7"/>',
  chevronRight: '<path d="M9 5l7 7-7 7"/>',
  chevronDown: '<path d="M5 9l7 7 7-7"/>',
  terminal: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3"/><path d="M13 15h4"/>',
  archive: '<rect x="3" y="4" width="18" height="5" rx="1.5"/><path d="M5 9v10.5A1.5 1.5 0 006.5 21h11A1.5 1.5 0 0019 19.5V9"/><path d="M10 13h4"/>',
  refresh: '<path d="M20 11a8 8 0 10-2.3 6.3"/><path d="M20 4v7h-7"/>',
  copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M15 6.5V5a1.5 1.5 0 00-1.5-1.5h-8A1.5 1.5 0 004 5v8a1.5 1.5 0 001.5 1.5H7"/>',
  more: '<circle cx="5" cy="12" r="1.2"/><circle cx="12" cy="12" r="1.2"/><circle cx="19" cy="12" r="1.2"/>',
  alert: '<path d="M12 4.5l8.5 15h-17L12 4.5z"/><path d="M12 10v4"/><path d="M12 17h.01"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  cpu: '<rect x="6" y="6" width="12" height="12" rx="2"/><path d="M9.5 2.5v3M14.5 2.5v3M9.5 18.5v3M14.5 18.5v3M2.5 9.5h3M2.5 14.5h3M18.5 9.5h3M18.5 14.5h3"/>',
  memory: '<rect x="3" y="7" width="18" height="10" rx="2"/><path d="M7 10v4M11 10v4M15 10v4"/>',
  disk: '<ellipse cx="12" cy="6" rx="8" ry="3"/><path d="M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6"/><path d="M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3"/>',
  network: '<path d="M12 20V10"/><path d="M12 10L6 4"/><path d="M12 10l6-6"/><circle cx="12" cy="20" r="1"/><circle cx="5" cy="3.5" r="1"/><circle cx="19" cy="3.5" r="1"/>',
  logout: '<path d="M15 5V3.5H4v17h11V19"/><path d="M10 12h10"/><path d="M17 8.5L20.5 12 17 15.5"/>',
  shield: '<path d="M12 3l7 3v6c0 4.2-2.9 7.6-7 9-4.1-1.4-7-4.8-7-9V6l7-3z"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5"/><path d="M12 8h.01"/>',
  save: '<path d="M5 3.5h11L20.5 8v12.5h-17v-17z"/><path d="M8 3.5V8h7V3.5"/><rect x="7.5" y="13" width="9" height="5.5"/>',
  power: '<path d="M12 3v9"/><path d="M7.5 6.5a7 7 0 109 0"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17"/><path d="M12 3.5c2.4 2.4 3.6 5.4 3.6 8.5S14.4 18.6 12 20.5C9.6 18.6 8.4 15.6 8.4 12S9.6 5.9 12 3.5z"/>',
  box: '<path d="M3.5 7.5L12 3l8.5 4.5v9L12 21l-8.5-4.5v-9z"/><path d="M3.5 7.5L12 12l8.5-4.5"/><path d="M12 12v9"/>',
};

export function icon(name, size = 16, className = "") {
  const paths = ICONS[name] || ICONS.info;
  return `<svg class="${className}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths}</svg>`;
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

/** Safe element creation with HTML children only for trusted, escaped strings. */
export function el(tag, attrs = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.className = value;
    else if (key === "text") node.textContent = value;
    else if (key === "html") node.innerHTML = value;
    else if (key.startsWith("on") && typeof value === "function") {
      node.addEventListener(key.slice(2).toLowerCase(), value);
    } else if (key === "dataset") {
      Object.assign(node.dataset, value);
    } else if (value === true) {
      node.setAttribute(key, "");
    } else {
      node.setAttribute(key, value);
    }
  }
  const list = Array.isArray(children) ? children : [children];
  for (const child of list) {
    if (child === null || child === undefined || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}

export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
  return node;
}

export function debounce(fn, wait = 220) {
  let timer = null;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), wait);
  };
}

export function formatBytes(bytes, precise = false) {
  const value = Number(bytes || 0);
  if (value <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let index = 0;
  let size = value;
  while (size >= 1024 && index < units.length - 1) {
    size /= 1024;
    index += 1;
  }
  const digits = precise || size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return `${size.toFixed(digits)} ${units[index]}`;
}

export function formatNumber(value) {
  return Number(value || 0).toLocaleString();
}

export function formatPercent(value, digits = 0) {
  return `${Number(value || 0).toFixed(digits)}%`;
}

export function formatDuration(ms) {
  const seconds = Math.max(0, Math.floor(Number(ms || 0) / 1000));
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  const days = Math.floor(hours / 24);
  return `${days}d ${hours % 24}h`;
}

export function formatUptime(seconds) {
  return formatDuration(Number(seconds || 0) * 1000);
}

export function formatDateTime(ms) {
  if (!ms) return "-";
  const date = new Date(Number(ms));
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function formatTime(ms) {
  if (!ms) return "";
  const date = new Date(Number(ms));
  return date.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

export function timeAgo(ms) {
  if (!ms) return "never";
  const diff = Date.now() - Number(ms);
  if (diff < 45_000) return "just now";
  if (diff < 3_600_000) return `${Math.round(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.round(diff / 3_600_000)}h ago`;
  if (diff < 2_592_000_000) return `${Math.round(diff / 86_400_000)}d ago`;
  return formatDateTime(ms);
}

export function statusPill(status, extra = "") {
  const labels = {
    running: "Running",
    stopped: "Stopped",
    starting: "Starting",
    stopping: "Stopping",
    crashed: "Crashed",
    installing: "Installing",
    install_failed: "Install failed",
  };
  return `<span class="status ${extra}" data-state="${escapeHtml(status)}"><span class="dot"></span>${labels[status] || escapeHtml(status)}</span>`;
}

export function toast(message, kind = "info", timeout = 4200) {
  const root = document.getElementById("toast-root");
  if (!root) return;
  const icons = {
    error: "alert",
    success: "check",
    info: "info",
  };
  const node = el("div", { class: `toast ${kind}` });
  node.innerHTML = `${icon(icons[kind] || "info", 15)}<span>${escapeHtml(message)}</span>`;
  root.appendChild(node);
  setTimeout(() => {
    node.classList.add("leaving");
    setTimeout(() => node.remove(), 200);
  }, timeout);
}

export async function copyText(text) {
  try {
    await navigator.clipboard.writeText(text);
    toast("Copied to clipboard", "success", 1800);
    return true;
  } catch {
    toast("Clipboard access was blocked by the browser", "error");
    return false;
  }
}

/* ------------------------------------------------------------------ modal */

export function openModal({ title, subtitle, body, footer, wide = false, narrow = false, onClose }) {
  const root = document.getElementById("modal-root");
  const panel = el("div", { class: `modal ${wide ? "wide" : ""} ${narrow ? "narrow" : ""}` });
  const head = el("div", { class: "modal-head" }, [
    el("div", {}, [
      el("div", { class: "modal-title", text: title }),
      subtitle ? el("div", { class: "modal-sub", text: subtitle }) : null,
    ]),
  ]);
  const closeBtn = el("button", {
    class: "icon-btn",
    type: "button",
    style: undefined,
    "aria-label": "Close",
    html: icon("x", 15),
  });
  head.appendChild(el("div", { class: "spacer" }));
  head.appendChild(closeBtn);

  const bodyNode = el("div", { class: "modal-body" });
  if (body) bodyNode.append(body);
  panel.append(head, bodyNode);
  let footNode = null;
  if (footer) {
    footNode = el("div", { class: "modal-foot" });
    footNode.append(footer);
    panel.append(footNode);
  }
  panel.appendChild(head);
  root.replaceChildren(panel);
  root.hidden = false;

  const close = () => {
    root.hidden = true;
    root.replaceChildren();
    document.removeEventListener("keydown", onKey);
    if (onClose) onClose();
  };
  const onKey = (event) => {
    if (event.key === "Escape") close();
  };
  closeBtn.addEventListener("click", close);
  root.addEventListener("mousedown", (event) => {
    if (event.target === root) close();
  });
  document.addEventListener("keydown", onKey);
  return { root, panel, body: bodyNode, footer: footNode, close };
}

export function confirmDialog({ title, message, confirmLabel = "Confirm", danger = false, requireText = null }) {
  return new Promise((resolve) => {
    const body = el("div", { class: "col" });
    body.append(el("p", { class: "muted", text: message }));
    let input = null;
    if (requireText) {
      input = el("input", { class: "input", placeholder: requireText, autocomplete: "off" });
      body.append(input);
    }
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const confirm = el("button", {
      class: `btn ${danger ? "btn-danger" : "btn-primary"}`,
      type: "button",
      text: confirmLabel,
    });
    const modal = openModal({
      title,
      body,
      narrow: true,
      footer: [cancel, confirm],
      onClose: () => resolve(false),
    });
    const finish = (value) => {
      resolve(value);
      modal.close();
      // Avoid resolving twice through onClose.
      modal.close = () => {};
    };
    cancel.addEventListener("click", () => finish(false));
    confirm.addEventListener("click", () => {
      if (requireText && input.value.trim() !== requireText) {
        input.focus();
        return;
      }
      finish(true);
    });
    setTimeout(() => (input || confirm).focus(), 30);
  });
}

/* ----------------------------------------------------------------- charts */

export function sparkline(values, { width = 120, height = 26, stroke = "#fafafa", fill = true } = {}) {
  const points = (values || []).filter((v) => Number.isFinite(v));
  if (points.length < 2) {
    return `<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"></svg>`;
  }
  const max = Math.max(...points, 0.001);
  const min = Math.min(...points, 0);
  const range = max - min || 1;
  const step = width / (points.length - 1);
  const coords = points.map((value, index) => {
    const x = index * step;
    const y = height - ((value - min) / range) * (height - 3) - 1.5;
    return `${x.toFixed(1)},${y.toFixed(1)}`;
  });
  const line = coords.join(" ");
  const area = `0,${height} ${line} ${width},${height}`;
  return `<svg width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none">
    ${fill ? `<polygon points="${area}" fill="rgba(255,255,255,0.07)"></polygon>` : ""}
    <polyline points="${line}" fill="none" stroke="${stroke}" stroke-width="1.3" stroke-linejoin="round" vector-effect="non-scaling-stroke"></polyline>
  </svg>`;
}

/**
 * Multi-series area chart on a fixed 600x160 canvas with a light grid.
 * series: [{ values: number[], color, label, unit }]
 */
export function lineChart(series, { height = 160, max = null, labels = [] } = {}) {
  const width = 600;
  const inner = { left: 6, right: 6, top: 8, bottom: 18 };
  const plotW = width - inner.left - inner.right;
  const plotH = height - inner.top - inner.bottom;
  const all = series.flatMap((s) => (s.values || []).filter(Number.isFinite));
  const peak = max ?? Math.max(...all, 0.001);
  const top = peak <= 0 ? 1 : peak;
  const longest = Math.max(...series.map((s) => (s.values || []).length), 2);

  let grid = "";
  for (let i = 0; i <= 3; i += 1) {
    const y = inner.top + (plotH / 3) * i;
    grid += `<line x1="${inner.left}" y1="${y.toFixed(1)}" x2="${width - inner.right}" y2="${y.toFixed(1)}" stroke="#1c1c1c" stroke-width="1" vector-effect="non-scaling-stroke"></line>`;
  }

  let paths = "";
  for (const item of series) {
    const values = item.values || [];
    if (values.length < 2) continue;
    const step = plotW / (longest - 1);
    const offset = plotW - (values.length - 1) * step;
    const coords = values.map((value, index) => {
      const x = inner.left + offset + index * step;
      const clamped = Math.max(0, Math.min(top, value));
      const y = inner.top + plotH - (clamped / top) * plotH;
      return [x, y];
    });
    const line = coords.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
    const first = coords[0];
    const last = coords[coords.length - 1];
    const area = `${first[0].toFixed(1)},${(inner.top + plotH).toFixed(1)} ${line} ${last[0].toFixed(1)},${(inner.top + plotH).toFixed(1)}`;
    if (item.fill !== false) {
      paths += `<polygon points="${area}" fill="${item.fillColor || "rgba(255,255,255,0.05)"}"></polygon>`;
    }
    paths += `<polyline points="${line}" fill="none" stroke="${item.color || "#fafafa"}" stroke-width="1.5" stroke-linejoin="round" stroke-linecap="round" vector-effect="non-scaling-stroke"></polyline>`;
  }

  const labelMarkup = labels
    .map((label) => `<text x="${inner.left}" y="${height - 3}" class="axis-label">${escapeHtml(label)}</text>`)
    .join("");

  return `<svg class="chart" viewBox="0 0 ${width} ${height}" preserveAspectRatio="none" role="img" aria-label="Resource history">
    ${grid}${paths}${labelMarkup}
  </svg>`;
}

/* --------------------------------------------------------------- skeleton */

export function skeletonStats(count = 6) {
  let html = "";
  for (let i = 0; i < count; i += 1) html += `<div class="skeleton skeleton-stat"></div>`;
  return `<div class="stat-grid">${html}</div>`;
}

export function skeletonRows(rows = 5) {
  let html = "";
  for (let i = 0; i < rows; i += 1) {
    html += `<div class="list-row"><div class="skeleton skeleton-line" data-w="46%"</div><div class="skeleton skeleton-line" data-w="20%"></div></div>`;
  }
  return `<div class="list">${html}</div>`;
}

export function emptyState({ icon: iconName = "info", title, text, actionHtml = "" }) {
  return `<div class="empty">
    <div class="empty-icon">${icon(iconName, 18)}</div>
    <div class="empty-title">${escapeHtml(title)}</div>
    <div class="empty-text">${escapeHtml(text)}</div>
    ${actionHtml}
  </div>`;
}

export function setBusy(button, busy, label = null) {
  if (!button) return;
  if (busy) {
    button.dataset.label = button.innerHTML;
    button.disabled = true;
    button.innerHTML = `<span class="spin"></span>${label ? `<span>${escapeHtml(label)}</span>` : ""}`;
  } else {
    button.disabled = false;
    if (button.dataset.label) button.innerHTML = button.dataset.label;
  }
}
