// The Optimization view: identical files and deleting the copies.
// One of the page's modules; main.js starts the page.
import { $, api, esc, loadMeta, plural, post, state } from "./core.js";
import { toast } from "./people.js";
import { render } from "./views.js";
import { askChoice } from "./viewer.js";

// ---- optimization: identical files ------------------------------------------------------
const fmtBytes = n => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB`
  : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;
const fmtFileDate = secs => new Date(secs * 1000).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
const DUP_BATCH = 100;
export async function renderOptimization(main, job) {
  const report = await api("/api/duplicates", { signal: job.signal });
  if (!job.alive()) return;
  const intro = `<h2>Duplicate photos</h2><p class="lead">Identical files (the very same bytes) take space twice. Of each set, the copy with the oldest file date is kept. Nothing is deleted until you click <b>Delete duplicates</b>.</p>`;
  if (report.deleting) {
    // Deleting (started here or on another device): progress until it ends.
    main.innerHTML = `<div class="opt">${intro}<div class="opt-progress" id="dupDeleting">${deleteProgressHtml(null)}</div></div>`;
    const timer = setInterval(async () => {
      if (!job.alive() || state.view !== "optimization") return clearInterval(timer);
      const p = await api("/api/duplicates/progress").catch(() => null);
      if (!p || !job.alive()) return;
      if (!p.deleting) { clearInterval(timer); loadMeta().catch(() => {}); return render(); }
      const box = $("#dupDeleting");
      if (box) box.innerHTML = deleteProgressHtml(p);
    }, 500);
    return;
  }
  if (report.scanning || report.running) {
    // While the scan or the search runs: its progress every second (the report itself only
    // once it's done, since it can be large).
    const progress = p => {
      const what = p.scanning ? "Waiting for the photo scan to finish…"
        : `Checking ${p.checked.toLocaleString()} of ${p.to_check.toLocaleString()} files that could be duplicates`;
      return `<div>${what}</div><progress ${p.scanning || !p.to_check ? "" : `max="${p.to_check}" value="${p.checked}"`}></progress>`;
    };
    main.innerHTML = `<div class="opt">${intro}<div class="opt-progress" id="dupSearch">${progress({ scanning: report.scanning, checked: report.done, to_check: report.total })}</div></div>`;
    // A failed request (the server busy for a moment) is tried again, not left on this screen.
    const again = async () => {
      if (!job.alive() || state.view !== "optimization") return;
      const p = await api("/api/duplicates/progress", { signal: job.signal }).catch(() => null);
      if (!job.alive()) return;
      if (p && !p.scanning && !p.searching) return renderOptimization(main, job).catch(() => setTimeout(again, 3000));
      const box = $("#dupSearch");
      if (p && box) box.innerHTML = progress(p);
      setTimeout(again, p ? 1000 : 3000);
    };
    setTimeout(again, 1000);
    return;
  }
  const checked = report.finished ? `Last checked ${esc(report.finished)}.` : "Not checked yet.";
  const summary = report.files
    ? `<div class="what"><div class="big">${fmtBytes(report.bytes)} can be freed</div>
        <small>${plural(report.files, "duplicate file")} in ${plural(report.groups.length, "set")}. ${checked}</small></div>
       <div class="acts"><button class="btn" data-dup-search>Search again</button>
         <button class="btn danger" data-dup-delete>Delete duplicates</button></div>`
    : `<div class="what"><div class="big">No duplicates</div><small>No two photos are identical files. ${checked}</small></div>
       <div class="acts"><button class="btn" data-dup-search>Search again</button></div>`;
  main.innerHTML = `<div class="opt">${intro}<div class="opt-summary">${summary}</div><div id="dupGroups"></div></div>`;
  const list = $("#dupGroups");
  const file = (f, kind) => `<div class="dup-file ${kind}">${kind === "keep"
    ? `<span class="tag" title="The oldest copy: it stays where it is">Keep</span>`
    : `<span class="tag" title="Moved to the bin only when you click Delete duplicates">To delete</span>`}
      <span class="p" title="${esc(f.path)}"><bdi>${esc(f.path)}</bdi></span><span class="when">${fmtFileDate(f.mtime)}</span></div>`;
  let shown = 0;
  const more = () => {
    list.querySelector(".dup-more")?.remove();
    list.insertAdjacentHTML("beforeend", report.groups.slice(shown, shown + DUP_BATCH).map(g => `
      <div class="dup-group"><img src="/thumb/${g.keep.id}" loading="lazy" alt="">
        <div class="files">${file(g.keep, "keep")}${g.remove.map(f => file(f, "remove")).join("")}</div></div>`).join(""));
    shown += DUP_BATCH;
    if (shown < report.groups.length)
      list.insertAdjacentHTML("beforeend", `<button class="btn dup-more">Show ${Math.min(DUP_BATCH, report.groups.length - shown)} more</button>`);
  };
  more();
  list.addEventListener("click", e => { if (e.target.classList.contains("dup-more")) more(); });
  main.querySelector("[data-dup-search]")?.addEventListener("click", async e => {
    e.target.disabled = true;
    try { await post("/api/duplicates/search"); }
    catch (err) {
      e.target.disabled = false;
      return toast(`Couldn't start the search: ${err.message}`, true);
    }
    setTimeout(render, 300);
  });
  main.querySelector("[data-dup-delete]")?.addEventListener("click", () => deleteDuplicates(report));
}
/** Progress of a deletion: files done of total and space freed so far (null: starting). */
function deleteProgressHtml(p) {
  const what = !p || !p.total ? "Deleting the duplicates…"
    : p.done < p.total ? `Deleting ${p.done.toLocaleString()} of ${p.total.toLocaleString()} duplicate files · ${fmtBytes(p.freed)} freed`
    : `Updating the gallery · ${fmtBytes(p.freed)} freed`;
  return `<div>${what}</div><progress ${p?.total ? `max="${p.total}" value="${p.done}"` : ""}></progress>`;
}
async function deleteDuplicates(report) {
  const choice = await askChoice(`<p>Move <b>${plural(report.files, "duplicate file")}</b> (${fmtBytes(report.bytes)}) to the bin of the computer running Imadive?</p>
    <p>Of each set of identical files, the one with the oldest file date is kept. A file that changed since the search is left alone.</p>
    <div class="dlg-actions"><button class="btn" data-choice="cancel">Cancel</button><button class="btn danger" data-choice="bin">Move to the bin</button></div>`,
    "Delete duplicates?");
  if (choice !== "bin") return;
  const run = body => post("/api/duplicates/delete", body);
  // The summary becomes a progress bar while the server works through the files.
  const summary = $(".opt-summary");
  const box = document.createElement("div");
  box.className = "opt-progress";
  box.id = "dupDeleting";
  box.innerHTML = deleteProgressHtml(null);
  summary?.replaceWith(box);
  const runWithProgress = async body => {
    let polling = true;
    box.innerHTML = deleteProgressHtml(null);
    (async () => {
      while (polling) {
        await new Promise(r => setTimeout(r, 400));
        const p = await api("/api/duplicates/progress").catch(() => null);
        if (polling && p?.deleting) box.innerHTML = deleteProgressHtml(p);
      }
    })();
    try { return await run(body); } finally { polling = false; }
  };
  try {
    let result = await runWithProgress({});
    let freed = result.freed, binned = result.binned, deleted = result.deleted;
    if (result.no_bin.length) {
      const again = await askChoice(`<p>${plural(result.no_bin.length, "file")} can't be moved to a bin on ${result.no_bin.length === 1 ? "its drive" : "their drive"}, so they can't be recovered once deleted. Their oldest copy is kept either way.</p>
        <div class="dlg-actions"><button class="btn" data-choice="cancel">Keep them</button><button class="btn danger" data-choice="permanently">Delete permanently</button></div>`,
        "Delete them permanently?");
      if (again === "permanently") {
        const r2 = await runWithProgress({ ids: result.no_bin, permanently: true });
        freed += r2.freed; deleted += r2.deleted;
        result.skipped.push(...r2.skipped);
      }
    }
    const parts = [binned && `${binned.toLocaleString()} moved to the bin`, deleted && `${deleted.toLocaleString()} deleted`].filter(Boolean).join(", ");
    toast(`${fmtBytes(freed)} freed${parts ? `: ${parts}` : ""}${result.skipped.length ? `. ${result.skipped.length} left alone (changed since the search)` : ""}`);
  } catch (err) {
    toast(`Couldn't delete the duplicates: ${err.message}`, true);
  }
  loadMeta().catch(() => {});
  render();
}
