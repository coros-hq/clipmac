## What and why

<!-- What does this change, and what problem does it solve? Link an issue if there is one. -->

## How I tested it

<!-- e.g. "ran cargo tauri dev and copied a JWT", "added a test for …" -->

## Checklist

- [ ] `cargo fmt --check`, `cargo check --locked` and `cargo test --locked` pass in `src-tauri/`
- [ ] JS files pass `node --check` (checked one at a time)
- [ ] Added or updated a test where it makes sense (especially for detection changes)
- [ ] No new network access, telemetry or unneeded dependencies
- [ ] If I touched how hidden (secret) clips are shown, copied, exported or dragged, I tested that they stay hidden
- [ ] Docs updated if behaviour changed (README / CONTRIBUTING)
