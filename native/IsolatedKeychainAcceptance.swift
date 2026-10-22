// Compile-only acceptance harness until explicit operational approval.
// Never uses an existing book, changes keychain preferences, or deletes items.
import Foundation
import Security
import CryptoKit
import Darwin

var finalPreferenceFailure = false
struct Failure: Error { let step: String; let status: OSStatus }
func check(_ status: OSStatus, _ step: String) throws {
    guard status == errSecSuccess else { throw Failure(step: step, status: status) }
}
func path(_ keychain: SecKeychain) throws -> String {
    var size: UInt32 = 4096
    var bytes = [CChar](repeating: 0, count: Int(size))
    try check(SecKeychainGetPath(keychain, &size, &bytes), "read-keychain-path")
    return String(cString: bytes)
}
struct Preferences: Equatable {
    let defaultPath: String
    let searchPaths: [String]
    var fingerprint: String {
        let bytes = Data(([defaultPath] + searchPaths).joined(separator: "\0").utf8)
        return SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }
    static func capture() throws -> Preferences {
        var primary: SecKeychain?
        try check(SecKeychainCopyDefault(&primary), "snapshot-default")
        guard let primary else { throw Failure(step: "missing-default", status: errSecNoDefaultKeychain) }
        var list: CFArray?
        try check(SecKeychainCopySearchList(&list), "snapshot-search-list")
        guard let items = list as? [SecKeychain] else { throw Failure(step: "invalid-search-list", status: errSecDecode) }
        return Preferences(defaultPath: try path(primary), searchPaths: try items.map(path))
    }
}
func random(_ count: Int) throws -> [UInt8] {
    var bytes = [UInt8](repeating: 0, count: count)
    try check(SecRandomCopyBytes(kSecRandomDefault, count, &bytes), "secure-random")
    return bytes
}
// SecKeychainCreate requires a UTF-8 password. Encode random seed bytes
// directly into ASCII, avoiding an unwipeable Swift String credential copy.
func asciiFixturePassword(_ seed: [UInt8]) -> [UInt8] {
    let digits = Array("0123456789abcdef".utf8)
    var encoded = [UInt8]()
    encoded.reserveCapacity(seed.count * 2)
    for byte in seed { encoded.append(digits[Int(byte >> 4)]); encoded.append(digits[Int(byte & 15)]) }
    return encoded
}
func validFixturePassword(_ bytes: [UInt8]) -> Bool {
    bytes.count == 64 && bytes.allSatisfy { (48...57).contains($0) || (97...102).contains($0) }
}
func status(_ keychain: SecKeychain) throws -> SecKeychainStatus {
    var result: SecKeychainStatus = 0
    try check(SecKeychainGetStatus(keychain, &result), "status-owned-keychain")
    return result
}
// Apple's implementation changes only this helper process's library mUI field.
// No daemon setting, login state, default keychain or search list is modified.
func deniedLockedRead(_ owned: SecKeychain, _ service: String, _ account: String) throws -> OSStatus {
    var original: DarwinBoolean = false
    try check(SecKeychainGetUserInteractionAllowed(&original), "snapshot-helper-interaction")
    try check(SecKeychainSetUserInteractionAllowed(false), "disable-only-helper-interaction")
    var restoreNeeded = true
    defer {
        if restoreNeeded { _ = SecKeychainSetUserInteractionAllowed(original.boolValue) }
    }
    var length: UInt32 = 0
    var data: UnsafeMutableRawPointer?
    let result = service.withCString { serviceBytes in
        account.withCString { accountBytes in
            SecKeychainFindGenericPassword(owned, UInt32(service.utf8.count), serviceBytes,
                UInt32(account.utf8.count), accountBytes, &length, &data, nil)
        }
    }
    let returnedSecret = data != nil || length != 0
    if let data {
        _ = memset_s(data, Int(length), 0, Int(length))
        _ = SecKeychainItemFreeContent(nil, data)
    }
    try check(SecKeychainSetUserInteractionAllowed(original.boolValue), "restore-only-helper-interaction")
    restoreNeeded = false
    guard !returnedSecret else { throw Failure(step: "locked-read-returned-secret", status: result) }
    // This host returns errSecUserCanceled with helper UI disabled. Record it
    // exactly; this does not imply a human clicked Cancel or prove a UI prompt.
    guard result == errSecInteractionNotAllowed || result == errSecInteractionRequired || result == errSecUserCanceled else {
        throw Failure(step: "locked-read-not-denied-as-expected", status: result)
    }
    return result
}
func run() throws {
    let parent = URL(fileURLWithPath: "/private/tmp", isDirectory: true).resolvingSymlinksInPath()
        .appendingPathComponent("ka-isolated-keychain-" + UUID().uuidString.lowercased())
    try FileManager.default.createDirectory(at: parent, withIntermediateDirectories: false,
                                           attributes: [.posixPermissions: 0o700])
    let file = parent.appendingPathComponent("acceptance.keychain")
    let attributes = try FileManager.default.attributesOfItem(atPath: parent.path)
    guard attributes[.type] as? FileAttributeType == .typeDirectory,
          (attributes[.posixPermissions] as? NSNumber)?.intValue == 0o700,
          (attributes[.ownerAccountID] as? NSNumber)?.uint32Value == getuid(),
          file.lastPathComponent == "acceptance.keychain",
          !file.path.contains("/login.keychain"),
          !file.path.contains("/System.keychain"),
          !FileManager.default.fileExists(atPath: file.path),
          !FileManager.default.fileExists(atPath: file.path + "-db") else {
        throw Failure(step: "unsafe-new-fixture-location", status: errSecParam)
    }
    var fixtureSeed = try random(32)
    var fixturePassword = asciiFixturePassword(fixtureSeed)
    var secret = [UInt8]()
    defer {
        fixtureSeed.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) }
        fixturePassword.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) }
        secret.withUnsafeMutableBytes { _ = memset_s($0.baseAddress, $0.count, 0, $0.count) }
    }
    guard validFixturePassword(fixturePassword) else { throw Failure(step: "invalid-utf8-fixture-credential", status: errSecParam) }
    secret = try random(32)
    let before = try Preferences.capture()
    print("preferences_before_sha256=" + before.fingerprint)
    var receipt: [String: Any] = ["format": "kAccounting-Extra-01.isolated-keychain-receipt.v1", "fixture_path": file.path, "preferences_before_sha256": before.fingerprint, "status": "incomplete", "secret_persisted": false, "login_logout_tested": false, "locked_read_tested": false]
    defer {
        do {
            let after = try Preferences.capture()
            receipt["preferences_after_sha256"] = after.fingerprint
            receipt["preferences_unchanged"] = after == before
            if after != before { finalPreferenceFailure = true; receipt["status"] = "preferences_changed_incomplete" }
            print("preferences_after_sha256=" + after.fingerprint)
            print("final_preferences_unchanged=" + String(after == before))
        } catch { receipt["preferences_after_capture_failed"] = true; finalPreferenceFailure = true; receipt["status"] = "preferences_unverified_incomplete" }
        do {
            let data = try JSONSerialization.data(withJSONObject: receipt, options: [.prettyPrinted, .sortedKeys])
            try data.write(to: parent.appendingPathComponent("public-receipt.json"), options: [.withoutOverwriting])
            print("public_receipt=" + parent.appendingPathComponent("public-receipt.json").path)
        } catch { print("public_receipt_write_failed=true") }
    }
    var owned: SecKeychain?
    let creation = fixturePassword.withUnsafeBytes { password in
        SecKeychainCreate(file.path, UInt32(password.count), password.baseAddress, false, nil, &owned)
    }
    try check(creation, "create-new-owned-keychain")
    guard let owned else { throw Failure(step: "missing-created-reference", status: errSecInvalidKeychain) }
    // Print public location immediately so a failure never hides retained state.
    print("owned_fixture_path=" + file.path)
    guard try Preferences.capture() == before else { throw Failure(step: "preferences-changed-stop-no-restoration", status: errSecDecode) }
    let account = UUID().uuidString.lowercased()
    receipt["new_test_account"] = account
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "kAccounting-Extra-01.isolated-os-acceptance.v1",
        kSecAttrAccount as String: account,
        kSecUseKeychain as String: owned,
        kSecValueData as String: Data(secret)
    ]
    try check(SecItemAdd(query as CFDictionary, nil), "create-owned-test-item")
    guard SecItemAdd(query as CFDictionary, nil) == errSecDuplicateItem else { throw Failure(step: "duplicate-not-refused", status: errSecDecode) }
    let read: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrService as String: "kAccounting-Extra-01.isolated-os-acceptance.v1",
        kSecAttrAccount as String: account,
        kSecMatchSearchList as String: [owned],
        kSecReturnData as String: true,
        kSecMatchLimit as String: kSecMatchLimitOne
    ]
    var value: CFTypeRef?
    try check(SecItemCopyMatching(read as CFDictionary, &value), "read-owned-test-item")
    guard let bytes = value as? Data, bytes == Data(secret) else { throw Failure(step: "readback-mismatch", status: errSecDecode) }
    try check(SecKeychainLock(owned), "lock-only-owned-keychain")
    guard try status(owned) & SecKeychainStatus(kSecUnlockStateStatus) == 0 else { throw Failure(step: "owned-keychain-not-locked", status: errSecDecode) }
    print("owned_locked_status_verified=true")
    let denied = try deniedLockedRead(owned, "kAccounting-Extra-01.isolated-os-acceptance.v1", account)
    receipt["locked_read_tested"] = true
    receipt["locked_read_status"] = denied
    receipt["locked_read_denial_kind"] = denied == errSecUserCanceled ? "framework_cancelled_with_helper_ui_disabled" : "interaction_denied_or_required"
    print("owned_locked_read_denied_status=" + String(denied))
    let unlock = fixturePassword.withUnsafeBytes { password in
        SecKeychainUnlock(owned, UInt32(password.count), password.baseAddress, true)
    }
    try check(unlock, "unlock-only-owned-keychain")
    guard try status(owned) & SecKeychainStatus(kSecUnlockStateStatus) != 0 else { throw Failure(step: "owned-keychain-not-unlocked", status: errSecDecode) }
    value = nil
    try check(SecItemCopyMatching(read as CFDictionary, &value), "read-owned-after-unlock")
    guard let again = value as? Data, again == Data(secret) else { throw Failure(step: "unlock-readback-mismatch", status: errSecDecode) }
    guard try Preferences.capture() == before else { throw Failure(step: "preferences-changed-stop-no-restoration", status: errSecDecode) }
    print("preferences_unchanged=true")
    receipt["status"] = "isolated_create_read_duplicate_lock_unlock_read_passed"
    print("owned_create_read_duplicate_lock_unlock_read=true")
    print("fixture_retained=true")
    print("login_logout_acceptance=not_attempted")
}
setvbuf(stdout, nil, _IONBF, 0)
if CommandLine.arguments == [CommandLine.arguments[0], "--self-test-credential-format"] {
    let seed = Array(UInt8(0)...UInt8(31))
    let encoded = asciiFixturePassword(seed)
    guard validFixturePassword(encoded), encoded.count == 64,
          String(bytes: encoded, encoding: .utf8) != nil,
          !validFixturePassword([UInt8](repeating: 0xff, count: 64)),
          !validFixturePassword([UInt8](repeating: 48, count: 63)) else { exit(1) }
    print("fixture_credential_ascii_utf8_format_verified=true")
    print("os_operations_performed=false")
    exit(0)
}
if CommandLine.arguments == [CommandLine.arguments[0], "--preflight"] {
    print("os_operations_performed=false")
    print("approval_required=new-isolated-keychain-and-ephemeral-in-memory-os-fixture-secret")
    exit(0)
}
guard CommandLine.arguments == [CommandLine.arguments[0], "--run-approved-isolated-keychain"] else {
    fputs("Explicit isolated-Keychain operational approval is required.\n", stderr)
    exit(64)
}
do { try run(); if finalPreferenceFailure { exit(1) } }
catch let error as Failure { fputs("fixture_failed_step=\(error.step) status=\(error.status)\n", stderr); exit(1) }
catch { fputs("fixture_failed=local-io-or-decoding\n", stderr); exit(1) }
