# Test photos

Small photos used by the desktop app's self-test in CI and for trying the gallery out
(`cargo run --release -- fixtures/photos --data /tmp/imadive-dev`).

| File | What it is | Source and license |
|---|---|---|
| `photos/portrait.jpg` | A face (for face recognition), a capture date (2012-12-06) and a GPS position (Washington, D.C.) in its EXIF data | Official White House portrait of Barack Obama by Pete Souza, 2012, [from Wikimedia Commons](https://commons.wikimedia.org/wiki/File:President_Barack_Obama.jpg). A work of the United States federal government, so in the **public domain**. Scaled down to 640x799 and given the EXIF date and position above. |
| `photos/plain.jpg` | A plain blue picture with no EXIF data (dated by its file) | Made for Imadive; public domain. |

The Rust tests don't use these files: they generate their own photos (`src/testutil.rs`).
