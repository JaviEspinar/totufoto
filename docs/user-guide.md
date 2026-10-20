# Using Imadive

How the gallery works, tab by tab. To install it, see the [README](../README.md).

- [Photos](#photos)
- [People](#people)
- [Upcoming](#upcoming)
- [The sidebar: people in the photo](#the-sidebar-people-in-the-photo)
- [Dates and filter chips](#dates-and-filter-chips)
- [The viewer](#the-viewer)
- [Rotating, downloading and deleting](#rotating-downloading-and-deleting)
- [Videos](#videos)
- [Manage: uploading and identical files](#manage-uploading-and-identical-files)
- [Settings: folders and the library](#settings-folders-and-the-library)
- [Phones and tablets](#phones-and-tablets)
- [The index](#the-index)
- [Keyboard](#keyboard)

## Photos

The timeline, sorted by capture date: the EXIF `DateTimeOriginal`, or the file date when a photo has none.

- With *Group* set to a year, month, day or place, the groups show as cards (cover, title, number of photos). Click one to see its photos, with a chip for that group, and close the chip to go back to the cards.
- Places come with the most photos first, and a *No location* card holds the photos without GPS data. Places are found offline, from a list of the cities with more than 1000 inhabitants (GeoNames).
- *No groups* shows all photos.
- *Newest/Oldest first* in the top bar sets the order.
- The bar with the filter chips and the photo count stays at the top while you scroll.

Every filter lives in the address, so a view can be bookmarked or shared with someone on the same network.

## People

Faces are found in every photo and grouped into people automatically. The tab shows them in two parts: **Classified**, the people you have named, and **Not classified**, the groups without a name yet, always with the most photos first (so the people who matter most come up first).

- Name each person by typing under their face: they move to *Classified*.
- Click a face to see that person's photos in Photos. It shows only them: filters set before (dates, a place, other people) are cleared. To combine people with other filters, tick them in the *People in the photo* column instead.
- *Same as…* merges two groups of the same person.
- *Hide* is for people you don't care about.
- Switch between *Cards* (big faces) and *List* (compact rows, several times more people on screen; the actions show when you point at a row). The size slider next to it makes cards or rows bigger or smaller.
- Sort the classified people by *Most photos* or by *Name* (A to Z); the sidebar uses the same order.

Names are unique: using a name someone already has asks whether they are the same person (*Merge them*) or not (*Keep separate* adds a number, like "Ana (1)").

After each scan only the new faces are placed into people, which is fast even with hundreds of thousands of faces. Regrouping everything from scratch is slower, and only happens on the first index or when you ask for it in Settings. Automatic grouping never moves a face you placed yourself.

## Upcoming

Memories from past years whose anniversary falls in the next 7 to 90 days ("2 years ago today").

## The sidebar: people in the photo

- Tick people (click the checkbox or their picture) to filter Photos and Upcoming.
- Pick how they combine:
  - *Together*: every selected person is in the photo,
  - *Any*: at least one of them,
  - *Only them*: all of them and no other known person.
- Click a name to rename that person right there (Enter saves, Esc cancels).
- The button at the top right of the column collapses it to a thin strip (showing how many people are selected), and opens it again.

## Dates and filter chips

- *From* and *to* in the top bar keep only photos taken between those days (both included; either can be left empty). The range applies to Photos and Upcoming.
- Each selected person, place and date range shows as a chip above the photos: × removes one, *Clear all* removes them all.

All filters combine: people, place, date range and upcoming.

## The viewer

Click a photo to open it. Use the arrow keys to move and `Esc` to close.

- **Zoom**: click the photo to zoom in where you clicked (click again to fit), drag to move around, and use the mouse wheel or a trackpad pinch to zoom in and out (up to 8x).
- **Details**: the side panel shows the date, place and people. The button at the top right of the details (or the `I` key) folds them away to give the photo more room, like the *People in the photo* column; the choice is remembered.
- **Naming**: click the name of an unnamed person (or of a face in no group) to name it there; a name that already exists offers to merge.
- **Correcting faces**: for each face, *Same as…* moves just that face to the person you pick, and *Not them* moves it to a new unnamed group, which you can rename, hide or merge in People.
- **Card photo** makes that face the one shown on the person's card in People (it stays while the face belongs to them).
- **Show face boxes** draws the detected faces.

## Rotating, downloading and deleting

The buttons under the photo's date in the viewer.

**Rotate** (or `R` and `Shift+R`) turns the photo right or left, and the file is saved turned.

- JPEG photos are turned through their EXIF orientation, so the picture is not compressed again and loses no quality. PNG is rewritten turned (it is lossless). Other formats (HEIC, WebP, GIF, TIFF) can't be rotated.
- Several quick turns are saved once.
- Face boxes, people and thumbnails stay right, and the photo isn't indexed again.
- Rotating waits while a scan is running.

**Download, Share and Open in folder**:

- In the browser there is *Download*, which saves the original file with its own name (HEIC and TIFF are saved as JPEG).
- The desktop app has *Share*, which opens the system share sheet with the photo (or *Download* where the system has none), and *Open in folder*, which shows the file selected in the file manager (Finder, Explorer, or the Linux file manager).

**Deleting**: the bin button (or the Delete key) asks how.

- *Remove from gallery* keeps the file and leaves it out of later scans. Settings can show such photos again.
- *Remove from disk* moves the file to the bin of the computer running Imadive. If its drive has no bin, it asks before deleting for good.

Only files inside the gallery's folders can be deleted. The gallery has no login, so anyone who can open it on your network can delete photos and videos too (see [Privacy and security](../README.md#privacy-and-security)).

## Videos

Videos are part of the timeline, places and Upcoming like photos: their date and place come from the file (MP4 and MOV, as phones and most cameras record them), or the file's date otherwise. In the grid they show a frame from about a second in, made by the gallery the first time it shows them in a browser that can play them (and kept for everyone), or the picture the camera saved beside them (`.THM`); until then, and for formats no browser plays, a generic picture with a play sign. Their length shows in a corner.

The viewer plays them with your browser's player. Formats a browser can't play (AVI, WMV, MPEG, and iPhone HEVC videos in some browsers) show a note with a link to download the file instead. When it's only the sound the browser can't play (some phones' videos, often ones sent with WhatsApp), the video plays without sound, with a note saying so. Videos have no people: faces are only looked for in photos.

## Manage: uploading and identical files

The *Manage* tab has two cards: uploading photos and videos (on the gallery served by a server), and the identical files. In the desktop app only the second one shows, since its photos are already on its computer.

### Identical files

After each scan, Imadive looks in the background for copies of the same photo or video and shows how much space they take. A copy counts as the same when it is the very same file, or the same picture with other details: for a JPEG, the picture without its metadata (but with its orientation, since a turned copy looks different); for an MP4 or MOV, the video without its details. That finds, for example, a photo uploaded from a phone's browser, which Android hands over without its location, next to the same photo synced with it. Only files that could have a twin are read (the same size, or the same size in pixels and capture time), so the search is quick. Other formats count only when the files are identical.

*Delete duplicates* moves the copies to the bin, keeping of each set the one with a place, then the one with the oldest file date (if two have the same date, one of them). A progress bar shows how far it has got; closing or reloading the page doesn't stop it. Each copy is checked just before it goes: it, and the file that is kept, must still be there and unchanged.

### Uploading photos and videos

On the gallery served by a server, the *Upload photos and videos* card sends photos and videos from the device you are using: a phone, a laptop. Drop a folder, or photos and videos, on it, or use *Choose a folder* or *Choose photos and videos*. Phones can't choose folders, and their *Choose photos and videos* opens the photo picker, one photo at a time and at most 100 (on Android). For many at once, use *Choose files*: it opens the file manager, where you open the folder (for example DCIM › Camera) and use *Select all* in its menu; files that aren't photos or videos are left out.

- They are saved in a folder called **`imaDive-uploads`** inside the gallery's folder, created the first time. With several gallery folders, a dialog asks which one.
- A folder keeps its folders: dropping `Holidays` puts its photos in `imaDive-uploads/Holidays/...`.
- Files that aren't photos or videos the gallery shows are left out, without being sent.
- When some files have the same names as ones already in `imaDive-uploads`, a dialog lists them and asks what to do with all of them: *Skip* them (they aren't sent), *Overwrite* the ones there, or *Keep both* (the new one is saved with a number, `photo (2).jpg`). A file exactly the same as the one there is never copied twice, whatever the choice.
- **On a phone, uploads lose their location:** Android hands photos and videos to the browser without it, so they don't show on the map. An app that syncs the camera folder to the server, such as Syncthing, keeps it.
- From the moment you choose, the card says what is happening (a phone can take a while to hand over a big folder, and the names are checked before sending). Then a progress bar shows the files and bytes sent. The upload goes on while you look at other tabs; leaving the page asks first, since it would stop it. *Cancel* stops it, keeping what was already sent.
- **Many files from a phone:** Android can take very long, or give up, handing thousands of files to the browser at once; the card says so beforehand. Choose a few hundred at a time: while they upload, *Add more…* adds another selection to the same upload. If Android gives up (the page comes back with nothing chosen, or reloads), the card explains it, and *Choose files* works again at once. For a whole phone, copying its camera folder to the server (with a cable, a network share or an app such as Syncthing) is faster.
- **Big uploads from a phone** (thousands of files): keep the page open and the screen on until it ends. If the connection drops (the phone locks its screen, the Wi-Fi hiccups), the card says *The connection was lost: trying again…* and the upload goes on by itself when the page is back and online, waiting a little longer each time (up to half a minute, six tries per file).
- Files the server refused, or that still couldn't be sent after those tries, are listed with the reason, and *Retry the ones that failed* sends them again, to the same folder.
- While the new files are indexed afterwards, the gallery keeps one processor core free and indexes at a lower priority (on Linux), so it stays usable.
- When it ends, the gallery scans for the new files: they appear in Photos within moments, and their faces are found as usual.

The server must be able to write in that folder: a gallery folder that is read-only (a Docker volume mounted with `:ro`, say) refuses uploads with an error saying so. An empty gallery folder takes uploads, unless `/etc/fstab` mounts a drive there that isn't connected: then they would end up on the system's disk.

## Settings: folders and the library

Settings is the gear at the top right. (The ImaDive logo at the top left shows the version, and how to support the project.)

**Folders**

- *Add folder…* opens the system folder picker in the desktop app. In the browser it lists the folders of the computer running Imadive: open one, or type or paste a path, then *Add this folder*.
- A folder inside one already in the gallery isn't needed and is refused. Adding a folder that contains others replaces them.
- *Remove* takes a folder's photos out of the gallery (the files stay on disk), except photos another folder still includes. Its row shows the progress. A scan still running stops first, and continues with the other folders afterwards. One folder is removed at a time.
- Folders given on the command line are marked *command line* and can't be removed here. One that can't be found when the gallery starts (a drive not mounted yet) shows as *not available* too, and is picked up by *Rescan* once it's back.
- A folder that can't be reached (an unplugged drive) shows as *not available*, and its photos stay in the gallery.
- A folder or file inside the library that can't be read during a scan (a disk or filesystem error) keeps its photos in the gallery too; the log names it. They are read again by a later scan, once the disk reads it.
- **Moved a folder** (to another drive, another place, or renamed it)? Its row shows *not available* and a *Moved to…* button: choose where it is now. Its photos and videos keep everything, people, faces you placed, card photos, items removed from the gallery and video thumbnails, under their new paths, instead of being indexed again as new files. A file counts as the same one when the new place has a file with the same name and size (its date may differ, as copies often get a new one). Files that changed are read again by the next scan, and files that aren't there leave the gallery. Only for folders added here: a command-line folder is changed where it is given.

**The library**

- The library status, with progress while indexing or grouping faces.
- *Rescan* picks up new, changed or deleted photos without restarting.
- Files that could not be read, with the reason. They are skipped until the file changes, or until you click *Try again*.
- Photos removed from the gallery, with *Show again*.
- *Regroup all faces* groups every face from scratch (named people are kept).
- *Appearance*: the theme (*System* follows your device; *Light* or *Dark* stays as chosen) and the language (English or Spanish; by default your browser's language if it is one of them, otherwise English). Both are remembered in each browser, so a phone and a computer can differ.
- *Backup*: see [Backups](#backups) below.
- *Updates*: see [Updates](#updates) below.
- *About*: the version, the license and the project's page. In the desktop app, *Open log folder* shows the log, to attach when reporting a problem. At the bottom, as in the logo's dialog, how to support the project.

While indexing, the first time, the Photos tab shows the progress until the first photos are in. After that a small card at the bottom right shows it on every tab: *Show N new* brings in the photos indexed since, *Details* opens Settings, *Hide* hides it until the next scan.

## Backups

*Settings → Library → Backup* keeps everything the gallery has learned that the photos themselves don't say: people and their names, the face groups, the faces you placed (*Not them*, *Same as*), card photos, hidden people, items removed from the gallery, and the list of folders (also those given on the command line). The photos and videos are **not** in it: back them up on their own. Neither are the theme and the language, which each browser keeps.

- *Back up…* saves a file named like `Imadive backup 2026-10-16.imadive` (` (2)` for a second one that day). On a server, you choose the folder of the server it goes in, in the same folder browser as *Add folder…*; in the desktop app, the system's save dialog asks where. The card shows the progress at once and says where it was saved; the gallery keeps working meanwhile. The photos' thumbnails are left out, so the file stays small; videos keep theirs.
- *Restore…* on a server lists the folders and backups of the server: click the backup to restore, or *Upload one from this device…* for one made elsewhere. The desktop app uses the system's open dialog. It says what the backup holds before anything changes: when it was made, how many photos, videos and named people, and its folders, marking those that aren't on this computer. *Restore* then replaces everything in this gallery with the backup; the backup file itself isn't changed.
- After a restore, the gallery scans: files changed since the backup are read again, and the photos' thumbnails are made again from the photos (those on screen first; the status card shows *Making thumbnails*). A folder of the backup that isn't on this computer keeps its photos as *not available*; if they are somewhere else here, use its *Moved to…* button.
- The index a restore replaced stays in the data folder as `index.before-restore.sqlite` (the last one only), in case the restore wasn't wanted.
- A backup made by a newer Imadive can't be restored until this one is updated.

## Updates

*Settings → Updates* says whether a newer release than yours is out, with *Check for updates* to look now and, when there is one, *What's new* (its release notes) and *Update*. Imadive looks on [GitHub](https://github.com/waiting4timeout/imadive/releases) a minute after it starts and then once a day; that request tells GitHub nothing but that someone is checking. To turn it off, start the command-line app with `--no-update-check`, or set the environment variable `IMADIVE_NO_UPDATE_CHECK=1` (for the desktop app too); *Check for updates* still works when you click it. A dot on the gear shows when an update is waiting.

What *Update* does depends on how Imadive was installed. The file is always checked against the release's published checksums before anything is replaced.

- **Windows** (.exe): the new version replaces the old file and the app starts again.
- **Linux AppImage**: the same, in place of the AppImage file.
- **Linux .deb**: the new package is installed with apt, after asking for your password in a system dialog; then the app starts again.
- **A server installed with `imadive setup`**: a small service of setup's (`imadive-update`) installs the new release as root and restarts the gallery; the page reloads when it's back. Servers installed with an older setup show the one command that adds that service.
- **A server built from source**: Settings shows the one command to run on the server (in a terminal or over SSH) to switch to the ready-made server. It is filled in with this server's settings: it stops its service and runs the install script, whose setup starts from the same user, index, photo folders and port, so nothing is indexed again and people keep their names. From then on, *Update* works from the browser.
- **Docker**: Settings shows the `docker pull` command to run on the computer that runs the container, since a container can't replace its own image.

## Phones and tablets

The gallery adapts to small screens; nothing needs installing, just open the server's address in the phone's browser.

- **Tablets and narrow windows**: dates, grouping and order are behind the sliders button in the top bar (a dot shows when a date range is active).
- **Phones**: the tabs get their own row, and *People in the photo* slides in from the people button (its badge shows how many are selected); tap outside to close it.
- **Back button**: it undoes one step at a time: it closes an open dialog, the open photo (back to where its thumbnail is) or a panel, then clears the filter chips, then goes back to Photos from another tab. With nothing left to undo it asks before leaving the gallery.
- **Viewer**: the photo uses the full width with the details below it. Swipe left or right for the next or previous photo, tap to zoom, drag with one finger and pinch to zoom in and out.

## The index

Everything the gallery learns is stored in one file, `index.sqlite` in the data folder: metadata, thumbnails, faces, names and corrections. Photos are never copied, and their files only change when you rotate or delete them.

- Stopping and starting again does **not** re-index. Only new or changed files are processed, and deleted ones are removed.
- Deleted photos are removed at the next scan. If you open one before that, the viewer tells you it is no longer in its folder and removes it at once; if its whole folder can't be reached (an unplugged drive), nothing is removed.
- A photo folder that is missing or completely empty is taken for an unplugged drive (on Linux an unmounted drive leaves an empty folder behind), so its photos stay in the gallery and Settings shows it as *not available*. If you really emptied it, remove it in Settings.
- A photo whose file changes (edited in another app, say) is read again and keeps its place in the gallery; its faces are found again.
- People you have named are remembered even when all their photos are gone: if the photos come back, or new ones appear, their faces rejoin the name.
- Photos are tracked by path. After moving or renaming a folder added in Settings, use its *Moved to…* button so nothing is indexed again (see [Settings](#settings-folders-and-the-library)). Otherwise, or for a command-line folder, the photos look new and are indexed again; named people are matched to the re-indexed faces automatically.
- With the command-line app, `--data DIR` keeps separate indexes for separate libraries. Delete the data folder to start from scratch.

Where the data folder is: `imadive-data` in the folder you start the command-line app from (or `--data`), and for the desktop app the folders listed in the [README](../README.md#desktop-app-windows-and-linux).

## Keyboard

| Key | Where | |
|---|---|---|
| `←` `→` | viewer | previous, next photo |
| `Esc` | viewer, dialogs | close |
| `I` | viewer | fold or show the details |
| `R`, `Shift+R` | viewer | rotate right, left |
| `Delete` | viewer | remove the photo (asks how) |
| `Enter`, `Space` | cards, faces, names | open, rename |
| `Tab` | everywhere | move between controls; the viewer keeps the focus inside while it is open |
