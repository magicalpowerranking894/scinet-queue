# Sci-Net Behavior Notes

These notes describe behavior observed while building `scinet-queue`. They are
not a Sci-Net API contract.

## DOI Lookup

`snq check <doi>` posts the DOI from the managed browser session and prints the
Sci-Net response as JSON.

Observed outcomes:

- Sci-Net may expose an open-access option.
- Sci-Net may expose an existing PDF or third-party paper index-backed option.
- Sci-Net may allow a token-backed request when no paper is available.

## Requests

`snq request <doi> --reward <n>` creates a request from the managed session.
The local queue is marked `requested` after a successful request call.

Observed request page states:

- `pending`: no visible solver and no PDF link.
- `working`: a member is working on the request.
- `pdf`: a PDF link is visible on the request page.
- `logged-out`: the session is no longer authenticated.

## Fetch And Approval

`snq fetch` downloads the first detected PDF link with the managed session and
marks the queue entry `fetched` after validating the file header.

`snq approve` is local review state. It requires a fetched queue entry unless
`--force` is passed.
