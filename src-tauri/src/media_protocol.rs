//! Range-aware, graph-scoped audio/video protocol. The requesting webview label
//! selects the current graph slot, and the core validates a top-level regular
//! asset on every request. Responses are capped to 1 MiB, so even a malformed or
//! range-less request can never make the app read a multi-gigabyte media file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext};

const MAX_CHUNK: u64 = 1024 * 1024;

fn decode_path(path: &str) -> Option<String> {
    let bytes = path.strip_prefix('/').unwrap_or(path).as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let text = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn mime(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
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

fn byte_range(value: Option<&header::HeaderValue>, len: u64) -> Option<(u64, u64)> {
    if len == 0 {
        return None;
    }
    let raw = value?.to_str().ok()?.strip_prefix("bytes=")?;
    let first = raw.split(',').next()?;
    let (start, end) = first.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(len);
        Some((len.saturating_sub(suffix), len.saturating_sub(1)))
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            len.saturating_sub(1)
        } else {
            end.parse::<u64>().ok()?.min(len.saturating_sub(1))
        };
        (end >= start).then_some((start, end))
    }
}

fn response(status: StatusCode, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder().status(status).body(body).unwrap()
}

pub(crate) fn respond<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let Some(bound_name) = decode_path(request.uri().path()) else {
        return response(StatusCode::BAD_REQUEST, Vec::new());
    };
    let Some((binding, name)) = bound_name.split_once('/') else {
        return response(StatusCode::BAD_REQUEST, Vec::new());
    };
    let Ok(binding) = binding.parse::<u64>() else {
        return response(StatusCode::BAD_REQUEST, Vec::new());
    };
    let state = ctx.app_handle().state::<crate::state::AppState>();
    let Ok(slot) = crate::state::slot_for_window(&state, ctx.webview_label()) else {
        return response(StatusCode::FORBIDDEN, Vec::new());
    };
    if slot.binding_generation != binding {
        return response(StatusCode::FORBIDDEN, Vec::new());
    }
    let Ok(path) = slot.graph.stream_asset_path(name) else {
        return response(StatusCode::NOT_FOUND, Vec::new());
    };
    let Ok(mut file) = File::open(path) else {
        return response(StatusCode::NOT_FOUND, Vec::new());
    };
    let Ok(len) = file.metadata().map(|metadata| metadata.len()) else {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
    };
    if len == 0 {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime(name))
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, 0)
            .body(Vec::new())
            .unwrap();
    }
    if request.method() == tauri::http::Method::HEAD {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime(name))
            .header(header::ACCEPT_RANGES, "bytes")
            .header(header::CONTENT_LENGTH, len)
            .body(Vec::new())
            .unwrap();
    }
    let range_header = request.headers().get(header::RANGE);
    let requested = byte_range(range_header, len);
    if range_header.is_some() && requested.is_none() {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Vec::new())
            .unwrap();
    }
    let start = requested.map(|range| range.0).unwrap_or(0);
    if start >= len && len != 0 {
        return Response::builder()
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{len}"))
            .body(Vec::new())
            .unwrap();
    }
    let requested_count = requested
        .map(|(_, end)| end.saturating_add(1).saturating_sub(start))
        .unwrap_or_else(|| len.saturating_sub(start));
    let count = requested_count.min(MAX_CHUNK);
    let mut body = Vec::with_capacity(count as usize);
    if file.seek(SeekFrom::Start(start)).is_err()
        || file.by_ref().take(count).read_to_end(&mut body).is_err()
    {
        return response(StatusCode::INTERNAL_SERVER_ERROR, Vec::new());
    }
    let end = start.saturating_add(count).saturating_sub(1);
    let partial = request.headers().contains_key(header::RANGE) || count < len;
    let mut builder = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header(header::CONTENT_TYPE, mime(name))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, count);
    if partial {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{len}"));
    }
    builder.body(body).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_safe_percent_encoded_names() {
        assert_eq!(
            decode_path("/voice%20memo.wav").as_deref(),
            Some("voice memo.wav")
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=123-")), 500),
            Some((123, 499))
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=-20")), 500),
            Some((480, 499))
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=4-9")), 500),
            Some((4, 9))
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=0-1")), 500),
            Some((0, 1))
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=9-4")), 500),
            None
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("garbage")), 500),
            None
        );
        assert_eq!(
            byte_range(Some(&header::HeaderValue::from_static("bytes=0-")), 0),
            None
        );
    }
}
