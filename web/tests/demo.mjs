// Records the README's demo loop of a running gallery (the library of the screenshots, see
// docs/screenshots/README.md), then turns it into docs/screenshots/demo.webp, an animated WebP:
//
//   node demo.mjs http://127.0.0.1:7878
//
// The gallery should have finished indexing. Needs ffmpeg and img2webp (libwebp) on the PATH.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const base = process.argv[2] ?? "http://127.0.0.1:7878";
const out = resolve(dirname(fileURLToPath(import.meta.url)), "../../docs/screenshots/demo.webp");
const size = { width: 1280, height: 720 };
const tmp = mkdtempSync(join(tmpdir(), "imadive-demo-"));

const browser = await chromium.launch();
const context = await browser.newContext({ viewport: size, colorScheme: "light", recordVideo: { dir: tmp, size } });
const page = await context.newPage();
const pause = ms => page.waitForTimeout(ms);

/** Waits until every image on screen has loaded (or failed). */
async function settle() {
  await page.waitForFunction(() => [...document.images].filter(i => i.getBoundingClientRect().top < innerHeight).every(i => i.complete));
  await pause(400);
}

/** Scrolls the page smoothly by `dy` pixels over `ms`. */
async function glide(dy, ms) {
  await page.evaluate(([dy, ms]) => new Promise(done => {
    const main = document.querySelector("#main"), from = main.scrollTop, start = performance.now();
    const step = now => {
      const k = Math.min(1, (now - start) / ms);
      main.scrollTop = from + dy * (0.5 - Math.cos(Math.PI * k) / 2);
      k < 1 ? requestAnimationFrame(step) : done();
    };
    requestAnimationFrame(step);
  }), [dy, ms]);
}

/** Groups the Photos tab (the choice is in the dates, grouping and order panel). */
async function groupBy(value) {
  await page.locator("#viewBtn").click();
  await page.selectOption("#groupBy", value);
  await page.locator("#viewBtn").click();
}

/** The id of the photo whose file name contains `text`. */
async function photoId(text) {
  const { items } = await (await page.request.get(`${base}/api/items`)).json();
  for (const [id] of items) {
    const d = await (await page.request.get(`${base}/api/items/${id}`)).json();
    if (d.path.includes(text)) return id;
  }
  throw new Error(`no photo with ${text}`);
}

// The photo opened in the viewer, as in the screenshots: the crew, all named.
const shown = await photoId("Crew Visits KSC 1");

// Recording starts with the page: everything before the timeline is shown is cut below.
const started = Date.now();
await page.goto(`${base}/#group=none`);
await page.locator(".tile img.ok").first().waitFor();
await settle();
const cut = (Date.now() - started) / 1000;

// 1. The timeline.
await pause(900);
await glide(700, 1800);
await pause(500);

// 2. A photo, with its people named.
await page.locator(`.tile[data-id="${shown}"]`).scrollIntoViewIfNeeded();
await page.locator(`.tile[data-id="${shown}"]`).click();
await page.locator("#viewer .info-body h3").waitFor();
await page.locator("#showBoxes").check();
await settle();
await pause(2600);
await page.keyboard.press("Escape");

// 3. People, then one person's photos.
await page.locator('nav [data-view="people"]').click();
await page.locator(".face-card:not(.skeleton)").first().waitFor();
await page.locator("#peopleZoom").evaluate(z => { z.value = z.max; z.dispatchEvent(new Event("input", { bubbles: true })); });
await settle();
await pause(1500);
await page.locator(".face-card", { has: page.locator('input[value="Christina Koch"]') }).locator(".avatar").click();
await page.locator(".tile img.ok").first().waitFor();
await settle();
await pause(1600);

// 4. The places.
await page.locator('[data-clear="all"]').click();
await groupBy("place");
await page.locator(".group-card").first().waitFor();
await settle();
await pause(1800);

await context.close();
await browser.close();

const video = join(tmp, readdirSync(tmp).find(f => f.endsWith(".webm")));
const FPS = 12;
execFileSync("ffmpeg", ["-v", "error", "-ss", String(cut), "-i", video, "-vf", `fps=${FPS},scale=960:-1:flags=lanczos`, join(tmp, "f%04d.png")]);
const frames = readdirSync(tmp).filter(f => f.endsWith(".png")).sort().map(f => join(tmp, f));
execFileSync("img2webp", ["-loop", "0", "-lossy", "-q", "65", "-m", "6", "-d", String(Math.round(1000 / FPS)), ...frames, "-o", out]);
rmSync(tmp, { recursive: true });
console.log(out);
