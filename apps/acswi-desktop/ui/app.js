"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

if (!navigator.userAgent.includes("Windows")) document.documentElement.classList.add("opaque");

const $ = (id) => document.getElementById(id);
const appsEl = $("apps");

let apps = [];
let showAll = readPref("showAll", false);
// Per-app note shown in the card while a sign-in runs elsewhere: { [appId]: text }
const waiting = {};

function readPref(key, fallback) {
  try {
    const v = localStorage.getItem(key);
    return v === null ? fallback : JSON.parse(v);
  } catch {
    return fallback;
  }
}

function writePref(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {}
}

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") node.className = v;
    else if (k.startsWith("on")) node.addEventListener(k.slice(2), v);
    else if (v !== false && v != null) node.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) {
    if (c != null && c !== false) node.append(c.nodeType ? c : document.createTextNode(String(c)));
  }
  return node;
}

// ------------------------------------------------------------------ data

async function load({ quiet = false } = {}) {
  const btn = $("refresh");
  btn.classList.add("spin");
  try {
    apps = await invoke("overview", { all: showAll });
    render();
  } catch (e) {
    if (!quiet) toast(message(e), true);
  } finally {
    btn.classList.remove("spin");
  }
}

function message(e) {
  return (e && e.message) || String(e);
}

// ------------------------------------------------------------------ render

function render() {
  appsEl.replaceChildren();
  if (!apps.length) {
    appsEl.append(el("p", { class: "empty" }, "No supported apps found. Use the … menu to show all apps."));
    return;
  }
  // Apps with accounts first, then the rest, each alphabetically.
  const sorted = [...apps].sort((a, b) => (b.profiles.length > 0) - (a.profiles.length > 0) || a.name.localeCompare(b.name));
  for (const app of sorted) appsEl.append(renderApp(app));
}

function renderApp(app) {
  const meta = [];
  if (!app.installed) meta.push("not installed");
  if (app.running) meta.push("open");
  const card = el("div", { class: "card" });

  if (waiting[app.id]) {
    card.append(el("div", { class: "banner" }, el("p", {}, waiting[app.id])));
  } else if (app.pending_add) {
    card.append(
      el("div", { class: "banner" },
        el("p", {}, `Sign in to ${app.name}, then choose Done to save it as “${app.pending_add}”.`),
        el("button", { onclick: () => cancelAdd(app) }, "Cancel"),
        el("button", { class: "primary", onclick: () => finishAdd(app) }, "Done"),
      ),
    );
  } else if (app.unsaved) {
    card.append(
      el("div", { class: "banner warn" },
        el("p", {}, "Signed in to an account that isn’t saved yet."),
        el("button", { class: "primary", onclick: () => saveCurrent(app) }, "Save as…"),
      ),
    );
  }
  if (app.warning) card.append(el("div", { class: "banner warn" }, el("p", {}, app.warning)));

  for (const p of app.profiles) card.append(renderProfile(app, p));
  if (app.installed && !app.pending_add) {
    card.append(
      el("div", { class: "row add", role: "button", tabindex: 0, onclick: () => addAccount(app), onkeydown: activateOnKey },
        el("span", { class: "plus" }, "+"),
        el("span", {}, "Add account"),
      ),
    );
  }

  return el("section", { class: "app", "data-app": app.id },
    el("div", { class: "app-head" },
      el("h2", {}, app.name),
      meta.length ? el("span", { class: "meta" }, meta.join(" · ")) : null,
    ),
    card,
  );
}

const dateFmt = new Intl.DateTimeFormat(undefined, { day: "numeric", month: "short", year: "numeric" });

// "until 23 Nov 2026", "expires in 3 days", "expired, sign in again". Estimates come from how
// long the provider keeps unused accounts signed in, so they are marked with ~.
function expiryText(e) {
  if (!e) return null;
  const at = new Date(e.at);
  const days = Math.floor((at - Date.now()) / 86400000);
  const approx = e.estimated ? "~" : "";
  if (e.level === "expired") {
    return e.estimated ? "Probably signed out after going unused · sign in again" : `Expired ${dateFmt.format(at)} · sign in again`;
  }
  if (e.level === "soon") return `Expires ${days <= 0 ? "today" : days === 1 ? "tomorrow" : `in ${approx}${days} days`}`;
  return `Until ${approx}${dateFmt.format(at)}`;
}

function expiryTitle(app, e) {
  if (!e) return null;
  const when = new Date(e.at).toLocaleString();
  return e.estimated
    ? `Estimate: ${app.name} signs out accounts that go unused for a while. If this one is still in use, it stays signed in. (${when})`
    : `${app.name} says this sign-in expires ${when}. Signing in again renews it.`;
}

function renderProfile(app, p) {
  const sub = p.identity && p.identity !== p.name ? p.identity : null;
  const exp = expiryText(p.expiry);
  const row = el("div", {
    class: "row" + (p.active ? " active" : ""),
    role: "button",
    tabindex: 0,
    "aria-current": p.active ? "true" : false,
    title: p.active ? `${app.name} is using ${p.name}` : `Switch ${app.name} to ${p.name}`,
    onclick: (ev) => {
      if (ev.target.closest("button.icon")) return;
      if (!p.active) switchTo(app, p.name, row);
    },
    onkeydown: activateOnKey,
  },
    el("span", { class: "dot", "aria-hidden": "true" }),
    el("div", { class: "who" },
      el("div", { class: "name" }, p.name),
      sub ? el("div", { class: "id" }, sub) : null,
      exp ? el("div", { class: "exp " + p.expiry.level, title: expiryTitle(app, p.expiry) }, exp) : null,
    ),
    p.active ? el("span", { class: "state" }, "Active") : el("button", { class: "use", tabindex: -1 }, "Use"),
  );
  const more = el("button", {
    class: "icon",
    title: "More",
    "aria-label": `More for ${p.name}`,
    "aria-haspopup": "menu",
    onclick: (ev) => {
      ev.stopPropagation();
      profileMenu(app, p, more);
    },
  });
  more.innerHTML = '<svg viewBox="0 0 16 16" aria-hidden="true"><circle cx="3.5" cy="8" r="1.1" fill="currentColor"/><circle cx="8" cy="8" r="1.1" fill="currentColor"/><circle cx="12.5" cy="8" r="1.1" fill="currentColor"/></svg>';
  row.append(more);
  return row;
}

function activateOnKey(ev) {
  if (ev.key === "Enter" || ev.key === " ") {
    ev.preventDefault();
    ev.currentTarget.click();
  }
}

// ------------------------------------------------------------------ actions

async function switchTo(app, profile, row, close = false) {
  row?.classList.add("busy");
  try {
    const r = await invoke("switch_profile", { app: app.id, profile, close });
    toast(r.changed ? `${app.name} now uses ${r.to}` + (r.relaunched ? " · reopened" : "") : `${app.name} already uses ${r.to}`);
  } catch (e) {
    if (e.code === "app_running" && !close) {
      const ok = await ask({
        title: `Close ${app.name}?`,
        text: `${app.name} is open. acswi will close it, switch to ${profile} and open it again.`,
        ok: "Close and switch",
      });
      if (ok) return switchTo(app, profile, row, true);
    } else if (e.code === "unsaved_session") {
      if (await saveCurrent(app)) return switchTo(app, profile, row, close);
    } else {
      toast(message(e), true);
    }
  } finally {
    await load({ quiet: true });
  }
}

async function saveCurrent(app) {
  const name = await ask({
    title: `Save the current ${app.name} account`,
    text: "Give this account a short name so you can switch back to it later.",
    label: "Profile name",
    placeholder: "personal",
    hint: "Letters, digits, - _ . @",
    ok: "Save",
  });
  if (!name) return false;
  try {
    await invoke("save_current", { app: app.id, profile: name });
    toast(`Saved as ${name}`);
    await load({ quiet: true });
    return true;
  } catch (e) {
    toast(message(e), true);
    return false;
  }
}

async function addAccount(app, preset, close = false) {
  const how = app.strategy === "swap"
    ? `acswi signs ${app.name} out (your current account stays saved) so you can sign in to another one.`
    : `${app.name} will ask you to sign in. Your other accounts stay signed in.`;
  const name = preset || await ask({
    title: app.profiles.length ? `Add another ${app.name} account` : `Add an account to ${app.name}`,
    text: how,
    label: "Profile name",
    placeholder: "work",
    hint: "A short name you will recognise, like work or personal.",
    ok: "Continue",
  });
  if (!name) return;
  try {
    const step = await invoke("add_begin", { app: app.id, profile: name, close });
    if (step.next === "run_login") {
      waiting[app.id] = `Finish signing in in the window that opened. The account will be saved as “${name}”.`;
    }
    await load({ quiet: true });
  } catch (e) {
    if (e.code === "app_running" && !close) {
      const ok = await ask({ title: `Close ${app.name}?`, text: `${app.name} needs to close before adding an account.`, ok: "Close" });
      if (ok) return addAccount(app, name, true);
    } else if (e.code === "unsaved_session") {
      if (await saveCurrent(app)) return addAccount(app, name, close);
    } else {
      toast(message(e), true);
    }
  }
}

async function finishAdd(app) {
  try {
    const p = await invoke("add_finish", { app: app.id });
    toast(`Added ${p}`);
  } catch (e) {
    toast(message(e), true);
  }
  await load({ quiet: true });
}

async function cancelAdd(app) {
  try {
    const back = await invoke("add_cancel", { app: app.id });
    toast(back ? `Cancelled · back to ${back}` : "Cancelled");
  } catch (e) {
    toast(message(e), true);
  }
  await load({ quiet: true });
}

async function renameProfile(app, p) {
  const name = await ask({ title: "Rename", label: "Profile name", value: p.name, ok: "Rename" });
  if (!name || name === p.name) return;
  try {
    await invoke("rename_profile", { app: app.id, old: p.name, new: name });
  } catch (e) {
    toast(message(e), true);
  }
  await load({ quiet: true });
}

async function removeProfile(app, p) {
  const ok = await ask({
    title: `Remove ${p.name}?`,
    text: app.strategy === "native"
      ? `acswi forgets the name “${p.name}”. ${app.name} itself stays signed in to ${p.identity || p.name}.`
      : `The saved sign-in for ${p.name} is deleted. To use it again you will have to sign in again.`,
    ok: "Remove",
    danger: true,
  });
  if (!ok) return;
  try {
    await invoke("remove_profile", { app: app.id, profile: p.name });
  } catch (e) {
    toast(message(e), true);
  }
  await load({ quiet: true });
}

async function refreshSaved(app) {
  try {
    const p = await invoke("refresh_saved", { app: app.id });
    toast(p ? `Updated the saved copy of ${p}` : "Nothing to update");
  } catch (e) {
    toast(message(e), true);
  }
}

// ------------------------------------------------------------------ menus

function openMenu(anchor, items) {
  const menu = $("menu");
  menu.replaceChildren();
  for (const item of items) {
    if (item === "-") {
      menu.append(el("hr"));
      continue;
    }
    menu.append(el("button", {
      role: "menuitem",
      class: item.danger ? "danger" : "",
      onclick: () => {
        closeMenu();
        item.run();
      },
    }, item.label));
  }
  menu.hidden = false;
  anchor.setAttribute("aria-expanded", "true");
  menu.dataset.anchor = "";
  const r = anchor.getBoundingClientRect();
  const w = menu.offsetWidth;
  const h = menu.offsetHeight;
  menu.style.left = Math.max(8, Math.min(r.right - w, window.innerWidth - w - 8)) + "px";
  menu.style.top = (r.bottom + 4 + h > window.innerHeight ? r.top - h - 4 : r.bottom + 4) + "px";
  menu._anchor = anchor;
  menu.querySelector("button")?.focus();
}

function closeMenu() {
  const menu = $("menu");
  if (menu.hidden) return;
  menu.hidden = true;
  menu._anchor?.setAttribute("aria-expanded", "false");
}

document.addEventListener("click", (ev) => {
  if (!ev.target.closest("#menu") && !ev.target.closest("[aria-haspopup]")) closeMenu();
});
document.addEventListener("keydown", (ev) => {
  if (ev.key === "Escape") closeMenu();
  if (ev.key === "F5") {
    ev.preventDefault();
    load();
  }
});
window.addEventListener("blur", closeMenu);

function profileMenu(app, p, anchor) {
  const items = [{ label: "Rename…", run: () => renameProfile(app, p) }];
  if (p.active && app.strategy === "swap") {
    items.push({ label: "Update saved copy", run: () => refreshSaved(app) });
  }
  if (p.saved) items.push("-", { label: "Remove…", danger: true, run: () => removeProfile(app, p) });
  openMenu(anchor, items);
}

$("more").addEventListener("click", (ev) => {
  ev.stopPropagation();
  openMenu($("more"), [
    {
      label: showAll ? "Hide apps that aren’t installed" : "Show all supported apps",
      run: () => {
        showAll = !showAll;
        writePref("showAll", showAll);
        load();
      },
    },
    { label: "Open app definitions folder", run: () => invoke("open_apps_folder").catch((e) => toast(message(e), true)) },
  ]);
});

$("refresh").addEventListener("click", () => load());

// ------------------------------------------------------------------ dialog + toast

function ask({ title, text = "", label, value = "", placeholder = "", hint = "", ok = "OK", danger = false }) {
  const dialog = $("dialog");
  $("dialog-title").textContent = title;
  $("dialog-text").textContent = text;
  $("dialog-text").hidden = !text;
  const field = $("dialog-field");
  const input = $("dialog-input");
  field.hidden = !label;
  $("dialog-label").textContent = label || "";
  $("dialog-hint").textContent = hint;
  input.value = value;
  input.placeholder = placeholder;
  const okBtn = $("dialog-ok");
  okBtn.textContent = ok;
  okBtn.className = danger ? "danger" : "primary";
  dialog.returnValue = "";
  return new Promise((resolve) => {
    const done = () => {
      dialog.removeEventListener("close", done);
      if (dialog.returnValue !== "ok") return resolve(label ? null : false);
      resolve(label ? input.value.trim() || null : true);
    };
    dialog.addEventListener("close", done);
    $("dialog-cancel").onclick = () => dialog.close("cancel");
    dialog.showModal();
    if (label) {
      input.focus();
      input.select();
    } else {
      okBtn.focus();
    }
  });
}

let toastTimer;
function toast(text, isError = false) {
  const t = $("toast");
  t.textContent = text;
  t.className = isError ? "error" : "";
  t.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.hidden = true), isError ? 6000 : 2800);
}

// ------------------------------------------------------------------ events from Rust

listen("acswi://changed", ({ payload }) => {
  delete waiting[payload.app];
  if (payload.error) toast(message(payload.error), true);
  else if (payload.message) toast(payload.message);
  load({ quiet: true });
});

listen("acswi://confirm-close", async ({ payload }) => {
  await load({ quiet: true });
  const ok = await ask({
    title: `Close ${payload.name}?`,
    text: `${payload.name} is open. acswi will close it, switch to ${payload.profile} and open it again.`,
    ok: "Close and switch",
  });
  if (!ok) return;
  const app = apps.find((a) => a.id === payload.app) || { id: payload.app, name: payload.name };
  switchTo(app, payload.profile, null, true);
});

// Refresh when the window comes back, since accounts may have changed in a terminal.
window.addEventListener("focus", () => load({ quiet: true }));

load();
