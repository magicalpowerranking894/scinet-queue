Canonical scinet-queue release.

Prebuilt `snq` archives are attached for Linux, macOS, and Windows, with
`SHA256SUMS` for verification.

Highlights in the current build:

- Managed browser login and headless authenticated Sci-Net session reuse.
- Workspace-local DOI queue with request, watch, fetch, and local approve
  workflows.
- JSON output for agent workflows, including Sci-Net availability links and
  token balance diagnostics.
- `balance` to print the visible Sci-Net token balance directly.
- `request --budget-check` to fail before posting requests when visible token
  balance is too low.
- Browser discovery for Chromium-compatible and Firefox/Gecko-based browsers
  without bundling a browser.
- macOS app-bundle browser paths resolve to the bundle's declared executable.
- Cross-platform CI/package validation for macOS, Linux, and Windows.
