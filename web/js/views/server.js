// Server detail: console, files, backups, settings and activity.

import { api, errorMessage, downloadUrl } from "../api.js";
import {
  el,
  escapeHtml,
  icon,
  openModal,
  confirmDialog,
  copyText,
  formatBytes,
  formatDateTime,
  formatDuration,
  formatPercent,
  formatTime,
  formatUptime,
  timeAgo,
  toast,
  emptyState,
  setBusy,
  statusPill,
} from "../ui.js";
import { onServerEvent, refreshServers, state, serverById } from "../app.js";
import {
  activityTimeline,
  envEditor,
  errorBox,
  field,
  findServerOr404,
  formatCores,
  noteBox,
  parseCores,
  portEditor,
  serverAction,
  templateName,
} from "./common.js";

const TABS = [
  { id: "console", label: "Console", iconName: "terminal" },
  { id: "files", label: "Files", iconName: "folder" },
  { id: "backups", label: "Backups", iconName: "archive" },
  { id: "settings", label: "Settings", iconName: "settings" },
  { id: "activity", label: "Activity", iconName: "clock" },
];

export async function renderServer(view, { id, tab }) {
  let server;
  try {
    server = await api.get(`/api/servers/${id}`);
  } catch (err) {
    view.innerHTML = emptyState({
      icon: "server",
      title: "Server unavailable",
      text: errorMessage(err),
      actionHtml: `<a class="btn" href="#/servers">Back to servers</a>`,
    });
    return null;
  }
  const cached = serverById(id);
  if (cached) Object.assign(cached, server);
  else state.servers.push({ ...server });

  const activeTab = TABS.some((entry) => entry.id === tab) ? tab : "console";

  view.innerHTML = `
    <div class="breadcrumb"><a href="#/servers">Servers</a>${icon("chevronRight", 11)}<span>${escapeHtml(server.name)}</span></div>
    <header class="server-head">
      <div class="server-title-wrap">
        <div class="server-title">
          <span id="server-title">${escapeHtml(server.name)}</span>
          <span id="server-status">${statusPill(server.runtime.status)}</span>
        </div>
        <div class="server-desc">${escapeHtml(server.description || templateName(server.templateId))}</div>
      </div>
      <div class="server-actions" id="server-actions"></div>
    </header>
    <div id="install-banner"></div>
    <div class="tabs">
      ${TABS.map(
        (entry) =>
          `<a class="tab ${entry.id === activeTab ? "active" : ""}" href="#/servers/${id}/${entry.id}">${icon(entry.iconName, 13)}<span>${entry.label}</span></a>`
      ).join("")}
    </div>
    <div class="server-layout">
      <div id="tab-content"></div>
      <aside class="server-side" id="server-side"></aside>
    </div>`;

  const content = view.querySelector("#tab-content");
  const side = view.querySelector("#server-side");
  const actionsNode = view.querySelector("#server-actions");
  const banner = view.querySelector("#install-banner");

  let cleanupTab = null;

  function renderSide() {
    const runtime = server.runtime || {};
    const memoryLimit = server.memoryMb || 0;
    const memPercent = memoryLimit ? Math.min(100, (runtime.memoryBytes / (memoryLimit * 1024 * 1024)) * 100) : 0;
    const diskLimit = server.storageMb || 0;
    const diskPercent = diskLimit ? Math.min(100, (runtime.diskBytes / (diskLimit * 1024 * 1024)) * 100) : 0;
    const canManage = server.level === "manage";

    side.innerHTML = `
      <div class="card">
        <div class="card-head"><div class="card-title">Resources</div></div>
        <div class="kv"><span class="k">CPU</span><span class="v">${formatPercent(runtime.cpuPercent || 0)}</span></div>
        <div class="meter">
          <div class="meter-head"><b>Memory</b><span>${formatBytes(runtime.memoryBytes || 0)}${memoryLimit ? ` / ${formatBytes(memoryLimit * 1024 * 1024)}` : ""}</span></div>
          <div class="meter-bar ${memPercent > 95 ? "danger" : ""}"><div class="mem-bar" data-width="${memPercent.toFixed(1)}"></div></div>
        </div>
        <div class="meter">
          <div class="meter-head"><b>Storage</b><span>${formatBytes(runtime.diskBytes || 0)}${diskLimit ? ` / ${formatBytes(diskLimit * 1024 * 1024)}` : ""}</span></div>
          <div class="meter-bar ${diskPercent > 95 ? "danger" : ""}"><div class="disk-bar" data-width="${diskPercent.toFixed(1)}"></div></div>
        </div>
        <div class="kv"><span class="k">Uptime</span><span class="v">${runtime.status === "running" ? formatUptime(runtime.uptimeMs) : "-"}</span></div>
        <div class="kv"><span class="k">PID</span><span class="v mono">${runtime.pid || "-"}</span></div>
        <div class="kv"><span class="k">Restarts</span><span class="v">${runtime.restarts || 0}</span></div>
        <div class="kv"><span class="k">Exit code</span><span class="v mono">${runtime.lastExitCode ?? "-"}</span></div>
      </div>
      <div class="card">
        <div class="card-head"><div class="card-title">Network</div></div>
        ${(server.ports || [])
          .map(
            (port) => `<div class="kv"><span class="k">${escapeHtml(port.label || "Port")} (${port.protocol})</span>
              <span class="v row" style="gap:4px">${escapeHtml(location.hostname || "localhost")}:${port.port}
              <button class="icon-btn" type="button" title="Copy address" data-copy="${escapeHtml(`${location.hostname || "localhost"}:${port.port}`)}">${icon("copy", 12)}</button></span></div>`
          )
          .join("") || '<div class="card-sub">No ports configured</div>'}
        <div class="kv"><span class="k">Panel</span><span class="v mono">${escapeHtml(location.host)}</span></div>
      </div>
      <div class="card">
        <div class="card-head"><div class="card-title">Details</div></div>
        <div class="kv"><span class="k">Template</span><span class="v">${escapeHtml(templateName(server.templateId))}</span></div>
        <div class="kv"><span class="k">Files</span><span class="v">${server.installed ? "Installed" : "Not installed"}</span></div>
        <div class="kv"><span class="k">Autorestart</span><span class="v">${server.autoRestart ? `Yes (${server.autoRestartMax})` : "No"}</span></div>
        <div class="kv"><span class="k">Created</span><span class="v">${formatDateTime(server.createdAt)}</span></div>
        <div class="kv"><span class="k">Directory</span><span class="v mono" style="font-size:11px">${escapeHtml(server.installDir)}</span></div>
        ${canManage ? "" : '<div class="inline-note" style="margin-top:10px">Read-only access</div>'}
      </div>`;

    side.querySelectorAll(".mem-bar").forEach((bar) => (bar.style.width = `${bar.dataset.width}%`));
    side.querySelectorAll(".disk-bar").forEach((bar) => (bar.style.width = `${bar.dataset.width}%`));
    side.querySelectorAll("[data-copy]").forEach((button) => {
      button.addEventListener("click", () => copyText(button.dataset.copy));
    });
  }

  function renderActions() {
    const status = server.runtime.status;
    const canControl = server.level !== "view";
    actionsNode.innerHTML = "";
    if (!canControl) return;

    const running = status === "running" || status === "starting";
    const stopped = ["stopped", "crashed", "install_failed"].includes(status);

    if (stopped) {
      const start = el("button", {
        class: "btn btn-primary",
        type: "button",
        html: `${icon("play", 13)}<span>Start</span>`,
        disabled: !server.installed,
        title: server.installed ? "Start the server" : "Install the server first",
        onclick: async () => {
          await serverAction(server, "start", start);
          await reloadServer();
        },
      });
      actionsNode.append(start);
    }
    if (running) {
      const restart = el("button", {
        class: "btn",
        type: "button",
        html: `${icon("restart", 13)}<span>Restart</span>`,
        onclick: async () => {
          await serverAction(server, "restart", restart);
          await reloadServer();
        },
      });
      const stop = el("button", {
        class: "btn",
        type: "button",
        html: `${icon("stop", 13)}<span>Stop</span>`,
        onclick: async () => {
          await serverAction(server, "stop", stop);
          await reloadServer();
        },
      });
      actionsNode.append(restart, stop);
    }

    const menuButton = el("button", {
      class: "btn btn-icon",
      type: "button",
      title: "More actions",
      html: icon("more", 16),
    });
    const anchor = el("div", { class: "popover-anchor" }, [menuButton]);
    menuButton.addEventListener("click", (event) => {
      event.stopPropagation();
      if (anchor.querySelector(".menu")) {
        anchor.querySelector(".menu").remove();
        return;
      }
      const menu = el("div", { class: "menu" });
      if (running) {
        menu.append(
          el("button", {
            type: "button",
            html: `${icon("power", 13)}<span>Force terminate</span>`,
            class: "danger",
            onclick: async () => {
              const ok = await confirmDialog({
                title: "Force terminate",
                message: `This kills ${server.name} immediately. Unsaved world data can be lost.`,
                confirmLabel: "Terminate",
                danger: true,
              });
              if (!ok) return;
              await serverAction(server, "kill", null);
              await reloadServer();
            },
          })
        );
      }
      if (server.level === "manage") {
        menu.append(
          el("button", {
            type: "button",
            html: `${icon("download", 13)}<span>Reinstall server</span>`,
            onclick: () => reinstall(server, reloadServer),
          }),
          el("button", {
            type: "button",
            html: `${icon("copy", 13)}<span>Clone server</span>`,
            onclick: () => cloneServer(server),
          }),
          el("div", { class: "sep" }),
          el("button", {
            type: "button",
            class: "danger",
            html: `${icon("trash", 13)}<span>Delete server</span>`,
            onclick: () => deleteServer(server),
          })
        );
      }
      anchor.append(menu);
    });
    actionsNode.append(anchor);

    const onDocClick = () => {
      anchor.querySelector(".menu")?.remove();
      document.removeEventListener("click", onDocClick);
    };
    document.addEventListener("click", onDocClick);
  }

  async function reloadServer() {
    try {
      const updated = await api.get(`/api/servers/${id}`);
      server = { ...server, ...updated };
      const cached = serverById(id);
      if (cached) Object.assign(cached, updated);
      updateHeader();
      renderSide();
      renderBanner();
      renderActions();
    } catch {
      /* the mutation already reported its own error */
    }
  }

  function updateHeader() {
    const titleNode = view.querySelector("#server-title");
    if (titleNode) titleNode.textContent = server.name;
    const statusNode = view.querySelector("#server-status");
    if (statusNode) statusNode.innerHTML = statusPill(server.runtime.status);
  }

  function renderBanner() {
    banner.innerHTML = "";
    const runtime = server.runtime || {};
    if (runtime.status === "installing" || (runtime.installStep && runtime.status !== "running")) {
      const percent = runtime.installProgress;
      const card = el("div", { class: "card", style: "margin-bottom:14px;padding:14px 16px" });
      card.innerHTML = `
        <div class="row" style="justify-content:space-between;margin-bottom:8px">
          <span class="card-title">Installing ${escapeHtml(templateName(server.templateId))}</span>
          <span class="card-sub">${percent != null ? `${percent}%` : ""}</span>
        </div>
        <div class="progress ${percent == null ? "indeterminate" : ""}"><div class="i-progress" data-width="${percent ?? 35}"></div></div>
        <div class="card-sub" style="margin-top:8px">${escapeHtml(runtime.installStep || "Preparing…")}</div>`;
      const bar = card.querySelector(".i-progress");
      if (bar) bar.style.width = `${percent ?? 35}%`;
      banner.append(card);
    }
    if (runtime.status === "install_failed") {
      banner.append(errorBox(server.installError || "The installation failed. Check the console output and try again."));
      banner.append(
        el("div", {
          class: "row",
          style: "margin-top:8px",
        }, [
          el("button", {
            class: "btn btn-sm",
            type: "button",
            html: `${icon("refresh", 12)}<span>Retry installation</span>`,
            onclick: () => reinstall(server, reloadServer),
          }),
        ])
      );
    }
  }

  renderSide();
  renderActions();
  renderBanner();

  const unsubscribe = onServerEvent(async (event) => {
    if (event.serverId !== id) return;
    if (event.type === "status") {
      server.runtime.status = event.data.status;
      if (event.data.exitCode !== undefined) server.runtime.lastExitCode = event.data.exitCode;
      updateHeader();
      renderActions();
      renderBanner();
      if (["stopped", "crashed", "running"].includes(event.data.status)) renderSide();
    } else if (event.type === "install") {
      server.runtime.installProgress = event.data.progress;
      server.runtime.installStep = event.data.step;
      if (event.data.status === "complete") {
        server.installed = true;
        server.runtime.status = "stopped";
        server.runtime.installStep = null;
        await reloadServer();
      } else if (event.data.status === "failed") {
        server.runtime.status = "install_failed";
        server.installError = event.data.error;
        renderBanner();
        renderActions();
      } else {
        renderBanner();
        updateHeader();
      }
    }
  });

  // Keep the runtime panel fresh while the server runs.
  const poll = setInterval(async () => {
    if (document.hidden) return;
    const runtime = server.runtime.status;
    if (!["running", "starting", "installing"].includes(runtime)) return;
    try {
      const fresh = await api.get(`/api/servers/${id}`);
      server.runtime = fresh.runtime;
      server.installError = fresh.installError;
      server.installed = fresh.installed;
      updateHeader();
      renderSide();
      renderBanner();
      renderActions();
    } catch {
      /* transient */
    }
  }, 3000);

  if (activeTab === "console") cleanupTab = await consoleTab(content, server, reloadServer);
  else if (activeTab === "files") cleanupTab = await filesTab(content, server);
  else if (activeTab === "backups") cleanupTab = await backupsTab(content, server);
  else if (activeTab === "settings") cleanupTab = await settingsTab(content, server, reloadServer);
  else cleanupTab = await activityTab(content, server);

  return () => {
    unsubscribe();
    clearInterval(poll);
    if (typeof cleanupTab === "function") cleanupTab();
  };
}

/* --------------------------------------------------------------- dialogs */

async function reinstall(server, done) {
  const ok = await confirmDialog({
    title: "Reinstall server",
    message: `This stops ${server.name}, deletes its files and runs the installer again. Worlds and configs will be lost — create a backup first.`,
    confirmLabel: "Reinstall",
    danger: true,
    requireText: server.name,
  });
  if (!ok) return;
  try {
    await api.post(`/api/servers/${server.id}/install`, { options: server.installOptions || {}, wipe: true });
    toast("Reinstall started", "success");
    await done();
  } catch (err) {
    toast(errorMessage(err), "error", 7000);
  }
}

async function cloneServer(server) {
  const body = el("div", { class: "col" });
  const input = el("input", { class: "input", value: `${server.name} copy`, maxlength: "64" });
  body.append(field("New server name", input), noteBox("Files are copied; the clone starts stopped."));
  const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
  const submit = el("button", { class: "btn btn-primary", type: "button", text: "Clone" });
  const modal = openModal({ title: "Clone server", body, footer: [cancel, submit], narrow: true });
  cancel.addEventListener("click", () => modal.close());
  submit.addEventListener("click", async () => {
    setBusy(submit, true, "Cloning");
    try {
      const clone = await api.post(`/api/servers/${server.id}/action`, {
        action: "clone",
        name: input.value.trim(),
      });
      modal.close();
      toast("Server cloned", "success");
      await refreshServers().catch(() => {});
      location.hash = `#/servers/${clone.id}`;
    } catch (err) {
      toast(errorMessage(err), "error", 7000);
      setBusy(submit, false);
    }
  });
}

async function deleteServer(server) {
  const ok = await confirmDialog({
    title: "Delete server",
    message: `Permanently delete ${server.name}, its files and all of its backups? This cannot be undone.`,
    confirmLabel: "Delete everything",
    danger: true,
    requireText: server.name,
  });
  if (!ok) return;
  const success = await serverAction(server, "delete");
  if (success) location.hash = "#/servers";
}

/* --------------------------------------------------------------- console */

async function consoleTab(container, server, reload) {
  const canInput = server.level !== "view";
  container.innerHTML = `
    <div class="console-wrap">
      <div class="console-toolbar">
        <span class="live-badge" id="console-live" data-live="false"><span class="dot"></span><span>Connecting</span></span>
        <span class="divider-v"></span>
        <span class="card-sub" id="console-count"></span>
        <div class="spacer"></div>
        <div class="search-inline" style="min-width:150px;height:27px">
          ${icon("search", 12)}<input id="console-search" placeholder="Filter output" autocomplete="off">
        </div>
        <button class="icon-btn" type="button" id="console-autoscroll" title="Auto-scroll enabled">${icon("chevronDown", 14)}</button>
        <button class="icon-btn" type="button" id="console-copy" title="Copy output">${icon("copy", 14)}</button>
        <button class="icon-btn" type="button" id="console-clear" title="Clear console">${icon("trash", 14)}</button>
      </div>
      <div class="console-log" id="console-log"></div>
      ${
        canInput
          ? `<form class="console-input" id="console-form">
               <span class="prompt">›</span>
               <input class="input" id="console-command" placeholder="Type a command and press Enter" autocomplete="off" spellcheck="false" ${server.runtime.status === "running" ? "" : "disabled"}>
               <button class="btn btn-sm" type="submit" id="console-send">Send</button>
             </form>`
          : `<div class="console-input"><span class="card-sub">Your access level is read-only.</span></div>`
      }
    </div>`;

  const log = container.querySelector("#console-log");
  const liveBadge = container.querySelector("#console-live");
  const countNode = container.querySelector("#console-count");
  const searchInput = container.querySelector("#console-search");
  const commandInput = container.querySelector("#console-command");
  const form = container.querySelector("#console-form");
  const logLines = [];
  const MAX_DOM_LINES = 2500;

  let autoscroll = true;
  let filter = "";
  let socket = null;
  let disposed = false;
  let reconnectTimer = null;

  const historyKey = `winpanel.history.${server.id}`;
  let history = [];
  try {
    history = JSON.parse(localStorage.getItem(historyKey) || "[]");
  } catch {
    history = [];
  }
  let historyIndex = -1;

  function setLive(state, label) {
    if (!liveBadge) return;
    liveBadge.dataset.live = state ? "true" : "false";
    liveBadge.querySelector("span:last-child").textContent = label;
  }

  function lineMatches(line) {
    return !filter || line.text.toLowerCase().includes(filter);
  }

  function lineNode(line) {
    const node = document.createElement("div");
    node.className = "log-line";
    node.dataset.level = line.level || "info";
    node.dataset.stream = line.stream || "stdout";
    node.dataset.text = line.text.toLowerCase();
    node.innerHTML = `<span class="log-time">${formatTime(line.ts)}</span><span class="log-text">${escapeHtml(line.text)}</span>`;
    return node;
  }

  function append(line) {
    logLines.push(line);
    if (logLines.length > 4000) logLines.shift();
    if (lineMatches(line)) {
      log.append(lineNode(line));
      while (log.childElementCount > MAX_DOM_LINES) log.firstElementChild.remove();
      if (autoscroll) log.scrollTop = log.scrollHeight;
    }
    countNode.textContent = `${logLines.length} lines`;
  }

  function renderAll() {
    log.innerHTML = "";
    if (!logLines.length) {
      log.innerHTML = `<div class="console-empty">No output yet. Start the server to see its console here.</div>`;
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const line of logLines.slice(-MAX_DOM_LINES)) {
      if (lineMatches(line)) fragment.append(lineNode(line));
    }
    if (!fragment.childNodes.length) {
      log.innerHTML = `<div class="console-empty">No lines match "${escapeHtml(filter)}".</div>`;
      return;
    }
    log.append(fragment);
    if (autoscroll) log.scrollTop = log.scrollHeight;
  }

  async function initialLoad() {
    try {
      const response = await api.get(`/api/servers/${server.id}/console?limit=1500`);
      logLines.length = 0;
      for (const line of response.lines) logLines.push(line);
      renderAll();
      countNode.textContent = `${logLines.length} lines`;
      if (commandInput) commandInput.disabled = response.status !== "running";
    } catch (err) {
      log.innerHTML = `<div class="console-empty">${escapeHtml(errorMessage(err))}</div>`;
    }
  }

  function connect() {
    if (disposed) return;
    const protocol = location.protocol === "https:" ? "wss" : "ws";
    socket = new WebSocket(`${protocol}://${location.host}/api/servers/${server.id}/ws`);
    setLive(false, "Connecting");

    socket.addEventListener("open", () => setLive(true, "Live"));
    socket.addEventListener("message", (message) => {
      let payload;
      try {
        payload = JSON.parse(message.data);
      } catch {
        return;
      }
      if (payload.type === "console" && payload.line) {
        append(payload.line);
      } else if (payload.type === "status") {
        if (commandInput) commandInput.disabled = payload.status !== "running";
      } else if (payload.type === "console_cleared") {
        logLines.length = 0;
        renderAll();
        countNode.textContent = "0 lines";
      } else if (payload.type === "notice") {
        append({
          ts: Date.now(),
          stream: "system",
          level: payload.level || "info",
          text: payload.message,
        });
      }
    });
    socket.addEventListener("close", () => {
      setLive(false, "Reconnecting");
      if (!disposed) reconnectTimer = setTimeout(connect, 1500);
    });
    socket.addEventListener("error", () => {
      setLive(false, "Disconnected");
    });
  }

  searchInput.addEventListener(
    "input",
    debounce(() => {
      filter = searchInput.value.trim().toLowerCase();
      autoscroll = true;
      renderAll();
    }, 140)
  );

  container.querySelector("#console-autoscroll").addEventListener("click", (event) => {
    autoscroll = !autoscroll;
    const button = event.currentTarget;
    button.style.opacity = autoscroll ? "1" : "0.45";
    button.title = autoscroll ? "Auto-scroll enabled" : "Auto-scroll paused";
    if (autoscroll) log.scrollTop = log.scrollHeight;
  });

  container.querySelector("#console-copy").addEventListener("click", () => {
    const text = logLines
      .filter(lineMatches)
      .map((line) => `[${formatTime(line.ts)}] ${line.text}`)
      .join("\n");
    if (!text) {
      toast("There is nothing to copy yet", "info");
      return;
    }
    copyText(text);
  });

  container.querySelector("#console-clear").addEventListener("click", async () => {
    try {
      await api.post(`/api/servers/${server.id}/console/clear`);
      logLines.length = 0;
      renderAll();
      countNode.textContent = "0 lines";
    } catch (err) {
      toast(errorMessage(err), "error");
    }
  });

  if (form) {
    form.addEventListener("submit", async (event) => {
      event.preventDefault();
      const command = commandInput.value.trim();
      if (!command) return;
      try {
        await api.post(`/api/servers/${server.id}/console/input`, { command });
        commandInput.value = "";
        history = [command, ...history.filter((entry) => entry !== command)].slice(0, 100);
        localStorage.setItem(historyKey, JSON.stringify(history));
        historyIndex = -1;
      } catch (err) {
        toast(errorMessage(err), "error", 6000);
      }
    });

    commandInput.addEventListener("keydown", (event) => {
      if (event.key === "ArrowUp") {
        event.preventDefault();
        if (!history.length) return;
        historyIndex = Math.min(historyIndex + 1, history.length - 1);
        commandInput.value = history[historyIndex] || "";
      } else if (event.key === "ArrowDown") {
        event.preventDefault();
        historyIndex = Math.max(historyIndex - 1, -1);
        commandInput.value = historyIndex >= 0 ? history[historyIndex] : "";
      }
    });
  }

  document.addEventListener("keydown", focusHandler);
  function focusHandler(event) {
    if (event.key === "/" && document.activeElement === document.body) {
      event.preventDefault();
      searchInput.focus();
    }
  }

  await initialLoad();
  connect();

  return () => {
    disposed = true;
    document.removeEventListener("keydown", focusHandler);
    if (reconnectTimer) clearTimeout(reconnectTimer);
    if (socket) {
      socket.onclose = null;
      socket.close();
    }
  };
}

function debounce(fn, wait) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), wait);
  };
}

/* ----------------------------------------------------------------- files */

async function filesTab(container, server) {
  const canWrite = server.level === "files" || server.level === "manage";
  let path = "";
  let listing = null;
  let searchMode = false;
  let disposed = false;

  container.innerHTML = `
    <div class="card flush" id="files-card" style="position:relative">
      <div class="files-toolbar">
        <div class="crumbs" id="crumbs"></div>
        <div class="spacer"></div>
        <div class="search-inline" style="height:27px;min-width:150px">
          ${icon("search", 12)}<input id="file-search" placeholder="Search files" autocomplete="off">
        </div>
        <button class="btn btn-sm" type="button" id="file-refresh" title="Refresh">${icon("refresh", 13)}</button>
        ${canWrite ? `<button class="btn btn-sm" type="button" id="file-mkdir">${icon("folder", 13)}<span>Folder</span></button>` : ""}
        ${canWrite ? `<button class="btn btn-sm" type="button" id="file-upload">${icon("upload", 13)}<span>Upload</span></button>` : ""}
      </div>
      <div id="file-list"></div>
    </div>`;

  const listNode = container.querySelector("#file-list");
  const crumbsNode = container.querySelector("#crumbs");
  const searchInput = container.querySelector("#file-search");
  const uploadInput = el("input", { type: "file", multiple: true, style: "display:none" });
  container.append(uploadInput);

  function renderCrumbs() {
    crumbsNode.innerHTML = "";
    const rootCrumb = el("span", {
      class: "crumb",
      text: server.name,
      onclick: () => {
        path = "";
        searchMode = false;
        searchInput.value = "";
        load();
      },
    });
    crumbsNode.append(rootCrumb);
    if (path) {
      const parts = path.split("/");
      parts.forEach((part, index) => {
        crumbsNode.append(el("span", { class: "crumb-sep", html: icon("chevronRight", 11) }));
        const target = parts.slice(0, index + 1).join("/");
        crumbsNode.append(
          el("span", {
            class: "crumb",
            text: part,
            onclick: () => {
              path = target;
              searchMode = false;
              load();
            },
          })
        );
      });
    }
    crumbsNode.append(el("span", { class: "spacer" }));
  }

  function rowActions(entry) {
    const actions = el("div", { class: "file-actions row", style: "gap:2px" });
    if (!entry.isDir && entry.editable && canWrite) {
      actions.append(
        el("button", {
          class: "icon-btn",
          type: "button",
          title: "Edit file",
          html: icon("edit", 13),
          onclick: (event) => {
            event.stopPropagation();
            openEditor(entry);
          },
        })
      );
    }
    if (!entry.isDir) {
      actions.append(
        el("button", {
          class: "icon-btn",
          type: "button",
          title: "Download",
          html: icon("download", 13),
          onclick: (event) => {
            event.stopPropagation();
            downloadUrl(
              `/api/servers/${server.id}/files/download?path=${encodeURIComponent(entry.path)}`
            );
          },
        })
      );
    }
    if (canWrite) {
      actions.append(
        el("button", {
          class: "icon-btn",
          type: "button",
          title: "Rename",
          html: icon("edit", 13),
          onclick: (event) => {
            event.stopPropagation();
            renameEntry(entry);
          },
        }),
        el("button", {
          class: "icon-btn danger",
          type: "button",
          title: "Delete",
          html: icon("trash", 13),
          onclick: (event) => {
            event.stopPropagation();
            deleteEntry(entry);
          },
        })
      );
    }
    return actions;
  }

  function renderListing() {
    if (!listing) return;
    if (!listing.entries.length) {
      listNode.innerHTML = emptyState({
        icon: "folder",
        title: searchMode ? "No matches" : "This folder is empty",
        text: searchMode ? "Try a different name." : "Upload files or create a folder to get started.",
      });
      return;
    }
    const table = el("table", { class: "table" });
    table.innerHTML = `<thead><tr>
      <th>Name</th><th style="width:120px">Size</th><th style="width:170px">Modified</th><th style="width:140px"></th>
    </tr></thead>`;
    const tbody = el("tbody");
    for (const entry of listing.entries) {
      const tr = el("tr", { class: "file-row" });
      tr.dataset.dir = entry.isDir ? "true" : "false";
      const nameCell = el("td");
      const nameWrap = el("div", { class: "file-name" });
      nameWrap.innerHTML = `${icon(entry.isDir ? "folder" : "file", 14)}<span class="truncate">${escapeHtml(entry.name)}</span>${entry.readonly ? '<span class="badge">read-only</span>' : ""}`;
      nameCell.append(nameWrap);
      if (entry.isDir) {
        nameCell.addEventListener("click", () => {
          path = entry.path;
          load();
        });
      }
      const sizeCell = el("td", { class: "num muted", text: entry.isDir ? "-" : formatBytes(entry.size) });
      const timeCell = el("td", { class: "muted", text: entry.modified ? timeAgo(entry.modified) : "-" });
      const actionCell = el("td");
      actionCell.append(rowActions(entry));
      tr.append(nameCell, sizeCell, timeCell, actionCell);
      tbody.append(tr);
    }
    table.append(tbody);
    listNode.replaceChildren(table);
  }

  async function load() {
    renderCrumbs();
    listNode.innerHTML = `<div style="padding:16px"><div class="skeleton skeleton-line" style="width:50%;margin-bottom:10px"></div><div class="skeleton skeleton-line" style="width:34%;margin-bottom:10px"></div><div class="skeleton skeleton-line" style="width:42%"></div></div>`;
    try {
      const query = new URLSearchParams();
      if (searchMode) {
        query.set("q", searchInput.value.trim());
        const results = await api.get(`/api/servers/${server.id}/files/search?${query}`);
        listing = {
          entries: results,
          path: "",
          totalSize: 0,
        };
      } else {
        query.set("path", path);
        listing = await api.get(`/api/servers/${server.id}/files?${query}`);
      }
      if (disposed) return;
      renderListing();
    } catch (err) {
      listNode.innerHTML = emptyState({
        icon: "alert",
        title: "Cannot open this folder",
        text: errorMessage(err),
        actionHtml: `<button class="btn" type="button" id="files-retry">Retry</button>`,
      });
      listNode.querySelector("#files-retry")?.addEventListener("click", load);
    }
  }

  async function openEditor(entry) {
    const body = el("div", { class: "col" });
    const textarea = el("textarea", { class: "textarea editor-textarea", spellcheck: "false" });
    textarea.value = "Loading…";
    textarea.disabled = true;
    const info = el("div", { class: "row", style: "justify-content:space-between" });
    info.innerHTML = `<span class="card-sub mono">${escapeHtml(entry.path)}</span><span class="card-sub" id="editor-size"></span>`;
    body.append(info, textarea);
    const cancel = el("button", { class: "btn", type: "button", text: "Close" });
    const save = el("button", {
      class: "btn btn-primary",
      type: "button",
      html: `${icon("save", 13)}<span>Save</span>`,
      disabled: true,
    });
    const modal = openModal({ title: "Edit file", body, footer: [cancel, save], wide: true });
    cancel.addEventListener("click", () => modal.close());

    try {
      const file = await api.get(
        `/api/servers/${server.id}/files/content?path=${encodeURIComponent(entry.path)}`
      );
      textarea.value = file.content;
      textarea.disabled = false;
      save.disabled = false;
      body.querySelector("#editor-size").textContent = formatBytes(file.size);
      textarea.focus();
    } catch (err) {
      textarea.value = "";
      body.append(errorBox(errorMessage(err)));
      return;
    }

    save.addEventListener("click", async () => {
      setBusy(save, true, "Saving");
      try {
        await api.put(`/api/servers/${server.id}/files/content`, {
          path: entry.path,
          content: textarea.value,
        });
        toast("File saved", "success");
        modal.close();
        load();
      } catch (err) {
        toast(errorMessage(err), "error", 7000);
        setBusy(save, false);
      }
    });
  }

  async function createEntry(kind) {
    const body = el("div", { class: "col" });
    const input = el("input", { class: "input", placeholder: kind === "dir" ? "folder name" : "file name" });
    body.append(field(kind === "dir" ? "New folder" : "New file", input, path ? `Inside ${path}` : "Inside the server root"));
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const submit = el("button", { class: "btn btn-primary", type: "button", text: "Create" });
    const modal = openModal({ title: kind === "dir" ? "New folder" : "New file", body, footer: [cancel, submit], narrow: true });
    cancel.addEventListener("click", () => modal.close());
    const create = async () => {
      const name = input.value.trim();
      if (!name) return;
      const target = path ? `${path}/${name}` : name;
      try {
        await api.post(`/api/servers/${server.id}/files/${kind === "dir" ? "mkdir" : "touch"}`, {
          path: target,
        });
        modal.close();
        toast(`${kind === "dir" ? "Folder" : "File"} created`, "success");
        load();
      } catch (err) {
        toast(errorMessage(err), "error", 6000);
      }
    };
    submit.addEventListener("click", create);
    input.addEventListener("keydown", (event) => {
      if (event.key === "Enter") create();
    });
    setTimeout(() => input.focus(), 40);
  }

  async function renameEntry(entry) {
    const body = el("div", { class: "col" });
    const input = el("input", { class: "input", value: entry.name });
    body.append(field("Rename", input, `Moving ${entry.path}`));
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const submit = el("button", { class: "btn btn-primary", type: "button", text: "Rename" });
    const modal = openModal({ title: "Rename", body, footer: [cancel, submit], narrow: true });
    cancel.addEventListener("click", () => modal.close());
    submit.addEventListener("click", async () => {
      const name = input.value.trim();
      if (!name || name === entry.name) {
        modal.close();
        return;
      }
      const parent = entry.path.includes("/") ? entry.path.slice(0, entry.path.lastIndexOf("/")) : "";
      const target = parent ? `${parent}/${name}` : name;
      try {
        await api.post(`/api/servers/${server.id}/files/rename`, { from: entry.path, to: target });
        modal.close();
        toast("Renamed", "success");
        load();
      } catch (err) {
        toast(errorMessage(err), "error", 6000);
      }
    });
    setTimeout(() => input.select(), 40);
  }

  async function deleteEntry(entry) {
    const ok = await confirmDialog({
      title: entry.isDir ? "Delete folder" : "Delete file",
      message: entry.isDir
        ? `${entry.path} and everything inside it will be deleted permanently.`
        : `${entry.path} will be deleted permanently.`,
      confirmLabel: "Delete",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.post(`/api/servers/${server.id}/files/delete`, {
        path: entry.path,
        recursive: entry.isDir,
      });
      toast("Deleted", "success");
      load();
    } catch (err) {
      toast(errorMessage(err), "error", 6000);
    }
  }

  async function uploadFiles(files) {
    if (!files || !files.length) return;
    let uploaded = 0;
    for (const file of files) {
      try {
        const query = new URLSearchParams({
          dir: path,
          name: file.name,
          overwrite: "true",
        });
        await api.upload(`/api/servers/${server.id}/files/upload?${query}`, file);
        uploaded += 1;
      } catch (err) {
        toast(`${file.name}: ${errorMessage(err)}`, "error", 7000);
      }
    }
    if (uploaded) {
      toast(`${uploaded} file${uploaded === 1 ? "" : "s"} uploaded`, "success");
      load();
    }
  }

  container.querySelector("#file-refresh").addEventListener("click", load);
  container.querySelector("#file-upload")?.addEventListener("click", () => uploadInput.click());
  container.querySelector("#file-mkdir")?.addEventListener("click", () => createEntry("dir"));
  uploadInput.addEventListener("change", () => {
    uploadFiles([...uploadInput.files]);
    uploadInput.value = "";
  });

  searchInput.addEventListener(
    "input",
    debounce(() => {
      const query = searchInput.value.trim();
      if (query.length >= 2) {
        searchMode = true;
        path = "";
        load();
      } else if (searchMode) {
        searchMode = false;
        load();
      }
    }, 300)
  );

  const dropzone = el("div", { class: "dropzone hidden" });
  dropzone.innerHTML = `${icon("upload", 22)}<span>Drop files to upload${path ? ` into /${path}` : ""}</span>`;
  container.querySelector("#files-card").append(dropzone);
  const card = container.querySelector("#files-card");

  const onDragOver = (event) => {
    if (!canWrite) return;
    event.preventDefault();
    dropzone.classList.remove("hidden");
  };
  const onDragLeave = (event) => {
    if (event.relatedTarget && card.contains(event.relatedTarget)) return;
    dropzone.classList.add("hidden");
  };
  const onDrop = (event) => {
    event.preventDefault();
    dropzone.classList.add("hidden");
    if (!canWrite) return;
    uploadFiles([...(event.dataTransfer?.files || [])]);
  };
  card.addEventListener("dragover", onDragOver);
  card.addEventListener("dragleave", onDragLeave);
  card.addEventListener("drop", onDrop);

  await load();

  return () => {
    disposed = true;
    card.removeEventListener("dragover", onDragOver);
    card.removeEventListener("dragleave", onDragLeave);
    card.removeEventListener("drop", onDrop);
  };
}

/* --------------------------------------------------------------- backups */

function backupsTab(container, server) {
  let disposed = false;
  container.innerHTML = `
    <div class="card flush">
      <div class="card-head" style="padding:16px;margin:0">
        <div class="card-title">Backups</div>
        <div class="card-actions">
          <input class="input" id="backup-name" placeholder="Backup name (optional)" style="width:200px;height:27px">
          <button class="btn btn-sm btn-primary" type="button" id="backup-create">${icon("plus", 13)}<span>Create</span></button>
        </div>
      </div>
      <div id="backup-list"><div style="padding:16px"><div class="skeleton skeleton-line" style="width:60%"></div></div></div>
    </div>
    <div class="inline-note" style="margin-top:12px">${icon("info", 15)}<span>Backups are zip archives of the server directory stored on this machine. Restoring stops the file swap until the server is offline.</span></div>`;

  const listNode = container.querySelector("#backup-list");
  const nameInput = container.querySelector("#backup-name");

  async function load() {
    try {
      const backups = await api.get(`/api/servers/${server.id}/backups`);
      if (disposed) return;
      if (!backups.length) {
        listNode.innerHTML = emptyState({
          icon: "archive",
          title: "No backups yet",
          text: "Create one before updating plugins, worlds or configuration.",
        });
        return;
      }
      const table = el("table", { class: "table" });
      table.innerHTML = `<thead><tr><th>Name</th><th style="width:110px">Size</th><th style="width:170px">Created</th><th style="width:90px">Status</th><th style="width:120px"></th></tr></thead>`;
      const tbody = el("tbody");
      for (const backup of backups) {
        const tr = el("tr");
        tr.append(
          el("td", {}, [el("span", { text: backup.name })]),
          el("td", { class: "num muted", text: formatBytes(backup.size) }),
          el("td", { class: "muted", text: formatDateTime(backup.createdAt) }),
          el("td", {}, [
            el("span", {
              class: `badge ${backup.status === "complete" ? "ok" : backup.status === "failed" ? "danger" : "warn"}`,
              text: backup.status,
            }),
          ])
        );
        const actions = el("td", {}, [
          el("div", { class: "table-actions" }, [
            el("button", {
              class: "icon-btn",
              type: "button",
              title: "Restore backup",
              html: icon("restart", 13),
              onclick: () => restore(backup),
            }),
            el("button", {
              class: "icon-btn danger",
              type: "button",
              title: "Delete backup",
              html: icon("trash", 13),
              onclick: () => remove(backup),
            }),
          ]),
        ]);
        tr.append(actions);
        tbody.append(tr);
      }
      table.append(tbody);
      listNode.replaceChildren(table);
    } catch (err) {
      listNode.innerHTML = emptyState({ icon: "alert", title: "Could not load backups", text: errorMessage(err) });
    }
  }

  async function restore(backup) {
    const ok = await confirmDialog({
      title: "Restore backup",
      message: `Replace the current files of ${server.name} with "${backup.name}"? The server must be stopped. Current files are replaced.`,
      confirmLabel: "Restore",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.post(`/api/servers/${server.id}/backups/${backup.id}/restore`);
      toast("Backup restored", "success");
    } catch (err) {
      toast(errorMessage(err), "error", 8000);
    }
  }

  async function remove(backup) {
    const ok = await confirmDialog({
      title: "Delete backup",
      message: `Delete "${backup.name}" permanently?`,
      confirmLabel: "Delete",
      danger: true,
    });
    if (!ok) return;
    try {
      await api.del(`/api/servers/${server.id}/backups/${backup.id}`);
      toast("Backup deleted", "success");
      load();
    } catch (err) {
      toast(errorMessage(err), "error", 6000);
    }
  }

  container.querySelector("#backup-create").addEventListener("click", async (event) => {
    const button = event.currentTarget;
    setBusy(button, true, "Starting");
    try {
      await api.post(`/api/servers/${server.id}/backups`, {
        name: nameInput.value.trim() || undefined,
      });
      nameInput.value = "";
      toast("Backup started", "success");
      setTimeout(load, 800);
    } catch (err) {
      toast(errorMessage(err), "error", 7000);
    } finally {
      setBusy(button, false);
    }
  });

  const unsubscribe = onServerEvent((event) => {
    if (event.serverId === server.id && event.type === "backup") load();
  });

  load();
  const timer = setInterval(() => {
    if (!document.hidden) load();
  }, 10000);

  return () => {
    disposed = true;
    unsubscribe();
    clearInterval(timer);
  };
}

/* -------------------------------------------------------------- settings */

async function settingsTab(container, server, reload) {
  const canManage = server.level === "manage";
  if (!canManage) {
    container.innerHTML = `<div class="card">${emptyState({
      icon: "shield",
      title: "Read-only access",
      text: "Managing server settings requires the manage access level.",
    })}</div>`;
    return null;
  }

  const env = envEditor(server.env || []);
  const ports = portEditor(server.ports || []);

  const name = el("input", { class: "input", value: server.name, maxlength: "64" });
  const description = el("input", { class: "input", value: server.description || "", maxlength: "200" });
  const exe = el("input", { class: "input", value: server.startupExe || "", spellcheck: "false" });
  const args = el("input", { class: "input", value: server.startupArgs || "", spellcheck: "false" });
  const memory = el("input", { class: "input", type: "number", min: "0", step: "128", value: server.memoryMb });
  const storage = el("input", { class: "input", type: "number", min: "0", step: "1024", value: server.storageMb });
  const cores = el("input", { class: "input", value: formatCores(server.cpuCores), placeholder: "All cores" });
  const cpuLimit = el("input", { class: "input", type: "number", min: "0", max: "100", value: server.cpuLimitPercent });
  const stopCommand = el("input", { class: "input", value: server.stopCommand || "", placeholder: "stop" });
  const stopTimeout = el("input", { class: "input", type: "number", min: "2", max: "300", value: Math.round(server.stopTimeoutMs / 1000) });
  const autoRestart = el("input", { type: "checkbox" });
  autoRestart.checked = !!server.autoRestart;
  const restartMax = el("input", { class: "input", type: "number", min: "0", max: "50", value: server.autoRestartMax });
  const restartDelay = el("input", { class: "input", type: "number", min: "1", max: "300", value: Math.round(server.autoRestartDelayMs / 1000) });
  const backupsEnabled = el("input", { type: "checkbox" });
  backupsEnabled.checked = !!server.backupsEnabled;
  const backupInterval = el("input", { class: "input", type: "number", min: "1", max: "720", value: server.backupIntervalHours });
  const backupKeep = el("input", { class: "input", type: "number", min: "1", max: "100", value: server.backupKeep });

  container.innerHTML = "";
  const card = el("div", { class: "card" });
  const errorSlot = el("div", { class: "hidden" });

  card.append(
    errorSlot,
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Identity" })]),
      el("div", { class: "form-grid" }, [field("Server name", name), field("Description", description)]),
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [
        el("h3", { text: "Startup" }),
        el("span", { text: "The command is executed directly - no shell" }),
      ]),
      el("div", { class: "form-grid" }, [
        field("Executable", exe, "Absolute path, relative file or a command on PATH."),
        field("Arguments", args, "Supports quotes. {{memoryMb}} was resolved at install time."),
        field("Stop command", stopCommand, "Sent to stdin before shutting down."),
        field("Stop timeout (seconds)", stopTimeout),
      ]),
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Resources" })]),
      el("div", { class: "form-grid" }, [
        field("Memory limit (MB)", memory, "0 = unlimited. Applied through a Windows Job Object."),
        field("Storage limit (MB)", storage, "0 = unlimited. Enforced for uploads and edits."),
        field("CPU cores", cores, "Example: 0-3 or 0,2,4"),
        field("CPU limit (%)", cpuLimit, "Hard CPU cap, 0 = unlimited."),
      ]),
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Network" })]),
      ports.node,
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Environment variables" })]),
      env.node,
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Automation" })]),
      el("label", { class: "switch" }, [autoRestart, el("span", { text: "Restart after crashes" })]),
      el("div", { class: "form-grid", style: "margin-top:12px" }, [
        field("Max restart attempts", restartMax),
        field("Restart delay (seconds)", restartDelay),
      ]),
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Backups" })]),
      el("label", { class: "switch" }, [backupsEnabled, el("span", { text: "Scheduled backups enabled" })]),
      el("div", { class: "form-grid", style: "margin-top:12px" }, [
        field("Interval (hours)", backupInterval),
        field("Keep last", backupKeep),
      ]),
    ])
  );

  const save = el("button", {
    class: "btn btn-primary",
    type: "button",
    html: `${icon("save", 14)}<span>Save changes</span>`,
  });
  const actions = el("div", { class: "form-actions" }, [
    save,
    el("span", { class: "card-sub", text: "Resource changes apply the next time the server starts." }),
  ]);
  card.append(actions);

  save.addEventListener("click", async () => {
    errorSlot.className = "hidden";
    errorSlot.replaceChildren();
    const payload = {
      name: name.value.trim(),
      description: description.value.trim(),
      templateId: server.templateId,
      startupExe: exe.value.trim(),
      startupArgs: args.value,
      env: env.read(),
      memoryMb: Number(memory.value) || 0,
      cpuCores: parseCores(cores.value),
      cpuLimitPercent: Number(cpuLimit.value) || 0,
      storageMb: Number(storage.value) || 0,
      ports: ports.read(),
      autoRestart: autoRestart.checked,
      autoRestartMax: Number(restartMax.value) || 0,
      autoRestartDelayMs: (Number(restartDelay.value) || 5) * 1000,
      stopCommand: stopCommand.value,
      stopTimeoutMs: (Number(stopTimeout.value) || 30) * 1000,
      backupsEnabled: backupsEnabled.checked,
      backupIntervalHours: Number(backupInterval.value) || 24,
      backupKeep: Number(backupKeep.value) || 5,
    };
    setBusy(save, true, "Saving");
    try {
      await api.patch(`/api/servers/${server.id}`, payload);
      toast("Settings saved", "success");
      await reload();
      await refreshServers().catch(() => {});
    } catch (err) {
      errorSlot.className = "inline-error";
      errorSlot.replaceChildren(errorBox(errorMessage(err)));
      errorSlot.scrollIntoView({ block: "nearest" });
    } finally {
      setBusy(save, false);
    }
  });

  container.append(card);

  // Danger zone
  const danger = el("div", { class: "card", style: "margin-top:14px" });
  danger.innerHTML = `<div class="card-head"><div class="card-title">Maintenance</div></div>
    <div class="card-sub" style="margin-bottom:12px">Reinstalling wipes the server files. Deleting removes the server, its files and backups.</div>`;
  const dangerRow = el("div", { class: "row wrap" });
  dangerRow.append(
    el("button", {
      class: "btn btn-sm",
      type: "button",
      html: `${icon("download", 13)}<span>Reinstall</span>`,
      onclick: () => reinstall(server, reload),
    }),
    el("button", {
      class: "btn btn-sm",
      type: "button",
      html: `${icon("copy", 13)}<span>Clone</span>`,
      onclick: () => cloneServer(server),
    }),
    el("button", {
      class: "btn btn-sm btn-danger",
      type: "button",
      html: `${icon("trash", 13)}<span>Delete server</span>`,
      onclick: () => deleteServer(server),
    })
  );
  danger.append(dangerRow);
  container.append(danger);

  return null;
}

/* -------------------------------------------------------------- activity */

async function activityTab(container, server) {
  container.innerHTML = `<div class="card"><div class="card-head"><div class="card-title">Activity</div></div><div id="activity-body"><div class="skeleton skeleton-line" style="width:50%"></div></div></div>`;
  const body = container.querySelector("#activity-body");
  let disposed = false;

  async function load() {
    try {
      const entries = await api.get(`/api/servers/${server.id}/activity?limit=100`);
      if (disposed) return;
      body.replaceChildren(
        activityTimeline(entries, { empty: "No activity recorded for this server yet." })
      );
      body.querySelector(".empty-text")?.classList.add("muted");
    } catch (err) {
      body.innerHTML = `<div class="inline-error">${escapeHtml(errorMessage(err))}</div>`;
    }
  }

  await load();
  const timer = setInterval(() => {
    if (!document.hidden) load();
  }, 15000);
  return () => {
    disposed = true;
    clearInterval(timer);
  };
}
