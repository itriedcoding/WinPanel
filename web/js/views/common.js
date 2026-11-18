// Shared building blocks for views.

import { el, icon, escapeHtml, formatUptime, toast, setBusy } from "../ui.js";
import { api, errorMessage } from "../api.js";
import { state, serverById, refreshServers } from "../app.js";

export function pageHeader({ title, subtitle = "", actions = "" }) {
  return `<header class="page-head">
    <div>
      <h1 class="page-title">${escapeHtml(title)}</h1>
      ${subtitle ? `<p class="page-sub">${escapeHtml(subtitle)}</p>` : ""}
    </div>
    <div class="page-actions">${actions}</div>
  </header>`;
}

export function templateName(id) {
  const template = state.templates.find((entry) => entry.id === id);
  return template ? template.name : id || "Custom";
}

export function hostPort(port) {
  const host = location.hostname || "localhost";
  return `${host}:${port}`;
}

export function statusOf(server) {
  return server?.runtime?.status || server?.status || "stopped";
}

/** Trigger a server action and reflect the result. */
export async function serverAction(server, action, button = null, extra = {}) {
  setBusy(button, true, action === "start" ? "Starting" : undefined);
  try {
    await api.post(`/api/servers/${server.id}/action`, { action, ...extra });
    if (action === "delete") {
      toast(`${server.name} deleted`, "success");
    } else if (action === "clone") {
      toast("Server cloned", "success");
    }
    await refreshServers();
    return true;
  } catch (err) {
    toast(errorMessage(err), "error", 7000);
    return false;
  } finally {
    setBusy(button, false);
  }
}

/** Editable KEY=VALUE environment variable rows. */
export function envEditor(initial = []) {
  const wrap = el("div", { class: "col", style: "gap:8px" });
  const rows = el("div", { class: "col", style: "gap:8px" });

  const addRow = (key = "", value = "") => {
    const row = el("div", { class: "env-row" });
    const keyInput = el("input", {
      class: "input",
      placeholder: "KEY",
      value: key,
      spellcheck: "false",
      autocomplete: "off",
    });
    const valueInput = el("input", {
      class: "input env-value",
      placeholder: "value",
      value: value,
      spellcheck: "false",
      autocomplete: "off",
    });
    const remove = el("button", {
      class: "icon-btn danger",
      type: "button",
      title: "Remove variable",
      html: icon("x", 14),
      onclick: () => row.remove(),
    });
    row.append(keyInput, valueInput, remove);
    rows.append(row);
  };
  (initial || []).forEach((entry) => addRow(entry.key, entry.value));

  const add = el("button", {
    class: "btn btn-sm",
    type: "button",
    html: `${icon("plus", 13)}<span>Add variable</span>`,
    onclick: () => addRow(),
  });

  wrap.append(rows, add);
  return {
    node: wrap,
    read() {
      const out = [];
      rows.querySelectorAll(".env-row").forEach((row) => {
        const [keyInput, valueInput] = row.querySelectorAll("input");
        const key = keyInput.value.trim();
        if (key) out.push({ key, value: valueInput.value });
      });
      return out;
    },
  };
}

/** Editable port rows with label, port and protocol. */
export function portEditor(initial = []) {
  const wrap = el("div", { class: "col", style: "gap:8px" });
  const rows = el("div", { class: "col", style: "gap:8px" });

  const addRow = (entry = { label: "Game", port: 25565, protocol: "tcp" }) => {
    const row = el("div", { class: "port-row" });
    const label = el("input", {
      class: "input",
      placeholder: "label",
      value: entry.label || "",
      autocomplete: "off",
    });
    const port = el("input", {
      class: "input port-extra",
      type: "number",
      min: "1",
      max: "65535",
      value: entry.port ?? "",
      placeholder: "port",
    });
    const protocol = el("select", { class: "select port-extra" }, [
      el("option", { value: "tcp", text: "TCP" }),
      el("option", { value: "udp", text: "UDP" }),
    ]);
    protocol.value = entry.protocol === "udp" ? "udp" : "tcp";
    const remove = el("button", {
      class: "icon-btn danger",
      type: "button",
      title: "Remove port",
      html: icon("x", 14),
      onclick: () => row.remove(),
    });
    row.append(label, port, protocol, remove);
    rows.append(row);
  };
  (initial || []).forEach(addRow);
  if (!initial || initial.length === 0) addRow();

  const add = el("button", {
    class: "btn btn-sm",
    type: "button",
    html: `${icon("plus", 13)}<span>Add port</span>`,
    onclick: () => addRow(),
  });

  wrap.append(rows, add);
  return {
    node: wrap,
    read() {
      const out = [];
      rows.querySelectorAll(".port-row").forEach((row) => {
        const labelInput = row.querySelector("input");
        const portInput = row.querySelector("input[type=number]");
        const select = row.querySelector("select");
        const port = Number(portInput.value);
        if (port >= 1 && port <= 65535) {
          out.push({
            label: labelInput.value.trim() || "Game",
            port,
            protocol: select.value,
          });
        }
      });
      return out;
    },
  };
}

export function parseCores(value) {
  const text = String(value || "").trim();
  if (!text) return [];
  const cores = new Set();
  for (const chunk of text.split(",")) {
    const part = chunk.trim();
    if (!part) continue;
    const range = part.split("-").map((n) => Number(n.trim()));
    if (range.length === 2 && Number.isInteger(range[0]) && Number.isInteger(range[1])) {
      for (let i = Math.min(...range); i <= Math.max(...range); i += 1) cores.add(i);
    } else if (Number.isInteger(range[0])) {
      cores.add(range[0]);
    }
  }
  return [...cores].sort((a, b) => a - b);
}

export function formatCores(cores) {
  if (!cores || cores.length === 0) return "";
  return cores.join(",");
}

export function field(label, input, hint = "") {
  const wrap = el("div", { class: "field" });
  wrap.append(el("label", { class: "field-label", text: label }), input);
  if (hint) wrap.append(el("div", { class: "field-hint", text: hint }));
  return wrap;
}

export function serverRow(server, { showActions = true } = {}) {
  const runtime = server.runtime || {};
  const status = statusOf(server);
  const href = `#/servers/${server.id}`;
  const node = el("div", { class: "list-row link" });
  node.addEventListener("click", (event) => {
    if (event.target.closest("button")) return;
    location.hash = href;
  });

  const main = el("div", { class: "row-main" });
  const title = el("div", { class: "row-title" });
  title.innerHTML = `<span class="truncate">${escapeHtml(server.name)}</span>`;
  const pill = el("span", { class: "status" });
  pill.dataset.state = status;
  pill.innerHTML = `<span class="dot"></span>`;
  title.prepend(pill);
  main.append(title);

  const meta = el("div", { class: "row-meta" });
  const bits = [
    escapeHtml(templateName(server.templateId)),
    `${escapeHtml((runtime.cpuPercent || 0).toFixed(0))}% CPU`,
    runtime.status === "running" ? `up ${formatUptime(runtime.uptimeMs)}` : "stopped",
  ];
  meta.innerHTML = bits.join(' <span class="divider-v"></span> ');
  main.append(meta);
  node.append(main);

  if (showActions) {
    const actions = el("div", { class: "row-actions" });
    const canControl = server.level !== "view";
    if (canControl) {
      const isRunning = status === "running" || status === "starting";
      const playBtn = el("button", {
        class: "icon-btn",
        type: "button",
        title: isRunning ? "Stop server" : "Start server",
        html: icon(isRunning ? "stop" : "play", 14),
        onclick: async (event) => {
          event.stopPropagation();
          await serverAction(server, isRunning ? "stop" : "start", playBtn);
        },
      });
      actions.append(playBtn);
    }
    actions.append(
      el("button", {
        class: "icon-btn",
        type: "button",
        title: "Open console",
        html: icon("terminal", 14),
        onclick: (event) => {
          event.stopPropagation();
          location.hash = href;
        },
      })
    );
    node.append(actions);
  }
  return node;
}

export function activityTimeline(entries, { empty = "No activity yet." } = {}) {
  if (!entries || entries.length === 0) {
    return el("div", { class: "empty-text", text: empty });
  }
  const list = el("div", { class: "timeline" });
  for (const entry of entries) {
    const item = el("div", { class: "timeline-item" });
    item.dataset.kind = entry.kind;
    const body = el("div", { class: "timeline-body" });
    body.append(
      el("div", { class: "timeline-title", text: entry.message }),
      el("div", {
        class: "timeline-meta",
        text: [entry.serverName, entry.username, new Date(entry.ts).toLocaleString()].filter(Boolean).join(" • "),
      })
    );
    item.append(el("div", { class: "timeline-dot" }), body);
    list.append(item);
  }
  return list;
}

export function errorBox(message) {
  const node = el("div", { class: "inline-error" });
  node.innerHTML = `${icon("alert", 15)}<span>${escapeHtml(message)}</span>`;
  return node;
}

export function noteBox(message, kind = "") {
  const node = el("div", { class: `inline-note ${kind}` });
  node.innerHTML = `${icon(kind === "warn" ? "alert" : "info", 15)}<span>${escapeHtml(message)}</span>`;
  return node;
}

export function findServerOr404(view, id) {
  const server = serverById(id);
  if (server) return server;
  view.innerHTML = `<div class="card">${`<div class="empty"><div class="empty-title">Server not found</div><div class="empty-text">It may have been deleted, or you no longer have access.</div><a class="btn" href="#/servers">Back to servers</a></div>`}</div>`;
  return null;
}
