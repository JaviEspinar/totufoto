// The Photos tab: group cards, opening a group, grouping and sorting, the date range.
import { expect, test } from "@playwright/test";

test.beforeEach(async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".group-card").first()).toBeVisible();
});

test("cards by month, newest first, with the photo count", async ({ page }) => {
  await expect(page.locator(".view-head .count")).toContainText("12 photos");
  const june = page.locator(".group-card", { hasText: "June 2021" });
  await expect(june).toContainText("4 photos");
  const titles = await page.locator(".group-card .t").allInnerTexts();
  expect(titles.indexOf("August 2025")).toBeLessThan(titles.indexOf("June 2021"));
});

test("opening a month, then its chip goes back to the cards", async ({ page }) => {
  await page.locator(".group-card", { hasText: "December 2022" }).click();
  await expect(page.locator(".tile")).toHaveCount(3);
  await expect(page.locator(".view-head .chip")).toHaveCount(1);
  expect(page.url()).toContain("date=2022-12");
  await page.locator('.view-head [data-clear="date"]').click();
  await expect(page.locator(".group-card", { hasText: "December 2022" })).toBeVisible();
});

test("grouping by year and by place, and oldest first", async ({ page }) => {
  await page.selectOption("#groupBy", "year");
  await expect(page.locator(".group-card", { hasText: "2022" })).toContainText("3 photos");
  expect(page.url()).toContain("group=year");

  await page.selectOption("#groupBy", "place");
  await expect(page.locator(".group-card", { hasText: "Madrid" })).toContainText("4 photos");
  await expect(page.locator(".group-card", { hasText: "Paris" })).toContainText("3 photos");
  await expect(page.locator(".group-card", { hasText: "No location" })).toContainText("5 photos");

  await page.selectOption("#groupBy", "none");
  await page.selectOption("#sort", "asc");
  await expect(page.locator(".tile")).toHaveCount(12);
  expect(page.url()).toContain("sort=asc");
});

test("a date range shows as a chip, and Clear all removes it", async ({ page }) => {
  await page.fill("#fromDate", "2022-01-01");
  await page.fill("#toDate", "2024-06-30");
  await page.locator("#toDate").dispatchEvent("change");
  await expect(page.locator(".view-head .chip")).toHaveCount(1);
  await expect(page.locator(".view-head .count")).toContainText("5 photos");
  await page.locator('[data-clear="all"]').click();
  await expect(page.locator(".view-head .chip")).toHaveCount(0);
  await expect(page.locator(".view-head .count")).toContainText("12 photos");
});

test("slideshow: the filtered photos full screen, with its bar to change order and speed", async ({ page }) => {
  // A month's photos: the slideshow shows only those.
  await page.locator(".group-card", { hasText: "December 2022" }).click();
  await expect(page.locator(".tile")).toHaveCount(3);
  const ids = await page.locator(".tile").evaluateAll(tiles => tiles.map(t => t.dataset.id));
  await page.locator("[data-slideshow]").click();
  const show = page.locator("#slideshow");
  await expect(show).toBeVisible();
  const photo = show.locator(".ss-photo.on");
  const shownId = async () => (await photo.getAttribute("src"))?.match(/original\/(\d+)/)?.[1];
  await expect.poll(shownId).toBeTruthy();
  const first = await shownId();
  expect(ids).toContain(first);
  await expect(show.locator(".ss-pos")).toHaveText("1 of 3");

  // The bar hides by itself, and comes back when the mouse moves.
  const bar = show.locator(".ss-bar");
  await expect(bar).toBeHidden({ timeout: 6000 });
  await page.mouse.move(300, 300);
  await page.mouse.move(320, 310);
  await expect(bar).toBeVisible();

  // Fastest: the next photo comes within seconds, and is one of the month's too.
  await show.locator(".ss-speed input").fill("9");
  await expect(show.locator(".ss-seconds")).toHaveText("2 s per photo");
  await expect.poll(shownId, { timeout: 8000 }).not.toBe(first);
  expect(ids).toContain(await shownId());

  // Shuffled, remembered for next time; ✕ stops it.
  await show.locator('[data-ss-order="shuffle"]').click();
  await expect(show.locator('[data-ss-order="shuffle"]')).toHaveAttribute("aria-pressed", "true");
  await show.locator(".ss-close").click();
  await expect(show).toBeHidden();
  expect(await page.evaluate(() => localStorage.getItem("imadive.slideshowOrder"))).toBe("shuffle");

  // Esc stops it too, and the gallery is as it was.
  await page.locator("[data-slideshow]").click();
  await expect(show).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(show).toBeHidden();
  await expect(page.locator(".tile")).toHaveCount(3);
});
