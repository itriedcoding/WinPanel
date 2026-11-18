// Application shell: authentication, routing, global live events.

import { api, setCsrf, errorMessage } from "./api.js";
import {
  el,
  icon,
  escapeHtml,
  toast,
  emptyState,
  skeletonStats,
  clear,
} from "./ui.js";
import { renderDashboard } from "./views/dashboard.js";
import { renderServers, openCreateServer } from "./views/servers.js";
import { renderServer } from "./views/server.js";
import { renderTemplates } from "./views/templates.js";
import { renderUsers } from "./views/users.js";
import { renderSettings } from "./views/settings.js";

export const state = {
  user: null,
  csrf: null,
  version: "",
  platform: "",
  servers: [],
  templates: [],
  warnings: [],
  socket: null,
  listeners: new Set(),
};

export function onServerEvent(handler) {
  state.listeners.add(handler);
  return () => state.listeners.delete(handler);
}

export function emitServerEvent(event) {
  for (const handler of state.listeners) {
    try {
      handler(event);
    } catch (err) {
      console.error("listener failed", err);
    }
  }
}

export function serverById(id) {
  return state.servers.find((server) => server.id === id) || null;
}

export function isAdmin() {
  return state.user?.role === "admin";
}

/* -------------------------------------------------------------- bootstrap */

async function boot() {
  try {
    const status = await api.get("/api/auth/status");
    state.version = status.version;
    state.platform = status.platform;

    if (status.setupRequired) {
      showAuth("setup");
      hideBoot();
      return;
    }
    if (status.authenticated) {
      state.user = status.user;
      setCsrf(status.csrf);
      await enterApp();
      hideBoot();
      return;
    }
    showAuth("login");
    hideBoot();
  } catch (err) {
    const bootNode = document.getElementById("boot");
    clear(bootNode).append(
      el("div", { class: "col", style: undefined }, [
        el("div", { class: "brand-name", text: "WinPanel could not start" }),
        el("div", { class: "empty-text", text: errorMessage(err) }),
        el("button", {
          class: "btn",
          type: "button",
          text: "Retry",
          onclick: () => location.reload(),
        }),
      ])
    );
  }
}

function hideBoot() {
  const bootNode = document.getElementById("boot");
  if (bootNode) bootNode.remove();
}

/* ------------------------------------------------------------------- auth */

function showAuth(mode) {
  const authRoot = document.getElementById("auth");
  const appRoot = document.getElementById("app");
  appRoot.hidden = true;
  authRoot.hidden = false;
  clear(authRoot);

  const isSetup = mode === "setup";
  const card = el("div", { class: "auth-card" });

  const head = el("div", { class: "auth-head" }, [
    el("span", { class: "brand-mark", text: "W" }),
    el("div", {}, [
      el("div", { class: "auth-title", text: isSetup ? "Set up WinPanel" : "Sign in" }),
    ]),
  ]);
  const sub = el("div", {
    class: "auth-sub",
    text: isSetup
      ? "Create the administrator account for this panel."
      : "Manage your Windows servers from one place.",
  });

  const form = el("form", { class: "col", style: undefined });
  const username = el("input", {
    class: "input",
    name: "username",
    autocomplete: "username",
    placeholder: "username",
    required: true,
    autofocus: true,
  });
  const password = el("input", {
    class: "input",
    type: "password",
    name: "password",
    autocomplete: isSetup ? "new-password" : "current-password",
    placeholder: isSetup ? "password (min. 8 characters)" : "password",
    required: true,
  });
  const confirm = isSetup
    ? el("input", {
        class: "input",
        type: "password",
        placeholder: "confirm password",
        autocomplete: "new-password",
        required: true,
      })
    : null;

  const errorBox = el("div", { class: "inline-error hidden" });
  const submit = el("button", {
    class: "btn btn-primary btn-lg btn-block",
    type: "submit",
    text: isSetup ? "Create administrator" : "Sign in",
  });

  form.append(
    el("div", { class: "field" }, [
      el("label", { class: "field-label", text: "Username" }),
      username,
    ]),
    el("div", { class: "field" }, [
      el("label", { class: "field-label", text: "Password" }),
      password,
    ])
  );
  if (confirm) {
    form.append(
      el("div", { class: "field" }, [
        el("label", { class: "field-label", text: "Confirm password" }),
        confirm,
      ])
    );
  }
  form.append(errorBox, submit);

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    errorBox.classList.add("hidden");
    if (isSetup && password.value !== confirm.value) {
      errorBox.textContent = "Passwords do not match";
      errorBox.classList.remove("hidden");
      return;
    }
    submit.disabled = true;
    submit.innerHTML = `<span class="spin"></span><span>${isSetup ? "Creating" : "Signing in"}</span>`;
    try {
      const response = await api.post(isSetup ? "/api/auth/setup" : "/api/auth/login", {
        username: username.value.trim(),
        password: password.value,
      });
      state.user = response.user;
      setCsrf(response.csrf);
      authRoot.hidden = true;
      await enterApp();
    } catch (err) {
      errorBox.textContent = errorMessage(err);
      errorBox.classList.remove("hidden");
      submit.disabled = false;
      submit.textContent = isSetup ? "Create administrator" : "Sign in";
    }
  });

  const foot = el("div", { class: "auth-foot" }, [
    el("span", { text: `WinPanel v${state.version || ""}` }),
    el("span", { text: state.platform || "" }),
  ]);

  card.append(head, sub, form, foot);
  authRoot.append(card);
  setTimeout(() => username.focus(), 60);
}

/* ------------------------------------------------------------------ shell */

async function enterApp() {
  document.getElementById("auth").hidden = true;
  document.getElementById("app").hidden = false;
  renderShell();
  await loadCoreData();
  connectEvents();

  const view = document.getElementById("view");
  view.innerHTML = `<h1 class="page-title">Overview</h1>${skeletonStats()}`;
  window.addEventListener("hashchange", route);
  await route();
}

function renderShell() {
  const sidebar = document.getElementById("sidebar");
  clear(sidebar);

  const brand = el("div", { class: "sidebar-brand" }, [
    el("span", { class: "brand-mark", text: "W" }),
    el("span", { class: "brand-name", text: "WinPanel" }),
    el("span", { class: "sidebar-version", text: `v${state.version}` }),
  ]);

  const nav = el("nav", { class: "nav" });
  const link = (href, iconName, label, extra = "") =>
    el("a", {
      class: "nav-item",
      href,
      "data-nav": href,
      html: `${icon(iconName, 16)}<span>${escapeHtml(label)}</span>${extra}`,
    });

  nav.append(el("div", { class: "nav-label", text: "Manage" }));
  nav.append(link("#/dashboard", "dashboard", "Overview"));
  nav.append(
    link(
      "#/servers",
      "server",
      "Servers",
      `<span class="nav-badge" id="nav-servers">${state.servers.length || ""}</span>`
    )
  );
  nav.append(link("#/templates", "templates", "Templates"));
  if (isAdmin()) nav.append(link("#/users", "users", "Users"));
  nav.append(el("div", { class: "nav-label", text: "System" }));
  nav.append(link("#/settings", "settings", "Settings"));

  const initial = (state.user?.username || "?").slice(0, 1);
  const logout = el("button", {
    class: "icon-btn",
    type: "button",
    title: "Sign out",
    html: icon("logout", 15),
    onclick: async () => {
      try {
        await api.post("/api/auth/logout");
      } catch {
        /* the session is cleared client-side regardless */
      }
      location.hash = "";
      location.reload();
    },
  });

  const foot = el("div", { class: "sidebar-foot" }, [
    el("div", { class: "user-chip" }, [
      el("div", { class: "user-avatar", text: initial }),
      el("div", { class: "user-meta" }, [
        el("div", { class: "user-name truncate", text: state.user?.username || "" }),
        el("div", { class: "user-role", text: state.user?.role || "" }),
      ]),
      el("div", { class: "spacer" }),
      logout,
    ]),
  ]);

  sidebar.append(brand, nav, foot);
}

function updateNavCount() {
  const badge = document.getElementById("nav-servers");
  if (badge) badge.textContent = state.servers.length ? String(state.servers.length) : "";
}

function markActiveNav() {
  const hash = location.hash || "#/dashboard";
  const base = `#/${hash.replace(/^#\//, "").split("/")[0].split("?")[0]}`;
  document.querySelectorAll(".nav-item").forEach((node) => {
    node.classList.toggle("active", node.dataset.nav === base || (base === "#/" && node.dataset.nav === "#/dashboard"));
  });
}

/* ------------------------------------------------------------------ data */

export async function loadCoreData() {
  const [servers, templates] = await Promise.all([
    api.get("/api/servers"),
    api.get("/api/templates"),
  ]);
  state.servers = servers;
  state.templates = templates.templates || [];
  state.warnings = templates.warnings || [];
  updateNavCount();
  return { servers, templates };
}

export async function refreshServers() {
  const servers = await api.get("/api/servers");
  state.servers = servers;
  updateNavCount();
  emitServerEvent({ type: "servers", data: { servers } });
  return servers;
}

/* ----------------------------------------------------------------- router */

let activeCleanup = null;
let routing = false;
let pendingRoute = false;

async function route() {
  if (routing) {
    pendingRoute = true;
    return;
  }
  routing = true;
  const view = document.getElementById("view");
  const raw = (location.hash || "#/dashboard").replace(/^#/, "");
  const [pathPart, queryPart] = raw.split("?");
  const parts = pathPart.split("/").filter(Boolean);
  const query = new URLSearchParams(queryPart || "");

  try {
    if (activeCleanup) {
      await activeCleanup();
      activeCleanup = null;
    }
    clear(view);
    markActiveNav();
    document.getElementById("sidebar").classList.remove("open");

    let cleanup = null;
    const section = parts[0] || "dashboard";
    if (section === "dashboard") {
      cleanup = await renderDashboard(view);
    } else if (section === "servers" && parts[1]) {
      cleanup = await renderServer(view, { id: parts[1], tab: parts[2] || query.get("tab") || "console" });
    } else if (section === "servers") {
      cleanup = await renderServers(view);
    } else if (section === "templates") {
      cleanup = await renderTemplates(view);
    } else if (section === "users") {
      cleanup = isAdmin() ? await renderUsers(view) : renderForbidden(view);
    } else if (section === "settings") {
      cleanup = await renderSettings(view);
    } else {
      cleanup = renderNotFound(view);
    }
    activeCleanup = typeof cleanup === "function" ? cleanup : null;
    window.scrollTo({ top: 0 });
  } catch (err) {
    view.replaceChildren(
      el("div", { class: "card" }, [
        el("div", { class: "card-head" }, [el("h2", { class: "card-title", text: "Something went wrong" })]),
        el("p", { class: "muted", text: errorMessage(err) }),
        el("div", { class: "row", style: undefined }, [
          el("button", {
            class: "btn",
            type: "button",
            text: "Retry",
            onclick: () => route(),
          }),
        ]),
      ])
    );
  } finally {
    routing = false;
    if (pendingRoute) {
      pendingRoute = false;
      route();
    }
  }
}

function renderForbidden(view) {
  view.innerHTML = emptyState({
    icon: "shield",
    title: "Administrators only",
    text: "Your account does not have permission to open this section.",
  });
  return null;
}

function renderNotFound(view) {
  view.innerHTML = emptyState({
    icon: "info",
    title: "Page not found",
    text: "That route does not exist in WinPanel.",
    actionHtml: `<a class="btn" href="#/dashboard">Back to overview</a>`,
  });
  return null;
}

/* ------------------------------------------------------------- live events */

function connectEvents() {
  if (state.socket) {
    state.socket.onclose = null;
    state.socket.close();
  }
  const protocol = location.protocol === "https:" ? "wss" : "ws";
  let attempt = 0;

  const open = () => {
    const socket = new WebSocket(`${protocol}://${location.host}/api/events`);
    state.socket = socket;

    socket.addEventListener("open", () => {
      attempt = 0;
    });

    socket.addEventListener("message", (message) => {
      let event;
      try {
        event = JSON.parse(message.data);
      } catch {
        return;
      }
      handleEvent(event);
    });

    socket.addEventListener("close", () => {
      attempt += 1;
      const delay = Math.min(800 * attempt, 8000);
      setTimeout(() => {
        if (!document.hidden) open();
      }, delay);
      // Try again even when hidden, but slower.
      setTimeout(() => {
        if (document.hidden) open();
      }, 20000);
    });
  };

  open();
}

function handleEvent(event) {
  const serverId = event.serverId;
  if (serverId) {
    const server = serverById(serverId);
    if (server) {
      if (event.type === "status") {
        server.runtime = { ...server.runtime, status: event.data.status, pid: undefined };
        if (event.data.status === "crashed") {
          server.runtime.lastExitCode = event.data.exitCode;
        }
      } else if (event.type === "install") {
        server.runtime.installProgress = event.data.progress;
        server.runtime.installStep = event.data.step;
        if (event.data.status === "complete") {
          server.installed = true;
          server.status = "stopped";
          server.runtime.status = "stopped";
          toast(`Installation finished for ${server.name}`, "success");
          refreshServers().catch(() => {});
        } else if (event.data.status === "failed") {
          server.runtime.status = "install_failed";
          toast(`Install failed for ${server.name}: ${event.data.error}`, "error", 9000);
          refreshServers().catch(() => {});
        }
      } else if (event.type === "backup") {
        if (event.data.status === "complete") {
          toast(`Backup "${event.data.name}" completed`, "success");
        } else if (event.data.status === "failed") {
          toast(`Backup failed: ${event.data.error}`, "error", 8000);
        }
      }
      if (event.type === "status" && event.data.status === "crashed") {
        toast(`${server.name} crashed (exit code ${event.data.exitCode ?? "unknown"})`, "error", 7000);
      }
    }
  }
  emitServerEvent(event);
}

document.addEventListener("click", (event) => {
  const anchor = event.target.closest("a[href^='#/']");
  if (anchor && anchor.dataset.nav) {
    event.preventDefault();
    if (location.hash === anchor.getAttribute("href")) route();
    else location.hash = anchor.getAttribute("href");
  }
});

const menuToggle = document.getElementById("menu-toggle");
if (menuToggle) {
  menuToggle.addEventListener("click", () => {
    document.getElementById("sidebar").classList.toggle("open");
  });
}

window.addEventListener("winpanel:unauthorized", () => {
  if (!state.user) return;
  state.user = null;
  if (state.socket) state.socket.close();
  toast("Your session expired. Please sign in again.", "error");
  showAuth("login");
});

document.addEventListener("visibilitychange", () => {
  if (!document.hidden && state.user && (!state.socket || state.socket.readyState > 1)) {
    connectEvents();
  }
});

// Exposed for the views (create modal is shared).
window.winpanel = { openCreateServer, state };

boot();
