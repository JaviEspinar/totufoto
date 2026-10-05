# Contributing to Imadive

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

- **A bug**: open an issue with the bug template. The version, the system and, for the desktop app, the self-test report save a lot of back and forth.
- **An idea**: open an issue first for anything bigger than a small fix, so we can agree on the approach before you spend time on it.
- **A security problem**: don't open an issue; see [SECURITY.md](SECURITY.md).

## Getting started

You need Rust 1.93 or newer (`rustup` installs it), and for the browser tests Node.js 24 or newer. [just](https://just.systems) is optional: every recipe in the `justfile` is a plain command you can also run yourself.

```sh
git clone https://github.com/JaviEspinar/totufoto.git imadive
cd imadive
just fetch     # face models and ONNX Runtime (optional for most work)
just dev       # the gallery on the sample photo, with a temporary index
```

`just dev ~/Pictures/some-folder` runs it on your own photos, still with a temporary index. Then open http://127.0.0.1:7878.

The first build takes a few minutes (10 to 15 on a slow machine). The desktop app takes longer, and on Linux needs the system libraries listed in the README.

[docs/architecture.md](docs/architecture.md) explains how the code fits together, and [docs/api.md](docs/api.md) the HTTP API.

## Before you send a pull request

```sh
just lint      # cargo fmt --check, clippy with warnings as errors, node --check
just test      # cargo test
just ui-test   # the browser tests (builds the release binary, installs Chromium once)
```

CI runs the same checks, the tests on Linux, macOS and Windows, a build with the minimum Rust version, and `cargo deny` on the dependencies.

- **Rust**: `cargo fmt` formats the code (`rustfmt.toml`); clippy must be clean.
- **Web UI**: plain HTML, CSS and JavaScript in `web/`, with no build step and no dependencies; keep it that way. The script is split by area into ES modules in `web/js/` (see [How it works](docs/architecture.md#the-page-web)). Match the style of the code around your change. For a change visible in the page, a browser test in `web/tests` helps, and a screenshot in the pull request helps more.
- **Tests**: a fix comes with a test that fails without it when that is practical. `src/testutil.rs` makes temporary libraries with real JPEG files and EXIF data, so most of the gallery can be tested without face models.
- **Anything that deletes or changes photo files** needs a test, and must only touch files inside the photo folders.
- **The index**: new columns go through the migrations in `src/db/mod.rs`, and an index made by an older version must keep working.
- **Commits**: small and focused, with a message that says what changes and why.

## Writing for people

Messages, labels and documentation are for people who just want to see their photos. Write short, plain sentences, say what happened and what to do next, and avoid technical words where an everyday one works.

## Releases

See [docs/releasing.md](docs/releasing.md).

## Licensing of contributions

Imadive is published under the [PolyForm Noncommercial License 1.0.0](LICENSE), and its maintainer also offers commercial licenses. So that both stay possible, by sending a contribution you agree that:

- you wrote it, or have the right to submit it;
- it is licensed under the same PolyForm Noncommercial License 1.0.0 as the project; and
- the maintainer ([waiting4timeout](https://github.com/waiting4timeout)) may also license it, as part of Imadive, under other terms, including commercial licenses and a future change of the project's license.

You keep the copyright of your contribution. If you can't agree to this, say so in the pull request before it is merged.

## Code of conduct

Everyone taking part is expected to follow the [code of conduct](CODE_OF_CONDUCT.md).
