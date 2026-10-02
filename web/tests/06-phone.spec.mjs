// Phones: the people panel, and the Back button undoing one step at a time.
import { devices, expect, test } from "@playwright/test";

test.use({ ...devices["Pixel 7"], viewport: { width: 390, height: 800 } });

test("the people panel slides in and the scrim closes it", async ({ page }) => {
  await page.goto("/");
  await page.locator("#peopleBtn").tap();
  await expect(page.locator("body")).toHaveClass(/drawer-open/);
  await page.locator("#scrim").tap({ position: { x: 360, y: 400 } });
  await expect(page.locator("body")).not.toHaveClass(/drawer-open/);
});

test("Back closes the photo, clears the chips, then asks before leaving", async ({ page }) => {
  await page.goto("/api/status"); // the page before the gallery
  await page.goto("/");
  await page.locator(".group-card", { hasText: "June 2021" }).tap();
  await page.locator(".tile").first().tap();
  await expect(page.locator("#viewer")).toHaveClass(/open/);
  const back = () => page.evaluate(() => history.back());

  await back();
  await expect(page.locator("#viewer")).not.toHaveClass(/open/);
  await expect(page.locator(".view-head .chip")).toHaveCount(1);
  await back();
  await expect(page.locator(".view-head .chip")).toHaveCount(0);
  await back();
  await expect(page.locator("#choiceTitle")).toHaveText("Leave Imadive?");
  await page.locator('#choiceDlg .dlg-actions [data-choice="cancel"]').tap();
  await expect(page.locator(".group-card").first()).toBeVisible();
});
