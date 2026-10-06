// The scan status: polling it, and the progress shown outside Settings.
// One of the page's modules; main.js starts the page.
import { num, t, tn } from "./i18n.js";
import { $, api, esc, loadMeta, state } from "./core.js";
import { indexedAtRender, shownItemCount } from "./items.js";
import { toast } from "./people.js";
import { render } from "./views.js";
import { viewer } from "./viewer.js";
import { excludedHtml, failureCount, loadFailures, loadFolders, openSettings, removeProgressHtml, renderSettings, scanStateHtml } from "./settings.js";

// ---- scan status -----------------------------------------------------------------
/** The last /api/status answer (or what a change on this page made of it). */
export let lastStatus = null;
export function setLastStatus(st) { lastStatus = st; }
let wasRunning = false;
// ---- indexing progress outside Settings ---------------------------------------------
let firstItemsCheck = 0; // last look for the first photos while the library is still empty
let pillHiddenUntilIdle = false, pillDoneTimer = 0, pillWasBusy = false;
/** What the library is doing, for the card and the first-index screen; null when idle or
 *  only checking for changes (that is quick and needs no attention). */
function indexWork(s) {
  if (s?.removing) {
    const name = s.removing.split(/[\\/]/).filter(Boolean).pop() || s.removing;
    return s.remove_total
      ? { title: t("Removing {done} of {total} files", { done: num(s.remove_done), total: num(s.remove_total) }), detail: t("Folder {name}", { name }), done: s.remove_done, total: s.remove_total }
      : { title: t("Removing a folder…"), detail: t("Folder {name}", { name }) };
  }
  if (!s?.running) return null;
  if (s.phase === "indexing files" && s.total > 0) {
    return { title: t("Indexing {done} of {total} files", { done: num(s.done), total: num(s.total) }),
      detail: s.faces ? tn(s.faces, "{n} face found", "{n} faces found") : t("Reading photos and videos, making thumbnails, finding faces"),
      done: s.done, total: s.total };
  }
  if (s.phase === "grouping faces" && s.group_total > 0) {
    return { title: t("Grouping faces {pct}%", { pct: Math.min(100, Math.floor((100 * s.group_done) / s.group_total)) }),
      detail: t("Placing faces into people"), done: s.group_done, total: s.group_total };
  }
  // Listing the files of a large library takes a while: say so when nothing is shown yet.
  if (s.phase === "listing files" && !shownItemCount) return { title: t("Looking for photos and videos…"), detail: t("Listing the files in your folders") };
  return null;
}
export function firstIndexHtml(s) {
  const w = indexWork(s) ?? { title: t("Getting ready…"), detail: "" };
  return `<h2>${t("Indexing your photos and videos")}</h2>
    <p>${t("Photos and videos appear here as they are indexed. You can close the app; it continues where it left off.")}</p>
    <progress ${w.total ? `max="${w.total}" value="${w.done}"` : ""}></progress>
    <div class="n">${esc(w.title)}${w.detail ? ` · ${esc(w.detail)}` : ""}</div>`;
}
export function updateIndexPill(s) {
  const pill = $("#indexPill"), w = indexWork(s);
  if (!s.running) pillHiddenUntilIdle = false;
  if (w) {
    pillWasBusy = true;
    clearTimeout(pillDoneTimer);
    pill.classList.remove("done");
    // The first-index screen already shows it big.
    pill.hidden = pillHiddenUntilIdle || !!$("#firstIndex");
    $(".t b", pill).textContent = w.title;
    $(".t small", pill).textContent = w.detail;
    const bar = $("progress", pill);
    if (w.total) { bar.max = w.total; bar.value = w.done; } else { bar.removeAttribute("value"); }
    // Photos indexed since the Photos view was drawn: offer to show them (no surprise reloads).
    const fresh = s.phase === "indexing files" ? s.done - Math.min(indexedAtRender, s.done) : 0;
    const more = $("[data-pill-new]", pill);
    more.hidden = !(fresh > 0 && shownItemCount > 0 && state.view === "photos");
    more.textContent = t("Show {n} new", { n: num(fresh) });
  } else if (pillWasBusy && !s.running && !s.removing) {
    pillWasBusy = false;
    if (pill.hidden) return;
    pill.classList.add("done");
    $(".t b", pill).textContent = t("Your library is up to date");
    $(".t small", pill).textContent = "";
    $("[data-pill-new]", pill).hidden = true;
    pillDoneTimer = setTimeout(() => { pill.hidden = true; }, 4000);
  } else if (!w && s.running) {
    pill.hidden = true;
  }
}
$("#indexPill").addEventListener("click", e => {
  const b = e.target.closest("button");
  if (!b) return;
  if (b.dataset.pillHide != null) { pillHiddenUntilIdle = true; $("#indexPill").hidden = true; }
  else if (b.dataset.pillNew != null) { b.hidden = true; render(); }
  else if (b.dataset.pillDetails != null) { openSettings(); loadFolders().then(() => { if ($("#settingsDlg").open) renderSettings(); }); }
});

let pollFailures = 0;
export async function pollStatus() {
  let s;
  try { s = await api("/api/status"); } catch {
    // Otherwise the page just stops changing when the program is closed or the network drops.
    if (++pollFailures === 3) toast(t("Can't reach Imadive. Is it still running?"), true);
    return setTimeout(pollStatus, 5000);
  }
  if (pollFailures >= 3) toast(t("Connected to Imadive again"));
  pollFailures = 0;
  const wasRemoving = lastStatus?.removing ?? null;
  lastStatus = s;
  // A folder removal (from this page or another device): its row in Settings follows it.
  const removeRow = $("#removeProgress");
  if (removeRow && s.removing) removeRow.innerHTML = removeProgressHtml(s);
  if ((s.removing ?? null) !== wasRemoving && $("#settingsDlg").open) {
    loadFolders().then(() => { if ($("#settingsDlg").open) renderSettings(); });
  }
  updateIndexPill(s);
  const first = $("#firstIndex");
  if (first) first.innerHTML = firstIndexHtml(s);
  // Indexing problems are shown in Settings.
  const settingsScan = $("#settingsScan");
  if (settingsScan) {
    settingsScan.innerHTML = scanStateHtml(s);
    $("#settingsExcluded").innerHTML = excludedHtml(s);
    if (s.failed !== failureCount) loadFailures();
  }
  if (wasRunning && !s.running) {
    await Promise.all([loadMeta(), loadFolders()]);
    if ($("#settingsDlg").open && !$("#settingsDlg").contains(document.activeElement?.closest("input"))) renderSettings(); // new photo counts
    render();
  }
  // During the first index, show photos as soon as some are in instead of waiting for the end.
  else if (s.running && s.done > 0 && !shownItemCount && state.view === "photos" && !viewer.classList.contains("open")
    && Date.now() - firstItemsCheck > 5000) {
    firstItemsCheck = Date.now();
    render();
  }
  wasRunning = s.running;
  setTimeout(pollStatus, s.running || s.removing ? 1500 : 10000);
}
