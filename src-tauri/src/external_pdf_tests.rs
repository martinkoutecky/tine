use super::*;
use std::fs;
fn url(path: &std::path::Path) -> String {
    tauri::Url::from_file_path(path).unwrap().into()
}

#[test]
fn external_pdf_reader_accepts_encoded_regular_pdf_and_changes_no_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("文 件%#.pdf");
    let original = b"%PDF-1.7\nsynthetic PDF bytes";
    fs::write(&path, original).unwrap();
    assert_eq!(read_pdf(&url(&path)).unwrap(), original);
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn external_pdf_reader_refuses_missing_non_pdf_directory_and_large_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("paper.pdf");
    assert!(read_pdf(&url(&path)).unwrap_err().contains("paper.pdf"));
    fs::write(&path, b"not a PDF").unwrap();
    assert!(read_pdf(&url(&path))
        .unwrap_err()
        .starts_with("external-pdf-not-pdf:"));
    fs::write(&path, b"%PDF").unwrap();
    assert!(read_pdf(&url(&path))
        .unwrap_err()
        .starts_with("external-pdf-not-pdf:"));
    assert!(read_pdf(&url(dir.path()))
        .unwrap_err()
        .starts_with("external-pdf-not-regular:"));
    fs::File::create(&path)
        .unwrap()
        .set_len(MAX_PDF_BYTES + 1)
        .unwrap();
    assert!(read_pdf(&url(&path))
        .unwrap_err()
        .starts_with("external-pdf-too-large:"));
    for link in [
        "file:paper.pdf",
        "file:../paper.pdf",
        "file:/tmp/paper.pdf",
        "https://example.org/paper.pdf",
    ] {
        assert!(read_pdf(link).is_err(), "{link}");
    }
    assert!(read_pdf(&format!("{}#section", url(&path))).is_err());
}

#[cfg(unix)]
#[test]
fn external_pdf_reader_refuses_fifo_without_blocking_and_resolves_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("fifo.pdf");
    let cpath = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(cpath.as_ptr(), 0o600) }, 0);
    assert!(read_pdf(&url(&fifo))
        .unwrap_err()
        .starts_with("external-pdf-not-regular:"));
    let actual = dir.path().join("source.pdf");
    fs::write(&actual, b"%PDF-1.4\n").unwrap();
    let alias = dir.path().join("alias.pdf");
    symlink(&actual, &alias).unwrap();
    assert_eq!(read_pdf(&url(&alias)).unwrap(), fs::read(&actual).unwrap());
    assert!(read_pdf("file://server/share/paper.pdf").is_err());
}

#[cfg(unix)]
#[test]
fn external_pdf_reader_names_permission_denial() {
    use std::os::unix::fs::PermissionsExt;
    // A root test runner can read mode-000 files; refusal is exercised by normal users.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("denied.pdf");
    fs::write(&path, b"%PDF-1.7\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    let result = read_pdf(&url(&path));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let error = result.unwrap_err();
    assert!(error.contains("denied.pdf"), "{error}");
    assert!(
        error.to_lowercase().contains("permission denied"),
        "{error}"
    );
}
