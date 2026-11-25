use super::*;

#[test]
fn empty_request_lists_roots() {
    let listing = list_directories(None).expect("roots");
    assert!(!listing.roots.is_empty());
    assert_eq!(listing.entries.len(), listing.roots.len());
}

#[test]
fn relative_path_is_rejected() {
    assert!(list_directories(Some("relative/path")).is_err());
}

#[test]
fn only_directories_are_returned() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(temp.path().join("child")).expect("mkdir");
    std::fs::write(temp.path().join("file.txt"), "x").expect("write");
    let listing = list_directories(Some(&temp.path().to_string_lossy())).expect("listing");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].name, "child");
    #[cfg(windows)]
    {
        assert!(!listing.path.starts_with(r"\\?\"));
        assert!(!listing.entries[0].path.starts_with(r"\\?\"));
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share")),
            r"\\server\share"
        );
    }
}
