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
// A folder with a video, to add in the video test: a QuickTime file with a 12.5-second,
// 1920x1080 track turned upright, recorded 2024-10-05 18:22 in Madrid. It has no playable
// media, so the viewer's "can't be played here" path shows.
const clips = join(dir, "clips");
mkdirSync(clips);
writeFileSync(join(clips, "clip.mov"), sampleMov());
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

/** The same small movie as src/video.rs's tests build. */
function sampleMov() {
  const u32 = n => { const b = Buffer.alloc(4); b.writeUInt32BE(n >>> 0); return b; };
  const i32 = n => { const b = Buffer.alloc(4); b.writeInt32BE(n); return b; };
  const bx = (kind, ...parts) => { const body = Buffer.concat(parts); return Buffer.concat([u32(body.length + 8), Buffer.from(kind, "latin1"), body]); };
  const created = 1_728_145_321 + 2_082_844_800;
  const mvhd = bx("mvhd", Buffer.alloc(4), u32(created), u32(created), u32(1000), u32(12_500), Buffer.alloc(80));
  const matrix = [0, 0x10000, 0, -0x10000, 0, 0, 0, 0, 0x4000_0000].map(i32);
  const tkhd = bx("tkhd", Buffer.alloc(40), ...matrix, u32(1920 * 65536), u32(1080 * 65536));
  const hdlr = bx("hdlr", Buffer.alloc(8), Buffer.from("vide"), Buffer.alloc(13));
  const trak = bx("trak", tkhd, bx("mdia", hdlr));
  const key = name => bx("mdta", Buffer.from(name));
  const keys = bx("keys", Buffer.alloc(4), u32(2), key("com.apple.quicktime.creationdate"), key("com.apple.quicktime.location.ISO6709"));
  const value = (index, text) => Buffer.concat([u32(Buffer.byteLength(text) + 24), u32(index), bx("data", u32(1), Buffer.alloc(4), Buffer.from(text))]);
  const ilst = bx("ilst", value(1, "2024-10-05T18:22:01+0200"), value(2, "+40.4168-003.7038+654.000/"));
  const meta = bx("meta", bx("hdlr", Buffer.alloc(8), Buffer.from("mdta"), Buffer.alloc(13)), keys, ilst);
  return Buffer.concat([bx("ftyp", Buffer.from("qt  \0\0\0\0qt  ", "latin1")), bx("mdat", Buffer.alloc(64, 7)), bx("moov", mvhd, trak, meta)]);
}

