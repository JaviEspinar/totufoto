// Videos: listed with a generic picture and their length, played by the browser (or offered
// for download when it can't), with their date and place from the file.
import { expect, test } from "@playwright/test";

test("a video is listed, opened, and offered for download when it can't play", async ({ page }) => {
  // The folder with the video is added for this test (the others count the photos only).
  const { folders } = await (await page.request.get("/api/folders")).json();
  const clips = folders[0].path.replace(/library$/, "clips");
  expect((await page.request.post("/api/folders", { data: { path: clips } })).ok()).toBe(true);
  await expect.poll(async () => (await (await page.request.get("/api/photos")).json()).photos.length, { timeout: 15_000 }).toBe(13);

  await page.goto("/#group=none");
  const tile = page.locator(".tile.video");
  await expect(tile).toHaveCount(1);
  await expect(tile.locator(".vbadge")).toHaveText("0:13");
  await expect(page.locator(".view-head .count")).toHaveText("12 photos, 1 video");

  await tile.click();
  const video = page.locator("#viewer .frame video");
  await expect(video).toHaveAttribute("src", /\/original\//);
  const info = page.locator("#viewer .info-body");
  await expect(info).toContainText("Video · 0:13 · 1080 × 1920");
  await expect(info).toContainText("Madrid");
  await expect(info).not.toContainText("People (");
  // No playable media in this file: the viewer says so and offers the file.
  await expect(page.locator("#viewer .video-note")).toContainText("can't be played here");
  await expect(page.locator("#viewer .video-note a")).toHaveAttribute("href", /download=1/);
  // The player that couldn't play gives way to the video's picture.
  await expect(video).toBeHidden();
  await expect(page.locator("#viewer .frame img")).toHaveAttribute("src", /\/thumb\//);
  await page.keyboard.press("Escape");
  await expect(page.locator("#viewer")).not.toHaveClass(/open/);

  expect((await page.request.delete(`/api/folders?path=${encodeURIComponent(clips)}`)).ok()).toBe(true);
});
