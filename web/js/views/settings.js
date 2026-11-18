// Application settings, account security, updates and activity log.

import { api, errorMessage } from "../api.js";
import {
  el,
  escapeHtml,
  icon,
  openModal,
  confirmDialog,
  formatDateTime,
  setBusy,
  toast,
  emptyState,
  formatBytes,
} from "../ui.js";
import { isAdmin, state, loadCoreData } from "../app.js";
import { activityTimeline, errorBox, field, noteBox, pageHeader } from "./common.js";

export async function renderSettings(view) {
  view.innerHTML =
    pageHeader({
      title: "Settings",
      subtitle: "Panel preferences, security and updates",
      actions: isAdmin()
        ? `<button class="btn btn-primary" type="button" id="settings-save">${icon("save", 14)}<span>Save settings</span></button>`
        : "",
    }) + `<div id="settings-body"><div class="skeleton" style="height:220px;border-radius:8px"></div></div>`;

  const body = view.querySelector("#settings-body");
  let current = null;
  let inputs = null;

  function numberInput(value, { min = 0, max = null, step = 1 } = {}) {
    const input = el("input", {
      class: "input",
      type: "number",
      value,
      min,
      max,
      step,
    });
    return input;
  }

  async function load() {
    try {
      const settingsResponse = isAdmin() ? await api.get("/api/settings") : null;
      const about = await api.get("/api/system/info");
      if (settingsResponse) current = settingsResponse.settings;
      render(settingsResponse, about);
    } catch (err) {
      body.innerHTML = emptyState({
        icon: "alert",
        title: "Could not load settings",
        text: errorMessage(err),
        actionHtml: `<button class="btn" type="button" id="settings-retry">Retry</button>`,
      });
      body.querySelector("#settings-retry")?.addEventListener("click", load);
    }
  }

  function render(settingsResponse, about) {
    body.innerHTML = "";
    const settings = settingsResponse?.settings;

    if (isAdmin() && settings) {
      const installDir = el("input", { class: "input", value: settings.defaultInstallDir });
      const defaultMemory = numberInput(settings.defaultMemoryMb, { min: 0, step: 128 });
      const defaultStorage = numberInput(settings.defaultStorageMb, { min: 0, step: 1024 });
      const defaultPort = numberInput(settings.defaultPort, { min: 1, max: 65535 });
      const maxUpload = numberInput(settings.maxUploadMb, { min: 1, max: 8192 });
      const diskAlert = numberInput(settings.diskAlertPercent, { min: 50, max: 100 });
      const autoRestart = el("input", { type: "checkbox" });
      autoRestart.checked = !!settings.autoRestartDefault;
      const stopTimeout = numberInput(Math.round(settings.stopTimeoutMs / 1000), { min: 2, max: 300 });

      const backupDir = el("input", { class: "input", value: settings.backupDir });
      const backupsEnabled = el("input", { type: "checkbox" });
      backupsEnabled.checked = !!settings.backupsEnabled;
      const backupInterval = numberInput(settings.backupIntervalHours, { min: 1, max: 720 });
      const backupKeep = numberInput(settings.backupKeep, { min: 1, max: 100 });

      const metricsInterval = numberInput(Math.round(settings.metricsIntervalMs / 1000), { min: 1, max: 60 });
      const consoleLines = numberInput(settings.consoleBufferLines, { min: 200, max: 20000, step: 100 });
      const logLevel = el("select", { class: "select" }, ["error", "warn", "info", "debug", "trace"].map((level) => el("option", { value: level, text: level })));
      logLevel.value = settings.logLevel;
      const logRetention = numberInput(settings.logRetentionDays, { min: 1, max: 365 });

      const sessionHours = numberInput(settings.sessionHours, { min: 1, max: 8760 });
      const allowRegistration = el("input", { type: "checkbox" });
      allowRegistration.checked = !!settings.allowRegistration;
      const updateChecks = el("input", { type: "checkbox" });
      updateChecks.checked = !!settings.updateCheckEnabled;

      inputs = { installDir, defaultMemory, defaultStorage, defaultPort, maxUpload, diskAlert, autoRestart, stopTimeout, backupDir, backupsEnabled, backupInterval, backupKeep, metricsInterval, consoleLines, logLevel, logRetention, sessionHours, allowRegistration, updateChecks };

      const general = el("div", { class: "card" });
      general.append(
        el("div", { class: "form-section-head" }, [
          el("h3", { text: "General" }),
          el("span", { text: "Defaults for new servers" }),
        ]),
        el("div", { class: "form-grid" }, [
          field("Default install directory", installDir, "Absolute path, or relative to the WinPanel data directory."),
          field("Default port", defaultPort),
          field("Default memory limit (MB)", defaultMemory),
          field("Default storage limit (MB)", defaultStorage),
          field("Maximum upload size (MB)", maxUpload),
          field("Disk alert threshold (%)", diskAlert),
          field("Default stop timeout (seconds)", stopTimeout),
        ]),
        el("label", { class: "switch", style: "margin-top:14px" }, [autoRestart, el("span", { text: "Restart crashed servers by default" })]),
        el("div", { class: "form-section" }, [
          el("div", { class: "form-section-head" }, [el("h3", { text: "Backups" }), el("span", { text: "Applies to scheduled backups" })]),
          el("div", { class: "form-grid" }, [
            field("Backup directory", backupDir, "Changing this requires a restart."),
            field("Interval (hours)", backupInterval),
            field("Keep last", backupKeep),
          ]),
          el("label", { class: "switch", style: "margin-top:14px" }, [backupsEnabled, el("span", { text: "Enable scheduled backups by default" })]),
        ])
      );

      const monitoring = el("div", { class: "card", style: "margin-top:14px" });
      monitoring.append(
        el("div", { class: "form-section-head" }, [el("h3", { text: "Monitoring and logging" })]),
        el("div", { class: "form-grid" }, [
          field("Metrics interval (seconds)", metricsInterval, "Lower values cost more CPU."),
          field("Console buffer (lines)", consoleLines),
          field("Log level", logLevel),
          field("Log retention (days)", logRetention),
        ]),
        noteBox("A restart is required for log level, metrics interval and backup directory changes.")
      );

      const security = el("div", { class: "card", style: "margin-top:14px" });
      security.append(
        el("div", { class: "form-section-head" }, [el("h3", { text: "Security and updates" })]),
        el("div", { class: "form-grid" }, [field("Session lifetime (hours)", sessionHours)]),
        el("label", { class: "switch", style: "margin-top:14px" }, [allowRegistration, el("span", { text: "Allow self-registration (not recommended)" })]),
        el("label", { class: "switch", style: "margin-top:10px" }, [updateChecks, el("span", { text: "Check GitHub for updates automatically" })]),
        noteBox(
          `The management interface is bound to ${settingsResponse.listen}${settingsResponse.remoteAccess ? " (remote access enabled)" : " (local only)"}. Binding and remote access are controlled by config.json and require a restart.`
        )
      );

      body.append(general, monitoring, security);
    } else if (!isAdmin()) {
      body.append(noteBox("Application settings can only be changed by an administrator."));
    }

    /* ------------------------------------------------------------ updates */
    if (isAdmin()) {
      const updatesCard = el("div", { class: "card", style: "margin-top:14px" });
      updatesCard.innerHTML = `
        <div class="card-head">
          <div>
            <div class="card-title">Updates</div>
            <div class="card-sub">Current version ${escapeHtml(about.version)} · GitHub Releases</div>
          </div>
          <div class="card-actions">
            <button class="btn btn-sm" type="button" id="update-check">${icon("refresh", 12)}<span>Check now</span></button>
          </div>
        </div>
        <div id="update-panel"><div class="card-sub">No update check performed yet.</div></div>`;
      body.append(updatesCard);

      const panel = updatesCard.querySelector("#update-panel");
      const checkButton = updatesCard.querySelector("#update-check");
      let info = null;
      let downloaded = false;

      const renderPanel = () => {
        panel.innerHTML = "";
        if (!info) {
          panel.append(el("div", { class: "card-sub", text: "No update check performed yet." }));
          return;
        }
        if (info.error) {
          panel.append(errorBox(`Update check failed: ${info.error}`));
          return;
        }
        const head = el("div", { class: "row wrap" });
        head.append(
          el("span", { class: `badge ${info.available ? "warn" : "ok"}`, text: info.available ? `Update ${info.latestVersion} available` : "Up to date" }),
          el("span", { class: "card-sub", text: `latest ${info.latestVersion} · checked ${formatDateTime(info.checkedAt)}` })
        );
        panel.append(head);
        if (info.notes) {
          const notes = el("pre", {
            class: "mono",
            style: "white-space:pre-wrap;max-height:180px;overflow:auto;margin:10px 0 0;color:#a3a3a3;font-size:11.5px",
            text: info.notes.slice(0, 4000),
          });
          panel.append(notes);
        }
        if (info.available) {
          const actions = el("div", { class: "row", style: "margin-top:12px" });
          const download = el("button", {
            class: "btn btn-sm",
            type: "button",
            html: `${icon("download", 12)}<span>Download update</span>`,
          });
          const install = el("button", {
            class: "btn btn-sm btn-primary",
            type: "button",
            html: `${icon("refresh", 12)}<span>Install and restart</span>`,
            disabled: !downloaded,
          });
          actions.append(download, install);
          panel.append(actions);

          download.addEventListener("click", async () => {
            setBusy(download, true, "Downloading");
            try {
              const result = await api.post("/api/updates/download");
              downloaded = true;
              install.disabled = false;
              toast(`Update ${result.version} downloaded and verified (${formatBytes(result.size)})`, "success");
            } catch (err) {
              toast(errorMessage(err), "error", 8000);
            } finally {
              setBusy(download, false);
            }
          });

          install.addEventListener("click", async () => {
            const ok = await confirmDialog({
              title: "Install update",
              message:
                "WinPanel will close, replace its executable and restart. Running servers are stopped safely first. Server data and settings are preserved.",
              confirmLabel: "Install and restart",
              danger: true,
            });
            if (!ok) return;
            setBusy(install, true, "Installing");
            try {
              await api.post("/api/updates/install");
              panel.replaceChildren(
                noteBox("WinPanel is restarting with the new version. This page will reload automatically.")
              );
              pollRestart(about.version);
            } catch (err) {
              toast(errorMessage(err), "error", 8000);
              setBusy(install, false);
            }
          });
        }
      };

      checkButton.addEventListener("click", async () => {
        setBusy(checkButton, true, "Checking");
        try {
          info = await api.get("/api/updates/check?force=true");
          downloaded = false;
          renderPanel();
        } catch (err) {
          toast(errorMessage(err), "error", 7000);
        } finally {
          setBusy(checkButton, false);
        }
      });

      api
        .get("/api/updates/check")
        .then((result) => {
          info = result;
          renderPanel();
        })
        .catch(() => {
          /* an update check failure is not fatal for the settings page */
        });
    }

    /* -------------------------------------------------------------- about */
    const aboutCard = el("div", { class: "card", style: "margin-top:14px" });
    aboutCard.innerHTML = `
      <div class="card-head"><div class="card-title">About WinPanel</div><div class="card-actions"><span class="badge">${escapeHtml(about.buildProfile)} build</span></div></div>
      <div class="kv"><span class="k">Version</span><span class="v mono">${escapeHtml(about.version)}</span></div>
      <div class="kv"><span class="k">Platform</span><span class="v">${escapeHtml(about.platform)} ${escapeHtml(about.arch)}</span></div>
      <div class="kv"><span class="k">Listening on</span><span class="v mono">${escapeHtml(about.listen)}</span></div>
      <div class="kv"><span class="k">Remote access</span><span class="v">${about.remoteAccess ? "Enabled" : "Disabled (local only)"}</span></div>
      <div class="kv"><span class="k">Data directory</span><span class="v mono" style="font-size:11px">${escapeHtml(about.dataDir)}</span></div>
      <div class="kv"><span class="k">Database</span><span class="v mono" style="font-size:11px">${escapeHtml(about.databaseFile)}</span></div>
      <div class="kv"><span class="k">Servers</span><span class="v">${about.serverCount} (${about.runningCount} running)</span></div>
      <div class="kv"><span class="k">Uptime</span><span class="v">${Math.floor(about.uptimeSecs / 60)} minutes</span></div>
      <div class="row" style="margin-top:12px"><a class="btn btn-sm" href="https://github.com/${escapeHtml(about.repository)}" target="_blank" rel="noreferrer noopener">${icon("globe", 12)}<span>GitHub repository</span></a></div>`;
    if (about.templateWarnings?.length) {
      for (const warning of about.templateWarnings) aboutCard.append(noteBox(warning, "warn"));
    }
    body.append(aboutCard);

    /* ------------------------------------------------------------ account */
    const account = el("div", { class: "card", style: "margin-top:14px" });
    const currentPassword = el("input", { class: "input", type: "password", autocomplete: "current-password" });
    const newPassword = el("input", { class: "input", type: "password", autocomplete: "new-password" });
    const confirmPassword = el("input", { class: "input", type: "password", autocomplete: "new-password" });
    const accountError = el("div", { class: "hidden" });
    const changeButton = el("button", { class: "btn btn-sm", type: "button", text: "Change password" });
    account.append(
      el("div", { class: "form-section-head" }, [
        el("h3", { text: "Your account" }),
        el("span", { text: `${state.user?.username} · ${state.user?.role}` }),
      ]),
      accountError,
      el("div", { class: "form-grid" }, [
        field("Current password", currentPassword),
        field("New password", newPassword, "At least 8 characters."),
        field("Confirm new password", confirmPassword),
      ]),
      el("div", { class: "row", style: "margin-top:12px" }, [changeButton]),
      noteBox("Changing your password signs out every other session.")
    );
    changeButton.addEventListener("click", async () => {
      accountError.className = "hidden";
      if (newPassword.value !== confirmPassword.value) {
        accountError.className = "inline-error";
        accountError.replaceChildren(errorBox("The new passwords do not match."));
        return;
      }
      setBusy(changeButton, true, "Saving");
      try {
        await api.post("/api/auth/password", {
          currentPassword: currentPassword.value,
          newPassword: newPassword.value,
        });
        currentPassword.value = "";
        newPassword.value = "";
        confirmPassword.value = "";
        toast("Password changed", "success");
      } catch (err) {
        accountError.className = "inline-error";
        accountError.replaceChildren(errorBox(errorMessage(err)));
      } finally {
        setBusy(changeButton, false);
      }
    });
    body.append(account);

    /* ----------------------------------------------------------- activity */
    if (isAdmin()) {
      const logCard = el("div", { class: "card", style: "margin-top:14px" });
      logCard.innerHTML = `<div class="card-head"><div class="card-title">Activity log</div><div class="card-actions"><span class="card-sub">Last 100 events</span></div></div><div id="activity-log"><div class="skeleton skeleton-line" style="width:50%"></div></div>`;
      body.append(logCard);
      const logBody = logCard.querySelector("#activity-log");
      api
        .get("/api/activity?limit=100")
        .then((entries) => {
          logBody.replaceChildren(activityTimeline(entries, { empty: "No events recorded yet." }));
        })
        .catch((err) => {
          logBody.replaceChildren(errorBox(errorMessage(err)));
        });
    }
  }

  function collect() {
    const toSeconds = (value) => (Number(value) || 0) * 1000;
    return {
      defaultInstallDir: inputs.installDir.value.trim(),
      defaultMemoryMb: Number(inputs.defaultMemory.value) || 0,
      defaultStorageMb: Number(inputs.defaultStorage.value) || 0,
      defaultPort: Number(inputs.defaultPort.value) || 25565,
      autoRestartDefault: inputs.autoRestart.checked,
      stopTimeoutMs: toSeconds(inputs.stopTimeout.value || 30),
      backupsEnabled: inputs.backupsEnabled.checked,
      backupIntervalHours: Number(inputs.backupInterval.value) || 24,
      backupKeep: Number(inputs.backupKeep.value) || 5,
      backupDir: inputs.backupDir.value.trim(),
      logLevel: inputs.logLevel.value,
      logRetentionDays: Number(inputs.logRetention.value) || 14,
      consoleBufferLines: Number(inputs.consoleLines.value) || 2000,
      metricsIntervalMs: toSeconds(inputs.metricsInterval.value || 3),
      updateCheckEnabled: inputs.updateChecks.checked,
      sessionHours: Number(inputs.sessionHours.value) || 168,
      allowRegistration: inputs.allowRegistration.checked,
      maxUploadMb: Number(inputs.maxUpload.value) || 2048,
      diskAlertPercent: Number(inputs.diskAlert.value) || 90,
    };
  }

  const saveButton = view.querySelector("#settings-save");
  if (saveButton) {
    saveButton.addEventListener("click", async () => {
      if (!inputs) return;
      setBusy(saveButton, true, "Saving");
      try {
        const response = await api.put("/api/settings", collect());
        toast("Settings saved", "success");
        if (response.restartRequired) {
          toast("Restart WinPanel for log, metrics and backup directory changes", "info", 7000);
        }
        await loadCoreData().catch(() => {});
      } catch (err) {
        toast(errorMessage(err), "error", 7000);
      } finally {
        setBusy(saveButton, false);
      }
    });
  }

  await load();
  return null;
}

/** Wait for the panel to come back on the new version, then reload. */
async function pollRestart(previousVersion) {
  let attempts = 0;
  const timer = setInterval(async () => {
    attempts += 1;
    try {
      const status = await api.get("/api/auth/status");
      if (status.version && status.version !== previousVersion) {
        clearInterval(timer);
        location.reload();
      }
    } catch {
      /* the panel is restarting */
    }
    if (attempts > 90) {
      clearInterval(timer);
      toast("The panel did not come back automatically. Start WinPanel again.", "error", 12000);
    }
  }, 2000);
}
