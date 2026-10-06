// render(): one job per view; and the clicks in the main area.
// One of the page's modules; main.js starts the page.
import { t } from "./i18n.js";
import { $, esc, saveHash, state } from "./core.js";
import { renderPeopleList, setDrawer, updateRailCount } from "./sidebar.js";
import { items, onChipClick, rememberCards, renderPhotos, renderUpcoming } from "./items.js";
import { renderOptimization } from "./optimization.js";
import { openMerge, renamePerson, renderPeople, toggleHidden, unmountPeopleGrid } from "./people.js";
import { openViewer } from "./viewer.js";

let renderToken = 0;
/** The render job of the view on screen: { token, signal, alive() } (see render()). */
export let currentJob = { token: 0, signal: undefined, alive: () => true };
let viewRequest = null; // the current view's request, cancelled when another view replaces it
export async function render() {
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
  unmountPeopleGrid();
  main.scrollTop = 0;
  try {
    if (state.view === "photos") await renderPhotos(main, job);
    else if (state.view === "upcoming") await renderUpcoming(main, job);
    else if (state.view === "optimization") await renderOptimization(main, job);
    else await renderPeople(main, job);
  } catch (err) {
    if (err.name === "AbortError" || !job.alive()) return;
    main.innerHTML = `<div class="blank">${t("Something went wrong: {error}", { error: esc(err.message) })}
      <p><button class="btn" data-rerender>${t("Try again")}</button></p></div>`;
  }
}

// ---- main area events ---------------------------------------------------------
$("#main").addEventListener("click", e => {
  if (e.target.closest("[data-rerender]")) return render();
  if (e.target.dataset.clear) return onChipClick(e);
  const tile = e.target.closest(".tile");
  if (tile) { e.preventDefault(); return openViewer(items.findIndex(p => p.id === +tile.dataset.id)); }
  const group = e.target.closest("[data-group]");
  if (group) {
    rememberCards();
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
export function syncRangeInputs() {
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
