# Running Imadive with Docker

Each release is also published as a container image for x86-64 and ARM64 (a Raspberry Pi 4 or 5 with a 64-bit system, an ARM NAS):

```
ghcr.io/javiespinar/imadive:latest       the latest release
ghcr.io/javiespinar/imadive:0.3          the latest 0.3.x
ghcr.io/javiespinar/imadive:0.3.3        exactly that version
```

It is the same server as the [Linux server install](server.md), with face recognition included, in a minimal image (no shell or package manager) that runs as an unprivileged user.

**Before you start**: the gallery has no login. Anyone who can reach it can see, rotate and delete your photos. Run it only on a network you trust, and never forward its port on your router. See [Privacy and security](../README.md#privacy-and-security).

## Quick start

```sh
docker run -d --name imadive --restart unless-stopped \
  -p 7878:7878 \
  -v imadive-data:/data \
  -v /srv/photos:/photos:ro \
  ghcr.io/javiespinar/imadive:latest
```

Then open `http://<server address>:7878`. The first scan indexes the photos and finds the faces; the gallery fills as it goes.

- **`/data`** holds the index and thumbnails (here in a Docker volume called `imadive-data`). Keep it: without it everything is indexed again, and names given to people are lost.
- **`/photos`** is the photo folder. `:ro` mounts it read-only: the gallery works, but rotating and deleting fail with an error. Leave `:ro` out to allow them (see [Permissions](#permissions)).

## Docker Compose

```yaml
services:
  imadive:
    image: ghcr.io/javiespinar/imadive:latest
    restart: unless-stopped
    ports:
      - "7878:7878"
    volumes:
      - ./imadive-data:/data
      - /srv/photos:/photos
    # The user that owns the photos and ./imadive-data (see `id` on the host).
    user: "1000:1000"
    environment:
      # Names you open the gallery by, besides IP addresses (see below).
      IMADIVE_ALLOW_HOST: "nas.local"
```

`docker compose up -d` starts it; `docker compose pull && docker compose up -d` updates it. The index carries over.

## Options

**Several photo folders**: mount each one, and list them as the command (the options before them stay as they are):

```sh
docker run ... -v /srv/photos:/photos/main -v /mnt/nas/phone:/photos/phone \
  ghcr.io/javiespinar/imadive:latest /photos/main /photos/phone
```

Folders can also be added in the gallery's Settings, from any folder mounted in the container.

**Opening it by a name**: the gallery only accepts IP addresses and its own name in the address bar (a protection against DNS rebinding), and inside a container it can't know the host's name. If you open it as `http://nas.local:7878`, set `IMADIVE_ALLOW_HOST=nas.local` (several names separated by commas).

**No face recognition** (a slow machine, or to index faster): add `--no-faces` as the first word of the command, before the folders: `... imadive:latest --no-faces /photos`.

**Another port**: change the first number, `-p 8080:7878`.

**Uploading from the browser** (the *Upload* tab) writes into `imaDive-uploads` inside the photo folder: mount it without `:ro`, and run the container as a user that can write there (see [Permissions](#permissions)).

## Permissions

The container runs as user 65532 by default. That user can read photos that everyone may read (the usual case), and write to the `imadive-data` volume.

To rotate and delete photos, it must be able to write them: run it as the user that owns them, with `--user 1000:1000` (or `user:` in Compose; `id` on the host shows the numbers). The data folder must then be writable by that user too: use a folder you own (`-v ./imadive-data:/data`) rather than a new Docker volume.

## Updating

```sh
docker pull ghcr.io/javiespinar/imadive:latest
docker rm -f imadive
docker run ...   # the same command as before
```

The index in `/data` carries over; nothing is indexed again. The release notes say when an update needs anything else.

## Checking the image

Each image carries a signed record of the GitHub build it came from:

```sh
gh attestation verify oci://ghcr.io/javiespinar/imadive:latest --repo JaviEspinar/totufoto
```

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| "unexpected Host header" | You opened it by a name: add it to `IMADIVE_ALLOW_HOST`, or use the IP address. |
| No photos, and "permission denied" in `docker logs imadive` | The container's user can't read the folder: run it with `--user` as the owner of the photos. |
| "readonly database" or "permission denied" for `/data` | The data folder isn't writable by the container's user: see [Permissions](#permissions). |
| Rotating or deleting fails | The photos are mounted read-only (`:ro`), or the container's user can't write them. |
| `exec format error` | An ARM image on an x86 machine, or the other way round: pull without `--platform` and Docker picks the right one. 32-bit ARM isn't supported. |
