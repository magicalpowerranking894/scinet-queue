# Contributing

`scinet-queue` is a small Rust CLI. Keep changes narrow, explicit, and easy to
review.

## Local Checks

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

Run the package check before a release or when changing crate metadata:

```sh
cargo package --locked
```

## Release Process

Releases are tagged from `main` after the full CI matrix passes.

1. Update `Cargo.toml` and `CHANGELOG.md` for the new version.
2. Run the local release checks:

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo package --locked
```

3. Create and push a version tag:

```sh
git tag -s v0.1.1 -m "v0.1.1"
git push origin v0.1.1
```

4. Wait for the release workflow to upload archives for:

```text
x86_64-unknown-linux-gnu
x86_64-apple-darwin
aarch64-apple-darwin
x86_64-pc-windows-msvc
```

5. Check the release assets and `SHA256SUMS` before publishing the draft
   release.

## Pull Requests

- Keep pull requests focused.
- Explain the behavior change.
- Add tests for parser, queue, state, and network edge behavior.
- Avoid unrelated formatting or refactors.
- External code contributions should use pull requests.
- Let the full CI matrix pass before merging.
- Use draft pull requests or temporary branches for platform-specific CI work.

## Project Layout

- `src/main.rs`: binary entrypoint only.
- `src/app.rs`: command dispatch and workflow glue.
- `src/args.rs`: handwritten argument parsing.
- `src/output.rs`: text and JSON output shapes.
- `src/browser.rs`: browser-engine discovery, profile paths, and process launch.
- `src/page.rs`: protocol-neutral page/session wrapper.
- `src/cdp.rs`: Chrome DevTools Protocol transport for Chromium-compatible browsers.
- `src/bidi.rs`: WebDriver BiDi transport for Firefox/Gecko-based browsers.
- `src/scinet.rs`: Sci-Net session, request, view, and remote-state behavior.
- `src/papers.rs`: DOI extraction, PDF naming, fetch validation.
- `src/queue.rs`: workspace-local JSONL queue state.

## Dependencies

Add dependencies slowly.

A dependency should remove protocol risk, remove cross-platform filesystem risk,
or replace code that would be worse to maintain in-tree.

Avoid Selenium-style stacks and bundled browsers unless the project has a clear
need for that weight.

## Commits

Use Conventional Commits:

```text
feat: add queue storage
fix: handle missing browser binary
docs: document login flow
test: cover DOI normalization
```
