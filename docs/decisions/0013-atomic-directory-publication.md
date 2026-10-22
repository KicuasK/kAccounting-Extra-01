# ADR 0013: Atomic no-replace directory publication

Status: accepted engineering decision and implemented primitive

## Decision

A fully prepared private `.kagextra` directory is made visible by one atomic
exclusive rename. Publication must fail if any file, directory, or symlink
already has the destination name. The implementation uses the operating
system's no-replace rename through the safe `renamore` wrapper:

- `renamex_np(..., RENAME_EXCL)` on supported Apple systems;
- `renameat2(..., RENAME_NOREPLACE)` on supported Linux systems used for
  development or CI.

There is no check-then-ordinary-rename fallback. If the operating system or
destination file system cannot guarantee atomic no-replace behavior,
publication fails closed and leaves the private source directory in place.
The backend continues to forbid local `unsafe` Rust code.

The primitive accepts only a real, non-symlink source directory and a real,
non-symlink destination parent. A successful call moves the source directory
to the destination name. A failed call never removes the source and never
changes an existing destination.

## Rationale

An ordinary existence check followed by `rename` has a race: another process
can create the destination after the check, and an ordinary rename may replace
it. Creating the destination directory first avoids replacement but exposes an
incomplete package. An exclusive atomic rename makes exactly one complete
directory visible and gives concurrent publishers exactly one winner.

## Consequences

The package builder must create private work beside the destination, finish all
copies and validation there, and perform the required synchronization before
calling this primitive. ADR 0016 defines that synchronization boundary and the
post-publication verification required before reporting success.

Removable file systems that do not support the required exclusive rename will
be rejected unless a later user-approved workflow provides an equally safe
alternative.
