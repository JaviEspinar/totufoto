# totufoto

A fast local photo gallery written in Rust. Point it at your photo folders and it indexes them in parallel: timeline, places, upcoming anniversaries and people found by face recognition. Everything runs on your computer and your photos are never uploaded; the files only change when you ask (rotating or deleting a photo).

It comes as a desktop app for Windows and Linux, and as a command-line app that serves the gallery to your browser. It is free for personal and other non-commercial use, and its source code is available ([license](#license)).

## Features

- **Timeline** sorted by capture date (EXIF `DateTimeOriginal`, falling back to the file date), newest or oldest first, grouped by day, month, year or place.
- **Places**: GPS coordinates are turned into cities with an offline reverse geocoder (GeoNames cities with more than 1000 inhabitants). No network calls. Group photos *by place* to see one card per city.
- **Upcoming**: photos from previous years whose anniversary falls in the next 7 to 90 days ("2 years ago today").
- **People**: faces are detected (SCRFD), aligned and turned into 512-d ArcFace embeddings, then grouped into people automatically. Name them, merge duplicates, hide people, or move a wrongly grouped face to a group of its own ("Not them").
- **People combinations**: tick people in the sidebar and choose
  - *Together*: every selected person is in the photo,
  - *Any*: at least one of them,
  - *Only them*: all of them and no other known person.
- All filters combine (people + place + date range + upcoming) and live in the URL, so views can be bookmarked.

## Privacy and security

- **Private by design**: everything runs on your computer. There is no account, no cloud and no telemetry; places are found with an offline city list, and the only network requests are the ones you make yourself (opening a map link).
- **No login**: by default the gallery only accepts this computer. If you share it with your network (`--host 0.0.0.0`, to use it from a phone), anyone who can reach it can see your photos, rotate them and delete them, and add any folder of the computer to the gallery (so also see and delete images you never shared). Only do that on a network you trust, such as your home network, and never expose it to the internet.
- **Other websites can't use it**: requests for a name other than an IP address, `localhost`, this computer's own name or one you allow with `--allow-host` are refused (this blocks DNS rebinding), and requests that change something are refused when they come from another site.

To report a security problem, see [SECURITY.md](SECURITY.md).

## Desktop app (Windows and Linux)

Download the file for your system from the [latest release](https://github.com/JaviEspinar/totufoto/releases/latest) and run it. There is nothing to install: face recognition and everything it needs are built in.

- **Windows 10/11**: `Totufoto-<version>-windows-x64.exe`. Double-click it. It is not signed, so the first time Windows SmartScreen may say "Windows protected your PC": click **More info** → **Run anyway**.
- **Linux (x86-64)**: `Totufoto-<version>-linux-x86_64.AppImage`. Make it executable and run it:
  ```sh
  chmod +x Totufoto-*.AppImage
  ./Totufoto-*.AppImage
  ```
  If it says FUSE is missing, install it (`sudo apt install libfuse2` on Ubuntu/Debian) or run it with `--appimage-extract-and-run`.

On first start, click **Add folder…** and pick a folder with photos. Add more folders, or remove them, in **Settings** (the gear at the top right). A second launch brings the open window to the front.

Where the index is kept (delete it to start over):

| | |
|---|---|
| Windows | `%APPDATA%\com.javiespinar.totufoto` |
| Linux | `~/.local/share/com.javiespinar.totufoto` |

Any 64-bit x86 CPU works, including older ones without AVX2 such as the AMD FX series. On Windows the app uses the WebView2 runtime that comes with Windows 10 and 11.

### Building the desktop app

Releases are built by GitHub Actions (`.github/workflows/desktop.yml`) when a `v*` tag is pushed, and each build runs a self-test before it is published. To build locally, follow steps 1 and 2 below, then:

```sh
# Linux (needs: libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf)
cargo install tauri-cli --version "^2" --locked
cd desktop && cargo tauri build --bundles appimage

# Windows: a single portable exe in target\release\totufoto-desktop.exe
cargo build --release -p totufoto-desktop --features custom-protocol
```

`totufoto-desktop --self-test [photos-folder] [report-file]` checks, without opening a window, that face recognition loads and that the photos index.

## Command-line app

The command-line app serves the gallery at `http://127.0.0.1:7878` for your browser. It is handy on a server or NAS, or on macOS.

### 1. Install Rust

You need Rust 1.93 or newer. If you don't have it:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

On macOS you also need the Xcode command line tools (`xcode-select --install`). On Linux you need a C compiler (for example `sudo apt install build-essential`). On Windows, install Rust with [rustup-init.exe](https://rustup.rs), which also sets up the Visual Studio C++ build tools it needs.

Any 64-bit x86 or ARM CPU works. AVX2 is **not** required: ONNX Runtime picks the best instructions your CPU has when it starts, so older processors such as the AMD FX series are fine.

### 2. Get the code, the face models and ONNX Runtime

macOS and Linux:

```sh
git clone https://github.com/JaviEspinar/totufoto.git
cd totufoto
scripts/fetch-models.sh
scripts/fetch-onnxruntime.sh
```

Windows (PowerShell):

```powershell
git clone https://github.com/JaviEspinar/totufoto.git
cd totufoto
powershell -ExecutionPolicy Bypass -File scripts\fetch-models.ps1
powershell -ExecutionPolicy Bypass -File scripts\fetch-onnxruntime.ps1
```

- `fetch-models` downloads the face detection and recognition models (about 16 MB) into `./models`. They are not stored in the repository because of their license (see below).
- `fetch-onnxruntime` downloads Microsoft's official [ONNX Runtime](https://github.com/microsoft/onnxruntime) library, which runs the face models, into `./onnxruntime`. Microsoft no longer publishes it for Intel Macs, so there the gallery runs without face recognition.
- Both scripts check each download against its known SHA-256 checksum and stop if it doesn't match.

Without either of them the gallery still works, just without people. When you add them later, the next start finds the faces in photos that were indexed without them.

### 3. Start it on a photo folder

```sh
cargo run --release -- ~/Pictures/Holidays
```

Or start it without folders (`cargo run --release`) and add them in **Settings** (the gear at the top right of the gallery). Either way you can add and remove more folders in Settings later; folders given on the command line are always included and can only be removed there.

The first build takes a minute or two. Then open **http://127.0.0.1:7878**. Indexing runs in the background: the first time, the Photos tab shows its progress until the first photos are in; after that a small card at the bottom right shows the progress on every tab (*Show N new* brings in the photos indexed since, *Details* opens Settings, *Hide* hides it until the next scan). Press `Ctrl+C` in the terminal to stop the app.

Subfolders are included automatically, and you can pass several folders:

```sh
cargo run --release -- ~/Pictures/2023 ~/Pictures/2024 /Volumes/Backup/Photos
```

To skip the build step next time, run the compiled binary directly from the project folder: `./target/release/totufoto ~/Pictures/Holidays` (on Windows `.\target\release\totufoto.exe C:\Users\me\Pictures`).

The startup log shows whether face recognition is on, which ONNX Runtime library it loaded, and on x86 which SIMD instructions the CPU has (for example `avx2=no avx=yes`).

### Apple Photos library

Point it at the originals inside the library package. The gallery only reads these files (don't rotate or delete photos inside the Photos library from Totufoto; use Photos for that):

```sh
cargo run --release -- ~/Pictures/"Photos Library.photoslibrary"/originals
```

Only photos stored on the Mac are found. Photos kept only in iCloud ("Optimize Mac Storage") are skipped.

## Using the gallery

- **Photos**: the timeline. With *Group* set to a year, month, day or place, the groups show as cards (cover, title, number of photos); click one to see its photos, with a chip for that group, and close the chip to go back to the cards. Places come with the most photos first, and a *No location* card holds the photos without GPS data. *No groups* shows all photos. *Newest/Oldest first* in the top bar sets the order. The bar with the filter chips and the photo count stays at the top while you scroll.
- **Upcoming**: memories from past years for the next 7 to 90 days.
- **Optimization**: finds identical photo files (the very same bytes) in the background after each scan and shows how much space they take. *Delete duplicates* moves the copies to the bin, keeping the one with the oldest file date of each set (if two have the same date, one of them). Only files that share their size with another are read, so the search is quick.
- **People**: name each person by typing under their face, use *Same as…* to merge two groups of the same person, and *Hide* for people you don't care about. Switch between *Cards* (big faces) and *List* (compact rows, several times more people on screen; the actions show when you point at a row). The size slider next to it makes cards or rows bigger or smaller. Sort people by *Most photos* or by *Name* (A to Z, unnamed last); the sidebar uses the same order.
- **Sidebar**: tick people (click the checkbox or their picture) to filter Photos and Upcoming, and pick *Together*, *Any* or *Only them* to decide how they combine. Click a name to rename that person right there (Enter saves, Esc cancels). The button at the top right of the column collapses it to a thin strip (showing how many people are selected), and opens it again. Names are unique: using a name someone already has asks whether they are the same person (*Merge them*) or not (*Keep separate* adds a number, like "Ana (1)").
- **Date range**: *From* and *to* in the top bar keep only photos taken between those days (both included; either can be left empty). It applies to Photos and Upcoming.
- **Filter chips**: each selected person, place and date range shows as a chip above the photos; × removes one, *Clear all* removes them all.
- **Deleting**: the bin button in the viewer (or the Delete key) asks how: *Remove from gallery* keeps the file and leaves it out of later scans (Settings can show such photos again); *Remove from disk* moves the file to the bin of the computer running Totufoto (if its drive has no bin, it asks before deleting for good). Only files inside the photo folders can be deleted. The gallery has no login, so anyone who can open it on your network can delete photos too (see [Privacy and security](#privacy-and-security)).
- **Download, Share and Open in folder**: buttons under the photo's date. In the browser there is *Download*, which saves the original file with its own name (HEIC and TIFF are saved as JPEG). The desktop app has *Share*, which opens the system share sheet with the photo (or *Download* where the system has none), and *Open in folder*, which shows the file selected in the file manager (Finder, Explorer, or the Linux file manager).
- **Rotate**: the two arrow buttons under the photo's date (or `R` and `Shift+R`) turn the photo right or left, and the file is saved turned. JPEG photos are turned through their EXIF orientation, so the picture is not compressed again and loses no quality; PNG is rewritten turned (it is lossless). Other formats (HEIC, WebP, GIF, TIFF) can't be rotated. Several quick turns are saved once. Face boxes, people and thumbnails stay right, and the photo isn't indexed again. Rotating waits while a scan is running.
- **Viewer**: click a photo to open it. Use the arrow keys to move and `Esc` to close. Click the open photo to zoom in where you clicked (click again to fit), drag to move around, and use the mouse wheel or a trackpad pinch to zoom in and out (up to 8x). The side panel shows the date, place and people. Click the name of an unnamed person (or of a face in no group) to name it there; a name that already exists offers to merge. For each face, *Same as…* moves just that face to the person you pick, and *Not them* moves it to a new unnamed group, which you can rename, hide or merge in People. *Card photo* makes that face the one shown on the person's card in People (it stays while the face belongs to them). Automatic grouping never moves a face you placed. *Show face boxes* draws the detected faces. The button at the top right of the details (or the `I` key) folds them away to give the photo more room, like the *People in the photo* column; the choice is remembered.
- **Settings** (the gear at the top right): photo folders. *Add folder…* opens the system folder picker in the desktop app; in the browser it lists the folders of the computer running Totufoto (open one, or type or paste a path, then *Add this folder*). A folder inside one already in the gallery isn't needed and is refused; adding a folder that contains others replaces them. *Remove* takes a folder's photos out of the gallery (the files stay on disk), except photos another folder still includes; its row shows the progress, a scan still running stops first (and continues with the other folders afterwards), and one folder is removed at a time. Folders given on the command line are marked *command line* and can't be removed here. Also in Settings: the library status with progress while indexing or grouping faces, and *Rescan* to pick up new, changed or deleted photos without restarting; files that could not be read, with the reason (they are skipped until the file changes, or *Try again*); and *Regroup all faces*. After each scan only the new faces are placed into people, which is fast even with hundreds of thousands of faces; regrouping everything from scratch is slower and only happens on the first index or when you ask.

### Phones and tablets

The gallery adapts to small screens; nothing needs installing, just open the server's address in the phone's browser.

- **Tablets and narrow windows**: dates, grouping and order are behind the sliders button in the top bar (a dot shows when a date range is active).
- **Phones**: the tabs get their own row, and *People in the photo* slides in from the people button (its badge shows how many are selected); tap outside to close it.
- **Back button**: it undoes one step at a time: it closes an open dialog, the open photo (back to where its thumbnail is) or a panel, then clears the filter chips, then goes back to Photos from another tab. With nothing left to undo it asks before leaving the gallery.
- **Viewer**: the photo uses the full width with the details below it. Swipe left or right for the next or previous photo, tap to zoom, drag with one finger and pinch to zoom in and out.

## The index

Everything the gallery learns is stored in `totufoto-data/index.sqlite`: metadata, thumbnails, faces, names and corrections. Photos are never copied, and their files only change when you rotate or delete them.

- Stopping and starting again does **not** re-index. Only new or changed files are processed, and deleted ones are removed.
- Deleted photos are removed at the next scan. If you open one before that, the viewer tells you it is no longer in its folder and removes it at once; if its whole folder can't be reached (an unplugged drive), nothing is removed.
- People you have named are remembered even when all their photos are gone: if the photos come back, or new ones appear, their faces rejoin the name.
- Photos are tracked by path. Moving or renaming the photo folder makes them look new, so they are indexed again. Named people are matched to the re-indexed faces automatically.
- Use `--data DIR` to keep separate indexes for separate libraries. Delete the data folder to start from scratch.

## Options

Run `totufoto --help` for the full list:

| flag | default | |
|---|---|---|
| `--data DIR` | `totufoto-data` | where the SQLite index (with thumbnails) is stored |
| `--models DIR` | `models` | folder with `det_500m.onnx` and `w600k_mbf.onnx` |
| `--onnxruntime PATH` | | ONNX Runtime library file or folder. By default it looks at `ORT_DYLIB_PATH`, next to the executable, and in `./onnxruntime` |
| `--port` / `--host` | `7878` / `127.0.0.1` | `--host 0.0.0.0` shares the gallery with your network; there is no login, so only on a network you trust (see [Privacy and security](#privacy-and-security)) |
| `--allow-host NAME` | | accept this host name too (repeatable), when you reach the gallery through a name such as `photos.home` instead of an IP address or the computer's own name |
| `--face-threshold` | `0.42` | cosine similarity to treat two faces as the same person; raise it if different people get mixed, lower it if one person is split |
| `--no-faces` | | skip face recognition |
| `--scan-only` | | index and exit |

When passing options through cargo, put them after `--`, for example `cargo run --release -- ~/Pictures --port 8080`.

Supported formats: JPEG, PNG, WebP, TIFF, GIF, BMP, and HEIC/HEIF on macOS (decoded with `sips`).

## Troubleshooting

- **"unexpected Host header"**: you opened the gallery through a host name it doesn't know (for example a name set up on your router). Start it with `--allow-host that-name`, or use the computer's IP address.
- **"face recognition disabled: ... not found"**: run `scripts/fetch-onnxruntime.sh` (or the `.ps1` on Windows) from the project folder, or point `--onnxruntime` at the library.

## How it's fast

- One pass per file: the file is read once, EXIF parsed, decoded, resized with SIMD (`fast_image_resize`), and faces are detected on the same buffer, spread over all cores with `rayon`, one ONNX Runtime session per worker thread.
- Thumbnails live inside SQLite (WAL, mmap), served with immutable cache headers; SQLite is faster than the filesystem for small blobs.
- The UI is a single dependency-free HTML page with a CSS-only justified layout, lazy images, and progressive rendering, so libraries with tens of thousands of photos stay smooth.

Measured on an 8-core Apple Silicon Mac: 300 photos at 12 MP fully indexed (thumbnails + faces) in 8.7 s.

## Layout

```
src/main.rs        CLI and startup
src/lib.rs         the library both apps are built on
src/app.rs         startup shared by both apps
src/scan.rs        file walking, per-photo pipeline, batched DB writes
src/imaging.rs     decoding, orientation, resizing, JPEG encoding
src/faces.rs       SCRFD detection, landmark alignment, ArcFace embeddings
src/cluster.rs     grouping faces into people
src/geo.rs         offline reverse geocoding
src/duplicates.rs  finding and deleting identical files
src/rotate.rs      rotating photos in their files
src/db.rs          SQLite schema and shared queries
src/server.rs      HTTP API (axum)
web/index.html     UI (embedded into the binary)
desktop/           Tauri desktop app: window, folder picker, embedded models and ONNX Runtime
```

## License

Totufoto is **source-available**: you may use, copy, change and share it for personal and other non-commercial purposes under the [PolyForm Noncommercial License 1.0.0](LICENSE). Non-profits, schools, public institutions and evaluation are covered too. For commercial use, ask [waiting4timeout](https://github.com/waiting4timeout) for a commercial license.

This is not an open-source license in the OSI sense, because it does not allow commercial use.

Third-party components come with their own terms, listed in [THIRD_PARTY.md](THIRD_PARTY.md). Two of them matter in particular:

- The **face models** (InsightFace `buffalo_s`) are released for non-commercial research use only, and the desktop app embeds them. This applies even with a commercial license for Totufoto's code: for commercial use, build it with face models whose license allows it.
- **City names** come from [GeoNames](https://www.geonames.org), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
