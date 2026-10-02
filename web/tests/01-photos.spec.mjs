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
