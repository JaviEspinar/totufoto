// The People in the photo column, and the panels that replace it on small screens.
// One of the page's modules; main.js starts the page.
import { num, t, tn } from "./i18n.js";
import { $, onPeopleLoaded, people, personById, personName, pref, saveHash, state } from "./core.js";
import { nameCollator, peopleAlphabetical, renamePerson } from "./people.js";
import { render } from "./views.js";
import { viewer } from "./viewer.js";

// ---- sidebar ---------------------------------------------------------------
// People keep their place on screen while you work: renaming someone (named people sort
// first) or refreshing counts must not reshuffle the list under you. The order is taken
// fresh from the server when the People tab is opened; new people are added at the end.
export let peopleOrder = [], orderedCache = null;
// The People tab's sort of named people: "count" (most photos first) or "name" (A to Z).
// People without a name always go by their number of photos.
export let peopleSort = "count";
if (pref.get("peopleSort") === "name") peopleSort = "name";
export function setPeopleSort(sort) {
  peopleSort = sort;
  pref.set("peopleSort", sort);
}
const byCount = (a, b) => b.count - a.count
  || (!a.name !== !b.name ? (a.name ? -1 : 1) : a.name ? nameCollator.compare(a.name, b.name) : 0)
  || a.id - b.id;
function sortedPeople() {
  if (peopleSort === "name") return [...peopleAlphabetical().filter(p => p.name), ...people.filter(p => !p.name).sort(byCount)];
  return [...people].sort(byCount);
}
export function resetPeopleOrder() { restorePeopleOrder(sortedPeople().map(p => p.id)); }
/** Puts back an order saved before (after a change that failed, say). */
export function restorePeopleOrder(order) { peopleOrder = order; orderedCache = null; }
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
export function matchPeople(query, list = orderedPeople()) {
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
    row.innerHTML = `<input type="checkbox" data-id="${p.id}" aria-label="${t("Select")}">
      <img loading="lazy" alt="" title="${t("Select")}"><span class="n" title="${t("Click to rename")}" tabindex="0" role="button"></span><span class="c"></span>`;
    sidebarRows.set(p.id, row);
  }
  const img = row.querySelector("img"), name = row.querySelector(".n");
  if (img.dataset.face !== String(p.face)) { img.dataset.face = p.face; img.src = `/face/${p.face}`; }
  name.textContent = personName(p);
  name.classList.toggle("unnamed", !p.name);
  name.setAttribute("aria-label", t("Rename {name}", { name: personName(p) }));
  row.querySelector("input").setAttribute("aria-label", t("Select {name}", { name: personName(p) }));
  row.querySelector(".c").textContent = p.count;
  row.querySelector("input").checked = state.people.has(p.id);
  return row;
}
// The column can be collapsed to a thin strip; remembered in this browser.
function setAsideCollapsed(collapsed) {
  document.body.classList.toggle("aside-collapsed", collapsed);
  const b = $("#asideToggle");
  b.setAttribute("aria-expanded", String(!collapsed));
  b.title = collapsed ? t("Show the people column") : t("Hide this column");
  pref.set("asideCollapsed", collapsed ? "1" : "");
}
const isPhone = () => matchMedia("(max-width: 639px)").matches;
// On phones the column is a panel that slides in; its button closes it.
$("#asideToggle").addEventListener("click", () => isPhone() ? setDrawer(false) : setAsideCollapsed(!document.body.classList.contains("aside-collapsed")));

// ---- small screens: the people panel and the View panel --------------------------------
export function setDrawer(open) {
  document.body.classList.toggle("drawer-open", open);
  $("#peopleBtn").setAttribute("aria-expanded", String(open));
  if (open) setViewPanel(false);
}
export function setViewPanel(open) {
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
  if (e.key !== "Escape" || e.defaultPrevented || document.querySelector("dialog[open]") || viewer.classList.contains("open")) return;
  setDrawer(false); setViewPanel(false);
});
// The top bar's height (two rows on phones), for placing the View panel under it.
new ResizeObserver(([entry]) => document.documentElement.style.setProperty("--header-h", `${entry.target.offsetHeight}px`)).observe($("header"));
if (pref.get("asideCollapsed") === "1") setAsideCollapsed(true);

let sidebarEditing = false;
/** How many people are selected, on the collapsed column's strip. */
export function updateRailCount() {
  const btnCount = $("#peopleBtnCount");
  btnCount.hidden = !state.people.size;
  btnCount.textContent = state.people.size;
  $("#viewBtnDot").hidden = !(state.from || state.to);
  const railCount = $("#railCount");
  railCount.hidden = !state.people.size;
  railCount.textContent = state.people.size;
  railCount.title = tn(state.people.size, "{n} person selected", "{n} people selected");
}
export function renderPeopleList() {
  updateRailCount();
  const list = $("#peopleList");
  if (sidebarEditing) return; // redrawn when the edit ends
  const all = orderedPeople();
  const visible = all.filter(p => !p.hidden);
  $("#peopleFilter").hidden = visible.length <= 12;
  for (const b of $("#match").children) b.classList.toggle("on", b.dataset.m === state.match);
  if (!visible.length) {
    list.innerHTML = `<div class="empty-side">${t("No people yet. Faces are grouped automatically while the library is indexed.")}</div>`;
    return;
  }
  const matches = matchPeople($("#peopleFilter").value, visible);
  const rest = matches.filter(p => !state.people.has(p.id));
  const shown = [...visible.filter(p => state.people.has(p.id)), ...rest.slice(0, SIDEBAR_LIMIT)];
  const hiddenCount = rest.length - SIDEBAR_LIMIT;
  const extra = document.createElement("div");
  extra.innerHTML = (hiddenCount > 0 ? `<div class="more">${t("{n} more. Search to find them.", { n: num(hiddenCount) })}</div>` : "") +
    (!matches.length ? `<div class="empty-side">${t("Nobody matches.")}</div>` : "");
  const rows = shown.map(sidebarRow);
  // Only touch the list when its rows changed; moving existing rows keeps their pictures.
  const current = [...list.children];
  const same = current.length === rows.length + extra.childElementCount && rows.every((r, i) => current[i] === r) &&
    extra.innerHTML === current.slice(rows.length).map(e => e.outerHTML).join("");
  if (!same) list.replaceChildren(...rows, ...extra.children);
  const keep = new Set(shown.map(p => p.id));
  for (const id of sidebarRows.keys()) if (!keep.has(id) && sidebarRows.size > 600) sidebarRows.delete(id);
}
onPeopleLoaded(renderPeopleList);
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
  input.placeholder = t("Add a name");
  input.setAttribute("aria-label", t("Name"));
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
