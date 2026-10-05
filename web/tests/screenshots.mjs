// Takes the README's screenshots of a running gallery (docs/screenshots/README.md says how
// the library in them was made):
//
//   node screenshots.mjs http://127.0.0.1:7878
//
// The gallery should have finished indexing. The pictures go to docs/screenshots/.
import { mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, devices } from "@playwright/test";

const base = process.argv[2] ?? "http://127.0.0.1:7878";
const out = resolve(dirname(fileURLToPath(import.meta.url)), "../../docs/screenshots");
mkdirSync(out, { recursive: true });

const browser = await chromium.launch();

/** Waits until every image on screen has loaded (or failed), and the page is still. */
async function settle(page) {
  await page.waitForFunction(() => [...document.images].filter(i => i.getBoundingClientRect().top < innerHeight).every(i => i.complete));
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(600); // fades and the justified layout
}

async function shot(page, name) {
  await settle(page);
  await page.screenshot({ path: `${out}/${name}.jpg`, type: "jpeg", quality: 82 });
  console.log(`${name}.jpg`);
}

/** The id of the photo whose file name contains `text`. */
async function photoId(page, text) {
  const { photos } = await (await page.request.get(`${base}/api/photos`)).json();
  for (const [id] of photos) {
    const d = await (await page.request.get(`${base}/api/photos/${id}`)).json();
    if (d.path.includes(text)) return id;
  }
  throw new Error(`no photo with ${text}`);
}

const desktop = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: "light" });
const page = await desktop.newPage();

// The timeline without groups: the justified grid.
await page.goto(`${base}/#group=none`);
await page.locator(".tile img.ok").first().waitFor();
await shot(page, "photos");

// One card per place.
await page.selectOption("#groupBy", "place");
await page.locator(".group-card").first().waitFor();
await shot(page, "places");

// The viewer, with the people in the photo.
const id = await photoId(page, "with his wife and the youngest");
await page.selectOption("#groupBy", "none");
await page.locator(`.tile[data-id="${id}"]`).click();
await page.locator("#viewer .info-body h3").waitFor();
await page.locator("#showBoxes").check();
await shot(page, "viewer");
await page.keyboard.press("Escape");

// People.
await page.locator('nav [data-view="people"]').click();
await page.locator(".face-card:not(.skeleton)").first().waitFor();
await shot(page, "people");

// A phone.
const phone = await browser.newContext({ ...devices["Pixel 7"] });
const small = await phone.newPage();
await small.goto(`${base}/`);
await small.locator(".group-card").first().waitFor();
await shot(small, "phone");

await browser.close();
