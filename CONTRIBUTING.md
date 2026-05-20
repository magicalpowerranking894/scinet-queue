# Contributing

`scinet-queue` is a small Rust CLI. Keep changes narrow, explicit, and easy to
review.

## Setup

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
```

## Pull Requests

- Keep pull requests focused.
- Explain the behavior change.
- Add tests for parser, queue, state, and network edge behavior.
- Avoid unrelated formatting or refactors.

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
