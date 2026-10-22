# macOS development runtime package

This is a **development runtime bundle**, not a `.app`, installer, Developer ID
signed/notarized release, or completed Version 01 UI. Packaging never executes
the application, opens a book, accesses Keychain, installs files, or starts a
service. Only build artifacts are copied; no book, backup, attachment store,
authentication state or private runtime configuration is added. Startup metadata
contains no real book paths or configured runtime identities. Executable build
metadata can still contain source paths; packaging does not scrub compiled bytes.

## Build and verify

Create a new current-user-owned private 0700 output parent outside the checkout.
The bundle destination itself must not exist. Supply absolute canonical paths
for the Release executable, built frontend and new bundle destination:

```sh
python3 scripts/package-macos-release.py \
  --binary /absolute/source/target/release/kaccounting-backend \
  --frontend /absolute/source/frontend/dist \
  --output /absolute/private-parent/kAccounting-Extra-01-development-runtime \
  --target macos-arm64 \
  --revision afba3de66661876e77c2e502f3b47dd71e2838ea
```

An independently built native helper can be included with `--native-helper`.
Both executable inputs must have a thin 64-bit Mach-O header matching the target;
this check does not establish signing or the safety of an arbitrary executable.
The only accepted frontend inputs are `index.html` and canonical Vite-style
`assets/*.js` / `assets/*.css` files. Other directories or files are refused.

The output receipt includes an external manifest SHA-256. Preserve that digest
in a separately trusted handoff record, then verify after copying:

```sh
python3 scripts/package-macos-release.py \
  --verify /absolute/private-parent/kAccounting-Extra-01-development-runtime \
  --expected-manifest-sha256 THE_TRUSTED_64_HEX_DIGEST
```

Verification checks exact file membership, all hashes and lengths, private modes,
architecture, startup metadata, completion marker and optional externally trusted
manifest digest. It never executes a packaged file. With no external digest it
can identify corruption, but cannot authenticate a maliciously rewritten bundle
whose manifest and completion marker were both replaced. This is a hash-based
development handoff, not a digital signature.

The declared revision is a supplied source-base identifier, explicitly labelled
`declared-base-not-build-attestation`. An uncommitted working tree can differ
from that revision. File hashes identify the actual packaged bytes; this tool
does not falsely attest that the executable was compiled from a clean commit.

## Layout, permissions and bounds

- `bin/kaccounting-backend`: copied Release binary, mode 0700.
- `bin/StoragePicker`: optional separately built native helper, mode 0700.
- `frontend/index.html`, `frontend/assets/*.js` and `*.css`: mode 0600.
- `manifest.json`, `complete.json`: mode 0600; all directories mode 0700.

Existing destinations are never adopted or overwritten. A new directory is
created exclusively; each file uses exclusive/no-follow creation. Failure
preserves its partial directory for inspection. A completion marker binds the
manifest, and the tool verifies the whole result before reporting success.
Incomplete directories must not be launched or treated as complete packages.
No recursive cleanup or automatic repair is performed.

Inputs and outputs reject symlinks, file hardlinks, unexpected owners, unsafe
write permissions, relative/parent paths and changed source identity during
reads. Output may not be inside the checkout or an ancestor of it. Files are
bounded at 256 MiB for the backend, 32 MiB for a helper/individual asset, 1 MiB
for the frontend entry, 130 runtime files and 512 MiB total. These are packaging
resource limits, not accounting/attachment policy. The current implementation
bounds memory by a single maximum-sized file; it does not claim zero-copy I/O.

Portable media that cannot preserve Unix permissions, such as exFAT, is not a
valid private runtime location. Transfer this code-only bundle in a container
that preserves metadata, then extract to a fresh private local macOS directory
and verify against the separately retained digest. Existing owners/modes may
need an explicitly scoped migration step on another Mac; this tool refuses
rather than silently changes them. Do not place financial data inside the bundle.

## Startup and remaining release work

Manifest startup metadata contains placeholders only: the `serve-chooser`
command needs a separately selected private book collection and the bundle's
frontend directory. Every launch requires explicit book selection. Existing
books still require their original device-key material or a verified recipient
encrypted transfer; a source/runtime bundle does not transfer Keychain secrets.
The optional helper is packaged as an artifact; this package tool does not open
its dialog or activate the native-host route automatically.

There is no automatic launcher, public service, Tailscale configuration, login
operation, LaunchAgent installation or macOS trust bypass. Runtime and book
selection integration, production signing/notarization, minimum-OS compatibility,
Safari interaction acceptance and the full UI remain separate release gates.
The current backend's read-only load-command inspection shows only system
Security/CoreFoundation/libiconv/libSystem dependencies; that observation does
not establish every supported macOS runtime version.

## Verified evidence — 2026-10-22

Ten synthetic fixture tests passed: deterministic manifests; unchanged source
paths omitted from metadata; no overwrite; corruption/missing marker/unknown
file refusal; input/output symlinks and hardlinks; changed permissions; wrong
architecture; oversized input; bounded frontend count; changed trusted manifest;
and preserved partial work after simulated interrupted copy. Fixture headers
are synthetic and are never executed.

A real packaging smoke copied the already built Release executable and frontend
without executing either. Build and verification both passed with four runtime
files, target `macos-arm64`. No native helper was included in this smoke.

- Bundle: `/private/tmp/ka-runtime-package-29d0f22f63d5c0b5/kAccounting-Extra-01-development-runtime`
- Manifest SHA-256: `776e47f68d7699f209b91d5ff988845e9f380f47a8dcccbfa89d9f514388f23d`
- External receipt: `/private/tmp/ka-runtime-package-29d0f22f63d5c0b5/build-receipt.json`

Do not commit these generated artifacts. Their existence is evidence of runtime
artifact preparation, not evidence that the completed application was deployed.
