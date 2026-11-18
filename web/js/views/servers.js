// Server list and the create-server flow.

import { api, errorMessage } from "../api.js";
import {
  el,
  escapeHtml,
  icon,
  openModal,
  setBusy,
  skeletonStats,
  toast,
  emptyState,
  formatBytes,
} from "../ui.js";
import { loadCoreData, onServerEvent, refreshServers, state, isAdmin } from "../app.js";
import {
  errorBox,
  field,
  formatCores,
  pageHeader,
  parseCores,
  serverRow,
  statusOf,
  templateName,
} from "./common.js";

export async function renderServers(view) {
  view.innerHTML =
    pageHeader({
      title: "Servers",
      subtitle: "Every server managed by this panel",
      actions: `${isAdmin() ? `<button class="btn btn-primary" type="button" id="new-server">${icon("plus", 14)}<span>New server</span></button>` : ""}`,
    }) +
    `<div class="row wrap" style="margin-bottom:12px">
       <div class="search-inline">${icon("search", 14)}<input id="server-search" placeholder="Search servers" autocomplete="off"></div>
       <div class="chip-row" id="server-filters"></div>
       <div class="spacer"></div>
       <button class="btn btn-sm" type="button" id="server-refresh">${icon("refresh", 13)}<span>Refresh</span></button>
     </div>
     <div class="card flush" id="server-list">${skeletonStats(1)}</div>`;

  let servers = [];
  let filter = "all";
  let query = "";
  let disposed = false;

  const listNode = view.querySelector("#server-list");
  const filtersNode = view.querySelector("#server-filters");
  const searchInput = view.querySelector("#server-search");

  view.querySelector("#new-server")?.addEventListener("click", () => openCreateServer());
  view.querySelector("#server-refresh")?.addEventListener("click", () => load(true));

  function counts() {
    return {
      all: servers.length,
      running: servers.filter((s) => statusOf(s) === "running").length,
      stopped: servers.filter((s) => ["stopped", "stopping"].includes(statusOf(s))).length,
      issues: servers.filter((s) => ["crashed", "install_failed"].includes(statusOf(s))).length,
    };
  }

  function renderFilters() {
    const entries = [
      ["all", "All"],
      ["running", "Running"],
      ["stopped", "Stopped"],
      ["issues", "Issues"],
    ];
    const totals = counts();
    filtersNode.innerHTML = "";
    for (const [key, label] of entries) {
      const chip = el("button", {
        class: `chip ${filter === key ? "active" : ""}`,
        type: "button",
        html: `${escapeHtml(label)} <span class="count">${totals[key]}</span>`,
        onclick: () => {
          filter = key;
          renderFilters();
          renderList();
        },
      });
      filtersNode.append(chip);
    }
  }

  function matches(server) {
    const status = statusOf(server);
    if (filter === "running" && status !== "running") return false;
    if (filter === "stopped" && !["stopped", "stopping"].includes(status)) return false;
    if (filter === "issues" && !["crashed", "install_failed"].includes(status)) return false;
    if (!query) return true;
    const haystack = `${server.name} ${server.description} ${templateName(server.templateId)}`.toLowerCase();
    return haystack.includes(query);
  }

  function renderList() {
    const visible = servers.filter(matches);
    if (servers.length === 0) {
      listNode.innerHTML = emptyState({
        icon: "server",
        title: "No servers yet",
        text: "Create a server from a template to get started. Installation, startup and monitoring are handled for you.",
        actionHtml: isAdmin()
          ? `<button class="btn btn-primary" type="button" id="empty-new">${icon("plus", 14)}<span>New server</span></button>`
          : "",
      });
      listNode.querySelector("#empty-new")?.addEventListener("click", () => openCreateServer());
      return;
    }
    if (visible.length === 0) {
      listNode.innerHTML = emptyState({
        icon: "search",
        title: "No matching servers",
        text: "Adjust the search or filter to see your servers.",
      });
      return;
    }

    const list = el("div", { class: "list" });
    for (const server of visible) {
      list.append(serverRow(server));
    }
    listNode.replaceChildren(list);
  }

  async function load(withSkeleton = false) {
    if (withSkeleton) listNode.innerHTML = `<div style="padding:16px">${skeletonStats(1)}</div>`;
    try {
      servers = await refreshServers();
      if (disposed) return;
      renderFilters();
      renderList();
    } catch (err) {
      toast(errorMessage(err), "error");
    }
  }

  searchInput.addEventListener(
    "input",
    debounceInput(() => {
      query = searchInput.value.trim().toLowerCase();
      renderList();
    })
  );

  await load();
  const timer = setInterval(() => {
    if (!document.hidden) load();
  }, 15000);
  const unsubscribe = onServerEvent((event) => {
    if (["status", "install", "servers"].includes(event.type)) load();
  });

  return () => {
    disposed = true;
    clearInterval(timer);
    unsubscribe();
  };
}

function debounceInput(fn, wait = 160) {
  let timer;
  return (...args) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), wait);
  };
}

/* --------------------------------------------------------- create server */

export async function openCreateServer(preselectId = null) {
  if (!isAdmin()) {
    toast("Only administrators can create servers", "error");
    return;
  }
  if (!state.templates.length) {
    await loadCoreData().catch(() => {});
  }
  const templates = state.templates;
  if (!templates.length) {
    toast("No server templates are available", "error");
    return;
  }

  let selectedId = preselectId && templates.some((t) => t.id === preselectId) ? preselectId : templates[0].id;
  let installOptions = {};

  const body = el("div", { class: "col", style: "gap:16px" });
  const errorSlot = el("div", { class: "hidden" });
  const choiceGrid = el("div", { class: "choice-grid" });
  const optionsWrap = el("div", { class: "template-option-grid" });
  const notes = el("div", { class: "inline-note hidden" });

  const nameInput = el("input", { class: "input", placeholder: "My Minecraft server", maxlength: "64", autofocus: true });
  const descInput = el("input", { class: "input", placeholder: "Optional description", maxlength: "200" });
  const portInput = el("input", { class: "input", type: "number", min: "1", max: "65535" });
  const protocolSelect = el("select", { class: "select" }, [
    el("option", { value: "tcp", text: "TCP" }),
    el("option", { value: "udp", text: "UDP" }),
  ]);
  const memoryInput = el("input", { class: "input", type: "number", min: "128", step: "128" });
  const storageInput = el("input", { class: "input", type: "number", min: "0", step: "1024" });
  const coresInput = el("input", { class: "input", placeholder: "All cores (e.g. 0-3 or 0,2,4)" });
  const cpuLimitInput = el("input", { class: "input", type: "number", min: "0", max: "100", value: "0" });
  const exeInput = el("input", { class: "input", placeholder: "Template default" });
  const argsInput = el("input", { class: "input", placeholder: "Template default" });
  const restartSwitch = el("input", { type: "checkbox" });
  const restartMax = el("input", { class: "input", type: "number", min: "0", max: "50", value: "3" });
  const restartDelay = el("input", { class: "input", type: "number", min: "1", max: "300", value: "5" });
  const backupSwitch = el("input", { type: "checkbox" });

  const templateById = (id) => templates.find((template) => template.id === id) || templates[0];

  function applyTemplate() {
    const template = templateById(selectedId);
    portInput.value = template.port || 25565;
    protocolSelect.value = (template.protocol || "tcp").toLowerCase();
    memoryInput.value = template.memoryMb || 2048;
    storageInput.value = template.storageMb || 10240;
    exeInput.value = "";
    argsInput.value = "";
    exeInput.placeholder = template.startupExe || "Set after upload";
    argsInput.placeholder = template.startupArgs || "No arguments";
    notes.className = template.notes ? "inline-note" : "inline-note hidden";
    notes.innerHTML = template.notes ? `${icon("info", 15)}<span>${escapeHtml(template.notes)}</span>` : "";

    installOptions = {};
    optionsWrap.innerHTML = "";
    for (const option of template.options || []) {
      installOptions[option.id] = option.default ?? "";
      optionsWrap.append(renderTemplateOption(option));
    }
    if (!(template.options || []).length) {
      optionsWrap.append(el("div", { class: "field-hint", text: "This template has no additional options." }));
    }
  }

  function renderTemplateOption(option) {
    const value = installOptions[option.id] ?? "";
    const wrap = el("div", { class: "field" + (option.kind === "bool" ? "" : "") });
    wrap.append(el("label", { class: "field-label", text: option.label + (option.required ? " *" : "") }));

    if (option.kind === "bool") {
      const toggle = el("input", { type: "checkbox" });
      toggle.checked = String(value).toLowerCase() !== "false";
      installOptions[option.id] = toggle.checked ? "true" : "false";
      toggle.addEventListener("change", () => {
        installOptions[option.id] = toggle.checked ? "true" : "false";
      });
      wrap.append(el("label", { class: "switch" }, [toggle, el("span", { text: "Enabled" })]));
    } else if (option.kind === "select") {
      const select = el("select", { class: "select" });
      for (const choice of option.choices || []) {
        select.append(el("option", { value: choice.value, text: choice.label }));
      }
      select.value = value || option.default || "";
      installOptions[option.id] = select.value;
      select.addEventListener("change", () => {
        installOptions[option.id] = select.value;
      });
      wrap.append(select);
    } else {
      const input = el("input", {
        class: "input",
        type: option.kind === "password" ? "password" : option.kind === "number" ? "number" : "text",
        value,
        autocomplete: "off",
        spellcheck: "false",
      });
      input.addEventListener("input", () => {
        installOptions[option.id] = input.value;
      });
      wrap.append(input);
    }
    if (option.help) wrap.append(el("div", { class: "field-hint", text: option.help }));
    return wrap;
  }

  function renderChoices() {
    choiceGrid.innerHTML = "";
    for (const template of templates) {
      const button = el("button", {
        class: `choice ${template.id === selectedId ? "active" : ""}`,
        type: "button",
      });
      button.innerHTML = `
        <span class="choice-icon">${escapeHtml((template.name || "?").slice(0, 1))}</span>
        <span style="min-width:0">
          <span class="choice-name truncate">${escapeHtml(template.name)}</span>
          <span class="choice-meta">${escapeHtml(template.category || "Application")}</span>
        </span>`;
      button.addEventListener("click", () => {
        selectedId = template.id;
        renderChoices();
        applyTemplate();
      });
      choiceGrid.append(button);
    }
  }

  const advanced = el("details", {});
  advanced.innerHTML = `<summary class="field-label" style="cursor:pointer;padding:4px 0">Startup command and automation</summary>`;
  const advancedBody = el("div", { class: "col", style: "gap:14px;margin-top:12px" });
  advancedBody.append(
    el("div", { class: "form-grid" }, [
      field("Startup executable", exeInput, "Leave blank to use the template default."),
      field("Startup arguments", argsInput, "Quotes are supported; no shell is used."),
    ]),
    el("div", { class: "row wrap" }, [
      el("label", { class: "switch" }, [restartSwitch, el("span", { text: "Restart automatically after a crash" })]),
      el("div", { class: "form-grid", style: "flex:1;min-width:220px" }, [
        field("Max restarts", restartMax),
        field("Delay (seconds)", restartDelay),
      ]),
    ]),
    el("label", { class: "switch" }, [backupSwitch, el("span", { text: "Enable scheduled backups" })])
  );
  advanced.append(advancedBody);

  body.append(
    errorSlot,
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [
        el("h3", { text: "Template" }),
        el("span", { text: "Pick what you want to run" }),
      ]),
      choiceGrid,
      optionsWrap,
      notes,
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [el("h3", { text: "Identity" })]),
      el("div", { class: "form-grid" }, [
        field("Server name", nameInput, "Used for the folder name and in the panel."),
        field("Description", descInput),
      ]),
    ]),
    el("div", { class: "form-section" }, [
      el("div", { class: "form-section-head" }, [
        el("h3", { text: "Resources and network" }),
        el("span", { text: "Limits are enforced on Windows" }),
      ]),
      el("div", { class: "form-grid" }, [
        field("Memory limit (MB)", memoryInput, "0 disables the limit."),
        field("Storage limit (MB)", storageInput, "Advisory quota for uploads and edits."),
        field("CPU cores", coresInput, "Empty uses all cores."),
        field("CPU limit (%)", cpuLimitInput, "Hard cap across all cores, 0 = unlimited."),
        field("Primary port", portInput),
        field("Protocol", protocolSelect),
      ]),
    ]),
    advanced
  );

  const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
  const create = el("button", {
    class: "btn btn-primary",
    type: "button",
    html: `${icon("plus", 14)}<span>Create and install</span>`,
  });

  const modal = openModal({
    title: "New server",
    subtitle: "Choose a template and configure limits",
    body,
    footer: [cancel, create],
    wide: true,
  });

  cancel.addEventListener("click", () => modal.close());
  applyTemplate();
  renderChoices();

  create.addEventListener("click", async () => {
    errorSlot.classList.add("hidden");
    errorSlot.replaceChildren();
    const name = nameInput.value.trim();
    if (!name) {
      errorSlot.className = "inline-error";
      errorSlot.append(errorBox("Give the server a name first."));
      nameInput.focus();
      return;
    }
    const template = templateById(selectedId);
    const payload = {
      name,
      description: descInput.value.trim(),
      templateId: selectedId,
      startupExe: exeInput.value.trim(),
      startupArgs: argsInput.value.trim(),
      memoryMb: Number(memoryInput.value) || template.memoryMb || 2048,
      storageMb: Number(storageInput.value) || 0,
      cpuCores: parseCores(coresInput.value),
      cpuLimitPercent: Number(cpuLimitInput.value) || 0,
      ports: [
        {
          label: "Game",
          port: Number(portInput.value) || template.port,
          protocol: protocolSelect.value,
        },
      ],
      env: template.env || [],
      autoRestart: restartSwitch.checked,
      autoRestartMax: Number(restartMax.value) || 0,
      autoRestartDelayMs: (Number(restartDelay.value) || 5) * 1000,
      stopCommand: template.stopCommand || "",
      stopTimeoutMs: 30000,
      backupsEnabled: backupSwitch.checked,
      backupIntervalHours: 24,
      backupKeep: 5,
      installOptions,
    };
    setBusy(create, true, "Creating");
    try {
      const server = await api.post("/api/servers", payload);
      modal.close();
      toast(`${server.name} created — installing now`, "success");
      await refreshServers().catch(() => {});
      location.hash = `#/servers/${server.id}`;
    } catch (err) {
      errorSlot.className = "inline-error";
      errorSlot.replaceChildren(errorBox(errorMessage(err)));
      setBusy(create, false);
    }
  });
}
