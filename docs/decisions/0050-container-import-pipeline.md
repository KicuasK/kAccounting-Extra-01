# ADR 0050: bounded single-file recipient import

Status: implemented, 2026-10-22.

Use a bounded streaming transport extractor followed by the full existing
recipient importer, rather than treating container framing/hash checks as
permission to publish a book. The destination must be new. Keep active-book
selection unchanged, matching the approved explicit-switch operation.

The worker owns its input and encrypted staging across caller cancellation.
On fully verified success, clear only the internal encrypted stage using live
recorded directory and file identities plus an exact entry list. Preserve original
Downloads/external-media packages. Retain failed/interrupted stages for inspection;
never adopt cleanup authority based on a filename. Cleanup warnings preserve a
successful published outcome. This follows the subsequent explicit user decision.

User decision: Safari exports first download to Downloads and may then be moved
manually to APFS or exFAT external media. Directory-package compatibility remains
at the lower-level import API.

Tests cover real encrypted package/attachment import, bounded reads, outer and
inner tampering, incomplete bodies, wrong recipients, destination conflicts,
capacity rejection and cancellation while a blocking read is active. Only
temporary books and in-memory keys are used.
