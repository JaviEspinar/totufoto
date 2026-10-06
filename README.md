# Imadive

*Formerly Totufoto (until version 0.1.10). Existing libraries carry over: the desktop app moves its data on first start, and the command-line app keeps using a `totufoto-data` folder when there is no `imadive-data`.*

A fast local photo gallery written in Rust. Point it at your photo folders and it indexes them in parallel: timeline, places, upcoming anniversaries and people found by face recognition. Everything runs on your computer and your photos are never uploaded; the files only change when you ask (rotating or deleting a photo).

It comes as a desktop app for Windows and Linux, and as a command-line app that serves the gallery to your browser. It is free for personal and other non-commercial use, and its source code is available ([license](#license)).

![The timeline: photos in a justified grid, with the people found in them in the sidebar](docs/screenshots/photos.jpg)

<p>
<img src="docs/screenshots/viewer.jpg" width="49%" alt="The viewer: a photo with its faces outlined and named, and its date, place and people beside it">
<img src="docs/screenshots/people.jpg" width="49%" alt="The People tab: one card per person found">
</p>
<p>
<img src="docs/screenshots/places.jpg" width="74%" alt="Photos grouped by place, one card per town">
<img src="docs/screenshots/phone.jpg" width="24%" alt="The gallery on a phone">
</p>

<sub>Photos: Library of Congress, FSA/OWI colour collection, 1940 to 1943, public domain ([about the screenshots](docs/screenshots/README.md)).</sub>

## Features

- **Timeline** sorted by capture date (EXIF `DateTimeOriginal`, falling back to the file date), newest or oldest first, grouped by day, month, year or place.
- **Places**: GPS coordinates are turned into cities with an offline reverse geocoder (GeoNames cities with more than 1000 inhabitants). No network calls. Group photos *by place* to see one card per city.
- **Upcoming**: photos from previous years whose anniversary falls in the next 7 to 90 days ("2 years ago today").
- **People**: faces are detected (SCRFD), aligned and turned into 512-d ArcFace embeddings, then grouped into people automatically. Name them, merge duplicates, hide people, or move a wrongly grouped face to a group of its own ("Not them").
- **People combinations**: tick people in the sidebar and choose
  - *Together*: every selected person is in the photo,
  - *Any*: at least one of them,
  - *Only them*: all of them and no other known person.
- **Videos** in the timeline, with their date, place and length, played in the viewer.
- All filters combine (people + place + date range + upcoming) and live in the URL, so views can be bookmarked.
- **In English or Spanish**, with a light or dark theme (or your system's), chosen in Settings.

## Privacy and security

- **Private by design**: everything runs on your computer. There is no account, no cloud and no telemetry; places are found with an offline city list, and the only network requests are the ones you make yourself (opening a map link).
- **No login**: by default the gallery only accepts this computer. If you share it with your network (`--host 0.0.0.0`, to use it from a phone), anyone who can reach it can see your photos, rotate them and delete them, and add any folder of the computer to the gallery (so also see and delete images you never shared). Only do that on a network you trust, such as your home network, and never expose it to the internet.
- **Other websites can't use it**: requests for a name other than an IP address, `localhost`, this computer's own name or one you allow with `--allow-host` are refused (this blocks DNS rebinding), and requests that change something are refused when they come from another site.

To report a security problem, see [SECURITY.md](SECURITY.md).

## Desktop app (Windows and Linux)

Download the file for your system from the [latest release](https://github.com/JaviEspinar/totufoto/releases/latest). Face recognition and everything it needs are built in.

- **Windows 10/11**: `Imadive-<version>-windows-x64.exe`. Double-click it. It is not signed, so the first time Windows SmartScreen may say "Windows protected your PC": click **More info** → **Run anyway**.
- **Debian, Ubuntu and Linux Mint** (Debian 12 or Ubuntu 22.04 and newer, x86-64): `Imadive-<version>-linux-amd64.deb`, the smaller download. Install it, then start **Imadive** from the applications menu:
  ```sh
  sudo apt install ./Imadive-*-linux-amd64.deb
  ```
  Installing a newer version the same way updates it; `sudo apt remove imadive` removes it (your library stays in its data folder).
- **Other Linux distributions** (x86-64): `Imadive-<version>-linux-x86_64.AppImage`, which brings everything it needs, so it runs on most distributions without installing anything. Make it executable and run it:
  ```sh
  chmod +x Imadive-*.AppImage
  ./Imadive-*.AppImage
  ```
  If it says FUSE is missing, install it (`libfuse2` on most distributions) or run it with `--appimage-extract-and-run`.

On first start, click **Add folder…** and pick a folder with photos. Add more folders, or remove them, in **Settings** (the gear at the top right). A second launch brings the open window to the front.

Where the index is kept (delete it to start over):

| | |
|---|---|
| Windows | `%APPDATA%\com.waiting4timeout.imadive` |
| Linux | `~/.local/share/com.waiting4timeout.imadive` |
| macOS | `~/Library/Application Support/com.waiting4timeout.imadive` |

It holds the index (`index.sqlite`, with the thumbnails and people), the log (`logs/`) and, in `runtime/`, the face models and ONNX Runtime the app unpacks on first start (older versions' copies are deleted).

Imadive was called Totufoto until version 0.1.10. On its first start, the app moves the library of a Totufoto installation (`com.javiespinar.totufoto`) to the folder above, so names, people and the index carry over.

Any 64-bit x86 CPU works, including older ones without AVX2 such as the AMD FX series. On Windows the app uses the WebView2 runtime that comes with Windows 10 and 11.

### Building the desktop app

Releases are built by GitHub Actions (`.github/workflows/desktop.yml`) when a `v*` tag is pushed, and each build runs a self-test before it is published. To build locally, follow steps 1 and 2 below, then:

```sh
# Linux (needs: libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf)
cargo install tauri-cli --version "^2" --locked
cd desktop && cargo tauri build --bundles appimage,deb

# Windows: a single portable exe in target\release\imadive-desktop.exe
cargo build --release -p imadive-desktop --features custom-protocol
```

`imadive-desktop --self-test [photos-folder] [report-file]` checks, without opening a window, that face recognition loads and that the photos index. With [just](https://just.systems), `just fetch` and `just desktop` do the same (see [CONTRIBUTING.md](CONTRIBUTING.md)).

## Command-line app

The command-line app serves the gallery at `http://127.0.0.1:7878` for your browser. It is handy on a server or NAS, or on macOS. To keep it running on a Linux server, see [Running it on a home server](docs/server.md).

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

To skip the build step next time, run the compiled binary directly from the project folder: `./target/release/imadive ~/Pictures/Holidays` (on Windows `.\target\release\imadive.exe C:\Users\me\Pictures`).

The startup log shows whether face recognition is on, which ONNX Runtime library it loaded, and on x86 which SIMD instructions the CPU has (for example `avx2=no avx=yes`).

### Apple Photos library

Point it at the originals inside the library package. The gallery only reads these files (don't rotate or delete photos inside the Photos library from Imadive; use Photos for that):

```sh
cargo run --release -- ~/Pictures/"Photos Library.photoslibrary"/originals
```

Only photos stored on the Mac are found. Photos kept only in iCloud ("Optimize Mac Storage") are skipped.

## Using the gallery

- **Photos**: the timeline, as cards per month (or year, day or place); open a card to see its photos.
- **People**: the faces found, grouped into people. Name them, merge two groups of the same person, hide the ones you don't care about.
- **Upcoming**: photos taken on the coming days in earlier years.
- **The sidebar**: tick people to see only their photos, together, any of them, or only them.
- **The viewer**: arrow keys to move, a click to zoom, the details and the people on the side, and buttons to rotate, download or delete a photo.
- **Optimization**: identical files, and how much space deleting the copies frees.
- **Settings** (the gear at the top right): photo folders, the indexing progress, files that could not be read.

On a phone, open the same address in the browser: the gallery adapts to small screens, and Back undoes one step at a time.

Everything is explained in the [user guide](docs/user-guide.md), including how the index follows changes to your folders.

## Options

Run `imadive --help` for the full list:

| flag | default | |
|---|---|---|
| `--data DIR` | `imadive-data` | where the SQLite index (with thumbnails) is stored |
| `--models DIR` | `models` | folder with `det_500m.onnx` and `w600k_mbf.onnx` |
| `--onnxruntime PATH` | | ONNX Runtime library file or folder. By default it looks at `ORT_DYLIB_PATH`, next to the executable, and in `./onnxruntime` |
| `--port` / `--host` | `7878` / `127.0.0.1` | `--host 0.0.0.0` shares the gallery with your network; there is no login, so only on a network you trust (see [Privacy and security](#privacy-and-security)) |
| `--allow-host NAME` | | accept this host name too (repeatable), when you reach the gallery through a name such as `photos.home` instead of an IP address or the computer's own name |
| `--face-threshold` | `0.42` | cosine similarity to treat two faces as the same person; raise it if different people get mixed, lower it if one person is split |
| `--no-faces` | | skip face recognition |
| `--scan-only` | | index and exit |

When passing options through cargo, put them after `--`, for example `cargo run --release -- ~/Pictures --port 8080`.

Supported formats: JPEG, PNG, WebP, TIFF, GIF, BMP, and HEIC/HEIF on macOS (decoded with `sips`); videos in MP4, MOV, M4V, 3GP, WebM, MKV, AVI, WMV, MPEG and MTS.

## Limitations

- **Formats**: JPEG, PNG, WebP, TIFF, GIF and BMP everywhere, and HEIC/HEIF only on macOS. No RAW files.
- **Videos** show with a generic picture, not a frame of their own, and nobody is looked for in them. They play in the browser when it can play their format: MP4 (H.264) and WebM everywhere, iPhone videos (HEVC) on Apple devices and in Chrome on most recent computers; others (AVI, WMV, MPEG...) can be downloaded.
- **Photos are tracked by path**: moving or renaming a photo folder makes its photos look new, so they are indexed again (named people are matched again automatically).
- **No login**: anyone who can reach the gallery can use all of it (see [Privacy and security](#privacy-and-security)).
- **Desktop app**: Windows and Linux only, not signed. On macOS use the command-line app.
- **Face recognition** needs ONNX Runtime, which Microsoft doesn't publish for Intel Macs, and its models are for non-commercial use only (see [License](#license)).

## Troubleshooting

- **Windows says "Windows protected your PC"**: the app isn't signed yet. Click **More info**, then **Run anyway**. You can check the file against `SHA256SUMS` on the release page first ([how](docs/releasing.md#checking-a-download)).
- **The desktop app opens an empty window on Windows**: it needs the Microsoft Edge WebView2 runtime, which comes with Windows 10 and 11. If it was removed, install it from Microsoft's WebView2 page.
- **People stay empty, or something else looks wrong in the desktop app**: run its self-test and include the report in an issue. Close the app, then on Windows run `.\Imadive-<version>-windows-x64.exe --self-test C:\path\to\photos report.txt` in PowerShell and open `report.txt`; on Linux run `./Imadive-<version>-linux-x86_64.AppImage --self-test ~/Pictures/some-folder`.
- **Logs**: the desktop app writes its log to `logs/imadive.log` in its data folder (the previous run's is `imadive.old`); **Settings → About → Open log folder** shows it. The command-line app logs to the terminal, or to `journalctl -u imadive` as a service. Set `RUST_LOG=imadive=debug` for more detail.
- **"unexpected Host header"**: you opened the gallery through a host name it doesn't know (for example a name set up on your router). Start it with `--allow-host that-name`, or use the computer's IP address.
- **"face recognition disabled: ... not found"**: run `scripts/fetch-onnxruntime.sh` (or the `.ps1` on Windows) from the project folder, or point `--onnxruntime` at the library.
- **A service that stopped after the rename to Imadive** (`status=203/EXEC`): see [Coming from Totufoto](docs/server.md#3-updating).

## Documentation

- [User guide](docs/user-guide.md): the gallery, tab by tab.
- [Running it on a home server](docs/server.md): the command-line app as a systemd service.
- [How it works](docs/architecture.md): the code, for people who want to change it.
- [HTTP API](docs/api.md).
- [Releasing](docs/releasing.md).

## Contributing

Bug reports, ideas and pull requests are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md) for how to build, test and send changes, and the [code of conduct](CODE_OF_CONDUCT.md).

## License

Imadive is **source-available**: you may use, copy, change and share it for personal and other non-commercial purposes under the [PolyForm Noncommercial License 1.0.0](LICENSE). Non-profits, schools, public institutions and evaluation are covered too. For commercial use, ask [waiting4timeout](https://github.com/waiting4timeout) for a commercial license.

This is not an open-source license in the OSI sense, because it does not allow commercial use.

Third-party components come with their own terms, listed in [THIRD_PARTY.md](THIRD_PARTY.md). Two of them matter in particular:

- The **face models** (InsightFace `buffalo_s`) are released for non-commercial research use only, and the desktop app embeds them. This applies even with a commercial license for Imadive's code: for commercial use, build it with face models whose license allows it.
- **City names** come from [GeoNames](https://www.geonames.org), licensed under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
