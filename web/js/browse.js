// The folder browser: folders (and backups) on the computer running Imadive, for a browser,
// which has no picker for those. Settings uses it to add and move folders, and to save and
// restore backups. One of the page's modules; main.js starts the page.
import { t } from "./i18n.js";
import { $, api, esc, icon } from "./core.js";

/** Picks a folder on the server by browsing its folders. Resolves to true once `choose` (an
 *  async function of the path) accepted one; `title`, `intro` and `button` say what for (the
 *  dialog's own texts are for adding a folder). With `files` (an extension), the files of each
 *  folder are listed too, and clicking one gives it to `choose` instead of the folder (no
 *  button then). `extra` is HTML under the list, whose `[data-extra]` buttons call `onExtra`
 *  with the dialog closed. */
export function browseForFolder({ choose, title, intro, button, files, extra, onExtra } = {}) {
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
        // Reading a big backup takes a moment: said on the row at once, and nothing else can be
        // clicked meanwhile.
        if (list.classList.contains("busy")) return;
        err.textContent = "";
        list.classList.add("busy");
        target.classList.add("reading");
        target.insertAdjacentHTML("beforeend", `<small class="reading-note"><span class="spinner small" aria-hidden="true"></span>${t("Reading the backup…")}</small>`);
        try { await choose(target.dataset.file); added = true; dlg.close(); }
        catch (e2) { err.textContent = e2.message; }
        finally {
          list.classList.remove("busy");
          target.classList.remove("reading");
          target.querySelector(".reading-note")?.remove();
        }
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
