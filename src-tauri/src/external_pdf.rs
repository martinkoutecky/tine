//! Explicit user-opened outside PDFs. No asset-protocol scope or graph writes.
use crate::state::{slot_for_context, GraphContext};
use std::io::Read;

const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;

#[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
fn read_pdf(url: &str) -> Result<Vec<u8>, String> {
    let read = || -> Result<Vec<u8>, String> {
        // Url::parse alone also accepts relative-looking file:foo.pdf.
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
        let path = parsed
            .to_file_path()
            .map_err(|()| "external-pdf-unsupported-url")?;
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        let mut file = crate::device_io::open_regular_file(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::InvalidInput {
                "external-pdf-not-regular".into()
            } else {
                error.to_string()
            }
        })?;
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_PDF_BYTES {
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
    };
    read().map_err(|error| format!("{error}: {url}"))
}

/// Only host link handlers request this read; plugins have no native IPC access.
/// The bound slot is admitted before dispatch; no graph data or files are read.
#[tauri::command]
pub(crate) async fn read_external_pdf(
    url: String,
    state: GraphContext<'_>,
) -> Result<tauri::ipc::Response, String> {
    let _slot = slot_for_context(&state)?;
    #[cfg(any(target_os = "linux", target_os = "windows", target_os = "macos"))]
    {
        crate::state::off_ui(move || read_pdf(&url).map(tauri::ipc::Response::new)).await
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        let _ = url;
        Err("external-pdf-desktop-only".into())
    }
}

#[cfg(all(
    test,
    any(target_os = "linux", target_os = "windows", target_os = "macos")
))]
#[path = "external_pdf_tests.rs"]
mod tests;
