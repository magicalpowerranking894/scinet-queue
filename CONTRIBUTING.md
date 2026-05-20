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

## Pull Requests

- Keep pull requests focused.
- Explain the behavior change.
- Add tests for parser, queue, state, and network edge behavior.
- Avoid unrelated formatting or refactors.
- Use pull requests for code changes to `main`.
- Let the full CI matrix pass before merging.
- Use draft pull requests or temporary branches for platform-specific CI work.

## Project Layout

- `src/main.rs`: binary entrypoint only.
- `src/app.rs`: command dispatch and workflow glue.
- `src/args.rs`: handwritten argument parsing.
- `src/output.rs`: text and JSON output shapes.
- `src/browser.rs`: browser discovery, profile paths, and process launch.
- `src/cdp.rs`: generic Chrome DevTools Protocol transport.
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
