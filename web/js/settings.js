// Settings: photo folders, browsing for one, the library status and About.
// One of the page's modules; main.js starts the page.
import { lang, languages, num, t, tn } from "./i18n.js";
import { $, api, del, esc, fmtBytes, icon, loadMeta, plural, post, pref } from "./core.js";
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
 *  those). Resolves to true once one was added, or, with `choose` (an async function of the
 *  path), once it accepted one; `title`, `intro` and `button` then say what for. With `files`
 *  (an extension), the files of each folder are listed too, and clicking one gives it to
 *  `choose` instead of the folder (no button then). `extra` is HTML under the list, whose
 *  `[data-extra]` buttons call `onExtra` with the dialog closed. */
export function browseForFolder({ choose = addFolder, title, intro, button, files, extra, onExtra } = {}) {
  const dlg = $("#browseDlg"), list = $(".browse-list", dlg), input = $("#browsePath"), err = $(".err", dlg);
  const hereBtn = $("[data-add-here]", dlg), extraBox = $(".browse-extra", dlg);
  const texts = [[$("#browseTitle"), title], [$(".dlg-body > p", dlg), intro], [hereBtn, button]];
  const before = texts.map(([el]) => el.innerHTML);
  for (const [el, text] of texts) if (text) el.innerHTML = text;
  hereBtn.hidden = !!files;
  extraBox.innerHTML = extra ?? "";
  extraBox.hidden = !extra;
  const folderIcon = icon("folder", 16), fileIcon = icon("file", 16);
  let here = null, token = 0;
  const open = async path => {
    const mine = ++token;
    // What the field says now: if it changes while this loads (a path typed or pasted
    // meanwhile), the answer doesn't overwrite it.
    const before = input.value;
    err.textContent = "";
    list.innerHTML = `<div class="empty">${t("Loading…")}</div>`;
    try {
      const query = new URLSearchParams();
      if (path != null) query.set("path", path);
      if (files) query.set("files", files);
      const r = await api(`/api/folders/browse${query.size ? `?${query}` : ""}`);
      if (mine !== token) return;
      here = r;
      if (input.value === before) {
        input.value = r.path;
        input.scrollLeft = input.scrollWidth; // the end of a long path is the folder you're in
      }
      $("[data-up]", dlg).disabled = r.parent == null;
      $("[data-add-here]", dlg).disabled = !r.path;
      const entries = [
        ...r.dirs.map(d => `<button type="button" data-dir="${esc(d.path)}">${folderIcon}<span>${esc(d.name)}</span></button>`),
        ...(r.files ?? []).map(f => `<button type="button" class="file" data-file="${esc(f.path)}">${fileIcon}<span>${esc(f.name)}</span></button>`),
      ];
      list.innerHTML = entries.length ? entries.join("")
        : `<div class="empty">${files ? t("No folders or backups inside.") : t("No folders inside.")}</div>`;
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
      if (target.dataset.extra != null) { dlg.close(); return onExtra?.(); }
      if (target.dataset.dir) return open(target.dataset.dir);
      if (target.dataset.file) {
        try { await choose(target.dataset.file); added = true; dlg.close(); }
        catch (e2) { err.textContent = e2.message; }
        return;
      }
      if (target.dataset.up != null && here?.parent != null) return open(here.parent);
      if (target.dataset.addHere != null && here?.path) {
        target.disabled = true;
        try { await choose(here.path); added = true; dlg.close(); }
        catch (e2) { err.textContent = e2.message; target.disabled = false; }
      }
    };
    const onKey = e => { if (e.key === "Enter" && e.target === input) { e.preventDefault(); open(input.value); } };
    dlg.addEventListener("click", onClick);
    dlg.addEventListener("keydown", onKey);
    dlg.addEventListener("close", () => {
      dlg.removeEventListener("click", onClick);
      dlg.removeEventListener("keydown", onKey);
      texts.forEach(([el], i) => { el.innerHTML = before[i]; });
      hereBtn.hidden = false;
      extraBox.hidden = true;
      resolve(added);
    }, { once: true });
    dlg.showModal();
    open(null);
  });
}
/** Tells the gallery where a folder that can't be found is now (moved, renamed, or copied
 *  to another drive): its photos and videos keep everything instead of being indexed again. */
async function moveFolder(from) {
  const move = async to => {
    const r = await post("/api/folders/move", { from, to });
    const parts = [tn(r.moved, "{n} file found in its new place", "{n} files found in their new place")];
    if (r.changed) parts.push(t("{n} changed, to be read again", { n: num(r.changed) }));
    if (r.missing) parts.push(t("{n} not there, removed from the gallery", { n: num(r.missing) }));
    toast(parts.join(" · "));
    await Promise.all([loadFolders(), loadMeta()]);
    renderSettings();
    render();
    setTimeout(pollStatus, 300);
  };
  if (folderInfo.desktop) {
    const picked = await post("/api/folders/pick", { save: false });
    if (picked) await move(picked.path);
    return;
  }
  await browseForFolder({
    choose: move,
    title: t("Where is this folder now?"),
    intro: `${t("Open the folder where its photos and videos are now. They keep their people, the faces you placed and everything else; files that changed are read again, and missing ones leave the gallery.")}<span class="browse-was">${t("It was in: {path}", { path: `<bdi>${esc(from)}</bdi>` })}</span>`,
    button: t("It's this one"),
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
        : `${f.available ? "" : `<button class="btn" data-move="${esc(f.path)}" title="${t("Tell the gallery where it is now: its photos keep their people and everything else, without being indexed again")}" ${removing ? "disabled" : ""}>${t("Moved to…")}</button>`}<button class="btn" data-remove="${esc(f.path)}" ${removing ? `disabled title="${t("Wait for the folder being removed")}"` : ""}>${t("Remove")}</button>`}
    </div>`).join("") : `<p>${t("No folders yet.")}</p>`;
  const add = `<div class="add-row"><button class="btn primary" ${desktop ? "data-pick" : "data-browse"} ${removing ? "disabled" : ""}>${t("Add folder…")}</button></div>`;
  // One card per part, so each is easy to tell apart.
  body.innerHTML = `<section class="set-part"><h4>${t("Folders")}</h4><p>${t("Subfolders are included. Files are only changed when you rotate or delete them.")}</p>${list}${add}<div class="err">${esc(error)}</div></section>
    <section class="set-part"><h4>${t("Library")}</h4><div class="scan-state" id="settingsScan">${scanStateHtml(lastStatus)}</div>
    <div id="settingsFailures">${failuresHtml()}</div>
    <div id="settingsExcluded">${excludedHtml(lastStatus)}</div>
    <div class="set-sub">${backupHtml()}</div></section>
    <section class="set-part"><h4>${t("People")}</h4><div class="scan-state"><div class="what">${t("Regroup all faces")}
      <small>${t("New faces are placed into people after every scan. This groups every face again from scratch instead: named people and faces you placed stay where they are, unnamed groups may change. It can take a long time on large libraries.")}</small></div>
      <button class="btn" data-regroup>${t("Regroup")}</button></div></section>
    <section class="set-part">${appearanceHtml()}</section>
    <section class="set-part" id="settingsUpdate">${updateHtml(updateInfo)}</section>
    <div id="settingsAbout">${aboutHtml(lastStatus)}</div>`;
}
// ---- updates ---------------------------------------------------------------------------
let updateInfo = null, updateTimer = 0;
const updateBusy = u => ["checking", "downloading", "installing", "restarting"].includes(u?.state);
/** This version, the latest release, and the buttons to check and to update. */
function updateHtml(u) {
  if (!u) return `<h4>${t("Updates")}</h4><p>${t("Loading…")}</p>`;
  const busy = updateBusy(u);
  const whatsNew = u.latest ? ` <a href="${esc(u.releases)}/tag/v${esc(u.latest)}" target="_blank" rel="noopener">${t("What's new")}</a>` : "";
  const text = u.available
    ? `<b class="update-available">${t("Imadive {latest} is available (you have {current}).", { latest: esc(u.latest), current: esc(u.current) })}</b>${whatsNew}`
    : u.latest ? t("Imadive {current} is up to date.", { current: esc(u.current) }) : t("This is Imadive {current}.", { current: esc(u.current) });
  let progress = "";
  if (u.state === "checking") progress = `<div>${t("Checking for updates…")}</div><progress></progress>`;
  if (u.state === "downloading") {
    progress = `<div>${u.total ? t("Downloading {done} of {total}", { done: fmtBytes(u.done), total: fmtBytes(u.total) }) : t("Downloading…")}</div>
      <progress ${u.total ? `max="${u.total}" value="${u.done}"` : ""}></progress>`;
  }
  if (u.state === "installing") progress = `<div>${t("Installing… The gallery restarts when it's done.")}</div><progress></progress>`;
  if (u.state === "restarting") progress = `<div>${t("Restarting…")}</div><progress></progress>`;
  // Copies that can't update themselves: what to run, and where (never in this browser).
  const lead = {
    source: t("This server was built from source, so it can't update itself. Run this once on the server, in a terminal or over SSH, to switch to the ready-made version: it keeps your index, your people and your folders, and from then on updates install from here with one click."),
    setup_without_helper: t("Run this once on the server, in a terminal or over SSH. It also adds the update service, so the next updates install from here."),
    docker: t("Run this on the computer that runs the container, then start the container again with the new image."),
  }[u.kind];
  const command = u.available && !u.can_install
    ? u.command && lead
      ? `<p>${lead}</p><div class="update-cmd"><code>${esc(u.command)}</code><button class="btn" data-copy-command>${t("Copy")}</button></div>`
      : `<p>${t("This copy can't update itself: get the new version from the releases page.")}</p>`
    : "";
  const buttons = `<div class="add-row">${u.available && u.can_install ? `<button class="btn primary" data-update-install ${busy ? "disabled" : ""}>${t("Update")}</button>` : ""}
    <button class="btn" data-update-check ${busy ? "disabled" : ""}>${t("Check for updates")}</button></div>`;
  return `<h4>${t("Updates")}</h4><p>${text}</p>
    ${progress ? `<div class="remove-progress">${progress}</div>` : ""}
    ${u.state === "error" && u.error ? `<div class="err">${esc(t("The last attempt failed: {error}", { error: u.error }))}</div>` : ""}
    ${command}${buttons}
    <p><small>${[u.checked ? t("Last checked on GitHub on {when}.", { when: esc(u.checked) }) : "",
      u.automatic ? t("Imadive looks for new releases once a day.") : t("Imadive only looks for new releases when you ask.")].filter(Boolean).join(" ")}</small></p>`;
}
/** Fetches the update state; while something is going on, again every second. */
async function loadUpdate(url = "/api/update", method = "GET") {
  clearTimeout(updateTimer);
  try { updateInfo = await api(url, { method }); } catch (err) { updateInfo = { ...updateInfo, state: "error", error: err.message }; }
  const el = $("#settingsUpdate");
  if (el) el.innerHTML = updateHtml(updateInfo);
  if (updateBusy(updateInfo) && $("#settingsDlg").open) updateTimer = setTimeout(loadUpdate, 1000);
}
/** Backing up the index, and restoring a backup. */
function backupHtml() {
  return `<h5>${t("Backup")}</h5>
    <p>${t("A backup keeps people and their names, face groups, your corrections and the list of folders. The photos and videos themselves aren't in it: back them up on their own. Neither are the theme and the language, which each browser keeps.")}</p>
    <div class="add-row"><button class="btn" data-backup ${backupBusy ? "disabled" : ""}>${t("Back up…")}</button><button class="btn" data-restore ${backupBusy ? "disabled" : ""}>${t("Restore…")}</button></div>
    <div id="backupProgress">${backupState}</div>
    <input type="file" id="restoreFile" accept=".imadive" hidden>`;
}
/** What the Backup card shows under its buttons, kept across redraws of Settings. */
let backupState = "", backupBusy = false, backupTimer = 0;
function showBackup(html, busy) {
  backupState = html;
  backupBusy = busy;
  const el = $("#backupProgress");
  if (el) el.innerHTML = html;
  for (const b of document.querySelectorAll("#settingsDlg [data-backup], #settingsDlg [data-restore]")) b.disabled = busy;
}
const progressBar = (text, done, total) =>
  `<div class="remove-progress"><div>${text}</div><progress ${total ? `max="${total}" value="${done}"` : ""}></progress></div>`;
/** Follows the backup being made until it is done. */
async function watchBackup() {
  clearTimeout(backupTimer);
  let p;
  try { p = await api("/api/backup/progress"); } catch (err) { return showBackup(`<div class="err">${esc(err.message)}</div>`, false); }
  if (p.running) {
    const pct = p.total ? Math.min(100, Math.floor((100 * p.done) / p.total)) : 0;
    showBackup(progressBar(p.total ? t("Making the backup… {pct}%", { pct }) : t("Making the backup…"), p.done, p.total), true);
    backupTimer = setTimeout(watchBackup, 500);
    return;
  }
  if (p.error) {
    showBackup(`<div class="err">${esc(t("Couldn't make the backup: {error}", { error: p.error }))}</div>`, false);
    toast(t("Couldn't make the backup: {error}", { error: p.error }), true);
  } else if (p.path) {
    showBackup(`<p class="backup-done">${t("Saved in {path}", { path: `<bdi>${esc(p.path)}</bdi>` })}</p>`, false);
    toast(t("Backup saved in {path}", { path: p.path }));
  } else showBackup("", false);
}
/** Makes a backup: in a folder of the server chosen in its folder browser, or where the
 *  desktop app's save dialog says. It runs in the background; the card follows it. */
async function backUp() {
  const start = async folder => {
    // Said at once: the backup of a large library takes a while.
    showBackup(progressBar(t("Making the backup…")), true);
    try { return await post("/api/backup/save", folder == null ? {} : { folder }); }
    catch (err) { showBackup("", false); throw err; }
  };
  if (folderInfo.desktop) {
    const r = await start(null);
    if (!r) return showBackup("", false); // the save dialog was cancelled
    return watchBackup();
  }
  const chosen = await browseForFolder({
    choose: start,
    title: t("Where to save the backup"),
    intro: t("Folders on the computer running Imadive. The backup is saved in the folder you're in."),
    button: t("Save here"),
  });
  if (chosen) watchBackup();
}
/** What a backup holds, then restoring it once confirmed. */
async function confirmRestore(summary) {
  const folders = summary.folders.map(f => `<li><bdi>${esc(f.path)}</bdi>${f.here ? "" : ` <span class="off">${t("not on this computer")}</span>`}</li>`).join("");
  const missing = summary.folders.some(f => !f.here);
  const choice = await askChoice(`<p>${t("Made on {date} by Imadive {version}.", { date: esc(summary.created), version: esc(summary.version) })}</p>
    <p>${[tn(summary.photos, "{n} photo", "{n} photos"), tn(summary.videos, "{n} video", "{n} videos"), tn(summary.named, "{n} named person", "{n} named people")].join(" · ")}</p>
    ${folders ? `<p>${t("Folders:")}</p><ul class="restore-folders">${folders}</ul>` : ""}
    ${missing ? `<p>${t("The photos of folders that aren't on this computer are kept, shown as not available. If they are somewhere else here, use <b>Moved to…</b> in Settings afterwards.")}</p>` : ""}
    <p>${t("Everything in this gallery is replaced by the backup: people, names, groups and folders. The photos and videos aren't touched. The index it replaces is kept in the data folder, as <b>index.before-restore.sqlite</b>.")}</p>
    <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button><button class="btn danger" data-choice="restore">${t("Restore")}</button></div>`,
    t("Restore this backup?"));
  if (choice !== "restore") return;
  showBackup(progressBar(t("Restoring the backup…")), true);
  try { await post("/api/backup/restore"); }
  catch (err) { showBackup("", false); throw err; }
  // Everything the page holds came from the index it replaced.
  location.reload();
}
async function restoreFrom(file) {
  showBackup(progressBar(t("Reading the backup…")), true);
  let summary;
  try { summary = await api("/api/backup/inspect", { method: "PUT", body: file }); }
  finally { showBackup("", false); }
  await confirmRestore(summary);
}
/** Restore…: a backup file on the server, chosen in its folder browser, or one uploaded from
 *  this device; in the desktop app, the system's open dialog. */
async function restore() {
  if (folderInfo.desktop) {
    const summary = await post("/api/backup/inspect", { pick: true });
    return summary && confirmRestore(summary);
  }
  let summary = null;
  await browseForFolder({
    files: "imadive",
    title: t("Choose a backup"),
    intro: t("Folders and backups on the computer running Imadive. Click the backup to restore."),
    choose: async path => { summary = await post("/api/backup/inspect", { path }); },
    extra: `<button type="button" class="btn" data-extra>${t("Upload one from this device…")}</button>`,
    onExtra: () => $("#restoreFile").click(),
  });
  if (summary) await confirmRestore(summary);
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
  if (e.target.id === "restoreFile" && e.target.files[0]) {
    const file = e.target.files[0];
    e.target.value = "";
    restoreFrom(file).catch(err => toast(t("Couldn't restore the backup: {error}", { error: err.message }), true));
  }
  if (e.target.id === "themePref") applyTheme(e.target.value);
  // Everything on the page is written in the language as it is drawn: start again in the new one.
  if (e.target.id === "langPref") { pref.set("lang", e.target.value); location.reload(); }
});
/** Version, license and the project's pages. */
function aboutHtml(st) {
  if (!st?.version) return "";
  const link = (path, text) => `<a href="${esc(st.project)}/${path}" target="_blank" rel="noopener">${text}</a>`;
  return `<section class="set-part"><h4>${t("About")}</h4><div class="about">
    <p>${t("<b>Imadive {version}</b>. Free for personal and other non-commercial use under the PolyForm Noncommercial License 1.0.0. The face recognition models are for non-commercial use only.", { version: esc(st.version) })}</p>
    <p>${link("blob/main/LICENSE", t("License"))} · ${link("blob/main/THIRD_PARTY.md", t("Third-party components"))} ·
      ${link("releases", t("Releases"))}</p>
    ${st.logs ? `<p><button class="btn" data-logs>${t("Open log folder")}</button> <small>${t("For reporting a problem: the log of this run, and of the one before.")}</small></p>` : ""}</div></section>
    <p class="about-coffee settings-coffee">${coffeeHtml(st).replace("<br>", " ")}</p>`;
}
/** The invitation to support the project, as in the About dialog. */
function coffeeHtml(st) {
  const coffee = st?.donate
    ? `<a href="${esc(st.donate)}" target="_blank" rel="noopener">${t("buying me a coffee")}</a>`
    : t("buying me a coffee");
  return t("ImaDive is free. If you like it, please<br>consider {coffee}&nbsp;☕", { coffee });
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
  loadUpdate();
  // A backup started earlier (or from another device) is still being made.
  if (lastStatus?.backing_up) watchBackup();
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
  $(".about-coffee", dlg).innerHTML = coffeeHtml(st);
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
  if (e.target.dataset.updateCheck != null) return loadUpdate("/api/update/check", "POST");
  if (e.target.dataset.updateInstall != null) {
    e.target.disabled = true;
    return post("/api/update/install").then(() => loadUpdate()).catch(err => {
      toast(t("Couldn't update: {error}", { error: err.message }), true);
      loadUpdate();
    });
  }
  if (e.target.dataset.copyCommand != null) {
    return navigator.clipboard.writeText(updateInfo?.command ?? "").then(() => toast(t("Copied"))).catch(() => toast(t("Couldn't copy it; select it and copy it yourself"), true));
  }
  if (e.target.dataset.backup != null) {
    return backUp().catch(err => toast(t("Couldn't make the backup: {error}", { error: err.message }), true));
  }
  if (e.target.dataset.restore != null) {
    return restore().catch(err => toast(t("Couldn't restore the backup: {error}", { error: err.message }), true));
  }
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
    else if (e.target.dataset.move) await moveFolder(e.target.dataset.move);
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
