"use strict";
// The photo viewer: details, faces, sharing, rotating, deleting, zoom and swipe.
// Part of the page's script, split by area; see web/index.html for the order.

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
  if (photos[i + 1]) {
    new Image().src = originalUrl(photos[i + 1].id, photos[i + 1].version);
    photoDetail(photos[i + 1].id, photos[i + 1].version).catch(() => {});
  }
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
  try { d = await photoDetail(id, v); }
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
