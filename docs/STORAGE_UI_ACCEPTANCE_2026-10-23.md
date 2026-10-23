# Storage and attachment UI acceptance — 2026-10-23

Use only new disposable books, fixture files, and dedicated test keys. This
checklist does not authorize reading real books or existing financial keys.
Formal Developer ID signing remains deferred; retain the development package.

## Evidence levels

- **Automated:** frontend model/controller/transport tests and backend route,
  streaming, session-generation, and native-host tests. Record the final command,
  actual counts, and log path after the integrated verification finishes.
- **Live:** an observed macOS panel, Safari action, or local runtime operation.
  Record the fixture, action, result, and evidence location individually.
- **Unverified:** a checklist item without corresponding observed evidence.
  Passing a fake-picker test or generating a package does not establish live
  panel, Keychain, login, reboot, or deployed private-ingress acceptance.

Complete integrated run after the native initial-directory fix:
**664 Rust passed, 9 intentionally ignored; 141 Python and 132 frontend passed**.
TypeScript/frontend build and locked Release build succeeded. Verification log:
`/private/tmp/kaccounting-ui3-release-candidate-verify.log`. Three package startup
regressions bring the tool total from the preceding 138 checkpoint to 141.
Final completion, package hashes and source binding use external
`deliverables/ui3-verification/DELIVERY.json`, not a repository self-reference.
Native-helper packages require `serve-native-chooser` and the exact helper argument;
old bundles are explicitly identified by `legacy_native_startup`.
The ignored maximum-attachment HTTP roundtrip/oversize-cleanup test was separately
run in Release and passed (one test); this is not live Safari large-file evidence.
Checked live items below use the root agent's actual Safari evidence.

## Observed Safari evidence

Disposable fake-key fixture book: `fa0a4b7f04c33270dc71338242cefb61`.
These observations use no real financial book or existing financial key.

- Manual backup: the user-flow checkbox and explicit confirmation produced a
  verified backup with `0` attachments, `0` attachment bytes, and `0` preserved
  unrecognized items.
- Posted entry `1`: selecting `kAccounting-ui3-receipt.txt` did not start an
  upload. Explicit confirmation returned 201 and the new 62-byte attachment
  appeared in the list. Integrity verification passed. No remove button was
  present for this posted entry.
- Native download: Safari Downloads showed the 62-byte receipt completed.
  Source `/private/tmp/kAccounting-ui3-receipt.txt` and downloaded
  `/Users/kicuashsu/Downloads/kAccounting-ui3-receipt.txt` both had SHA-256
  `2b22672456be7d47674f3d739c627655a6b28cf1311e787075eab808621dc809`.
- A later disposable fixture `06e9107a4a65c6b0b6da2d55201a2b4d` used the corrected
  trusted collection initial directory. Native Save of `Safari-New-Book` succeeded;
  UI reported an empty book created and remained unselected. The prior full-path
  Save attempt was refused and is retained only as the corrected issue's history.
- On that later fixture, native Save of `recipient-request.json` and explicit
  confirmation created request `71ad75a89cc4797c418ac9fa703aefd3`, fingerprint
  `967041c11e04559bc221d10ea5457e02a1cd7aa68a06267a01e22699afe7ce94`.
  Native selection of source book, same request and new `transfer.kagextra` destination
  displayed the matching full fingerprint. Explicit checkbox confirmation admitted
  export job `1e01a422efb1fd949c79fa8e5fac5f22`; verified success returned package
  `cfbec3d9808e476413b186f4f05f1232`, zero attachments and zero attachment bytes.
- Safari Downloads completed `kAccounting-Extra-01.kagextra` (displayed about
  112 KB). The browser File for that downloaded container, same native request
  and new `Safari-Imported-Book` destination were explicitly confirmed for import.
  Verified success returned the same package, zero attachments, and kept the
  source selected. After refreshing candidates and explicitly choosing the new
  imported book, trial-balance debit and credit each displayed exactly
  `9,007,199,254,740,993`.
- Native restore selected `backup.kabackup` inside the verified managed backup
  `8ac106cd9484e17367799dcde48decf6-000000000000000000001792722686112956000-605e6afa8cbdb12aeda19edb78e77c76.managed`
  and new `Safari-Restored-Book` destination. Explicit checkbox confirmation produced
  succeeded job `698027988cdcc806533836cdee4734de`, zero attachments/bytes and
  `selection_required=true`; source selection remained. The restored book was
  not subsequently opened for report verification.
- The later fixture's canonical temp root is `/private/var/folders/.../T/ka-http...`,
  not `/private/tmp`. This was one Mac with fake keys and two logical endpoints;
  physical cross-Mac, large-attachment transfer, directory-package import and
  interrupted recovery have not passed live acceptance.
  That is the UI3 checkpoint only. Later interrupted-recovery evidence appears
  below; directory import and draft-attachment live acceptance now appear in
  `DRAFT_FOLDER_SAFARI_2026-10-23.md`. Physical cross-Mac and large-file Safari
  evidence remain separate outstanding items.

## Attachments

- [x] Automated: canonical string IDs, exact byte lengths, ownership, duplicate
  IDs, filename/path/package screening, file-size boundary, and verify-result
  identity fail closed; uploaded body is the original `File`.
- [x] Automated: upload requires confirmation; draft upload/detach carry the
  current revision and refresh it; unknown outcomes retain selection and block
  repeat writes pending explicit reconciliation.
- [x] Live: select one ordinary fixture file in Safari, cancel once, then
  explicitly confirm upload to a saved draft; verify name, exact size, list,
  revision update, integrity verification, and confirmed draft detach.
  See `DRAFT_FOLDER_SAFARI_2026-10-23.md`: 70-byte draft fixture, revisions
  1 → 2 on upload, 3 after explicit journal save, 4 after confirmed detach;
  original fixture bytes preserved.
- [x] Live: edit an unsaved journal field and observe that attachment mutations
  become disabled immediately without discarding the journal input.
- [x] Live: append to a posted fixture; existing references and posted fields
  remain immutable; no replace or detach control is offered.
- [x] Live: native download of a real 32-character hexadecimal attachment ID
  reaches Safari; inspect Downloads for completion and compare fixture bytes.
  The UI's handoff message alone is not download-completion evidence.

## Backup and transfer

- [x] Automated: strict backup/result/status models, exact counters, expected
  operation identity, request fingerprint, and unknown-outcome handling pass.
- [x] Live: explicitly create a managed encrypted backup and observe a verified
  successful checkpoint with exact attachment counts (all zero in this fixture).
- [ ] Live: separately exercise retention-warning presentation; a warning must
  remain distinct from checkpoint failure. The successful run above did not
  exercise that branch.
- [ ] Live: each native source/destination panel can be cancelled without
  admitting a storage job; no browser field accepts a filesystem path.
  Incremental Safari observation (UI5): recovery-parent OpenPanel Cancel returned
  “已取消選擇；沒有開始財務作業。”; selection remained empty and confirmation
  stayed disabled. Other panel purposes remain individually unverified.
- [x] Live: create recipient request at a new destination; record public request
  ID and fingerprint; compare the full values at the source before export.
- [x] Live: export to a new destination, observe job progress, and hand the
  completed encrypted container to Safari; verify Downloads completion before
  manually moving it. Preserve the original downloaded package.
- [x] Live: import a `.kagextra` file into a new destination without switching;
  afterward explicitly select it and verify exact trial balances.
- [x] Live: separately import a directory package into a new destination.
  See `DRAFT_FOLDER_SAFARI_2026-10-23.md`: native selection and explicit
  confirmation succeeded with one attachment; the original selection remained.
  Explicitly opening the imported book verified exact trial balances above the
  JavaScript safe-integer limit and the 71-byte attachment's integrity.
- [x] Live: restore into a new destination without changing selection.
- [x] Live: explicitly open the restored book and verify its exact account
  balances (UI4 legacy restore fixture); other report screens are not implied.
- [x] Live: exercise authenticated interrupted import/export recovery; preserve
  unrecognized/uncertain work and confirm original operations are not resubmitted.
  See `BACKUP_IDENTITY_RECOVERY_2026-10-23.md` for exact counts and preserved-file
  receipts; injected authenticated work and the separate SIGKILL test are distinct.

## Concurrency and native boundaries

- [x] Automated: stale generation and A → B → A requests are rejected; native
  panels require mandatory generation and the same dispatch/registration lease.
- [x] Automated: `download_selection` is accepted only on the two exact binary
  GET routes; wrong generation, malformed/duplicate query, extra fields, wrong
  method, and conflicting header are refused before financial dispatch.
- [ ] Live: keep one tab stale, switch the selected disposable book elsewhere,
  then try upload, native selection, and native download; no other book is read
  or changed. Record the server refusal separately from the UI message.
  Incremental UI5 observation: tab B explicitly switched `book` to `empty`; tab A
  still displayed `book`. Clicking its recovery-parent selector did not open a
  native panel and reconciled the page to `empty`, with cleared operation inputs.
  This is UI evidence for stale native selection only; HTTP status capture,
  upload and native-download variants remain separate acceptance items.
- [ ] Automated/live separately: loopback browser composition exposes the typed
  native selection action; private Unix/HTTPS ingress excludes native panels.
- [ ] Unverified until individually observed: production Keychain authorization,
  permanent login item, login/reboot continuity, formal signing/notarization,
  and permanent private proxy deployment. Physical APFS/exFAT acceptance remains
  deferred to Version 02 under the existing user decision.
