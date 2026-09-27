//! Shared admission for feature clients that parse stored text.

use std::io;
use tine_store::{FileId, FileRev, Store};

pub(crate) fn read(store: &Store, file: &FileId) -> io::Result<(String, FileRev)> {
    let (bytes, rev) = store
        .read(file, Some(tine_store::PARSE_INPUT_MAX_BYTES))
        .map_err(crate::store_error)?;
    let text = String::from_utf8(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "stream did not contain valid UTF-8",
        )
    })?;
    if !tine_store::parse_input_depth_within_limit(&text)
        || (file.as_str().to_ascii_lowercase().ends_with(".org")
            && !tine_core::org::headline_levels_within_limit(&text, 512))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "I-22: input nesting exceeds 512 levels",
        ));
    }
    Ok((text, rev))
}
