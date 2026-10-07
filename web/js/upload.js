// The Upload view (the gallery on a server only): photos and videos from this device into
// imaDive-uploads in one of the gallery's folders, keeping the folders they came in; a scan
// indexes them afterwards. An upload goes on while other tabs are open; coming back shows
// how it goes.
// One of the page's modules; main.js starts the page.
import { num, t, tn, translateMessage } from "./i18n.js";
import { $, api, esc, fmtBytes, post, state } from "./core.js";
import { render } from "./views.js";
import { askChoice } from "./viewer.js";

/** Files sent at the same time. */
const AT_ONCE = 3;
/** The upload in progress or the last one: null before the first. */
let job = null;

export async function renderUpload(main, renderJob) {
  const targets = await api("/api/uploads", { signal: renderJob.signal });
  if (!renderJob.alive()) return;
  const where = targets.folders.length === 1
    ? t("They are saved in <b>{subfolder}</b> in {folder}, in the folders they came in.", { subfolder: esc(targets.subfolder), folder: `<bdi>${esc(targets.folders[0])}</bdi>` })
    : t("They are saved in a folder called <b>{subfolder}</b>, in the gallery folder you choose, in the folders they came in.", { subfolder: esc(targets.subfolder) });
  main.innerHTML = `<div class="opt upload">
    <h2>${t("Upload photos and videos")}</h2>
    <p class="lead">${t("Photos and videos from this device into the gallery.")} ${where} ${t("Files that aren't photos or videos are left out.")}</p>
    <div id="uploadArea"></div>
    <input type="file" id="pickFolder" webkitdirectory multiple hidden>
    <input type="file" id="pickFiles" multiple accept="image/*,video/*" hidden>
  </div>`;
  showArea(targets);
}

/** The drop zone, or the upload's progress, or its result. */
function showArea(targets = null) {
  const area = $("#uploadArea");
  if (!area) return;
  if (job?.running) area.innerHTML = progressHtml();
  else if (targets && !targets.folders.length) {
    area.innerHTML = `<div class="blank">${t("Add a folder in Settings first: uploads go into one of the gallery's folders.")}</div>`;
  } else {
    area.innerHTML = (job ? resultHtml() : "") + `<div class="drop" id="dropZone">
      <svg class="icon" viewBox="0 0 24 24" width="40" height="40" aria-hidden="true"><use href="#i-upload"/></svg>
      <div class="big for-pointer">${t("Drop a folder, or photos and videos, here")}</div>
      <div class="big for-touch">${t("Add photos and videos from this device")}</div>
      <div class="acts">
        <button class="btn primary" data-pick="folder">${t("Choose a folder")}</button>
        <button class="btn" data-pick="files">${t("Choose photos and videos")}</button>
      </div>
    </div>`;
  }
}

function progressHtml() {
  const doneFiles = job.saved + job.same + job.failed.length;
  const sent = job.done + [...job.loading.values()].reduce((a, b) => a + b, 0);
  const current = [...job.current].slice(-1)[0];
  return `<div class="opt-progress">
    <div class="up-head"><b>${t("Uploading {done} of {total} files", { done: num(doneFiles), total: num(job.files.length) })}</b>
      <span class="up-bytes">${t("{sent} of {total}", { sent: fmtBytes(sent), total: fmtBytes(job.bytes) })}</span></div>
    <progress max="${job.bytes}" value="${sent}"></progress>
    <div class="up-foot"><small class="up-file">${current ? esc(current) : "&nbsp;"}</small>
      <button class="btn" data-upload-cancel>${t("Cancel")}</button></div>
  </div>`;
}

function resultHtml() {
  const lines = [];
  if (job.saved) lines.push(tn(job.saved, "{n} file uploaded", "{n} files uploaded"));
  if (job.same) lines.push(tn(job.same, "{n} was already there", "{n} were already there"));
  if (job.skipped) lines.push(tn(job.skipped, "{n} left out: not a photo or video", "{n} left out: not photos or videos"));
  if (job.failed.length) lines.push(t("{n} couldn't be uploaded", { n: num(job.failed.length) }));
  if (job.cancelled) lines.push(t("Cancelled"));
  const failures = job.failed.slice(0, 20).map(f => `<li><bdi>${esc(f.path)}</bdi>: ${esc(f.error)}</li>`).join("");
  const more = job.failed.length > 20 ? `<li>${t("and {n} more", { n: num(job.failed.length - 20) })}</li>` : "";
  return `<div class="opt-progress up-result${job.failed.length ? " has-errors" : ""}">
    <div class="up-head"><b>${lines.join(" · ") || t("Nothing to upload")}</b>
      ${job.saved ? `<button class="btn" data-upload-show>${t("Show the photos")}</button>` : ""}</div>
    ${job.saved ? `<small>${job.target ? t("In {folder}. They are being added to the gallery.", { folder: `<bdi>${esc(job.target)}</bdi>` }) : ""}</small>` : ""}
    ${failures ? `<ul class="up-failures">${failures}${more}</ul>` : ""}
  </div>`;
}

// ---- choosing files --------------------------------------------------------------------
$("#main").addEventListener("click", e => {
  if (state.view !== "manage") return;
  const pick = e.target.closest("[data-pick]")?.dataset.pick;
  if (pick) return $(pick === "folder" ? "#pickFolder" : "#pickFiles").click();
  if (e.target.closest("[data-upload-cancel]")) return cancel();
  if (e.target.closest("[data-upload-show]")) { state.view = "photos"; return render(); }
});
$("#main").addEventListener("change", e => {
  if (e.target.id !== "pickFolder" && e.target.id !== "pickFiles") return;
  // A folder's files carry their path inside it; single files just their name.
  const files = [...e.target.files].map(file => ({ file, path: file.webkitRelativePath || file.name }));
  e.target.value = "";
  start(files);
});
// Dropping anywhere on the view; elsewhere the browser would open the file instead.
for (const type of ["dragenter", "dragover"]) {
  $("#main").addEventListener(type, e => {
    if (state.view !== "manage" || job?.running || !e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    $("#dropZone")?.classList.add("over");
  });
}
$("#main").addEventListener("dragleave", e => {
  if (!e.relatedTarget || !$("#main").contains(e.relatedTarget)) $("#dropZone")?.classList.remove("over");
});
$("#main").addEventListener("drop", async e => {
  if (state.view !== "manage" || job?.running) return;
  e.preventDefault();
  $("#dropZone")?.classList.remove("over");
  // The entries have to be taken now: the drop's data is gone after the first await.
  const entries = [...e.dataTransfer.items].map(i => i.webkitGetAsEntry?.()).filter(Boolean);
  const files = [];
  for (const entry of entries) await collect(entry, files);
  start(files);
});

/** The files in a dropped file or folder, with their paths from the dropped item on. */
async function collect(entry, out) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject)).catch(() => null);
    if (file) out.push({ file, path: entry.fullPath.replace(/^\/+/, "") });
    return;
  }
  const reader = entry.createReader();
  // A folder is read in batches until one comes back empty.
  for (;;) {
    const batch = await new Promise((resolve, reject) => reader.readEntries(resolve, reject)).catch(() => []);
    if (!batch.length) break;
    for (const child of batch) await collect(child, out);
  }
}

// ---- uploading -------------------------------------------------------------------------
async function start(chosen) {
  if (job?.running || !chosen.length) return;
  const targets = await api("/api/uploads").catch(err => { alertArea(err.message); return null; });
  if (!targets) return;
  const extensions = new Set(targets.extensions);
  const ext = path => path.split(".").pop().toLowerCase();
  const files = chosen.filter(f => !f.path.split("/").some(p => p.startsWith(".")) && extensions.has(ext(f.path)));
  let target = targets.folders[0];
  if (!target) return showArea(targets);
  if (targets.folders.length > 1 && files.length) {
    const buttons = targets.folders.map((f, i) => `<button class="btn up-target" data-choice="${i}"><bdi>${esc(f)}</bdi></button>`).join("");
    const choice = await askChoice(`<p>${t("Which of the gallery's folders should they go to? They are saved in its <b>{subfolder}</b> folder, created if it isn't there.", { subfolder: esc(targets.subfolder) })}</p>
      <div class="up-targets">${buttons}</div>
      <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button></div>`, t("Upload to"));
    if (choice === "cancel") return;
    target = targets.folders[+choice];
  }
  job = {
    running: files.length > 0, files, queue: [...files], target: `${target}/${targets.subfolder}`, folder: target,
    bytes: files.reduce((a, f) => a + f.file.size, 0), done: 0, loading: new Map(), current: new Set(),
    saved: 0, same: 0, skipped: chosen.length - files.length, failed: [], requests: new Set(), cancelled: false,
  };
  if (!job.running) return showArea(targets);
  showArea();
  for (let i = 0; i < AT_ONCE; i++) next();
}

function next() {
  const item = job.queue.shift();
  if (!item) {
    if (!job.loading.size && job.running) finish();
    return;
  }
  send(item, 0);
}

/** Sends one file with XMLHttpRequest (fetch can't tell how much was sent). */
function send(item, tries) {
  const current = job;
  const xhr = new XMLHttpRequest();
  const query = new URLSearchParams({ folder: current.folder, path: item.path });
  xhr.open("PUT", `/api/uploads?${query}`);
  current.requests.add(xhr);
  current.current.add(item.path);
  current.loading.set(item, 0);
  xhr.upload.onprogress = e => { current.loading.set(item, e.loaded); redraw(); };
  xhr.onloadend = () => {
    current.requests.delete(xhr);
    current.current.delete(item.path);
    current.loading.delete(item);
    if (current.cancelled) return;
    // The connection or the server failing for a moment is worth one more try.
    if ((xhr.status === 0 || xhr.status >= 500) && tries < 1) return send(item, tries + 1);
    let body = null;
    try { body = JSON.parse(xhr.responseText); } catch {}
    if (xhr.status === 200 || xhr.status === 201) {
      current.done += item.file.size;
      if (body?.status === "same") current.same++; else current.saved++;
    } else {
      current.done += item.file.size;
      current.failed.push({ path: item.path, error: body?.error ? translateMessage(body.error) : xhr.status ? `HTTP ${xhr.status}` : t("the connection was lost") });
    }
    redraw();
    next();
  };
  xhr.send(item.file);
}

let drawing = 0;
function redraw() {
  if (drawing) return;
  drawing = requestAnimationFrame(() => {
    drawing = 0;
    if (state.view === "manage" && job?.running && $("#uploadArea")) $("#uploadArea").innerHTML = progressHtml();
  });
}

function finish() {
  job.running = false;
  // The new files join the gallery with the next scan: start it now.
  if (job.saved) post("/api/scan").catch(() => {});
  if (state.view === "manage") showArea();
}

function cancel() {
  if (!job?.running) return;
  job.cancelled = true;
  job.queue = [];
  for (const xhr of job.requests) xhr.abort();
  job.running = false;
  if (job.saved) post("/api/scan").catch(() => {});
  showArea();
}

function alertArea(message) {
  const area = $("#uploadArea");
  if (area) area.insertAdjacentHTML("afterbegin", `<div class="err">${esc(message)}</div>`);
}

// Leaving the page would stop the upload: the browser asks first.
addEventListener("beforeunload", e => { if (job?.running) e.preventDefault(); });
