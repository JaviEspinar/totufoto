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
  // The page loaded again while the picker was handing over files.
  if (session.take()) notice = tooMany();
  main.innerHTML = `<div class="opt upload">
    <h2>${t("Upload photos and videos")}</h2>
    <p class="lead">${t("Photos and videos from this device into the gallery.")} ${where} ${t("Files that aren't photos or videos are left out.")}</p>
    <div id="uploadArea"></div>
    <input type="file" id="pickFolder" webkitdirectory multiple hidden>
    <input type="file" id="pickFiles" multiple accept="image/*,video/*" hidden>
    <input type="file" id="pickAny" multiple hidden>
  </div>`;
  showArea(targets);
}

/** The drop zone, or what is being prepared, or the upload's progress, or its result. */
function showArea(targets = null) {
  const area = $("#uploadArea");
  if (!area) return;
  if (preparing) area.innerHTML = preparingHtml();
  else if (job?.running) area.innerHTML = progressHtml();
  else if (targets && !targets.folders.length) {
    area.innerHTML = `<div class="blank">${t("Add a folder in Settings first: uploads go into one of the gallery's folders.")}</div>`;
  } else {
    area.innerHTML = (notice ? `<div class="up-notice">${notice}</div>` : "") + (job ? resultHtml() : "") + `<div class="drop" id="dropZone">
      <svg class="icon" viewBox="0 0 24 24" width="40" height="40" aria-hidden="true"><use href="#i-upload"/></svg>
      <div class="big for-pointer">${t("Drop a folder, or photos and videos, here")}</div>
      <div class="big for-touch">${t("Add photos and videos from this device")}</div>
      <div class="acts">
        <button class="btn primary" data-pick="folder">${t("Choose a folder")}</button>
        <button class="btn" data-pick="files">${t("Choose photos and videos")}</button>
        <button class="btn for-touch" data-pick="any">${t("Choose files")}</button>
      </div>
      <p class="hint for-touch">${t("Many at once: <b>Choose files</b>, open the folder (for example DCIM › Camera) and use <b>Select all</b> in its menu. <b>Choose photos and videos</b> takes up to 100.")}</p>
      <p class="hint for-touch">${t("Android can take very long, or give up, handing over thousands of files at once: choose a few hundred at a time, and add more while they upload. For a whole phone, copying its camera folder to the server is faster: with a cable, a network share or an app such as Syncthing.")}</p>
    </div>`;
  }
}

// ---- preparing: from choosing the files to the first one sent ---------------------------
// Big selections take a while (a phone hands over a folder of thousands slowly, and they
// are filtered and their names checked): the card says what is going on all along, and the
// work is done in slices so the page keeps answering.
let preparing = null; // { text, waiting } while preparing; null otherwise
function setPreparing(text, waiting = false) {
  preparing = text == null ? null : { text, waiting };
  showArea();
}
function preparingHtml() {
  if (!preparing.waiting) {
    return `<div class="opt-progress up-preparing">
      <div class="up-head"><b>${esc(preparing.text)}</b><span class="spinner small" aria-hidden="true"></span></div>
      <progress></progress>
    </div>`;
  }
  // Waiting for the picker: Android can give up on a big selection without saying so, so
  // there is always a way back, and after a while the card says what probably happened.
  return `<div class="opt-progress up-preparing">
    <div class="up-head"><b>${esc(preparing.text)}</b><button class="btn" data-pick-stop>${t("Stop waiting")}</button></div>
    <small>${preparing.slow ? esc(tooMany()) : t("A phone can take a little while to hand over a big folder.")}</small>
  </div>`;
}
/** Lets the page draw and answer between slices of work. */
const breathe = () => new Promise(resolve => setTimeout(resolve, 0));

/** What the progress card shows, worked out once per redraw. */
function progressParts() {
  const doneFiles = job.saved + job.same + job.existing + job.overwritten + job.failed.length;
  const sent = job.done + [...job.loading.values()].reduce((a, b) => a + b, 0);
  const current = [...job.current].slice(-1)[0];
  return {
    title: t("Uploading {done} of {total} files", { done: num(doneFiles), total: num(job.count) }),
    bytes: t("{sent} of {total}", { sent: fmtBytes(sent), total: fmtBytes(job.bytes) }),
    sent,
    file: current ?? "",
    note: job.waiting ? t("The connection was lost: trying again…") : "",
  };
}
function progressHtml() {
  const p = progressParts();
  return `<div class="opt-progress" id="upProgress">
    <div class="up-head"><b id="upTitle">${esc(p.title)}</b><span class="up-bytes" id="upBytes">${esc(p.bytes)}</span></div>
    <progress id="upBar" max="${job.bytes}" value="${p.sent}"></progress>
    <div class="up-foot"><small class="up-file" id="upFile">${p.file ? esc(p.file) : "&nbsp;"}</small>
      <span class="up-acts"><button class="btn" data-pick="any">${t("Add more…")}</button><button class="btn" data-upload-cancel>${t("Cancel")}</button></span></div>
    <div class="up-retrying" id="upNote" ${p.note ? "" : "hidden"}>${esc(p.note)}</div>
    <small class="hint for-touch">${t("Keep this page open and the screen on until it finishes. If the phone locks, the upload goes on when you come back.")}</small>
  </div>`;
}
/** Updates the progress card in place: rebuilding it many times a second would make its
 *  Cancel button hard to tap, and costs a phone more than it should. */
function updateProgress() {
  if (!$("#upProgress")) return void ($("#uploadArea").innerHTML = progressHtml());
  const p = progressParts();
  $("#upTitle").textContent = p.title;
  $("#upBytes").textContent = p.bytes;
  $("#upBar").value = p.sent;
  $("#upFile").textContent = p.file || "\u00a0";
  $("#upNote").textContent = p.note;
  $("#upNote").hidden = !p.note;
}

function resultHtml() {
  const lines = [];
  if (job.saved) lines.push(tn(job.saved, "{n} file uploaded", "{n} files uploaded"));
  if (job.overwritten) lines.push(tn(job.overwritten, "{n} replaced the file of the same name", "{n} replaced the files of the same name"));
  if (job.renamed) lines.push(tn(job.renamed, "{n} kept beside the file of the same name", "{n} kept beside the files of the same name"));
  if (job.existing) lines.push(tn(job.existing, "{n} skipped: a file with that name was there", "{n} skipped: files with those names were there"));
  if (job.same) lines.push(tn(job.same, "{n} was already there", "{n} were already there"));
  if (job.left) lines.push(tn(job.left, "{n} left out: not a photo or video", "{n} left out: not photos or videos"));
  if (job.failed.length) lines.push(t("{n} couldn't be uploaded", { n: num(job.failed.length) }));
  if (job.cancelled) lines.push(t("Cancelled"));
  const failures = job.failed.slice(0, 20).map(f => `<li><bdi>${esc(f.item.path)}</bdi>: ${esc(f.error)}</li>`).join("");
  const more = job.failed.length > 20 ? `<li>${t("and {n} more", { n: num(job.failed.length - 20) })}</li>` : "";
  const stored = job.saved + job.overwritten;
  return `<div class="opt-progress up-result${job.failed.length ? " has-errors" : ""}">
    <div class="up-head"><b>${lines.join(" · ") || t("Nothing to upload")}</b>
      <span class="up-acts">
        ${job.failed.length ? `<button class="btn primary" data-upload-retry>${t("Retry the ones that failed")}</button>` : ""}
        ${stored ? `<button class="btn" data-upload-show>${t("Show the photos")}</button>` : ""}
      </span></div>
    ${stored ? `<small>${t("In {folder}. They are being added to the gallery.", { folder: `<bdi>${esc(job.target)}</bdi>` })}</small>` : ""}
    ${failures ? `<ul class="up-failures">${failures}${more}</ul>` : ""}
  </div>`;
}

// ---- choosing files --------------------------------------------------------------------
$("#main").addEventListener("click", e => {
  if (state.view !== "manage") return;
  const pick = e.target.closest("[data-pick]")?.dataset.pick;
  // On phones, "files" opens the photo picker (a few at a time, at most 100) and "any" the
  // file manager, where a whole folder can be selected; files that aren't photos or videos
  // are left out afterwards.
  if (pick) {
    // A fresh field each time: after a picker that failed to hand over its files, Chrome on
    // Android can leave the old one stuck, and the button would do nothing until a reload.
    const old = $({ folder: "#pickFolder", files: "#pickFiles", any: "#pickAny" }[pick]);
    const input = old.cloneNode();
    old.replaceWith(input);
    waitForPicker(input);
    return input.click();
  }
  if (e.target.closest("[data-pick-stop]")) return stopWaiting();
  if (e.target.closest("[data-upload-cancel]")) return cancel();
  if (e.target.closest("[data-upload-retry]")) return retry();
  if (e.target.closest("[data-upload-show]")) { state.view = "photos"; return render(); }
});
/** Something to say above the drop zone (why the last choice brought no files). */
let notice = "";
/** Set while the picker is open: if the page loads again meanwhile, the browser gave up
 *  (or ran out of memory) handing over the files, and the new page can say so. */
const PICKING = "imadive.picking";
const session = {
  set(v) { try { sessionStorage.setItem(PICKING, v); } catch {} },
  take() { try { const v = sessionStorage.getItem(PICKING); sessionStorage.removeItem(PICKING); return v; } catch { return null; } },
};
const tooMany = () => t("No files arrived from the picker. With thousands of files at once, Android sometimes gives up handing them over: choose fewer at a time (a few hundred), and add more while they upload.");
/** From the tap until the files arrive (or the picker is closed without any). A phone can take
 *  minutes to hand over a big selection, so only a real cancel ends the wait. */
function waitForPicker(input) {
  notice = "";
  const since = Date.now();
  session.set(String(since));
  setPreparing(t("Choose the files…"), true);
  const done = () => {
    session.take();
    if (!preparing?.waiting) return;
    // Closed after a long wait: the selection was probably too big, not cancelled.
    if (Date.now() - since > 15_000) notice = tooMany();
    setPreparing(null);
  };
  input.addEventListener("cancel", done, { once: true });
  // Browsers without the cancel event: back on the page with nothing chosen.
  if (!("oncancel" in input)) addEventListener("focus", () => setTimeout(() => { if (!input.files?.length) done(); }, 1500), { once: true });
  // Back on the page and still nothing after a while: Android has probably given up.
  addEventListener("focus", () => setTimeout(() => {
    if (preparing?.waiting && !input.files?.length) { preparing.slow = true; showArea(); }
  }, 20_000), { once: true });
}
/** Stop waiting: back to the buttons. Files that still arrive afterwards are uploaded. */
function stopWaiting() {
  session.take();
  if (preparing?.waiting) setPreparing(null);
}
$("#main").addEventListener("change", async e => {
  if (!["pickFolder", "pickFiles", "pickAny"].includes(e.target.id)) return;
  session.take();
  const list = e.target.files;
  setPreparing(tn(list.length, "Preparing {n} file…", "Preparing {n} files…"));
  await breathe();
  // A folder's files carry their path inside it; single files just their name.
  const files = [];
  for (let i = 0; i < list.length; i++) {
    const file = list[i];
    files.push({ file, path: file.webkitRelativePath || file.name });
    if (i % 500 === 499) await breathe();
  }
  e.target.value = "";
  start(files);
});
// Dropping anywhere on the view; elsewhere the browser would open the file instead.
for (const type of ["dragenter", "dragover"]) {
  $("#main").addEventListener(type, e => {
    if (state.view !== "manage" || preparing || !e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    $("#dropZone")?.classList.add("over");
  });
}
$("#main").addEventListener("dragleave", e => {
  if (!e.relatedTarget || !$("#main").contains(e.relatedTarget)) $("#dropZone")?.classList.remove("over");
});
$("#main").addEventListener("drop", async e => {
  if (state.view !== "manage" || preparing) return;
  e.preventDefault();
  $("#dropZone")?.classList.remove("over");
  // The entries have to be taken now: the drop's data is gone after the first await.
  const entries = [...e.dataTransfer.items].map(i => i.webkitGetAsEntry?.()).filter(Boolean);
  const files = [];
  setPreparing(t("Reading what was dropped…"));
  for (const entry of entries) await collect(entry, files);
  start(files);
});

/** The files in a dropped file or folder, with their paths from the dropped item on. */
async function collect(entry, out) {
  if (entry.isFile) {
    const file = await new Promise((resolve, reject) => entry.file(resolve, reject)).catch(() => null);
    if (file) out.push({ file, path: entry.fullPath.replace(/^\/+/, "") });
    if (out.length % 200 === 0) setPreparing(tn(out.length, "Reading what was dropped: {n} file…", "Reading what was dropped: {n} files…"));
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
/** Paths asked about at once when checking which names are already there. */
const CHECK_AT_ONCE = 1000;

async function start(chosen) {
  if (!chosen.length) return setPreparing(null);
  // Chosen while an upload runs: they join it, into the same folder.
  const adding = !!job?.running;
  try {
    const targets = await api("/api/uploads");
    const extensions = new Set(targets.extensions);
    const ext = path => path.split(".").pop().toLowerCase();
    const files = [];
    for (let i = 0; i < chosen.length; i++) {
      const f = chosen[i];
      if (!f.path.split("/").some(p => p.startsWith(".")) && extensions.has(ext(f.path))) files.push(f);
      if (i % 2000 === 1999) await breathe();
    }
    let folder = adding ? job.folder : targets.folders[0];
    if (!folder) { setPreparing(null); return showArea(targets); }
    if (!adding && targets.folders.length > 1 && files.length) {
      const buttons = targets.folders.map((f, i) => `<button class="btn up-target" data-choice="${i}"><bdi>${esc(f)}</bdi></button>`).join("");
      const choice = await askChoice(`<p>${t("Which of the gallery's folders should they go to? They are saved in its <b>{subfolder}</b> folder, created if it isn't there.", { subfolder: esc(targets.subfolder) })}</p>
        <div class="up-targets">${buttons}</div>
        <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button></div>`, t("Upload to"));
      if (choice === "cancel") return setPreparing(null);
      folder = targets.folders[+choice];
    }

    // Names already in the uploads folder: ask once what to do with all of them.
    const existing = new Set();
    for (let i = 0; i < files.length; i += CHECK_AT_ONCE) {
      setPreparing(t("Checking which names are already there: {done} of {total}", { done: num(i), total: num(files.length) }));
      const r = await post("/api/uploads/existing", { folder, paths: files.slice(i, i + CHECK_AT_ONCE).map(f => f.path) });
      for (const path of r.existing) existing.add(path);
    }
    let onConflict = "keep";
    if (existing.size) {
      const names = [...existing].sort().slice(0, 5).map(p => `<li><bdi>${esc(p)}</bdi></li>`).join("");
      const more = existing.size > 5 ? `<li>${t("and {n} more", { n: num(existing.size - 5) })}</li>` : "";
      onConflict = await askChoice(`<p>${tn(existing.size, "{n} file has the same name as one already in <b>{subfolder}</b>:", "{n} files have the same names as ones already in <b>{subfolder}</b>:", { subfolder: esc(targets.subfolder) })}</p>
        <ul class="up-conflicts">${names}${more}</ul>
        <p class="up-note">${t("Files that are exactly the same as the ones there are never copied twice.")}</p>
        <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button>
          <button class="btn" data-choice="skip">${t("Skip")}</button>
          <button class="btn" data-choice="overwrite">${t("Overwrite")}</button>
          <button class="btn primary" data-choice="keep">${t("Keep both")}</button></div>`, t("Files with the same name"));
      if (onConflict === "cancel") return setPreparing(null);
    }
    // Skipped ones aren't even sent; each file goes with the choice made for it.
    const toSend = (onConflict === "skip" ? files.filter(f => !existing.has(f.path)) : files).map(f => ({ ...f, onConflict }));
    const counts = { left: chosen.length - files.length, existing: files.length - toSend.length };
    if (adding && job?.running) return add(toSend, counts);
    run(toSend, { folder, subfolder: targets.subfolder, ...counts });
  } catch (err) {
    setPreparing(null);
    alertArea(err.message);
  }
}

/** More files for the upload that is running: they join its queue. */
function add(files, { left, existing }) {
  job.queue.push(...files);
  job.count += files.length;
  job.bytes += files.reduce((a, f) => a + f.file.size, 0);
  job.left += left;
  job.existing += existing;
  preparing = null;
  showArea();
  for (let i = job.loading.size + job.waiting; i < AT_ONCE; i++) next();
}

/** Sends `files` into `folder`'s uploads folder, AT_ONCE at a time. */
function run(files, { folder, subfolder, left = 0, existing = 0 }) {
  job = {
    running: files.length > 0, count: files.length, queue: [...files], waiting: 0, target: `${folder}/${subfolder}`, folder, subfolder,
    bytes: files.reduce((a, f) => a + f.file.size, 0), done: 0, loading: new Map(), current: new Set(),
    saved: 0, same: 0, overwritten: 0, renamed: 0, existing, left, failed: [], requests: new Set(), cancelled: false,
  };
  preparing = null;
  showArea();
  for (let i = 0; i < AT_ONCE; i++) next();
}

/** The files that failed last time, again, into the same folder and with the same choice. */
function retry() {
  if (!job || job.running || !job.failed.length) return;
  const { folder, subfolder } = job;
  run(job.failed.map(f => f.item), { folder, subfolder });
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
  const query = new URLSearchParams({ folder: current.folder, path: item.path, on_conflict: item.onConflict ?? "keep" });
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
    // The connection lost (a phone locking its screen, a Wi-Fi hiccup) or the server failing
    // for a moment: this file waits and is sent again, rather than every file after it
    // failing in turn.
    if ((xhr.status === 0 || xhr.status >= 500) && tries < RETRIES) {
      current.waiting++;
      redraw();
      return retryLater(tries).then(() => {
        current.waiting--;
        if (!current.cancelled) send(item, tries + 1);
      });
    }
    let body = null;
    try { body = JSON.parse(xhr.responseText); } catch {}
    current.done += item.file.size;
    if (xhr.status === 200 || xhr.status === 201) {
      const status = body?.status;
      if (status === "same") current.same++;
      else if (status === "skipped") current.existing++;
      else if (status === "overwritten") current.overwritten++;
      else { current.saved++; if (status === "renamed") current.renamed++; }
    } else {
      current.failed.push({ item, error: body?.error ? translateMessage(body.error) : xhr.status ? `HTTP ${xhr.status}` : t("the connection was lost") });
    }
    redraw();
    next();
  };
  xhr.send(item.file);
}

/** Tries for a file whose connection failed, waiting longer each time. */
const RETRIES = 6;
/** Resolves after a pause that grows with `tries` (1 s, 2 s, 4 s... up to 30 s), and once the
 *  page is visible and the device online again: a hidden page on a phone can't send. */
async function retryLater(tries) {
  await new Promise(resolve => setTimeout(resolve, Math.min(30_000, 1000 * 2 ** tries)));
  if (document.hidden) await new Promise(resolve => document.addEventListener("visibilitychange", function seen() {
    if (!document.hidden) { document.removeEventListener("visibilitychange", seen); resolve(); }
  }));
  if (!navigator.onLine) await new Promise(resolve => addEventListener("online", resolve, { once: true }));
}

/** At most a few redraws a second: progress events come much more often. */
let drawing = 0;
function redraw() {
  if (drawing) return;
  drawing = setTimeout(() => {
    drawing = 0;
    if (state.view === "manage" && job?.running && !preparing && $("#uploadArea")) updateProgress();
  }, 250);
}

function finish() {
  job.running = false;
  // The new files join the gallery with the next scan: start it now.
  if (job.saved || job.overwritten) post("/api/scan").catch(() => {});
  if (state.view === "manage") showArea();
}

function cancel() {
  if (!job?.running) return;
  job.cancelled = true;
  job.queue = [];
  for (const xhr of job.requests) xhr.abort();
  job.running = false;
  if (job.saved || job.overwritten) post("/api/scan").catch(() => {});
  showArea();
}

function alertArea(message) {
  const area = $("#uploadArea");
  if (area) area.insertAdjacentHTML("afterbegin", `<div class="err">${esc(message)}</div>`);
}

// Leaving the page would stop the upload: the browser asks first.
addEventListener("beforeunload", e => { if (job?.running) e.preventDefault(); });
