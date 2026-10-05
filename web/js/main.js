// The Back button, and startup. The page's entry point: the modules it imports (and
// theirs) run first, so everything it starts is ready.
import { loadHash, loadMeta, saveHash, state } from "./core.js";
import { renderPeopleList, setDrawer, setViewPanel } from "./sidebar.js";
import { render, syncRangeInputs } from "./views.js";
import { askChoice, closeViewer, viewer } from "./viewer.js";
import { folderInfo, loadFolders } from "./settings.js";
import { pollStatus } from "./status.js";

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
