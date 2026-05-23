# Sci-Net Behavior Notes

These notes describe behavior observed while building `scinet-queue`. They are
not a Sci-Net API contract.

## DOI Lookup

`snq check <doi>` posts the DOI from the managed browser session and prints the
Sci-Net response as JSON.

`snq url <doi>` only normalizes and encodes the DOI into its Sci-Net request
URL. It does not open a browser or call Sci-Net.

Observed outcomes:

- Sci-Net may expose open-access or Sci-Hub availability.
- Sci-Net may allow a token-backed request when no paper is available.

## Requests

`snq balance` reads the visible token balance from the logged-in managed
session. It fails if the session is logged out or the balance cannot be read.

`snq request <doi> --reward <n>` creates a request from the managed session.
The local queue is marked `requested` after a successful request call.

With `--budget-check`, `snq` reads the visible token balance from the logged-in
Sci-Net page before posting any request. If the balance is unavailable or lower
than `reward * targeted DOI count`, the command fails locally before calling
Sci-Net's request endpoint. The check is a guard, not a reservation; Sci-Net can
still reject a later request if the site state changes.

If Sci-Net rejects a request but the DOI's request page already exists and is
visible in the managed session, `snq` treats that as an existing remote request
and syncs the local queue instead of leaving it queued. The page must match the
target DOI; a redirect to a generic Sci-Net page is reported as `not-found` and
does not update the local queue.

Observed request page states:

- `pending`: no visible solver and no PDF link.
- `working`: a member is working on the request.
- `pdf`: a PDF link is visible on the request page.
- `not-found`: the page did not match the requested DOI.
- `logged-out`: the session is no longer authenticated.

Local queue states:

| State | Meaning | Typical movement |
| --- | --- | --- |
| `queued` | DOI is known locally but no request has been confirmed. | `add`, `import`, or direct `fetch <doi>` setup |
| `requested` | Sci-Net accepted or already has a visible pending request. | `request` or `fetch` sync from remote `pending` |
| `working` | Sci-Net reports someone is working on the request. | `request`, `watch`, or `fetch` sync from remote `working` |
| `fetched` | A request-page PDF was downloaded and validated locally. | `fetch` |
| `approved` | The local operator marked the fetched PDF as reviewed. | `approve` |

## Fetch And Approval

`snq watch` checks requested and working queue entries. It skips queued,
fetched, and approved entries so completed or not-yet-requested local state does
not start a browser session.

`snq fetch` downloads the first detected PDF link with the managed session and
marks the queue entry `fetched` after validating the file header. With
`--wait`, batch fetches keep polling until every targeted DOI has either been
fetched or reached another Sci-Net-visible availability path.
When no request-page PDF is visible, `fetch` also checks Sci-Net search
availability and reports `open-access` and `sci-hub` hints where Sci-Net exposes
them. JSON output also includes resolved provider URLs when Sci-Net exposes
them. It does not independently search or download from publishers,
repositories, or Sci-Hub.

`snq approve` is local review state. It requires a fetched queue entry unless
`--force` is passed.
