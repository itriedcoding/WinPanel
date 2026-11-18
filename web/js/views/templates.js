// Server template catalog with live runtime detection.

import { api, errorMessage } from "../api.js";
import { el, escapeHtml, icon, openModal, toast, emptyState } from "../ui.js";
import { isAdmin, state, loadCoreData } from "../app.js";
import { pageHeader, noteBox } from "./common.js";
import { openCreateServer } from "./servers.js";

export async function renderTemplates(view) {
  view.innerHTML =
    pageHeader({
      title: "Templates",
      subtitle: "Installable server applications and their runtime requirements",
      actions: isAdmin()
        ? `<button class="btn" type="button" id="tpl-reload">${icon("refresh", 13)}<span>Reload templates</span></button>`
        : "",
    }) +
    `<div id="tpl-body">
      <div class="template-grid">
        ${Array.from({ length: 6 }).map(() => `<div class="skeleton" style="height:170px;border-radius:8px"></div>`).join("")}
      </div>
    </div>`;

  const body = view.querySelector("#tpl-body");

  async function load() {
    try {
      const [report] = await Promise.all([api.get("/api/system/runtimes"), loadCoreData().catch(() => {})]);
      state.templates = report.templates || [];
      render(report);
    } catch (err) {
      body.innerHTML = emptyState({
        icon: "alert",
        title: "Could not load templates",
        text: errorMessage(err),
        actionHtml: `<button class="btn" type="button" id="tpl-retry">Retry</button>`,
      });
      body.querySelector("#tpl-retry")?.addEventListener("click", load);
    }
  }

  function render(report) {
    const templates = report.templates || [];
    const runtimes = report.runtimes || [];
    const warnings = report.warnings || [];

    body.innerHTML = "";

    const runtimeCard = el("div", { class: "card" });
    runtimeCard.innerHTML = `
      <div class="card-head">
        <div>
          <div class="card-title">Installed runtimes</div>
          <div class="card-sub">Detected on this machine — required by some templates</div>
        </div>
      </div>
      <div class="row wrap" style="gap:20px">
        ${runtimes
          .map(
            (runtime) => `<div class="row" style="gap:9px;min-width:180px">
              <span class="badge ${runtime.available ? "ok" : "danger"}">${runtime.available ? icon("check", 11) : icon("x", 11)}</span>
              <div>
                <div style="font-size:12.5px">${escapeHtml(runtime.label)}</div>
                <div class="card-sub">${runtime.available ? escapeHtml(runtime.version || "installed") : escapeHtml(runtime.hint)}</div>
              </div>
            </div>`
          )
          .join("")}
      </div>`;

    const categories = new Map();
    for (const template of templates) {
      const key = template.category || "Applications";
      if (!categories.has(key)) categories.set(key, []);
      categories.get(key).push(template);
    }

    for (const [category, items] of categories) {
      const section = el("div", { style: "margin-top:18px" });
      section.innerHTML = `<div class="row" style="margin-bottom:10px">
        <h2 class="card-title">${escapeHtml(category)}</h2>
        <span class="card-sub">${items.length} template${items.length === 1 ? "" : "s"}</span>
      </div>`;
      const grid = el("div", { class: "template-grid" });

      for (const template of items) {
        const requirements = template.runtimeStatus || [];
        const missing = requirements.filter((runtime) => !runtime.available);
        const card = el("div", { class: "template-card" });
        card.innerHTML = `
          <div class="template-top">
            <span class="template-icon">${escapeHtml((template.name || "?").slice(0, 1))}</span>
            <div style="min-width:0">
              <div class="template-name">${escapeHtml(template.name)}</div>
              <div class="template-vendor">${escapeHtml(template.vendor || "Custom")}${template.source === "builtin" ? "" : ` · ${escapeHtml(template.source)}`}</div>
            </div>
          </div>
          <div class="template-desc">${escapeHtml(template.description || "")}</div>
          <div class="template-foot">
            <span class="badge">${escapeHtml(installLabel(template.install?.method))}</span>
            ${template.port ? `<span class="badge">port ${template.port}/${escapeHtml(template.protocol || "tcp")}</span>` : ""}
            ${template.memoryMb ? `<span class="badge">${template.memoryMb} MB</span>` : ""}
            ${requirements.map((runtime) => `<span class="badge ${runtime.available ? "ok" : "warn"}">${escapeHtml(runtime.label)}</span>`).join("")}
          </div>
          <div class="row" style="gap:6px">
            <button class="btn btn-sm btn-primary" type="button">${icon("plus", 12)}<span>Use template</span></button>
            <button class="btn btn-sm" type="button" data-details>${icon("info", 12)}<span>Details</span></button>
          </div>`;
        const useButton = card.querySelector("button");
        useButton.disabled = !isAdmin() || missing.length > 0;
        useButton.title = !isAdmin()
          ? "Administrators create servers"
          : missing.length
            ? `Install ${missing.map((m) => m.label).join(", ")} first`
            : "Create a server from this template";
        useButton.addEventListener("click", () => openCreateServer(template.id));
        card.querySelector("[data-details]").addEventListener("click", () => showDetails(template));
        grid.append(card);
      }
      section.append(grid);
      body.append(section);
    }

    if (warnings.length) {
      const warningCard = el("div", { class: "card", style: "margin-top:16px" });
      warningCard.innerHTML = `<div class="card-head"><div class="card-title">Template warnings</div><div class="card-actions"><span class="badge warn">${warnings.length}</span></div></div>`;
      for (const warning of warnings) warningCard.append(noteBox(warning, "warn"));
      body.append(warningCard);
    }

    const custom = el("div", { class: "card", style: "margin-top:16px" });
    custom.innerHTML = `<div class="card-head"><div class="card-title">Adding your own templates</div></div>
      <p class="card-sub" style="line-height:1.6">Drop a JSON template into the <span class="mono">templates</span> folder inside the WinPanel data directory, then reload. Templates are validated on load and can be overwritten by id. A template defines the install method, runtime requirements, startup command, default ports, configurable options and scaffolded files.</p>
      <div class="row" style="margin-top:12px"><span class="badge">methods: none, download, minecraft_vanilla, paper, bedrock, fivem</span><span class="badge">archives: none, zip</span></div>`;
    body.append(runtimeCard, custom);
  }

  function showDetails(template) {
    const body = el("div", { class: "col" });
    const rows = [
      ["Identifier", template.id],
      ["Category", template.category || "Applications"],
      ["Install method", installLabel(template.install?.method)],
      ["Startup", `${template.startupExe || "(none)"} ${template.startupArgs || ""}`],
      ["Default port", template.port ? `${template.port}/${template.protocol || "tcp"}` : "none"],
      ["Memory", template.memoryMb ? `${template.memoryMb} MB` : "unset"],
      ["Stop command", template.stopCommand || "(none)"],
    ];
    for (const [key, value] of rows) {
      const row = el("div", { class: "kv" }, [
        el("span", { class: "k", text: key }),
        el("span", { class: "v mono", text: String(value) }),
      ]);
      body.append(row);
    }
    if (template.options?.length) {
      body.append(el("div", { class: "field-label", text: "Options" }));
      for (const option of template.options) {
        body.append(
          el("div", {
            class: "card-sub",
            text: `${option.label} (${option.kind}) — default: ${option.default || "empty"}${option.help ? ` — ${option.help}` : ""}`,
          })
        );
      }
    }
    if (template.notes) body.append(noteBox(template.notes));
    if (template.install?.url) body.append(el("div", { class: "mono card-sub", text: template.install.url }));
    openModal({ title: template.name, subtitle: template.description, body });
  }

  view.querySelector("#tpl-reload")?.addEventListener("click", async (event) => {
    const button = event.currentTarget;
    button.disabled = true;
    try {
      await api.post("/api/templates/reload");
      toast("Templates reloaded", "success");
      await load();
    } catch (err) {
      toast(errorMessage(err), "error", 6000);
    } finally {
      button.disabled = false;
    }
  });

  await load();
  return null;
}

function installLabel(method) {
  return (
    {
      none: "No download",
      download: "Direct download",
      minecraft_vanilla: "Mojang",
      paper: "PaperMC",
      bedrock: "Microsoft",
      fivem: "Cfx.re",
    }[method] || method || "No download"
  );
}
