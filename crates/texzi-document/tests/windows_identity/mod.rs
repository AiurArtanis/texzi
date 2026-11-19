use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};
use texzi_document::files::windows_identity::*;
fn fixture() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "texzi-t07-identity-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    eprintln!("T07 generated identity fixture retained: {}", dir.display());
    dir
}
#[test]
fn actual_file_id_unifies_hardlinks_and_detects_external_bytes() {
    let dir = fixture();
    let original = dir.join("original.txt");
    let alias = dir.join("hardlink.txt");
    fs::write(&original, b"original\r\n").unwrap();
    fs::hard_link(&original, &alias).unwrap();
    let a = ObservedTarget::inspect(&original).unwrap();
    let b = ObservedTarget::inspect(&alias).unwrap();
    assert_eq!(a.version().unwrap().identity, b.version().unwrap().identity);
    assert_eq!(a.reservation_key(), b.reservation_key());
    assert_eq!(a.parent_identity(), b.parent_identity());
    assert!(!a.parent_security().as_bytes().is_empty());
    assert!(!a.version().unwrap().security.as_bytes().is_empty());
    assert_eq!(a.save_scope(), b.save_scope());
    assert_eq!(a.save_scope().object, Some(a.reservation_key()));
    a.recheck().unwrap();
    b.recheck().unwrap();
    fs::write(&alias, b"external\r\n").unwrap();
    assert!(matches!(a.recheck(), Err(IdentityError::Changed)));
    assert_eq!(a.version().unwrap().bytes.as_ref(), b"original\r\n");
    assert_eq!(fs::read(&original).unwrap(), b"external\r\n");
}
#[test]
fn absent_names_reserve_parent_and_reject_creation_conflict() {
    let dir = fixture();
    let p = dir.join("new.txt");
    let a = ObservedTarget::inspect(&p).unwrap();
    let b = ObservedTarget::inspect(&dir.join("other.txt")).unwrap();
    assert!(a.version().is_none());
    assert_eq!(a.reservation_key(), b.reservation_key());
    assert_eq!(a.save_scope().parent, b.save_scope().parent);
    assert_eq!(a.save_scope().object, None);
    a.recheck().unwrap();
    fs::write(&p, b"external").unwrap();
    assert!(matches!(a.recheck(), Err(IdentityError::Changed)));
    b.recheck().unwrap();
    assert_eq!(fs::read(p).unwrap(), b"external");
    assert!(matches!(
        ObservedTarget::inspect(std::path::Path::new("relative.txt")),
        Err(IdentityError::InvalidPath)
    ));
    assert!(matches!(
        ObservedTarget::inspect(&dir.join("new.txt:stream")),
        Err(IdentityError::InvalidPath)
    ));
    assert!(matches!(
        ObservedTarget::inspect(&dir.join("new.txt.")),
        Err(IdentityError::InvalidPath)
    ));
}
