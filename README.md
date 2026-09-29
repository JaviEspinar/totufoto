# totufoto

A fast local photo gallery written in Rust. Point it at your photo folders and it indexes them in parallel, then serves a web UI at `http://127.0.0.1:7878`.

## Features

- **Timeline** sorted by capture date (EXIF `DateTimeOriginal`, falling back to the file date), newest or oldest first, grouped by day, month, year or place.
- **Places**: GPS coordinates are turned into cities with an offline reverse geocoder (GeoNames cities with more than 1000 inhabitants). No network calls.
- **Upcoming**: photos from previous years whose anniversary falls in the next 7 to 90 days ("2 years ago today").
- **People**: faces are detected (SCRFD), aligned and turned into 512-d ArcFace embeddings, then grouped into people automatically. Name them, merge duplicates, hide people, or remove a wrong face ("Not them").
- **People combinations**: tick people in the sidebar and choose
  - *Together*: every selected person is in the photo,
  - *Any*: at least one of them,
  - *Only them*: all of them and no other known person.
- All filters combine (people + place + date + upcoming) and live in the URL, so views can be bookmarked.

## Getting started

### 1. Install Rust

You need Rust 1.88 or newer. If you don't have it:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On macOS you also need the Xcode command line tools (`xcode-select --install`). On Linux you need a C compiler (for example `sudo apt install build-essential`).

### 2. Get the code and the face models

```sh
git clone https://github.com/JaviEspinar/totufoto.git
cd totufoto
scripts/fetch-models.sh
```

`fetch-models.sh` downloads the face detection and recognition models (about 16 MB) into `./models`. They are not stored in the repository because of their license (see below). Without them the gallery still works, just without people.

### 3. Start it on a photo folder

```sh
cargo run --release -- ~/Pictures/Holidays
```

The first build takes a minute or two. Then open **http://127.0.0.1:7878**. Indexing runs in the background with progress shown in the top-right corner, and photos appear as soon as it finishes. Press `Ctrl+C` in the terminal to stop the app.

Subfolders are included automatically, and you can pass several folders:

```sh
cargo run --release -- ~/Pictures/2023 ~/Pictures/2024 /Volumes/Backup/Photos
```

To skip the build step next time, run the compiled binary directly: `./target/release/totufoto ~/Pictures/Holidays`.

### Apple Photos library

Point it at the originals inside the library package. The gallery only reads files and never changes the Photos library:

```sh
cargo run --release -- ~/Pictures/"Photos Library.photoslibrary"/originals
```

Only photos stored on the Mac are found. Photos kept only in iCloud ("Optimize Mac Storage") are skipped.

## Using the gallery

- **Photos**: the timeline. Use *Group* (day, month, year, place, none) and *Newest/Oldest first* in the top bar.
- **Places**: one card per city. Click a card to see its photos.
- **Upcoming**: memories from past years for the next 7 to 90 days.
- **People**: name each person by typing under their face, use *Same as…* to merge two groups of the same person, and *Hide* for people you don't care about.
- **Sidebar checkboxes**: tick one or more people to filter any view, and pick *Together*, *Any* or *Only them* to decide how they combine.
- **Viewer**: click a photo to open it. Use the arrow keys to move and `Esc` to close. The side panel shows the date, place and people. *Not them* removes a wrongly grouped face, and *Show face boxes* draws the detected faces.
- **Rescan**: picks up new, changed or deleted photos without restarting.

## The index

Everything the gallery learns is stored in `totufoto-data/index.sqlite`: metadata, thumbnails, faces, names and corrections. Your photos are never modified or copied.

- Stopping and starting again does **not** re-index. Only new or changed files are processed, and deleted ones are removed.
- Photos are tracked by path. Moving or renaming the photo folder makes them look new, so they are indexed again. Named people are matched to the re-indexed faces automatically.
- Use `--data DIR` to keep separate indexes for separate libraries. Delete the data folder to start from scratch.

## Options

Run `totufoto --help` for the full list:

| flag | default | |
|---|---|---|
| `--data DIR` | `totufoto-data` | where the SQLite index (with thumbnails) is stored |
| `--models DIR` | `models` | folder with `det_500m.onnx` and `w600k_mbf.onnx` |
| `--port` / `--host` | `7878` / `127.0.0.1` | |
| `--face-threshold` | `0.42` | cosine similarity to treat two faces as the same person; raise it if different people get mixed, lower it if one person is split |
| `--no-faces` | | skip face recognition |
| `--scan-only` | | index and exit |

When passing options through cargo, put them after `--`, for example `cargo run --release -- ~/Pictures --port 8080`.

Supported formats: JPEG, PNG, WebP, TIFF, GIF, BMP, and HEIC/HEIF on macOS (decoded with `sips`).

## How it's fast

- One pass per file: the file is read once, EXIF parsed, decoded, resized with SIMD (`fast_image_resize`), and faces are detected on the same buffer, spread over all cores with `rayon`, one ONNX Runtime session per worker thread.
- Thumbnails live inside SQLite (WAL, mmap), served with immutable cache headers; SQLite is faster than the filesystem for small blobs.
- The UI is a single dependency-free HTML page with a CSS-only justified layout, lazy images, and progressive rendering, so libraries with tens of thousands of photos stay smooth.

Measured on an 8-core Apple Silicon Mac: 300 photos at 12 MP fully indexed (thumbnails + faces) in 8.7 s.

## Layout

```
src/main.rs     CLI and startup
src/scan.rs     file walking, per-photo pipeline, batched DB writes
src/faces.rs    SCRFD detection, landmark alignment, ArcFace embeddings
src/cluster.rs  grouping faces into people
src/geo.rs      offline reverse geocoding
src/db.rs       SQLite schema
src/server.rs   HTTP API (axum)
web/index.html  UI (embedded into the binary)
```

## Face models license

The InsightFace `buffalo_s` models are released for non-commercial research purposes. For commercial use, swap in models with a suitable license (any SCRFD-style detector with 5 landmarks and a 112x112 ArcFace embedder works).
