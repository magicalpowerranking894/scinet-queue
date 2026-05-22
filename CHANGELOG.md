# Changelog

## Unreleased

- Reorganized app, browser, and Sci-Net code into module folders for cleaner
  long-term ownership boundaries.
- Split browser discovery and preference handling from browser launch code.
- Split Sci-Net response parsing from page-session request code.
- Rejected non-executable browser paths before saving or launching browser
  preferences.
- Surfaced Sci-Net-provided open-access and Sci-Hub provider URLs in fetch JSON
  output.
- Documented local and remote queue state movement and clearer `fetch --wait`
  stop conditions.
- Synced local queue state when Sci-Net rejects a request for a DOI whose
  request page already exists remotely.
- Closed waited `snq login` browser sessions gracefully after login detection so
  fresh authentication state can be reused by later headless commands.
- Surfaced Sci-Net request error reasons when response bodies include them.
- Documented quoting DOI arguments that contain shell metacharacters such as
  parentheses.
- Clarified browser-exit and profile-in-use errors after `login --no-wait`.
- Added Sci-Net open-access and Sci-Hub availability hints to `fetch` output
  when no request-page PDF is downloadable.
- Clarified `login --no-wait` terminal guidance and Sci-Net-visible PDF fetch
  behavior.
- Fixed text output for mixed batch fetches so pending DOIs stay visible when
  another DOI downloads successfully.
- Made malformed `.snq/browser.json` preferences visible in `snq browsers` and
  authenticated command errors.
- Made `approve` errors distinguish missing, not-yet-fetched, and already
  approved entries.
- Fixed batch `fetch --wait` so one available PDF does not stop polling for
  the remaining targeted DOIs.
- Limited `snq watch` to requested and working entries so inactive queue state
  does not start a browser session.
- Aligned managed login launch flags across browser engines to avoid Chromium
  keychain prompts and Firefox remote handoff.
- Added a GitHub Actions release workflow for precompiled `snq` archives and
  `SHA256SUMS`.
- Documented release binary installation and the release process.

## 0.1.0 - 2026-05-20

- Added a workspace-local DOI queue.
- Added managed browser login and headless Sci-Net session reuse.
- Added Sci-Net `check`, `request`, `watch`, `view`, `fetch`, and local
  `approve` commands.
- Added batch DOI import and queued request/fetch workflows.
- Added explicit safeguards around local approval and managed browser profile
  reuse.
- Added `snq doctor` for browser, profile, queue, and session diagnostics.
- Added JSON output for session, list, request, watch, view, and doctor
  workflows.
- Added Sci-Net HTML fixtures for logged-out, pending, working, and solved
  request states.
- Added public-release documentation, security reporting guidance, and GitHub
  issue templates.
- Added queue locking, bounded CDP socket timeouts, package metadata, and
  public contribution safeguards.
- Hardened CLI edge cases around argument validation, DOI normalization/import,
  failed browser startup cleanup, request failures, and forced local approval.
- Hardened CDP startup retries, managed profile lock ownership, angle-wrapped
  DOI cleanup, and queue locking under parallel CLI bursts.
- Hardened queue and managed-profile locks with held OS file locks across
  supported platforms.
- Hardened direct fetch state, doctor exit status, request logical-error
  detection, and public fixtures.
