// The slideshow: the photos and videos of the Photos tab's filters, one after another, full
// screen. Moving the mouse or touching the screen shows its bar (in order or shuffled, the
// speed, and ✕ to stop). One of the page's modules; main.js starts the page.
import { t } from "./i18n.js";
import { $, api, icon, itemQuery, pref } from "./core.js";
import { VIDEO_PICTURE, isVideo, itemRow, originalUrl, silentUrl, thumbUrl } from "./items.js";
import { toast } from "./people.js";

/** Seconds a photo stays, from slowest to fastest: the speed slider picks one. */
const SPEEDS = [20, 15, 12, 10, 8, 6, 5, 4, 3, 2];
const DEFAULT_SPEED = SPEEDS.indexOf(5);
/** How long the bar stays after the last movement or touch. */
const BAR_TIME = 3000;

const savedSpeed = +pref.get("slideshowSpeed");
const show = {
  el: null,
  all: [], // the items, in the gallery's order
  order: [], // indices into `all`, in the order they are shown
  pos: 0, // where in `order` the slideshow is
  shuffled: pref.get("slideshowOrder") === "shuffle",
  speed: Number.isInteger(savedSpeed) && savedSpeed >= 0 && savedSpeed < SPEEDS.length && pref.get("slideshowSpeed") !== null
    ? savedSpeed : DEFAULT_SPEED,
  timer: 0, // the next step
  barTimer: 0, // hiding the bar
  step: 0, // counts every change, so a late photo or video event knows it is stale
  front: 0, // which of the two photos is on top
  wakeLock: null,
  returnFocus: null,
};

export const slideshowOpen = () => !!show.el && !show.el.hidden;

function build() {
  const el = document.createElement("div");
  el.id = "slideshow";
  el.hidden = true;
  el.setAttribute("role", "dialog");
  el.setAttribute("aria-modal", "true");
  el.setAttribute("aria-label", t("Slideshow"));
  el.innerHTML = `<div class="ss-stage">
      <img class="ss-photo" alt=""><img class="ss-photo" alt="">
      <video class="ss-video" playsinline hidden></video>
    </div>
    <div class="ss-bar">
      <div class="seg ss-order" role="group" aria-label="${t("Order")}">
        <button data-ss-order="order">${t("In order")}</button>
        <button data-ss-order="shuffle">${t("Shuffle")}</button>
      </div>
      <label class="ss-speed"><span>${t("Speed")}</span>
        <input type="range" min="0" max="${SPEEDS.length - 1}" step="1" aria-label="${t("Speed")}">
        <span class="ss-seconds"></span></label>
      <span class="ss-pos"></span>
      <button class="ss-close" title="${t("Stop the slideshow (Esc)")}" aria-label="${t("Stop the slideshow")}">✕</button>
    </div>`;
  document.body.append(el);
  el.querySelector(".ss-order").addEventListener("click", e => {
    const order = e.target.closest("[data-ss-order]")?.dataset.ssOrder;
    if (order) setShuffled(order === "shuffle");
  });
  const speed = el.querySelector(".ss-speed input");
  speed.addEventListener("input", () => setSpeed(+speed.value));
  el.querySelector(".ss-close").addEventListener("click", stopSlideshow);
  // Any movement or touch shows the bar for a while; a touch on the picture also hides it
  // again at once when it is showing, as phone galleries do.
  el.addEventListener("pointermove", e => { if (e.pointerType === "mouse") showBar(); });
  el.addEventListener("pointerdown", e => {
    if (e.target.closest(".ss-bar")) return showBar();
    if (e.pointerType !== "mouse" && el.classList.contains("bar")) return hideBar();
    showBar();
  });
  el.addEventListener("focusin", showBar);
  const video = el.querySelector(".ss-video");
  video.addEventListener("ended", () => next());
  document.addEventListener("fullscreenchange", () => {
    // Leaving full screen (Esc, the browser's own button) stops the slideshow too.
    if (slideshowOpen() && !document.fullscreenElement && el.dataset.fullscreen) stopSlideshow();
  });
  return el;
}

/** Starts the slideshow on the Photos tab's current filters. */
export async function startSlideshow(button) {
  let data;
  try { data = await api(itemQuery()); }
  catch (err) { return toast(err.message, true); }
  const all = (data.items ?? []).map(itemRow);
  if (!all.length) return toast(t("Nothing matches these filters."));
  show.el ??= build();
  show.all = all;
  show.returnFocus = button ?? document.activeElement;
  makeOrder(0);
  const el = show.el;
  el.hidden = false;
  el.classList.remove("bar");
  for (const img of el.querySelectorAll(".ss-photo")) { img.classList.remove("on"); img.removeAttribute("src"); }
  syncBar();
  document.body.classList.add("slideshow-on");
  delete el.dataset.fullscreen;
  el.requestFullscreen?.().then(() => { el.dataset.fullscreen = "1"; }).catch(() => {});
  // The screen stays on while it plays, where the browser allows it.
  navigator.wakeLock?.request("screen").then(lock => { show.wakeLock = lock; }).catch(() => {});
  el.querySelector(".ss-close").focus({ preventScroll: true });
  showBar();
  showCurrent();
}

export function stopSlideshow() {
  if (!slideshowOpen()) return;
  const el = show.el;
  clearTimeout(show.timer);
  clearTimeout(show.barTimer);
  show.step++;
  stopVideo();
  el.hidden = true;
  document.body.classList.remove("slideshow-on");
  if (document.fullscreenElement === el) document.exitFullscreen().catch(() => {});
  delete el.dataset.fullscreen;
  show.wakeLock?.release().catch(() => {});
  show.wakeLock = null;
  show.returnFocus?.focus?.({ preventScroll: true });
}

/** The order to show the items in, starting from item `first` (an index into `all`). */
function makeOrder(first) {
  const n = show.all.length;
  if (!show.shuffled) {
    show.order = Array.from({ length: n }, (_, i) => i);
    show.pos = first;
    return;
  }
  const rest = Array.from({ length: n }, (_, i) => i).filter(i => i !== first);
  for (let i = rest.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [rest[i], rest[j]] = [rest[j], rest[i]];
  }
  show.order = [first, ...rest];
  show.pos = 0;
}

function setShuffled(shuffled) {
  if (shuffled === show.shuffled) return;
  show.shuffled = shuffled;
  pref.set("slideshowOrder", shuffled ? "shuffle" : "order");
  // Goes on from the item on screen, in the new order.
  makeOrder(show.order[show.pos]);
  syncBar();
}

function setSpeed(speed) {
  show.speed = speed;
  pref.set("slideshowSpeed", String(speed));
  syncBar();
  // A photo on screen waits the new time from now on; a video still plays to its end.
  if (!isVideo(current())) schedule();
}

function syncBar() {
  const el = show.el;
  for (const b of el.querySelectorAll("[data-ss-order]")) {
    const on = (b.dataset.ssOrder === "shuffle") === show.shuffled;
    b.classList.toggle("on", on);
    b.setAttribute("aria-pressed", String(on));
  }
  el.querySelector(".ss-speed input").value = show.speed;
  el.querySelector(".ss-seconds").textContent = t("{s} s per photo", { s: SPEEDS[show.speed] });
  el.querySelector(".ss-pos").textContent = t("{n} of {total}", { n: show.pos + 1, total: show.order.length });
}

function showBar() {
  if (!slideshowOpen()) return;
  show.el.classList.add("bar");
  clearTimeout(show.barTimer);
  show.barTimer = setTimeout(() => {
    // Stays while the pointer is over it, or the keyboard is in it.
    if (show.el.querySelector(".ss-bar:hover, .ss-bar :focus-visible")) return showBar();
    hideBar();
  }, BAR_TIME);
}
function hideBar() {
  clearTimeout(show.barTimer);
  show.el.classList.remove("bar");
}

const current = () => show.all[show.order[show.pos]];

/** Moves `by` items (wrapping around at the ends) and shows that one. */
function next(by = 1) {
  if (!slideshowOpen()) return;
  const n = show.order.length;
  const end = show.pos + by >= n;
  show.pos = (show.pos + by + n) % n;
  // A new round of a shuffled slideshow is shuffled again.
  if (end && show.shuffled && by > 0) makeOrder(show.order[show.pos]);
  showCurrent();
}

function schedule() {
  clearTimeout(show.timer);
  show.timer = setTimeout(() => next(), SPEEDS[show.speed] * 1000);
}

function stopVideo() {
  const video = show.el.querySelector(".ss-video");
  video.pause();
  video.removeAttribute("src");
  video.load();
  video.hidden = true;
}

/** Shows the item at `pos`: a photo fades in over the last one; a video plays to its end. */
async function showCurrent() {
  const step = ++show.step;
  clearTimeout(show.timer);
  syncBar();
  const item = current();
  const el = show.el;
  if (isVideo(item)) return playVideo(item, step);
  // The photo is loaded before it replaces the one on screen; one that can't be is skipped.
  const img = el.querySelectorAll(".ss-photo")[1 - show.front];
  img.src = originalUrl(item.id, item.version);
  try { await img.decode(); }
  catch {
    if (step === show.step) show.timer = setTimeout(() => next(), 300);
    return;
  }
  if (step !== show.step) return;
  stopVideo();
  img.classList.add("on");
  el.querySelectorAll(".ss-photo")[show.front].classList.remove("on");
  show.front = 1 - show.front;
  schedule();
  preloadNext();
}

/** The next photo, fetched while this one is on screen. */
function preloadNext() {
  const after = show.all[show.order[(show.pos + 1) % show.order.length]];
  if (after && !isVideo(after)) new Image().src = originalUrl(after.id, after.version);
}

function playVideo(item, step) {
  const el = show.el, video = el.querySelector(".ss-video");
  video.poster = thumbUrl(item.id, item.version);
  video.onerror = () => {
    if (step !== show.step) return;
    // Its sound may be what can't be played: the picture alone, then; else the next item.
    if (!video.dataset.silent) {
      video.dataset.silent = "1";
      video.src = silentUrl(item.id, item.version);
      video.play().catch(() => {});
      return;
    }
    video.poster = VIDEO_PICTURE;
    show.timer = setTimeout(() => next(), 1500);
  };
  delete video.dataset.silent;
  video.muted = false;
  video.src = originalUrl(item.id, item.version);
  video.hidden = false;
  // The photo underneath fades out while the video takes the screen.
  for (const img of el.querySelectorAll(".ss-photo")) img.classList.remove("on");
  // Started from a click, so the browser normally lets it play with sound; if it doesn't,
  // it plays muted rather than not at all.
  video.play().catch(() => {
    if (step !== show.step) return;
    video.muted = true;
    video.play().catch(() => { if (step === show.step) show.timer = setTimeout(() => next(), 1500); });
  });
  preloadNext();
}

document.addEventListener("keydown", e => {
  if (!slideshowOpen()) return;
  if (e.key === "Escape") { e.preventDefault(); e.stopImmediatePropagation(); stopSlideshow(); }
  else if (e.key === "ArrowRight") { e.preventDefault(); next(1); showBar(); }
  else if (e.key === "ArrowLeft") { e.preventDefault(); next(-1); showBar(); }
}, true);

// The button next to the photo count, in the Photos tab's filter bar.
document.addEventListener("click", e => {
  const button = e.target.closest("[data-slideshow]");
  if (button) startSlideshow(button);
});
