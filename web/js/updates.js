// Settings' Updates card: this version, the latest release, and updating. One of the
// page's modules; main.js starts the page.
import { t } from "./i18n.js";
import { $, api, esc, fmtBytes, post } from "./core.js";
import { toast } from "./people.js";

let updateInfo = null, updateTimer = 0;
const updateBusy = u => ["checking", "downloading", "installing", "restarting"].includes(u?.state);
/** This version, the latest release, and the buttons to check and to update. */
function updateHtml(u) {
  if (!u) return `<h4>${t("Updates")}</h4><p>${t("Loading…")}</p>`;
  const progress = progressHtml(u);
  return `<h4>${t("Updates")}</h4><p>${versionHtml(u)}</p>
    ${progress ? `<div class="remove-progress">${progress}</div>` : ""}
    ${u.state === "error" && u.error ? `<div class="err">${esc(t("The last attempt failed: {error}", { error: u.error }))}</div>` : ""}
    ${commandHtml(u)}${buttonsHtml(u)}
    <p><small>${[u.checked ? t("Last checked on GitHub on {when}.", { when: esc(u.checked) }) : "",
      u.automatic ? t("Imadive looks for new releases once a day.") : t("Imadive only looks for new releases when you ask.")].filter(Boolean).join(" ")}</small></p>`;
}
/** This version, and whether a newer one is out. */
function versionHtml(u) {
  if (!u.available) {
    return u.latest ? t("Imadive {current} is up to date.", { current: esc(u.current) }) : t("This is Imadive {current}.", { current: esc(u.current) });
  }
  const whatsNew = ` <a href="${esc(u.releases)}/tag/v${esc(u.latest)}" target="_blank" rel="noopener">${t("What's new")}</a>`;
  return `<b class="update-available">${t("Imadive {latest} is available (you have {current}).", { latest: esc(u.latest), current: esc(u.current) })}</b>${whatsNew}`;
}
/** How a check or an update is going. */
function progressHtml(u) {
  switch (u.state) {
    case "checking": return `<div>${t("Checking for updates…")}</div><progress></progress>`;
    case "downloading": return `<div>${u.total ? t("Downloading {done} of {total}", { done: fmtBytes(u.done), total: fmtBytes(u.total) }) : t("Downloading…")}</div>
      <progress ${u.total ? `max="${u.total}" value="${u.done}"` : ""}></progress>`;
    case "installing": return `<div>${t("Installing… The gallery restarts when it's done.")}</div><progress></progress>`;
    case "restarting": return `<div>${t("Restarting…")}</div><progress></progress>`;
    default: return "";
  }
}
/** For a copy that can't update itself: what to run, and where (never in this browser). */
function commandHtml(u) {
  if (!u.available || u.can_install) return "";
  const lead = {
    source: t("This server was built from source, so it can't update itself. Run this once on the server, in a terminal or over SSH, to switch to the ready-made version: it keeps your index, your people and your folders, and from then on updates install from here with one click."),
    setup_without_helper: t("Run this once on the server, in a terminal or over SSH. It also adds the update service, so the next updates install from here."),
    docker: t("Run this on the computer that runs the container, then start the container again with the new image."),
  }[u.kind];
  if (!u.command || !lead) return `<p>${t("This copy can't update itself: get the new version from the releases page.")}</p>`;
  return `<p>${lead}</p><div class="update-cmd"><code>${esc(u.command)}</code><button class="btn" data-copy-command>${t("Copy")}</button></div>`;
}
function buttonsHtml(u) {
  const disabled = updateBusy(u) ? "disabled" : "";
  return `<div class="add-row">${u.available && u.can_install ? `<button class="btn primary" data-update-install ${disabled}>${t("Update")}</button>` : ""}
    <button class="btn" data-update-check ${disabled}>${t("Check for updates")}</button></div>`;
}
/** Copies `text`. The clipboard API only exists on HTTPS pages (and localhost), and the gallery
 *  on a home network is plain http: there, a hidden text field is copied the old way. */
function copyText(text) {
  if (window.isSecureContext && navigator.clipboard) {
    navigator.clipboard.writeText(text).catch(() => {});
    return true;
  }
  const field = document.createElement("textarea");
  field.value = text;
  field.setAttribute("readonly", "");
  field.style.cssText = "position: fixed; top: 0; left: 0; opacity: 0;";
  // Inside the open dialog, which takes the focus away from anything outside it.
  ($("#settingsDlg").open ? $("#settingsDlg") : document.body).append(field);
  field.select();
  let copied = false;
  try { copied = document.execCommand("copy"); } catch {}
  field.remove();
  return copied;
}
/** Fetches the update state; while something is going on, again every second. */
export async function loadUpdate(url = "/api/update", method = "GET") {
  clearTimeout(updateTimer);
  const before = updateInfo;
  const installing = ["installing", "restarting"].includes(before?.state);
  try {
    updateInfo = await api(url, { method });
    // Back after an update: the page is the old version's, so it starts again.
    if (installing && before.current && updateInfo.current !== before.current) return location.reload();
  } catch (err) {
    // While an update installs, the server going away is it restarting with the new
    // version, not a failure: wait for it to come back.
    updateInfo = installing
      ? { ...before, state: "restarting" }
      : { ...before, state: "error", error: err.message };
  }
  const el = $("#settingsUpdate");
  if (el) el.innerHTML = updateHtml(updateInfo);
  if (updateBusy(updateInfo) && $("#settingsDlg").open) {
    updateTimer = setTimeout(loadUpdate, updateInfo.state === "restarting" ? 2000 : 1000);
  }
}

/** The card, as Settings draws it. */
export const updateCardHtml = () => `<section class="set-part" id="settingsUpdate">${updateHtml(updateInfo)}</section>`;

/** The card's buttons, by their data attribute (see Settings' click handler). */
export const updateActions = {
  updateCheck: () => loadUpdate("/api/update/check", "POST"),
  updateInstall: button => {
    button.disabled = true;
    return post("/api/update/install").then(() => loadUpdate()).catch(err => {
      toast(t("Couldn't update: {error}", { error: err.message }), true);
      loadUpdate();
    });
  },
  copyCommand: () => {
    if (copyText(updateInfo?.command ?? "")) return toast(t("Copied"));
    // Not even that: the command is selected, ready to copy with the keyboard.
    const code = $("#settingsUpdate .update-cmd code");
    if (code) getSelection().selectAllChildren(code);
    toast(t("Couldn't copy it; select it and copy it yourself"), true);
  },
};
