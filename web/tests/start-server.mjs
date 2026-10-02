// Starts Imadive on a temporary copy of the fixture library, for the UI tests (Playwright's
// webServer runs this). The copy gets two exact duplicates and a photo taken two years ago
// today; after indexing, three people with their faces are written into the index, since the
// tests run without face recognition. Everything is deleted when the server stops.
//
// The program is ../../target/release/imadive, or IMADIVE_BIN.

import { spawn, spawnSync } from "node:child_process";
import { cpSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const bin = process.env.IMADIVE_BIN ?? resolve(here, "../../target/release/imadive");
const port = process.env.PORT ?? "7979";

// The real path, as the app stores it (on macOS the temporary folder is behind a link).
const dir = realpathSync(mkdtempSync(join(tmpdir(), "imadive-ui-")));
const library = join(dir, "library"), data = join(dir, "data"), other = join(dir, "other");
cpSync(join(here, "fixtures/library"), library, { recursive: true });

// Two copies of the same photo; the original's file is older, so it is the one kept.
const longAgo = new Date("2021-06-12T11:00:00");
utimesSync(join(library, "2021/beach-1.jpg"), longAgo, longAgo);
mkdirSync(join(library, "copies"));
for (const name of ["beach-1 (copy).jpg", "beach-1 (copy 2).jpg"]) copyFileSync(join(library, "2021/beach-1.jpg"), join(library, "copies", name));
// No EXIF date, so the file's date counts: two years ago today, at noon, for Upcoming.
const twoYearsAgo = new Date();
twoYearsAgo.setFullYear(twoYearsAgo.getFullYear() - 2);
twoYearsAgo.setHours(12, 0, 0, 0);
utimesSync(join(library, "plain.jpg"), twoYearsAgo, twoYearsAgo);
// A second folder, to add and remove in Settings.
mkdirSync(other);
copyFileSync(join(library, "2025/city.jpg"), join(other, "street.jpg"));
writeFileSync(join(dir, "README"), "Temporary library for Imadive's UI tests.\n");

const index = spawnSync(bin, [library, "--data", data, "--no-faces", "--scan-only"], { stdio: "inherit" });
if (index.status !== 0) {
  console.error(`indexing the test library failed (${bin})`);
  process.exit(1);
}

// People: Ana (named), Ben (named) and someone unnamed, with faces on some photos.
const db = new DatabaseSync(join(data, "index.sqlite"));
const id = rel => db.prepare("SELECT id FROM photos WHERE path = ?").get(join(library, rel)).id;
db.exec("INSERT INTO persons (id, name) VALUES (1, 'Ana'), (2, 'Ben'), (3, NULL)");
const picture = readFileSync(join(library, "plain.jpg"));
const embedding = new Uint8Array(512 * 4);
const addFace = db.prepare(
  "INSERT INTO faces (photo_id, x, y, w, h, score, embedding, thumb, person_id, grouped) VALUES (?, ?, 0.2, 0.25, 0.3, 0.9, ?, ?, ?, 1)",
);
const faces = [
  ["2021/beach-1.jpg", [1, 2]], ["2021/beach-2.jpg", [1]], ["2022/dinner-1.jpg", [1, 2]], ["2022/dinner-2.jpg", [2]],
  ["2022/dinner-3.jpg", [3]], ["2024/park-1.jpg", [1]], ["2025/city.jpg", [3]],
];
for (const [rel, people] of faces) people.forEach((person, i) => addFace.run(id(rel), 0.1 + 0.4 * i, embedding, picture, person));
db.close();

const server = spawn(bin, [library, "--data", data, "--no-faces", "--port", port], { stdio: "inherit" });
const stop = () => { server.kill(); };
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
server.on("exit", code => {
  rmSync(dir, { recursive: true, force: true });
  process.exit(code ?? 0);
});
