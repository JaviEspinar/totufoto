# Totufoto: code quality and open-source readiness plan

Review of the whole project at commit `17002fc` (release 0.1.9): the Rust service (`src/`, about 3,300 lines), the web UI (`web/index.html`, 2,890 lines), the desktop shell (`desktop/`), CI, scripts and repository hygiene. Nothing has been changed; this document is the plan.

Line numbers refer to the files as they are at that commit.

## 1. Summary

The code is in good shape for a project of this age: intent is explained in comments, SQL is parameterised, the scan pipeline is careful about offline drives and partial runs, `cargo clippy` is clean, and the loopback Host check against DNS rebinding is more than most local servers do. The debt is concentrated in a few places:

- **Legal:** the repository is public with no `LICENSE`, so nobody may legally use or fork it yet. The binaries embed non-commercial models and CC BY data without the required notices. Decision taken during the review: the code will be published under **PolyForm Noncommercial 1.0.0** (free for personal and other non-commercial use, commercial use by arrangement), with a donation link. That makes the project source-available rather than open source in the OSI sense; the README should use that wording.
- **Security posture:** the server has no authentication and can delete and rewrite files. On loopback this is contained; with `--host 0.0.0.0` (the documented way to use it from a phone) every protection is off, and any LAN client can enrol any folder on the machine and then delete photos in it.
- **Two files carry most of the complexity:** `src/server.rs` (1,071 lines: routing, 34 SQL statements, folder logic and response building) and `web/index.html` (2,170 lines of JavaScript with about 45 module-level mutable variables).
- **Tests:** 6 unit tests exist (`db::unique_names` and 5 in `rotate.rs`) and CI never runs them. Every destructive path (pruning, folder removal, duplicate deletion, rotation's database update, migrations) is untested.
- **Two "stuck forever" bugs** and one data-loss risk were found while reviewing (section 3.2).
- **Process:** releasing means editing four files by hand, CI has no checks on pull requests, and the code was never formatted with rustfmt (`cargo fmt --check` reports 161 differences).

The plan has five phases (section 8). Phase 1 (license, metadata, security fixes, CI that runs the tests) is what should happen before announcing the project; it is about two days of work.

## 2. Open-source readiness

### 2.1 Findings

| Item | Status | Detail |
|---|---|---|
| `LICENSE` | missing | Public repository with no license. |
| Cargo metadata | partial | `Cargo.toml:1-5` has only name, version, edition, rust-version. No `description`, `license`, `repository`, `readme`, `keywords`, `categories`. |
| Third-party notices | missing | Release binaries embed InsightFace buffalo_s models (non-commercial research only), ONNX Runtime (MIT, its LICENSE is fetched to `onnxruntime/LICENSE` but not shipped), the Visual C++ redistributables, and `reverse_geocoder` bundles GeoNames data (CC BY 4.0, attribution required). `README.md:10` names GeoNames but not its license. |
| `SECURITY.md`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md` | missing | |
| Issue and PR templates, Dependabot | missing | `.github/` only holds `workflows/desktop.yml`. |
| Personal data in the tree | clean | No IPs, personal paths or emails in tracked files. `.claude/` is untracked. The bundle id `com.javiespinar.totufoto` and the commit email are public by choice. |
| Formatting config | missing | No `rustfmt.toml`, `.editorconfig`; code uses lines up to about 150 characters, so any contributor's `cargo fmt` rewrites the tree. |

### 2.2 Planned changes

1. **License the code under PolyForm Noncommercial 1.0.0** (decided). Add a `LICENSE` file with the text from polyformproject.org/licenses/noncommercial/1.0.0, the licensor name and year filled in, and `license = "PolyForm-Noncommercial-1.0.0"` (the SPDX identifier, accepted by Cargo) in both `Cargo.toml` files. Why this one: written for software, defines "noncommercial" clearly (personal use, non-profits, education, government and evaluation are allowed), short, and it lets the author sell commercial licenses separately. Not Creative Commons BY-NC (not meant for code) and not the Commons Clause (ambiguous). Consequences to state plainly in the README: the project is **source-available, free for personal and non-commercial use**, not open source by the OSI definition; GitHub's license detection will not recognise it; some contributors and employers will stay away. The dependencies allow it (MIT, Apache and BSD crates can be redistributed under stricter terms; GeoNames only needs attribution). The license binds other people, not the author: donations, commercial licenses or sales by the author stay possible.
2. **State the model restriction separately.** The InsightFace buffalo_s terms (non-commercial research use) apply to every downloadable `.exe` and `.AppImage` on their own, even to someone who obtains a commercial license for the code. Say so in the `## License` section of the README, at the top of every release note, and in a `THIRD_PARTY.md` listing ONNX Runtime (MIT), the VC++ redistributable terms, the InsightFace terms and GeoNames (CC BY 4.0, with the attribution text). `cargo about` can generate the crate part. If the project ever earns money beyond donations, replace the models with permissively licensed ones first; that is a two-file swap in `faces.rs`, not a rewrite.
3. **Donations.** Add `.github/FUNDING.yml` (GitHub Sponsors, Ko-fi or similar) so the Sponsor button shows on the repository, a donation link in the README's License section, and one in the app (an About entry at the bottom of Settings linking to the same page; it opens through `/api/open`, whose allow-list must then include that URL). Voluntary donations are the safest kind of income while the models are inside the product. Say in the README from day one: "free for personal use; commercial licenses available on request", so nobody is surprised later.
4. **Contributions.** A license change later is possible only while every author agrees. While the author is the only contributor the terms can still be changed freely (to MIT or AGPL, say); before the first outside pull request, add a short note to `CONTRIBUTING.md` that contributions are licensed to the project under the same terms (a one-line "inbound = outbound" statement or a DCO sign-off), so a future change or a commercial license does not need a hunt for permissions.
5. **Fill the Cargo metadata** (`description`, `license`, `repository`, `homepage`, `readme`, `keywords`, `categories`) and move `version` to `[workspace.package]` with `version.workspace = true` in both packages. Remove `version` from `desktop/tauri.conf.json:4` (Tauri 2 falls back to the Cargo version). Releasing then touches one file plus the notes.
6. **Add `SECURITY.md`** with the threat model (trusted network only, no authentication, what the server can do to files) and a private reporting channel (GitHub private vulnerability reporting). Add a short **Privacy and security** section near the top of the README, a warning next to `--host` in the options table (`README.md:171`) and in the clap help (`src/main.rs:20`).
7. **Add `CONTRIBUTING.md`** (build steps, `just` targets, test and lint commands, how releases work, the contribution licensing note from item 4), a short `CODE_OF_CONDUCT.md`, and issue templates that ask for the version, the OS and the `--self-test` report.
8. **Commit `rustfmt.toml`** (`max_width = 120`, `use_small_heuristics = "Max"`) and run `cargo fmt` once in its own commit; add `.editorconfig`.
9. **GitHub settings:** branch protection on `main` requiring CI, tag protection for `v*`, Dependabot for cargo and GitHub Actions (weekly), secret scanning, Discussions for support questions.

## 3. Rust service

### 3.1 Architecture

`src/server.rs` is the file every change touches and its SQL cannot be tested without HTTP. Specific misplacements:

- Folder domain logic lives in handlers: `save_folder` (`server.rs:209-242`) and `remove_folder_now` (`:293-325`) implement root nesting and the `<root><separator>` prefix rule, duplicating `ScanConfig::roots` (`scan.rs:108-123`), which is itself a config struct that queries the database.
- The query builders `photo_filters`, `people_filter`, `date_range_filter`, `upcoming_days` (`server.rs:423-523`) are pure functions with no HTTP in them and are the most valuable thing to unit-test; today they are unreachable from tests.
- `merge_person` (`server.rs:899-916`) is a three-statement transaction inline in a handler while its siblings `assign_face` and `move_face_to_new_person` live in `db.rs`.
- `check_photo` and `remove_photo` (`server.rs:593-670`) decide what may be deleted based on roots and the filesystem; `rotate.rs` already shows the better pattern (an `Outcome` enum mapped to status codes by a thin handler).
- `Pool` (`server.rs:27-48`) is database infrastructure living in the HTTP module.
- `scan.rs` mixes status bookkeeping, root resolution, the indexing pipeline and EXIF parsing; `exif_datetime` and `exif_gps` (`scan.rs:519-553`) are pure and testable on their own.
- "Is this path under one of the roots" is written nine times (`server.rs:220, 314, 603, 648`, `scan.rs:241, 255, 256`, `rotate.rs:41`, `duplicates.rs:208`); the SQL prefix match twice (`server.rs:188-193, 307-311`); the "delete empty unnamed persons" statement four times (`db.rs:184, 264, 289`, `cluster.rs:81`); mtime extraction four times; L2 normalisation three times; the "has column, add column" block six times in `migrate()` (`db.rs:115-163`).

**Target layout**

```
src/app.rs            Config, Gallery, Host (as today)
src/http/mod.rs       router(), AppState, check_host, ApiError, db() helper
src/http/photos.rs    /api/photos, /api/groups, /api/photos/{id}/*, /thumb, /original
src/http/people.rs    /api/people/*, /api/faces/*
src/http/folders.rs   /api/folders/*
src/http/system.rs    /api/status, scan, regroup, failures, excluded, open
src/http/duplicates.rs
src/db/mod.rs         open(), init(conn), SCHEMA, migrate(), Pool
src/db/photos.rs      PhotoFilter builder, list, detail, groups, places
src/db/people.rs      rename, merge, assign, move, remember, delete_empty
src/db/folders.rs     folders table, photos_under(root)
src/library.rs        roots nesting, root_of(path), add_root, remove_root,
                      check_photo, remove_photo (Outcome enums)
src/scan.rs           indexing pipeline only
src/exif.rs           exif_datetime, exif_gps
src/fs.rs             file_stamp(&Metadata), unchanged()
src/{cluster,faces,imaging,geo,duplicates,rotate}.rs   unchanged
```

The move is mechanical and can be done one resource at a time once `ApiError` (3.4) and `library.rs` exist.

### 3.2 Correctness and robustness

Ranked by impact. The first three were confirmed by reading the code during this review.

1. **A panic inside a scan leaves `running = true` forever** (`scan.rs:181-193`). `run()` clears the flag only on the normal return path. Image decoding and the ONNX output indexing (`faces.rs:177-190`, no shape check) can panic on a malformed file; after that `/api/status` says "running" forever, every `scan::spawn` only sets `rerun`, and rotating returns 409 until a restart. `duplicates::run` (`duplicates.rs:57-66`) has the same shape. **Fix:** a `Drop` guard that resets `running` and `phase` (modelled on `RemovalGuard`, `server.rs:259-270`), plus `catch_unwind` per file inside the `par_iter` so one bad file becomes a `failures` row instead of ending the scan.
2. **`duplicates_delete` leaves `deleting = true` if the client disconnects** (`server.rs:708-722`). Hyper drops the handler future when the connection closes, so line 720 never runs and every later call gets 409. `remove_folder` already solves this with `tokio::spawn` plus a guard; apply the same pattern.
3. **An unmounted Linux mount point looks like an empty folder and prunes the whole index** (`scan.rs:215` uses `root.is_dir()` to detect offline roots). On Linux `/mnt/photos` stays as an empty directory when the drive is not mounted, so every photo under it is deleted with thumbnails, faces and unnamed groups. **Fix:** treat a root that yields zero files while the index holds photos under it as offline, log loudly, and never prune it. This needs the pruning tests from section 6 first.
4. **Modified files get a new id** (`scan.rs:261-287`): a changed photo is deleted and reinserted, so its faces and person assignments go, unnamed groups lose the faces, and the UI's URLs 404 mid-scan. The delete also happens long before the reinsert, so a crash loses the photo until the next run. **Fix:** `UPDATE` in place, bump `version`, prune at the end of the scan.
5. **`Pool` is unbounded** (`server.rs:27-48`) and each connection sets `cache_size = -65536` (64 MiB) and a 1 GiB mmap (`db.rs:100-107`). A burst of `/thumb/{id}` requests can open hundreds of connections. **Fix:** a `tokio::sync::Semaphore` sized to `available_parallelism()`, and a smaller cache for pooled read connections.
6. **Two concurrent `scan::spawn` calls when idle start two threads** (`scan.rs:131-150`; the check and the spawn are not atomic). The loser then runs `duplicates::run` next to the live scan. Use `compare_exchange` on a "spawned" flag.
7. **Excluding a photo during a scan can bring it back** (`excluded` is read at scan start, `scan.rs:228-231`). Re-check in `write_results`.
8. **`rotate_photo` checks `running` without taking `hold`** (`server.rs:985-987`). Harmless today because of the atomic rename and the mtime rewrite; take `hold` like folder removal does, or document why it is fine.
9. **`let _ = sender.send(outcome)`** (`scan.rs:307`): if `write_results` fails, producers keep decoding every remaining file for nothing. Stop on send error.
10. **`original` returns 500 when the file was deleted externally** (`server.rs:1050`); should be 404 so the UI can go straight to `/check`.
11. **SQL built with `format!`** (`server.rs:461, 555-569`, `db.rs:195`): no injection today (only parsed integers and constants are inlined), but each distinct people list is a distinct `prepare_cached` entry, churning rusqlite's 16-entry cache, and the pattern invites a future mistake. Pass id lists through `rusqlite::vtab::array` or `json_each(?)`.
12. **Mutex `.unwrap()`** (`server.rs:39, 45, 265, 277, 685`, `scan.rs:73, 80, 92`, `duplicates.rs:64`): use `unwrap_or_else(PoisonError::into_inner)` so `/api/status` survives a panic elsewhere.
13. Smaller: the two "is under root" checks disagree in nature (SQL byte prefix vs component-wise `Path::starts_with`), which only matters if a path ever bypasses `canonicalize`; `is_fixed` (`scan.rs:125`) compares the raw UI string to canonical roots; `(w - 1) as f32` in `faces.rs:298` deserves a `debug_assert!(w > 0)`; `duplicates::fingerprint` uses raw `BEGIN`/`COMMIT` (`duplicates.rs:93-97`) instead of `conn.transaction()`.

No panic reachable from HTTP input was found: `Path<i64>` and `Json<T>` rejections are 400/422, `parse_ids` drops junk, `upcoming` is clamped, the EXIF rewriting is bounds-checked and tested, `list_dirs` truncates at 5,000 entries.

### 3.3 Security

Already in place and worth advertising: the Host allow-list on loopback (`app.rs:94-101`, `server.rs:127-134`); no CORS layer; all mutating endpoints are POST and the ones with bodies need `Content-Type: application/json`, which cross-origin pages cannot send without a preflight; `/api/open` only accepts OpenStreetMap URLs (`server.rs:388-399`); `/thumb`, `/face` and `/original` only serve paths from the database; deletion and rotation require the file to be under a root.

Gaps, in order:

1. **No Host or Origin check once bound to a non-loopback address** (`app.rs:96-101` returns `None`, which disables `check_host`). A web page can DNS-rebind to the LAN address and call every endpoint, including `POST /api/photos/{id}/remove {"from":"disk","permanently":true}`. **Fix (one middleware, always on):** for every non-GET request require `Sec-Fetch-Site: same-origin` or `none`, or an `Origin` header whose host equals the request `Host`. All current browsers send `Sec-Fetch-Site`. Keep the Host allow-list too, built from the bound address and the machine's own interface addresses instead of being switched off.
2. **`POST /api/folders` enrols any directory on the machine** (`server.rs:244-246`). A LAN client can add `/` or `C:\Users`, wait for the scan, then delete any image through `remove`, `duplicates/delete` or `rotate`, because "inside the photo folders" now means anywhere. Combined with gap 1 this is reachable from a web page. **Fix:** make folder add, remove, browse and pick available only to loopback clients or the desktop app (like `pick_folder` already is), or behind a token printed at startup.
3. **`GET /api/folders/browse?path=` lists any directory** (`server.rs:334-381`): host filesystem layout disclosure to the LAN. Same fix as 2.
4. **Body-less POSTs are reachable from a plain cross-site form** when the allow-list is off: `/api/scan`, `/api/regroup`, `/api/failures/retry`, `/api/excluded/clear`, `/api/duplicates/search`, `/api/photos/{id}/check` (which can forget index rows), `/api/faces/{id}/reject`, `/api/faces/{id}/cover`. The `Sec-Fetch-Site` check from gap 1 closes all of them.
5. **Error bodies leak internal paths** (`server.rs:70, 211, 214, 349`). With the typed `ApiError` (3.4), log the chain and return a short message.
6. **Release binaries are unsigned on both platforms** and nothing lets a user verify a download. Attach `SHA256SUMS` and a build provenance attestation to each release (section 5.3).

### 3.4 API design

- **`ApiError` is a single 500** (`server.rs:59-74`). Handlers needing other codes build responses by hand, so bodies are inconsistent: `remove_photo` returns `{"error": ...}` JSON, `save_folder`, `remove_folder`, `rotate_photo` and `groups` return plain text, `photo_detail` returns 500 for not found (`:789`), `reveal_photo` says 409 and `pick_folder` says 501 for the same "not the desktop app" condition. **Plan:** `enum ApiError { NotFound, BadRequest(String), Conflict(String), Forbidden(String), Unsupported(String), Internal(anyhow::Error) }` with one `IntoResponse` that produces `{"error": "...", "code": "..."}` and logs only `Internal`.
- **Response shapes:** `/api/photos` rows are positional arrays `[id, w, h, taken, place, version]` (`server.rs:534-541`) and the UI reads them by index in about 25 places; `/api/status` is a typed struct with two fields patched in by hand (`:145-148`). Convert the remaining `json!` blobs to `#[derive(Serialize)]` structs (as `DupGroup` and `DeleteResult` already are); keep the array form for photos if size matters, but document it.
- **Routes:** verbs are inconsistent (`POST /api/folders/remove` with a body where `DELETE` would be idiomatic; `POST /api/people/{id}` is an update; `/thumb`, `/face`, `/original` sit outside `/api`). The only consumer is the embedded UI, so renaming is cheap now and expensive after other clients exist. Decide before 1.0 and document the API in `docs/api.md`.
- `update_person` returns 204 or 200 depending on which field changed; always return the person.
- `/thumb/{id}` is served as immutable for a year and relies on the UI appending `?v=`; put the version in the path (`/thumb/{id}/{version}`) so the server enforces the contract.

### 3.5 Dependencies and build

- `ort = "=2.0.0-rc.13"` with `load-dynamic` ties the build to the ONNX Runtime version the scripts fetch (1.28.2). Record that version in one place (a constant the scripts and `build.rs` read, or at least a comment in `Cargo.toml`).
- `clap` is a dependency of the library but only `main.rs` uses it; the desktop app compiles it for nothing. Make it optional behind a `cli` feature with `[[bin]] required-features`.
- `chrono`'s `serde` feature appears unused; drop it.
- Add `strip = "symbols"` to the release profile. Do not set `panic = "abort"` until 3.2 item 1 is fixed with a guard.
- Add `cargo-deny` (licenses, advisories, duplicate crates) with a `deny.toml` allowing MIT, Apache-2.0, BSD, ISC, Zlib, Unicode and CC-BY-4.0 for dependencies (the project's own PolyForm license is not a dependency license and needs no entry).
- `rust-version = "1.88"` is never verified; add an MSRV job or drop the field.
- `tower-http` compression skips `image/*` by default; add a comment at `server.rs:119`, since readers will wonder whether thumbnails are recompressed.

### 3.6 Documentation in code

- `db.rs` has no module doc; it should state the schema ownership rules (who may delete persons; what `rejected`, `grouped`, `face_memory` and `cover_face` mean), which today live only in SQL comments.
- The concurrency model (one scan thread, one duplicate-search thread, one folder removal at a time; who sets and clears `running`, `rerun`, `hold`, `removing`, `deleting`) spans three files. Describe it once in `scan.rs`'s module doc.
- Stale text: `scan.rs:206` says "add one from the Folders button" (it is Settings now).
- `Gallery` (`app.rs:35`) and `Config.host` have no doc comments; `init_logging` should say it is idempotent and that `RUST_LOG` overrides the default filter.
- Document the `IMMUTABLE` cache contract and the positional row format where a client author will look.

## 4. Web UI

### 4.1 Structure

The file is well sectioned with banner comments, but 2,170 lines of script with about 45 module-level mutables is past the point where a contributor can change one feature without reading the rest. The hidden ordering dependencies show it: `nameCollator` is declared at line 1759 and used at 817, `viewer` at 1988 and used at 905, `folderInfo` at 2500 and used at 1203, `lastStatus` at 2570 and used at 1187. It works only because everything runs after the whole script is evaluated.

**Plan, keeping the no-build-step property:**

1. **Step 1 (small):** split into `web/index.html`, `web/app.css`, `web/app.js`, embedded with three `include_str!` and served as `/app.css` and `/app.js` with the right content types, referenced as `/app.js?v=<CARGO_PKG_VERSION>` so browsers do not keep an old script after an upgrade. The desktop shell opens the same URL, so nothing else changes.
2. **Step 2 (medium, later):** native ES modules under `web/js/` (`state.js`, `api.js`, `sidebar.js`, `photos.js`, `people.js`, `viewer.js`, `settings.js`, `status.js`, `main.js`) served by one route from a table of `include_str!`. Prerequisites: replace the inline `onload="thumbLoaded(this)"` / `onerror` handlers (lines 1049, 1455) with one capturing `load`/`error` listener on `#main` (this also unblocks a CSP later), and break the sidebar -> `render()` -> sidebar cycle with an explicit entry point in `main.js`.
3. Not recommended: a `build.rs` that concatenates files. It adds a build step for little gain and makes browser error line numbers not match the sources.

### 4.2 State and data flow

- `photos` is a positional tuple read as `p[0]`, `p[3]`, `p[4]`, `p[5]` in about 25 places (1010, 1015, 1045-1049, 1304-1319, 2039, 2144, 2153, 2186). One wrong index is silent. Add a `photo(p)` destructuring helper or have the server send objects.
- `render()` is called from 14 sites and re-entrancy is handled by hand-written `renderToken` checks in 11 places. `renderPeople` is the only view without an abort `signal`, and Optimization and People bypass `beginViewLoad` (writing `main.innerHTML` directly at 1335, 1350, 1363, 1612), so they lose the stale-dimming behaviour. **Plan:** `beginViewLoad` returns a `ViewLoad { token, signal, alive() }` and every view uses it.
- `loadMeta()` (778) is called after almost every mutation (11 sites), refetches the whole people list (the heaviest request in the UI for a 20k-person library), mutates `state.people` and may call `render()`. Split into a fetch and an explicit `applyMeta()`; have mutation endpoints return the updated person and patch locally, resyncing only when a scan ends.
- Viewer state is four unrelated variables (`viewerIndex`, `viewerPath`, `rotation`, `zoom`), and `openViewer` and `flushRotation` both compare against `viewerIndex` to detect staleness (2015, 2042, 2166). Group them into one object; same for the People tab (`peopleQuery`, `peopleSort`, `peopleLayout`, `peopleZoom`, `peopleGrid`). Rename `state.people` (the selected ids) to `state.selectedPeople`.
- `const zoom = $("#peopleZoom")` at 1628 shadows the module-level `zoom` (2311).
- `lastStatus` is patched with spreads (2693, 2707) while `pollStatus` overwrites it; two pollers run during a folder removal (2695 and 2794) and can overlap during duplicate deletion (1415 and 1336).

### 4.3 Escaping

Every template was checked: file paths, person names, place names, search text, error messages and browse entries all go through `esc()`; toasts and dialog titles use `textContent`. No XSS hole found. Two uniformity nits: `d.lat`/`d.lon` (2062) and `f.box[i]` (2048) are interpolated raw (they are numbers from the server; wrap in `Number()`). The inline `onload`/`onerror` handlers (1049, 1455) are the only thing standing between the page and a strict CSP.

### 4.4 Duplication and naming

- 14 inline SVG icons across 11 lines (606-608, 617, 704, 711, 2057-2060, 2509; the folder path appears twice, the panel-toggle icon in four variants). Use a `<symbol>` sprite with `<use>`; the four toggles become one symbol with CSS `scaleX(-1)`.
- `askDelete` / `#deleteDlg` are a generic confirm dialog used for "Leave Totufoto?" (2855), folder removal (2687) and duplicates (1396, 1426). Rename to `confirm(title, html)` and `#confirmDlg`. Four promise-wrapped dialogs (`askSameName` 1873, `askDelete` 2218, `browseForFolder` 2534, `photoMissing` 2097) hand-roll the same listen/resolve/cleanup dance; one `openChoice(dlg)` helper covers them.
- Four direct `fetch` calls beside `api`/`post` (1401, 2134, 2243, 2560) exist because `api` cannot report the status code; make `api` throw `ApiError { status, body }`.
- "N photo(s)" pluralisation is written 16 times; localStorage try/catch 10 times; the "after a face change" sequence (`await loadMeta(); refreshPeopleViews(); openViewer(viewerIndex)`) 4 times; `loadFolders().then(renderSettings if open)` 3 times; three hand-written day parsers (`fmtDay` 726, `fmtDate` 1148, `dateLabel` 794); the identical `face()` template at 1742 and 1863; `start()` (2667) and its inline copy for rescan (2676). Add `plural()`, `pref.get/set`, `parseDay()`, and reuse `start()`.
- People-grid geometry constants (110, 112, 44, 250, 100, 150, 14, 6) are written in both CSS (269-308) and JS (`peopleLayoutFor`, 1495-1503) with a "must match" comment. Set them as CSS custom properties from JS, one source of truth.
- Names: four words for placeholders (`photoPlaceholders`, `cardPlaceholders`, `skeletonCard`, `L.skeleton`); `LOADING_DELAY` and `SKELETON_DELAY` are the same 150 ms; `loadMeta` does four things.
- Dead: class `sort-label` (598) has no CSS; `groupTitle`'s `sample` parameter (1019) is only used for places; the `k === "people"` chip branch (1176) has no producer. A scripted check found no other unused CSS selector.

### 4.5 CSS

- Tokens (8-20) are used consistently outside the viewer. The viewer (373-452) is an intentional always-dark surface with about 20 hard-coded greys; give it its own token block (`--v-panel`, `--v-line`, `--v-muted`).
- Dark-mode gaps: `.btn.danger`, `.err`, `.folder .off` use `#c2410c` in both schemes (about 4.1:1 on the dark panel, borderline for 12-14 px text); `#toast.error` is fixed. Add `--danger` / `--danger-ink` tokens.
- Breakpoints 639 px and 1099 px are consistent in CSS, but JS has its own copies (`isPhone()` 872, `stage.clientWidth < 640` at 1995). Expose them once through `matchMedia`.
- Animation durations are duplicated between CSS and JS (350 ms vs 0.28 s, 220 ms vs 0.2 s). One `!important` (535) exists only because of the rule at 113.

### 4.6 Accessibility

Ranked by impact:

1. Group cards are `<div>`s (1263): with the default grouping by month, the main Photos view cannot be opened from the keyboard. Make them `<a href="#date=...">` (the hash router already encodes the state).
2. Merge/assign candidates are `<div>`s (1773); the picker cannot be used beyond typing in the filter.
3. Click-to-rename spans (853, 964, 2069) and people avatars (1458, 1472) are not focusable.
4. `<button role="listitem">` at 2524 removes the button role; use `<ul><li><button>`.
5. The viewer (701) is not a dialog: no `role`, `aria-modal`, focus move on open or restore on close; Tab leaves it. Make it a `<dialog>` opened with `showModal()`, like the other dialogs.
6. Sidebar checkboxes all have `aria-label="Select"` (852); use the person's name.
7. Tabs and segmented controls express state only through a class; add `aria-pressed` or `role="tablist"`.
8. Reduced motion covers the people grid and shimmer but not the view fade (329), toast (342), column transition (104), drawer (528), zoom (383), turning (384) or the swipe translate (2385).

Positive: `esc()` everywhere, `focus-visible` outlines, `aria-expanded` kept in sync, deliberate initial focus in dialogs.

### 4.7 Performance

- `people.find` inside `d.faces.map` twice per viewer open (2046, 2065): O(faces x people), 10 x 20k lookups per arrow press on a large library. Build `peopleById` in `loadMeta` next to `placeById` and use it in all 13 `people.find` sites.
- `mountPeopleGrid.draw()` (1534-1576) calls `update()` for every visible card on every scroll frame, with four `querySelector`s each; skip it on plain scrolls.
- `frameEl()` and `$(".stage", viewer)` are queried on every `pointermove` and wheel event (2315, 2328, 2341); hoist them.
- `renderOptimization` refetches the whole duplicates report every second while a search runs (1352); poll `/api/duplicates/progress` instead.
- `/api/photos/{id}` is refetched on every viewer move (2043); a small cache keyed by `id:version` makes arrow navigation instant.

### 4.8 Error handling

Silent failures that should not be silent: the one-second Optimization refresh (1352) stops rescheduling after one failed request and the view stays on "Checking…" forever; a failed duplicate search (1383) leaves the button disabled with no message; a failed `/api/photos/{id}` in `openViewer` leaves the previous photo's details in the panel; `/api/open` (2450) has no catch; the view-level error page (1934-1937) replaces the filter bar and offers no retry. After the server goes away the UI freezes silently; one toast after three consecutive failed polls would help.

## 5. Desktop app, scripts and releases

### 5.1 Desktop app (`desktop/src`)

Small, readable, consistent error handling. Points to plan:

- **Stale runtimes:** `runtime::install` extracts into `app_data/runtime/<version>/` and never removes other versions (`runtime.rs:33-45`), so a user who installed 0.1.4 to 0.1.9 carries about 0.5 GB of old copies. Delete sibling version folders after a successful install, or key the folder by content hash so unchanged files are shared.
- **Integrity:** `write_once` trusts a length match (`runtime.rs:47-50`). Embed a BLAKE3 hash per file (already a dependency) and verify it; the same hashes pin the fetch scripts.
- **Logs vanish on Windows:** `init_logging` writes to stderr (`app.rs:125-129`) and the release build has `windows_subsystem = "windows"`, so desktop logs go nowhere and the README has no "where are the logs" answer. Add `tracing-appender` to `app_data/logs/` with rotation and an "Open log folder" link in Settings.
- **First-run latency:** `runtime::install` runs inside `setup` before the window exists (`main.rs:80, 110`); show the window first with a "Starting…" page.
- **Duplicated constant:** `FACE_THRESHOLD = 0.42` in `desktop/src/main.rs:15` and the clap default in `src/main.rs:25`; export `DEFAULT_FACE_THRESHOLD` from the library.
- **Comments to add:** why `csp: null` is fine (the window loads a loopback URL; Tauri IPC is unavailable to remote origins), and that `blocking_pick_folder` must never run on the main thread (it would deadlock a future macOS build).
- `let _ = std::fs::write(path, &report)` in the self-test swallows errors; the self-test could also hit `/api/status` once so the server path is exercised.
- Keep embedding the models and ONNX Runtime (offline first start, single file, matches the "nothing to install" promise); document the trade-off and the SmartScreen implications in `docs/architecture.md`.

### 5.2 Scripts

- The `.sh` and `.ps1` pairs implement the same logic twice, and the VC++ copy exists only in `.ps1`. A `cargo xtask fetch` (one implementation for all platforms, called from the `justfile`, CI and `build.rs`'s error message) removes the duplication.
- **No checksums anywhere.** Add SHA-256 pins for `buffalo_s.zip` and each ONNX Runtime archive and verify them; these bytes end up in every release binary.
- Add `$ProgressPreference = 'SilentlyContinue'` before `Invoke-WebRequest` (Windows PowerShell 5 downloads are many times slower with the progress bar).

### 5.3 CI and releases (`.github/workflows/desktop.yml`)

What exists is good as far as it goes (rust-cache, AppImage self-test with fallback diagnostics and annotations, tag-triggered release with written notes). Gaps:

- No `pull_request` trigger, so contributors get no checks.
- No `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`, no CLI build on any OS, no macOS build. The existing tests have never run in CI.
- **`cargo test --workspace` and `cargo clippy --workspace` fail on a fresh clone** because `desktop/build.rs:21-26` panics when `models/` and `onnxruntime/` are absent. Add `default-members = ["."]` to the root `[workspace]`, or let `build.rs` skip the check under an environment variable for lint and test jobs.
- `cargo install tauri-cli` on every run (`:58`) costs minutes; use `taiki-e/install-action` with `tauri-cli@2`.
- Test photos are downloaded from Wikimedia and a third-party repository on every run (`:71-78`): flaky, rate-limited, and the portrait is a real public figure. Replace with a committed `fixtures/` folder of small CC0 images.
- `permissions: contents: write` applies to the build job too (`:11-12`); scope it to the release job. No `concurrency` group, `timeout-minutes` or artifact `retention-days`.
- Actions are referenced by major tag; pin to commit SHAs (Dependabot keeps them fresh) given the release job has write permission.
- The release job silently produces an empty body when the notes file is missing; nothing checks that the tag equals the Cargo version; no checksums or provenance are attached.

**Target layout**

```
ci.yml       on: pull_request, push(main)
  fmt        cargo fmt --all --check
  clippy     cargo clippy -p totufoto --all-targets -- -D warnings
  test       matrix ubuntu / macos / windows: cargo test -p totufoto --locked
  msrv       toolchain 1.88: cargo check -p totufoto --locked
  deny       cargo-deny-action (advisories, licenses, bans)
  ui         Playwright suite (section 6.3), chromium only
desktop.yml  on: push(tags v*), workflow_dispatch
  build      linux-x86_64, windows-x64, macos-aarch64 (cached tauri-cli, self-test)
  release    verify tag == version and notes file exists; SHA256SUMS;
             attest-build-provenance; gh-release
```

**Releasing:** today four edits (`Cargo.toml:3`, `desktop/Cargo.toml:3`, `desktop/tauri.conf.json:4`, the notes file) plus `Cargo.lock`. After the workspace version change (2.2 item 3), a `scripts/release.sh X.Y.Z` (or `cargo-release`) checks a clean tree and the notes file, bumps, commits "Release X.Y.Z" and tags. Standardise the notes template (intro, Downloads generated from a template, What's new, Fixes, Known issues, the model license line) and validate it in CI. Semver for 0.x: patch for fixes, minor for features or any schema or on-disk layout change; make "the index carries over" a tested guarantee with a migration test.

## 6. Testing strategy

### 6.1 Today

6 unit tests: `db::tests::unique_names` and five in `rotate::tests` (the rotate tests are the model to follow: they generate images, build EXIF blocks by hand and check pixels after a round trip). No integration tests, no UI tests, CI runs none of them.

### 6.2 Backend

1. Split `db::open` into `open(path)` and `pub(crate) fn init(conn)`, add `#[cfg(test)] open_in_memory()`. Nearly every function already takes `&mut Connection`, so nothing else changes.
2. A `tests/common.rs` fixture that writes tiny JPEG/PNG files into a `TempDir` using the `sample()` approach from `rotate.rs`, with the EXIF builder generalised to add `DateTimeOriginal` and GPS, so `scan::run` can be tested without models or network (`models: None`).
3. HTTP tests with `tower::ServiceExt::oneshot` against `router()` and an in-memory pool.

First twelve cases, covering every destructive path:

1. Root nesting: fixed `/a` + saved `/a/b` gives `[/a]`; saved `/a/b` then `/a` gives `[/a]`; identical fixed and saved root appears once; siblings `/a` and `/ab` both kept.
2. Pruning: a deleted file is pruned; rows under a root whose directory is missing are kept; with no fixed roots, photos outside the current roots are not pruned; a root that yields zero files with existing photos is treated as offline (after 3.2 item 3).
3. `excluded` paths and unchanged `failures` are skipped; a failure is retried once its mtime changes.
4. `people_filter` `all` / `any` / `only` on 3 photos and 2 persons plus a hidden one.
5. `date_range_filter`: reversed range swapped, invalid dates ignored, `to` inclusive, `upcoming` clamped and excluding the current year.
6. `duplicates::report` ordering and `delete` with `only`, outside-root and changed-file skips, rows forgotten.
7. `rotate_photo` end to end: size swapped, `version + 1`, face box transformed, following scan indexes nothing, `Changed` after touching the file, `Outside` for a foreign path.
8. `migrate()` from a legacy schema adds every column and index; idempotent.
9. `rename_person` uniqueness and name surrender; `assign_face` and `move_face_to_new_person` delete vacated unnamed persons but keep named ones.
10. `cluster::update_groups` with synthetic embeddings: two clusters and a singleton; incremental join; `rejected` faces never move.
11. `check_host` allow and deny; same-origin check from 3.3 item 1.
12. Handler status codes after the `ApiError` change, including two concurrent `duplicates/delete` calls (one 409, flag cleared afterwards).

### 6.3 UI

A committed Playwright suite under `web/tests/` (`package.json` with only `@playwright/test`, `playwright.config.ts`, `fixtures/library/` of a dozen generated JPEGs: different months, GPS, two identical files, faces when models are present). `webServer` runs `cargo run --release -- --data <tmp> --port 7979 --no-faces web/tests/fixtures/library`; a global setup waits for `/api/status` to be idle. A second project runs with faces when `models/` exists and is skipped otherwise. Add a few stable `data-test` attributes rather than relying on text.

First scenarios: welcome screen and the browse dialog; month cards, opening one, Back restoring cards and scroll; group and sort changes reflected in the URL; date range chip and Clear all; viewer keys (arrows, Esc, `I`), face boxes; zoom; rotate (`?v=` changes, tile ratio flips); delete from gallery with the "Show again" path in Settings; rename and the same-name dialog; merge via "Same as…"; sidebar Together/Any/Only; Upcoming days; duplicates report and a cancelled deletion; adding and removing a folder with the progress row; legacy deep links (`#view=places`); phone viewport (drawer, scrim, View panel).

### 6.4 Fixtures

A `fixtures/` folder with 3-5 small CC0 images (one with a visible face, one with GPS EXIF) used by the Rust tests, the CI self-test (replacing the downloads), `just dev` and README screenshots.

## 7. Documentation

- **README:** `README.md:3` and `:154` still say photos are "never changed" / "never modified", which contradicts Rotate and Remove from disk; reword to "never uploaded; files change only when you ask (rotate, delete)". Add Privacy and security, License, Contributing, Screenshots (there are none), Limitations (HEIC only on macOS, no RAW or video, photos tracked by path, non-commercial models, no auth) and a refreshed Troubleshooting (logs, `--self-test`, "face recognition disabled", missing WebView2). The "Using the gallery" section (`:128-141`, bullets over 800 characters) is a user manual; move it to `docs/user-guide.md` and keep a ten-line tour. The Layout list (`:193-206`) misses `lib.rs`, `duplicates.rs`, `rotate.rs`, `imaging.rs`; replace it with a short "How it works" and move the per-file list to `docs/architecture.md`. The data folder table (`:33-38`) omits macOS and the `runtime/<version>/` subfolder. Drop the pre-release "illegal instruction" troubleshooting entry (`:183`) and date the benchmark (`:191`).
- **`docs/`:** `architecture.md` (scan pipeline, SQLite index, axum API, single-page UI, Tauri shell, concurrency model, embedding trade-off), `user-guide.md`, `api.md` (routes, positional row format, cache contract, stability promise), `releasing.md`.
- **Developer experience:** a `justfile` with `fetch`, `dev` (CLI on `fixtures/` with a temporary data dir), `test`, `lint` (fmt + clippy), `ui-test`, `desktop`, `release X.Y.Z`; say in the README what to expect (CLI build 10-15 minutes, desktop on Linux 25 minutes or more).

## 8. Roadmap

Effort: S under half a day, M one to two days, L a week or more. Dependencies are noted where order matters.

### Phase 1: before announcing (about two days)

| # | Change | Effort | Section |
|---|---|---|---|
| 1 | `LICENSE` (PolyForm Noncommercial 1.0.0), `THIRD_PARTY.md` (ORT, VC++, InsightFace, GeoNames), License section in README and release notes with the source-available wording, `.github/FUNDING.yml` and the donation link | S | 2.2 |
| 2 | Cargo metadata with `license = "PolyForm-Noncommercial-1.0.0"`; `[workspace.package] version`; drop `version` from `tauri.conf.json` | S | 2.2 |
| 3 | Same-origin check for non-GET requests, always on; keep the Host allow-list built from own addresses | S | 3.3 |
| 4 | Folder add/remove/browse/pick only for loopback or the desktop app | S | 3.3 |
| 5 | `Drop` guards for `running` and `deleting` | S | 3.2 |
| 6 | `ci.yml` with fmt, clippy `-D warnings`, `cargo test` on three OSes; `default-members` or a build.rs skip flag so it works on a fresh clone | M | 5.3 |
| 7 | `rustfmt.toml` plus one formatting commit; `.editorconfig` | S | 2.2 |
| 8 | README: fix the "never changed" claims, Privacy and security section, `--host` warning; `SECURITY.md` | S | 7 |
| 9 | SHA-256 pins in the fetch scripts | S | 5.2 |

### Phase 2: make the service safe to refactor (about one week)

| # | Change | Effort | Depends on |
|---|---|---|---|
| 10 | `db::init(conn)` and `open_in_memory()`; test fixtures with generated images | S | |
| 11 | Typed `ApiError` enum with JSON bodies, used by every handler | M | |
| 12 | `library.rs`: `roots`, `root_of`, `photos_under`, `add_root`, `remove_root`, `check_photo`, `remove_photo` with `Outcome` enums | M | 11 |
| 13 | Backend tests 1-9 from 6.2 | M | 10, 12 |
| 14 | Fix pruning of empty mount points and keep ids stable for modified files | M | 13 |
| 15 | Bound the `Pool`; `compare_exchange` in `scan::spawn`; stop producers on write failure; `original` 404 | S | |
| 16 | `fixtures/` folder; CI self-test and `just dev` use it | S | |

### Phase 3: split the big files (about one week)

| # | Change | Effort | Depends on |
|---|---|---|---|
| 17 | Move SQL out of `server.rs` into `db/` modules returning `Serialize` structs; split handlers into `http/` | L | 11, 12, 13 |
| 18 | UI step 1: `index.html` + `app.css` + `app.js`, versioned URLs | S | |
| 19 | UI quick wins: `peopleById`, `confirm()` helper and `#confirmDlg`, `ApiError` in `api()`, the three silent-failure fixes, `/api/open` catch, About entry in Settings with version, license and donation link | S | 18 |
| 20 | UI accessibility batch: cards and candidates as buttons or links, focusable rename spans and avatars, `listitem` fix, viewer as `<dialog>`, reduced-motion block | S | 18 |
| 21 | UI helpers: icon sprite, `plural()`, `pref`, `parseDay()`, geometry via custom properties, `ViewLoad` object used by every view | M | 18 |

### Phase 4: tests and process (about one week)

| # | Change | Effort | Depends on |
|---|---|---|---|
| 22 | Committed Playwright suite with the first 16 scenarios; `ui` job in CI | M | 16, 18 |
| 23 | Release hardening: tag/version check, fail on missing notes, `SHA256SUMS`, provenance attestation, cached tauri-cli, scoped permissions, SHA-pinned actions, Dependabot | M | |
| 24 | `justfile`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, issue templates, `docs/architecture.md`, `docs/user-guide.md`, `docs/api.md`, `docs/releasing.md`; README restructure with screenshots | M | |
| 25 | `cargo-deny` and MSRV jobs; `clap` optional; drop unused features; `strip = "symbols"` | S | |

### Phase 5: later

| # | Change | Effort |
|---|---|---|
| 26 | UI step 2: ES modules, event delegation instead of inline handlers, grouped viewer and people state | L |
| 27 | Desktop: remove stale `runtime/<version>` folders, verify embedded files by hash, file logging with an "Open log folder" link, window before extraction | M |
| 28 | macOS desktop build in `desktop.yml`; `cargo xtask fetch` replacing the four scripts | L |
| 29 | API cleanup before 1.0: route verbs, `/thumb/{id}/{version}`, objects instead of positional rows, `update_person` response | M |
| 30 | Remaining backend tests (10-12), `exif.rs` with tests, `migrate()` via `user_version`, `db.rs` module doc, concurrency model doc | M |

## 9. What not to change

- The single-binary design with the UI embedded and served by axum, and the desktop shell loading a loopback URL: it is the reason the project needs no installer and no build step, and both reviews recommend keeping it.
- Embedding the models and ONNX Runtime in the desktop binary: offline first start and the self-test depend on it. Document the trade-off instead.
- The hand-written release notes as the source of truth for the changelog; generate the Downloads section and validate the file in CI rather than replacing it with commit logs.
- The virtualised people grid, the stable sidebar ordering, the `beginViewLoad` choreography and the rotate tests: these are the parts other code should be brought up to, not rewritten.
