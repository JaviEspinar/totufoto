// Settings → Appearance: the theme and the language, remembered in this browser.
import { expect, test } from "@playwright/test";

const background = page => page.evaluate(() => getComputedStyle(document.body).backgroundColor);

test("the theme can be chosen, and stays after a reload", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  const light = await background(page);
  await page.locator("#settingsBtn").click();
  await page.selectOption("#themePref", "dark");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  expect(await background(page)).not.toBe(light);

  // Applied before the page is drawn: already there when the document is parsed.
  await page.reload({ waitUntil: "domcontentloaded" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

  // System follows the system again.
  await page.locator("#settingsBtn").click();
  await page.selectOption("#themePref", "");
  await expect(page.locator("html")).not.toHaveAttribute("data-theme", /./);
  expect(await background(page)).toBe(light);
});

test("Spanish: the page, its numbers and the server's messages", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator(".group-card").first()).toBeVisible();
  await page.locator("#settingsBtn").click();
  await Promise.all([page.waitForEvent("load"), page.selectOption("#langPref", "es")]);
  await expect(page.locator("html")).toHaveAttribute("lang", "es");
  await expect(page.locator('nav [data-view="people"]')).toHaveText("Personas");
  await expect(page.locator(".view-head .count")).toContainText("12 fotos");
  await expect(page.locator(".group-card", { hasText: "junio de 2021" })).toContainText("4 fotos");

  await page.locator("#settingsBtn").click();
  const settings = page.locator("#settingsDlg");
  await expect(settings.locator("h4", { hasText: "Carpetas de fotos" })).toBeVisible();
  // A message from the server, translated with the path it names.
  await settings.locator("[data-browse]").click();
  const browse = page.locator("#browseDlg");
  await browse.locator("[data-dir]", { hasText: "library" }).click();
  await browse.locator("[data-add-here]").click();
  await expect(browse.locator(".err")).toContainText("ya está en la galería");
});

test("without a choice: the browser's language if the page has it, else English", async ({ browser, baseURL }) => {
  const tabs = async languages => {
    const context = await browser.newContext({ baseURL });
    await context.addInitScript(list => {
      Object.defineProperty(navigator, "languages", { get: () => list });
      Object.defineProperty(navigator, "language", { get: () => list[0] });
    }, languages);
    const page = await context.newPage();
    await page.goto("/");
    const text = await page.locator('nav [data-view="people"]').innerText();
    await context.close();
    return text;
  };
  expect(await tabs(["es-ES", "en"])).toBe("Personas");
  expect(await tabs(["es-MX"])).toBe("Personas");
  expect(await tabs(["fr-FR", "es-ES"])).toBe("People"); // Spanish only as a second choice
  expect(await tabs(["de-DE"])).toBe("People");
  expect(await tabs(["en-GB", "es-ES"])).toBe("People");
});
