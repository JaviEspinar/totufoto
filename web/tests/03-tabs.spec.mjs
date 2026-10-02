// Upcoming, Optimization and Settings.
import { expect, test } from "@playwright/test";

test("Upcoming shows the photo taken two years ago today", async ({ page }) => {
  await page.goto("/#view=upcoming");
  // Other photos may have their anniversary in the next 30 days too, depending on the date.
  await expect(page.locator(".tile").first()).toBeVisible();
  await expect(page.locator("#main")).toContainText("2 years ago");
});

test("Optimization lists the identical files and keeps the oldest", async ({ page }) => {
  await page.goto("/#view=optimization");
  const set = page.locator(".dup-group");
  await expect(set).toHaveCount(1, { timeout: 15_000 });
  await expect(set.locator(".dup-file.keep")).toContainText("2021");
  await expect(set.locator(".dup-file.remove")).toHaveCount(2);
  // Asking and cancelling deletes nothing.
  await page.locator("[data-dup-delete]").click();
  await page.locator('#choiceDlg .dlg-actions [data-choice="cancel"]').click();
  await expect(set.locator(".dup-file.remove")).toHaveCount(2);
});

test("Settings: About, adding and removing a folder", async ({ page }) => {
  await page.goto("/");
  await page.locator("#settingsBtn").click();
  const settings = page.locator("#settingsDlg");
  await expect(settings.locator(".about")).toContainText("Imadive");
  await expect(settings.locator(".folder")).toHaveCount(1);
  await expect(settings.locator(".folder .fixed")).toHaveText("command line");

  // The browser opens next to the library: "library" and "other" are there.
  await settings.locator("[data-browse]").click();
  const browse = page.locator("#browseDlg");
  await expect(browse.locator("[data-dir]", { hasText: "other" })).toBeVisible();
  await browse.locator("[data-dir]", { hasText: "library" }).click();
  await browse.locator("[data-add-here]").click();
  await expect(browse.locator(".err")).toContainText("already in the gallery");
  await browse.locator("[data-up]").click();
  await browse.locator("[data-dir]", { hasText: "other" }).click();
  await browse.locator("[data-add-here]").click();
  await expect(browse).not.toHaveAttribute("open");
  await expect(settings.locator(".folder")).toHaveCount(2);

  await settings.locator("[data-remove]").click();
  await page.locator('#choiceDlg [data-choice="remove"]').click();
  await expect(settings.locator(".folder")).toHaveCount(1);
  await expect(page.locator("#toast")).toContainText("1 photo removed from the gallery");
});
