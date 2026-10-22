use kaccounting_backend::{
    portable_media::{CiphertextStage, MediaIdentity},
    storage::publish_private_directory_noreplace,
};
use std::path::PathBuf;
fn fixture() -> PathBuf {
    let mut random = [0; 16];
    getrandom::fill(&mut random).unwrap();
    let id: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let root = std::env::temp_dir().join(format!("kAccounting-Extra-01-portable-media-{id}"));
    std::fs::create_dir(&root).unwrap();
    root
}
fn source(root: &std::path::Path) -> PathBuf {
    let source = root.join("source");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("attachments")).unwrap();
    for name in ["envelope.json", "book.kae", "manifest.kae"] {
        std::fs::write(source.join(name), name.as_bytes()).unwrap();
    }
    std::fs::write(source.join("attachments/a.kae"), vec![0x84; 128 * 1024 + 7]).unwrap();
    source
}
const NAME: &str = ".kAccounting-Extra-01-ciphertext-0123456789abcdef.partial";
#[test]
fn permissionless_media_contract_does_not_require_private_permissions() {
    let root = fixture();
    let source = source(&root);
    let media = root.join("media");
    std::fs::create_dir(&media).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&media, std::fs::Permissions::from_mode(0o777)).unwrap();
    }
    let identity = MediaIdentity::capture(&media).unwrap();
    let stage = CiphertextStage::create(&media, NAME, &identity).unwrap();
    stage.copy_package(&source, &["a.kae".into()]).unwrap();
    assert_eq!(
        std::fs::read(stage.path().join("attachments/a.kae")).unwrap(),
        vec![0x84; 128 * 1024 + 7]
    );
    let destination = media.join("transfer.kagextra");
    publish_private_directory_noreplace(stage.path(), &destination).unwrap();
    assert_eq!(std::fs::read_dir(&destination).unwrap().count(), 4);
    assert!(!destination.join("snapshot.sqlite3").exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn changed_media_identity_rejects_staging() {
    let root = fixture();
    let media = root.join("media");
    std::fs::create_dir(&media).unwrap();
    let identity = MediaIdentity::capture(&media).unwrap();
    std::fs::rename(&media, root.join("original-media")).unwrap();
    std::fs::create_dir(&media).unwrap();
    assert!(CiphertextStage::create(&media, NAME, &identity).is_err());
    assert_eq!(std::fs::read_dir(&media).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn unplugged_media_and_incomplete_ciphertext_are_preserved() {
    let root = fixture();
    let source = source(&root);
    let media = root.join("media");
    std::fs::create_dir(&media).unwrap();
    let identity = MediaIdentity::capture(&media).unwrap();
    let stage = CiphertextStage::create(&media, NAME, &identity).unwrap();
    std::fs::remove_file(source.join("book.kae")).unwrap();
    assert!(stage.copy_package(&source, &[]).is_err());
    assert!(stage.path().join("envelope.json").exists());
    std::fs::rename(&media, root.join("unplugged-media")).unwrap();
    assert!(stage.validate().is_err());
    drop(stage);
    assert!(root.join("unplugged-media").join(NAME).exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn publication_does_not_replace_existing_output() {
    let root = fixture();
    let source = source(&root);
    let media = root.join("media");
    std::fs::create_dir(&media).unwrap();
    let identity = MediaIdentity::capture(&media).unwrap();
    let stage = CiphertextStage::create(&media, NAME, &identity).unwrap();
    stage.copy_package(&source, &[]).unwrap();
    let destination = media.join("transfer.kagextra");
    std::fs::create_dir(&destination).unwrap();
    std::fs::write(destination.join("sentinel"), b"keep").unwrap();
    assert!(publish_private_directory_noreplace(stage.path(), &destination).is_err());
    assert_eq!(
        std::fs::read(destination.join("sentinel")).unwrap(),
        b"keep"
    );
    assert!(stage.path().exists());
    std::fs::remove_dir_all(root).unwrap();
}
#[cfg(unix)]
#[test]
fn symlink_media_parent_and_stage_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = fixture();
    let media = root.join("media");
    std::fs::create_dir(&media).unwrap();
    let identity = MediaIdentity::capture(&media).unwrap();
    symlink(&media, root.join("alias")).unwrap();
    assert!(MediaIdentity::capture(&root.join("alias")).is_err());
    let stage = CiphertextStage::create(&media, NAME, &identity).unwrap();
    std::fs::rename(stage.path(), media.join("original-stage")).unwrap();
    symlink(media.join("original-stage"), stage.path()).unwrap();
    assert!(stage.validate().is_err());
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn path_traversal_never_creates_staging() {
    let root = fixture();
    let identity = MediaIdentity::capture(&root).unwrap();
    assert!(
        CiphertextStage::create(
            &root,
            ".kAccounting-Extra-01-ciphertext-../oops.partial",
            &identity
        )
        .is_err()
    );
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
    std::fs::remove_dir_all(root).unwrap();
}
