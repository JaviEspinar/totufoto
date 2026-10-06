// The photo viewer: details, faces, sharing, rotating, deleting, zoom and swipe.
// One of the page's modules; main.js starts the page.
import { t } from "./i18n.js";
import { $, api, del, esc, fmtFull, icon, itemDetail, loadMeta, personById, personName, post, pref, regionName, state } from "./core.js";
import { renderPeopleList } from "./sidebar.js";
import { VIDEO_PICTURE, fmtDuration, isVideo, items, mediaCount, oneItemLess, originalUrl, shownItemCount, silentUrl, thumbUrl, versionOf } from "./items.js";
import { makeVideoThumb } from "./videothumbs.js";
import { openAssign, refreshPeopleViews, renamePerson, toast } from "./people.js";
import { render } from "./views.js";
import { folderInfo } from "./settings.js";

// ---- viewer ----------------------------------------------------------------
export const viewer = $("#viewer");
/** The photo open in the viewer, as an index into `photos`; -1 when it is closed. */
export let viewerIndex = -1;
/** Replaces the details panel. Focus inside it would fall out of the viewer with the old
 *  content, so it moves to the button that folds the details, next to them. */
function setDetails(html) {
  const body = $(".info-body", viewer);
  const hadFocus = body.contains(document.activeElement);
  body.innerHTML = html;
  if (hadFocus) $("#infoToggle").focus();
}
/** Sizes the photo to its final on-screen size from its known dimensions, so the thumbnail
 *  shown first and the full photo that replaces it occupy exactly the same box. */
/** Sizes the viewer's photo to fit; `sideways` while it shows turned a quarter (rotating). */
function fitViewerImage(w, h, sideways = false) {
  const stage = $(".stage", viewer);
  // Wide screens keep room for the ‹ › buttons beside the photo; on phones they float over it.
  const maxW = stage.clientWidth - (stage.clientWidth < 640 ? 16 : 120), maxH = stage.clientHeight - (stage.clientWidth < 640 ? 16 : 40);
  const scale = sideways ? Math.min(maxW / h, maxH / w, 1) : Math.min(maxW / w, maxH / h, 1);
  for (const el of [$(".frame img", viewer), $(".frame video", viewer)]) {
    el.style.width = `${Math.max(1, Math.round(w * scale))}px`;
    el.style.height = `${Math.max(1, Math.round(h * scale))}px`;
  }
}
const videoEl = () => $(".frame video", viewer);
const showingVideo = () => viewerIndex >= 0 && isVideo(items[viewerIndex]);
/** Stops the video (and its download) when the viewer moves on or closes. */
function stopVideo() {
  const video = videoEl();
  if (!video.getAttribute("src")) return;
  video.pause();
  video.removeAttribute("src");
  video.load();
}
/** Plays video `i` with the browser's player; one it can't play offers its download. */
function showViewerVideo(i, id, w, h, v) {
  const img = $(".frame img", viewer), video = videoEl(), note = $(".video-note", viewer);
  img.hidden = true;
  img.removeAttribute("src");
  note.hidden = true;
  video.hidden = false;
  fitViewerImage(w, h);
  // Its thumbnail before it plays; the generic picture (and a thumbnail made now) without one.
  video.poster = thumbUrl(id, v);
  const probe = new Image();
  probe.onerror = () => {
    if (viewerIndex !== i) return;
    video.poster = VIDEO_PICTURE;
    makeVideoThumb(id, v);
  };
  probe.src = video.poster;
  video.src = originalUrl(id, v);
  // Plays at once: opening it was the click (or key) that browsers want before playing with
  // sound. If one refuses anyway (some phones), the player waits for a tap as before.
  video.play().catch(() => {});
  let silent = false;
  video.onerror = () => {
    if (viewerIndex !== i) return;
    if (!silent) {
      // Often it's the sound the browser can't decode (a broken first packet, in many phones'
      // videos): the picture alone, then.
      silent = true;
      video.src = silentUrl(id, v);
      video.play().catch(() => {});
      note.textContent = t("Playing without sound: its sound can't be played here.");
      note.hidden = false;
      return;
    }
    // The player would spin for ever: the video's picture instead, and the way to get it.
    stopVideo();
    video.hidden = true;
    img.onerror = () => { img.onerror = null; img.src = VIDEO_PICTURE; };
    img.src = thumbUrl(id, v);
    img.classList.add("generic");
    img.hidden = false;
    note.innerHTML = `${t("This video can't be played here.")} <a href="/original/${id}?download=1" download>${t("Download it")}</a>`;
    note.hidden = false;
    itemMissing(id); // says so if the file itself is gone
  };
}
window.addEventListener("resize", () => {
  if (viewerIndex < 0) return;
  resetZoom();
  if (rotation) previewRotation();
  else fitViewerImage(items[viewerIndex].width, items[viewerIndex].height);
});

/** Shows photo `i`: its thumbnail at the final size at once, then the full photo. */
async function showViewerImage(i, id, w, h, v) {
  stopVideo();
  if (isVideo(items[i])) return showViewerVideo(i, id, w, h, v);
  videoEl().hidden = true;
  $(".video-note", viewer).hidden = true;
  const img = $(".frame img", viewer);
  img.hidden = false;
  img.classList.remove("generic");
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
    .catch(() => { if (viewerIndex === i) itemMissing(id); });
  // Ready for the arrows: neighbours' thumbnails, and the next photo in full size.
  for (const j of [i - 1, i + 1]) if (items[j]) new Image().src = thumbUrl(items[j].id, items[j].version);
  if (items[i + 1]) {
    // A video is only fetched when it is played.
    if (!isVideo(items[i + 1])) new Image().src = originalUrl(items[i + 1].id, items[i + 1].version);
    itemDetail(items[i + 1].id, items[i + 1].version).catch(() => {});
  }
}
/** `keepImage`: only refresh the details and face boxes (the photo shown is already right). */
export async function openViewer(i, { keepImage = false } = {}) {
  if (i < 0 || i >= items.length) return;
  if (rotation && rotation.id !== items[i].id) flushRotation();
  viewerIndex = i;
  resetZoom();
  // No arrow where there is no photo to go to.
  $(".prev", viewer).hidden = i === 0;
  $(".next", viewer).hidden = i === items.length - 1;
  const { id, width: w, height: h, version: v } = items[i];
  viewer.classList.add("open");
  setViewerModal(true);
  if (!keepImage) await showViewerImage(i, id, w, h, v);
  if (viewerIndex !== i) return;
  let d;
  try { d = await itemDetail(id, v); }
  catch (err) {
    if (viewerIndex !== i) return;
    // Not the previous photo's details: say what happened.
    $(".boxes", viewer).innerHTML = "";
    setDetails(`<div class="s">${t("Couldn't load this photo's details: {error}", { error: esc(err.message) })}</div>`);
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
  setDetails(`
    <h3>${esc(fmtFull(d.taken))}</h3>
    <div class="s">${d.duration != null ? `${[t("Video"), fmtDuration(d.duration)].filter(Boolean).join(" · ")} · ` : ""}${d.dateFromExif ? "" : `${t("Date from file (no EXIF)")} · `}${d.width} × ${d.height}</div>
    <div class="item-acts">
      ${folderInfo.desktop && canShareFiles
        ? `<button data-share-item="${id}" title="${t("Send this photo with another app")}">${icon("share", 15)} ${t("Share")}</button>`
        : `<a class="btn-like" href="/original/${id}?download=1&v=${v}" download title="${t("Save the photo on this device")}">${icon("download", 15)} ${t("Download")}</a>`}
      ${d.rotatable ? `<button data-rotate="-1" title="${t("Rotate left (Shift+R)")}" aria-label="${t("Rotate left")}">${icon("rotate", 15)}</button><button data-rotate="1" title="${t("Rotate right (R)")}" aria-label="${t("Rotate right")}">${icon("rotate", 15, true)}</button>` : ""}
      ${folderInfo.desktop ? `<button data-reveal="${id}" title="${t("Show the file in its folder")}">${icon("folder", 15)} ${t("Open in folder")}</button>` : ""}
    </div>
    ${place ? `<h5>${t("Place")}</h5><div>${esc(place)}</div><div class="s"><a href="https://www.openstreetmap.org/?mlat=${d.lat}&mlon=${d.lon}#map=14/${d.lat}/${d.lon}" target="_blank" rel="noopener">${t("Open map")}</a></div>` : ""}
    ${d.duration != null ? "" : `<h5>${t("People ({n})", { n: d.faces.length })}</h5>
    ${d.faces.length ? d.faces.map(f => {
      const person = personById(f.person);
      return `<div class="vface"><img src="/face/${f.id}" alt="">
        <span class="who">${person?.name
          ? `<span class="n" data-person="${f.person}" title="${t("Show {name}'s photos", { name: esc(person.name) })}" tabindex="0" role="button">${esc(person.name)}</span>`
          : `<span class="n unnamed editable" data-name-face="${f.id}" data-current="${f.person ?? ""}" title="${t("Click to name this person")}" tabindex="0" role="button">${person ? esc(personName(person)) : t("Not grouped")}</span>`}
        <span class="acts">
          <button data-assign="${f.id}" data-current="${f.person ?? ""}" title="${t("Pick who this face is")}">${t("Same as…")}</button>
          ${person ? `<button data-reject="${f.id}" title="${t("This face is not {name}: move it to a new group you can rename or hide", { name: esc(personName(person)) })}">${t("Not them")}</button>` : ""}
          ${!person ? "" : person.face === f.id
            ? `<button class="is-cover" disabled title="${t("This face is on {name}'s card in People", { name: esc(personName(person)) })}">✓ ${t("Card photo")}</button>`
            : `<button data-cover="${f.id}" title="${t("Show this face on {name}'s card in People", { name: esc(personName(person)) })}">${t("Card photo")}</button>`}
        </span></span></div>`;
    }).join("") : `<div class="s">${t("No faces detected.")}</div>`}
    <label><input type="checkbox" id="showBoxes" ${viewer.classList.contains("boxes") ? "checked" : ""}> ${t("Show face boxes")}</label>`}
    <h5>${t("File")}</h5><div class="s">${esc(d.path)}</div>`);
}
/** The full photo couldn't be loaded: ask the server why. A file that is gone (its folder
 *  still there) is removed from the gallery; an unreachable folder removes nothing. */
async function itemMissing(id) {
  let result;
  try { result = await post(`/api/items/${id}/check`); } catch { return; }
  if (!result || result.status === "present") return; // a passing glitch; nothing to say
  const dlg = $("#goneDlg");
  if (result.status === "removed") {
    $("#goneTitle").textContent = t("This photo is no longer there");
    $("#goneText").textContent = t("Its file is no longer in its folder, so it has been removed from the gallery.");
  } else {
    $("#goneTitle").textContent = t("This photo can't be reached");
    $("#goneText").textContent = t("Its folder can't be reached right now (an unplugged drive or a network folder, for example). Nothing was removed: it will show again when the folder is back.");
  }
  $("#gonePath").textContent = result.path || "";
  dlg.showModal();
  await new Promise(r => dlg.addEventListener("close", r, { once: true }));
  if (result.status !== "removed") return;
  dropItemFromView(id);
}
/** A photo left the gallery: take it off the screen, update counts, move the viewer on. */
function dropItemFromView(id) {
  const index = items.findIndex(p => p.id === id);
  if (index >= 0) items.splice(index, 1);
  $(`.tile[data-id="${id}"]`)?.remove();
  oneItemLess();
  const count = $(".view-head .count");
  if (count?.dataset.items != null) count.textContent = mediaCount(shownItemCount, items.filter(isVideo).length);
  if (items.length && viewer.classList.contains("open")) openViewer(Math.min(Math.max(index, 0), items.length - 1));
  else closeViewer();
  loadMeta().then(() => refreshPeopleViews({ animate: false })).catch(() => {});
}

// ---- sharing and showing a photo's file (desktop app) -----------------------------------
let viewerPath = "";
/** Whether this window has a system share sheet that takes files. */
const canShareFiles = (() => {
  try { return !!navigator.canShare?.({ files: [new File([""], "photo.jpg", { type: "image/jpeg" })] }); } catch { return false; }
})();
async function shareItem(id, button) {
  const name = viewerPath.split(/[\\/]/).pop() || `photo-${id}.jpg`;
  button.disabled = true;
  try {
    const blob = await (await fetch(originalUrl(id, versionOf(id)))).blob();
    const file = new File([blob], blob.type === "image/jpeg" ? name.replace(/\.(heic|heif|tiff?|avif)$/i, ".jpg") : name, { type: blob.type });
    await navigator.share({ files: [file] });
  } catch (err) {
    if (err.name !== "AbortError") toast(t("Couldn't share it: {error}", { error: err.message }), true);
  } finally {
    button.disabled = false;
  }
}
async function revealItem(id) {
  try { await post(`/api/items/${id}/reveal`); }
  catch (err) { toast(t("Couldn't open its folder: {error}", { error: err.message }), true); }
}

// ---- rotating a photo ------------------------------------------------------------------
// The photo turns on screen at once; the file is saved once the clicks stop for a moment
// (or on leaving the photo), so several quick turns are one save.
let rotation = null; // { id, turns, timer } while turns wait to be saved
function rotateViewer(dir) {
  if (viewerIndex < 0) return;
  const id = items[viewerIndex].id;
  if (rotation?.id !== id) { flushRotation(); rotation = { id, turns: 0, timer: 0 }; }
  rotation.turns += dir;
  resetZoom();
  previewRotation();
  clearTimeout(rotation.timer);
  rotation.timer = setTimeout(flushRotation, 800);
}
function previewRotation() {
  const { width: w, height: h } = items[viewerIndex];
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
  const here = () => viewerIndex >= 0 && items[viewerIndex]?.id === r.id;
  const unturn = (w, h) => {
    const img = $(".frame img", viewer);
    img.classList.remove("turning");
    img.style.rotate = "";
    viewer.classList.remove("rotating");
    fitViewerImage(w, h);
  };
  if (!turns) {
    if (here() && !rotation) unturn(items[viewerIndex].width, items[viewerIndex].height);
    return;
  }
  let res;
  try {
    res = await post(`/api/items/${r.id}/rotate`, { turns });
  } catch (err) {
    if (here() && !rotation) unturn(items[viewerIndex].width, items[viewerIndex].height);
    return toast(t("Couldn't rotate the photo: {error}", { error: err.message }), true);
  }
  const p = items.find(p => p.id === r.id);
  if (p) Object.assign(p, { width: res.width, height: res.height, version: res.version });
  // The new pictures are ready before they replace the turned one, so nothing flickers.
  const full = new Image(), thumb = new Image();
  full.src = originalUrl(r.id, res.version);
  thumb.src = thumbUrl(r.id, res.version);
  await Promise.all([full.decode(), thumb.decode()]).catch(() => {});
  for (const tile of document.querySelectorAll(`.tile[data-id="${r.id}"]`)) {
    tile.style.setProperty("--r", (res.width / res.height).toFixed(3));
    tile.href = originalUrl(r.id, res.version);
  }
  for (const im of document.querySelectorAll("img")) {
    if (im.src && new URL(im.src).pathname.startsWith(`/thumb/${r.id}/`)) im.src = thumb.src;
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
export function askChoice(body, title) {
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
async function deleteViewerItem() {
  if (viewerIndex < 0) return;
  const id = items[viewerIndex].id;
  const detail = await api(`/api/items/${id}`).catch(() => null);
  const path = detail?.path ?? "";
  const what = `<div class="delete-what"><img src="${thumbUrl(id, versionOf(id))}" alt=""><div class="p">${esc(path)}</div></div>`;
  const choice = await askChoice(`${what}
    <div class="delete-options">
      <button class="btn" data-choice="gallery">${t("Remove from gallery")}<small>${t("The file stays on disk. It won't come back with the next scan (Settings can show it again).")}</small></button>
      <button class="btn danger" data-choice="disk">${t("Remove from disk")}<small>${t("Moves the file to the bin of the computer running Imadive.")}</small></button>
    </div>
    <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button></div>`, isVideo(items[viewerIndex]) ? t("Delete this video?") : t("Delete this photo?"));
  if (choice === "cancel") return;
  const remove = (from, permanently = false) => del(`/api/items/${id}`, permanently ? { from, permanently } : { from })
    .then(body => ({ ok: true, status: 200, body }))
    .catch(err => ({ ok: false, status: err.status, body: err.body ?? { error: err.message } }));
  let result = await remove(choice);
  if (result.status === 409 && result.body.status === "no-bin") {
    // No bin to move it to (some external or network drives): only delete for good if confirmed.
    const again = await askChoice(`${what}<p>${t("This file can't be moved to a bin on its drive, so it can't be recovered once deleted.")}</p>
      <div class="dlg-actions"><button class="btn" data-choice="cancel">${t("Cancel")}</button><button class="btn danger" data-choice="permanently">${t("Delete permanently")}</button></div>`,
      t("Delete it permanently?"));
    if (again !== "permanently") return;
    result = await remove("disk", true);
  }
  if (!result.ok) return toast(t("Couldn't delete the photo: {error}", { error: result.body.error || result.status }), true);
  toast({ removed: t("Removed from the gallery; the file is still on disk"), binned: t("Moved to the bin"), deleted: t("Deleted permanently") }[result.body.status] || t("Removed"));
  dropItemFromView(id);
}
$(".trash", viewer).addEventListener("click", deleteViewerItem);
$("#goneDlg").addEventListener("click", e => { if (e.target.dataset.close != null || e.target === e.currentTarget) e.currentTarget.close(); });

/** Names an unnamed person (or a face in no group) from the viewer. A name that exists
 *  already offers to merge, as anywhere else; a face in no group gets a group of its own. */
function startViewerRename(label) {
  const face = +label.dataset.nameFace, current = label.dataset.current ? +label.dataset.current : null;
  const input = document.createElement("input");
  input.className = "rename";
  input.placeholder = t("Add a name");
  input.setAttribute("aria-label", t("Name"));
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
      toast(t("Couldn't save the name: {error}", { error: err.message }), true);
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

export function closeViewer() {
  stopVideo();
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
  const axis = (pos, offset, size, limit) => {
    const scaled = size * zoom.s;
    if (scaled <= limit) return (limit - scaled) / 2 - offset;
    return Math.min(-offset, Math.max(limit - scaled - offset, pos));
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
    if (e.target.closest("video, .video-note")) return; // the player's own controls
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
    const width = items[viewerIndex]?.width ?? 0;
    zoomAt(e.clientX, e.clientY, Math.min(4, Math.max(2, width / frame.offsetWidth)), true);
  };
  frame.addEventListener("pointerup", release);
  frame.addEventListener("pointercancel", release);
  $(".stage", viewer).addEventListener("wheel", e => {
    if (viewerIndex < 0 || showingVideo()) return;
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
function showInfoCollapsed(collapsed) {
  viewer.classList.toggle("info-collapsed", collapsed);
  const b = $("#infoToggle");
  b.setAttribute("aria-expanded", String(!collapsed));
  b.title = collapsed ? t("Show the details (I)") : t("Hide the details (I)");
}
function setInfoCollapsed(collapsed) {
  showInfoCollapsed(collapsed);
  pref.set("infoCollapsed", collapsed ? "1" : "");
  // The photo fits the room it has now.
  if (viewerIndex >= 0) {
    resetZoom();
    if (rotation) previewRotation();
    else fitViewerImage(items[viewerIndex].width, items[viewerIndex].height);
  }
}
$("#infoToggle").addEventListener("click", e => { e.stopPropagation(); setInfoCollapsed(!viewer.classList.contains("info-collapsed")); });
if (pref.get("infoCollapsed") === "1") showInfoCollapsed(true);
$(".info", viewer).addEventListener("change", e => { if (e.target.id === "showBoxes") viewer.classList.toggle("boxes", e.target.checked); });
$(".info", viewer).addEventListener("click", async e => {
  const link = e.target.closest("a[target=_blank]");
  if (link && folderInfo.desktop) {
    e.preventDefault();
    return post("/api/open", { url: link.href }).catch(err => toast(t("Couldn't open the map: {error}", { error: err.message }), true));
  }
  const act = e.target.closest(".item-acts button");
  if (act?.dataset.shareItem) return shareItem(+act.dataset.shareItem, act);
  if (act?.dataset.reveal) return revealItem(+act.dataset.reveal);
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
      toast(p ? t("This face is now on {name}'s card", { name: personName(p) }) : t("Card photo changed"));
    } catch (err) {
      e.target.disabled = false;
      return toast(t("Couldn't change the card photo: {error}", { error: err.message }), true);
    }
    await loadMeta();
    refreshPeopleViews({ animate: false });
    return openViewer(viewerIndex);
  }
  if (e.target.dataset.reject) {
    e.target.disabled = true;
    try {
      const { person } = await post(`/api/faces/${e.target.dataset.reject}/reject`);
      toast(t("Moved to a new group, {name}. Rename or hide it in People.", { name: personName({ id: person }) }));
    } catch (err) {
      toast(t("Couldn't move the face: {error}", { error: err.message }), true);
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
  // Arrows and Space on the player seek and pause it.
  if (e.target.tagName === "VIDEO" && (e.key.startsWith("Arrow") || e.key === " ")) return;
  // Handled here: other Escape handlers (the side panels') leave it alone.
  if (e.key === "Escape") { e.preventDefault(); closeViewer(); }
  else if (e.key === "Delete") deleteViewerItem();
  else if (e.key === "r" || e.key === "R") { if ($("#viewer [data-rotate]")) rotateViewer(e.shiftKey ? -1 : 1); }
  else if (e.key === "i" || e.key === "I") setInfoCollapsed(!viewer.classList.contains("info-collapsed"));
  else if (e.key === "ArrowLeft") openViewer(viewerIndex - 1);
  else if (e.key === "ArrowRight") openViewer(viewerIndex + 1);
});
