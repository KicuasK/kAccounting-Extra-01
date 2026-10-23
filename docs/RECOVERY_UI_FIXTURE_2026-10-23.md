# Reproducible disposable recovery UI fixture

The new `backend/examples/recovery_ui_acceptance.rs` uses unique temporary
directories, invented accounting data, and memory-only device/recipient keys.
It never calls Keychain or opens an existing financial book. It does not modify
the production runtime or automatically register browser capabilities.

## Deterministic fixture and self-check

```
cargo run --locked --example recovery_ui_acceptance -- --self-check
```

This consumes its own temporary work and exits without opening a listener. It
asserts import removal `2`, independently verified published destination `1`,
unrecognized preserved work `1`, and export removal `1` plus unrecognized
preserved work `1`; all active counts are zero. The self-check passed during
development. Its machine-readable receipt also confirmed unchanged device,
inode, byte length, and SHA-256 for both unknown sentinels, the published book
database, source database, and source encrypted attachment.
Latest self-check receipt: `/private/tmp/kaccounting-recovery-fixture-self-check.json`.

The import fixture is built with genuine import calls: one authenticated-package
decryption refusal leaves unpublished owned work; one successful published book
retains owned work and then loses its advisory `published.json` marker. The
export fixture uses the genuine exporter with a deliberately damaged invented
attachment, then restores the exact source ciphertext immediately after refusal.
Only the new fixture bytes are changed. Each recovery parent also contains an
unauthenticated sentinel tree that must remain untouched.

These are **injected failure and advisory-marker-loss states**, not a claim of
process death. The independent SIGKILL regression below covers actual death.

## Safari/native-panel run

Stop any other disposable listener on `127.0.0.1:3000`, then run:

```
cargo run --locked --example recovery_ui_acceptance -- <frontend-dist> <trusted-native-helper>
```

Both paths are trusted local startup arguments, never browser fields. The server
prints the canonical fixture root, import recovery parent, export source book,
and export recovery parent. No key bytes are written or printed. Capture stdout
to an external acceptance log if a durable receipt is needed.

1. In Safari explicitly choose `Recovery-Control-Book`; it is never selected
   automatically.
2. Open storage, choose interrupted import recovery, and use the native panel to
   select the printed `IMPORT-RECOVERY-PARENT`. Explicitly confirm the job.
   Expect removed `2`, verified `1`, active `0`, unrecognized `1`.
3. Choose interrupted export recovery. Use native panels to choose the printed
   `EXPORT-SOURCE-BOOK` and `EXPORT-RECOVERY-PARENT` (the source book directory,
   not the media/package directory). Explicitly confirm.
   Expect removed `1`, verified `0`, active `0`, unrecognized `1`.
4. Preserve the UI job-result evidence, then stop this exact fixture with
   Control-C. Before deleting its unique temporary root, it prints a final JSON
   receipt with missing owned-work counts and tracked-file before/after identity
   and digest checks. The receipt distinguishes filesystem evidence from the
   UI's authenticated `verified_count`; it does not invent a job result.

No recovery HTTP is called by default startup. No capabilities are pre-registered
to bypass the native panels. If only one flow is exercised, the final receipt
reports the other owned work as still present rather than claiming success.
Closing this fixture destroys only its own temporary data and memory keys; use
the receipt before teardown as evidence, not as a reusable financial backup.

## Separate real-process-death evidence

Executed successfully:

```
cargo test --release --locked --test export_process_recovery killed_export_recovers_private_source_work_without_plaintext_on_media -- --ignored --exact
```

Result: `1 passed`, `0 failed`; log
`/private/tmp/kaccounting-recovery-sigkill.log`. The existing regression kills
only its own child during encryption of an invented 128 MiB attachment, reopens
the source using disposable memory keys, authenticates/removes owned work,
checks original source database/ciphertext unchanged, and checks that no
plaintext/source recovery touched transfer media. It is not live Safari
large-attachment or physical cross-Mac evidence.
