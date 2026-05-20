# scinet-queue

`scinet-queue` is a small command-line tool for managing Sci-Net paper
requests.

The binary is `snq`.

## Status

Early development. The local queue, Chromium session probe, Sci-Net search,
request, watch, fetch, and approve commands are supported.

## Install

```sh
cargo install --path .
```

## Usage

```sh
snq login
snq session
snq add 10.1287/mnsc.2024.05040
snq import papers.md
snq list
snq remove 10.1287/mnsc.2024.05040
snq check 10.1287/mnsc.2024.05040
snq request 10.1287/mnsc.2024.05040 --reward 1
snq request --all --reward 1
snq watch
snq fetch 10.1287/mnsc.2024.05040 --out papers
snq fetch --out papers
snq fetch --wait --poll 30 --out papers
snq approve 10.1287/mnsc.2024.05040
snq approve 10.1287/mnsc.2024.05040 --force
```

`snq add` accepts one or more DOIs. `snq import <path>` extracts DOIs from a
plain text or Markdown file. Use `snq import -` to read from stdin.

`snq` stores the queue in `.snq/queue.jsonl` in the current workspace.

## Workflow

```sh
snq login
snq import research-papers.md
snq request --all --reward 1
snq watch
snq fetch --wait --poll 30 --out papers
snq approve 10.1287/mnsc.2024.05040
```

Approval is always explicit.

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

`snq login` opens a tool-owned browser profile, waits until Sci-Net is logged
in, then closes the browser:

```sh
snq login
```

The user logs into Sci-Net once. Later commands reuse that profile headlessly
without taking over the user's normal browser. Use `snq login --no-wait` to
leave the login browser open.

`snq session` starts the managed profile headlessly and checks whether Sci-Net
loads with a logged-in session.

`snq check <doi>` calls Sci-Net's search endpoint from that browser session and
prints the JSON response.

`snq request <doi> --reward <n>` posts a Sci-Net request from the same session.
`snq request --all --reward <n>` requests queued entries.

`snq watch` checks queued requests for visible PDF uploads.

`snq fetch <doi> --out <dir>` downloads one available PDF into the output
directory and marks the queue entry as fetched. Without a DOI, `snq fetch`
checks queued, requested, and working entries. Use `--wait --poll <seconds>` to
keep checking until a PDF appears.

`snq approve <doi>` accepts a submitted solution and marks the queue entry as
approved. By default, the queue entry must already be fetched. Use `--force`
only when the PDF was reviewed outside `snq`.

This avoids decrypting cookies from Chrome, Firefox, Edge, Brave, Zen, or the
operating system keychain. Importing an existing browser profile is outside the
default flow.

Chromium-compatible browsers are supported first through Chrome DevTools
Protocol. Firefox support is planned through WebDriver BiDi.

Set `SCINET_QUEUE_BROWSER=/path/to/browser` to use a specific browser binary.

## Storage

Queue state is workspace-local by default. Account and browser profile state
lives under the user's platform state directory.

The queue is plain and inspectable:

```text
queue.jsonl
papers/
state/
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).
