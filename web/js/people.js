// The People view, the merge dialog, and changes to people (shown at once, saved after).
// One of the page's modules; main.js starts the page.
import { lang, num, t } from "./i18n.js";
import { $, esc, loadMeta, onPeopleLoaded, patch, people, personById, personName, plural, post, pref, setPeople, state } from "./core.js";
import { matchPeople, peopleOrder, peopleSort, renderPeopleList, resetPeopleOrder, restorePeopleOrder, setPeopleSort } from "./sidebar.js";
import { render } from "./views.js";
import { openViewer, viewerIndex } from "./viewer.js";

// ---- people grid ---------------------------------------------------------------------
// Libraries can have tens of thousands of people, so only the cards on screen exist in the
// page. Everything else (opening dialogs, merging, searching) stays cheap because of that.
const CARD_MIN_W = 150, GRID_GAP = 14, OVERSCAN_ROWS = 2;
let peopleGrid = null, peopleQuery = "";
// The People tab opened before there was anyone: show them once there are.
onPeopleLoaded(() => {
  if (state.view === "people" && !peopleGrid && people.length && $("#peopleHost .blank, #peopleHost .skeleton")) render();
});
export function unmountPeopleGrid() {
  peopleGrid?.destroy();
  peopleGrid = null;
}

// Faces fade in the first time they load; after that they show at once.
const loadedFaces = new Set();
function faceLoaded(img) { loadedFaces.add(img.dataset.face); img.classList.add("ok"); }
// Loaded or failed, a face picture stops shimmering. (Load events don't bubble, so this
// listens on the way down.)
for (const type of ["load", "error"]) {
  document.addEventListener(type, e => { if (e.target.matches?.("img[data-face]")) faceLoaded(e.target); }, true);
}
const faceImg = id => loadedFaces.has(String(id))
  ? `<img src="/face/${id}" alt="" data-face="${id}" class="ok">`
  : `<img src="/face/${id}" alt="" data-face="${id}" decoding="async">`;
const faceCard = p => `
  <div class="card face-card ${p.hidden ? "hidden-person" : ""}" data-person="${p.id}">
    <div class="avatar" title="${t("Show photos")}" tabindex="0" role="button" aria-label="${t("Show {name}'s photos", { name: esc(personName(p)) })}">${faceImg(p.face)}</div>
    <input value="${esc(p.name || "")}" placeholder="${t("Add a name")}" data-rename="${p.id}">
    <div class="s">${plural(p.count, "photo")}</div>
    <div class="row">
      <button class="btn" data-merge="${p.id}" title="${t("Merge with another person")}">${t("Same as…")}</button>
      <button class="btn" data-hide="${p.id}">${p.hidden ? t("Show") : t("Hide")}</button>
    </div>
  </div>`;
const skeletonCard = () => `
  <div class="card face-card skeleton" aria-hidden="true">
    <div class="avatar shimmer"></div><div class="line shimmer"></div><div class="line short shimmer"></div>
  </div>`;
const faceRow = p => `
  <div class="face-row ${p.hidden ? "hidden-person" : ""}" data-person="${p.id}">
    <div class="avatar" title="${t("Show photos")}" tabindex="0" role="button" aria-label="${t("Show {name}'s photos", { name: esc(personName(p)) })}">${faceImg(p.face)}</div>
    <input value="${esc(p.name || "")}" placeholder="${t("Add a name")}" data-rename="${p.id}">
    <span class="s">${plural(p.count, "photo")}</span>
    <button class="btn" data-merge="${p.id}" title="${t("Merge with another person")}">${t("Same as…")}</button>
    <button class="btn" data-hide="${p.id}">${p.hidden ? t("Show") : t("Hide")}</button>
  </div>`;
const skeletonRow = () => `
  <div class="face-row skeleton" aria-hidden="true"><div class="avatar shimmer"></div><div class="line shimmer"></div></div>`;
const nextFrame = () => new Promise(r => requestAnimationFrame(() => setTimeout(r, 0)));

// Cards show big faces; the list fits several times more people on screen.
// The size slider scales both layouts. Cards stop at 1.6x: face pictures are 128 px, so
// bigger avatars would look blurry.
const PEOPLE_ZOOM = { cards: { min: 0.8, max: 1.6, step: 0.1 }, list: { min: 1, max: 3, step: 0.25 } };
const peopleZoom = { cards: 1, list: 1 };
let peopleLayout = "cards";
if (pref.get("peopleLayout") === "list") peopleLayout = "list";
try {
  const saved = JSON.parse(pref.get("peopleZoom") || "{}");
  for (const [k, r] of Object.entries(PEOPLE_ZOOM))
    if (typeof saved[k] === "number") peopleZoom[k] = Math.min(r.max, Math.max(r.min, saved[k]));
} catch {} // a corrupted saved value: the default sizes
/** Grid geometry for a layout at its current size. The one place these sizes are set:
 *  `applyPeopleSize` hands them to the CSS, which draws cards and rows with them. */
function peopleLayoutFor(name) {
  const z = peopleZoom[name];
  if (name === "list") {
    // Rows get taller faster than wider, so bigger sizes still show several columns.
    return { minW: Math.round(250 + 100 * (z - 1)), h: Math.round(44 * z), gap: 6, item: faceRow,
      skeleton: () => `<div class="people-list">${skeletonRow().repeat(24)}</div>` };
  }
  const face = Math.round(110 * z); // the round picture; name, count and buttons below it
  return { minW: Math.round(CARD_MIN_W * z), h: face + 112, face, gap: GRID_GAP, item: faceCard,
    skeleton: () => `<div class="cards people-grid">${skeletonCard().repeat(18)}</div>` };
}
/** Sizes of cards and rows for the CSS (`--card-h`, `--card-face`, `--row-h`), and the size
 *  slider's value (`--z`), which scales the text and the small pictures. */
function applyPeopleSize(host) {
  const cards = peopleLayoutFor("cards"), list = peopleLayoutFor("list");
  host.style.setProperty("--z", peopleZoom[peopleLayout]);
  host.style.setProperty("--card-h", `${cards.h}px`);
  host.style.setProperty("--card-face", `${cards.face}px`);
  host.style.setProperty("--row-h", `${list.h}px`);
}

function mountPeopleGrid(host, { fadeIn = false, layout: L = peopleLayoutFor("cards") } = {}) {
  const main = $("#main");
  host.innerHTML = `<div class="vgrid${fadeIn ? " fade-in" : ""}"></div>`;
  const outer = host.firstElementChild;
  const rowH = L.h + L.gap;
  const overscan = Math.max(OVERSCAN_ROWS, Math.ceil(400 / rowH)); // about 400 px above and below
  const cards = new Map(); // person id -> .vcard element
  const grid = { items: [], cols: 1, cardW: L.minW };
  let animateTimer = 0;
  const pos = i => `translate(${(i % grid.cols) * (grid.cardW + L.gap)}px, ${Math.floor(i / grid.cols) * rowH}px)`;

  function layout() {
    const w = outer.clientWidth;
    grid.cols = Math.max(1, Math.floor((w + L.gap) / (L.minW + L.gap)));
    grid.cardW = (w - L.gap * (grid.cols - 1)) / grid.cols;
    outer.style.height = `${Math.max(0, Math.ceil(grid.items.length / grid.cols) * rowH - L.gap)}px`;
  }
  /** Updates a card's contents in place (no rebuild, so pictures and typing are kept). */
  function update(el, p) {
    const card = el.firstElementChild;
    card.classList.toggle("hidden-person", !!p.hidden);
    const input = card.querySelector("input");
    if (document.activeElement !== input && input.value !== (p.name || "")) input.value = p.name || "";
    card.querySelector(".s").textContent = `${plural(p.count, "photo")}`;
    card.querySelector("[data-hide]").textContent = p.hidden ? t("Show") : t("Hide");
    if (card.querySelector("img").dataset.face !== String(p.face)) card.querySelector(".avatar").innerHTML = faceImg(p.face);
  }
  // `animate`: true slides cards to their new places; false (a re-sort, a search) moves them
  // at once, cutting short a slide still running; unset (scrolling) leaves things as they are.
  function draw({ animate } = {}) {
    if (animate) {
      outer.classList.add("animate");
      clearTimeout(animateTimer);
      animateTimer = setTimeout(() => outer.classList.remove("animate"), 350);
    } else if (animate === false) {
      clearTimeout(animateTimer);
      outer.classList.remove("animate");
    }
    const top = outer.getBoundingClientRect().top - main.getBoundingClientRect().top;
    const firstRow = Math.max(0, Math.floor(-top / rowH) - overscan);
    const lastRow = Math.ceil((main.clientHeight - top) / rowH) + overscan;
    const first = firstRow * grid.cols, last = Math.min(grid.items.length, lastRow * grid.cols);
    const keep = new Set();
    for (let i = first; i < last; i++) {
      const p = grid.items[i];
      keep.add(p.id);
      let el = cards.get(p.id);
      if (!el) {
        el = document.createElement("div");
        el.className = animate ? "vcard enter" : "vcard";
        el.innerHTML = L.item(p);
        el.style.transform = pos(i);
        el.style.width = `${grid.cardW}px`;
        outer.append(el);
        cards.set(p.id, el);
        continue;
      }
      update(el, p);
      const at = pos(i);
      if (el.style.transform !== at) el.style.transform = at;
      el.style.width = `${grid.cardW}px`;
    }
    const alive = new Set(grid.items.map(p => p.id));
    for (const [id, el] of cards) {
      if (keep.has(id)) continue;
      // Keep a card someone is typing in while it's still a person, even off screen.
      if (el.contains(document.activeElement) && alive.has(id)) continue;
      cards.delete(id);
      if (animate && !alive.has(id)) { el.classList.add("leave"); setTimeout(() => el.remove(), 220); }
      else el.remove();
    }
  }
  let raf = 0;
  const onScroll = () => { if (!raf) raf = requestAnimationFrame(() => { raf = 0; draw(); }); };
  main.addEventListener("scroll", onScroll, { passive: true });
  const resize = new ResizeObserver(() => { layout(); draw(); });
  resize.observe(outer);
  grid.setItems = (next, opts) => { grid.items = next; layout(); draw(opts); };
  /** Index of the first person on screen, to come back to after a size or layout change. */
  grid.firstVisible = () => {
    const top = outer.getBoundingClientRect().top - main.getBoundingClientRect().top;
    return Math.max(0, Math.floor(-top / rowH)) * grid.cols;
  };
  grid.scrollToIndex = i => {
    const top = outer.getBoundingClientRect().top - main.getBoundingClientRect().top;
    if (i > 0) main.scrollTop += top + Math.floor(i / grid.cols) * rowH;
    draw();
  };
  grid.destroy = () => { main.removeEventListener("scroll", onScroll); resize.disconnect(); clearTimeout(animateTimer); };
  return grid;
}

function peopleCountText(n) {
  return peopleQuery ? t("{n} matching", { n: num(n) }) : t("{n} people", { n: num(n) });
}
/** Redraws everything that shows people from the in-memory list (cheap). `animate` makes
 *  cards slide to their new places, for changes like a merge. */
export function refreshPeopleViews({ animate = true } = {}) {
  renderPeopleList();
  if (!peopleGrid) return;
  const items = matchPeople(peopleQuery);
  peopleGrid.setItems(items, { animate });
  $("#peopleCount").textContent = peopleCountText(items.length);
}

const SKELETON_DELAY = 150;
export async function renderPeople(main, job) {
  main.innerHTML = `<div class="people-bar">
      <input class="search" id="peopleSearch" type="search" placeholder="${t("Search people")}" autocomplete="off" value="${esc(peopleQuery)}">
      <span id="peopleCount"></span>
      <select id="peopleSort" title="${t("Sort people")}" aria-label="${t("Sort people")}">
        <option value="count" ${peopleSort === "count" ? "selected" : ""}>${t("Most photos")}</option>
        <option value="name" ${peopleSort === "name" ? "selected" : ""}>${t("Name")}</option>
      </select>
      <label class="zoom" title="${t("Size")}"><span aria-hidden="true">A</span>
        <input type="range" id="peopleZoom" aria-label="${t("Size")}"><span class="big" aria-hidden="true">A</span></label>
      <div class="seg" id="peopleLayout" role="group" aria-label="${t("Layout")}">
        <button data-layout="cards" class="${peopleLayout === "cards" ? "on" : ""}" title="${t("Big pictures")}">${t("Cards")}</button>
        <button data-layout="list" class="${peopleLayout === "list" ? "on" : ""}" title="${t("More people on screen")}">${t("List")}</button>
      </div>
    </div>
    <div id="peopleHost"></div>`;
  const host = $("#peopleHost");
  const zoom = $("#peopleZoom");
  const syncZoom = () => {
    Object.assign(zoom, PEOPLE_ZOOM[peopleLayout]);
    zoom.value = peopleZoom[peopleLayout];
    applyPeopleSize(host);
  };
  // Rebuilds the grid for the current layout and size, keeping the same people on screen.
  const remount = () => {
    if (!peopleGrid) return;
    const anchor = peopleGrid.firstVisible();
    peopleGrid.destroy();
    peopleGrid = mountPeopleGrid(host, { layout: peopleLayoutFor(peopleLayout) });
    refreshPeopleViews({ animate: false });
    peopleGrid.scrollToIndex(anchor);
  };
  syncZoom();
  let zoomFrame = 0;
  zoom.addEventListener("input", () => {
    peopleZoom[peopleLayout] = +zoom.value;
    applyPeopleSize(host);
    pref.set("peopleZoom", JSON.stringify(peopleZoom));
    cancelAnimationFrame(zoomFrame);
    zoomFrame = requestAnimationFrame(remount);
  });
  $("#peopleLayout").addEventListener("click", e => {
    const next = e.target.dataset.layout;
    if (!next || next === peopleLayout) return;
    peopleLayout = next;
    pref.set("peopleLayout", next);
    for (const b of $("#peopleLayout").children) b.classList.toggle("on", b.dataset.layout === next);
    syncZoom();
    remount();
  });
  $("#peopleSort").addEventListener("change", e => {
    setPeopleSort(e.target.value);
    resetPeopleOrder();
    main.scrollTop = 0;
    refreshPeopleViews({ animate: false });
  });
  $("#peopleSearch").addEventListener("input", e => {
    peopleQuery = e.target.value;
    main.scrollTop = 0;
    refreshPeopleViews({ animate: false });
  });
  let placeholders = false, shown = false;
  const show = () => {
    if (!job.alive()) return;
    shown = true;
    if (!people.length) { host.innerHTML = `<div class="blank">${t("No faces found yet.")}</div>`; $("#peopleCount").textContent = ""; return; }
    if (!peopleGrid) {
      resetPeopleOrder(); // opening the tab is when the list gets sorted again
      peopleGrid = mountPeopleGrid(host, { fadeIn: placeholders, layout: peopleLayoutFor(peopleLayout) });
    }
    refreshPeopleViews({ animate: false });
  };
  // What we already have shows at once. Placeholders only appear if loading is slow.
  if (people.length) show();
  else {
    $("#peopleCount").textContent = t("Loading people…");
    setTimeout(() => {
      if (job.alive() && !shown) {
        placeholders = true;
        host.innerHTML = peopleLayoutFor(peopleLayout).skeleton();
      }
    }, SKELETON_DELAY);
  }
  await loadMeta();
  if (!job.alive()) return;
  if (peopleGrid) refreshPeopleViews({ animate: true }); // fresh counts, in place
  else show();
}

// ---- merge dialog ------------------------------------------------------------------
// The same picker serves two purposes: merging a whole person into another ("Same as" in
// People) and moving one face to a person ("Same as" in the photo viewer).
let mergeFrom = null, assignFace = null;
const MERGE_LIMIT = 40;
function openPicker() {
  $("#mergeFilter").value = "";
  showMergeStep(null);
  // Open at once with a spinner; the list fills in on the next frame.
  $("#mergeDlg .candidates").innerHTML = `<div class="spinner" role="progressbar" aria-label="${t("Loading")}"></div>`;
  $("#mergeDlg").showModal();
  $("#mergeFilter").focus();
  const opened = { mergeFrom, assignFace };
  nextFrame().then(() => {
    if (mergeFrom === opened.mergeFrom && assignFace === opened.assignFace && $("#mergeDlg").open) renderMergeCandidates();
  });
}
export function openMerge(id) {
  mergeFrom = id;
  assignFace = null;
  openPicker();
}
/** Picks who a single face in a photo is. `current` is the face's person, if any. */
export function openAssign(face, current) {
  mergeFrom = current;
  assignFace = face;
  openPicker();
}
/** Switches the dialog between picking a person (`into` null) and confirming the merge. */
function showMergeStep(into) {
  const picking = into == null;
  $("#mergePick").hidden = !picking;
  $("#mergeConfirm").hidden = picking;
  $("#mergeTitle").textContent = !picking ? t("Merge these people?") : assignFace != null ? t("Who is this?") : t("Same person as…");
  if (picking) {
    $("#mergeHint").innerHTML = assignFace != null
      ? `<img src="/face/${assignFace}" alt=""><span>${t("Pick who this face is. Only this photo changes.")}</span>`
      : t("Pick who this is. Their photos are combined into one person.");
    return;
  }
  const a = personById(mergeFrom), b = personById(into);
  const face = p => `<figure>
      <div class="avatar">${faceImg(p.face)}</div>
      <figcaption class="${p.name ? "" : "unnamed"}">${esc(personName(p))}</figcaption>
      <div class="s">${plural(p.count, "photo")}</div>
    </figure>`;
  const resultName = b.name || a.name;
  $("#mergeConfirm").innerHTML = `
    <div class="merge-pair">${face(a)}<span class="arrow" aria-hidden="true">→</span>${face(b)}</div>
    <p class="merge-note">${resultName
      ? t("The photos of <b>{from}</b> move to <b>{into}</b>, named <b>{name}</b>. This can't be undone, but a wrong face can later be removed with <i>Not them</i> in the photo viewer.", { from: esc(personName(a)), into: esc(personName(b)), name: esc(resultName) })
      : t("The photos of <b>{from}</b> move to <b>{into}</b>. This can't be undone, but a wrong face can later be removed with <i>Not them</i> in the photo viewer.", { from: esc(personName(a)), into: esc(personName(b)) })}</p>
    <div class="dlg-actions">
      <button class="btn" data-back>${t("Back")}</button>
      <button class="btn primary" data-confirm="${into}">${t("Merge")}</button>
    </div>`;
  $("#mergeConfirm [data-confirm]").focus();
}
// The picker lists people A to Z (accents and case ignored, "Person 2" before "Person 10"),
// with unnamed people last. Sorted once per people list.
export const nameCollator = new Intl.Collator(lang, { sensitivity: "base", numeric: true });
let alphabeticalCache = null;
export function peopleAlphabetical() {
  if (alphabeticalCache?.source === people) return alphabeticalCache.out;
  const out = [...people].sort((a, b) =>
    !a.name !== !b.name ? (a.name ? -1 : 1)
      : a.name ? nameCollator.compare(a.name, b.name) || a.id - b.id
      : a.id - b.id);
  alphabeticalCache = { source: people, out };
  return out;
}
function renderMergeCandidates() {
  const all = matchPeople($("#mergeFilter").value, peopleAlphabetical()).filter(p => p.id !== mergeFrom);
  $("#mergeDlg .candidates").innerHTML = all.slice(0, MERGE_LIMIT).map(p => `
    <button type="button" class="candidate" data-into="${p.id}">
      <span class="avatar-sm">${faceImg(p.face)}</span>
      <span class="n ${p.name ? "" : "unnamed"}">${esc(personName(p))}</span>
      <span class="c">${plural(p.count, "photo")}</span>
    </button>`).join("") +
    (all.length > MERGE_LIMIT ? `<div class="more">${t("{n} more. Type a name to narrow it down.", { n: num(all.length - MERGE_LIMIT) })}</div>` : "") +
    (!all.length ? `<div class="more">${t("Nobody matches.")}</div>` : "");
}
$("#mergeFilter").addEventListener("input", renderMergeCandidates);
$("#mergeDlg").addEventListener("click", e => {
  const dlg = $("#mergeDlg");
  if (e.target === dlg || e.target.dataset.close != null) return dlg.close();
  if (e.target.dataset.back != null) { showMergeStep(null); return $("#mergeFilter").focus(); }
  const row = e.target.closest("[data-into]");
  if (row && assignFace != null) { dlg.close(); return assignFaceTo(assignFace, +row.dataset.into); }
  if (row) return showMergeStep(+row.dataset.into);
  if (!e.target.dataset.confirm) return;
  dlg.close();
  mergePeople(mergeFrom, +e.target.dataset.confirm);
});

// ---- people changes: shown at once, saved in the background ---------------------------
let toastTimer = 0;
export function toast(message, error = false) {
  const el = $("#toast");
  el.textContent = message;
  el.classList.toggle("error", error);
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), error ? 6000 : 2500);
}
/** Reloads the real numbers after a change (or restores them after a failed one). */
const resync = () => loadMeta().then(refreshPeopleViews).catch(() => {});

function mergePeople(from, into) {
  const a = personById(from), b = personById(into);
  if (!a || !b) return;
  const names = { from: personName(a), into: personName(b) };
  const orderBefore = peopleOrder; // so a failed merge puts the person back in their place
  b.count += a.count; // exact count (photos with both) comes with the resync
  if (!b.name) b.name = a.name;
  setPeople(people.filter(p => p.id !== from));
  if (state.people.delete(from)) state.people.add(into);
  refreshPeopleViews();
  toast(t("Merged {from} into {into}", names));
  return post(`/api/people/${from}/merge`, { into })
    .then(resync)
    .catch(err => {
      toast(t("Couldn't merge {from} into {into}: {error}", { ...names, error: err.message }), true);
      restorePeopleOrder(orderBefore);
      resync();
    });
}

/** "Same as" in the photo viewer: moves one face to `person`, then refreshes the viewer. */
async function assignFaceTo(face, person) {
  const p = personById(person);
  try {
    await post(`/api/faces/${face}/assign`, { person });
    toast(p ? t("Moved to {name}", { name: personName(p) }) : t("Moved to that person"));
  } catch (err) {
    toast(t("Couldn't move the face: {error}", { error: err.message }), true);
  }
  await loadMeta();
  refreshPeopleViews();
  if (viewerIndex >= 0) openViewer(viewerIndex);
}

export function toggleHidden(id) {
  const p = personById(id);
  if (!p) return;
  p.hidden = !p.hidden;
  refreshPeopleViews();
  patch(`/api/people/${id}`, { hidden: p.hidden })
    .catch(err => { toast(t("Couldn't save: {error}", { error: err.message }), true); resync(); });
}

// Names are unique. Giving someone a name another person already has asks whether they're
// the same person (merge) or not (the name gets the next free " (n)", like file names).
const sameName = (a, b) => a.trim().toLocaleLowerCase() === b.trim().toLocaleLowerCase();
function uniqueName(wanted, id) {
  const taken = n => people.some(o => o.id !== id && o.name && sameName(o.name, n));
  if (!taken(wanted)) return wanted;
  const base = wanted.match(/^(.*) \((\d+)\)$/)?.[1] ?? wanted; // "Ana (1)" -> "Ana (2)"
  for (let n = 1; ; n++) if (!taken(`${base} (${n})`)) return `${base} (${n})`;
}
/** Resolves to "merge", "keep" or "cancel". */
function askSameName(p, other, name, keepName) {
  const dlg = $("#nameDlg");
  const face = x => `<figure>
      <div class="avatar">${faceImg(x.face)}</div>
      <figcaption class="${x.name ? "" : "unnamed"}">${esc(personName(x))}</figcaption>
      <div class="s">${plural(x.count, "photo")}</div>
    </figure>`;
  $("#nameTitle").textContent = t("\"{name}\" already exists", { name: other.name });
  $("#namePair").innerHTML = `${face(p)}<span class="arrow" aria-hidden="true">?</span>${face(other)}`;
  $("#nameNote").innerHTML = t("Is this the same person? <b>Merge them</b> to combine their photos, or keep them apart and name this one <b>{name}</b>.", { name: esc(keepName) });
  $("#nameKeep").textContent = t("Keep separate as \"{name}\"", { name: keepName });
  dlg.returnValue = "cancel";
  return new Promise(resolve => {
    // Opened after the key press that saved the name has finished, so that Enter can't also
    // press a button here. Focus starts on the close button: a stray Enter cancels rather
    // than merging, which can't be undone.
    setTimeout(() => dlg.showModal(), 0);
    const onClick = e => {
      const choice = e.target.closest("[data-choice]")?.dataset.choice ?? (e.target === dlg ? "cancel" : null);
      if (choice) { dlg.returnValue = choice; dlg.close(); }
    };
    dlg.addEventListener("click", onClick);
    dlg.addEventListener("close", () => { dlg.removeEventListener("click", onClick); resolve(dlg.returnValue || "cancel"); }, { once: true });
  });
}

export async function renamePerson(id, value) {
  const p = personById(id);
  let name = value.trim() || null;
  if (!p || p.name === name) return;
  const other = name && people.find(o => o.id !== id && o.name && sameName(o.name, name));
  if (other) {
    const keepName = uniqueName(name, id);
    const choice = await askSameName(p, other, name, keepName);
    if (choice === "cancel") { refreshPeopleViews({ animate: false }); return; } // old name back in the fields
    if (choice === "merge") return mergePeople(id, other.id);
    name = keepName;
  }
  p.name = name;
  alphabeticalCache = null;
  refreshPeopleViews({ animate: false });
  for (const label of document.querySelectorAll(`[data-person-chip="${id}"] .label`)) label.textContent = personName(p);
  return patch(`/api/people/${id}`, { name: name ?? "" })
    .then(saved => { if (saved?.name && saved.name !== p.name) toast(t("Saved as \"{name}\", a name that was free", { name: saved.name })); })
    .then(resync)
    .catch(err => { toast(t("Couldn't save the name: {error}", { error: err.message }), true); resync(); });
}
