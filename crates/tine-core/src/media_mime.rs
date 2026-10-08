//! MIME for graph assets and opted-in local images, shared by native and wasm.
//!
//! `from_path` answers from the final case-insensitive extension in O(path bytes).
//! Unknown extensions return `application/octet-stream`; no I/O or codec probing.
//! Callers do not maintain MIME tables or need to know the transport in use.

pub fn from_path(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "ogv" => "video/ogg",
        "mov" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "mp3" | "mpeg" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "wav" => "audio/wav",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/opus",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
    }
}
