//! Shared admission for feature clients that parse stored text.

use std::io;
use tine_core::model::Format;
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
        || (Format::from_path(file.as_str().as_ref()) == Format::Org
            && !tine_core::org::headline_levels_within_limit(&text, 128))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "I-22: outline nesting exceeds 128 levels",
        ));
    }
    Ok((text, rev))
}
