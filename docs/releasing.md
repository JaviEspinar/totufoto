# Releasing

A release is a `vX.Y.Z` tag on `main`. Pushing the tag makes `.github/workflows/desktop.yml` build the Windows and Linux apps, run their self-test, and publish a GitHub Release with the written notes, `SHA256SUMS` and a signed build provenance attestation.

## Version numbers

Before 1.0:

- **patch** (`0.1.11` → `0.1.12`): fixes and small features;
- **minor** (`0.1` → `0.2`): bigger features, API changes, or anything that changes the index or the data folder in a way an older version couldn't read.

An index from any earlier version must keep working: new columns go through the migrations in `src/db/mod.rs`, and the tests open an old schema.

There is one version for both packages, in `[workspace.package]` in `Cargo.toml`; the desktop app's `tauri.conf.json` takes it from there.

## Steps

1. **Check `main` is green**: the CI and, when the desktop app changed, the Desktop apps workflow. To build the desktop apps without releasing, run that workflow by hand (Actions → Desktop apps → Run workflow) and try the files it keeps for 14 days.
2. **Write the notes** in `release-notes/vX.Y.Z.md`, in the format below.
3. **Bump, commit and tag**:
   ```sh
   just release X.Y.Z        # or: scripts/release.sh X.Y.Z
   ```
   The script checks that you are on a clean `main`, that the notes exist, have a Downloads section and name this version's files, then sets the version, updates `Cargo.lock` and the README's download links (checked by `scripts/check-download-links.sh`, which CI and the release workflow run too), commits "Release X.Y.Z" and tags `vX.Y.Z`. It doesn't push.
4. **Publish**: `git push origin main vX.Y.Z`.
5. **Watch the Desktop apps run**. It fails before building anything if the tag doesn't match `Cargo.toml` or the notes are missing.
6. **Check the release page**: the five files (exe, AppImage, .deb, and the server for x86-64 and ARM64), `SHA256SUMS`, and the notes; and the image on `ghcr.io/javiespinar/imadive` with the version's tags.

The first time an image is published, GitHub makes its package private: on the repository's page, Packages → imadive → Package settings → Change visibility → Public, once.

If something goes wrong after pushing the tag, fix it on `main` and release the next patch version rather than moving the tag: people may already have downloaded the files.

## Release notes

Written for the people who use the app, not for developers: what they can do now, what was fixed, in their words. GitHub adds the list of commits below them.

```markdown
One or two sentences: what this version is about.

## Downloads

- **Windows 10/11**: `Imadive-X.Y.Z-windows-x64.exe`. Double-click it. It is not signed, so the first time Windows may show "Windows protected your PC": click **More info**, then **Run anyway**.
- **Debian, Ubuntu and Linux Mint** (Debian 12 or Ubuntu 22.04 and newer): `Imadive-X.Y.Z-linux-amd64.deb`. Install it with `sudo apt install ./Imadive-X.Y.Z-linux-amd64.deb`, then start Imadive from the applications menu.
- **Other Linux distributions (x86-64)**: `Imadive-X.Y.Z-linux-x86_64.AppImage`. Run `chmod +x Imadive-*.AppImage`, then start it. If it says FUSE is missing, install `libfuse2` or run it with `--appimage-extract-and-run`.
- **A Linux server** (x86-64, or ARM64 such as a Raspberry Pi 4 or 5): `Imadive-server-X.Y.Z-linux-x86_64.tar.gz` or `…-linux-aarch64.tar.gz`, installed by the one-line command in the [server guide](https://github.com/JaviEspinar/totufoto/blob/main/docs/server.md).

Your folders, people names and index carry over from earlier versions; nothing is indexed again.

Free for personal and other non-commercial use under the [PolyForm Noncommercial License 1.0.0](https://github.com/JaviEspinar/totufoto/blob/main/LICENSE). The face recognition models built into the app are for non-commercial use only (see [THIRD_PARTY.md](https://github.com/JaviEspinar/totufoto/blob/main/THIRD_PARTY.md)).

## What's new

- ...

## Fixes

- ...

## Known issues

- ... (leave the section out when there are none)
```

## Checking a download

Anyone can check that a file is the one GitHub Actions built from this repository:

```sh
sha256sum -c SHA256SUMS --ignore-missing             # the file is intact
gh attestation verify Imadive-X.Y.Z-linux-x86_64.AppImage --repo JaviEspinar/totufoto
```

The second command needs the GitHub CLI. It shows the workflow, commit and tag the file was built from.

## Signing

The executables aren't code-signed yet, so Windows SmartScreen warns on first start. The checksums and the attestation let people verify a download in the meantime.
