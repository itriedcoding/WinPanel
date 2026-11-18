// Overview: live system metrics, server totals, recent activity.

import { api, errorMessage } from "../api.js";
import {
  escapeHtml,
  formatBytes,
  formatDuration,
  formatNumber,
  formatPercent,
  formatUptime,
  icon,
  lineChart,
  sparkline,
  timeAgo,
  toast,
  emptyState,
} from "../ui.js";
import { onServerEvent, state, isAdmin, serverById } from "../app.js";
import { openCreateServer } from "./servers.js";
import { activityTimeline, pageHeader, statusOf, templateName } from "./common.js";

export async function renderDashboard(view) {
  view.innerHTML =
    pageHeader({
      title: "Overview",
      subtitle: "Live system load and server status",
      actions: `
        <button class="btn" type="button" id="dash-refresh">${icon("refresh", 14)}<span>Refresh</span></button>
        ${isAdmin() ? `<button class="btn btn-primary" type="button" id="dash-new">${icon("plus", 14)}<span>New server</span></button>` : ""}
      `,
    }) +
    `<div id="dash-body">
      <div class="stat-grid">
        ${Array.from({ length: 6 }).map(() => `<div class="skeleton skeleton-stat"></div>`).join("")}
      </div>
      <div class="grid-2">
        <div class="skeleton" style="height:250px;border-radius:8px"></div>
        <div class="skeleton" style="height:250px;border-radius:8px"></div>
      </div>
    </div>`;

  const body = view.querySelector("#dash-body");
  view.querySelector("#dash-refresh")?.addEventListener("click", () => load(true));
  view.querySelector("#dash-new")?.addEventListener("click", () => openCreateServer());

  let data = null;
  let timer = null;
  let disposed = false;

  async function load(showSpinner = false) {
    if (showSpinner) {
      body.style.opacity = "0.6";
    }
    try {
      data = await api.get("/api/dashboard");
      if (disposed) return;
      render(data);
    } catch (err) {
      if (!disposed) {
        body.innerHTML = "";
        body.append(
          emptyState({
            icon: "alert",
            title: "Could not load the dashboard",
            text: errorMessage(err),
            actionHtml: `<button class="btn" id="dash-retry" type="button">Retry</button>`,
          })
        );
        body.querySelector("#dash-retry")?.addEventListener("click", () => load());
      }
    } finally {
      body.style.opacity = "";
    }
  }

  function render(payload) {
    const { system, history, totals, recentActivity, crashes, recentServers } = payload;
    const cpuSeries = history.map((point) => point.cpu);
    const memSeries = history.map((point) => (system.memoryTotal ? (point.memoryUsed / system.memoryTotal) * 100 : 0));
    const rxSeries = history.map((point) => point.netRx);
    const memPercent = system.memoryTotal ? (system.memoryUsed / system.memoryTotal) * 100 : 0;
    const panelPercent = system.memoryTotal ? (totals.memoryBytes / system.memoryTotal) * 100 : 0;

    body.innerHTML = "";

    /* --------------------------------------------------------------- stats */
    const stats = document.createElement("div");
    stats.className = "stat-grid";

    const statCard = ({ label, iconName, value, foot, spark, accent }) => {
      const card = document.createElement("div");
      card.className = "stat";
      card.innerHTML = `
        <div class="stat-label">${icon(iconName, 12)}<span>${escapeHtml(label)}</span></div>
        <div class="stat-value">${value}</div>
        ${spark ? `<div class="stat-spark">${spark}</div>` : `<div class="stat-foot">${foot || ""}</div>`}
        ${spark && foot ? `<div class="stat-foot">${foot}</div>` : ""}
      `;
      if (accent) card.dataset.accent = accent;
      return card;
    };

    stats.append(
      statCard({
        label: "Servers",
        iconName: "server",
        value: `${totals.servers}<small>total</small>`,
        foot: `${totals.running} online · ${totals.offline} offline`,
      }),
      statCard({
        label: "Running",
        iconName: "play",
        value: `${totals.running}<small>live</small>`,
        foot: `${totals.offline} stopped${totals.errors ? ` · ${totals.errors} with errors` : ""}`,
      }),
      statCard({
        label: "Issues",
        iconName: "alert",
        value: `${totals.errors}`,
        foot: totals.errors ? "Crashes or failed installs" : "No crashes detected",
      }),
      statCard({
        label: "CPU",
        iconName: "cpu",
        value: `${formatPercent(system.cpuPercent || 0)}`,
        foot: `${system.cpuCores || 0} cores · ${formatPercent(totals.serverCpuPercent || 0)} by servers`,
        spark: sparkline(cpuSeries.slice(-60), { width: 150, height: 22 }),
      }),
      statCard({
        label: "Memory",
        iconName: "memory",
        value: `${formatBytes(system.memoryUsed)}<small>/ ${formatBytes(system.memoryTotal)}</small>`,
        foot: `${formatPercent(memPercent, 0)} used · ${formatBytes(totals.memoryBytes)} by servers`,
        spark: sparkline(memSeries.slice(-60), { width: 150, height: 22 }),
      }),
      statCard({
        label: "Network",
        iconName: "network",
        value: `${formatBytes(system.network?.transmittedPerSec || 0)}<small>/s up</small>`,
        foot: `${formatBytes(system.network?.receivedPerSec || 0)}/s down`,
        spark: sparkline(rxSeries.slice(-60), { width: 150, height: 22 }),
      })
    );

    /* --------------------------------------------------------------- chart */
    const chartCard = document.createElement("div");
    chartCard.className = "card";
    chartCard.innerHTML = `
      <div class="card-head">
        <div>
          <div class="card-title">System load</div>
          <div class="card-sub">CPU and memory over the last ${Math.round(history.length * 3 / 60) || 0} minutes</div>
        </div>
        <div class="card-actions">
          <div class="chart-legend">
            <span><i class="legend-swatch"></i>CPU</span>
            <span><i class="legend-swatch dim"></i>Memory</span>
          </div>
        </div>
      </div>
      ${lineChart(
        [
          { values: cpuSeries, color: "#fafafa", fillColor: "rgba(255,255,255,0.06)" },
          { values: memSeries, color: "#5a5a5a", fill: false },
        ],
        { height: 170, max: 100 }
      )}
      <div class="row" style="margin-top:10px;justify-content:space-between">
        <span class="card-sub">${system.hostname || ""} · ${escapeHtml(system.cpuBrand || "")}</span>
        <span class="card-sub">${system.processCount || 0} processes · panel ${formatBytes(system.panelMemory || 0)}</span>
      </div>
    `;

    /* ---------------------------------------------------------------- host */
    const hostCard = document.createElement("div");
    hostCard.className = "card";
    const volumes = (system.disks || [])
      .filter((disk) => disk.total > 0 && !disk.mount?.startsWith("/sys"))
      .slice(0, 4)
      .map((disk) => {
        const used = disk.total - disk.available;
        const percent = disk.total ? Math.max(0, Math.min(100, (used / disk.total) * 100)) : 0;
        return `<div class="meter">
          <div class="meter-head"><b>${escapeHtml(disk.mount || disk.name || "disk")}</b><span>${formatBytes(used)} / ${formatBytes(disk.total)}</span></div>
          <div class="meter-bar ${percent > 90 ? "danger" : ""}"><div data-width="${percent.toFixed(1)}"></div></div>
        </div>`;
      })
      .join("");

    hostCard.innerHTML = `
      <div class="card-head">
        <div class="card-title">Host</div>
        <div class="card-actions"><span class="badge">${escapeHtml(system.os || "")}</span></div>
      </div>
      <div class="kv"><span class="k">Hostname</span><span class="v">${escapeHtml(system.hostname || "")}</span></div>
      <div class="kv"><span class="k">Uptime</span><span class="v">${formatUptime(system.uptimeSecs)}</span></div>
      <div class="kv"><span class="k">CPU</span><span class="v">${escapeHtml(system.cpuBrand || "")} (${system.cpuCores || 0})</span></div>
      <div class="kv"><span class="k">Panel uptime</span><span class="v">${formatDuration(payload.uptimeSecs * 1000)}</span></div>
      <div class="kv"><span class="k">Backups</span><span class="v">${payload.backupCount} · ${formatBytes(payload.backupBytes)}</span></div>
      <div class="form-section"><div class="meter-head"><b>Volumes</b><span>${formatBytes(totals.storageBytes)} used by servers</span></div>${volumes || '<div class="card-sub" style="margin-top:8px">No volumes reported</div>'}</div>
    `;
    hostCard.querySelectorAll(".meter-bar > div").forEach((bar) => {
      bar.style.width = `${bar.dataset.width}%`;
    });

    /* ------------------------------------------------------------ servers */
    const serversCard = document.createElement("div");
    serversCard.className = "card flush";
    const serverRows = (recentServers || [])
      .map((server) => {
        const status = statusOf(server);
        return `<a class="list-row link" href="#/servers/${server.id}">
          <div class="row-main">
            <div class="row-title"><span class="truncate">${escapeHtml(server.name)}</span></div>
            <div class="row-meta"><span>${escapeHtml(templateName(server.templateId))}</span><span class="divider-v"></span><span>${status === "running" ? "up " + formatUptime(server.runtime.uptimeMs) : "stopped"}</span></div>
          </div>
          <div class="row-actions">
            <span class="status" data-state="${status}"><span class="dot"></span>${status === "install_failed" ? "failed" : status}</span>
            <span class="dim">${icon("chevronRight", 14)}</span>
          </div>
        </a>`;
      })
      .join("");
    serversCard.innerHTML = `
      <div class="card-head" style="padding:16px 16px 12px;margin:0">
        <div class="card-title">Recent servers</div>
        <div class="card-actions"><a class="btn btn-sm btn-ghost" href="#/servers">View all ${icon("chevronRight", 12)}</a></div>
      </div>
      <div class="list">${serverRows || `<div class="empty" style="padding:30px 20px"><div class="empty-title">No servers yet</div><div class="empty-text">Create your first server from a template.</div></div>`}</div>
    `;

    /* ---------------------------------------------------------- activity */
    const activityCard = document.createElement("div");
    activityCard.className = "card";
    activityCard.innerHTML = `<div class="card-head"><div class="card-title">Recent activity</div><div class="card-actions"><a class="btn btn-sm btn-ghost" href="#/settings">Activity log ${icon("chevronRight", 12)}</a></div></div>`;
    activityCard.append(activityTimeline((recentActivity || []).slice(0, 9)));

    const grid = document.createElement("div");
    grid.className = "grid-2";
    grid.append(chartCard, hostCard);

    const lower = document.createElement("div");
    lower.className = "grid-2";
    lower.append(serversCard, activityCard);

    body.append(stats, grid);

    if (crashes && crashes.length) {
      const crashCard = document.createElement("div");
      crashCard.className = "card";
      crashCard.innerHTML = `<div class="card-head"><div class="card-title">Recent errors</div><div class="card-actions"><span class="badge danger">${crashes.length}</span></div></div>`;
      crashCard.append(activityTimeline(crashes));
      body.append(crashCard);
    }

    body.append(lower);

    if (totals.servers === 0) {
      const welcome = document.createElement("div");
      welcome.className = "card";
      welcome.style.marginTop = "14px";
      welcome.innerHTML = emptyState({
        icon: "server",
        title: "Create your first server",
        text: "Pick a template — Minecraft Java, Bedrock, FiveM, or a generic Java, Node.js, Python or custom executable.",
        actionHtml: isAdmin() ? `<button class="btn btn-primary" id="welcome-new" type="button">${icon("plus", 14)}<span>New server</span></button>` : "",
      });
      welcome.querySelector("#welcome-new")?.addEventListener("click", () => openCreateServer());
      body.append(welcome);
    }
  }

  await load();
  timer = setInterval(() => {
    if (!document.hidden) load();
  }, 5000);

  const unsubscribe = onServerEvent((event) => {
    if (["status", "install", "backup"].includes(event.type) && !document.hidden) load();
  });

  return () => {
    disposed = true;
    clearInterval(timer);
    unsubscribe();
  };
}
