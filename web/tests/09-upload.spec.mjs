// Upload: a folder chosen in the browser goes into imaDive-uploads in the gallery's folder,
// keeping its folder; files that aren't photos or videos are left out; with two gallery
// folders, a dialog asks which one. The scan afterwards adds them to the gallery.
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test } from "@playwright/test";

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

  // The same folder again: nothing new.
  await page.locator("#pickFolder").setInputFiles(picked);
  await expect(result).toContainText("2 were already there", { timeout: 15_000 });
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
