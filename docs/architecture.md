# How Imadive works

A tour of the code for people who want to change it. For using the gallery, see the [user guide](user-guide.md); for the HTTP API, [api.md](api.md).

## The big picture

```
             ┌────────────── imadive (library crate, src/) ───────────────┐
 photo       │  scan ──► imaging, faces, geo ──► db (SQLite: index.sqlite) │
 folders ───►│    │                               ▲                       │
             │    └► cluster (faces → people)     │                       │
             │       duplicates (identical files) │                       │
             │                                    │                       │
             │  http (axum) ── guard ── library ──┘                       │
             │    └ serves web/ (index.html, app.css, js/, embedded)      │
             └────────────────────────────▲───────────────────────────────┘
                                          │ http://127.0.0.1:<port>
            src/main.rs (command line) ───┤
            desktop/ (Tauri window) ──────┘
```

There is one program: a library crate (`imadive`) that indexes photos into a SQLite file and serves a single-page UI and a JSON API over HTTP. Two thin front ends start it:

- **The command-line app** (`src/main.rs`) parses the options with clap, finds the face models and ONNX Runtime, and serves on `127.0.0.1:7878` (or `--host`/`--port`) for any browser.
- **The desktop app** (`desktop/`, Tauri 2) embeds the face models and ONNX Runtime in the executable, writes them to the app data folder on first start (`desktop/src/runtime.rs`), serves on a random loopback port and opens a window on that address. It adds native services through the `Host` trait (`src/app.rs`): the folder picker, opening links, showing a file in the file manager.

Both call `app::Gallery::open`, `start_scan` and `serve`, so the UI and the API are the same everywhere; the page asks `/api/folders` whether it runs in the desktop app (`"desktop": true`) to show or hide the native buttons.

## Source files

```
src/main.rs        command-line options and startup (the `cli` feature)
src/lib.rs         the library both apps are built on
src/app.rs         startup shared by both apps; the Host trait
src/scan.rs        file walking, the per-photo pipeline, batched database writes
src/imaging.rs     decoding, orientation, resizing, JPEG encoding
src/faces.rs       SCRFD detection, landmark alignment, ArcFace embeddings
src/cluster.rs     grouping faces into people
src/geo.rs         offline reverse geocoding
src/metadata.rs    the date and place a photo's EXIF data gives
src/duplicates.rs  finding and deleting identical files
src/rotate.rs      rotating photos in their files
src/library.rs     photo folders, and what may happen to the photos in them
src/guard.rs       request checks (DNS rebinding, cross-site requests)
src/db/            the SQLite index: schema and migrations (mod.rs), photos, people, filters
src/http/          the HTTP API: router, errors and pool (mod.rs), then one file per area
src/testutil.rs    test helpers: temporary libraries with generated JPEGs and EXIF
web/               the UI: index.html, app.css and js/ (the script, by area), built into the binary
web/tests/         browser tests (Playwright) on a fixture library
desktop/           the Tauri desktop app
fixtures/photos/   a public-domain photo for the desktop self-test
scripts/           downloads of the face models and ONNX Runtime, with checksums
```

## Indexing (`scan.rs`)

A scan runs in a background thread, one at a time: asking for a scan while one runs makes another follow it. Each pass:

1. **Lists the files** under the photo folders (`library::roots`: the command-line folders and the ones saved in Settings, without folders nested in others) and compares them with the index by path, size and modification time.
2. **Prunes** photos whose files are gone, except under a folder that can't be reached: a missing or empty folder counts as an unplugged drive. Photos removed from the gallery (`excluded`) stay out, and files that failed before are only tried again once they change.
3. **Processes** the new and changed files in parallel with rayon. Each file is read once: EXIF (date, GPS, orientation), decoding, the thumbnail (resized with SIMD by `fast_image_resize`), and the faces detected on the same decoded image, with one ONNX Runtime session per worker thread. A file that crashes a decoder or the models is a failed file, not a failed scan (`catch_unwind`).
4. **Writes** the results from one thread, in transactions of 64 photos, fed through a bounded channel. Changed files keep their photo id and get `version + 1`, so the page's cached thumbnails are fetched again.
5. **Groups the faces** (`cluster.rs`). Faces of named people are fixed; new faces are matched against them and the rest are clustered by cosine similarity. Normally only the new faces are placed; everything is regrouped on the first index, when most faces are new, or when asked.
6. **Looks for identical files** (`duplicates.rs`): only files that share their size with another are read and fingerprinted with BLAKE3.

Progress is kept in `ScanStatus` (atomics), which `/api/status` reads without touching the database. Busy flags are cleared by guards when dropped, so a panic never leaves the gallery stuck as busy.

## The index (`db/`)

One SQLite file, `index.sqlite`, in WAL mode with memory-mapped reads. The tables, and who writes them, are described at the top of `src/db/mod.rs`. In short:

- `items` (the photos and videos) with their `thumbs` and `faces` are written by the scan; handlers only forget items through `forget_items`, which first remembers the faces of named people.
- `persons` are created by grouping and by the user. A named person with no faces left is kept, with an average face (`face_memory`), so the name comes back when their photos do.
- `faces.rejected` marks faces the user placed ("Not them", "Same as"); grouping never moves them.
- `folders` (saved in Settings), `excluded` (removed from the gallery) and `failures` (files that could not be read).

New columns are added by a table of migrations (`ADDED_COLUMNS`), so an index from any earlier version opens and carries over. Indexes from 0.2 and before called the `items` table `photos` (and its references `photo_id`); `rename_photos_to_items` renames them in place when such an index is opened. Thumbnails live in SQLite too: for small blobs it is faster than the filesystem, and the whole library is one file to back up or delete.

The handlers never write SQL: `db/items.rs`, `db/people.rs` and `db/filters.rs` hold the queries and return typed rows that are serialized as they are.

## The server (`http/`, `guard.rs`, `library.rs`)

axum on tokio. Every database call runs on the blocking pool through a small connection pool (`http::Pool`), limited to twice the number of cores, since a page asks for dozens of thumbnails at a time. Errors are an `ApiError` with a status code, sent as `{"error": "..."}`.

`guard.rs` runs before every route. There is no login, so it only keeps other websites from using the gallery through a visitor's browser: the `Host` header must be an IP address, `localhost`, one of the computer's own names or an `--allow-host` name (this stops DNS rebinding), and requests that change something must come from the gallery's own page (`Sec-Fetch-Site`, or `Origin`). Anyone who can reach the server can use it; see the README's Privacy and security section and [SECURITY.md](../SECURITY.md).

`library.rs` decides what may happen to photos: which folders make up the library, adding and removing folders, checking a photo whose file has gone, and removing photos from the gallery or the disk. Only files inside the photo folders can be deleted or rotated.

Thumbnails and face pictures are served with immutable cache headers, keyed by the photo's `version` (`/thumb/{id}?v=`). The page's stylesheet and script are linked by a hash of their contents, so browsers keep them until an upgrade changes them.

## The page (`web/`)

Plain HTML, one stylesheet and one script, with no build step and no dependencies; `include_str!` puts them in the binary. The state of a view (tab, filters, grouping, order) lives in the address (`#view=people&people=3,5`), so Back, bookmarks and reloads work.

- **Rendering**: each view renders into `#main` under a render job (`{token, signal, alive()}`); starting a new view cancels the old job's requests and stops its rendering.
- **Photos**: a justified layout done in CSS (each tile's width comes from its aspect ratio), lazy images, and progressive rendering, so libraries with tens of thousands of photos stay smooth.
- **People**: a virtualised grid, keyed by person, that changes without flicker as names and merges come in.
- **Viewer**: a modal dialog with focus kept inside, zoom and swipe, the details panel and the face tools.
- **Accessibility**: cards and candidates are buttons, names and faces can be reached from the keyboard, and animations follow `prefers-reduced-motion`.

**Languages and theme.** Text is written in English in the code, inside `t("...")` or `tn(n, one, other)` (`js/i18n.js`); each other language is a dictionary from that English to its own (`js/i18n_es.js`), so text without a translation shows in English rather than breaking. `translatePage()` translates the fixed text of `index.html` as the page starts, and messages from the server are translated as they arrive, including those with a path in them (`"{path} does not exist"` matches them). Dates, numbers, country names and sorting follow the language through `Intl`. `web/tests/strings.mjs` checks in CI that every text has a translation in each language, with the same `{placeholders}`. The theme is `data-theme` on `<html>`, set from the saved choice by a line in `index.html`'s head before anything is drawn; without it, the system's choice applies.

**Video thumbnails.** The server doesn't decode video, so it has no thumbnail for a video until a camera left a `.THM` beside it or a page makes one: `/thumb` answers 204, the page shows its own generic picture, and `js/videothumbs.js` loads the start of the video in a hidden `<video>`, draws a frame from about a second in, and sends it (`PUT /api/items/{id}/thumb`). The server makes a normal thumbnail from it and raises the version, so every browser then gets the real one. Formats the browser can't decode keep the generic picture. The frame comes from the video without its sound (`/original/{id}?silent=1`): many phones' videos (lots of those sent with WhatsApp) begin with a broken sound packet that makes Chrome refuse the whole file, picture included. The server finds the sound tracks in the `moov` box (`video::sound_track_types`) and writes `free` over their box type as the bytes stream out, so players skip them and nothing else in the file moves. The viewer falls back to the same version, with a note, when the original won't play.

Small helpers in `js/core.js` (`api()` with typed errors, `icon()` for the SVG sprite in `index.html`, `plural()`, `pref` for remembered settings) and `askChoice()` for questions keep the rest short.

**The script's modules.** `web/js/` holds one ES module per area: `core` (helpers, the view state and the address, loading people and places), `sidebar`, `items` (the Photos and Upcoming tabs: the photos and videos in groups), `people`, `views` (`render()` and the main area's clicks), `viewer`, `settings`, `status`, and `main`, the entry point (the Back button and startup). Each imports what it uses from the others, with no build step: the page loads `/js/<hash>/main.js` and the browser follows the imports, all under the same content hash. `core` imports nothing, so it runs first; parts that react to it register a callback instead (`onPeopleLoaded`). Imported variables can't be assigned, so a variable is only changed by its own module, through a small function when others need to (`setPeopleSort`, `setLastStatus`, `rememberCards`).

Modules that import each other in a cycle run in an order the imports decide. `web/tests/script-order.mjs` works that order out and checks that nothing running at startup uses a `let` or `const` before its module has run (functions are fine anywhere), that every file is reached from `main`, and that every file parses. It runs in CI and in `just lint`. A new module goes into `SCRIPTS` in `src/http/mod.rs` so the server can serve it.

## The desktop app (`desktop/`)

- **Embedded models and runtime**: the face models (about 16 MB) and ONNX Runtime (and on Windows the Visual C++ runtime it needs) are in the executable, written to `runtime/<version>/` in the app data folder on first start. This makes the first start work offline with nothing to install, at the cost of a bigger download; it also means the executable isn't signed by anyone Windows knows, so SmartScreen warns about it.
- **One window**: a second launch brings the open window to the front.
- **Log**: Windows release builds have no console, so the app logs to `logs/imadive.log` in its data folder as well as to the terminal (`init_desktop_logging`, `log_to_file`), keeping the previous run's log as `imadive.old`.
- **Self-test**: `imadive-desktop --self-test [photos] [report]` loads face recognition and indexes the photos without opening a window. CI runs it on every build, on `fixtures/photos`.
- **Old installations**: the library of a Totufoto installation (`com.javiespinar.totufoto`) is moved to the new folder on first start.

## Tests

- `cargo test`: unit tests next to the code, and the `testutil` fixtures (temporary libraries with generated JPEGs carrying real EXIF dates and GPS). They cover the destructive paths (deleting, rotating, removing folders, pruning), the filters, migrations, the request guard and the JSON shapes the page reads.
- `web/tests`: Playwright drives Chromium against the real program on a fixture library, with people written into the index (the tests run without face recognition). See [CONTRIBUTING.md](../CONTRIBUTING.md) for how to run them.
- The desktop self-test, in `desktop.yml`.

## Speed

Measured in September 2026 on an 8-core Apple Silicon Mac: 300 photos at 12 MP fully indexed (thumbnails and faces) in 8.7 s. The main reasons:

- one read and one decode per file, with all the work on the same decoded image;
- every core busy (rayon), and one ONNX Runtime session per worker;
- one writer thread with batched transactions;
- SQLite for thumbnails, served with immutable cache headers;
- a page without a framework, which only builds what is on screen.
