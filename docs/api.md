# HTTP API

The gallery's page talks to the program through this API, and scripts can use it too. It is served on the same address as the page (`http://127.0.0.1:7878` by default).

**Stability**: before version 1.0 the API can change in a minor release (0.2, 0.3...), never in a patch release; the page in `web/` is the client it is written for. Changes are listed below and in the release notes.

## Changes in 0.3

The gallery shows videos as well as photos, so what the API calls a photo is now an **item** (a photo or a video).

| Before (0.2) | Now |
|---|---|
| `GET /api/photos`, answering `{"photos": [...]}` | `GET /api/items`, answering `{"items": [...]}` |
| `/api/photos/{id}`, and its `/check`, `/reveal` and `/rotate` | `/api/items/{id}`, and its `/check`, `/reveal` and `/rotate` |
| `"photos"` (how many) on each folder of `GET /api/folders` | `"items"` |
| `indexing photos` as the `phase` of `GET /api/status` | `indexing files` |
| | videos: `duration` as the seventh value of each row, `videos` in `/api/groups`, `duration` in `/api/items/{id}` |
| | `PUT /api/items/{id}/thumb` (a video's thumbnail, made by the page), `/thumb` answering `204` for a video without one |
| | `?silent=1` on `/original/{id}`: a video without its sound |
| | `GET` and `PUT /api/uploads`: uploading photos and videos (server only) |

## Changes in 0.2

| Before (0.1) | Now |
|---|---|
| `POST /api/photos/{id}/remove` with `{"from", "permanently"}` | `DELETE /api/photos/{id}?from=gallery` (or `from=disk`, `&permanently=true`) |
| `POST /api/folders/remove` with `{"path"}` | `DELETE /api/folders?path=...` |
| `POST /api/excluded/clear` | `DELETE /api/excluded` |
| `POST /api/people/{id}`, answering `{"name"}` or nothing | `PATCH /api/people/{id}`, always answering with the person |
| `GET /thumb/{id}?v={version}` | `GET /thumb/{id}/{version}` |
| | `version` on each file of `GET /api/duplicates` |

## Conventions

- Requests and answers are JSON. An error is a status code with `{"error": "message"}`.
- Requests that change something (`POST`, `PATCH`, `DELETE`) must come from the gallery's own page or from a program that isn't a browser: browsers send `Sec-Fetch-Site`, and only `same-origin` or `none` are accepted (or else an `Origin` matching the address). `curl` and scripts send neither, so they work.
- The `Host` header must be an IP address, `localhost`, the computer's own name or a name given with `--allow-host`. Otherwise, and for refused cross-site requests, the answer is `403` with a plain-text message.
- There is no login: anyone who can reach the address can use every endpoint.
- Ids are integers. Dates are `YYYY-MM-DD`; capture times are `YYYY-MM-DD HH:MM:SS` in the photo's local time.
- `202 Accepted` means the work was started in the background; follow it with `/api/status`.
- Photo files are tied to the folders: deleting and rotating only work on files inside the photo folders (`403` otherwise).

## Items (photos and videos)

### `GET /api/items`

The photos and videos matching a filter.

| Query | |
|---|---|
| `sort` | `desc` (default, newest first) or `asc` |
| `people` | comma-separated person ids |
| `match` | how the people combine: `all` (default, all of them together), `any`, `only` (all of them and no other known person) |
| `place` | a place id (`0`: no location) |
| `date` | a capture date prefix: `2024`, `2024-10` or `2024-10-05` |
| `from`, `to` | a capture date range, both days included |
| `upcoming` | photos from earlier years whose anniversary falls in the next N days |

```json
{ "items": [[42, 4032, 3024, "2024-10-05 18:22:01", 7, 0], ...], "days": [] }
```

Each photo or video is an array, to keep large libraries small: `[id, width, height, taken, place_id or null, version, duration]`. Width and height are as shown (after the EXIF orientation, or a video's rotation). `duration` is null for photos, and a video's length in seconds (0 when the file doesn't say). `days` lists the days (`MM-DD`) that `upcoming` covers.

### `GET /api/groups`

The same filters plus `by` (`year`, `month`, `day` or `place`): one line per group, for the cards.

```json
{ "groups": [{ "key": "2024-10", "count": 31, "cover": 42, "v": 0, "videos": 2 }], "total": 31, "videos": 2 }
```

`key` is the date prefix, or the place id with `by=place`. `cover` is the newest photo of the group (the oldest with `sort=asc`), and `v` its version.

### `GET /api/places`

Places of the items, the most items first. Takes `people`, `match`, `from` and `to`.

```json
[{ "id": 7, "city": "Madrid", "region": "Madrid", "country": "ES", "count": 120, "cover": 42 }]
```

### `GET /api/items/{id}`

```json
{
  "id": 42, "path": "/home/ana/Pictures/2024/IMG_0001.jpg",
  "taken": "2024-10-05 18:22:01", "dateFromExif": true,
  "width": 4032, "height": 3024,
  "lat": 40.41, "lon": -3.70, "city": "Madrid", "region": "Madrid", "country": "ES",
  "version": 0, "rotatable": true,
  "faces": [{ "id": 9, "box": [0.31, 0.22, 0.12, 0.16], "person": 3, "name": "Ana" }]
}
```

A face's `box` is `[x, y, width, height]` as fractions of the photo as shown.

### Pictures

| | |
|---|---|
| `GET /thumb/{id}/{version}` | the thumbnail, JPEG. `204 No Content` for a video whose thumbnail the page hasn't made yet |
| `PUT /api/items/{id}/thumb?v={version}` | a video's thumbnail, made by the page: a JPEG frame of the video at that version as the body. The server makes its own thumbnail from it and raises the version: `{"version": n}`. `409` for a photo or when the video changed, `400` for a body that isn't a JPEG |
| `GET /face/{id}` | a face's picture, JPEG |
| `GET /original/{id}` | the file itself (HEIC and TIFF converted to JPEG); `?download=1` to save it with its own name. Videos are streamed and answer range requests (`Range: bytes=...`), for seeking; `?silent=1` serves a video with its sound tracks marked as filler (same size and offsets), for videos whose broken sound makes the browser refuse them (the page takes thumbnail frames from it, and plays it when the original fails) |

Thumbnails and faces are cached for good by browsers (`immutable`). A photo's version goes up when it is rotated or its file changes, so its thumbnail gets a new address; an address with an old version still answers with the current picture, but marked not to be kept (`no-cache`).

### `POST /api/items/{id}/rotate`

`{"turns": 1}`: quarter turns clockwise (negative: counterclockwise). Answers the new `{"width", "height", "version"}`.

`409` while a scan runs or when the file changed since it was indexed; `415` for formats other than JPEG and PNG.

### `DELETE /api/items/{id}`

| Query | |
|---|---|
| `from=gallery` | leave the photo out of the gallery; the file stays |
| `from=disk` | move the file to the bin |
| `from=disk&permanently=true` | delete it for good if there is no bin |

Answers `{"status": "removed" or "binned" or "deleted", "path": ...}`. When the file can't go to a bin, the answer is `409` with `{"status": "no-bin", ...}`, and nothing is deleted until asked again with `permanently=true`.

### `POST /api/items/{id}/check`

For a photo that can't be opened: `{"status": "present"}`, `{"status": "removed"}` (its file is gone, so it left the index) or `{"status": "unavailable", "folder": ...}` (its folder can't be reached, so nothing changed).

### `POST /api/items/{id}/reveal`

Shows the file in the file manager. Desktop app only (`501` otherwise).

## People and faces

| | |
|---|---|
| `GET /api/people` | `[{"id", "name", "hidden", "count", "face"}]`: everyone with a face, named people first. `face` is the face on their card |
| `PATCH /api/people/{id}` | `{"name": "Ana"}` renames (`null` forgets the name), `{"hidden": true}` hides; both at once work too. Answers with the person, as in `/api/people`. Names are kept unique, so the name saved may have a number added ("Ana (1)") |
| `POST /api/people/{id}/merge` | `{"into": 5}`: this person's faces go to person 5 |
| `POST /api/faces/{id}/assign` | `{"person": 5}`: this one face goes to person 5 |
| `POST /api/faces/{id}/reject` | "Not them": the face gets a new unnamed person, answered as `{"person": id}` |
| `POST /api/faces/{id}/cover` | shows this face on its person's card |

Faces moved by hand are never moved by automatic grouping.

## Library and scanning

### `GET /api/status`

Progress of the current or last scan, cheap enough to poll. `phase` is `listing files`, `indexing files`, `grouping faces` or, after an error, `failed`.

```json
{
  "running": true, "phase": "indexing files", "total": 1200, "done": 300, "errors": 0, "faces": 410,
  "group_done": 0, "group_total": 0,
  "removing": null, "remove_done": 0, "remove_total": 0,
  "failed": 2, "excluded": 1, "version": "0.1.11", "project": "https://github.com/...", "logs": false
}
```

| | |
|---|---|
| `POST /api/scan` | scan now (`202`); one asked for during a scan runs after it |
| `POST /api/regroup` | scan, then regroup every face from scratch (`202`) |
| `GET /api/failures` | files that could not be read: `[{"path", "error"}]` (the first 500) |
| `POST /api/failures/retry` | forget them and scan, so they are tried again (`202`) |
| `DELETE /api/excluded` | bring back the items removed from the gallery (`202`) |

### Folders

| | |
|---|---|
| `GET /api/folders` | `{"folders": [{"path", "available", "items", "fixed"}], "desktop": bool}`. `fixed`: given on the command line. `desktop`: running in the desktop app |
| `POST /api/folders` | `{"path": "/home/ana/Pictures"}` adds a folder and scans it. `409` when it is already in, or inside a folder that is |
| `POST /api/folders/pick` | opens the folder picker and adds the folder chosen (desktop app only; `204` when cancelled) |
| `DELETE /api/folders?path=...` | takes the folder's items out of the gallery (the files stay), and answers `{"removed", "kept"}` when done; `kept` are items another folder still includes. One at a time (`409`) |
| `GET /api/folders/browse?path=` | the folders inside `path` on the computer running Imadive: `{"path", "parent", "dirs": [{"name", "path"}]}`. Without `path`: next to the first photo folder |

### Uploads

Only on the server (`501` in the desktop app). Uploads go into an `imaDive-uploads` folder inside one of the gallery's folders; a scan (`POST /api/scan`) adds them to the gallery afterwards.

| | |
|---|---|
| `GET /api/uploads` | `{"folders": [...], "subfolder": "imaDive-uploads", "extensions": ["jpg", ...]}`: the gallery folders that can take uploads now (reachable, or empty and not a drive mount point in `/etc/fstab`), and the file types the gallery shows |
| `PUT /api/uploads?folder=...&path=...` | one file, as the body (any size; streamed to disk, and only put in place once complete). `folder`: one of `folders`. `path`: the file's name, or its path inside the folder chosen in the browser (`Holidays/beach/1.jpg`); plain names only, no `..` or hidden parts. Answers `201` with `{"status": "saved"}`, or `"renamed"` when another file had the name (it gets ` (2)`), and the `path` it was saved at; `200` with `"same"` when that very file is already there. `400` for a bad path or an empty file, `403` for a folder that isn't in the gallery or can't be written, `415` for a file that isn't a photo or video |

### Identical files

| | |
|---|---|
| `GET /api/duplicates` | the sets of identical files, the copy kept and the others, with the search progress |
| `POST /api/duplicates/search` | search again (`202`) |
| `POST /api/duplicates/delete` | `{}` moves every copy to the bin, keeping the oldest file of each set; `{"ids": [...]}` only those; `"permanently": true` deletes for good. Answers when done; `409` if a deletion is already running |
| `GET /api/duplicates/progress` | cheap, for polling: `{"scanning", "searching", "checked", "to_check"}` for the search, `{"deleting", "done", "total", "freed"}` for a deletion |

### Desktop

- `POST /api/open` with `{"url": ...}` opens a link in the system browser: desktop app only, and only for the map and project links the page uses.
- `POST /api/logs/reveal` shows the log file in the file manager: desktop app only. `logs` in `/api/status` says whether there is one.
