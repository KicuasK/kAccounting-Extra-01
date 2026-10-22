use kaccounting_backend::{
    crypto::{EncryptedObjectPurpose, RootEncryptionKey, encrypt_object},
    storage::describe_file,
    transfer_container::*,
};
use std::{
    fs,
    io::{self, Cursor, Read, Write},
    path::PathBuf,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let path =
            std::env::temp_dir().join(format!("ka-container-{:x}", u128::from_be_bytes(random)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn source(&self) -> (PathBuf, Vec<ContainerEntry>) {
        let source = self.0.join("source");
        fs::create_dir(&source).unwrap();
        fs::create_dir(source.join("attachments")).unwrap();
        let names = [
            "book.kae",
            "envelope.json",
            "manifest.kae",
            "attachments/0123456789abcdef0123456789abcdef.kae",
        ];
        let mut entries = vec![];
        for name in names {
            let path = source.join(name);
            let mut f = fs::File::create(&path).unwrap();
            if name == "envelope.json" {
                f.write_all(br#"{"format":"test-public-envelope"}"#)
                    .unwrap();
            } else {
                let purpose = if name == "book.kae" {
                    EncryptedObjectPurpose::TransferDatabase
                } else if name == "manifest.kae" {
                    EncryptedObjectPurpose::TransferManifest
                } else {
                    EncryptedObjectPurpose::TransferAttachment
                };
                let mut generated = io::repeat(42).take(33 * 65536);
                encrypt_object(
                    &mut generated,
                    &mut f,
                    &RootEncryptionKey::new([71; 32]),
                    purpose,
                )
                .unwrap();
            }
            entries.push(ContainerEntry::new(name, describe_file(path).unwrap()).unwrap());
        }
        (source, entries)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
struct Bounded<'a>(&'a mut Cursor<Vec<u8>>);
impl Read for Bounded<'_> {
    fn read(&mut self, b: &mut [u8]) -> io::Result<usize> {
        assert!(b.len() <= COPY_BUFFER_BYTES);
        self.0.read(b)
    }
}
fn encoded(f: &Fixture) -> Vec<u8> {
    let (s, e) = f.source();
    let mut out = vec![];
    let count = write_container(&s, &e, &mut out).unwrap();
    assert_eq!(count, out.len() as u64);
    out
}
#[test]
fn roundtrip_large_bounded_and_source_unchanged() {
    let f = Fixture::new();
    let (s, e) = f.source();
    let before = e.clone();
    let mut out = vec![];
    write_container(&s, &e, &mut out).unwrap();
    let mut cursor = Cursor::new(out);
    let got = extract_container(
        Bounded(&mut cursor),
        &f.0.join("stage"),
        ContainerLimits::default(),
    )
    .unwrap();
    assert_eq!(got.len(), 4);
    for item in before {
        assert_eq!(
            describe_file(s.join(item.name())).unwrap(),
            item.descriptor()
        );
        assert_eq!(
            describe_file(f.0.join("stage").join(item.name())).unwrap(),
            item.descriptor()
        );
    }
}
#[test]
fn unsafe_names_rejected() {
    for n in [
        "../book.kae",
        "/book.kae",
        "attachments/../book.kae",
        "attachments\\abc.kae",
        "book.sqlite3",
        "attachments/ABCDEF0123456789abcdef0123456789ab.kae",
        "attachments/0123456789abcdef0123456789abcdef.kae/other",
    ] {
        let d = kaccounting_backend::storage::StoredObjectDescriptor::new(
            kaccounting_backend::storage::ContentDigest::from_bytes([0; 32]),
            0,
        );
        assert!(ContainerEntry::new(n, d).is_err());
    }
}
#[test]
fn reject_truncation_trailing_wrong_version_and_tamper() {
    let f = Fixture::new();
    let original = encoded(&f);
    for (i, bytes) in [
        original[..original.len() - 1].to_vec(),
        {
            let mut b = original.clone();
            b.push(0);
            b
        },
        {
            let mut b = original.clone();
            b[6] = b'9';
            b
        },
        {
            let mut b = original.clone();
            let n = b.len();
            b[n - 1] ^= 1;
            b
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            extract_container(
                Cursor::new(bytes),
                &f.0.join(format!("stage{i}")),
                ContainerLimits::default()
            )
            .is_err()
        );
    }
}
#[test]
fn reject_capacity_existing_destination_duplicates_and_incomplete_set() {
    let f = Fixture::new();
    let bytes = encoded(&f);
    let limit = ContainerLimits {
        max_total_bytes: 100,
        ..ContainerLimits::default()
    };
    assert!(extract_container(Cursor::new(bytes.clone()), &f.0.join("small"), limit).is_err());
    let dst = f.0.join("exists");
    fs::create_dir(&dst).unwrap();
    fs::write(dst.join("sentinel"), b"preserved").unwrap();
    assert!(extract_container(Cursor::new(bytes), &dst, ContainerLimits::default()).is_err());
    assert_eq!(fs::read(dst.join("sentinel")).unwrap(), b"preserved");
    let (s, e) = (f.0.join("source"), {
        let mut e = vec![];
        for n in ["book.kae", "envelope.json", "manifest.kae"] {
            e.push(
                ContainerEntry::new(n, describe_file(f.0.join("source").join(n)).unwrap()).unwrap(),
            );
        }
        e
    });
    let mut dup = e.clone();
    dup.push(e[0].clone());
    assert!(write_container(&s, &dup, vec![]).is_err());
    assert!(write_container(&s, &e[..2], vec![]).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_sources_and_stage_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let (s, e) = f.source();
    let p = s.join("book.kae");
    fs::rename(&p, s.join("real")).unwrap();
    symlink(s.join("real"), &p).unwrap();
    assert!(write_container(&s, &e, vec![]).is_err());
}
#[test]
fn output_failure_preserves_source() {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("disk full"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let f = Fixture::new();
    let (s, e) = f.source();
    assert!(write_container(&s, &e, Broken).is_err());
    for item in e {
        assert_eq!(
            describe_file(s.join(item.name())).unwrap(),
            item.descriptor()
        );
    }
}

fn raw_headers(names: &[&str]) -> Vec<u8> {
    use kaccounting_backend::storage::describe_reader;
    let digest = describe_reader(&mut Cursor::new(Vec::<u8>::new()))
        .unwrap()
        .digest()
        .to_hex();
    let mut result = CONTAINER_MAGIC.to_vec();
    result.extend_from_slice(&(names.len() as u32).to_be_bytes());
    for name in names {
        result.extend_from_slice(&(name.len() as u16).to_be_bytes());
        result.extend_from_slice(name.as_bytes());
        result.extend_from_slice(&0u64.to_be_bytes());
        for i in 0..32 {
            result.push(u8::from_str_radix(&digest[i * 2..i * 2 + 2], 16).unwrap());
        }
    }
    result
}
#[test]
fn parser_rejects_unknown_duplicate_unsorted_and_missing_names() {
    let f = Fixture::new();
    for (i, names) in [
        vec!["../book.kae", "envelope.json", "manifest.kae"],
        vec!["book.kae", "book.kae", "manifest.kae"],
        vec!["manifest.kae", "envelope.json", "book.kae"],
        vec![
            "attachments/0123456789abcdef0123456789abcdef.kae",
            "book.kae",
            "manifest.kae",
        ],
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            extract_container(
                Cursor::new(raw_headers(&names)),
                &f.0.join(format!("bad{i}")),
                ContainerLimits::default()
            )
            .is_err()
        );
    }
}
#[cfg(unix)]
#[test]
fn extraction_is_owner_only_and_existing_symlink_is_preserved() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let bytes = encoded(&f);
    let destination = f.0.join("private");
    extract_container(
        Cursor::new(bytes.clone()),
        &destination,
        ContainerLimits::default(),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(destination.join("book.kae"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let link = f.0.join("link");
    symlink(&destination, &link).unwrap();
    assert!(extract_container(Cursor::new(bytes), &link, ContainerLimits::default()).is_err());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}
