// The page and every tab load without a script error (a module that fails as it starts
// would break parts of the page no other test looks at).
import { expect, test } from "@playwright/test";

test("every tab opens without a script error", async ({ page }) => {
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  page.on("console", m => { if (m.type() === "error") errors.push(m.text()); });
  await page.goto("/");
  await expect(page.locator(".group-card").first()).toBeVisible();
  for (const view of ["upcoming", "people", "optimization", "photos"]) {
    await page.locator(`nav [data-view="${view}"]`).click();
    await expect(page.locator("#main")).not.toBeEmpty();
  }
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#settingsDlg .about")).toBeVisible();
  expect(errors).toEqual([]);
});
