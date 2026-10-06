// Videos: listed with the page's generic picture until the page makes a thumbnail from a
// frame, played by the browser (or offered for download when it can't), with their date and
// place from the file.
import { expect, test } from "@playwright/test";

test("videos: thumbnails made by the page, playing or offered for download", async ({ page }) => {
  // The folder with the videos is added for this test (the others count the photos only).
  const { folders } = await (await page.request.get("/api/folders")).json();
  const clips = folders[0].path.replace(/library$/, "clips");
  expect((await page.request.post("/api/folders", { data: { path: clips } })).ok()).toBe(true);
  await expect.poll(async () => (await (await page.request.get("/api/photos")).json()).photos.length, { timeout: 15_000 }).toBe(14);
  const rows = (await (await page.request.get("/api/photos")).json()).photos;
  const idOf = async name => {
    for (const [id, , , , , , duration] of rows) {
      if (duration == null) continue;
      if ((await (await page.request.get(`/api/photos/${id}`)).json()).path.endsWith(name)) return id;
    }
  };
  const [webm, mov] = [await idOf("green.webm"), await idOf("clip.mov")];

  await page.goto("/#group=none");
  await expect(page.locator(".tile.video")).toHaveCount(2);
  await expect(page.locator(".view-head .count")).toHaveText("12 photos, 2 videos");
  await expect(page.locator(`.tile[data-id="${mov}"] .vbadge`)).toHaveText("0:13");

  // The WebM can be decoded here: the page makes its thumbnail and the tile shows it.
  const green = page.locator(`.tile[data-id="${webm}"] img`);
  await expect(green).toHaveAttribute("src", new RegExp(`/thumb/${webm}/1$`), { timeout: 20_000 });
  const thumb = await page.request.get(`/thumb/${webm}/1`);
  expect(thumb.headers()["content-type"]).toBe("image/jpeg");
  // The MOV has no playable media: it keeps the page's own picture.
  await expect(page.locator(`.tile[data-id="${mov}"] img`)).toHaveAttribute("src", /^data:image\/svg\+xml/);

  await page.locator(`.tile[data-id="${mov}"]`).click();
  const video = page.locator("#viewer .frame video");
  await expect(video).toHaveAttribute("src", /\/original\//);
  const info = page.locator("#viewer .info-body");
  await expect(info).toContainText("Video · 0:13 · 1080 × 1920");
  await expect(info).toContainText("Madrid");
  await expect(info).not.toContainText("People (");
  // No playable media in this file: the viewer says so and offers the file.
  await expect(page.locator("#viewer .video-note")).toContainText("can't be played here");
  await expect(page.locator("#viewer .video-note a")).toHaveAttribute("href", /download=1/);
  await expect(video).toBeHidden();
  await expect(page.locator("#viewer .frame img")).toHaveAttribute("src", /^data:image\/svg\+xml/);
  await page.keyboard.press("Escape");
  await expect(page.locator("#viewer")).not.toHaveClass(/open/);

  expect((await page.request.delete(`/api/folders?path=${encodeURIComponent(clips)}`)).ok()).toBe(true);
});
