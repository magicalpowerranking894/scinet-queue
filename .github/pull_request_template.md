## Summary


## Tests

```sh
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --locked
cargo package --locked # when changing crate metadata or release packaging
```

## Notes

Keep the change focused. Avoid unrelated formatting and dependency changes.

- [ ] This change does not expose account data, browser profiles, cookies, tokens, downloaded papers, or other private user data.
