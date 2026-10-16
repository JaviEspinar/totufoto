// Settings: updates (against start-server.mjs's stand-in for GitHub, which has 99.0.0),
// backing up and restoring the index, and the invitation to support the project.
import { expect, test } from "@playwright/test";

const openSettings = async page => {
  await page.goto("/");
  await page.locator("#settingsBtn").click();
  await expect(page.locator("#settingsDlg")).toBeVisible();
};

test("Settings ends with the invitation to buy a coffee", async ({ page }) => {
  await openSettings(page);
  const coffee = page.locator("#settingsDlg .settings-coffee");
  await expect(coffee).toContainText("ImaDive is free");
  await expect(coffee.locator("a")).toHaveAttribute("href", /ko-fi\.com\/waiting4timeout/);
});

test("a newer release is found, and a build from source says how to update", async ({ page }) => {
  await openSettings(page);
  const update = page.locator("#settingsUpdate");
  await expect(update).toContainText("This is Imadive");
  await expect(update).toContainText("only looks for new releases when you ask");
  await update.locator("[data-update-check]").click();
  await expect(update).toContainText("Imadive 99.0.0 is available");
  await expect(update.locator("a", { hasText: "What's new" })).toHaveAttribute("href", /releases\/tag\/v99\.0\.0$/);
  // This copy can't install it: it says what to run instead.
  await expect(update.locator("[data-update-install]")).toHaveCount(0);
  await expect(update.locator(".update-cmd code")).toContainText("cargo build --release");
  // The gear shows there is one, on this page and the next.
  await page.reload();
  await expect(page.locator("#settingsBtnDot")).toBeVisible();
});

test("a backup brings back what changed after it", async ({ page }) => {
  await openSettings(page);
  const backup = page.locator(".set-part", { has: page.locator("[data-backup]") });
  await expect(backup).toContainText("A backup keeps people and their names");
  const downloading = page.waitForEvent("download");
  await backup.locator("[data-backup]").click();
  const download = await downloading;
  expect(download.suggestedFilename()).toMatch(/^Imadive backup \d{4}-\d\d-\d\d\.imadive$/);
  const file = await download.path();

  // After the backup, Ana gets another name.
  const people = async () => (await (await page.request.get("/api/people")).json()).map(p => p.name);
  await page.request.patch("/api/people/1", { data: { name: "Zed" } });
  expect(await people()).toContain("Zed");

  // The backup is described before anything changes.
  await page.locator("#restoreFile").setInputFiles(file);
  const dialog = page.locator("#choiceDlg");
  await expect(dialog).toContainText("Restore this backup?");
  await expect(dialog).toContainText("2 named people");
  await expect(dialog.locator(".restore-folders")).toContainText("library");
  await dialog.locator('[data-choice="restore"]').click();
  // The page starts again on the restored index.
  await page.waitForEvent("load");
  await expect.poll(people).toContain("Ana");
  expect(await people()).not.toContain("Zed");
  // Photo thumbnails, left out of backups, are made again.
  await expect(page.locator(".tile img.ok, .group-card img").first()).toBeVisible();
});

test("a file that isn't a backup is refused", async ({ page }) => {
  await openSettings(page);
  await page.locator("#restoreFile").setInputFiles({ name: "notes.imadive", mimeType: "application/octet-stream", buffer: Buffer.from("hello") });
  await expect(page.locator("#toast")).toContainText("isn't an Imadive backup");
});
