# New Mac verification and first transfer increment

This records the initial setup/manifest increment. Later six-agent implementation
and verification supersede its remaining-work list; see
`BUILD_45MIN_2026-10-22.md`.

Date: 2026-10-22. Checkout: `~/Desktop/kAccounting-Extra-01`.
Starting branch: `main`; starting HEAD:
`afba3de66661876e77c2e502f3b47dd71e2838ea`. Starting worktree was clean.

Read AGENTS.md and all six requested handoff/planning/API/decision documents.
No real book or user Keychain entry was opened. No public service, Tailscale,
login agent, commit or push was enabled.

## Machine and dependencies

- Apple Silicon arm64, macOS 27.0 (26A5416b).
- Apple Command Line Tools: `/Library/Developer/CommandLineTools`.
- Installed Rust 1.99.1 with rustup, Clippy and rustfmt in the user directory;
  installer was instructed not to modify shell startup files.
- Node 24.22.1 / npm 11.19.1 are installed without overwriting files in
  `~/.local/share/kAccounting-Extra-01/toolchains/node-v24.22.1-darwin-arm64/bin`.
  Shell startup files and system Node were not changed.
- Python 3.9.6 and ripgrep 15.2.0 were already available.
- `cargo fetch --locked` and `npm --prefix frontend ci` completed. Lockfiles
  were preserved. Dependency retrieval and loopback access needed sandbox
  permission. No package upgrade was performed.

For this session's tools:

```sh
export PATH="$HOME/.local/share/kAccounting-Extra-01/toolchains/node-v24.22.1-darwin-arm64/bin:$HOME/.cargo/bin:$PATH"
```

## Baseline validation

- `./scripts/verify.sh`: passed, 472 Rust tests, 41 frontend tests;
  formatting, Clippy, documentation, TypeScript and Vite build passed.
- `cargo build --release --locked`: passed.
- `python3 -B scripts/test_launch_agent.py`: 3 passed.
- Release ignored maximum-size attachment test: 1 passed (250 MiB).
- Disposable `local_acceptance` server plus `http-acceptance.py --writes`:
  passed actual frontend/health/account/report reads, Host/Origin/Fetch Metadata
  rejection, drafts, atomic posting, amounts above JavaScript's safe range,
  balanced reports and verified encrypted backup.
- Server was stopped with Control-C; its disposable runtime was cleaned up.

## First implementation increment

- Added `backend/src/transfer_manifest.rs` and registered the module in
  `backend/src/lib.rs`.
- Added `backend/tests/transfer_manifest.rs` for strict authenticated catalog
  and complete database-reference reconciliation behavior.
- Recorded the contract and limitations in ADR 0036.
- Increment validation: complete verifier passed with 476 Rust tests and 41
  frontend tests. Clippy, formatting, documentation and frontend build passed;
  Release build passed again. Final accessor validation was additionally checked
  by all-target Clippy and the four manifest tests. `git diff --check` passed.
- Git: `main` remains at the starting HEAD; one modified module index and four
  new source/test/document files remain uncommitted. No lockfile changed.

This increment is the manifest contract only. Complete encrypted package
assembly/publication/import/recovery remains first priority. Next are bounded
attachment HTTP streaming, pre-migration backup, active-book/runtime integration,
the approved Catalina UI and actual Mac Safari acceptance. Existing unapproved
account classification questions remain unresolved and were not guessed.

Version 01 is not complete. A successful HTTP script is not Safari end-to-end
acceptance, and in-memory fixture keys do not validate real Keychain behavior.
