// The photo viewer: moving between photos, details, zoom, and the keyboard.
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/#group=none");
  await expect(page.locator(".tile").first()).toBeVisible();
});

test("arrows move between photos and Escape closes", async ({ page }) => {
  await page.locator(".tile").nth(1).click();
  const heading = page.locator("#viewer .info-body h3");
  await expect(heading).toBeVisible();
  const first = await heading.innerText();
  await page.keyboard.press("ArrowRight");
  await expect(heading).not.toHaveText(first);
  await page.keyboard.press("ArrowLeft");
  await expect(heading).toHaveText(first);
  await page.keyboard.press("Escape");
  await expect(page.locator("#viewer")).not.toHaveClass(/open/);
});

/** The id of the photo whose file name ends with `name`. */
async function photoId(page, name) {
  const { photos } = await (await page.request.get("/api/photos")).json();
  for (const [id] of photos) {
    const d = await (await page.request.get(`/api/photos/${id}`)).json();
    if (d.path.endsWith(name)) return id;
  }
  throw new Error(`no photo ${name}`);
}

test("details show the place and the people", async ({ page }) => {
  // The Paris photo from August 2025 has one unnamed person.
  await page.locator(`.tile[data-id="${await photoId(page, "city.jpg")}"]`).click();
  const info = page.locator("#viewer .info-body");
  await expect(info).toContainText("Paris");
  await expect(info).toContainText("People (1)");
});

test("the details panel folds away, and stays folded", async ({ page }) => {
  await page.locator(".tile").first().click();
  await page.locator("#infoToggle").click();
  await expect(page.locator("#viewer")).toHaveClass(/info-collapsed/);
  await page.keyboard.press("ArrowRight");
  await expect(page.locator("#viewer")).toHaveClass(/info-collapsed/);
  await page.keyboard.press("i");
  await expect(page.locator("#viewer")).not.toHaveClass(/info-collapsed/);
});

test("a click zooms in and another fits again", async ({ page }) => {
  await page.locator(".tile").first().click();
  const frame = page.locator("#viewer .frame");
  await expect(frame).toBeVisible();
  await page.locator("#viewer .frame img").click();
  await expect(page.locator("#viewer")).toHaveClass(/zoomed/);
  await page.locator("#viewer .frame img").click();
  await expect(page.locator("#viewer")).not.toHaveClass(/zoomed/);
});

test("keyboard: focus goes into the viewer and back to the photo", async ({ page }) => {
  const tile = page.locator(".tile").nth(2);
  await tile.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#viewer .close")).toBeFocused();
  for (let i = 0; i < 8; i++) {
    await page.keyboard.press("Tab");
    await expect.poll(() => page.evaluate(() => !!document.activeElement?.closest("#viewer"))).toBe(true);
  }
  await page.keyboard.press("Escape");
  await expect(tile).toBeFocused();
});
