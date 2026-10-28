// Settings' Backup part (in the Library card): making a backup, and restoring one. One of
// the page's modules; main.js starts the page.
import { t, tn } from "./i18n.js";
import { $, api, esc, post } from "./core.js";
import { browseForFolder } from "./browse.js";
import { toast } from "./people.js";
import { folderInfo } from "./settings.js";
import { askChoice } from "./viewer.js";

/** Backing up the index, and restoring a backup. */
export function backupHtml() {
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
export async function watchBackup() {
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
export async function restoreFrom(file) {
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

/** Its buttons, by their data attribute (see Settings' click handler). */
export const backupActions = {
  backup: () => backUp().catch(err => toast(t("Couldn't make the backup: {error}", { error: err.message }), true)),
  restore: () => restore().catch(err => toast(t("Couldn't restore the backup: {error}", { error: err.message }), true)),
};
