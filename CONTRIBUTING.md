# Contributing to ClipMac

Thanks for your interest! Bug reports, ideas and pull requests are all welcome.

## Reporting bugs and suggesting features

Open an issue and fill in the template. For bugs, the most useful details are your macOS version, whether you ran a release build or `cargo tauri dev`, and exact steps to reproduce.

**Security problems:** please don't open a public issue. See [SECURITY.md](SECURITY.md).

## Setting up

You need macOS, [Rust](https://rustup.rs) (stable, 1.85+) and the Xcode Command Line Tools.

```sh
cargo install tauri-cli --version "^2" --locked
cd src-tauri
cargo tauri dev
```

The frontend is plain HTML, CSS and JavaScript in `src/` — there is no Node, npm or bundler. Edit a file, restart `cargo tauri dev` and it's picked up. See the [README](README.md#project-layout) for what lives where.

The first screenshot needs the **Screen Recording** permission. In development macOS grants it to the terminal app you launched from.

## Before you open a pull request

```sh
cd src-tauri
cargo fmt --check
cargo check --locked
cargo test --locked
for f in ../src/*.js; do node --check "$f"; done   # `node --check` only looks at one file at a time
```

CI runs the tests and a universal build on every pull request, so a green check there is what we're aiming for.

### Guidelines

- **Keep PRs focused.** One change per pull request is much easier to review.
- **Match the surrounding code** — naming, comment density and style. The codebase prefers short comments that explain *why*, not *what*.
- **Add a test when you can.** Secret detection (`secrets.rs`), code detection and formatting are pure logic and easy to test. Please add a case for any false positive or false negative you fix.
- **Don't add dependencies lightly.** The app is deliberately small and offline: no network calls, no telemetry, no build step for the frontend. A new dependency (Rust or vendored JS) needs a clear reason and a permissive license. If you vendor a library, put its license text next to it and list it in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).
- **Privacy is a feature.** Anything that reads the clipboard or screen must stay on-device. Changes that weaken the secret-hiding behaviour (what gets copied, exported or dragged while a clip is hidden) need extra care and a test.
- **macOS only for now.** Cross-platform changes are welcome in principle, but please open an issue to discuss first.

### Commit messages

Short, imperative subject line ("Hide secrets in dragged images"), with a body explaining the reason if it isn't obvious.

## License

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE), the same as the project.
