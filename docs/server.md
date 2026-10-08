# Running Imadive on a home server

How to run the command-line app on a Linux machine that is always on (a NAS, a mini PC, a Raspberry Pi 4 or 5), started by systemd, so the gallery is available to the browsers of your home network.

**Before you start**: the gallery has no login. Anyone who can reach it can see, rotate and delete your photos, and add any folder of the server to the gallery. Run it only on a network you trust, and never forward its port on your router. See [Privacy and security](../README.md#privacy-and-security).

## Quick install

On Debian 12, Ubuntu 22.04, Raspberry Pi OS (64-bit) or newer, on an x86-64 or ARM64 machine (a Raspberry Pi 4 or 5 included):

```sh
curl -fsSL https://raw.githubusercontent.com/JaviEspinar/totufoto/main/scripts/install.sh | sh
```

It downloads the latest release's server build for the machine, checks it against the release's checksums, and starts **`imadive setup`**, a wizard in the terminal. Every question has an answer ready (press Enter to keep it):

- the user it runs as (yours, or a new system user `imadive`), which must be able to read the photos;
- the photo folders (or none, to add them later in the gallery's Settings);
- where the index goes, who can open it (your network or only this computer) and the port;
- face recognition on or off, and, if you want them, advanced options: other names the gallery is opened by (`--allow-host`), how alike faces must be, and the priority of indexing.

Before installing it shows a summary, and lets you **see or edit the systemd service file** it is about to write. It then installs the program in `/opt/imadive`, the service in `/etc/systemd/system/imadive.service`, starts it, waits for the gallery to answer, and shows its address.

Run it again to **update** (the same `curl` line), **change the settings** or **uninstall** (`sudo imadive setup`). An update keeps the settings, and a service file you edited. Uninstalling never touches the photos, and asks before deleting the index.

Without questions, for scripts: add `--yes` and the options you want (`sudo imadive setup --help` lists them):

```sh
curl -fsSL https://raw.githubusercontent.com/JaviEspinar/totufoto/main/scripts/install.sh | sh -s -- --yes --folder /srv/photos
```

**With Docker** instead: see [Running Imadive with Docker](docker.md).

The rest of this guide does the same by hand, building the program from source: for other systems, or to see every step.

## By hand

The examples use the user `ana`, the code in `/home/ana/imadive`, the index in `/home/ana/imadive-data` and photos in `/srv/photos`. Change them to yours.

## 1. Build it

As the user that will run the gallery (not root):

```sh
sudo apt install build-essential git curl        # Debian, Ubuntu, Raspberry Pi OS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env

git clone https://github.com/JaviEspinar/totufoto.git ~/imadive
cd ~/imadive
scripts/fetch-models.sh
scripts/fetch-onnxruntime.sh
cargo build --release
```

The first build takes 10 to 15 minutes on a small machine. The program is `~/imadive/target/release/imadive`.

Try it once by hand, then stop it with `Ctrl+C`:

```sh
./target/release/imadive --host 0.0.0.0 --data ~/imadive-data /srv/photos
```

The log says whether face recognition is on. Open `http://<server address>:7878` from another computer (find the address with `hostname -I`).

## 2. The service

Create `/etc/systemd/system/imadive.service` (for example with `sudo nano`):

```ini
[Unit]
Description=Imadive photo gallery
# Waits for the network, and for network drives when the photos are on one.
After=network-online.target remote-fs.target
Wants=network-online.target

[Service]
User=ana
# The models and ONNX Runtime are found in ./models and ./onnxruntime from here.
WorkingDirectory=/home/ana/imadive
ExecStart=/home/ana/imadive/target/release/imadive --host 0.0.0.0 --port 7878 --data /home/ana/imadive-data /srv/photos
Restart=on-failure
RestartSec=5
# Indexing uses every core; this keeps the server responsive for everything else.
Nice=10
IOSchedulingClass=idle
# Small protections that don't get in the way: no privilege changes, its own /tmp,
# /usr, /boot and /etc read-only.
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full

[Install]
WantedBy=multi-user.target
```

- **Several photo folders**: list them at the end of `ExecStart`, or leave them out and add folders in the gallery's Settings instead. Folders on the command line can only be removed by changing the service.
- **A name instead of the IP address**: if you open the gallery as `http://photos.home:7878` (a name set on your router, say), add `--allow-host photos.home`. The server's own host name works without it.
- **No face recognition** (a slow machine, or to index faster): add `--no-faces`.

Start it, and start it at every boot:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now imadive
systemctl status imadive
```

`active (running)` means it works. The log:

```sh
journalctl -u imadive -f          # follow it; Ctrl+C to stop following
journalctl -u imadive -b          # everything since the last boot
```

## 3. Updating

```sh
cd ~/imadive
git pull
cargo build --release
sudo systemctl restart imadive
```

The index carries over; nothing is indexed again. The release notes say when an update needs anything else.

**Coming from Totufoto**: the program is now `target/release/imadive`. If your service still says `.../target/release/totufoto`, the old file stays there and keeps running the old version, or the service fails with `status=203/EXEC` once that file is gone. Change `ExecStart` in the service file to `imadive`, then `sudo systemctl daemon-reload` and restart. Your existing `totufoto-data` folder is used as it is when you pass it with `--data`, or when no `imadive-data` exists in the working directory.

## 4. Optional: only your network

If the server has a firewall, allow the port only from your home network (with `ufw`, and your network's range):

```sh
sudo ufw allow from 192.168.0.0/24 to any port 7878 proto tcp
```

## 5. Optional: photos read-only

The gallery only changes photo files when someone rotates or deletes a photo. To rule that out, run the service as a user that can read the photos but not change them: rotating and deleting then fail with an error, and everything else works. The data folder must stay writable.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `status=203/EXEC` in `systemctl status` | `ExecStart` points to a file that doesn't exist (or isn't executable). Check the path with `ls -l`. |
| `status=200/CHDIR` | `WorkingDirectory` doesn't exist. |
| The page doesn't load from another computer | Is `--host 0.0.0.0` in `ExecStart`? Is the port open in the firewall? Does `curl http://127.0.0.1:7878/api/status` on the server answer? |
| "unexpected Host header" | You opened it through a name the server doesn't know: add `--allow-host that-name`, or use the IP address. |
| "face recognition disabled: ... not found" in the log | `models` or `onnxruntime` is missing in `WorkingDirectory`: run the two fetch scripts there. |
| Photos on a drive or network drive are missing after a reboot | The drive wasn't mounted when the gallery started (or isn't yet). The gallery starts anyway, shows the folder as *not available* in Settings and keeps its photos; once the drive is mounted, *Rescan* in Settings finds them. Mount it at every boot with a line in `/etc/fstab` (`nofail` keeps the boot going without it); `After=remote-fs.target` makes the gallery wait for network drives listed there. |
| `Error: folder /mnt/... No such file or directory`, and the service restarting over and over | Versions before 0.3.3 stop when a command-line folder isn't there: mount the drive (or fix the path in `ExecStart`), and the service starts by itself within seconds. |
| A folder's name looks different after mounting the drive again (`Imágenes` instead of `Imagenes`, say) | The drive was mounted another way (another driver or options). Mount it as before (see `/etc/fstab`), or rename the folder back to the name in `ExecStart`: with another name, every photo would look new and be indexed again. |
| Changes to the service file do nothing | Run `sudo systemctl daemon-reload` before restarting. |
