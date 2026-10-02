# Third-party components

Imadive's own code is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). The release downloads also contain, or the build fetches, the following components under their own terms. These terms apply on their own, whatever license you have for Imadive's code.

## Face recognition models (desktop app; fetched by `scripts/fetch-models.*`)

The InsightFace `buffalo_s` models (`det_500m.onnx`, an SCRFD face detector, and `w600k_mbf.onnx`, an ArcFace face recognition model), from <https://github.com/deepinsight/insightface>. InsightFace releases its pretrained models **for non-commercial research purposes only**. They are not stored in this repository; `scripts/fetch-models.sh` downloads them, and the desktop app embeds them.

For commercial use, build Imadive with models whose license allows it: any SCRFD-style detector with 5 landmarks and a 112x112 ArcFace-style embedder works (see `src/faces.rs`).

## ONNX Runtime (desktop app; fetched by `scripts/fetch-onnxruntime.*`)

Microsoft's [ONNX Runtime](https://github.com/microsoft/onnxruntime), which runs the face models. MIT License, Copyright (c) Microsoft Corporation. The full license text is downloaded next to the library as `onnxruntime/LICENSE`.

## Visual C++ runtime (Windows desktop app)

The Windows app includes the Microsoft Visual C++ runtime DLLs that ONNX Runtime needs (`msvcp140.dll`, `msvcp140_1.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`), redistributed under the Microsoft Visual Studio redistribution terms.

## GeoNames city data

Places are found with the [`reverse_geocoder`](https://crates.io/crates/reverse_geocoder) crate, which includes a list of world cities derived from [GeoNames](https://www.geonames.org) (cities with more than 1000 inhabitants). GeoNames data is licensed under the [Creative Commons Attribution 4.0 License](https://creativecommons.org/licenses/by/4.0/). Imadive uses it unchanged, as shipped by the crate.

## Rust crates

Imadive is built from open-source Rust crates, listed with their versions in `Cargo.lock`. Their licenses are permissive (mostly MIT and Apache-2.0, also BSD, ISC, Zlib, Unicode-3.0 and Unlicense), except for a few crates used only by the desktop app through Tauri (`cssparser`, `cssparser-macros`, `selectors`, `dtoa-short`, `option-ext`), which are under the Mozilla Public License 2.0; their unmodified source code is available from <https://crates.io>.

To list every crate with its license: `cargo tree --format "{p} {l}" --prefix none | sort -u`.
