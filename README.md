# scinet-queue

[![ci](https://github.com/tivris/scinet-queue/actions/workflows/ci.yml/badge.svg)](https://github.com/tivris/scinet-queue/actions/workflows/ci.yml)

`scinet-queue` is a small command-line tool for managing Sci-Net paper
requests.

The binary is `snq`.

## Status

Early development. The local queue, browser session probe, Sci-Net search,
request, watch, fetch, local approve, JSON output, and doctor commands are
supported.

Authenticated commands currently require a Chromium-compatible browser with
Chrome DevTools Protocol support. macOS, Linux, and Windows builds are checked
in CI. Firefox/Gecko-based browser automation through WebDriver BiDi is
planned, but not implemented.

| Area | Status |
| --- | --- |
| macOS | CI checked |
| Linux | CI checked |
| Windows | CI checked |
| Chromium-compatible browsers (Chrome, Chromium, Brave, Edge) | Supported for authenticated commands through Chrome DevTools Protocol |
| Firefox/Gecko-based browsers (Firefox, Zen) | Planned through WebDriver BiDi |
| Existing browser cookie import | Not supported |
| Automatic token approval | Not supported |

## Install

Requires Rust 1.85 or newer.

From GitHub:

```sh
cargo install --locked --git https://github.com/tivris/scinet-queue --tag v0.1.0
```

From a local checkout:

```sh
cargo install --locked --path .
```

## Usage

```sh
snq login
snq session
snq add 10.1000/snq-example
snq import papers.md
snq list
snq list --json
snq remove 10.1000/snq-example
snq check 10.1000/snq-example
snq request 10.1000/snq-example --reward 1
snq request --all --reward 1
snq request --all --reward 1 --json
snq watch
snq watch --json
snq view 10.1000/snq-example
snq view 10.1000/snq-example --json
snq fetch 10.1000/snq-example --out papers
snq fetch --out papers
snq fetch --wait --poll 30 --out papers
snq approve 10.1000/snq-example
snq approve 10.1000/snq-example --force
snq doctor
snq doctor --json
```

Agent-facing JSON:

```sh
$ snq list --json
[
  {
    "doi": "10.1000/snq-example",
    "status": "working",
    "created_at": 1779283748,
    "updated_at": 1779285046
  }
]

$ snq watch --json
[
  {
    "doi": "10.1000/snq-example",
    "status": "working",
    "remote_state": "pdf"
  }
]
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
snq approve 10.1000/snq-example
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
loads with a logged-in session. Pass `--json` for structured output.

`snq doctor` checks browser discovery, profile path resolution, queue
readability, and Sci-Net session state. Pass `--json` for structured output.
It exits nonzero if any check fails. Redact local usernames, profile paths, and
paper paths before posting doctor output publicly.

`snq check <doi>` calls Sci-Net's search endpoint from that browser session and
prints the JSON response.

`snq request <doi> --reward <n>` posts a Sci-Net request from the same session.
`snq request --all --reward <n>` requests queued entries.
If `--reward` is omitted, `snq` uses `1`.
Pass `--json` for structured request results.

`snq watch` checks queued requests for visible PDF uploads.
Pass `--json` for structured queue and remote state.

`snq view <doi>` prints the remote request state, detected PDF links, and a
short text excerpt for one request. Pass `--json` for the full captured text
and PDF links.

`snq fetch <doi> --out <dir>` downloads one available PDF into the output
directory and marks the queue entry as fetched. Without a DOI, `snq fetch`
checks queued, requested, and working entries. Use `--wait --poll <seconds>` to
keep checking until a PDF appears.

`snq approve <doi>` marks a fetched paper as reviewed in the local queue. By
default, the queue entry must already be fetched. Use `--force` only when the
PDF was reviewed outside `snq`.

This avoids decrypting cookies from existing browser profiles or the operating
system keychain. Importing an existing browser profile is outside the default
flow.

Chromium-compatible browsers are supported first through Chrome DevTools
Protocol. Firefox/Gecko-based browser automation is planned through WebDriver
BiDi.

Set `SCINET_QUEUE_BROWSER=/path/to/browser` to use a specific browser binary.

## Known Limitations

- Authenticated commands use Chrome DevTools Protocol and currently require a
  Chromium-compatible browser.
- Firefox/Gecko-based browser automation through WebDriver BiDi is not
  implemented yet.
- Sci-Net is a third-party website. UI or endpoint changes can break request
  detection, PDF detection, or download behavior.
- `snq approve` is local review state only. It does not automatically release
  tokens or submit approval actions on Sci-Net.
- `snq` does not import cookies from an existing personal browser profile.

## Responsible Use

`scinet-queue` is an independent tool for automating a user's own Sci-Net
session for lawful educational and research workflows. It is not affiliated
with Sci-Net, any third-party paper index, repository, or publisher. It does
not bypass authentication, paywalls, access controls, or usage terms. Use it
only where you have the right to request, download, and store the papers
involved.

## Storage

Queue state is workspace-local by default. Account and browser profile state
lives under the user's platform state directory.

The queue is plain and inspectable:

```text
.snq/
  queue.jsonl
  queue.lock
papers/
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

Observed Sci-Net behavior is documented in [docs/behavior.md](docs/behavior.md).
