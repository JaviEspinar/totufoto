# Running Imadive on a home server

How to run the command-line app on a Linux machine that is always on (a NAS, a mini PC, a Raspberry Pi 4 or 5), started by systemd, so the gallery is available to the browsers of your home network.

**Before you start**: the gallery has no login. Anyone who can reach it can see, rotate and delete your photos, and add any folder of the server to the gallery. Run it only on a network you trust, and never forward its port on your router. See [Privacy and security](../README.md#privacy-and-security).

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
| Photos on a network drive are missing after a reboot | The drive was mounted after the gallery started. Imadive keeps the photos of a folder it can't reach, and finds them at the next scan (*Rescan* in Settings). `After=remote-fs.target` helps for drives in `/etc/fstab`. |
| Changes to the service file do nothing | Run `sudo systemctl daemon-reload` before restarting. |
