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

Download a precompiled `snq` binary from the
[latest GitHub release](https://github.com/tivris/scinet-queue/releases/latest).

Release archives are published for:

- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

Unpack the archive for your platform, move `snq` or `snq.exe` somewhere on
your `PATH`, and check it:

```sh
snq --help
```

Each release includes `SHA256SUMS` for the uploaded archives.

You can also install from source with Rust 1.85 or newer:

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

Shells such as zsh treat some DOI characters, including parentheses, as glob
syntax. Quote DOIs with shell metacharacters:

```sh
snq check '10.1000/snq-example(1)'
```

Request one queued paper, or request all queued papers:

```sh
snq request '10.1000/snq-example(1)' --reward 1
snq request --all --reward 1
```

If `--reward` is omitted, `snq` uses `1`.
If Sci-Net reports that a request cannot be created but the DOI already has a
visible request page, `snq` syncs the local queue to that remote state.

Watch requested and working entries for visible PDF uploads:

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

When Sci-Net reports open-access or Sci-Hub availability but no request-page PDF
is downloadable yet, `snq fetch` reports that availability instead of treating
the DOI as simply pending. `snq` does not download from publisher pages,
open-access repositories, or Sci-Hub itself. In JSON output, resolved provider
URLs are included when Sci-Net exposes them.

Keep polling until every targeted DOI reaches an actionable outcome: a
downloadable request-page PDF, or a Sci-Net availability hint such as
open-access or Sci-Hub:

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
snq browsers --json
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
    "availability": ["open-access", "sci-hub"],
    "availability_links": [
      {
        "source": "sci-hub",
        "url": "https://sci-hub.example/10.1000/snq-example"
      }
    ],
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
browser binary for the current workspace, run:

```sh
snq browsers --pick
```

For scripts or agents, set the path explicitly:

```sh
snq browsers --set /path/to/browser
```

The workspace browser preference is stored in `.snq/browser.json`. It is local
to the current workspace, checked every time it is used, and can be removed
with:

```sh
snq browsers --clear
```

Selection order is:

1. `SCINET_QUEUE_BROWSER`, when set.
2. `.snq/browser.json`, when present and valid.
3. The first discovered compatible browser.

If more than one compatible browser is available and no preference exists,
interactive login and authenticated commands ask once and save the answer. JSON
and noninteractive paths do not prompt.

To inspect discovery and the active selection, run:

```sh
snq browsers
snq browsers --json
```

If `.snq/browser.json` is edited by hand and becomes invalid, authenticated
commands fail until the preference is cleared or replaced.

To override everything without writing `.snq/browser.json`, set:

```sh
SCINET_QUEUE_BROWSER=/path/to/browser
```

`snq login` opens the managed profile and waits until Sci-Net is logged in:

```sh
snq login
```

After login is detected, `snq` closes that browser cleanly so authenticated
commands can reuse the managed profile headlessly. Use `snq login --no-wait` to
leave the login browser open; finish login and close that browser before
running `snq session`, `snq fetch`, or other authenticated commands. The
printed PID is the launcher process; on macOS the long-lived app process may
have a different PID. While that login browser remains open, it owns the
managed profile and later authenticated commands may fail until it is closed.

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
  browser.json
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
