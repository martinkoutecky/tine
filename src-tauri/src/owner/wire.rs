//! The owner protocol's framing and messages (CLI PROTOCOL §2): NDJSON, one
//! object per `\n`-terminated UTF-8 line, each line bounded before it is
//! buffered. Handshake first each way, then requests and replies in order.

use std::io::{BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Incompatible changes bump this; additive ones do not (§2.4).
pub(crate) const PROTOCOL: u32 = 1;
/// The `--json` / wire success schema (spec A3).
pub(crate) const SCHEMA: &str = "tine.cli/1";
/// Slice 1 verbs carry no page text (§2.1).
pub(crate) const REQUEST_CAP: usize = 1 << 20;
/// Eight times the parse bound (JSON escaping) plus outline metadata.
pub(crate) const REPLY_CAP: usize = 8 * tine_store::PARSE_INPUT_MAX_BYTES as usize + (16 << 20);

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Hello {
    /// "cli" from a client, "owner" from an owner.
    pub(crate) tine: String,
    pub(crate) protocol: u32,
    pub(crate) version: String,
    /// The canonical graph root.
    pub(crate) graph: String,
    /// "app" or "serve", from an owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) owner: Option<String>,
}

/// Where a page host keeps its crash drafts: the inputs `PageHost::start`
/// places them by. Two hosts with equal ones recover each other's drafts,
/// so a yield hands over only between them (PROTOCOL §3.5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Drafts {
    pub(crate) app_data: String,
    pub(crate) graph_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Request {
    pub(crate) id: u64,
    pub(crate) verb: String,
    #[serde(default)]
    pub(crate) args: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub(crate) struct ErrorBody {
    pub(crate) code: String,
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Reply {
    #[serde(default)]
    pub(crate) id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ok: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<ErrorBody>,
}

impl ErrorBody {
    /// An error with `detail.admitted` set, as owners always do (§2.4).
    pub(crate) fn new(code: &str, message: impl Into<String>, admitted: bool) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: Some(serde_json::json!({ "admitted": admitted })),
        }
    }
}

/// Why a frame could not be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FrameError {
    /// The line exceeded the cap; nothing past the cap was buffered.
    TooLarge,
    /// The peer closed mid-line; the partial frame is never executed.
    Incomplete,
    /// I/O failed or timed out.
    Io(String),
}

/// Read one line of at most `cap` bytes (without its `\n`). `Ok(None)` at a
/// clean end of stream.
pub(crate) fn read_frame(
    reader: &mut impl BufRead,
    cap: usize,
) -> Result<Option<Vec<u8>>, FrameError> {
    let mut line = Vec::new();
    loop {
        let buffer = match reader.fill_buf() {
            Ok(buffer) => buffer,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(FrameError::Io(e.to_string())),
        };
        if buffer.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(FrameError::Incomplete)
            };
        }
        let (take, done) = match buffer.iter().position(|&b| b == b'\n') {
            Some(at) => (at, true),
            None => (buffer.len(), false),
        };
        if line.len() + take > cap {
            return Err(FrameError::TooLarge);
        }
        line.extend_from_slice(&buffer[..take]);
        reader.consume(take + usize::from(done));
        if done {
            return Ok(Some(line));
        }
    }
}

/// The one wire reader (§2.1, §2.4): a message is a JSON object, and fields
/// this side does not know are ignored (an additive change needs no protocol
/// bump). An array or a scalar is not a message, even one serde could read
/// positionally into a struct.
pub(crate) fn read_line<T: serde::de::DeserializeOwned>(line: &[u8]) -> Result<T, String> {
    let value: Value = serde_json::from_slice(line).map_err(|e| e.to_string())?;
    read_object(value)
}

/// [`read_line`] for a parsed value: `null` request args read as `{}`.
pub(crate) fn read_object<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    let value = if value.is_null() {
        Value::Object(Default::default())
    } else {
        value
    };
    if !value.is_object() {
        return Err("not a JSON object".into());
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}

/// Write one message as a line and flush it.
pub(crate) fn write_frame(
    writer: &mut impl Write,
    message: &impl Serialize,
) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)?;
    writer.flush()
}

/// A success payload: `schema` and `kind` first, then the verb's fields.
pub(crate) fn ok(kind: &str, fields: Value) -> Value {
    let mut payload = serde_json::json!({ "schema": SCHEMA, "kind": kind });
    if let (Some(payload), Value::Object(fields)) = (payload.as_object_mut(), fields) {
        payload.extend(fields);
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_bounded_before_buffering_and_partial_lines_never_parse() {
        let mut input = std::io::Cursor::new(b"{\"a\":1}\n{\"b\":2}".to_vec());
        assert_eq!(
            read_frame(&mut input, 64).unwrap(),
            Some(b"{\"a\":1}".to_vec())
        );
        assert_eq!(read_frame(&mut input, 64), Err(FrameError::Incomplete));
        let mut big = std::io::BufReader::with_capacity(4, std::io::Cursor::new(vec![b'x'; 100]));
        assert_eq!(read_frame(&mut big, 10), Err(FrameError::TooLarge));
        let mut empty = std::io::Cursor::new(Vec::new());
        assert_eq!(read_frame(&mut empty, 10).unwrap(), None);
    }

    #[test]
    fn readers_take_objects_only_and_ignore_unknown_fields() {
        // Review CODE-1 F3: `[1,"pages",{}]` used to execute as a request.
        assert!(read_line::<Request>(br#"[1,"pages",{}]"#).is_err());
        assert!(read_line::<Request>(b"42").is_err());
        let request: Request =
            read_line(br#"{"id":3,"verb":"pages","args":{},"trace":"x"}"#).unwrap();
        assert_eq!((request.id, request.verb.as_str()), (3, "pages"));
        let hello: Hello = read_line(
            br#"{"tine":"owner","protocol":1,"version":"9","graph":"/g","owner":"app","caps":[]}"#,
        )
        .unwrap();
        assert_eq!(hello.owner.as_deref(), Some("app"));
        #[derive(Deserialize, Debug)]
        struct Args {
            #[allow(dead_code)]
            limit: Option<u32>,
        }
        assert!(read_object::<Args>(serde_json::json!({ "limit": 2, "later": true })).is_ok());
        assert!(read_object::<Args>(Value::Null).is_ok());
        assert!(read_object::<Args>(serde_json::json!([2])).is_err());
    }

    #[test]
    fn an_ok_payload_leads_with_schema_and_kind() {
        let payload = ok("status", serde_json::json!({ "owner": "serve" }));
        assert_eq!(payload["schema"], SCHEMA);
        assert_eq!(payload["kind"], "status");
        assert_eq!(payload["owner"], "serve");
    }
}
