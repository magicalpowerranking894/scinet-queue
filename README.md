# scinet-queue

`scinet-queue` is a small command-line tool for managing Sci-Net paper
requests.

The binary is `snq`.

## Status

Early development. The local queue, Chromium session probe, Sci-Net search
check, and request command are supported.

## Install

```sh
cargo install --path .
```

## Usage

```sh
snq login
snq session
snq add 10.1287/mnsc.2024.05040
snq list
snq remove 10.1287/mnsc.2024.05040
snq check 10.1287/mnsc.2024.05040
snq request 10.1287/mnsc.2024.05040 --reward 1
```

`snq` stores the queue in `.snq/queue.jsonl` in the current workspace.

## Design

- Native CLI.
- Plain local state.
- Explicit commands.
- Sci-Net-specific behavior.
- No Selenium.
- No bundled browser.
- No background daemon by default.
- No token approval without an explicit user command.

## Browser Session

`snq login` opens a tool-owned browser profile:

```sh
snq login
```

The user logs into Sci-Net once. Later commands reuse that profile without
taking over the user's normal browser.

`snq session` starts the managed profile headlessly and checks whether Sci-Net
loads with a logged-in session.

`snq check <doi>` calls Sci-Net's search endpoint from that browser session and
prints the JSON response.

`snq request <doi> --reward <n>` posts a Sci-Net request from the same session.

This avoids decrypting cookies from Chrome, Firefox, Edge, Brave, Zen, or the
operating system keychain. Importing an existing browser profile is outside the
default flow.

Chromium-compatible browsers are supported first through Chrome DevTools
Protocol. Firefox support is planned through WebDriver BiDi.

Set `SCINET_QUEUE_BROWSER=/path/to/browser` to use a specific browser binary.

## Storage

Queue state is workspace-local by default. Account and browser profile state can
live under the user's platform data directory once `snq login` exists.

The queue is plain and inspectable:

```text
queue.jsonl
papers/
state/
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
