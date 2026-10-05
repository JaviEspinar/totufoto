// The Photos and Upcoming views: the justified grid, group cards, and progressive loading.
// One of the page's modules; main.js starts the page.
import { lang, num, t, tn } from "./i18n.js";
import { $, api, dateLabel, esc, fmtDate, fmtDay, fmtMonth, personById, personName, photoQuery, placeById, placeLabel, plural, regionName, state } from "./core.js";
import { renderPeopleList } from "./sidebar.js";
import { toast } from "./people.js";
import { currentJob, render, syncRangeInputs } from "./views.js";
import { addFolder, browseForFolder, folderInfo } from "./settings.js";
import { firstIndexHtml, lastStatus } from "./status.js";

// ---- photo grid ------------------------------------------------------------
/** A photo as /api/photos sends it (an array, to keep large libraries small), with names. */
const photoRow = ([id, width, height, taken, place, version]) => ({ id, width, height, taken, place, version });

function groupKey(p, mode = state.groupBy) {
  const taken = p.taken;
  switch (mode) {
    case "day": return taken.slice(0, 10);
    case "month": return taken.slice(0, 7);
    case "year": return taken.slice(0, 4);
    case "place": return "p" + (p.place ?? "");
    default: return "";
  }
}
function groupTitle(key, sample, mode = state.groupBy) {
  switch (mode) {
    case "day": return fmtDay(key);
    case "month": return fmtMonth(key);
    case "year": return key;
    case "place": return sample.place == null ? t("No location") : placeLabel(sample.place);
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
// Loaded or failed, a thumbnail fades in. (Load events don't bubble, so this listens on the
// way down.)
for (const type of ["load", "error"]) {
  document.addEventListener(type, e => { if (e.target.matches?.("img[data-thumb]")) thumbLoaded(e.target); }, true);
}
/** Picture URLs with the photo's version: a rotated photo gets new ones, so browsers don't
 *  show the old (cached) picture. */
/** A thumbnail's address, with the photo's version (a new version is a new address). */
export const thumbUrl = (id, v = 0) => `/thumb/${id}/${v}`;
export const originalUrl = (id, v) => v ? `/original/${id}?v=${v}` : `/original/${id}`;
export const versionOf = id => photos.find(p => p.id === id)?.version ?? 0;
const tile = p => `<a class="tile" data-id="${p.id}" style="--r:${(p.width / p.height).toFixed(3)}" href="${originalUrl(p.id, p.version)}">` +
  (loadedThumbs.has(String(p.id))
    ? `<img src="${thumbUrl(p.id, p.version)}" loading="lazy" alt="" data-thumb="${p.id}" class="ok"></a>`
    : `<img src="${thumbUrl(p.id, p.version)}" loading="lazy" decoding="async" alt="" data-thumb="${p.id}"></a>`);

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
  if (state.from && state.to) return t("{from} to {to}", { from: fmtDate(state.from), to: fmtDate(state.to) });
  return state.from ? t("Since {date}", { date: fmtDate(state.from) }) : t("Until {date}", { date: fmtDate(state.to) });
}
/** Chips for the active filters. */
function filterChips() {
  const chip = (label, key, person = "") =>
    `<span class="chip"${person ? ` data-person-chip="${person}"` : ""}><span class="label">${esc(label)}</span>` +
    `<button data-clear="${key}" title="${t("Remove filter")}" aria-label="${t("Remove {name}", { name: esc(label) })}">×</button></span>`;
  const selected = [...state.people].map(id => personById(id)).filter(Boolean);
  const parts = [];
  // How the people combine, when it matters.
  if (selected.length > 1 || (selected.length && state.match === "only"))
    parts.push(`<span class="chip-mode">${{ all: t("Together:"), any: t("Any of:"), only: t("Only:") }[state.match]}</span>`);
  for (const p of selected) parts.push(chip(personName(p), `person:${p.id}`, p.id));
  let count = selected.length;
  if (state.place != null) { parts.push(chip(placeLabel(state.place), "place")); count++; }
  if (state.date) { parts.push(chip(dateLabel(state.date), "date")); count++; }
  if (state.from || state.to) { parts.push(chip(rangeLabel(), "range")); count++; }
  if (count) parts.push(`<button class="clear-all" data-clear="all">${t("Clear all")}</button>`);
  return `<div class="filters">${parts.join("")}</div>`;
}
export function onChipClick(e) {
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

/** The photos of the view on screen, in order (the viewer moves through them). */
export let photos = [];
export let shownPhotoCount = 0; // photos the Photos tab shows (as cards or tiles)
export let indexedAtRender = 0; // photos indexed (this scan) when the Photos view was last drawn
/** A photo left the view (deleted, or removed from the gallery). */
export function onePhotoLess() { shownPhotoCount = Math.max(0, shownPhotoCount - 1); }
export async function renderPhotos(main, job) {
  indexedAtRender = lastStatus?.running ? lastStatus.done : 0;
  const shape = photosShape();
  const load = beginViewLoad(main, "photos", filterChips() + `<div class="count">${t("Loading photos…")}</div>`,
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
  const count = load.head.querySelector(".count");
  count.textContent = plural(shownPhotoCount, "photo");
  count.dataset.photos = ""; // a plain photo count, which removing a photo updates
  if (!shownPhotoCount) {
    const filtered = state.people.size || state.place != null || state.date || state.from || state.to;
    if (!filtered && !folderInfo.folders.length) {
      main.innerHTML = `<div class="welcome"><h2>${t("Welcome to Imadive")}</h2>
        <p>${t("Add a folder with photos. It is indexed in the background, and subfolders are included.")}</p>
        <button class="btn primary" id="welcomeAdd">${t("Add folder…")}</button></div>`;
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
    const msg = filtered ? t("No photos match these filters.") : t("No photos yet. They appear here while the folders are indexed.");
    load.area.innerHTML = `<div class="blank">${msg}</div>`;
    return;
  }
  if (shape.cards) {
    const groups = data.groups;
    const n = groups.length;
    const groupCount = { year: () => plural(n, "year"), month: () => plural(n, "month"), day: () => plural(n, "day"), place: () => plural(n, "place") }[shape.cards]();
    delete count.dataset.photos;
    count.textContent = `${groupCount}, ${plural(shownPhotoCount, "photo")}`;
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
/** Opening a group: the cards view to come back to, and where it was scrolled. */
export function rememberCards() { cardsReturn = { key: cardsViewKey(), scroll: $("#main").scrollTop }; }
/** Cards from the server's group summaries: `{ key, count, cover }` per group. */
function renderGroupCards(container, groups, mode) {
  const title = key => mode === "place" ? (key === 0 ? t("No location") : placeById.get(key)?.city ?? t("Unknown place"))
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

export async function renderUpcoming(main, job) {
  const opts = [7, 14, 30, 60, 90].map(d => `<button data-d="${d}" class="${d === state.upcoming ? "on" : ""}">${t("{n} days", { n: d })}</button>`).join("");
  const load = beginViewLoad(main, "upcoming",
    `<div class="upbar">${t("Memories from past years for the next")} <div class="seg" id="days">${opts}</div></div>` + filterChips(),
    photoPlaceholders, job);
  $("#days").addEventListener("click", e => { if (e.target.dataset.d) { state.upcoming = +e.target.dataset.d; render(); } });
  const data = await api(photoQuery({ upcoming: state.upcoming }), { signal: job.signal });
  if (!job.alive()) return;
  load.finish();
  const order = new Map(data.days.map((d, i) => [d, i]));
  // Upcoming days first (today, tomorrow...), and within a day the most recent year first.
  photos = data.photos.map(photoRow).sort((a, b) =>
    order.get(a.taken.slice(5, 10)) - order.get(b.taken.slice(5, 10)) || b.taken.localeCompare(a.taken));
  if (!photos.length) { load.area.innerHTML = `<div class="blank">${t("No photos were taken on these dates in previous years.")}</div>`; return; }
  const thisYear = new Date().getFullYear();
  const groups = groupPhotos(photos, p => p.taken.slice(5, 10));
  const header = g => {
    const idx = order.get(g.key);
    const [m, d] = g.key.split("-").map(Number);
    const label = new Date(thisYear, m - 1, d).toLocaleDateString(lang, { weekday: "long", month: "long", day: "numeric" });
    const when = idx === 0 ? t("Today") : idx === 1 ? t("Tomorrow") : t("In {n} days", { n: idx });
    return `<h2>${when} · ${esc(label)}<small>${g.items.length}</small></h2>`;
  };
  const sub = (p, prev) => {
    if (prev && prev.taken.slice(0, 4) === p.taken.slice(0, 4)) return null;
    const y = +p.taken.slice(0, 4), ago = thisYear - y;
    return `${tn(ago, "{n} year ago", "{n} years ago")} · ${y}`;
  };
  renderGroups(load.area, groups, header, sub);
}
