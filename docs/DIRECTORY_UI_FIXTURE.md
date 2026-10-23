# Disposable directory-import Safari fixture

`backend/examples/directory_ui_acceptance.rs` prepares invented data and memory-only
keys. It never opens Keychain or real books, and does not register selection
capabilities or import automatically in server mode.

```sh
cargo run --locked --example directory_ui_acceptance -- --self-check
cargo run --locked --example directory_ui_acceptance -- frontend/dist /absolute/path/to/StoragePicker.app/Contents/MacOS/StoragePicker
```

Server mode uses the normal native host and loopback listener. Follow its printed
canonical paths rather than assuming `/private/tmp`:

1. Explicitly select and open `Directory-Control-Book` in Safari.
2. In storage workflows choose `匯入資料夾移轉包`.
3. Use native panels to select the printed request file, package directory, and
   new destination `Imported-Directory-Book`.
4. Explicitly confirm the import. Observe one attachment, required separate book
   selection, and that the control book remains selected.
5. Stop only this fixture server with Ctrl-C. Its final JSON must report
   `all_checks_passed: true` before the disposable directory is removed.

The receipt checks source database, source attachment, request, and every package
file for identical device/inode/length/SHA-256. It then opens the imported book,
compares the full posted entry (amount `9007199254740993`), streams and verifies
the attachment plaintext, confirms changed attachment ciphertext and a distinct
device-key ID and secret. Secrets are never printed. A missing destination or
failed check exits unsuccessfully; the unique fixture is still cleaned up.

`--self-check` calls the public import function without a browser or listener. It
validates fixture construction and receipt logic only; it is not Safari evidence,
physical cross-Mac evidence, or real Keychain acceptance. Server-mode success
also needs the independently observed Safari workflow recorded in the round's
acceptance document.
