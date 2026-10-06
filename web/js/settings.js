// Settings: photo folders, browsing for one, the library status and About.
// One of the page's modules; main.js starts the page.
import { lang, languages, num, t, tn } from "./i18n.js";
import { $, api, del, esc, icon, loadMeta, plural, post, pref } from "./core.js";
import { items } from "./items.js";
import { toast } from "./people.js";
import { render } from "./views.js";
import { askChoice } from "./viewer.js";
import { lastStatus, pollStatus, setLastStatus, updateIndexPill } from "./status.js";

// ---- folders -----------------------------------------------------------------------
export let folderInfo = { folders: [], desktop: false };
export async function loadFolders() {
  folderInfo = await api("/api/folders");
  return folderInfo;
}
/** Picks a folder on the server by browsing its folders (the browser has no picker for
 *  those). Resolves to true once one was added. */
export function browseForFolder() {
  const dlg = $("#browseDlg"), list = $(".browse-list", dlg), input = $("#browsePath"), err = $(".err", dlg);
  const folderIcon = icon("folder", 16);
  let here = null, token = 0;
  const open = async path => {
    const mine = ++token;
    err.textContent = "";
    list.innerHTML = `<div class="empty">${t("Loading…")}</div>`;
    try {
      const r = await api(`/api/folders/browse${path == null ? "" : `?path=${encodeURIComponent(path)}`}`);
      if (mine !== token) return;
      here = r;
      input.value = r.path;
      input.scrollLeft = input.scrollWidth; // the end of a long path is the folder you're in
      $("[data-up]", dlg).disabled = r.parent == null;
      $("[data-add-here]", dlg).disabled = !r.path;
      list.innerHTML = r.dirs.length
        ? r.dirs.map(d => `<button type="button" data-dir="${esc(d.path)}">${folderIcon}<span>${esc(d.name)}</span></button>`).join("")
        : `<div class="empty">${t("No folders inside.")}</div>`;
      list.scrollTop = 0;
    } catch (e) {
      if (mine !== token) return;
      err.textContent = e.message;
      list.innerHTML = "";
      if (here) input.value = here.path;
    }
  };
  return new Promise(resolve => {
    let added = false;
    const onClick = async e => {
      const target = e.target.closest("button") ?? e.target;
      if (target === dlg || target.dataset.close != null) return dlg.close();
      if (target.dataset.dir) return open(target.dataset.dir);
      if (target.dataset.up != null && here?.parent != null) return open(here.parent);
      if (target.dataset.addHere != null && here?.path) {
        target.disabled = true;
        try { await addFolder(here.path); added = true; dlg.close(); }
        catch (e2) { err.textContent = e2.message; target.disabled = false; }
      }
    };
    const onKey = e => { if (e.key === "Enter" && e.target === input) { e.preventDefault(); open(input.value); } };
    dlg.addEventListener("click", onClick);
    dlg.addEventListener("keydown", onKey);
    dlg.addEventListener("close", () => {
      dlg.removeEventListener("click", onClick);
      dlg.removeEventListener("keydown", onKey);
      resolve(added);
    }, { once: true });
    dlg.showModal();
    open(null);
  });
}
export async function addFolder(path) {
  const added = await post(path == null ? "/api/folders/pick" : "/api/folders", path == null ? {} : { path });
  if (added === null) return false; // picker cancelled
  await loadFolders();
  setTimeout(pollStatus, 300);
  return true;
}
// ---- settings: photo folders and indexing -------------------------------------------
export function scanStateHtml(st) {
  if (!st) return `<div class="what">${t("Checking…")}</div>`;
  const rescan = `<button class="btn" data-rescan title="${st.running ? t("Scan again when this one ends") : t("Look for new, changed or deleted photos and videos")}">${t("Rescan")}</button>`;
  if (st.running && st.phase === "grouping faces") {
    const pct = st.group_total ? Math.min(100, Math.floor((100 * st.group_done) / st.group_total)) : 0;
    return `<div class="what">${t("Grouping faces {pct}%", { pct })}<small>${st.faces ? tn(st.faces, "Placing faces into people, {n} new face", "Placing faces into people, {n} new faces") : t("Placing faces into people")}</small>
      <progress max="${st.group_total || 1}" value="${st.group_done}"></progress></div>${rescan}`;
  }
  if (st.running && st.phase === "indexing files") {
    return `<div class="what">${t("Indexing {done} of {total} files", { done: num(st.done), total: num(st.total) })}<small>${st.faces ? tn(st.faces, "{n} face found", "{n} faces found") : t("Reading photos and videos, making thumbnails, finding faces")}</small>
      <progress max="${st.total || 1}" value="${st.done}"></progress></div>${rescan}`;
  }
  if (st.running) return `<div class="what">${t("Looking for new or changed files…")}<progress></progress></div>${rescan}`;
  return `<div class="what">${st.phase === "failed" ? t("The last scan failed") : t("Up to date")}</div>${rescan}`;
}

// Photos removed from the gallery (their files kept), with a way to bring them back.
export function excludedHtml(st) {
  const n = st?.excluded ?? 0;
  if (!n) return "";
  return `<div class="scan-state"><div class="what">${tn(n, "{n} file removed from the gallery", "{n} files removed from the gallery")}
      <small>${t("Their files are still on disk. Showing them again indexes them at the next scan.")}</small></div>
    <button class="btn" data-show-excluded>${t("Show again")}</button></div>`;
}

// Files that could not be indexed, with the reason.
export let failureCount = null, failureList = [];
export async function loadFailures() {
  const count = lastStatus?.failed ?? 0;
  failureList = count ? await api("/api/failures").catch(() => []) : [];
  failureCount = count;
  const el = $("#settingsFailures");
  if (el) el.innerHTML = failuresHtml();
}
function failuresHtml() {
  if (!failureCount) return "";
  const more = failureCount > failureList.length ? `<li class="more">${t("and {n} more", { n: num(failureCount - failureList.length) })}</li>` : "";
  return `<div class="failures">
    <div class="scan-state"><div class="what">${tn(failureCount, "{n} file could not be read", "{n} files could not be read")}
      <small>${t("They are skipped until the file changes.")}</small></div>
      <button class="btn" data-retry title="${t("Try these files again now")}">${t("Try again")}</button></div>
    <details><summary>${t("Show the files")}</summary><ul>${failureList.map(f =>
      `<li><span class="p" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span><span class="why">${esc(f.error)}</span></li>`).join("")}${more}</ul></details>
  </div>`;
}
/** "Removing 1,200 of 40,000 photos", or waiting for a scan to stop first. */
export function removeProgressHtml(st) {
  if (!st?.remove_total) {
    return `<div>${st?.running ? t("Stopping the scan first…") : t("Removing its photos and videos…")}</div><progress></progress>`;
  }
  return `<div>${t("Removing {done} of {total} files", { done: num(st.remove_done), total: num(st.remove_total) })}</div>
    <progress max="${st.remove_total}" value="${st.remove_done}"></progress>`;
}
export function renderSettings(error = "") {
  const dlg = $("#settingsDlg"), body = $(".dlg-body", dlg);
  const { folders, desktop } = folderInfo;
  const removing = lastStatus?.removing ?? null;
  const list = folders.length ? folders.map(f => f.path === removing
    ? `<div class="folder removing">
        <span class="p" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span>
        <div class="remove-progress" id="removeProgress">${removeProgressHtml(lastStatus)}</div>
      </div>`
    : `<div class="folder">
      <span class="p" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span>
      ${f.available ? `<span class="c">${plural(f.items, "file")}</span>` : `<span class="off" title="${t("The folder can't be found right now. Its photos and videos are kept until it's back or you remove it.")}">${t("not available")}</span>`}
      ${f.fixed
        ? `<span class="fixed" title="${t("Given on the command line when Imadive was started; remove it there")}">${t("command line")}</span>`
        : `<button class="btn" data-remove="${esc(f.path)}" ${removing ? `disabled title="${t("Wait for the folder being removed")}"` : ""}>${t("Remove")}</button>`}
    </div>`).join("") : `<p>${t("No folders yet.")}</p>`;
  const add = `<div class="add-row"><button class="btn primary" ${desktop ? "data-pick" : "data-browse"} ${removing ? "disabled" : ""}>${t("Add folder…")}</button></div>`;
  body.innerHTML = `<h4>${t("Folders")}</h4><p>${t("Subfolders are included. Files are only changed when you rotate or delete them.")}</p>${list}${add}<div class="err">${esc(error)}</div>
    <h4>${t("Library")}</h4><div class="scan-state" id="settingsScan">${scanStateHtml(lastStatus)}</div>
    <div id="settingsFailures">${failuresHtml()}</div>
    <div id="settingsExcluded">${excludedHtml(lastStatus)}</div>
    <h4>${t("People")}</h4><div class="scan-state"><div class="what">${t("Regroup all faces")}
      <small>${t("New faces are placed into people after every scan. This groups every face again from scratch instead: named people and faces you placed stay where they are, unnamed groups may change. It can take a long time on large libraries.")}</small></div>
      <button class="btn" data-regroup>${t("Regroup")}</button></div>
    ${appearanceHtml()}
    <div id="settingsAbout">${aboutHtml(lastStatus)}</div>`;
}
/** The theme and the language, remembered in this browser. */
function appearanceHtml() {
  const option = (value, label, current) => `<option value="${value}" ${value === current ? "selected" : ""}>${label}</option>`;
  const theme = pref.get("theme") ?? "";
  return `<h4>${t("Appearance")}</h4><div class="prefs">
    <label>${t("Theme")} <select id="themePref">
      ${option("", t("System"), theme)}${option("light", t("Light"), theme)}${option("dark", t("Dark"), theme)}
    </select></label>
    <label>${t("Language")} <select id="langPref">
      ${Object.entries(languages).map(([code, name]) => option(code, name, lang)).join("")}
    </select></label>
  </div>`;
}
/** The theme chosen: "light", "dark", or "" to follow the system. index.html applies the
 *  saved one before the page is drawn, so it never flashes the other one. */
function applyTheme(theme) {
  pref.set("theme", theme);
  if (theme) document.documentElement.dataset.theme = theme;
  else delete document.documentElement.dataset.theme;
}
$("#settingsDlg").addEventListener("change", e => {
  if (e.target.id === "themePref") applyTheme(e.target.value);
  // Everything on the page is written in the language as it is drawn: start again in the new one.
  if (e.target.id === "langPref") { pref.set("lang", e.target.value); location.reload(); }
});
/** Version, license and the project's pages. */
function aboutHtml(st) {
  if (!st?.version) return "";
  const link = (path, text) => `<a href="${esc(st.project)}/${path}" target="_blank" rel="noopener">${text}</a>`;
  return `<h4>${t("About")}</h4><div class="about">
    <p>${t("<b>Imadive {version}</b>. Free for personal and other non-commercial use under the PolyForm Noncommercial License 1.0.0. The face recognition models are for non-commercial use only.", { version: esc(st.version) })}</p>
    <p>${link("blob/main/LICENSE", t("License"))} · ${link("blob/main/THIRD_PARTY.md", t("Third-party components"))} ·
      ${link("releases", t("Releases"))}</p>
    ${st.logs ? `<p><button class="btn" data-logs>${t("Open log folder")}</button> <small>${t("For reporting a problem: the log of this run, and of the one before.")}</small></p>` : ""}</div>`;
}
export function openSettings() {
  renderSettings();
  // Fresh status for the Library part (it is otherwise polled only every few seconds).
  api("/api/status").then(st => {
    setLastStatus(st);
    if (!$("#settingsDlg").open) return;
    $("#settingsScan").innerHTML = scanStateHtml(st);
    $("#settingsAbout").innerHTML = aboutHtml(st);
    $("#settingsExcluded").innerHTML = excludedHtml(st);
    if (st.failed !== failureCount) loadFailures();
  }).catch(() => {});
  if (lastStatus?.failed !== failureCount) loadFailures();
  const dlg = $("#settingsDlg");
  if (!dlg.open) dlg.showModal();
}
// ---- the About dialog, opened from the logo -------------------------------------------
/** The icon, the name, the version, and an invitation to support the project. The logos are
 *  copies of the top bar's with their own ids: one of the bar's is always hidden, and a
 *  gradient defined in a hidden SVG doesn't draw. */
async function openAbout() {
  const dlg = $("#aboutDlg"), art = $(".about-art", dlg);
  if (!art.firstChild) {
    art.innerHTML = [".brand-icon", ".brand-name"].map(s => $(s).outerHTML.replaceAll("brand-", "about-")).join("");
  }
  const st = lastStatus?.version ? lastStatus : await api("/api/status").catch(() => null);
  $(".about-version", dlg).textContent = st?.version ? `Imadive v${st.version}` : "Imadive";
  const coffee = st?.donate
    ? `<a href="${esc(st.donate)}" target="_blank" rel="noopener">${t("buying me a coffee")}</a>`
    : t("buying me a coffee");
  $(".about-coffee", dlg).innerHTML = t("ImaDive is free. If you like it and find it useful, please consider {coffee}&nbsp;☕", { coffee });
  const author = st?.author ? `<a href="${esc(st.author)}" target="_blank" rel="noopener">W4T</a>` : "W4T";
  $(".about-credit", dlg).innerHTML = t("Created by {author}", { author });
  if (!dlg.open) dlg.showModal();
}
$("#brandBtn").onclick = openAbout;
$("#aboutDlg").addEventListener("click", e => {
  const dlg = $("#aboutDlg");
  if (e.target === dlg || e.target.closest("[data-close]")) return dlg.close();
  // The desktop app opens links in the system browser.
  const link = e.target.closest("a[target=_blank]");
  if (link && folderInfo.desktop) {
    e.preventDefault();
    post("/api/open", { url: link.href }).catch(err => toast(t("Couldn't open the link: {error}", { error: err.message }), true));
  }
});
$("#settingsBtn").onclick = () => { openSettings(); loadFolders().then(() => { if ($("#settingsDlg").open) renderSettings(); }); };
$("#settingsDlg").addEventListener("click", async e => {
  const dlg = $("#settingsDlg");
  if (e.target === dlg || e.target.dataset.close != null) return dlg.close();
  // The desktop app opens links in the system browser.
  const link = e.target.closest("a[target=_blank]");
  if (link && folderInfo.desktop) {
    e.preventDefault();
    return post("/api/open", { url: link.href }).catch(err => toast(t("Couldn't open the link: {error}", { error: err.message }), true));
  }
  if (e.target.closest("[data-logs]")) {
    return post("/api/logs/reveal").catch(err => toast(t("Couldn't open the log folder: {error}", { error: err.message }), true));
  }
  const start = async (button, url, method = "POST") => {
    button.disabled = true;
    button.textContent = t("Starting…");
    await (method === "DELETE" ? del(url) : post(url)).catch(err => renderSettings(err.message));
    setTimeout(pollStatus, 300);
  };
  if (e.target.dataset.regroup != null) return start(e.target, "/api/regroup");
  if (e.target.dataset.showExcluded != null) return start(e.target, "/api/excluded", "DELETE");
  if (e.target.dataset.retry != null) return start(e.target, "/api/failures/retry");
  if (e.target.dataset.rescan != null) {
    e.target.disabled = true;
    e.target.textContent = t("Starting…");
    await post("/api/scan").catch(err => renderSettings(err.message));
    return setTimeout(pollStatus, 300);
  }
  try {
    if (e.target.dataset.pick != null) { if (await addFolder(null)) renderSettings(); }
    else if (e.target.dataset.browse != null) { if (await browseForFolder()) { await loadFolders(); renderSettings(); } }
    else if (e.target.dataset.remove) {
      const path = e.target.dataset.remove;
      const choice = await askChoice(`<p class="gone-path">${esc(path)}</p>
        <p>${t("Its photos and videos leave the gallery, with their faces. The files stay on disk, and adding the folder again brings them back.")}</p>
        <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button><button class="btn danger" data-choice="remove">${t("Remove folder")}</button></div>`,
        t("Remove this folder from the gallery?"));
      if (choice !== "remove") return;
      // Progress in the folder's row at once; the status polling keeps it current.
      setLastStatus({ ...lastStatus, removing: path, remove_done: 0, remove_total: 0 });
      renderSettings();
      const watch = setInterval(async () => {
        const st = await api("/api/status").catch(() => null);
        if (!st?.removing) return;
        setLastStatus(st);
        const el = $("#removeProgress");
        if (el) el.innerHTML = removeProgressHtml(st);
        updateIndexPill(st);
      }, 500);
      let r;
      try { r = await del("/api/folders", { path }); }
      finally {
        clearInterval(watch);
        setLastStatus({ ...lastStatus, removing: null });
      }
      toast(tn(r.removed, "{n} file removed from the gallery", "{n} files removed from the gallery")
        + (r.kept ? `; ${tn(r.kept, "{n} stays, as another folder includes it", "{n} stay, as another folder includes them")}` : ""));
      await Promise.all([loadFolders(), loadMeta()]);
      renderSettings();
      render(); // the view behind Settings shows the photos that are left
      setTimeout(pollStatus, 300);
    }
  } catch (err) { renderSettings(err.message); }
});
$("#settingsDlg").addEventListener("close", () => { if (!items.length) render(); });
