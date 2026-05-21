# Sci-Net Behavior Notes

These notes describe behavior observed while building `scinet-queue`. They are
not a Sci-Net API contract.

## DOI Lookup

`snq check <doi>` posts the DOI from the managed browser session and prints the
Sci-Net response as JSON.

Observed outcomes:

- Sci-Net may expose an open-access option.
- Sci-Net may expose an existing availability option.
- Sci-Net may allow a token-backed request when no paper is available.

## Requests

`snq request <doi> --reward <n>` creates a request from the managed session.
The local queue is marked `requested` after a successful request call.

If Sci-Net rejects a request but the DOI's request page already exists and is
visible in the managed session, `snq` treats that as an existing remote request
and syncs the local queue instead of leaving it queued.

Observed request page states:

- `pending`: no visible solver and no PDF link.
- `working`: a member is working on the request.
- `pdf`: a PDF link is visible on the request page.
- `logged-out`: the session is no longer authenticated.

## Fetch And Approval

`snq watch` checks requested and working queue entries. It skips queued,
fetched, and approved entries so completed or not-yet-requested local state does
not start a browser session.

`snq fetch` downloads the first detected PDF link with the managed session and
marks the queue entry `fetched` after validating the file header. With
`--wait`, batch fetches keep polling until every targeted DOI has been fetched
or Sci-Net reports another availability path.
When no request-page PDF is visible, `fetch` also checks Sci-Net search
availability and reports `open-access` and `sci-hub` hints where Sci-Net exposes
them. It does not independently search or download from publishers,
repositories, or Sci-Hub.

`snq approve` is local review state. It requires a fetched queue entry unless
`--force` is passed.
