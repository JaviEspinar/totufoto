// People: the sidebar's combinations, renaming with unique names, merging.
// (These tests change the people, so they run after the read-only ones.)
import { expect, test } from "@playwright/test";

/** Ticks people in the sidebar by name. */
async function select(page, ...names) {
  for (const name of names) await page.locator("#peopleList .person", { hasText: name }).locator("input").check();
}

test("together, any and only them", async ({ page }) => {
  await page.goto("/#group=none");
  await expect(page.locator("#peopleList .person")).toHaveCount(3);
  await select(page, "Ana", "Ben");
  await expect(page.locator(".view-head .count")).toContainText("2 photos");
  await page.locator('#match [data-m="any"]').click();
  await expect(page.locator(".view-head .count")).toContainText("5 photos");
  await page.locator('#match [data-m="only"]').click();
  await expect(page.locator(".view-head .count")).toContainText("2 photos");
  await page.locator('[data-clear="all"]').click();
  await expect(page.locator(".view-head .count")).toContainText("12 photos");
});

test("People: classified and not, and a person's photos without the filters set before", async ({ page }) => {
  // A date range set before opening People.
  await page.goto("/#view=people&from=2025-01-01");
  await expect(page.locator(".face-card:not(.skeleton)")).toHaveCount(3);
  await expect(page.locator("#peopleNamed h2")).toContainText("Classified 2");
  await expect(page.locator("#peopleUnnamed h2")).toContainText("Not classified 1");
  await expect(page.locator("#peopleUnnamed .face-card input")).toHaveValue("");
  const ana = page.locator("#peopleNamed .face-card", { has: page.locator('input[value="Ana"]') });
  const photos = (await ana.locator(".s").textContent()).trim();
  await ana.locator(".avatar").click();
  // Only Ana: the date range is gone, and so is any other filter.
  await expect(page.locator(".view-head .count")).toContainText(photos);
  expect(page.url()).not.toContain("from=");
  await expect(page.locator(".chip")).toHaveCount(1);
  // Ticking people in the sidebar still combines with the other filters.
  await page.goto("/#group=none&from=2025-01-01");
  await select(page, "Ana");
  expect(page.url()).toContain("from=2025-01-01");
});

test("the People tab, and names stay unique", async ({ page }) => {
  await page.goto("/#view=people");
  await expect(page.locator(".face-card:not(.skeleton)")).toHaveCount(3);
  const unnamed = page.locator(".face-card", { hasText: "2 photos" }).locator("input");
  await unnamed.fill("ana");
  await unnamed.press("Enter");
  // Someone is already called Ana: same person, or keep them apart?
  await expect(page.locator("#nameDlg")).toHaveAttribute("open");
  await page.locator("#nameKeep").click();
  await expect(page.locator(".face-card input").nth(2)).toHaveValue("ana (1)");
});

test("Same as... merges two people", async ({ page }) => {
  await page.goto("/#view=people");
  await expect(page.locator(".face-card:not(.skeleton)")).toHaveCount(3);
  await page.locator(".face-card", { has: page.locator('input[value="Ben"]') }).locator("[data-merge]").click();
  await page.locator("#mergeDlg .candidate", { has: page.locator(".n", { hasText: /^Ana$/ }) }).click();
  await page.locator("#mergeConfirm [data-confirm]").click();
  await expect(page.locator(".vcard:not(.leave) .face-card")).toHaveCount(2);
  await expect(page.locator("#toast")).toContainText("Merged Ben into Ana");
});
