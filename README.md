# scinet-queue

[![ci](https://github.com/tivris/scinet-queue/actions/workflows/ci.yml/badge.svg)](https://github.com/tivris/scinet-queue/actions/workflows/ci.yml)

`scinet-queue` is a small command-line tool for managing Sci-Net paper
requests. The binary is `snq`.

The project is in early development. The local queue, browser session probe,
Sci-Net search, request, watch, fetch, local approve, JSON output, and doctor
commands are supported. macOS, Linux, and Windows builds are checked in CI.

Authenticated commands use a managed browser profile. Chromium-compatible
browsers are supported through Chrome DevTools Protocol. Firefox/Gecko-based
browsers are supported through WebDriver BiDi.

`snq` does not bundle a browser, import cookies from an existing browser
profile, or approve tokens automatically.

## Install

Requires Rust 1.85 or newer.

Install the released tag from GitHub:

```sh
cargo install --locked --git https://github.com/tivris/scinet-queue --tag v0.1.0
```

Install from a local checkout:

```sh
cargo install --locked --path .
```

## Quick Start

Log in once, import DOIs, request them, wait for PDFs, fetch them, then mark
reviewed papers as approved in the local queue:

```sh
snq login
snq import research-papers.md
snq request --all --reward 1
snq watch
snq fetch --wait --poll 30 --out papers
snq approve 10.1000/snq-example
```

Approval is always explicit. `snq approve` records local review state only; it
does not automatically release tokens or submit approval actions on Sci-Net.

## Queue Basics

Add one or more DOIs directly:

```sh
snq add 10.1000/snq-example
```

Import DOIs from a plain text or Markdown file:

```sh
snq import papers.md
```

Use `snq import -` to read from stdin.

List or remove queued entries:

```sh
snq list
snq remove 10.1000/snq-example
```

`snq` stores the queue in `.snq/queue.jsonl` in the current workspace.

## Sci-Net Commands

Check whether Sci-Net can find a DOI:

```sh
snq check 10.1000/snq-example
```

Request one queued paper, or request all queued papers:

```sh
snq request 10.1000/snq-example --reward 1
snq request --all --reward 1
```

If `--reward` is omitted, `snq` uses `1`.

Watch queued requests for visible PDF uploads:

```sh
snq watch
```

Inspect one remote request:

```sh
snq view 10.1000/snq-example
```

Download one available PDF, or fetch available PDFs for queued, requested, and
working entries:

```sh
snq fetch 10.1000/snq-example --out papers
snq fetch --out papers
```

Keep polling until a PDF appears:

```sh
snq fetch --wait --poll 30 --out papers
```

Mark a fetched paper as reviewed in the local queue:

```sh
snq approve 10.1000/snq-example
```

By default, the queue entry must already be fetched. Use `--force` only when
the PDF was reviewed outside `snq`.

## JSON Output

Agent-facing JSON is available for commands that need structured output:

```sh
snq session --json
snq list --json
snq request --all --reward 1 --json
snq watch --json
snq view 10.1000/snq-example --json
snq fetch --json
snq approve 10.1000/snq-example --json
snq doctor --json
```

`snq check <doi>` prints the Sci-Net response as JSON without a separate
`--json` flag.

Example `snq list --json` output:

```json
[
  {
    "doi": "10.1000/snq-example",
    "status": "working",
    "created_at": 1779283748,
    "updated_at": 1779285046
  }
]
```

Example `snq watch --json` output:

```json
[
  {
    "doi": "10.1000/snq-example",
    "status": "working",
    "remote_state": "pdf"
  }
]
```

Example `snq fetch --json` output:

```json
[
  {
    "doi": "10.1000/snq-example",
    "status": "no-pdf",
    "remote_state": "working",
    "path": null
  }
]
```

Example `snq approve --json` output:

```json
{
  "doi": "10.1000/snq-example",
  "status": "approved",
  "forced": false
}
```

## Browser Sessions

Authenticated Sci-Net commands run through a `snq`-managed browser profile.
The profile is separate from the user's normal browser profile.

The browser support model is engine/protocol oriented:

| Engine family | Protocol | Status |
| --- | --- | --- |
| Chromium-compatible | Chrome DevTools Protocol | Supported |
| Firefox/Gecko-based | WebDriver BiDi | Supported |

`snq` discovers a compatible browser on the system. To choose a specific
browser binary, set:

```sh
SCINET_QUEUE_BROWSER=/path/to/browser
```

`snq login` opens the managed profile and waits until Sci-Net is logged in:

```sh
snq login
```

After login, authenticated commands reuse that managed profile headlessly. Use
`snq login --no-wait` to leave the login browser open.

`snq session` starts the managed profile headlessly and checks whether Sci-Net
loads with a logged-in session:

```sh
snq session
snq session --json
```

`snq doctor` checks browser discovery, profile path resolution, queue
readability, and Sci-Net session state:

```sh
snq doctor
snq doctor --json
```

`snq doctor` exits nonzero if any check fails. Redact local usernames, profile
paths, and paper paths before posting doctor output publicly.

The login flow avoids decrypting cookies from existing browser profiles or the
operating system keychain. Importing an existing browser profile is outside the
default flow.

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

## Design

- Native CLI.
- Plain local state.
- Explicit commands.
- Sci-Net-specific behavior.
- No Selenium.
- No bundled browser.
- No background daemon by default.
- No token approval without an explicit user command.

## Known Limitations

- Authenticated commands require a supported browser engine: Chromium-compatible
  through Chrome DevTools Protocol or Firefox/Gecko-based through WebDriver
  BiDi.
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

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

Observed Sci-Net behavior is documented in [docs/behavior.md](docs/behavior.md).
