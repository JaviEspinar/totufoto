// Settings: photo folders, browsing for one, the library status and About.
// One of the page's modules; main.js starts the page.
import { $, api, esc, icon, loadMeta, plural, post } from "./core.js";
import { photos } from "./photos.js";
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
    list.innerHTML = `<div class="empty">Loading…</div>`;
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
        : `<div class="empty">No folders inside.</div>`;
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
      const t = e.target.closest("button") ?? e.target;
      if (t === dlg || t.dataset.close != null) return dlg.close();
      if (t.dataset.dir) return open(t.dataset.dir);
      if (t.dataset.up != null && here?.parent != null) return open(here.parent);
      if (t.dataset.addHere != null && here?.path) {
        t.disabled = true;
        try { await addFolder(here.path); added = true; dlg.close(); }
        catch (e2) { err.textContent = e2.message; t.disabled = false; }
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
  if (!st) return `<div class="what">Checking…</div>`;
  const rescan = `<button class="btn" data-rescan title="${st.running ? "Scan again when this one ends" : "Look for new, changed or deleted photos"}">Rescan</button>`;
  if (st.running && st.phase === "grouping faces") {
    const pct = st.group_total ? Math.min(100, Math.floor((100 * st.group_done) / st.group_total)) : 0;
    return `<div class="what">Grouping faces ${pct}%<small>Placing faces into people${st.faces ? `, ${st.faces.toLocaleString()} new faces` : ""}</small>
      <progress max="${st.group_total || 1}" value="${st.group_done}"></progress></div>${rescan}`;
  }
  if (st.running && st.phase === "indexing photos") {
    return `<div class="what">Indexing ${st.done.toLocaleString()} of ${st.total.toLocaleString()} photos<small>${st.faces ? `${st.faces.toLocaleString()} faces found` : "Reading photos, making thumbnails, finding faces"}</small>
      <progress max="${st.total || 1}" value="${st.done}"></progress></div>${rescan}`;
  }
  if (st.running) return `<div class="what">Looking for new or changed photos…<progress></progress></div>${rescan}`;
  return `<div class="what">${st.phase === "failed" ? "The last scan failed" : "Up to date"}</div>${rescan}`;
}

// Photos removed from the gallery (their files kept), with a way to bring them back.
export function excludedHtml(st) {
  const n = st?.excluded ?? 0;
  if (!n) return "";
  return `<div class="scan-state"><div class="what">${plural(n, "photo")} removed from the gallery
      <small>Their files are still on disk. Showing them again indexes them at the next scan.</small></div>
    <button class="btn" data-show-excluded>Show again</button></div>`;
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
  const more = failureCount > failureList.length ? `<li class="more">and ${(failureCount - failureList.length).toLocaleString()} more</li>` : "";
  return `<div class="failures">
    <div class="scan-state"><div class="what">${plural(failureCount, "file")} could not be read
      <small>They are skipped until the file changes.</small></div>
      <button class="btn" data-retry title="Try these files again now">Try again</button></div>
    <details><summary>Show the files</summary><ul>${failureList.map(f =>
      `<li><span class="p" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span><span class="why">${esc(f.error)}</span></li>`).join("")}${more}</ul></details>
  </div>`;
}
/** "Removing 1,200 of 40,000 photos", or waiting for a scan to stop first. */
export function removeProgressHtml(st) {
  if (!st?.remove_total) {
    return `<div>${st?.running ? "Stopping the scan first…" : "Removing its photos…"}</div><progress></progress>`;
  }
  return `<div>Removing ${st.remove_done.toLocaleString()} of ${st.remove_total.toLocaleString()} photos</div>
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
      ${f.available ? `<span class="c">${f.photos.toLocaleString()} photos</span>` : `<span class="off" title="The folder can't be found right now. Its photos are kept until it's back or you remove it.">not available</span>`}
      ${f.fixed
        ? `<span class="fixed" title="Given on the command line when Imadive was started; remove it there">command line</span>`
        : `<button class="btn" data-remove="${esc(f.path)}" ${removing ? `disabled title="Wait for the folder being removed"` : ""}>Remove</button>`}
    </div>`).join("") : `<p>No folders yet.</p>`;
  const add = `<div class="add-row"><button class="btn primary" ${desktop ? "data-pick" : "data-browse"} ${removing ? "disabled" : ""}>Add folder…</button></div>`;
  body.innerHTML = `<h4>Photo folders</h4><p>Subfolders are included. Photos are only changed when you rotate or delete them.</p>${list}${add}<div class="err">${esc(error)}</div>
    <h4>Library</h4><div class="scan-state" id="settingsScan">${scanStateHtml(lastStatus)}</div>
    <div id="settingsFailures">${failuresHtml()}</div>
    <div id="settingsExcluded">${excludedHtml(lastStatus)}</div>
    <h4>People</h4><div class="scan-state"><div class="what">Regroup all faces
      <small>New faces are placed into people after every scan. This groups every face again from scratch instead: named people and faces you placed stay where they are, unnamed groups may change. It can take a long time on large libraries.</small></div>
      <button class="btn" data-regroup>Regroup</button></div>
    <div id="settingsAbout">${aboutHtml(lastStatus)}</div>`;
}
/** Version, license and the project's pages. */
function aboutHtml(st) {
  if (!st?.version) return "";
  const link = (path, text) => `<a href="${esc(st.project)}/${path}" target="_blank" rel="noopener">${text}</a>`;
  return `<h4>About</h4><div class="about">
    <p><b>Imadive ${esc(st.version)}</b>. Free for personal and other non-commercial use under the
      PolyForm Noncommercial License 1.0.0. The face recognition models are for non-commercial use only.</p>
    <p>${link("blob/main/LICENSE", "License")} · ${link("blob/main/THIRD_PARTY.md", "Third-party components")} ·
      ${link("releases", "Releases")}</p>
    ${st.logs ? `<p><button class="btn" data-logs>Open log folder</button> <small>For reporting a problem: the log of this run, and of the one before.</small></p>` : ""}</div>`;
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
$("#settingsBtn").onclick = () => { openSettings(); loadFolders().then(() => { if ($("#settingsDlg").open) renderSettings(); }); };
$("#settingsDlg").addEventListener("click", async e => {
  const dlg = $("#settingsDlg");
  if (e.target === dlg || e.target.dataset.close != null) return dlg.close();
  // The desktop app opens links in the system browser.
  const link = e.target.closest("a[target=_blank]");
  if (link && folderInfo.desktop) {
    e.preventDefault();
    return post("/api/open", { url: link.href }).catch(err => toast(`Couldn't open the link: ${err.message}`, true));
  }
  if (e.target.closest("[data-logs]")) {
    return post("/api/logs/reveal").catch(err => toast(`Couldn't open the log folder: ${err.message}`, true));
  }
  const start = async (button, url) => {
    button.disabled = true;
    button.textContent = "Starting…";
    await post(url).catch(err => renderSettings(err.message));
    setTimeout(pollStatus, 300);
  };
  if (e.target.dataset.regroup != null) return start(e.target, "/api/regroup");
  if (e.target.dataset.showExcluded != null) return start(e.target, "/api/excluded/clear");
  if (e.target.dataset.retry != null) return start(e.target, "/api/failures/retry");
  if (e.target.dataset.rescan != null) {
    e.target.disabled = true;
    e.target.textContent = "Starting…";
    await post("/api/scan").catch(err => renderSettings(err.message));
    return setTimeout(pollStatus, 300);
  }
  try {
    if (e.target.dataset.pick != null) { if (await addFolder(null)) renderSettings(); }
    else if (e.target.dataset.browse != null) { if (await browseForFolder()) { await loadFolders(); renderSettings(); } }
    else if (e.target.dataset.remove) {
      const path = e.target.dataset.remove;
      const choice = await askChoice(`<p class="gone-path">${esc(path)}</p>
        <p>Its photos leave the gallery, with their faces. The files stay on disk, and adding the folder again brings them back.</p>
        <div class="dlg-actions"><button class="btn" data-choice="cancel">Cancel</button><button class="btn danger" data-choice="remove">Remove folder</button></div>`,
        "Remove this folder from the gallery?");
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
      try { r = await post("/api/folders/remove", { path }); }
      finally {
        clearInterval(watch);
        setLastStatus({ ...lastStatus, removing: null });
      }
      toast(`${plural(r.removed, "photo")} removed from the gallery`
        + (r.kept ? `; ${r.kept.toLocaleString()} stay, as another folder includes them` : ""));
      await Promise.all([loadFolders(), loadMeta()]);
      renderSettings();
      render(); // the view behind Settings shows the photos that are left
      setTimeout(pollStatus, 300);
    }
  } catch (err) { renderSettings(err.message); }
});
$("#settingsDlg").addEventListener("close", () => { if (!photos.length) render(); });
