// Upload: a folder chosen in the browser goes into imaDive-uploads in the gallery's folder,
// keeping its folder; files that aren't photos or videos are left out; with two gallery
// folders, a dialog asks which one. The scan afterwards adds them to the gallery.
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { devices, expect, test } from "@playwright/test";

const fixtures = join(dirname(fileURLToPath(import.meta.url)), "fixtures/library");
const count = async page => (await (await page.request.get("/api/items")).json()).items.length;
let library, other, picked;

test.beforeAll(async ({ request }) => {
  const { folders } = await (await request.get("/api/folders")).json();
  library = folders[0].path;
  other = library.replace(/library$/, "other");
  // What a person would pick: a folder with two photos in a subfolder, and a note.
  picked = join(mkdtempSync(join(tmpdir(), "imadive-pick-")), "Holidays");
  mkdirSync(join(picked, "beach"), { recursive: true });
  copyFileSync(join(fixtures, "plain.jpg"), join(picked, "beach", "one.jpg"));
  copyFileSync(join(fixtures, "2025/city.jpg"), join(picked, "two.jpg"));
  writeFileSync(join(picked, "notes.txt"), "not a photo");
});

// The other tests count the library's photos: the uploads go again, even when this fails.
test.afterAll(async ({ request }) => {
  rmSync(join(library, "imaDive-uploads"), { recursive: true, force: true });
  rmSync(join(other, "imaDive-uploads"), { recursive: true, force: true });
  await request.delete(`/api/folders?path=${encodeURIComponent(other)}`);
  await request.post("/api/scan");
  rmSync(dirname(picked), { recursive: true, force: true });
});

test("a folder is uploaded into imaDive-uploads, with a progress bar", async ({ page }) => {
  const before = await count(page);
  await page.goto("/");
  await page.locator('#tabs [data-view="manage"]').click();
  await expect(page.locator(".upload .drop")).toContainText("Drop a folder");
  await expect(page.locator(".upload .lead")).toContainText("imaDive-uploads");

  await page.locator("#pickFolder").setInputFiles(picked);
  const result = page.locator(".up-result");
  await expect(result).toContainText("2 files uploaded", { timeout: 15_000 });
  await expect(result).toContainText("1 left out");
  expect(existsSync(join(library, "imaDive-uploads/Holidays/beach/one.jpg"))).toBe(true);
  expect(existsSync(join(library, "imaDive-uploads/Holidays/two.jpg"))).toBe(true);
  expect(existsSync(join(library, "imaDive-uploads/Holidays/notes.txt"))).toBe(false);
  // The scan the page started adds them to the gallery.
  await expect.poll(() => count(page), { timeout: 15_000 }).toBe(before + 2);

  // The same folder again: the names are there, so it asks; identical files are never
  // copied twice, whatever the choice.
  const sameNames = page.locator("#choiceDlg[open]");
  await page.locator("#pickFolder").setInputFiles(picked);
  await expect(sameNames).toContainText("2 files have the same names");
  await sameNames.locator('[data-choice="keep"]').click();
  await expect(result).toContainText("2 were already there", { timeout: 15_000 });

  // A different photo under a name that's there: Skip leaves the old one, Overwrite replaces it.
  copyFileSync(join(fixtures, "plain.jpg"), join(picked, "two.jpg"));
  await page.locator("#pickFolder").setInputFiles(picked);
  await sameNames.locator('[data-choice="skip"]').click();
  // Skip leaves out every name that's there, without sending them.
  await expect(result).toContainText("2 skipped", { timeout: 15_000 });
  await page.locator("#pickFolder").setInputFiles(picked);
  await sameNames.locator('[data-choice="overwrite"]').click();
  await expect(result).toContainText("1 replaced", { timeout: 15_000 });
  expect(readFileSync(join(library, "imaDive-uploads/Holidays/two.jpg"))).toEqual(readFileSync(join(fixtures, "plain.jpg")));
  copyFileSync(join(fixtures, "2025/city.jpg"), join(picked, "two.jpg")); // as it was, for the next tests
});

test("waiting for the files is shown, and failed files can be retried", async ({ page }) => {
  await page.goto("/#view=manage");
  // Nothing chosen yet: the card says it is waiting for the picker.
  const chooser = page.waitForEvent("filechooser");
  await page.locator('[data-pick="files"]').click();
  await expect(page.locator(".up-preparing")).toContainText("Choose the files");
  // The connection fails (both tries): the file is listed with a Retry button.
  await page.route("**/api/uploads?*", route => route.abort());
  const fresh = join(dirname(picked), "retry-me.jpg"); // a name not uploaded yet
  copyFileSync(join(fixtures, "plain.jpg"), fresh);
  await (await chooser).setFiles(fresh);
  const result = page.locator(".up-result");
  // With two gallery folders (the test before added one), the first.
  const target = page.locator("#choiceDlg[open] .up-target").first();
  if (await target.isVisible({ timeout: 2_000 }).catch(() => false)) await target.click();
  await expect(result).toContainText("couldn't be uploaded", { timeout: 15_000 });
  await page.unroute("**/api/uploads?*");
  await result.locator("[data-upload-retry]").click();
  await expect(result).not.toContainText("couldn't be uploaded", { timeout: 15_000 });
  await expect(result.locator("[data-upload-retry]")).toHaveCount(0);
});

test("with two gallery folders, the upload asks which one", async ({ page }) => {
  const added = await page.request.post("/api/folders", { data: { path: other } });
  expect(added.ok() || added.status() === 409).toBe(true);
  await page.goto("/");
  await page.locator('#tabs [data-view="manage"]').click();
  await page.locator("#pickFiles").setInputFiles(join(picked, "two.jpg"));
  const dialog = page.locator("#choiceDlg[open]");
  await expect(dialog).toContainText("imaDive-uploads");
  await dialog.locator(".up-target", { hasText: /other$/ }).click();
  await expect(page.locator(".up-result")).toContainText("1 file uploaded", { timeout: 15_000 });
  expect(existsSync(join(other, "imaDive-uploads/two.jpg"))).toBe(true);
});

test("on a phone: no folder button, and Choose files opens the file manager", async ({ browser }) => {
  // Phones open their photo picker for "Choose photos and videos" (at most 100, one by one):
  // "Choose files" asks for any file, which opens the file manager, where a whole folder can
  // be selected. What isn't a photo or video is left out.
  const { defaultBrowserType, ...pixel } = devices["Pixel 7"];
  const context = await browser.newContext({ ...pixel, baseURL: test.info().project.use.baseURL });
  const page = await context.newPage();
  await page.goto("/#view=manage");
  await expect(page.locator('.upload [data-pick="any"]')).toBeVisible();
  await expect(page.locator('.upload [data-pick="folder"]')).toBeHidden();
  await expect(page.locator(".upload .drop .hint")).toContainText("Select all");
  expect(await page.locator("#pickAny").getAttribute("accept")).toBeNull();
  await page.locator("#pickAny").setInputFiles([join(picked, "beach", "one.jpg"), join(picked, "notes.txt")]);
  const dialog = page.locator("#choiceDlg[open]");
  if (await dialog.isVisible().catch(() => false)) await dialog.locator(".up-target").first().click();
  await expect(page.locator(".up-result")).toContainText("left out", { timeout: 15_000 });
  await context.close();
});

