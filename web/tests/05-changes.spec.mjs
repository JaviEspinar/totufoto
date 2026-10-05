// Changing photos: rotating and removing from the gallery (never from disk: that would use
// the bin of the computer running the tests).
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/#group=none");
  await expect(page.locator(".tile").first()).toBeVisible();
});

test("rotating turns the photo and its thumbnail", async ({ page }) => {
  const tile = page.locator(".tile", { has: page.locator("img") }).last();
  const id = await tile.getAttribute("data-id");
  const ratio = () => page.locator(`.tile[data-id="${id}"]`).evaluate(t => parseFloat(t.style.getPropertyValue("--r")));
  const before = await ratio();
  await tile.click();
  await page.locator('#viewer [data-rotate="1"]').click();
  await expect.poll(ratio, { timeout: 10_000 }).toBeCloseTo(1 / before, 2);
  await expect(page.locator(`.tile[data-id="${id}"] img`)).toHaveAttribute("src", new RegExp(`/thumb/${id}/1$`));
});

test("removing from the gallery, then showing it again from Settings", async ({ page }) => {
  await expect(page.locator(".view-head .count")).toContainText("12 photos");
  await page.locator(".tile").first().click();
  await page.keyboard.press("Delete");
  await page.locator('#choiceDlg [data-choice="gallery"]').click();
  await expect(page.locator("#toast")).toContainText("Removed from the gallery");
  await page.keyboard.press("Escape");
  await expect(page.locator(".view-head .count")).toContainText("11 photos");

  await page.locator("#settingsBtn").click();
  const excluded = page.locator("#settingsExcluded");
  await expect(excluded).toContainText("1 photo removed from the gallery");
  await excluded.locator("[data-show-excluded]").click();
  await page.keyboard.press("Escape");
  await expect.poll(async () => (await page.request.get("/api/photos").then(r => r.json())).photos.length, { timeout: 15_000 }).toBe(12);
});
