// kAccounting-Extra-01 trusted direct-child picker. No sockets or shell commands.
// Compile only during development; do not run without a deliberate user action.
import AppKit
import Foundation
import Darwin

let format = "kAccounting-Extra-01.native-selection.v1"
let kinds: Set<String> = ["book", "request", "package", "backup", "request_destination", "export_destination", "book_destination", "recovery_parent"]
func stop() -> Never { exit(64) }
func posixCanonicalPath(_ path: String) -> String? {
    guard let pointer = path.withCString({ realpath($0, nil) }) else { return nil }
    defer { free(pointer) }
    return String(cString: pointer)
}
// Foundation can omit /private only when resolving directory URLs. Compare
// POSIX paths instead, permitting only the two standard Darwin root aliases.
// Any other symlink (including one nested below these roots) remains refused.
func nonSymlinkCanonicalPath(_ path: String) -> String? {
    guard let canonical = posixCanonicalPath(path) else { return nil }
    let expected: String
    if path == "/var" || path.hasPrefix("/var/") || path == "/tmp" || path.hasPrefix("/tmp/") {
        expected = "/private" + path
    } else {
        expected = path
    }
    return canonical == expected ? canonical : nil
}
func validatedInitialDirectoryPath(_ path: String, kind: String) -> String? {
    guard kind == "book_destination", path.hasPrefix("/"), path.utf8.count <= 4096,
          !path.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
          let canonical = posixCanonicalPath(path), canonical == path else { return nil }
    var isDirectory: ObjCBool = false
    guard FileManager.default.fileExists(atPath: path, isDirectory: &isDirectory), isDirectory.boolValue else { return nil }
    return canonical
}
let input = FileHandle.standardInput.readData(ofLength: 16385)
guard input.count <= 16384,
      let document = try? JSONSerialization.jsonObject(with: input) as? [String: Any],
      Set(document.keys) == Set(["format", "nonce", "kind"]) || Set(document.keys) == Set(["format", "nonce", "kind", "initial_directory"]),
      document["format"] as? String == format,
      let nonce = document["nonce"] as? String, nonce.count == 64,
      nonce.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
      let kind = document["kind"] as? String, kinds.contains(kind) else { stop() }
// This optional path comes only from trusted process startup configuration,
// never browser JSON. Refuse aliases, traversal, controls and non-directories.
var initialDirectory: URL?
if let supplied = document["initial_directory"] {
    guard let path = supplied as? String,
          let canonical = validatedInitialDirectoryPath(path, kind: kind) else { stop() }
    initialDirectory = URL(fileURLWithPath: canonical, isDirectory: true)
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
// Complete standalone AppKit startup and provide the standard responder-chain
// editing actions required by the panel's path and file-name fields.
let mainMenu = NSMenu()
let appMenuItem = NSMenuItem(title: "StoragePicker", action: nil, keyEquivalent: "")
let appMenu = NSMenu(title: "StoragePicker")
appMenu.addItem(withTitle: "Quit StoragePicker", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
appMenuItem.submenu = appMenu
mainMenu.addItem(appMenuItem)
let editMenuItem = NSMenuItem(title: "Edit", action: nil, keyEquivalent: "")
let editMenu = NSMenu(title: "Edit")
editMenu.autoenablesItems = true
for (title, action, equivalent) in [
    ("Undo", Selector(("undo:")), "z"),
    ("Cut", #selector(NSText.cut(_:)), "x"),
    ("Copy", #selector(NSText.copy(_:)), "c"),
    ("Paste", #selector(NSText.paste(_:)), "v"),
    ("Select All", #selector(NSText.selectAll(_:)), "a")
] {
    let item = editMenu.addItem(withTitle: title, action: action, keyEquivalent: equivalent)
    item.keyEquivalentModifierMask = [.command]
    item.target = nil // Let AppKit resolve the focused panel/field-editor responder.
}
editMenuItem.submenu = editMenu
mainMenu.addItem(editMenuItem)
application.mainMenu = mainMenu
application.finishLaunching()
application.activate(ignoringOtherApps: true)
let destinations: Set<String> = ["request_destination", "export_destination", "book_destination"]
var selection: URL?
if destinations.contains(kind) {
    let panel = NSSavePanel()
    panel.title = "kAccounting-Extra-01"
    panel.canCreateDirectories = false
    panel.isExtensionHidden = false
    if let directory = initialDirectory { panel.directoryURL = directory }
    switch kind {
    case "request_destination":
        panel.message = "Choose a new recipient request file."
        panel.nameFieldStringValue = "recipient-request.json"
    case "export_destination":
        panel.message = "Choose a new encrypted directory package destination."
        panel.nameFieldStringValue = "transfer.kagextra"
    default:
        panel.message = "Choose a new book folder name."
        panel.nameFieldStringValue = "New Book"
    }
    if panel.runModal() == .OK { selection = panel.url }
} else {
    let panel = NSOpenPanel()
    panel.title = "kAccounting-Extra-01"
    panel.allowsMultipleSelection = false
    panel.canCreateDirectories = false
    panel.canChooseFiles = kind == "request"
    panel.canChooseDirectories = kind != "request"
    panel.treatsFilePackagesAsDirectories = true
    panel.resolvesAliases = false
    switch kind {
    case "request": panel.message = "Select the public recipient request file."
    case "book": panel.message = "Select a book folder."
    case "package": panel.message = "Select an encrypted directory package. Single-file packages use the browser upload flow."
    case "backup": panel.message = "Select a local encrypted backup folder."
    default: panel.message = "Select the private recovery parent folder."
    }
    if panel.runModal() == .OK, panel.urls.count == 1 { selection = panel.urls.first }
}
var result: [String: Any] = ["outcome": "cancelled"]
if let url = selection {
    guard url.isFileURL else { stop() }
    // Reject aliases/symlinks rather than silently granting a different target.
    let standardized = url.standardizedFileURL
    if destinations.contains(kind) {
        guard !FileManager.default.fileExists(atPath: standardized.path) else { stop() }
    }
    // Foundation deliberately shortens /private/var and /private/tmp into
    // /var and /tmp; Rust's fs::canonicalize uses POSIX realpath. Emit the
    // latter with the no-custom-symlink check, so both sides bind the same path.
    let canonical: String
    if destinations.contains(kind) {
        guard let parent = nonSymlinkCanonicalPath(standardized.deletingLastPathComponent().path) else { stop() }
        canonical = (parent as NSString).appendingPathComponent(standardized.lastPathComponent)
    } else {
        guard let path = nonSymlinkCanonicalPath(standardized.path) else { stop() }
        canonical = path
    }
    guard canonical.utf8.count <= 4096 else { stop() }
    result = ["outcome": "selected", "path": canonical]
}
let response: [String: Any] = ["format": format, "nonce": nonce, "kind": kind, "result": result]
guard let output = try? JSONSerialization.data(withJSONObject: response, options: [.sortedKeys]), output.count <= 16384 else { stop() }
FileHandle.standardOutput.write(output)
