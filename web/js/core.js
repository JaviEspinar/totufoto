"use strict";
// Shared helpers, the view state and the address it lives in, and loading people and places.
// Part of the page's script, split by area; see web/index.html for the order.

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
/** Photo details already fetched (promises, by "id:version"), so moving through the viewer
 *  is instant. Anything sent to the server may change them (a face moved, people merged), so
 *  every POST empties it, and so does every reload of the people (a scan may regroup them). */
const details = new Map();
function photoDetail(id, version) {
  const key = `${id}:${version}`;
  if (!details.has(key)) {
    if (details.size >= 200) details.delete(details.keys().next().value);
    details.set(key, api(`/api/photos/${id}`).catch(err => { details.delete(key); throw err; }));
  }
  return details.get(key);
}
const post = (url, body) => {
  details.clear();
  return api(url, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) });
};

async function loadMeta() {
  let list;
  [list, places] = await Promise.all([api("/api/people"), api("/api/places")]);
  details.clear();
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
