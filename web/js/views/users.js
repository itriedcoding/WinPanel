// User accounts and per-server permissions (administrators only).

import { api, errorMessage } from "../api.js";
import {
  el,
  escapeHtml,
  icon,
  openModal,
  confirmDialog,
  setBusy,
  toast,
  emptyState,
  formatDateTime,
  timeAgo,
} from "../ui.js";
import { isAdmin, refreshServers, state } from "../app.js";
import { errorBox, field, noteBox, pageHeader, templateName } from "./common.js";

export async function renderUsers(view) {
  if (!isAdmin()) {
    view.innerHTML = emptyState({
      icon: "shield",
      title: "Administrators only",
      text: "User management requires an administrator account.",
    });
    return null;
  }

  view.innerHTML =
    pageHeader({
      title: "Users",
      subtitle: "Accounts and per-server access",
      actions: `<button class="btn btn-primary" type="button" id="user-new">${icon("plus", 14)}<span>New user</span></button>`,
    }) + `<div class="card flush" id="users-body"><div style="padding:16px"><div class="skeleton skeleton-line" style="width:40%"></div></div></div>`;

  const body = view.querySelector("#users-body");
  let users = [];
  let disposed = false;

  async function load() {
    try {
      users = await api.get("/api/users");
      if (!state.servers.length) await refreshServers().catch(() => {});
      if (disposed) return;
      render();
    } catch (err) {
      body.innerHTML = emptyState({
        icon: "alert",
        title: "Could not load users",
        text: errorMessage(err),
        actionHtml: `<button class="btn" type="button" id="users-retry">Retry</button>`,
      });
      body.querySelector("#users-retry")?.addEventListener("click", load);
    }
  }

  function render() {
    if (!users.length) {
      body.innerHTML = emptyState({ icon: "users", title: "No accounts", text: "Create an account to delegate a server." });
      return;
    }
    const table = el("table", { class: "table" });
    table.innerHTML = `<thead><tr>
      <th>User</th><th style="width:100px">Role</th><th style="width:110px">Servers</th>
      <th style="width:170px">Last sign-in</th><th style="width:60px">Status</th><th style="width:110px"></th>
    </tr></thead>`;
    const tbody = el("tbody");
    for (const user of users) {
      const isSelf = user.id === state.user?.id;
      const tr = el("tr");
      tr.append(
        el("td", {}, [
          el("div", { class: "row", style: "gap:9px" }, [
            el("div", { class: "user-avatar", text: (user.username || "?").slice(0, 1) }),
            el("div", {}, [
              el("div", { text: user.username }),
              el("div", { class: "row-meta", text: isSelf ? "you" : formatDateTime(user.createdAt) }),
            ]),
          ]),
        ]),
        el("td", {}, [el("span", { class: "badge", text: user.role })]),
        el("td", { class: "muted", text: String(user.access?.length || 0) }),
        el("td", { class: "muted", text: user.lastLogin ? timeAgo(user.lastLogin) : "never" }),
        el("td", {}, [
          el("span", { class: `badge ${user.disabled ? "danger" : "ok"}`, text: user.disabled ? "disabled" : "active" }),
        ])
      );
      const actions = el("td", {});
      const wrap = el("div", { class: "table-actions" });
      wrap.append(
        el("button", {
          class: "icon-btn",
          type: "button",
          title: "Server access",
          html: icon("server", 13),
          onclick: () => editAccess(user),
        }),
        el("button", {
          class: "icon-btn",
          type: "button",
          title: "Edit account",
          html: icon("edit", 13),
          onclick: () => editUser(user),
        }),
        el("button", {
          class: "icon-btn danger",
          type: "button",
          title: "Delete account",
          html: icon("trash", 13),
          disabled: isSelf,
          onclick: () => deleteUser(user),
        })
      );
      actions.append(wrap);
      tr.append(actions);
      tbody.append(tr);
    }
    table.append(tbody);
    body.replaceChildren(table);
  }

  async function editUser(user) {
    const role = el("select", { class: "select" }, [
      el("option", { value: "user", text: "User" }),
      el("option", { value: "admin", text: "Administrator" }),
    ]);
    role.value = user.role;
    const disabled = el("input", { type: "checkbox" });
    disabled.checked = !!user.disabled;
    const password = el("input", { class: "input", type: "password", placeholder: "Leave blank to keep current" });

    const body = el("div", { class: "col" });
    const err = el("div", { class: "hidden" });
    body.append(
      err,
      field("Username", el("input", { class: "input", value: user.username, disabled: true })),
      field("Role", role),
      field("Reset password", password, "At least 8 characters."),
      el("label", { class: "switch" }, [disabled, el("span", { text: "Account disabled" })])
    );

    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const save = el("button", { class: "btn btn-primary", type: "button", text: "Save" });
    const modal = openModal({ title: `Edit ${user.username}`, body, footer: [cancel, save], narrow: true });
    cancel.addEventListener("click", () => modal.close());

    save.addEventListener("click", async () => {
      err.className = "hidden";
      const payload = { role: role.value, disabled: disabled.checked };
      if (password.value) payload.password = password.value;
      setBusy(save, true, "Saving");
      try {
        await api.patch(`/api/users/${user.id}`, payload);
        modal.close();
        toast("Account updated", "success");
        load();
      } catch (error) {
        err.className = "inline-error";
        err.replaceChildren(errorBox(errorMessage(error)));
        setBusy(save, false);
      }
    });
  }

  async function editAccess(user) {
    const servers = await refreshServers().catch(() => state.servers);
    const current = new Map((user.access || []).map((entry) => [entry.serverId, entry.level]));

    const body = el("div", { class: "col" });
    body.append(
      noteBox(`Grant ${user.username} access to individual servers. Levels are cumulative: view → console → files → manage.`),
      el("div", {
        class: "card-sub",
        text: `${servers.length} server${servers.length === 1 ? "" : "s"} in this panel${user.role === "admin" ? " — administrators already have full access" : ""}`,
      })
    );
    const selects = new Map();
    for (const server of servers) {
      const select = el("select", { class: "select" }, [
        el("option", { value: "", text: "No access" }),
        el("option", { value: "view", text: "View" }),
        el("option", { value: "console", text: "Console" }),
        el("option", { value: "files", text: "Files" }),
        el("option", { value: "manage", text: "Manage" }),
      ]);
      select.value = current.get(server.id) || "";
      selects.set(server.id, select);
      const row = el("div", { class: "row", style: "justify-content:space-between;gap:12px" });
      row.append(
        el("div", { style: "min-width:0" }, [
          el("div", { class: "truncate", text: server.name }),
          el("div", { class: "card-sub", text: templateName(server.templateId) }),
        ]),
        select
      );
      body.append(row);
    }
    if (!servers.length) body.append(el("div", { class: "card-sub", text: "No servers exist yet." }));

    const cancel = el("button", { class: "btn", type: "button", text: "Close" });
    const save = el("button", { class: "btn btn-primary", type: "button", text: "Save access" });
    const modal = openModal({ title: "Server access", subtitle: user.username, body, footer: [cancel, save], wide: true });
    cancel.addEventListener("click", () => modal.close());

    save.addEventListener("click", async () => {
      setBusy(save, true, "Saving");
      try {
        for (const [serverId, select] of selects) {
          const level = select.value || null;
          const previous = current.get(serverId) || null;
          if (level === previous) continue;
          await api.post(`/api/users/${user.id}/access`, { serverId, level });
        }
        modal.close();
        toast("Access updated", "success");
        load();
      } catch (err) {
        toast(errorMessage(err), "error", 6000);
        setBusy(save, false);
      }
    });
  }

  async function deleteUser(user) {
    const ok = await confirmDialog({
      title: "Delete account",
      message: `Delete ${user.username}? Their sessions and access grants are removed. Servers they created stay online.`,
      confirmLabel: "Delete account",
      danger: true,
      requireText: user.username,
    });
    if (!ok) return;
    try {
      await api.del(`/api/users/${user.id}`);
      toast("Account deleted", "success");
      load();
    } catch (err) {
      toast(errorMessage(err), "error", 6000);
    }
  }

  view.querySelector("#user-new").addEventListener("click", () => {
    const username = el("input", { class: "input", placeholder: "username", autocomplete: "off" });
    const password = el("input", { class: "input", type: "password", placeholder: "at least 8 characters" });
    const role = el("select", { class: "select" }, [
      el("option", { value: "user", text: "User" }),
      el("option", { value: "admin", text: "Administrator" }),
    ]);
    const body = el("div", { class: "col" });
    const err = el("div", { class: "hidden" });
    body.append(
      err,
      field("Username", username, "3-32 characters: letters, digits, dot, dash or underscore."),
      field("Password", password),
      field("Role", role)
    );
    const cancel = el("button", { class: "btn", type: "button", text: "Cancel" });
    const create = el("button", { class: "btn btn-primary", type: "button", text: "Create user" });
    const modal = openModal({ title: "New user", body, footer: [cancel, create], narrow: true });
    cancel.addEventListener("click", () => modal.close());
    create.addEventListener("click", async () => {
      err.className = "hidden";
      setBusy(create, true, "Creating");
      try {
        await api.post("/api/users", {
          username: username.value.trim(),
          password: password.value,
          role: role.value,
        });
        modal.close();
        toast("User created", "success");
        load();
      } catch (error) {
        err.className = "inline-error";
        err.replaceChildren(errorBox(errorMessage(error)));
        setBusy(create, false);
      }
    });
  });

  await load();

  return () => {
    disposed = true;
  };
}
