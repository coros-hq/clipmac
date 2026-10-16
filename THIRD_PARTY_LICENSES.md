# Third-party licenses

ClipMac itself is MIT-licensed (see [LICENSE](LICENSE)). It includes and depends on the following third-party software.

## Bundled in this repository

These files are copied into `src/vendor/` so the app works offline. Their license texts are kept next to them.

| Library | Version | License | Files |
| --- | --- | --- | --- |
| [highlight.js](https://highlightjs.org) | 11.9.0 | BSD-3-Clause | `src/vendor/highlight.min.js`, license: [`src/vendor/LICENSE.highlight.js`](src/vendor/LICENSE.highlight.js) |
| [Prettier](https://prettier.io) | 3.3.3 | MIT (plus the licenses of the code it bundles) | `src/vendor/prettier/*.js`, license: [`src/vendor/prettier/LICENSE`](src/vendor/prettier/LICENSE) |

## Rust dependencies

The app is built from crates.io dependencies (Tauri, arboard, `drag`, `objc2` and friends), resolved in `src-tauri/Cargo.lock`. At the time of writing, every one of them is under a permissive license (MIT, Apache-2.0, BSD, Zlib, Unlicense, Unicode-3.0), apart from a handful under MPL-2.0, which are used unmodified.

To list them yourself:

```sh
cd src-tauri
cargo metadata --format-version 1 --locked \
  | python3 -c "import json,sys; [print(p['name'], p['version'], p['license']) for p in json.load(sys.stdin)['packages']]"
```

## macOS frameworks

Vision (text recognition), AppKit, and the system `screencapture` tool are part of macOS and are not redistributed.
