# Private deployment review preparation — 2026-10-23 Taipei

Current user scope: single Mac host with private remote browsers; Developer ID signing/notarization are not required. Development-package integrity and real host lifecycle/Keychain/private ingress qualification remain required. Cross-Mac migration is retained but not an operational release gate. No UI build/change is authorized in this phase.

Status: disabled plans only. Nothing is installed, started, signed, configured
with Serve, or qualified for production. Formal signing and notarization are excluded by the
user. The installed GUI distribution remains unqualified for the required Unix
ingress; historical ephemeral-daemon acceptance does not qualify a persistent
deployment. See `PRIVATE_ACCESS.md` and `PRIVATE_INGRESS_DESIGN.md`.

`scripts/prepare-private-deployment.py` now prepares a concrete review package
without running any supplied executable, `sudo`, `launchctl`, Tailscale or the
accounting service. It never reads existing Tailscale state or Keychain material,
copies GUI state, opens a network listener, configures Serve, or substitutes a
TCP transport. The output contains plans and input identities, not executables
or financial data. Keep output outside the checkout and outside book collections.

## Required inputs

- A completed development runtime from `package-macos-release.py`, its separately
  retained expected manifest SHA-256, and the bundled `bin/StoragePicker` helper.
  The complete bundle and every member are verified against the existing contract.
- Separate, explicitly staged `tailscaled` and `tailscale` thin Mach-O binaries
  matching the runtime architecture, each with an independently reviewed expected
  SHA-256. Files must be owned, executable, single-link and not writable by others;
  each immediate staging directory must be owned mode 700. Symlink components,
  privilege-bearing files, replaced identities and hash mismatches are refused.
- An owned, single-link mode-600 provenance JSON in an owned mode-700 parent.
  Its exact fields are `version` (three numeric components), `source_url`
  (`https://github.com/tailscale/tailscale`), `source_revision` (40 lowercase hex),
  `build_note` (nonempty single-line bounded text), `tailscaled_sha256` and
  `tailscale_sha256`. Both hashes must equal the independently supplied hashes.
  Provenance is an operator assertion, **not** verified reproducible-build or
  signing attestation. No binary is executed to infer its version.
- Existing safe private runtime configuration and books collection. The existing
  launcher validates exact hostname/login/socket schema, private modes, a new
  short Unix ingress socket, the frontend tree, native helper and existing log
  safety. No book is opened or remembered selection restored.
- A new absolute review directory under an existing owned mode-700 parent,
  outside `LaunchAgents`, `LaunchDaemons`, runtime bundle and books collection.

The historical `/private/tmp/ka-ts-source-stage-8876a10dbb362373` inputs remain
absent. During the current takeover the coordinator separately prepared reviewed
source-built v1.104.2 binaries and provenance under the private deliverables
directory (`tailscale-v1.104.2-kmgtcn1z`). The preparer checked those actual files
with the UI2 development runtime to generate an **example-only** disabled review,
using explicitly illustrative hostname/login and an empty disposable collection.
This is input-integrity evidence, not actual private deployment or acceptance.
Do not substitute GUI binaries or infer provenance from a version string.

## Interface

Use `python3 -B scripts/prepare-private-deployment.py --help` for all required
arguments. The complete argument set is:

```text
--runtime <EXISTING_DEVELOPMENT_BUNDLE>
--expected-manifest <TRUSTED_MANIFEST_SHA256>
--tailscaled <STAGED_DAEMON> --tailscaled-hash <TRUSTED_DAEMON_SHA256>
--tailscale <STAGED_CLI> --tailscale-hash <TRUSTED_CLI_SHA256>
--provenance <PRIVATE_PROVENANCE_JSON>
--books-root <EXISTING_PRIVATE_BOOK_COLLECTION>
--private-config <EXISTING_PRIVATE_RUNTIME_JSON>
--output <NEW_PRIVATE_REVIEW_DIRECTORY>
--example-only  (required for illustrative identities; never use as production input)
```

Output uses exclusive creation and mode-600 files in a new mode-700 directory:

| File | Review purpose |
| --- | --- |
| `disabled-root-launchdaemon.plist` | Root daemon plan; `Disabled=true`, `RunAtLoad=false`, `KeepAlive=false`, umask 077 |
| `disabled-user-launchagent.plist` | Existing generator's disabled `serve-private-native-chooser` plan, explicit book selection and native helper |
| `review.json` | Runtime digest, identities, provenance, fixed future paths, both plist hashes, example-only flag and rollback scope |
| `complete.json` | Final review digest marker; `installed=false`, `started=false`, `qualified=false` |

The final marker publishes completion only after all plans are written and every
input is checked again immediately before marker creation, including runtime
file/directory identities, launcher configuration, transport and provenance. Failures
preserve partial output without adopting it on a later run. Existing output is
never overwritten. Input checks detect observed substitutions; this is a review
generator, not an atomic privileged installer. An installer must independently
repeat all integrity and ownership checks under its own final install boundary.

Preparation prints `review_sha256`; retain it independently. The following mode
checks only the saved review's artifact integrity and disabled flags, including
both plist hashes bound by the trusted review digest. It does not revalidate
source inputs or inspect any actual installation:

```text
python3 -B scripts/prepare-private-deployment.py
  --verify <EXISTING_PRIVATE_REVIEW_DIRECTORY>
  --expected-review-sha256 <INDEPENDENTLY_RETAINED_REVIEW_DIGEST>
```

For a visible native panel, prefer packaging a complete helper application with
`package-macos-release.py --native-helper-bundle <StoragePicker.app>` instead of
the standalone `--native-helper` option. The options are mutually exclusive.
The minimal reviewed bundle retains Info.plist identity and its exact executable
at `native/StoragePicker.app/Contents/MacOS/StoragePicker`; metadata and bytes are
hash-bound and checked, while unknown resources/links are refused. The launcher
and deployment preparer consume the verified child path; they never execute the
`.app` directory. A standalone executable remains backward compatible but is
not evidence that its panel is discoverable by app-based UI automation.

## Future installation boundary and operator review

The fixed future root transport location is
`/Library/Application Support/kAccounting-Extra-01/private-transport/`, with
`bin/tailscaled`, `bin/tailscale` and **new** `state/tailscaled.state`. Control
uses **new** `/private/var/run/kaccounting-private-transport/tailscaled.sock`.
This spells out the real canonical macOS target: `/var` is a symlink to
`/private/var`, so `/var/run` would conflict with the no-symlink path policy.
Future directories require `root:wheel`, mode 700; installed binaries require
root ownership, single links, mode 700 and the reviewed hashes. The LaunchDaemon
label is `local.kAccounting-Extra-01.private-transport`. Nothing at these paths
is created, adopted, overwritten or inspected for secrets by this generator.
The root transport control socket is separate from the user's configured
accounting ingress socket; no GUI socket or GUI identity/state is reused.

A separately reviewed administrator-authorized installer must refuse existing
future root directories/state/control paths, provision and securely recreate
the ephemeral `/private/var/run` parent across reboot, and review coexistence with the
existing GUI before any activation. The disabled daemon plan deliberately lacks
that lifecycle installation mechanism. Do not enable it as an operationally
complete service. Login, private HTTPS hostname ownership, tailnet policy and an
exact private-only Unix Serve route need separate operator authorization and
review against the staged release. No Funnel, public service or unauthenticated
TCP fallback is allowed.

Before qualifying production, use only disposable books and dedicated authorized
test keys to verify root peer credentials, HTTPS identity envelopes, wrong-user
and tagged-client refusal, forged headers, Safari flows, native helper behavior,
Keychain ACL/prompt behavior, shutdown, logout/login, restart and new control
directory creation. Preserve the exact-integer, atomic-posting and audit rules.
Permanent login items and restart actions are not performed by preparation.

Rollback must stop only the specifically installed new accounting/transport jobs
and their exact reviewed Serve route. Preserve books, keys, backup packages and
the new node state for review; do not delete persistent state, modify the
original GUI, or remove unrelated jobs. Any later cleanup requires its own
explicit reviewed scope.

## Verification

`python3 -B scripts/test_private_deployment.py` passes twelve synthetic tests for
disabled output and false qualification, digest/runtime tampering, symlinks,
hardlinks, modes/privilege bits, architecture, helper presence, existing output,
unsafe launch/output paths, private configuration/books/socket checks,
provenance mismatches/duplicate keys, interrupted publication and input mutation
after plan emission without a completion marker, offline hash binding and
example-only status, altered/missing/extra/unsafe review files. Fixtures are
non-executable Mach-O headers; no real daemon, financial book or Keychain is used.

See `PRIVATE_DEPLOYMENT_ACCEPTANCE_RUNBOOK_2026-10-23.md` for operator steps and
evidence requirements. Production transport, permanent lifecycle and OS Keychain
qualification remain open; formal signing remains deferred.

## Read-only source-input revalidation (single-host increment)

A new preparation includes `input_snapshot` version 1 binding the private-config path and runtime tree file/directory identities. Before requesting installation authorization, check the retained review plus its current inputs:

```sh
python3 -B scripts/prepare-private-deployment.py \
  --verify /absolute/private/review-directory \
  --expected-review-sha256 INDEPENDENTLY_RETAINED_REVIEW_DIGEST \
  --revalidate-inputs
```

This mode never executes supplied binaries, installs jobs, writes files, starts networking or reads financial keys. It refuses illustrative/example reviews and older reviews without the snapshot. Regular offline `--verify` remains supported for historical artifact checks and continues to return `source_inputs_revalidated=false`.

Live revalidation compares the current runtime digest and identity tree, transport hashes/identities, provenance, private configuration, books collection identity and regenerated disabled launcher. Repeated scans detect observed changes; they are not an atomic installation boundary. Even success keeps `deployment_ready`, `installed`, `started`, `qualified` and `atomic_installation_boundary` false. An authorized installer must repeat checks at publication. No host/login values are inferred and no example identity is promoted to production.
