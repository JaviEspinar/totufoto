// Video thumbnails, made by the page: the server can't decode videos, the browser can (the
// formats it plays). A video whose thumbnail doesn't come shows the generic picture
// (photos.js) and asks here for a real one: a frame about a second in, sent to the server,
// which keeps it for everyone. A video the browser can't decode keeps the generic picture.
// One of the page's modules; main.js starts the page.
import { api } from "./core.js";
import { originalUrl, photos, thumbUrl } from "./photos.js";

const AT_ONCE = 2;
/** The biggest side of the frame sent; the server makes the thumbnail from it. */
const FRAME_SIDE = 1280;
const queue = [], waiting = new Set(), failed = new Set();
let running = 0;

/** Makes the thumbnail of video `id` (at `version`), unless it is on its way or failed. */
export function makeVideoThumb(id, version) {
  if (waiting.has(id) || failed.has(id)) return;
  waiting.add(id);
  queue.push({ id, version });
  pump();
}

function pump() {
  while (running < AT_ONCE && queue.length) {
    const job = queue.shift();
    running++;
    frameOf(job)
      .then(blob => api(`/api/photos/${job.id}/thumb?v=${job.version}`, { method: "PUT", headers: { "content-type": "image/jpeg" }, body: blob }))
      .then(saved => show(job.id, saved.version))
      .catch(() => failed.add(job.id)) // not this visit again
      .finally(() => { running--; waiting.delete(job.id); pump(); });
  }
}

/** A frame of the video as a JPEG: about a second in, or the first one when the file can't
 *  seek (some WebM files don't say how long they are). */
async function frameOf({ id, version }) {
  const video = document.createElement("video");
  video.muted = true;
  video.playsInline = true;
  video.preload = "auto";
  const event = (name, ms) => new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`no ${name}`)), ms);
    video.addEventListener(name, () => { clearTimeout(timer); resolve(); }, { once: true });
    video.addEventListener("error", () => { clearTimeout(timer); reject(new Error("can't decode it")); }, { once: true });
  });
  try {
    const ready = event("loadeddata", 15_000);
    video.src = originalUrl(id, version);
    await ready;
    const length = Number.isFinite(video.duration) ? video.duration : 0;
    if (length > 0.1) {
      const seeked = event("seeked", 3_000);
      video.currentTime = Math.min(1, length / 2);
      await seeked.catch(() => {}); // the frame it has, then
    }
    const { videoWidth: w, videoHeight: h } = video;
    if (!w || !h) throw new Error("no picture");
    const scale = Math.min(1, FRAME_SIDE / Math.max(w, h));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(w * scale);
    canvas.height = Math.round(h * scale);
    canvas.getContext("2d").drawImage(video, 0, 0, canvas.width, canvas.height);
    const blob = await new Promise(resolve => canvas.toBlob(resolve, "image/jpeg", 0.85));
    if (!blob) throw new Error("no picture");
    return blob;
  } finally {
    video.removeAttribute("src");
    video.load(); // stops the download
  }
}

/** The new thumbnail wherever the video shows (tiles, cards), and its new version. */
function show(id, version) {
  const entry = photos.find(p => p.id === id);
  if (entry) entry.version = version;
  for (const img of document.querySelectorAll(`img[data-thumb="${id}"], img[data-cover="${id}"]`)) {
    img.classList.remove("generic");
    img.src = thumbUrl(id, version);
  }
}
