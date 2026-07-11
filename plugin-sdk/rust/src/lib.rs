//! Minimal guest bindings for Tine plugin API 0.1.
//!
//! A plugin receives JSON events and returns inert, host-validated effects. This
//! crate intentionally exposes no host imports: compile with the template's
//! `--import-memory` linker configuration and Tine supplies bounded memory.

use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024;

#[doc(hidden)]
pub mod __private {
    pub use serde_json;
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub protocol_version: u32,
    pub kind: String,
    #[serde(default)]
    pub contribution_id: Option<String>,
    #[serde(default)]
    pub focused_block: Option<BlockSnapshot>,
    #[serde(default)]
    pub blocks: Vec<BlockSnapshot>,
    #[serde(default)]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockSnapshot {
    pub id: String,
    pub raw: String,
    pub parent_id: Option<String>,
    pub depth: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Effect {
    Notice {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        level: Option<NoticeLevel>,
    },
    ReplaceBlockText {
        #[serde(rename = "blockId")]
        block_id: String,
        #[serde(rename = "expectedRaw")]
        expected_raw: String,
        raw: String,
    },
    InsertAtCaret {
        text: String,
    },
    BlockDecoration {
        #[serde(rename = "blockId")]
        block_id: String,
        decoration: Decoration,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tone: Option<Tone>,
    },
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decoration {
    ThreadLine,
    Badge,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Neutral,
    Accent,
    Warning,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    protocol_version: u32,
    effects: Vec<Effect>,
}

pub fn notice(message: impl Into<String>) -> Effect {
    Effect::Notice {
        message: message.into(),
        level: None,
    }
}

#[doc(hidden)]
pub fn encode_response(result: Result<Vec<Effect>, String>) -> Vec<u8> {
    let effects = match result {
        Ok(effects) => effects,
        Err(message) => vec![Effect::Notice {
            message,
            level: Some(NoticeLevel::Error),
        }],
    };
    serde_json::to_vec(&Response {
        protocol_version: PROTOCOL_VERSION,
        effects,
    })
    .unwrap_or_else(|_| br#"{"protocolVersion":1,"effects":[]}"#.to_vec())
}

/// Export the stable Tine ABI around `fn handle(&Event) -> Result<Vec<Effect>, String>`.
#[macro_export]
macro_rules! tine_plugin {
    ($handler:path) => {
        std::thread_local! {
            static TINE_RESULT: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
        }

        #[no_mangle]
        pub extern "C" fn tine_alloc(length: u32) -> u32 {
            let mut input = Vec::<u8>::with_capacity(length as usize);
            let pointer = input.as_mut_ptr() as usize as u32;
            std::mem::forget(input);
            pointer
        }

        #[no_mangle]
        pub unsafe extern "C" fn tine_handle(pointer: u32, length: u32) -> u32 {
            let result = if length as usize > $crate::MAX_MESSAGE_BYTES {
                Err("plugin event exceeded the SDK input limit".to_string())
            } else {
                // SAFETY: Tine calls tine_alloc(length), fills exactly that region,
                // and passes the same pointer and length once. Reconstructing the
                // allocation here also ensures it is freed after parsing.
                let input = unsafe { Vec::from_raw_parts(pointer as usize as *mut u8, length as usize, length as usize) };
                match $crate::__private::serde_json::from_slice::<$crate::Event>(&input) {
                    Ok(event) if event.protocol_version == $crate::PROTOCOL_VERSION => $handler(&event),
                    Ok(_) => Err("unsupported Tine plugin protocol".to_string()),
                    Err(_) => Err("invalid Tine plugin event".to_string()),
                }
            };
            TINE_RESULT.with(|slot| {
                let mut bytes = slot.borrow_mut();
                *bytes = $crate::encode_response(result);
                bytes.as_ptr() as usize as u32
            })
        }

        #[no_mangle]
        pub extern "C" fn tine_result_len() -> u32 {
            TINE_RESULT.with(|slot| slot.borrow().len() as u32)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_uses_the_wire_field_names() {
        let bytes = encode_response(Ok(vec![Effect::InsertAtCaret {
            text: "hello".to_string(),
        }]));
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["effects"][0]["kind"], "insert-at-caret");
    }
}
