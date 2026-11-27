//! Explicit user-opened outside PDFs. No asset-protocol scope or graph writes.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
use crate::outside_file::{admit, Binding, Probe, Purpose};
use crate::state::GraphContext;
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
use std::io::Read;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;

/// The local path a PDF link URL names. Url::parse alone also accepts
/// relative-looking file:foo.pdf, so the `file://` prefix is required.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn pdf_url_path(url: &str) -> Result<std::path::PathBuf, String> {
    if !url
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("file://"))
    {
        return Err("external-pdf-invalid-link".into());
    }
    let parsed = tauri::Url::parse(url).map_err(|error| error.to_string())?;
    if parsed.scheme() != "file" || parsed.query().is_some() || parsed.fragment().is_some() {
        return Err("external-pdf-invalid-link".into());
    }
    Ok(parsed
        .to_file_path()
        .map_err(|()| "external-pdf-unsupported-url")?)
}

/// Read the PDF at the admitted path: canonicalize (only now, after
/// admission), a regular file, at most 256 MiB, starting with `%PDF-`.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn read_pdf_at(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let (mut file, meta) = tine_store::open_regular_file(&path, tine_store::FinalSymlink::Follow)
        .map_err(|error| match error {
        tine_store::OpenRegularError::Io(error) => error.to_string(),
        _ => "external-pdf-not-regular".into(),
    })?;
    if meta.len() > MAX_PDF_BYTES {
        return Err("external-pdf-too-large".into());
    }
    let mut header = [0; 5];
    if let Err(error) = file.read_exact(&mut header) {
        if error.kind() != std::io::ErrorKind::UnexpectedEof {
            return Err(error.to_string());
        }
    }
    if &header != b"%PDF-" {
        return Err("external-pdf-not-pdf".into());
    }
    let mut bytes = header.to_vec();
    file.take(MAX_PDF_BYTES + 1 - 5)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err("external-pdf-too-large".into());
    }
    Ok(bytes)
}

/// Parse, admit (tine#985: an outside PDF is never read before the user
/// agrees), then read exactly the admitted path. Reader errors carry the URL;
/// an Ask does not.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
pub(crate) fn read_pdf(
    binding: &Binding<'_>,
    url: &str,
    purpose: Purpose,
    probe: &mut impl Probe,
) -> Result<Vec<u8>, String> {
    let path = pdf_url_path(url).map_err(|error| format!("{error}: {url}"))?;
    let path = admit(binding, &path, purpose, probe)?.into_path()?;
    read_pdf_at(&path).map_err(|error| format!("{error}: {url}"))
}

/// `read_pdf` for the calling window's graph binding and its in-memory grant.
#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn read_pdf_in_slot(
    slot: &crate::state::GraphSlot,
    url: &str,
    purpose: Purpose,
) -> Result<tauri::ipc::Response, String> {
    let binding = Binding::Graph {
        root_key: &slot.root_key,
        granted: &slot.outside_files_granted,
    };
    read_pdf(&binding, url, purpose, &mut crate::outside_file::Fs).map(tauri::ipc::Response::new)
}

/// Only host link handlers request this read; plugins have no native IPC access.
/// The bound slot is admitted before dispatch; no graph data or files are read.
#[tauri::command]
pub(crate) async fn read_external_pdf(
    url: String,
    purpose: Option<String>,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, String> {
    let slot = crate::state::slot_for_context(&state)?;
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    {
        let purpose = Purpose::parse(purpose.as_deref())?;
        crate::state::off_ui(move || read_pdf_in_slot(&slot, &url, purpose)).await
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        let _ = (url, purpose, slot);
        Err("external-pdf-desktop-only".into())
    }
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "windows", target_os = "macos")
))]
#[path = "external_pdf_tests.rs"]
mod tests;
