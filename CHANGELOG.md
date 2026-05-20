# Changelog

## 0.1.0 - 2026-05-20

- Added a workspace-local DOI queue.
- Added managed Chromium login and headless Sci-Net session reuse.
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
