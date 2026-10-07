// A folder moved on disk: Settings shows it can't be found and offers "Moved to…"; pointing
// it at the new place keeps its photos as they were (the same items), without indexing them
// again.
import { renameSync } from "node:fs";
import { expect, test } from "@playwright/test";

let other, moved;

test.beforeAll(async ({ request }) => {
  const { folders } = await (await request.get("/api/folders")).json();
  other = folders[0].path.replace(/library$/, "other");
  moved = other.replace(/other$/, "other-moved");
});

// The other tests use the folder where it was: put it back, out of the gallery.
test.afterAll(async ({ request }) => {
  for (const path of [other, moved]) await request.delete(`/api/folders?path=${encodeURIComponent(path)}`);
  try { renameSync(moved, other); } catch {}
  await request.post("/api/scan");
});

test("a folder moved on disk is pointed at its new place, keeping its photos", async ({ page }) => {
  const added = await page.request.post("/api/folders", { data: { path: other } });
  expect(added.ok()).toBe(true);
  const idOf = async () => {
    const { items } = await (await page.request.get("/api/items")).json();
    for (const [id] of items) if ((await (await page.request.get(`/api/items/${id}`)).json()).path.endsWith("street.jpg")) return id;
  };
  await expect.poll(idOf, { timeout: 15_000 }).toBeTruthy();
  const before = await idOf();

  renameSync(other, moved);
  await page.goto("/");
  await page.locator("#settingsBtn").click();
  const row = page.locator("#settingsDlg .folder", { hasText: "other" });
  await expect(row).toContainText("not available");
  await row.locator("[data-move]").click();

  const browse = page.locator("#browseDlg[open]");
  await expect(browse.locator("#browseTitle")).toHaveText("Where is this folder now?");
  await browse.locator("#browsePath").fill(moved);
  await browse.locator("#browsePath").press("Enter");
  await expect(browse.locator("#browsePath")).toHaveValue(moved);
  await browse.locator("[data-add-here]").click();
  await expect(page.locator("#toast")).toContainText("1 file found in its new place");
  await expect(page.locator("#settingsDlg .folder", { hasText: "other-moved" })).not.toContainText("not available");
  expect(await idOf()).toBe(before);
  // The browse dialog says "Add" again for the next folder added.
  await expect(page.locator("#browseTitle")).toHaveText("Add a folder");
});
