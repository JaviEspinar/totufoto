// Settings: updates (against start-server.mjs's stand-in for GitHub, which has 99.0.0),
// backing up and restoring the index, and the invitation to support the project.
import { mkdtempSync, readdirSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "@playwright/test";

/** Opens `folder` in the folder browser, once its first listing (next to the library) is in. */
const goTo = async (page, folder) => {
  await expect(page.locator("#browseDlg .browse-list button", { hasText: "library" })).toBeVisible();
  await page.locator("#browsePath").fill(folder);
  await page.locator("#browsePath").press("Enter");
  await expect(page.locator("#browsePath")).toHaveValue(folder);
};

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

test("a backup is saved in a folder of the server, with its progress at once, and restored from there", async ({ page }) => {
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "imadive-backups-")));
  try {
    await openSettings(page);
    const card = page.locator(".set-part", { has: page.locator("[data-backup]") });
    await expect(card).toContainText("A backup keeps people and their names");
    await card.locator("[data-backup]").click();
    const browse = page.locator("#browseDlg");
    await expect(browse.locator("#browseTitle")).toHaveText("Where to save the backup");
    await goTo(page, folder);
    await browse.locator("[data-add-here]").click();
    // Said at once, while the backup is being made.
    await expect(browse).toBeHidden();
    await expect(card.locator("#backupProgress progress")).toBeVisible();
    await expect(card.locator("#backupProgress")).toContainText("Making the backup");
    await expect(card.locator("[data-backup]")).toBeDisabled();
    await expect(card.locator("#backupProgress")).toContainText("Saved in", { timeout: 10_000 });
    const saved = readdirSync(folder);
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatch(/^Imadive backup \d{4}-\d\d-\d\d\.imadive$/);

    // After the backup, Ana gets another name.
    const people = async () => (await (await page.request.get("/api/people")).json()).map(p => p.name);
    await page.request.patch("/api/people/1", { data: { name: "Zed" } });
    expect(await people()).toContain("Zed");

    // Restore…: the backup is in the folder browser, and described before anything changes.
    await card.locator("[data-restore]").click();
    await expect(browse.locator("#browseTitle")).toHaveText("Choose a backup");
    await expect(browse.locator("[data-add-here]")).toBeHidden();
    await goTo(page, folder);
    await browse.locator("button.file", { hasText: saved[0] }).click();
    const dialog = page.locator("#choiceDlg");
    await expect(dialog).toContainText("Restore this backup?");
    await expect(dialog).toContainText("2 named people");
    await expect(dialog.locator(".restore-folders")).toContainText("library");
    await dialog.locator('[data-choice="restore"]').click();
    // The page starts again on the restored index.
    await page.waitForEvent("load");
    await expect.poll(people).toContain("Ana");
    expect(await people()).not.toContain("Zed");
    // The backup file chosen is left as it was, and the photos' thumbnails are made again.
    expect(readdirSync(folder)).toEqual(saved);
    await expect(page.locator(".tile img.ok, .group-card img").first()).toBeVisible();
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
});

test("a backup can also be uploaded from this device, and other files are refused", async ({ page }) => {
  await openSettings(page);
  await page.locator("#settingsDlg [data-restore]").click();
  const choosing = page.waitForEvent("filechooser");
  await page.locator("#browseDlg [data-extra]").click();
  await (await choosing).setFiles({ name: "notes.imadive", mimeType: "application/octet-stream", buffer: Buffer.from("hello") });
  await expect(page.locator("#toast")).toContainText("isn't an Imadive backup");
});
