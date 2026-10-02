"use strict";
// Elements that act as buttons without being <button>s (names to rename, faces to open)
// answer Enter and Space like buttons do.
document.addEventListener("keydown", e => {
  if ((e.key === "Enter" || e.key === " ") && e.target.matches?.('[role="button"]:not(button)')) {
    e.preventDefault();
    e.target.click();
  }
});
// Imadive was called Totufoto: preferences saved under the old name carry over once.
try {
  for (const key of Object.keys(localStorage)) {
    const now = key.replace(/^totufoto\./, "imadive.");
    if (now !== key && localStorage.getItem(now) === null) localStorage.setItem(now, localStorage.getItem(key));
  }
} catch {}
const $ = (s, el = document) => el.querySelector(s);
/** An icon from the page's sprite (index.html); `mirrored` flips it left to right. */
const icon = (name, size = 15, mirrored = false) =>
  `<svg class="icon" viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true"><use href="#i-${name}"${mirrored ? ' transform="matrix(-1 0 0 1 24 0)"' : ""}/></svg>`;
/** "1 photo", "2,345 photos". */
const plural = (n, word) => `${n.toLocaleString()} ${word}${n === 1 ? "" : "s"}`;
/** Preferences remembered in this browser (`imadive.<key>`). Storage may be unavailable
 *  (private windows, blocked cookies): then nothing is remembered. */
const pref = {
  get(key) { try { return localStorage.getItem(`imadive.${key}`); } catch { return null; } },
  set(key, value) { try { localStorage.setItem(`imadive.${key}`, value); } catch {} },
};
const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const regionName = (() => {
  try { const d = new Intl.DisplayNames([navigator.language], { type: "region" }); return cc => { try { return d.of(cc); } catch { return cc; } }; }
  catch { return cc => cc; }
})();
const MONTHS = Array.from({ length: 12 }, (_, i) => new Date(2000, i, 1).toLocaleString(undefined, { month: "long" }));
/** The local day of "YYYY-MM-DD..." (capture times are local, without a time zone). */
const parseDay = t => new Date(+t.slice(0, 4), +t.slice(5, 7) - 1, +t.slice(8, 10));
/** "Thu, December 26, 2024" */
const fmtDay = t => parseDay(t).toLocaleDateString(undefined, { weekday: "short", year: "numeric", month: "long", day: "numeric" });
/** "Dec 26, 2024" */
const fmtDate = t => parseDay(t).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
const fmtFull = t => fmtDay(t) + " · " + t.slice(11, 16);

const state = {
  view: "photos", sort: "desc", groupBy: "month", match: "all",
  people: new Set(), place: null, date: null, upcoming: 30,
  from: null, to: null, // capture date range, YYYY-MM-DD, both included
};
let people = [], peopleById = new Map(), places = [], placeById = new Map(), photos = [], viewerIndex = -1, renderToken = 0;
/** The render job of the view on screen: { token, signal, alive() } (see render()). */
let currentJob = { token: 0, signal: undefined, alive: () => true };
/** Replaces the people list, and the lookup by id that goes with it. */
function setPeople(list) {
  people = list;
  peopleById = new Map(list.map(p => [p.id, p]));
}
const personById = id => peopleById.get(id);

// ---- URL state -------------------------------------------------------------
function saveHash() {
  const p = new URLSearchParams();
  if (state.view !== "photos") p.set("view", state.view);
  if (state.sort !== "desc") p.set("sort", state.sort);
  if (state.groupBy !== "month") p.set("group", state.groupBy);
  if (state.match !== "all") p.set("match", state.match);
  if (state.people.size) p.set("people", [...state.people].join(","));
  if (state.place != null) p.set("place", state.place);
  if (state.date) p.set("date", state.date);
  if (state.upcoming !== 30) p.set("days", state.upcoming);
  if (state.from) p.set("from", state.from);
  if (state.to) p.set("to", state.to);
  history.replaceState(history.state, "", "#" + p.toString());
}
function loadHash() {
  const p = new URLSearchParams(location.hash.slice(1));
  state.view = p.get("view") || "photos";
  // The Places tab became "Photos, grouped by place": old links still work.
  const oldPlaces = state.view === "places";
  if (oldPlaces) state.view = "photos";
  state.sort = p.get("sort") || "desc";
  state.groupBy = oldPlaces ? "place" : p.get("group") || "month";
  state.match = p.get("match") || "all";
  state.people = new Set((p.get("people") || "").split(",").filter(Boolean).map(Number));
  state.place = p.get("place") != null && p.get("place") !== "" ? Number(p.get("place")) : null;
  state.date = p.get("date");
  state.upcoming = Number(p.get("days") || 30);
  const day = v => (/^\d{4}-\d{2}-\d{2}$/.test(v || "") ? v : null);
  state.from = day(p.get("from"));
  state.to = day(p.get("to"));
}

// ---- data ------------------------------------------------------------------
/** A failed request: its message (`{"error": "..."}` from the API, else the plain text of
 *  axum's own rejections and the request guard), status code and parsed body. */
class ApiError extends Error {
  constructor(status, message, body) {
    super(message);
    this.status = status;
    this.body = body;
  }
}
/** Fetches JSON; null for answers without a body (202, 204). Throws an ApiError. */
const api = async (url, opts) => {
  const r = await fetch(url, opts);
  if (!r.ok) {
    const text = await r.text();
    let body = null;
    try { body = JSON.parse(text); } catch {}
    throw new ApiError(r.status, body?.error || text || String(r.status), body);
  }
  return r.status === 204 || r.status === 202 ? null : r.json();
};
const post = (url, body) => api(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) });

async function loadMeta() {
  let list;
  [list, places] = await Promise.all([api("/api/people"), api("/api/places")]);
  setPeople(list);
  placeById = new Map(places.map(p => [p.id, p]));
  const known = new Set(people.map(p => p.id));
  for (const id of state.people) if (!known.has(id)) state.people.delete(id);
  renderPeopleList();
  // The People tab opened before there was anyone: show them now that there are.
  if (state.view === "people" && !peopleGrid && people.length && $("#peopleHost .blank, #peopleHost .skeleton")) render();
}
const personName = p => p.name || `Unnamed #${p.id}`;
const placeLabel = id => {
  if (id === 0) return "No location";
  const p = placeById.get(id);
  return p ? `${p.city}, ${regionName(p.country)}` : "Unknown place";
};
/** "2024", "October 2024" or a full day, for a YYYY / YYYY-MM / YYYY-MM-DD date filter. */
const dateLabel = d => d.length === 4 ? d : d.length === 7 ? `${MONTHS[+d.slice(5, 7) - 1]} ${d.slice(0, 4)}` : fmtDay(d);

function photoQuery(extra = {}) {
  const q = new URLSearchParams({ sort: state.sort, ...extra });
  if (state.people.size) { q.set("people", [...state.people].join(",")); q.set("match", state.match); }
  if (state.place != null) q.set("place", state.place);
  if (state.date) q.set("date", state.date);
  if (state.from) q.set("from", state.from);
  if (state.to) q.set("to", state.to);
  return "/api/photos?" + q;
}

// ---- sidebar ---------------------------------------------------------------
// People keep their place on screen while you work: renaming someone (named people sort
// first) or refreshing counts must not reshuffle the list under you. The order is taken
// fresh from the server when the People tab is opened; new people are added at the end.
let peopleOrder = [], orderedCache = null;
// The People tab's sort: "count" (most photos first) or "name" (A to Z, unnamed last).
let peopleSort = "count";
if (pref.get("peopleSort") === "name") peopleSort = "name";
function sortedPeople() {
  if (peopleSort === "name") return peopleAlphabetical();
  return [...people].sort((a, b) => b.count - a.count
    || (!a.name !== !b.name ? (a.name ? -1 : 1) : a.name ? nameCollator.compare(a.name, b.name) : 0)
    || a.id - b.id);
}
function resetPeopleOrder() { peopleOrder = sortedPeople().map(p => p.id); orderedCache = null; }
function orderedPeople() {
  if (!peopleOrder.length && people.length) resetPeopleOrder();
  // Recomputed only when the people list or the order changed.
  if (orderedCache && orderedCache.source === people && orderedCache.order === peopleOrder) return orderedCache.out;
  const byId = new Map(people.map(p => [p.id, p]));
  const out = [];
  for (const id of peopleOrder) { const p = byId.get(id); if (p) { out.push(p); byId.delete(id); } }
  for (const p of people) if (byId.has(p.id)) out.push(p);
  peopleOrder = out.map(p => p.id);
  orderedCache = { source: people, order: peopleOrder, out };
  return out;
}

/** People whose name matches `query`, in display order. */
function matchPeople(query, list = orderedPeople()) {
  const q = query.trim().toLowerCase();
  return q ? list.filter(p => personName(p).toLowerCase().includes(q)) : list;
}

// Large libraries can have thousands of face groups: the sidebar shows the selected
// people plus the top matches, and search finds the rest. Rows are reused between
// updates, so pictures don't reload and nothing flashes.
const SIDEBAR_LIMIT = 150;
const sidebarRows = new Map();
function sidebarRow(p) {
  let row = sidebarRows.get(p.id);
  if (!row) {
    // The checkbox, the picture and the count select; the name is edited in place.
    row = document.createElement("div");
    row.className = "person";
    row.dataset.person = p.id;
    row.innerHTML = `<input type="checkbox" data-id="${p.id}" aria-label="Select">
      <img loading="lazy" alt="" title="Select"><span class="n" title="Click to rename" tabindex="0" role="button"></span><span class="c"></span>`;
    sidebarRows.set(p.id, row);
  }
  const img = row.querySelector("img"), name = row.querySelector(".n");
  if (img.dataset.face !== String(p.face)) { img.dataset.face = p.face; img.src = `/face/${p.face}`; }
  name.textContent = personName(p);
  name.classList.toggle("unnamed", !p.name);
  name.setAttribute("aria-label", `Rename ${personName(p)}`);
  row.querySelector("input").setAttribute("aria-label", `Select ${personName(p)}`);
  row.querySelector(".c").textContent = p.count;
  row.querySelector("input").checked = state.people.has(p.id);
  return row;
}
// The column can be collapsed to a thin strip; remembered in this browser.
function setAsideCollapsed(collapsed) {
  document.body.classList.toggle("aside-collapsed", collapsed);
  const b = $("#asideToggle");
  b.setAttribute("aria-expanded", String(!collapsed));
  b.title = collapsed ? "Show the people column" : "Hide this column";
  pref.set("asideCollapsed", collapsed ? "1" : "");
}
const isPhone = () => matchMedia("(max-width: 639px)").matches;
// On phones the column is a panel that slides in; its button closes it.
$("#asideToggle").addEventListener("click", () => isPhone() ? setDrawer(false) : setAsideCollapsed(!document.body.classList.contains("aside-collapsed")));

// ---- small screens: the people panel and the View panel --------------------------------
function setDrawer(open) {
  document.body.classList.toggle("drawer-open", open);
  $("#peopleBtn").setAttribute("aria-expanded", String(open));
  if (open) setViewPanel(false);
}
function setViewPanel(open) {
  document.body.classList.toggle("view-open", open);
  $("#viewBtn").setAttribute("aria-expanded", String(open));
}
$("#peopleBtn").addEventListener("click", () => setDrawer(!document.body.classList.contains("drawer-open")));
$("#viewBtn").addEventListener("click", e => { e.stopPropagation(); setViewPanel(!document.body.classList.contains("view-open")); });
$("#scrim").addEventListener("click", () => { setDrawer(false); setViewPanel(false); });
// A tap outside the View panel only closes it: the click that follows doesn't also open
// whatever was under the finger.
let swallowClick = false;
document.addEventListener("pointerdown", e => {
  if (!document.body.classList.contains("view-open") || e.target.closest("#tools, #viewBtn")) return;
  setViewPanel(false);
  swallowClick = true;
  setTimeout(() => { swallowClick = false; }, 600);
});
document.addEventListener("click", e => {
  if (!swallowClick) return;
  swallowClick = false;
  e.preventDefault();
  e.stopPropagation();
}, true);
document.addEventListener("keydown", e => {
  if (e.key !== "Escape" || document.querySelector("dialog[open]") || viewer.classList.contains("open")) return;
  setDrawer(false); setViewPanel(false);
});
// The top bar's height (two rows on phones), for placing the View panel under it.
new ResizeObserver(([entry]) => document.documentElement.style.setProperty("--header-h", `${entry.target.offsetHeight}px`)).observe($("header"));
if (pref.get("asideCollapsed") === "1") setAsideCollapsed(true);

let sidebarEditing = false;
/** How many people are selected, on the collapsed column's strip. */
function updateRailCount() {
  const btnCount = $("#peopleBtnCount");
  btnCount.hidden = !state.people.size;
  btnCount.textContent = state.people.size;
  $("#viewBtnDot").hidden = !(state.from || state.to);
  const railCount = $("#railCount");
  railCount.hidden = !state.people.size;
  railCount.textContent = state.people.size;
  railCount.title = `${state.people.size} ${state.people.size === 1 ? "person" : "people"} selected`;
}
function renderPeopleList() {
  updateRailCount();
  const list = $("#peopleList");
  if (sidebarEditing) return; // redrawn when the edit ends
  const all = orderedPeople();
  const visible = all.filter(p => !p.hidden);
  $("#peopleFilter").hidden = visible.length <= 12;
  for (const b of $("#match").children) b.classList.toggle("on", b.dataset.m === state.match);
  if (!visible.length) {
    list.innerHTML = `<div class="empty-side">No people yet. Faces are grouped automatically while the library is indexed.</div>`;
    return;
  }
  const matches = matchPeople($("#peopleFilter").value, visible);
  const rest = matches.filter(p => !state.people.has(p.id));
  const shown = [...visible.filter(p => state.people.has(p.id)), ...rest.slice(0, SIDEBAR_LIMIT)];
  const hiddenCount = rest.length - SIDEBAR_LIMIT;
  const extra = document.createElement("div");
  extra.innerHTML = (hiddenCount > 0 ? `<div class="more">${hiddenCount.toLocaleString()} more. Search to find them.</div>` : "") +
    (!matches.length ? `<div class="empty-side">Nobody matches.</div>` : "");
  const rows = shown.map(sidebarRow);
  // Only touch the list when its rows changed; moving existing rows keeps their pictures.
  const current = [...list.children];
  const same = current.length === rows.length + extra.childElementCount && rows.every((r, i) => current[i] === r) &&
    extra.innerHTML === current.slice(rows.length).map(e => e.outerHTML).join("");
  if (!same) list.replaceChildren(...rows, ...extra.children);
  const keep = new Set(shown.map(p => p.id));
  for (const id of sidebarRows.keys()) if (!keep.has(id) && sidebarRows.size > 600) sidebarRows.delete(id);
}
$("#peopleFilter").addEventListener("input", renderPeopleList);
$("#peopleList").addEventListener("change", e => {
  if (e.target.type !== "checkbox") return;
  const id = Number(e.target.dataset.id);
  e.target.checked ? state.people.add(id) : state.people.delete(id);
  // Photos and Upcoming filter by the selected people; People switches to Photos.
  if (!["photos", "upcoming"].includes(state.view)) state.view = "photos";
  render();
});
$("#peopleList").addEventListener("click", e => {
  const row = e.target.closest(".person");
  if (!row || e.target.type === "checkbox" || e.target.classList.contains("rename")) return;
  if (e.target.classList.contains("n")) return startSidebarRename(row);
  // Anywhere else on the row (picture, count) toggles the checkbox.
  row.querySelector("input[type=checkbox]").click();
});
/** Turns a sidebar name into a text field: Enter or leaving it saves, Esc cancels. */
function startSidebarRename(row) {
  const p = personById(+row.dataset.person);
  if (!p || sidebarEditing) return;
  const label = row.querySelector(".n");
  const input = document.createElement("input");
  input.className = "rename";
  input.value = p.name || "";
  input.placeholder = "Add a name";
  input.setAttribute("aria-label", "Name");
  label.hidden = true;
  label.after(input);
  sidebarEditing = true;
  input.focus();
  input.select();
  let done = false;
  const finish = save => {
    if (done) return;
    done = true;
    sidebarEditing = false;
    const value = input.value;
    input.remove();
    label.hidden = false;
    if (save) renamePerson(p.id, value);
    renderPeopleList();
  };
  input.addEventListener("keydown", e => {
    if (e.key === "Enter") { e.preventDefault(); finish(true); }
    else if (e.key === "Escape") { e.preventDefault(); finish(false); }
  });
  input.addEventListener("blur", () => finish(true));
}
$("#match").addEventListener("click", e => {
  if (!e.target.dataset.m) return;
  state.match = e.target.dataset.m;
  renderPeopleList();
  if (state.people.size) render();
  else saveHash();
});

// ---- photo grid ------------------------------------------------------------
/** A photo as /api/photos sends it (an array, to keep large libraries small), with names. */
const photoRow = ([id, width, height, taken, place, version]) => ({ id, width, height, taken, place, version });

function groupKey(p, mode = state.groupBy) {
  const t = p.taken;
  switch (mode) {
    case "day": return t.slice(0, 10);
    case "month": return t.slice(0, 7);
    case "year": return t.slice(0, 4);
    case "place": return "p" + (p.place ?? "");
    default: return "";
  }
}
function groupTitle(key, sample, mode = state.groupBy) {
  switch (mode) {
    case "day": return fmtDay(key);
    case "month": return `${MONTHS[+key.slice(5, 7) - 1]} ${key.slice(0, 4)}`;
    case "year": return key;
    case "place": return sample.place == null ? "No location" : placeLabel(sample.place);
    default: return "";
  }
}
function groupPhotos(list, keyFn) {
  const groups = [], index = new Map();
  for (const p of list) {
    const k = keyFn(p);
    let g = index.get(k);
    if (!g) { g = { key: k, items: [] }; index.set(k, g); groups.push(g); }
    g.items.push(p);
  }
  return groups;
}
// Thumbnails load lazily with a shimmer behind them and fade in the first time only.
const loadedThumbs = new Set();
function thumbLoaded(img) { loadedThumbs.add(img.dataset.thumb); img.classList.add("ok"); }
/** Picture URLs with the photo's version: a rotated photo gets new ones, so browsers don't
 *  show the old (cached) picture. */
const thumbUrl = (id, v) => v ? `/thumb/${id}?v=${v}` : `/thumb/${id}`;
const originalUrl = (id, v) => v ? `/original/${id}?v=${v}` : `/original/${id}`;
const versionOf = id => photos.find(p => p.id === id)?.version ?? 0;
const tile = p => `<a class="tile" data-id="${p.id}" style="--r:${(p.width / p.height).toFixed(3)}" href="${originalUrl(p.id, p.version)}">` +
  (loadedThumbs.has(String(p.id))
    ? `<img src="${thumbUrl(p.id, p.version)}" loading="lazy" alt="" data-thumb="${p.id}" class="ok"></a>`
    : `<img src="${thumbUrl(p.id, p.version)}" loading="lazy" decoding="async" alt="" data-thumb="${p.id}" onload="thumbLoaded(this)" onerror="thumbLoaded(this)"></a>`);

// ---- loading views: filters change at once, results follow --------------------------
const LOADING_DELAY = 150;
const TILE_SHAPES = [1.5, 0.67, 1.33, 1.5, 1, 1.78, 0.75, 1.5, 1.33, 0.67, 1.5, 1.2];
const photoPlaceholders = () => `<section class="group"><div class="sk-title shimmer"></div><div class="grid">${
  Array.from({ length: 36 }, (_, i) => `<div class="tile skeleton shimmer" style="--r:${TILE_SHAPES[i % TILE_SHAPES.length]}"></div>`).join("")}</div></section>`;
const cardPlaceholders = () => `<div class="cards">${
  `<div class="card skeleton" aria-hidden="true"><div class="sk-img shimmer"></div><div class="meta"><div class="line shimmer"></div><div class="line short shimmer"></div></div></div>`.repeat(10)}</div>`;
/**
 * Prepares `main` as a head (filters, counts) and an area (results) for `view`. The head is
 * updated at once; the current results stay, dimmed, while new ones load, and placeholders
 * replace them if loading is slow. Call `finish()` when the new results are ready.
 */
let headObserver = null;
function watchViewHead(main, head) {
  headObserver?.disconnect();
  headObserver = new ResizeObserver(() => main.style.setProperty("--head-h", `${head.offsetHeight}px`));
  headObserver.observe(head);
  main.style.setProperty("--head-h", `${head.offsetHeight}px`);
}
// A line under the filters bar once content scrolls beneath it.
$("#main").addEventListener("scroll", e => {
  e.currentTarget.querySelector(":scope > .view-head")?.classList.toggle("stuck", e.currentTarget.scrollTop > 0);
}, { passive: true });
function beginViewLoad(main, view, headHtml, placeholders, job) {
  let head = main.querySelector(`:scope > .view-head[data-view="${view}"]`);
  let area = main.querySelector(`:scope > .view-area[data-view="${view}"]`);
  if (!head || !area) {
    main.innerHTML = `<div class="view-head" data-view="${view}"></div><div class="view-area" data-view="${view}"></div>`;
    head = main.firstElementChild; area = main.lastElementChild;
  }
  head.innerHTML = headHtml;
  watchViewHead(main, head);
  area.classList.add("stale");
  let shown = false;
  const timer = setTimeout(() => {
    if (!job.alive()) return;
    shown = true;
    area.classList.remove("stale");
    area.innerHTML = placeholders();
  }, LOADING_DELAY);
  return {
    head, area,
    finish() {
      clearTimeout(timer);
      area.classList.remove("stale", "fade-in");
      area.innerHTML = "";
      if (shown) { void area.offsetWidth; area.classList.add("fade-in"); } // restart the animation
    },
  };
}

/** Renders groups progressively so huge libraries stay responsive. */
function renderGroups(container, groups, headerFn, subFn) {
  const job = currentJob;
  const sentinel = document.createElement("div");
  sentinel.id = "sentinel";
  container.append(sentinel);
  let gi = 0, ii = 0, currentGrid = null, currentSub = null;
  const BATCH = 400;
  function pump() {
    if (!job.alive()) return observer.disconnect();
    let budget = BATCH, html = "";
    const frag = document.createDocumentFragment();
    while (budget > 0 && gi < groups.length) {
      const g = groups[gi];
      if (ii === 0) {
        const sec = document.createElement("section");
        sec.className = "group";
        if (headerFn) sec.innerHTML = headerFn(g);
        frag.append(sec);
        currentGrid = null; currentSub = sec;
      }
      const sub = subFn ? subFn(g.items[ii], ii ? g.items[ii - 1] : null) : null;
      if (sub || !currentGrid) {
        if (sub) currentSub.insertAdjacentHTML("beforeend", `<h4>${sub}</h4>`);
        currentGrid = document.createElement("div");
        currentGrid.className = "grid";
        currentSub.append(currentGrid);
      }
      html = "";
      let end = ii;
      while (end < g.items.length && budget > 0) {
        if (end > ii && subFn && subFn(g.items[end], g.items[end - 1])) break;
        html += tile(g.items[end]); end++; budget--;
      }
      currentGrid.insertAdjacentHTML("beforeend", html);
      ii = end;
      if (ii >= g.items.length) { gi++; ii = 0; }
    }
    container.insertBefore(frag, sentinel);
    if (gi >= groups.length) { observer.disconnect(); sentinel.remove(); }
  }
  const observer = new IntersectionObserver(es => { if (es.some(e => e.isIntersecting)) pump(); }, { root: $("#main"), rootMargin: "1500px" });
  pump();
  observer.observe(sentinel);
}

function rangeLabel() {
  if (state.from && state.to) return `${fmtDate(state.from)} to ${fmtDate(state.to)}`;
  return state.from ? `Since ${fmtDate(state.from)}` : `Until ${fmtDate(state.to)}`;
}
/** Chips for the active filters. */
function filterChips() {
  const chip = (label, key, person = "") =>
    `<span class="chip"${person ? ` data-person-chip="${person}"` : ""}><span class="label">${esc(label)}</span>` +
    `<button data-clear="${key}" title="Remove filter" aria-label="Remove ${esc(label)}">×</button></span>`;
  const selected = [...state.people].map(id => personById(id)).filter(Boolean);
  const parts = [];
  // How the people combine, when it matters.
  if (selected.length > 1 || (selected.length && state.match === "only"))
    parts.push(`<span class="chip-mode">${{ all: "Together:", any: "Any of:", only: "Only:" }[state.match]}</span>`);
  for (const p of selected) parts.push(chip(personName(p), `person:${p.id}`, p.id));
  let count = selected.length;
  if (state.place != null) { parts.push(chip(placeLabel(state.place), "place")); count++; }
  if (state.date) { parts.push(chip(dateLabel(state.date), "date")); count++; }
  if (state.from || state.to) { parts.push(chip(rangeLabel(), "range")); count++; }
  if (count) parts.push(`<button class="clear-all" data-clear="all">Clear all</button>`);
  return `<div class="filters">${parts.join("")}</div>`;
}
function onChipClick(e) {
  const k = e.target.dataset.clear;
  if (!k) return;
  if (k.startsWith("person:")) state.people.delete(+k.slice(7));
  else if (k === "people") state.people.clear();
  else if (k === "range") { state.from = null; state.to = null; }
  else if (k === "all") { state.people.clear(); state.place = null; state.date = null; state.from = null; state.to = null; }
  else state[k] = null;
  syncRangeInputs();
  renderPeopleList();
  render();
}

let shownPhotoCount = 0; // photos the Photos tab shows (as cards or tiles)
async function renderPhotos(main, job) {
  indexedAtRender = lastStatus?.running ? lastStatus.done : 0;
  const shape = photosShape();
  const load = beginViewLoad(main, "photos", filterChips() + `<div class="count">Loading photos…</div>`,
    shape.cards ? cardPlaceholders : photoPlaceholders, job);
  // Grouped photos show as cards: the server sends one line per group, not every photo.
  // Only an opened group loads its photos.
  const data = shape.cards
    ? await api(photoQuery().replace("/api/photos?", `/api/groups?by=${shape.cards}&`), { signal: job.signal })
    : await api(photoQuery(), { signal: job.signal });
  if (!job.alive()) return;
  photos = (data.photos ?? []).map(photoRow);
  shownPhotoCount = data.total ?? photos.length;
  load.finish();
  load.head.querySelector(".count").textContent = `${shownPhotoCount.toLocaleString()} photos`;
  if (!shownPhotoCount) {
    const filtered = state.people.size || state.place != null || state.date || state.from || state.to;
    if (!filtered && !folderInfo.folders.length) {
      main.innerHTML = `<div class="welcome"><h2>Welcome to Imadive</h2>
        <p>Add a folder with photos. It is indexed in the background, and subfolders are included.</p>
        <button class="btn primary" id="welcomeAdd">Add folder…</button></div>`;
      $("#welcomeAdd").onclick = () => (folderInfo.desktop ? addFolder(null) : browseForFolder())
        .then(ok => ok && render()).catch(err => toast(err.message, true));
      return;
    }
    if (!filtered && lastStatus?.running) {
      // The first index: its progress, big, until the first photos are in.
      load.area.innerHTML = `<div class="first-index" id="firstIndex">${firstIndexHtml(lastStatus)}</div>`;
      $("#indexPill").hidden = true;
      return;
    }
    const msg = filtered ? "No photos match these filters." : "No photos yet. They appear here while the folders are indexed.";
    load.area.innerHTML = `<div class="blank">${msg}</div>`;
    return;
  }
  if (shape.cards) {
    const groups = data.groups;
    const unit = { year: "year", month: "month", day: "day", place: "place" }[shape.cards];
    load.head.querySelector(".count").textContent =
      `${plural(groups.length, unit)}, ${shownPhotoCount.toLocaleString()} photos`;
    renderGroupCards(load.area, groups, shape.cards);
    return;
  }
  const groups = groupPhotos(photos, p => groupKey(p, shape.headers));
  const header = shape.headers === "none" ? null :
    g => `<h2>${esc(groupTitle(g.key, g.items[0], shape.headers))}<small>${g.items.length}</small></h2>`;
  renderGroups(load.area, groups, header);
}

// ---- group cards: grouped photos show as cards; a card opens its photos ---------------
const DATE_KEY_LENGTH = { year: 4, month: 7, day: 10 };
const FINER = { year: "month", month: "day", day: "none", place: "month" };
/**
 * What Photos shows for the current grouping and filters: `{ cards: mode }` for one card
 * per group, or `{ headers: mode }` for the photos under headers. A place filter, or a date
 * filter as precise as the grouping, means one group was opened: its photos show, under
 * the next finer headers. A coarser date filter (a year, grouped by month) gives cards
 * inside it.
 */
function photosShape() {
  const mode = state.groupBy;
  if (mode === "none") return { headers: "none" };
  const dateLen = state.date?.length ?? 0;
  if (state.place != null) return { headers: mode === "place" ? "month" : dateLen >= DATE_KEY_LENGTH[mode] ? FINER[mode] : mode };
  if (mode !== "place" && dateLen >= DATE_KEY_LENGTH[mode]) return { headers: FINER[mode] };
  return { cards: mode };
}
/** Filters as a key, to come back to the same scroll position when a group is closed. */
const cardsViewKey = () => JSON.stringify([state.groupBy, state.sort, [...state.people].sort(), state.match, state.place, state.date, state.from, state.to]);
let cardsReturn = null; // { key, scroll } of the cards view left by opening a group
/** Cards from the server's group summaries: `{ key, count, cover }` per group. */
function renderGroupCards(container, groups, mode) {
  const title = key => mode === "place" ? (key === 0 ? "No location" : placeById.get(key)?.city ?? "Unknown place")
    : groupTitle(key, null, mode);
  const card = g => {
    const place = mode === "place" && g.key !== 0 ? placeById.get(g.key) : null;
    const where = place ? `${esc([place.region, regionName(place.country)].filter(Boolean).join(", "))} · ` : "";
    return `<button type="button" class="card group-card" data-group="${esc(String(g.key))}">
      <img src="${thumbUrl(g.cover, g.v)}" loading="lazy" decoding="async" alt="">
      <span class="meta"><span class="t">${esc(title(g.key))}</span>
      <span class="s">${where}${plural(g.count, "photo")}</span></span>
    </button>`;
  };
  const grid = document.createElement("div");
  grid.className = "cards";
  container.append(grid);
  // Cards are added in batches as you scroll; enough are added at once to reach a scroll
  // position being restored.
  const back = cardsReturn?.key === cardsViewKey() ? cardsReturn.scroll : 0;
  cardsReturn = null;
  const main = $("#main"), BATCH = 120, job = currentJob;
  let next = 0;
  const sentinel = document.createElement("div");
  container.append(sentinel);
  const pump = () => {
    if (!job.alive()) return observer.disconnect();
    grid.insertAdjacentHTML("beforeend", groups.slice(next, next + BATCH).map(card).join(""));
    next += BATCH;
    if (next >= groups.length) { observer.disconnect(); sentinel.remove(); }
  };
  const observer = new IntersectionObserver(es => { if (es.some(e => e.isIntersecting)) pump(); }, { root: main, rootMargin: "1200px" });
  pump();
  while (next < groups.length && main.scrollHeight < back + main.clientHeight * 2) pump();
  if (back) main.scrollTop = back;
  if (next < groups.length) observer.observe(sentinel);
}

async function renderUpcoming(main, job) {
  const opts = [7, 14, 30, 60, 90].map(d => `<button data-d="${d}" class="${d === state.upcoming ? "on" : ""}">${d} days</button>`).join("");
  const load = beginViewLoad(main, "upcoming",
    `<div class="upbar">Memories from past years for the next <div class="seg" id="days">${opts}</div></div>` + filterChips(),
    photoPlaceholders, job);
  $("#days").addEventListener("click", e => { if (e.target.dataset.d) { state.upcoming = +e.target.dataset.d; render(); } });
  const data = await api(photoQuery({ upcoming: state.upcoming }), { signal: job.signal });
  if (!job.alive()) return;
  load.finish();
  const order = new Map(data.days.map((d, i) => [d, i]));
  // Upcoming days first (today, tomorrow...), and within a day the most recent year first.
  photos = data.photos.map(photoRow).sort((a, b) =>
    order.get(a.taken.slice(5, 10)) - order.get(b.taken.slice(5, 10)) || b.taken.localeCompare(a.taken));
  if (!photos.length) { load.area.innerHTML = `<div class="blank">No photos were taken on these dates in previous years.</div>`; return; }
  const thisYear = new Date().getFullYear();
  const groups = groupPhotos(photos, p => p.taken.slice(5, 10));
  const header = g => {
    const idx = order.get(g.key);
    const [m, d] = g.key.split("-").map(Number);
    const label = new Date(thisYear, m - 1, d).toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
    const when = idx === 0 ? "Today" : idx === 1 ? "Tomorrow" : `In ${idx} days`;
    return `<h2>${when} · ${esc(label)}<small>${g.items.length}</small></h2>`;
  };
  const sub = (p, prev) => {
    if (prev && prev.taken.slice(0, 4) === p.taken.slice(0, 4)) return null;
    const y = +p.taken.slice(0, 4), ago = thisYear - y;
    return `${plural(ago, "year")} ago · ${y}`;
  };
  renderGroups(load.area, groups, header, sub);
}

// ---- optimization: identical files ------------------------------------------------------
const fmtBytes = n => n < 1024 ? `${n} B` : n < 1048576 ? `${(n / 1024).toFixed(0)} KB`
  : n < 1073741824 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1073741824).toFixed(2)} GB`;
const fmtFileDate = secs => new Date(secs * 1000).toLocaleString(undefined, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
const DUP_BATCH = 100;
async function renderOptimization(main, job) {
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
    // While the scan or the search runs: progress, refreshed every second.
    const what = report.scanning ? "Waiting for the photo scan to finish…"
      : `Checking ${report.done.toLocaleString()} of ${report.total.toLocaleString()} files that could be duplicates`;
    main.innerHTML = `<div class="opt">${intro}<div class="opt-progress"><div>${what}</div>
      <progress ${report.scanning || !report.total ? "" : `max="${report.total}" value="${report.done}"`}></progress></div></div>`;
    // A failed refresh (the server busy for a moment) is tried again, not left on this screen.
    const again = () => {
      if (job.alive() && state.view === "optimization") renderOptimization(main, job).catch(() => setTimeout(again, 3000));
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

// ---- people grid ---------------------------------------------------------------------
// Libraries can have tens of thousands of people, so only the cards on screen exist in the
// page. Everything else (opening dialogs, merging, searching) stays cheap because of that.
const CARD_MIN_W = 150, GRID_GAP = 14, OVERSCAN_ROWS = 2;
let peopleGrid = null, peopleQuery = "";

// Faces fade in the first time they load; after that they show at once.
const loadedFaces = new Set();
function faceLoaded(img) { loadedFaces.add(img.dataset.face); img.classList.add("ok"); }
const faceImg = id => loadedFaces.has(String(id))
  ? `<img src="/face/${id}" alt="" data-face="${id}" class="ok">`
  : `<img src="/face/${id}" alt="" data-face="${id}" decoding="async" onload="faceLoaded(this)" onerror="faceLoaded(this)">`;
const faceCard = p => `
  <div class="card face-card ${p.hidden ? "hidden-person" : ""}" data-person="${p.id}">
    <div class="avatar" title="Show photos" tabindex="0" role="button" aria-label="Show ${esc(personName(p))}'s photos">${faceImg(p.face)}</div>
    <input value="${esc(p.name || "")}" placeholder="Add a name" data-rename="${p.id}">
    <div class="s">${plural(p.count, "photo")}</div>
    <div class="row">
      <button class="btn" data-merge="${p.id}" title="Merge with another person">Same as…</button>
      <button class="btn" data-hide="${p.id}">${p.hidden ? "Show" : "Hide"}</button>
    </div>
  </div>`;
const skeletonCard = () => `
  <div class="card face-card skeleton" aria-hidden="true">
    <div class="avatar shimmer"></div><div class="line shimmer"></div><div class="line short shimmer"></div>
  </div>`;
const faceRow = p => `
  <div class="face-row ${p.hidden ? "hidden-person" : ""}" data-person="${p.id}">
    <div class="avatar" title="Show photos" tabindex="0" role="button" aria-label="Show ${esc(personName(p))}'s photos">${faceImg(p.face)}</div>
    <input value="${esc(p.name || "")}" placeholder="Add a name" data-rename="${p.id}">
    <span class="s">${plural(p.count, "photo")}</span>
    <button class="btn" data-merge="${p.id}" title="Merge with another person">Same as…</button>
    <button class="btn" data-hide="${p.id}">${p.hidden ? "Show" : "Hide"}</button>
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
    card.querySelector("[data-hide]").textContent = p.hidden ? "Show" : "Hide";
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
      const t = pos(i);
      if (el.style.transform !== t) el.style.transform = t;
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
  return peopleQuery ? `${n.toLocaleString()} matching` : `${n.toLocaleString()} people`;
}
/** Redraws everything that shows people from the in-memory list (cheap). `animate` makes
 *  cards slide to their new places, for changes like a merge. */
function refreshPeopleViews({ animate = true } = {}) {
  renderPeopleList();
  if (!peopleGrid) return;
  const items = matchPeople(peopleQuery);
  peopleGrid.setItems(items, { animate });
  $("#peopleCount").textContent = peopleCountText(items.length);
}

const SKELETON_DELAY = 150;
async function renderPeople(main, job) {
  main.innerHTML = `<div class="people-bar">
      <input class="search" id="peopleSearch" type="search" placeholder="Search people" autocomplete="off" value="${esc(peopleQuery)}">
      <span id="peopleCount"></span>
      <select id="peopleSort" title="Sort people" aria-label="Sort people">
        <option value="count" ${peopleSort === "count" ? "selected" : ""}>Most photos</option>
        <option value="name" ${peopleSort === "name" ? "selected" : ""}>Name</option>
      </select>
      <label class="zoom" title="Size"><span aria-hidden="true">A</span>
        <input type="range" id="peopleZoom" aria-label="Size"><span class="big" aria-hidden="true">A</span></label>
      <div class="seg" id="peopleLayout" role="group" aria-label="Layout">
        <button data-layout="cards" class="${peopleLayout === "cards" ? "on" : ""}" title="Big pictures">Cards</button>
        <button data-layout="list" class="${peopleLayout === "list" ? "on" : ""}" title="More people on screen">List</button>
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
    peopleSort = e.target.value;
    pref.set("peopleSort", peopleSort);
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
    if (!people.length) { host.innerHTML = `<div class="blank">No faces found yet.</div>`; $("#peopleCount").textContent = ""; return; }
    if (!peopleGrid) {
      resetPeopleOrder(); // opening the tab is when the list gets sorted again
      peopleGrid = mountPeopleGrid(host, { fadeIn: placeholders, layout: peopleLayoutFor(peopleLayout) });
    }
    refreshPeopleViews({ animate: false });
  };
  // What we already have shows at once. Placeholders only appear if loading is slow.
  if (people.length) show();
  else {
    $("#peopleCount").textContent = "Loading people…";
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
  $("#mergeDlg .candidates").innerHTML = `<div class="spinner" role="progressbar" aria-label="Loading"></div>`;
  $("#mergeDlg").showModal();
  $("#mergeFilter").focus();
  const opened = { mergeFrom, assignFace };
  nextFrame().then(() => {
    if (mergeFrom === opened.mergeFrom && assignFace === opened.assignFace && $("#mergeDlg").open) renderMergeCandidates();
  });
}
function openMerge(id) {
  mergeFrom = id;
  assignFace = null;
  openPicker();
}
/** Picks who a single face in a photo is. `current` is the face's person, if any. */
function openAssign(face, current) {
  mergeFrom = current;
  assignFace = face;
  openPicker();
}
/** Switches the dialog between picking a person (`into` null) and confirming the merge. */
function showMergeStep(into) {
  const picking = into == null;
  $("#mergePick").hidden = !picking;
  $("#mergeConfirm").hidden = picking;
  $("#mergeTitle").textContent = !picking ? "Merge these people?" : assignFace != null ? "Who is this?" : "Same person as…";
  if (picking) {
    $("#mergeHint").innerHTML = assignFace != null
      ? `<img src="/face/${assignFace}" alt=""><span>Pick who this face is. Only this photo changes.</span>`
      : `Pick who this is. Their photos are combined into one person.`;
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
    <p class="merge-note">The photos of <b>${esc(personName(a))}</b> move to <b>${esc(personName(b))}</b>${resultName ? `, named <b>${esc(resultName)}</b>` : ""}. This can't be undone, but a wrong face can later be removed with <i>Not them</i> in the photo viewer.</p>
    <div class="dlg-actions">
      <button class="btn" data-back>Back</button>
      <button class="btn primary" data-confirm="${into}">Merge</button>
    </div>`;
  $("#mergeConfirm [data-confirm]").focus();
}
// The picker lists people A to Z (accents and case ignored, "Person 2" before "Person 10"),
// with unnamed people last. Sorted once per people list.
const nameCollator = new Intl.Collator(undefined, { sensitivity: "base", numeric: true });
let alphabeticalCache = null;
function peopleAlphabetical() {
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
    (all.length > MERGE_LIMIT ? `<div class="more">${(all.length - MERGE_LIMIT).toLocaleString()} more. Type a name to narrow it down.</div>` : "") +
    (!all.length ? `<div class="more">Nobody matches.</div>` : "");
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
function toast(message, error = false) {
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
  const label = `${personName(a)} into ${personName(b)}`;
  const orderBefore = peopleOrder; // so a failed merge puts the person back in their place
  b.count += a.count; // exact count (photos with both) comes with the resync
  if (!b.name) b.name = a.name;
  setPeople(people.filter(p => p.id !== from));
  if (state.people.delete(from)) state.people.add(into);
  refreshPeopleViews();
  toast(`Merged ${label}`);
  return post(`/api/people/${from}/merge`, { into })
    .then(resync)
    .catch(err => {
      toast(`Couldn't merge ${label}: ${err.message}`, true);
      peopleOrder = orderBefore;
      orderedCache = null;
      resync();
    });
}

/** "Same as" in the photo viewer: moves one face to `person`, then refreshes the viewer. */
async function assignFaceTo(face, person) {
  const p = personById(person);
  try {
    await post(`/api/faces/${face}/assign`, { person });
    toast(`Moved to ${p ? personName(p) : "that person"}`);
  } catch (err) {
    toast(`Couldn't move the face: ${err.message}`, true);
  }
  await loadMeta();
  refreshPeopleViews();
  if (viewerIndex >= 0) openViewer(viewerIndex);
}

function toggleHidden(id) {
  const p = personById(id);
  if (!p) return;
  p.hidden = !p.hidden;
  refreshPeopleViews();
  post(`/api/people/${id}`, { hidden: p.hidden })
    .catch(err => { toast(`Couldn't save: ${err.message}`, true); resync(); });
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
  $("#nameTitle").textContent = `"${other.name}" already exists`;
  $("#namePair").innerHTML = `${face(p)}<span class="arrow" aria-hidden="true">?</span>${face(other)}`;
  $("#nameNote").innerHTML = `Is this the same person? <b>Merge them</b> to combine their photos, or keep them apart and name this one <b>${esc(keepName)}</b>.`;
  $("#nameKeep").textContent = `Keep separate as "${keepName}"`;
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

async function renamePerson(id, value) {
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
  return post(`/api/people/${id}`, { name: name ?? "" })
    .then(saved => { if (saved && saved.name !== p.name) toast(`Saved as "${saved.name}", a name that was free`); })
    .then(resync)
    .catch(err => { toast(`Couldn't save the name: ${err.message}`, true); resync(); });
}

let viewRequest = null; // the current view's request, cancelled when another view replaces it
async function render() {
  saveHash();
  for (const b of $("#tabs").children) {
    b.classList.toggle("on", b.dataset.view === state.view);
    if (b.dataset.view === state.view) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  }
  // People and Optimization have no "People in the photo" column.
  const noAside = state.view === "people" || state.view === "optimization";
  document.body.classList.toggle("no-aside", noAside);
  if (noAside) setDrawer(false);
  updateRailCount();
  $("#groupBy").value = state.groupBy;
  $("#sort").value = state.sort;
  const main = $("#main");
  // One job per render. A newer one replaces it: its request is cancelled, and alive()
  // tells the views still loading or drawing in batches to stop.
  const token = ++renderToken;
  viewRequest?.abort();
  viewRequest = new AbortController();
  const job = currentJob = { token, signal: viewRequest.signal, alive: () => token === renderToken };
  peopleGrid?.destroy();
  peopleGrid = null;
  main.scrollTop = 0;
  try {
    if (state.view === "photos") await renderPhotos(main, job);
    else if (state.view === "upcoming") await renderUpcoming(main, job);
    else if (state.view === "optimization") await renderOptimization(main, job);
    else await renderPeople(main, job);
  } catch (err) {
    if (err.name === "AbortError" || !job.alive()) return;
    main.innerHTML = `<div class="blank">Something went wrong: ${esc(err.message)}</div>`;
  }
}

// ---- main area events ---------------------------------------------------------
$("#main").addEventListener("click", e => {
  if (e.target.dataset.clear) return onChipClick(e);
  const t = e.target.closest(".tile");
  if (t) { e.preventDefault(); return openViewer(photos.findIndex(p => p.id === +t.dataset.id)); }
  const group = e.target.closest("[data-group]");
  if (group) {
    cardsReturn = { key: cardsViewKey(), scroll: $("#main").scrollTop };
    const key = group.dataset.group;
    if (state.groupBy === "place") state.place = +key;
    else state.date = key;
    return render();
  }
  if (e.target.dataset.merge) return openMerge(+e.target.dataset.merge);
  if (e.target.dataset.hide) return toggleHidden(+e.target.dataset.hide);
  if (e.target.closest(".face-card:not(.skeleton) .avatar, .face-row:not(.skeleton) .avatar")) {
    state.people = new Set([+e.target.closest("[data-person]").dataset.person]);
    state.view = "photos"; renderPeopleList(); return render();
  }
});
$("#main").addEventListener("change", e => {
  if (e.target.dataset.rename) renamePerson(+e.target.dataset.rename, e.target.value);
});
$("#main").addEventListener("keydown", e => { if (e.key === "Enter" && e.target.dataset.rename) e.target.blur(); });

// Date range: either end may be empty; each picker can't go past the other.
function syncRangeInputs() {
  const from = $("#fromDate"), to = $("#toDate");
  from.value = state.from || "";
  to.value = state.to || "";
  from.max = state.to || "";
  to.min = state.from || "";
}
for (const id of ["fromDate", "toDate"]) {
  $("#" + id).addEventListener("change", () => {
    const from = $("#fromDate").value || null, to = $("#toDate").value || null;
    // A reversed range (typed by hand) is put the right way round.
    [state.from, state.to] = from && to && from > to ? [to, from] : [from, to];
    syncRangeInputs();
    if (state.view === "people" || state.view === "optimization") state.view = "photos";
    render();
  });
}
$("#tabs").addEventListener("click", e => { if (e.target.dataset.view) { state.view = e.target.dataset.view; render(); } });
$("#groupBy").addEventListener("change", e => { state.groupBy = e.target.value; if (state.view !== "photos") state.view = "photos"; render(); });
$("#sort").addEventListener("change", e => { state.sort = e.target.value; render(); });

// ---- viewer ----------------------------------------------------------------
const viewer = $("#viewer");
/** Sizes the photo to its final on-screen size from its known dimensions, so the thumbnail
 *  shown first and the full photo that replaces it occupy exactly the same box. */
/** Sizes the viewer's photo to fit; `sideways` while it shows turned a quarter (rotating). */
function fitViewerImage(w, h, sideways = false) {
  const stage = $(".stage", viewer), img = $(".frame img", viewer);
  // Wide screens keep room for the ‹ › buttons beside the photo; on phones they float over it.
  const maxW = stage.clientWidth - (stage.clientWidth < 640 ? 16 : 120), maxH = stage.clientHeight - (stage.clientWidth < 640 ? 16 : 40);
  const scale = sideways ? Math.min(maxW / h, maxH / w, 1) : Math.min(maxW / w, maxH / h, 1);
  img.style.width = `${Math.max(1, Math.round(w * scale))}px`;
  img.style.height = `${Math.max(1, Math.round(h * scale))}px`;
}
window.addEventListener("resize", () => {
  if (viewerIndex < 0) return;
  resetZoom();
  if (rotation) previewRotation();
  else fitViewerImage(photos[viewerIndex].width, photos[viewerIndex].height);
});

/** Shows photo `i`: its thumbnail at the final size at once, then the full photo. */
async function showViewerImage(i, id, w, h, v) {
  const img = $(".frame img", viewer);
  // Keep the current photo until the next thumbnail is ready (they're cached, so this is
  // quick), then show it at the final size; the full photo sharpens it in place.
  const thumb = new Image();
  thumb.src = thumbUrl(id, v);
  await thumb.decode().catch(() => {});
  if (viewerIndex !== i) return;
  img.classList.remove("turning");
  img.style.rotate = "";
  viewer.classList.remove("rotating");
  fitViewerImage(w, h);
  img.src = thumb.src;
  const full = new Image();
  full.src = originalUrl(id, v);
  full.decode()
    .then(() => { if (viewerIndex === i) img.src = full.src; })
    .catch(() => { if (viewerIndex === i) photoMissing(id); });
  // Ready for the arrows: neighbours' thumbnails, and the next photo in full size.
  for (const j of [i - 1, i + 1]) if (photos[j]) new Image().src = thumbUrl(photos[j].id, photos[j].version);
  if (photos[i + 1]) new Image().src = originalUrl(photos[i + 1].id, photos[i + 1].version);
}
/** `keepImage`: only refresh the details and face boxes (the photo shown is already right). */
async function openViewer(i, { keepImage = false } = {}) {
  if (i < 0 || i >= photos.length) return;
  if (rotation && rotation.id !== photos[i].id) flushRotation();
  viewerIndex = i;
  resetZoom();
  // No arrow where there is no photo to go to.
  $(".prev", viewer).hidden = i === 0;
  $(".next", viewer).hidden = i === photos.length - 1;
  const { id, width: w, height: h, version: v } = photos[i];
  viewer.classList.add("open");
  setViewerModal(true);
  if (!keepImage) await showViewerImage(i, id, w, h, v);
  if (viewerIndex !== i) return;
  let d;
  try { d = await api(`/api/photos/${id}`); }
  catch (err) {
    if (viewerIndex !== i) return;
    // Not the previous photo's details: say what happened.
    $(".boxes", viewer).innerHTML = "";
    $(".info-body", viewer).innerHTML = `<div class="s">Couldn't load this photo's details: ${esc(err.message)}</div>`;
    return;
  }
  if (viewerIndex !== i) return;
  $(".boxes", viewer).innerHTML = d.faces.map(f => {
    const person = personById(f.person);
    const label = person ? personName(person) : "";
    return `<div class="box" style="left:${f.box[0] * 100}%;top:${f.box[1] * 100}%;width:${f.box[2] * 100}%;height:${f.box[3] * 100}%">${label ? `<span>${esc(label)}</span>` : ""}</div>`;
  }).join("");
  viewerPath = d.path;
  const place = d.city ? `${d.city}${d.region ? ", " + d.region : ""}, ${regionName(d.country)}` : null;
  $(".info-body", viewer).innerHTML = `
    <h3>${esc(fmtFull(d.taken))}</h3>
    <div class="s">${d.dateFromExif ? "" : "Date from file (no EXIF) · "}${d.width} × ${d.height}</div>
    <div class="photo-acts">
      ${folderInfo.desktop && canShareFiles
        ? `<button data-share-photo="${id}" title="Send this photo with another app">${icon("share", 15)} Share</button>`
        : `<a class="btn-like" href="/original/${id}?download=1&v=${v}" download title="Save the photo on this device">${icon("download", 15)} Download</a>`}
      ${d.rotatable ? `<button data-rotate="-1" title="Rotate left (Shift+R)" aria-label="Rotate left">${icon("rotate", 15)}</button><button data-rotate="1" title="Rotate right (R)" aria-label="Rotate right">${icon("rotate", 15, true)}</button>` : ""}
      ${folderInfo.desktop ? `<button data-reveal="${id}" title="Show the file in its folder">${icon("folder", 15)} Open in folder</button>` : ""}
    </div>
    ${place ? `<h5>Place</h5><div>${esc(place)}</div><div class="s"><a href="https://www.openstreetmap.org/?mlat=${d.lat}&mlon=${d.lon}#map=14/${d.lat}/${d.lon}" target="_blank" rel="noopener">Open map</a></div>` : ""}
    <h5>People (${d.faces.length})</h5>
    ${d.faces.length ? d.faces.map(f => {
      const person = personById(f.person);
      return `<div class="vface"><img src="/face/${f.id}" alt="">
        <span class="who">${person?.name
          ? `<span class="n" data-person="${f.person}" title="Show ${esc(person.name)}'s photos" tabindex="0" role="button">${esc(person.name)}</span>`
          : `<span class="n unnamed editable" data-name-face="${f.id}" data-current="${f.person ?? ""}" title="Click to name this person" tabindex="0" role="button">${person ? esc(personName(person)) : "Not grouped"}</span>`}
        <span class="acts">
          <button data-assign="${f.id}" data-current="${f.person ?? ""}" title="Pick who this face is">Same as…</button>
          ${person ? `<button data-reject="${f.id}" title="This face is not ${esc(personName(person))}: move it to a new group you can rename or hide">Not them</button>` : ""}
          ${!person ? "" : person.face === f.id
            ? `<button class="is-cover" disabled title="This face is on ${esc(personName(person))}'s card in People">✓ Card photo</button>`
            : `<button data-cover="${f.id}" title="Show this face on ${esc(personName(person))}'s card in People">Card photo</button>`}
        </span></span></div>`;
    }).join("") : `<div class="s">No faces detected.</div>`}
    <label><input type="checkbox" id="showBoxes" ${viewer.classList.contains("boxes") ? "checked" : ""}> Show face boxes</label>
    <h5>File</h5><div class="s">${esc(d.path)}</div>`;
}
/** The full photo couldn't be loaded: ask the server why. A file that is gone (its folder
 *  still there) is removed from the gallery; an unreachable folder removes nothing. */
async function photoMissing(id) {
  let result;
  try { result = await post(`/api/photos/${id}/check`); } catch { return; }
  if (!result || result.status === "present") return; // a passing glitch; nothing to say
  const dlg = $("#goneDlg");
  if (result.status === "removed") {
    $("#goneTitle").textContent = "This photo is no longer there";
    $("#goneText").textContent = "Its file is no longer in its folder, so it has been removed from the gallery.";
  } else {
    $("#goneTitle").textContent = "This photo can't be reached";
    $("#goneText").textContent = `Its folder can't be reached right now (an unplugged drive or a network folder, for example). Nothing was removed: it will show again when the folder is back.`;
  }
  $("#gonePath").textContent = result.path || "";
  dlg.showModal();
  await new Promise(r => dlg.addEventListener("close", r, { once: true }));
  if (result.status !== "removed") return;
  dropPhotoFromView(id);
}
/** A photo left the gallery: take it off the screen, update counts, move the viewer on. */
function dropPhotoFromView(id) {
  const index = photos.findIndex(p => p.id === id);
  if (index >= 0) photos.splice(index, 1);
  $(`.tile[data-id="${id}"]`)?.remove();
  const count = $(".view-head .count");
  if (count && /^[\d,]+ photos$/.test(count.textContent)) count.textContent = `${Math.max(0, parseInt(count.textContent.replace(/,/g, "")) - 1).toLocaleString()} photos`;
  shownPhotoCount = Math.max(0, shownPhotoCount - 1);
  if (photos.length && viewer.classList.contains("open")) openViewer(Math.min(Math.max(index, 0), photos.length - 1));
  else closeViewer();
  loadMeta().then(() => refreshPeopleViews({ animate: false })).catch(() => {});
}

// ---- sharing and showing a photo's file (desktop app) -----------------------------------
let viewerPath = "";
/** Whether this window has a system share sheet that takes files. */
const canShareFiles = (() => {
  try { return !!navigator.canShare?.({ files: [new File([""], "photo.jpg", { type: "image/jpeg" })] }); } catch { return false; }
})();
async function sharePhoto(id, button) {
  const name = viewerPath.split(/[\\/]/).pop() || `photo-${id}.jpg`;
  button.disabled = true;
  try {
    const blob = await (await fetch(originalUrl(id, versionOf(id)))).blob();
    const file = new File([blob], blob.type === "image/jpeg" ? name.replace(/\.(heic|heif|tiff?|avif)$/i, ".jpg") : name, { type: blob.type });
    await navigator.share({ files: [file] });
  } catch (err) {
    if (err.name !== "AbortError") toast(`Couldn't share it: ${err.message}`, true);
  } finally {
    button.disabled = false;
  }
}
async function revealPhoto(id) {
  try { await post(`/api/photos/${id}/reveal`); }
  catch (err) { toast(`Couldn't open its folder: ${err.message}`, true); }
}

// ---- rotating a photo ------------------------------------------------------------------
// The photo turns on screen at once; the file is saved once the clicks stop for a moment
// (or on leaving the photo), so several quick turns are one save.
let rotation = null; // { id, turns, timer } while turns wait to be saved
function rotateViewer(dir) {
  if (viewerIndex < 0) return;
  const id = photos[viewerIndex].id;
  if (rotation?.id !== id) { flushRotation(); rotation = { id, turns: 0, timer: 0 }; }
  rotation.turns += dir;
  resetZoom();
  previewRotation();
  clearTimeout(rotation.timer);
  rotation.timer = setTimeout(flushRotation, 800);
}
function previewRotation() {
  const { width: w, height: h } = photos[viewerIndex];
  const img = $(".frame img", viewer);
  img.classList.add("turning");
  viewer.classList.add("rotating"); // face boxes hide until the turned photo is back
  img.style.rotate = `${rotation.turns * 90}deg`;
  fitViewerImage(w, h, Math.abs(rotation.turns) % 2 === 1);
}
async function flushRotation() {
  const r = rotation;
  if (!r) return;
  clearTimeout(r.timer);
  rotation = null;
  const turns = ((r.turns % 4) + 4) % 4;
  const here = () => viewerIndex >= 0 && photos[viewerIndex]?.id === r.id;
  const unturn = (w, h) => {
    const img = $(".frame img", viewer);
    img.classList.remove("turning");
    img.style.rotate = "";
    viewer.classList.remove("rotating");
    fitViewerImage(w, h);
  };
  if (!turns) {
    if (here() && !rotation) unturn(photos[viewerIndex].width, photos[viewerIndex].height);
    return;
  }
  let res;
  try {
    res = await post(`/api/photos/${r.id}/rotate`, { turns });
  } catch (err) {
    if (here() && !rotation) unturn(photos[viewerIndex].width, photos[viewerIndex].height);
    return toast(`Couldn't rotate the photo: ${err.message}`, true);
  }
  const p = photos.find(p => p.id === r.id);
  if (p) Object.assign(p, { width: res.width, height: res.height, version: res.version });
  // The new pictures are ready before they replace the turned one, so nothing flickers.
  const full = new Image(), thumb = new Image();
  full.src = originalUrl(r.id, res.version);
  thumb.src = thumbUrl(r.id, res.version);
  await Promise.all([full.decode(), thumb.decode()]).catch(() => {});
  for (const t of document.querySelectorAll(`.tile[data-id="${r.id}"]`)) {
    t.style.setProperty("--r", (res.width / res.height).toFixed(3));
    t.href = originalUrl(r.id, res.version);
  }
  for (const im of document.querySelectorAll("img")) {
    if (im.src && new URL(im.src).pathname === `/thumb/${r.id}`) im.src = thumb.src;
  }
  if (!here()) return;
  const img = $(".frame img", viewer);
  if (rotation) {
    img.src = full.src;
    previewRotation(); // turned again meanwhile: keep showing that
  } else {
    unturn(res.width, res.height);
    img.src = full.src;
  }
  openViewer(viewerIndex, { keepImage: true });
}

/** Asks something in the app's own dialog (deleting a photo, removing a folder, leaving the
 *  gallery...). `body` is HTML whose buttons carry `data-choice`; resolves to the chosen
 *  value, or "cancel" when the dialog is closed. */
function askChoice(body, title) {
  const dlg = $("#choiceDlg");
  $("#choiceTitle").textContent = title;
  $(".dlg-body", dlg).innerHTML = body;
  dlg.returnValue = "cancel";
  return new Promise(resolve => {
    // Opened after the key press that asked for it; Cancel has the focus.
    setTimeout(() => { dlg.showModal(); dlg.querySelector('.dlg-actions [data-choice="cancel"]')?.focus(); }, 0);
    const onClick = e => {
      const choice = e.target.closest("[data-choice]")?.dataset.choice ?? (e.target === dlg ? "cancel" : null);
      if (choice) { dlg.returnValue = choice; dlg.close(); }
    };
    dlg.addEventListener("click", onClick);
    dlg.addEventListener("close", () => { dlg.removeEventListener("click", onClick); resolve(dlg.returnValue || "cancel"); }, { once: true });
  });
}
// ---- deleting a photo -----------------------------------------------------------------
async function deleteViewerPhoto() {
  if (viewerIndex < 0) return;
  const id = photos[viewerIndex].id;
  const detail = await api(`/api/photos/${id}`).catch(() => null);
  const path = detail?.path ?? "";
  const what = `<div class="delete-what"><img src="${thumbUrl(id, versionOf(id))}" alt=""><div class="p">${esc(path)}</div></div>`;
  const choice = await askChoice(`${what}
    <div class="delete-options">
      <button class="btn" data-choice="gallery">Remove from gallery<small>The file stays on disk. It won't come back with the next scan (Settings can show it again).</small></button>
      <button class="btn danger" data-choice="disk">Remove from disk<small>Moves the file to the bin of the computer running Imadive.</small></button>
    </div>
    <div class="dlg-actions"><button class="btn" data-choice="cancel">Cancel</button></div>`, "Delete this photo?");
  if (choice === "cancel") return;
  const remove = (from, permanently = false) => post(`/api/photos/${id}/remove`, { from, permanently })
    .then(body => ({ ok: true, status: 200, body }))
    .catch(err => ({ ok: false, status: err.status, body: err.body ?? { error: err.message } }));
  let result = await remove(choice);
  if (result.status === 409 && result.body.status === "no-bin") {
    // No bin to move it to (some external or network drives): only delete for good if confirmed.
    const again = await askChoice(`${what}<p>This file can't be moved to a bin on its drive, so it can't be recovered once deleted.</p>
      <div class="dlg-actions"><button class="btn" data-choice="cancel">Cancel</button><button class="btn danger" data-choice="permanently">Delete permanently</button></div>`,
      "Delete it permanently?");
    if (again !== "permanently") return;
    result = await remove("disk", true);
  }
  if (!result.ok) return toast(`Couldn't delete the photo: ${result.body.error || result.status}`, true);
  toast({ removed: "Removed from the gallery; the file is still on disk", binned: "Moved to the bin", deleted: "Deleted permanently" }[result.body.status] || "Removed");
  dropPhotoFromView(id);
}
$(".trash", viewer).addEventListener("click", deleteViewerPhoto);
$("#goneDlg").addEventListener("click", e => { if (e.target.dataset.close != null || e.target === e.currentTarget) e.currentTarget.close(); });

/** Names an unnamed person (or a face in no group) from the viewer. A name that exists
 *  already offers to merge, as anywhere else; a face in no group gets a group of its own. */
function startViewerRename(label) {
  const face = +label.dataset.nameFace, current = label.dataset.current ? +label.dataset.current : null;
  const input = document.createElement("input");
  input.className = "rename";
  input.placeholder = "Add a name";
  input.setAttribute("aria-label", "Name");
  label.hidden = true;
  label.after(input);
  input.focus();
  let done = false;
  const finish = async save => {
    if (done) return;
    done = true;
    const value = input.value.trim();
    input.remove();
    label.hidden = false;
    if (!save || !value) return;
    label.textContent = value; // shown at once; the panel refreshes when it's saved
    label.classList.remove("unnamed");
    try {
      let person = current;
      if (person == null) {
        person = (await post(`/api/faces/${face}/reject`)).person; // a group of its own first
        await loadMeta();
      }
      await renamePerson(person, value);
    } catch (err) {
      toast(`Couldn't save the name: ${err.message}`, true);
    }
    await loadMeta().catch(() => {});
    refreshPeopleViews({ animate: false });
    if (viewerIndex >= 0) openViewer(viewerIndex);
  };
  input.addEventListener("keydown", e => {
    e.stopPropagation(); // arrows and Esc belong to the field, not the viewer
    if (e.key === "Enter") { e.preventDefault(); finish(true); }
    else if (e.key === "Escape") { e.preventDefault(); finish(false); }
  });
  input.addEventListener("blur", () => finish(true));
}

function closeViewer() {
  flushRotation();
  resetZoom();
  viewer.classList.remove("open");
  setViewerModal(false);
  viewerIndex = -1;
  $(".frame img", viewer).src = "";
}
/** While the viewer is open, the page behind it can't be reached (keyboard, screen
 *  readers); focus moves into it, and back to where it was when it closes. */
let focusBeforeViewer = null;
function setViewerModal(open) {
  const behind = [$("header"), $("aside"), $("#main"), $("#indexPill")];
  if (open && !behind[0].inert) {
    focusBeforeViewer = document.activeElement;
    behind.forEach(el => { el.inert = true; });
    $(".close", viewer).focus({ preventScroll: true });
  } else if (!open && behind[0].inert) {
    behind.forEach(el => { el.inert = false; });
    if (focusBeforeViewer?.isConnected) focusBeforeViewer.focus({ preventScroll: true });
    focusBeforeViewer = null;
  }
}

// ---- viewer zoom ----------------------------------------------------------------------
// Click zooms in where clicked (click again to fit), drag moves the zoomed photo, the wheel
// (or a trackpad pinch) zooms around the pointer. The frame holding the photo and its face
// boxes is scaled from its top left corner and moved from its fitted place.
const zoom = { s: 1, x: 0, y: 0 };
const ZOOM_MAX = 8;
const frameEl = () => $(".frame", viewer);
function applyZoom(animate = false) {
  const frame = frameEl();
  frame.classList.toggle("animate-zoom", animate);
  frame.style.transform = zoom.s === 1 ? "" : `translate(${zoom.x}px, ${zoom.y}px) scale(${zoom.s})`;
  frame.style.setProperty("--zs", zoom.s);
  viewer.classList.toggle("zoomed", zoom.s > 1);
}
function resetZoom(animate = false) {
  zoom.s = 1; zoom.x = 0; zoom.y = 0;
  applyZoom(animate);
}
/** Keeps the zoomed photo in view: centred while smaller than the stage, and never with
 *  a gap at an edge once larger. */
function clampZoom() {
  const stage = $(".stage", viewer), frame = frameEl();
  const axis = (t, offset, size, limit) => {
    const scaled = size * zoom.s;
    if (scaled <= limit) return (limit - scaled) / 2 - offset;
    return Math.min(-offset, Math.max(limit - scaled - offset, t));
  };
  zoom.x = axis(zoom.x, frame.offsetLeft, frame.offsetWidth, stage.clientWidth);
  zoom.y = axis(zoom.y, frame.offsetTop, frame.offsetHeight, stage.clientHeight);
}
/** Zooms to `scale`, keeping the point under the pointer where it is. */
function zoomAt(clientX, clientY, scale, animate = false) {
  const next = Math.min(ZOOM_MAX, Math.max(1, scale));
  if (next === 1) return resetZoom(animate);
  const frame = frameEl(), stage = $(".stage", viewer).getBoundingClientRect();
  const px = clientX - stage.left - frame.offsetLeft, py = clientY - stage.top - frame.offsetTop;
  zoom.x = px - ((px - zoom.x) * next) / zoom.s;
  zoom.y = py - ((py - zoom.y) * next) / zoom.s;
  zoom.s = next;
  clampZoom();
  applyZoom(animate);
}
{
  const frame = frameEl();
  // Pointers on the photo: one to tap, drag or swipe; two to pinch (touch screens).
  const pointers = new Map(); // pointerId -> { x, y }
  let press = null; // single pointer: { x, y, zx, zy, moved }
  let pinch = null; // two pointers: { dist, scale }
  const distance = () => { const [a, b] = [...pointers.values()]; return Math.hypot(a.x - b.x, a.y - b.y); };
  const middle = () => { const [a, b] = [...pointers.values()]; return [(a.x + b.x) / 2, (a.y + b.y) / 2]; };
  const SWIPE = 60; // px sideways to go to the next or previous photo
  frame.addEventListener("pointerdown", e => {
    if (e.pointerType === "mouse" && e.button !== 0) return;
    e.preventDefault();
    frame.setPointerCapture(e.pointerId);
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pointers.size === 2) {
      pinch = { dist: distance(), scale: zoom.s };
      press = null; // a pinch is never a tap or a swipe
      frame.style.translate = "";
    } else if (pointers.size === 1) {
      press = { x: e.clientX, y: e.clientY, zx: zoom.x, zy: zoom.y, moved: false };
    }
  });
  frame.addEventListener("pointermove", e => {
    if (!pointers.has(e.pointerId)) return;
    pointers.set(e.pointerId, { x: e.clientX, y: e.clientY });
    if (pinch && pointers.size === 2) {
      const [mx, my] = middle();
      zoomAt(mx, my, (pinch.scale * distance()) / pinch.dist);
      return;
    }
    if (!press) return;
    const dx = e.clientX - press.x, dy = e.clientY - press.y;
    if (!press.moved && Math.hypot(dx, dy) < 5) return; // still a tap
    press.moved = true;
    if (zoom.s === 1) {
      // Not zoomed: the photo follows a sideways swipe.
      if (e.pointerType !== "mouse") frame.style.translate = `${dx}px 0`;
      return;
    }
    frame.classList.add("dragging");
    zoom.x = press.zx + dx;
    zoom.y = press.zy + dy;
    clampZoom();
    applyZoom();
  });
  const release = e => {
    if (!pointers.has(e.pointerId)) return;
    pointers.delete(e.pointerId);
    if (pinch) {
      if (pointers.size < 2) pinch = null;
      return; // lifting a finger after a pinch does nothing else
    }
    if (!press) return;
    const { x, y, moved } = press;
    press = null;
    frame.classList.remove("dragging");
    const dx = e.clientX - x, dy = e.clientY - y;
    if (frame.style.translate) {
      frame.style.translate = "";
      if (e.type !== "pointercancel" && Math.abs(dx) > SWIPE && Math.abs(dx) > Math.abs(dy) * 1.5) {
        return openViewer(viewerIndex + (dx < 0 ? 1 : -1));
      }
    }
    if (moved || e.type === "pointercancel") return;
    if (zoom.s > 1) return resetZoom(true);
    // Zoom to the photo's real pixels (between 2x and 4x) where it was tapped.
    const width = photos[viewerIndex]?.width ?? 0;
    zoomAt(e.clientX, e.clientY, Math.min(4, Math.max(2, width / frame.offsetWidth)), true);
  };
  frame.addEventListener("pointerup", release);
  frame.addEventListener("pointercancel", release);
  $(".stage", viewer).addEventListener("wheel", e => {
    if (viewerIndex < 0) return;
    e.preventDefault();
    // Trackpad pinches come as wheel events with ctrlKey and small steps.
    zoomAt(e.clientX, e.clientY, zoom.s * Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.0015)));
  }, { passive: false });
}
$(".close", viewer).onclick = closeViewer;
$(".prev", viewer).onclick = () => openViewer(viewerIndex - 1);
$(".next", viewer).onclick = () => openViewer(viewerIndex + 1);
$(".stage", viewer).addEventListener("click", e => { if (e.target.classList.contains("stage")) closeViewer(); });
// The details can fold away, leaving the photo more room; remembered in this browser.
function setInfoCollapsed(collapsed) {
  viewer.classList.toggle("info-collapsed", collapsed);
  const b = $("#infoToggle");
  b.setAttribute("aria-expanded", String(!collapsed));
  b.title = collapsed ? "Show the details (I)" : "Hide the details (I)";
  pref.set("infoCollapsed", collapsed ? "1" : "");
  // The photo fits the room it has now.
  if (viewerIndex >= 0) {
    resetZoom();
    if (rotation) previewRotation();
    else fitViewerImage(photos[viewerIndex].width, photos[viewerIndex].height);
  }
}
$("#infoToggle").addEventListener("click", e => { e.stopPropagation(); setInfoCollapsed(!viewer.classList.contains("info-collapsed")); });
if (pref.get("infoCollapsed") === "1") setInfoCollapsed(true);
$(".info", viewer).addEventListener("change", e => { if (e.target.id === "showBoxes") viewer.classList.toggle("boxes", e.target.checked); });
$(".info", viewer).addEventListener("click", async e => {
  const link = e.target.closest("a[target=_blank]");
  if (link && folderInfo.desktop) {
    e.preventDefault();
    return post("/api/open", { url: link.href }).catch(err => toast(`Couldn't open the map: ${err.message}`, true));
  }
  const act = e.target.closest(".photo-acts button");
  if (act?.dataset.sharePhoto) return sharePhoto(+act.dataset.sharePhoto, act);
  if (act?.dataset.reveal) return revealPhoto(+act.dataset.reveal);
  if (act?.dataset.rotate) return rotateViewer(+act.dataset.rotate);
  if (e.target.dataset.assign) {
    return openAssign(+e.target.dataset.assign, e.target.dataset.current ? +e.target.dataset.current : null);
  }
  if (e.target.dataset.nameFace) return startViewerRename(e.target);
  if (e.target.dataset.cover) {
    e.target.disabled = true;
    try {
      const { person } = await post(`/api/faces/${e.target.dataset.cover}/cover`);
      const p = personById(person);
      toast(p ? `This face is now on ${personName(p)}'s card` : "Card photo changed");
    } catch (err) {
      e.target.disabled = false;
      return toast(`Couldn't change the card photo: ${err.message}`, true);
    }
    await loadMeta();
    refreshPeopleViews({ animate: false });
    return openViewer(viewerIndex);
  }
  if (e.target.dataset.reject) {
    e.target.disabled = true;
    try {
      const { person } = await post(`/api/faces/${e.target.dataset.reject}/reject`);
      toast(`Moved to a new group, Unnamed #${person}. Rename or hide it in People.`);
    } catch (err) {
      toast(`Couldn't move the face: ${err.message}`, true);
    }
    await loadMeta();
    openViewer(viewerIndex);
  } else if (e.target.dataset.person) {
    state.people = new Set([+e.target.dataset.person]);
    state.view = "photos"; closeViewer(); renderPeopleList(); render();
  }
});
document.addEventListener("keydown", e => {
  // Keys go to fields being typed in (a name), not to checkboxes, which only take Space.
  const typing = e.target.matches('input:not([type="checkbox"]):not([type="range"]), textarea, select');
  if (!viewer.classList.contains("open") || typing) return;
  if (document.querySelector("dialog[open]")) return;
  if (e.key === "Escape") closeViewer();
  else if (e.key === "Delete") deleteViewerPhoto();
  else if (e.key === "r" || e.key === "R") { if ($("#viewer [data-rotate]")) rotateViewer(e.shiftKey ? -1 : 1); }
  else if (e.key === "i" || e.key === "I") setInfoCollapsed(!viewer.classList.contains("info-collapsed"));
  else if (e.key === "ArrowLeft") openViewer(viewerIndex - 1);
  else if (e.key === "ArrowRight") openViewer(viewerIndex + 1);
});

// ---- folders -----------------------------------------------------------------------
let folderInfo = { folders: [], desktop: false };
async function loadFolders() {
  folderInfo = await api("/api/folders");
  return folderInfo;
}
/** Picks a folder on the server by browsing its folders (the browser has no picker for
 *  those). Resolves to true once one was added. */
function browseForFolder() {
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
async function addFolder(path) {
  const added = await post(path == null ? "/api/folders/pick" : "/api/folders", path == null ? {} : { path });
  if (added === null) return false; // picker cancelled
  await loadFolders();
  setTimeout(pollStatus, 300);
  return true;
}
// ---- settings: photo folders and indexing -------------------------------------------
let lastStatus = null;
function scanStateHtml(st) {
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
function excludedHtml(st) {
  const n = st?.excluded ?? 0;
  if (!n) return "";
  return `<div class="scan-state"><div class="what">${plural(n, "photo")} removed from the gallery
      <small>Their files are still on disk. Showing them again indexes them at the next scan.</small></div>
    <button class="btn" data-show-excluded>Show again</button></div>`;
}

// Files that could not be indexed, with the reason.
let failureCount = null, failureList = [];
async function loadFailures() {
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
function removeProgressHtml(st) {
  if (!st?.remove_total) {
    return `<div>${st?.running ? "Stopping the scan first…" : "Removing its photos…"}</div><progress></progress>`;
  }
  return `<div>Removing ${st.remove_done.toLocaleString()} of ${st.remove_total.toLocaleString()} photos</div>
    <progress max="${st.remove_total}" value="${st.remove_done}"></progress>`;
}
function renderSettings(error = "") {
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
function openSettings() {
  renderSettings();
  // Fresh status for the Library part (it is otherwise polled only every few seconds).
  api("/api/status").then(st => {
    lastStatus = st;
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
      lastStatus = { ...lastStatus, removing: path, remove_done: 0, remove_total: 0 };
      renderSettings();
      const watch = setInterval(async () => {
        const st = await api("/api/status").catch(() => null);
        if (!st?.removing) return;
        lastStatus = st;
        const el = $("#removeProgress");
        if (el) el.innerHTML = removeProgressHtml(st);
        updateIndexPill(st);
      }, 500);
      let r;
      try { r = await post("/api/folders/remove", { path }); }
      finally {
        clearInterval(watch);
        lastStatus = { ...lastStatus, removing: null };
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

// ---- scan status -----------------------------------------------------------------
let wasRunning = false;
// ---- indexing progress outside Settings ---------------------------------------------
let firstPhotosCheck = 0; // last look for the first photos while the library is still empty
let indexedAtRender = 0; // photos indexed (this scan) when the Photos view was last drawn
let pillHiddenUntilIdle = false, pillDoneTimer = 0, pillWasBusy = false;
/** What the library is doing, for the card and the first-index screen; null when idle or
 *  only checking for changes (that is quick and needs no attention). */
function indexWork(s) {
  if (s?.removing) {
    const name = s.removing.split(/[\\/]/).filter(Boolean).pop() || s.removing;
    return s.remove_total
      ? { title: `Removing ${s.remove_done.toLocaleString()} of ${s.remove_total.toLocaleString()} photos`, detail: `Folder ${name}`, done: s.remove_done, total: s.remove_total }
      : { title: "Removing a folder…", detail: `Folder ${name}` };
  }
  if (!s?.running) return null;
  if (s.phase === "indexing photos" && s.total > 0) {
    return { title: `Indexing ${s.done.toLocaleString()} of ${s.total.toLocaleString()} photos`,
      detail: s.faces ? `${s.faces.toLocaleString()} faces found` : "Reading photos, making thumbnails, finding faces",
      done: s.done, total: s.total };
  }
  if (s.phase === "grouping faces" && s.group_total > 0) {
    return { title: `Grouping faces ${Math.min(100, Math.floor((100 * s.group_done) / s.group_total))}%`,
      detail: "Placing faces into people", done: s.group_done, total: s.group_total };
  }
  // Listing the files of a large library takes a while: say so when nothing is shown yet.
  if (s.phase === "listing files" && !shownPhotoCount) return { title: "Looking for photos…", detail: "Listing the files in your folders" };
  return null;
}
function firstIndexHtml(s) {
  const w = indexWork(s) ?? { title: "Getting ready…", detail: "" };
  return `<h2>Indexing your photos</h2>
    <p>Photos appear here as they are indexed. You can close the app; it continues where it left off.</p>
    <progress ${w.total ? `max="${w.total}" value="${w.done}"` : ""}></progress>
    <div class="n">${esc(w.title)}${w.detail ? ` · ${esc(w.detail)}` : ""}</div>`;
}
function updateIndexPill(s) {
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
    const fresh = s.phase === "indexing photos" ? s.done - Math.min(indexedAtRender, s.done) : 0;
    const more = $("[data-pill-new]", pill);
    more.hidden = !(fresh > 0 && shownPhotoCount > 0 && state.view === "photos");
    more.textContent = `Show ${fresh.toLocaleString()} new`;
  } else if (pillWasBusy && !s.running && !s.removing) {
    pillWasBusy = false;
    if (pill.hidden) return;
    pill.classList.add("done");
    $(".t b", pill).textContent = "Your library is up to date";
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
async function pollStatus() {
  let s;
  try { s = await api("/api/status"); } catch {
    // Otherwise the page just stops changing when the program is closed or the network drops.
    if (++pollFailures === 3) toast("Can't reach Imadive. Is it still running?", true);
    return setTimeout(pollStatus, 5000);
  }
  if (pollFailures >= 3) toast("Connected to Imadive again");
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
  else if (s.running && s.done > 0 && !shownPhotoCount && state.view === "photos" && !viewer.classList.contains("open")
    && Date.now() - firstPhotosCheck > 5000) {
    firstPhotosCheck = Date.now();
    render();
  }
  wasRunning = s.running;
  setTimeout(pollStatus, s.running || s.removing ? 1500 : 10000);
}

// ---- the Back button ------------------------------------------------------------------
// The page keeps one entry of its own above the one it was opened with. Back lands on the
// lower one; the app then undoes one step (closes what is open, clears the filters, goes
// back to Photos) and puts its entry back. With nothing left to undo, it asks before
// leaving. Browsers only honour an entry added after the user touched the page, so it is
// added on the first touch or key press.
const hasFilters = () => state.people.size > 0 || state.place != null || !!state.date || !!state.from || !!state.to;
function pushAppEntry() {
  saveHash(); // the address of what is on screen now
  history.pushState({ imadive: "top" }, "", location.href);
}
history.replaceState({ imadive: "base" }, "", location.href);
const armBack = () => {
  removeEventListener("pointerdown", armBack, true);
  removeEventListener("keydown", armBack, true);
  if (history.state?.imadive === "base") pushAppEntry();
};
addEventListener("pointerdown", armBack, true);
addEventListener("keydown", armBack, true);
let leaving = false;
addEventListener("popstate", async e => {
  if (leaving || e.state?.imadive !== "base") return;
  const undone = await undoOneStep();
  // The desktop app has no page to go back to (a mouse's back button, say).
  if (undone || folderInfo.desktop) return pushAppEntry();
  const choice = await askChoice(`<p>Close the gallery and go back to the previous page?</p>
    <div class="dlg-actions"><button class="btn" data-choice="cancel">Stay</button><button class="btn primary" data-choice="leave">Leave</button></div>`,
    "Leave Imadive?");
  if (choice !== "leave") return pushAppEntry();
  leaving = true;
  history.back();
  // Nothing to go back to (the gallery was opened in a new tab or as an app): close it if allowed.
  setTimeout(() => window.close(), 400);
});
/** Undoes the most recent step a Back press stands for; false when there is none. */
async function undoOneStep() {
  const dialog = [...document.querySelectorAll("dialog[open]")].pop();
  if (dialog) { dialog.close(); return true; }
  if (viewer.classList.contains("open")) { closeViewer(); return true; }
  if (document.body.classList.contains("drawer-open") || document.body.classList.contains("view-open")) {
    setDrawer(false); setViewPanel(false); return true;
  }
  if ((state.view === "photos" || state.view === "upcoming") && hasFilters()) {
    Object.assign(state, { place: null, date: null, from: null, to: null });
    state.people.clear();
    syncRangeInputs();
    renderPeopleList();
    render();
    return true;
  }
  if (state.view !== "photos") { state.view = "photos"; render(); return true; }
  return false;
}

loadHash();
syncRangeInputs();
Promise.all([loadMeta(), loadFolders()]).then(render);
pollStatus();
