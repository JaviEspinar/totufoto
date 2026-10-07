// Shared helpers, the view state and the address it lives in, and loading people and places.
// One of the page's modules; main.js starts the page.
import { lang, num, t, tn, translateMessage, translatePage } from "./i18n.js";

translatePage();

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
export const $ = (s, el = document) => el.querySelector(s);
/** An icon from the page's sprite (index.html); `mirrored` flips it left to right. */
export const icon = (name, size = 15, mirrored = false) =>
  `<svg class="icon" viewBox="0 0 24 24" width="${size}" height="${size}" aria-hidden="true"><use href="#i-${name}"${mirrored ? ' transform="matrix(-1 0 0 1 24 0)"' : ""}/></svg>`;
/** "1 photo", "2,345 photos", in the page's language: the dictionary has "{n} photo" and
 *  "{n} photos" (web/tests/strings.mjs checks every word used here has both). */
export const plural = (n, word) => tn(n, `{n} ${word}`, `{n} ${word}s`);
/** Preferences remembered in this browser (`imadive.<key>`). Storage may be unavailable
 *  (private windows, blocked cookies): then nothing is remembered. */
export const pref = {
  get(key) { try { return localStorage.getItem(`imadive.${key}`); } catch { return null; } },
  set(key, value) { try { localStorage.setItem(`imadive.${key}`, value); } catch {} },
};
/** A size in bytes, in the page's language: "820 KB", "1.4 GB". */
export const fmtBytes = n => n < 1024 ? `${num(n)} B` : n < 1048576 ? `${num(Math.round(n / 1024))} KB`
  : n < 1073741824 ? `${num(+(n / 1048576).toFixed(1))} MB` : `${num(+(n / 1073741824).toFixed(2))} GB`;
export const esc = s => String(s ?? "").replace(/[&<>"']/g, c => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const regionName = (() => {
  try { const d = new Intl.DisplayNames([lang], { type: "region" }); return cc => { try { return d.of(cc); } catch { return cc; } }; }
  catch { return cc => cc; }
})();
/** The local day of "YYYY-MM-DD..." (capture times are local, without a time zone). */
const parseDay = t => new Date(+t.slice(0, 4), +t.slice(5, 7) - 1, +t.slice(8, 10));
/** "Thu, December 26, 2024" */
export const fmtDay = d => parseDay(d).toLocaleDateString(lang, { weekday: "short", year: "numeric", month: "long", day: "numeric" });
/** "Dec 26, 2024" */
export const fmtDate = d => parseDay(d).toLocaleDateString(lang, { day: "numeric", month: "short", year: "numeric" });
export const fmtFull = d => fmtDay(d) + " · " + d.slice(11, 16);
/** "October 2024" (or "octubre de 2024") for "2024-10". */
export const fmtMonth = d => new Date(+d.slice(0, 4), +d.slice(5, 7) - 1, 1).toLocaleDateString(lang, { month: "long", year: "numeric" });

export const state = {
  view: "photos", sort: "desc", groupBy: "month", match: "all",
  people: new Set(), place: null, date: null, upcoming: 30,
  from: null, to: null, // capture date range, YYYY-MM-DD, both included
};
export let people = [], peopleById = new Map(), places = [], placeById = new Map();
/** Replaces the people list, and the lookup by id that goes with it. */
export function setPeople(list) {
  people = list;
  peopleById = new Map(list.map(p => [p.id, p]));
}
export const personById = id => peopleById.get(id);

// ---- URL state -------------------------------------------------------------
export function saveHash() {
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
export function loadHash() {
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
export const api = async (url, opts) => {
  const r = await fetch(url, opts);
  if (!r.ok) {
    const text = await r.text();
    let body = null;
    try { body = JSON.parse(text); } catch {}
    throw new ApiError(r.status, translateMessage(body?.error || text || String(r.status)), body);
  }
  return r.status === 204 || r.status === 202 ? null : r.json();
};
/** Photo details already fetched (promises, by "id:version"), so moving through the viewer
 *  is instant. Anything sent to the server may change them (a face moved, people merged), so
 *  every POST empties it, and so does every reload of the people (a scan may regroup them). */
const details = new Map();
export function itemDetail(id, version) {
  const key = `${id}:${version}`;
  if (!details.has(key)) {
    if (details.size >= 200) details.delete(details.keys().next().value);
    details.set(key, api(`/api/items/${id}`).catch(err => { details.delete(key); throw err; }));
  }
  return details.get(key);
}
/** A request that changes something (POST, PATCH, DELETE), with a JSON body. */
const send = (method, url, body) => {
  details.clear();
  return api(url, { method, headers: { "content-type": "application/json" }, body: JSON.stringify(body ?? {}) });
};
export const post = (url, body) => send("POST", url, body);
export const patch = (url, body) => send("PATCH", url, body);
/** DELETE: what to delete goes in the address (`params`), not in a body. */
export const del = (url, params) => {
  details.clear();
  return api(params ? `${url}?${new URLSearchParams(params)}` : url, { method: "DELETE" });
};

export async function loadMeta() {
  let list;
  [list, places] = await Promise.all([api("/api/people"), api("/api/places")]);
  details.clear();
  setPeople(list);
  placeById = new Map(places.map(p => [p.id, p]));
  const known = new Set(people.map(p => p.id));
  for (const id of state.people) if (!known.has(id)) state.people.delete(id);
  for (const fn of afterPeopleLoad) fn();
}
/** What other parts of the page do once the people are loaded again (in the order they
 *  were added). This file uses nothing from the others, so it can run first. */
const afterPeopleLoad = [];
export const onPeopleLoaded = fn => afterPeopleLoad.push(fn);
export const personName = p => p.name || t("Unnamed #{id}", { id: p.id });
export const placeLabel = id => {
  if (id === 0) return t("No location");
  const p = placeById.get(id);
  return p ? `${p.city}, ${regionName(p.country)}` : t("Unknown place");
};
/** "2024", "October 2024" or a full day, for a YYYY / YYYY-MM / YYYY-MM-DD date filter. */
export const dateLabel = d => d.length === 4 ? d : d.length === 7 ? fmtMonth(d) : fmtDay(d);

export function itemQuery(extra = {}) {
  const q = new URLSearchParams({ sort: state.sort, ...extra });
  if (state.people.size) { q.set("people", [...state.people].join(",")); q.set("match", state.match); }
  if (state.place != null) q.set("place", state.place);
  if (state.date) q.set("date", state.date);
  if (state.from) q.set("from", state.from);
  if (state.to) q.set("to", state.to);
  return "/api/items?" + q;
}
