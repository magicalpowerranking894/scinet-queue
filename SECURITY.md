# Security

`scinet-queue` is a local CLI. It does not run a hosted service or store a
remote database.

The main security risk is accidental disclosure of local user data while
debugging. Do not post account details, browser profiles, cookies, tokens,
session dumps, downloaded papers, or private reproduction data in public
issues, pull requests, or logs.

For a sensitive report, use GitHub private vulnerability reporting if it is
available. Otherwise, open a security contact issue that asks for a private
channel without including details.

`scinet-queue` stores queue state in the current workspace and browser session
state in a tool-owned browser profile. Treat both as local user data.

Only the latest release is supported.
