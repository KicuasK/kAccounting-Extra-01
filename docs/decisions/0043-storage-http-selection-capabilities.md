# ADR 0043 — Storage HTTP selection capabilities and owned jobs

Date: 2026-10-22. Status: backend implemented; native file-selection UI pending.

Storage operations accept process-local, random 128-bit capability identifiers,
never filesystem paths, private keys, overwrite flags or automatic book-switch
instructions. Only a trusted local/native adapter can register a typed selection
using `StorageOperations::register_selection`; there is deliberately no HTTP
registration endpoint. A genuine operating-system file-selection action must
precede registration. This is an engineering security boundary, not a new user
workflow decision.

Each selection is constrained to one role: existing book, public recipient
request, recipient package, local backup, new request file, new export package,
new book destination, or recovery parent. Relative paths, existing destination
paths, and symlink components are refused at registration. Execution retains
all existing audited command checks, destination no-replace publication,
package authentication, exact accounting invariants and runtime locks.
Registration is not authorization to bypass a command's private-storage checks.

Public recipient requests are parsed and retained immutably on registration.
The preview route exposes only the request ID and fingerprint. Export requires
both values to match the retained request; replacing its file after preview
cannot redirect an export to a different recipient.

A single shared process service admits one storage job at a time with no
unbounded queue. Accepted commands return HTTP 202 and a random operation ID.
Owned worker and supervisor tasks retain the concurrency permit, keys and book
ownership until completion, even if a requester disconnects. Worker failures
and panics become a sanitized failed status; paths and low-level key errors do
not cross HTTP. Statuses and selections each have a 256-entry bound. The trusted
adapter can revoke selections and acknowledge completed statuses. Destination
capabilities are single-use once admitted. Running jobs cannot be forgotten.
No HTTP cancellation endpoint is provided because dropping a request cannot
safely revoke an already publishing operation.

The optional active-book context owns `Arc<RuntimeBook>` and its runtime lock.
It exports and recovers using that already opened book rather than attempting
to acquire a second conflicting runtime lock. Other selected books continue to
use the explicit command layer. All imports and restores target new books;
neither changes or closes the active book. No pre-import live-book backup is
created for these operations because they do not modify the live book.

The parent API wrapper applies the existing exact loopback Host/Origin guard,
body-size limit and no-store response headers to commands, previews and status
reads. Every result count/byte length is a canonical decimal JSON string.
Operation IDs and selection capabilities last only for this process. After a
restart, the authenticated on-disk recovery mechanisms, not stale operation
IDs, determine recovery state. Recovery of newly staged exports scans the
private source book root; portable media is not the trusted staging area.

Tests use temporary books and mutex-backed in-memory keys only. They cover an
active-book transfer roundtrip; new-destination restore; source preservation;
refusal of replaced destination, symlink, wrong-type and unknown capabilities;
Host/Origin and strict JSON controls; immutable recipient confirmation; bounded
admission; response-independent job ownership; sanitized tamper failure; and
recovery counts without paths. Native picker, real Keychain prompts, physical
APFS/exFAT and Safari operation acceptance remain separate release gates.
