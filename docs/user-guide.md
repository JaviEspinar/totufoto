# Using Imadive

How the gallery works, tab by tab. To install it, see the [README](../README.md).

- [Photos](#photos)
- [Upcoming](#upcoming)
- [People](#people)
- [The sidebar: people in the photo](#the-sidebar-people-in-the-photo)
- [Dates and filter chips](#dates-and-filter-chips)
- [The viewer](#the-viewer)
- [Rotating, downloading and deleting](#rotating-downloading-and-deleting)
- [Optimization: identical files](#optimization-identical-files)
- [Settings: photo folders and the library](#settings-photo-folders-and-the-library)
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

## Upcoming

Memories from past years whose anniversary falls in the next 7 to 90 days ("2 years ago today").

## People

Faces are found in every photo and grouped into people automatically.

- Name each person by typing under their face.
- *Same as…* merges two groups of the same person.
- *Hide* is for people you don't care about.
- Switch between *Cards* (big faces) and *List* (compact rows, several times more people on screen; the actions show when you point at a row). The size slider next to it makes cards or rows bigger or smaller.
- Sort people by *Most photos* or by *Name* (A to Z, unnamed last); the sidebar uses the same order.

Names are unique: using a name someone already has asks whether they are the same person (*Merge them*) or not (*Keep separate* adds a number, like "Ana (1)").

After each scan only the new faces are placed into people, which is fast even with hundreds of thousands of faces. Regrouping everything from scratch is slower, and only happens on the first index or when you ask for it in Settings. Automatic grouping never moves a face you placed yourself.

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

Only files inside the photo folders can be deleted. The gallery has no login, so anyone who can open it on your network can delete photos too (see [Privacy and security](../README.md#privacy-and-security)).

## Optimization: identical files

After each scan, Imadive looks in the background for identical photo files (the very same bytes) and shows how much space they take. Only files that share their size with another are read, so the search is quick.

*Delete duplicates* moves the copies to the bin, keeping the one with the oldest file date of each set (if two have the same date, one of them). A progress bar shows how far it has got; closing or reloading the page doesn't stop it. Each copy is checked just before it goes: it, and the file that is kept, must still be there and unchanged.

## Settings: photo folders and the library

Settings is the gear at the top right.

**Photo folders**

- *Add folder…* opens the system folder picker in the desktop app. In the browser it lists the folders of the computer running Imadive: open one, or type or paste a path, then *Add this folder*.
- A folder inside one already in the gallery isn't needed and is refused. Adding a folder that contains others replaces them.
- *Remove* takes a folder's photos out of the gallery (the files stay on disk), except photos another folder still includes. Its row shows the progress. A scan still running stops first, and continues with the other folders afterwards. One folder is removed at a time.
- Folders given on the command line are marked *command line* and can't be removed here.
- A folder that can't be reached (an unplugged drive) shows as *not available*, and its photos stay in the gallery.

**The library**

- The library status, with progress while indexing or grouping faces.
- *Rescan* picks up new, changed or deleted photos without restarting.
- Files that could not be read, with the reason. They are skipped until the file changes, or until you click *Try again*.
- Photos removed from the gallery, with *Show again*.
- *Regroup all faces* groups every face from scratch (named people are kept).
- *About*: the version, the license and the project's page.

While indexing, the first time, the Photos tab shows the progress until the first photos are in. After that a small card at the bottom right shows it on every tab: *Show N new* brings in the photos indexed since, *Details* opens Settings, *Hide* hides it until the next scan.

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
- Photos are tracked by path. Moving or renaming the photo folder makes them look new, so they are indexed again. Named people are matched to the re-indexed faces automatically.
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
