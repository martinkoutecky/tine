// Test-only guest with no SDK dependency: it echoes only the command event's
// focused runtime ID into one host-validated effect and burns a bounded amount
// of worker time so the native journey can switch graphs while the response is
// genuinely pending.
use std::cell::RefCell;

const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const MAX_BLOCK_ID_JSON_BYTES: usize = 1024;
const MAX_JSON_DEPTH: usize = 32;
const EMPTY_RESPONSE: &[u8] = br#"{"protocolVersion":2,"effects":[]}"#;
const RESPONSE_PREFIX: &[u8] =
    br#"{"protocolVersion":2,"effects":[{"kind":"replace-block-text","blockId":"#;
const RESPONSE_SUFFIX: &[u8] =
    br#","expectedRaw":"same raw\nid:: shared-id","raw":"plugin result\nid:: shared-id"}]}"#;

std::thread_local! {
    static TINE_RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn skip_whitespace(input: &[u8], mut at: usize) -> usize {
    while matches!(input.get(at), Some(b' ' | b'\n' | b'\r' | b'\t')) {
        at += 1;
    }
    at
}

fn is_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f' | b'A'..=b'F')
}

fn parse_string(input: &[u8], mut at: usize) -> Option<usize> {
    if input.get(at) != Some(&b'"') {
        return None;
    }
    at += 1;
    while let Some(&byte) = input.get(at) {
        match byte {
            b'"' => return Some(at + 1),
            0..=0x1f => return None,
            b'\\' => {
                at += 1;
                match input.get(at) {
                    Some(b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't') => at += 1,
                    Some(b'u') => {
                        let digits = input.get(at + 1..at + 5)?;
                        if !digits.iter().all(|byte| is_hex(*byte)) {
                            return None;
                        }
                        at += 5;
                    }
                    _ => return None,
                }
            }
            _ => at += 1,
        }
    }
    None
}

fn parse_number(input: &[u8], mut at: usize) -> Option<usize> {
    if input.get(at) == Some(&b'-') {
        at += 1;
    }
    match input.get(at) {
        Some(b'0') => at += 1,
        Some(b'1'..=b'9') => {
            at += 1;
            while matches!(input.get(at), Some(b'0'..=b'9')) {
                at += 1;
            }
        }
        _ => return None,
    }
    if input.get(at) == Some(&b'.') {
        at += 1;
        let fraction_start = at;
        while matches!(input.get(at), Some(b'0'..=b'9')) {
            at += 1;
        }
        if at == fraction_start {
            return None;
        }
    }
    if matches!(input.get(at), Some(b'e' | b'E')) {
        at += 1;
        if matches!(input.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let exponent_start = at;
        while matches!(input.get(at), Some(b'0'..=b'9')) {
            at += 1;
        }
        if at == exponent_start {
            return None;
        }
    }
    Some(at)
}

fn parse_array(input: &[u8], mut at: usize, depth: usize) -> Option<usize> {
    if depth >= MAX_JSON_DEPTH || input.get(at) != Some(&b'[') {
        return None;
    }
    at = skip_whitespace(input, at + 1);
    if input.get(at) == Some(&b']') {
        return Some(at + 1);
    }
    loop {
        at = skip_whitespace(input, parse_value(input, at, depth + 1)?);
        match input.get(at) {
            Some(b',') => at = skip_whitespace(input, at + 1),
            Some(b']') => return Some(at + 1),
            _ => return None,
        }
    }
}

fn parse_object(input: &[u8], mut at: usize, depth: usize) -> Option<usize> {
    if depth >= MAX_JSON_DEPTH || input.get(at) != Some(&b'{') {
        return None;
    }
    at = skip_whitespace(input, at + 1);
    if input.get(at) == Some(&b'}') {
        return Some(at + 1);
    }
    loop {
        at = skip_whitespace(input, parse_string(input, at)?);
        if input.get(at) != Some(&b':') {
            return None;
        }
        at = skip_whitespace(input, parse_value(input, at + 1, depth + 1)?);
        match input.get(at) {
            Some(b',') => at = skip_whitespace(input, at + 1),
            Some(b'}') => return Some(at + 1),
            _ => return None,
        }
    }
}

fn parse_value(input: &[u8], at: usize, depth: usize) -> Option<usize> {
    let at = skip_whitespace(input, at);
    match input.get(at) {
        Some(b'"') => parse_string(input, at),
        Some(b'{') => parse_object(input, at, depth),
        Some(b'[') => parse_array(input, at, depth),
        Some(b't') if input.get(at..at + 4) == Some(b"true") => Some(at + 4),
        Some(b'f') if input.get(at..at + 5) == Some(b"false") => Some(at + 5),
        Some(b'n') if input.get(at..at + 4) == Some(b"null") => Some(at + 4),
        Some(b'-' | b'0'..=b'9') => parse_number(input, at),
        _ => None,
    }
}

fn json_string_equals(input: &[u8], start: usize, end: usize, expected: &[u8]) -> bool {
    input.get(start..end) == Some(expected)
}

// Returns a direct object field and the index after the closing brace. Exact
// field spelling is intentional: Tine serializes these protocol keys literally.
fn object_field(
    input: &[u8],
    mut at: usize,
    wanted: &[u8],
) -> Option<(Option<(usize, usize)>, usize)> {
    if input.get(at) != Some(&b'{') {
        return None;
    }
    at = skip_whitespace(input, at + 1);
    let mut found = None;
    if input.get(at) == Some(&b'}') {
        return Some((found, at + 1));
    }
    loop {
        let key_start = at;
        let key_end = parse_string(input, key_start)?;
        at = skip_whitespace(input, key_end);
        if input.get(at) != Some(&b':') {
            return None;
        }
        let value_start = skip_whitespace(input, at + 1);
        let value_end = parse_value(input, value_start, 0)?;
        if json_string_equals(input, key_start, key_end, wanted) {
            if found.is_some() {
                return None;
            }
            found = Some((value_start, value_end));
        }
        at = skip_whitespace(input, value_end);
        match input.get(at) {
            Some(b',') => at = skip_whitespace(input, at + 1),
            Some(b'}') => return Some((found, at + 1)),
            _ => return None,
        }
    }
}

fn root_object_field(input: &[u8], wanted: &[u8]) -> Option<Option<(usize, usize)>> {
    let (field, end) = object_field(input, 0, wanted)?;
    (skip_whitespace(input, end) == input.len()).then_some(field)
}

fn command_event(input: &[u8]) -> bool {
    std::str::from_utf8(input).is_ok()
        && matches!(
            root_object_field(input, b"\"kind\""),
            Some(Some((start, end))) if json_string_equals(input, start, end, b"\"command\"")
        )
}

// Keep the original JSON token rather than decoding/re-encoding it. It was
// fully validated above, so JSON parsing on the host yields the exact runtime
// ID while preserving any legal escapes without an SDK dependency.
fn focused_block_id(input: &[u8]) -> Option<&[u8]> {
    let (focused_start, focused_end) = root_object_field(input, b"\"focusedBlock\"")??;
    let (id, end) = object_field(input, focused_start, b"\"id\"")?;
    if end != focused_end {
        return None;
    }
    let (id_start, id_end) = id?;
    let id = input.get(id_start..id_end)?;
    if id.len() <= 2
        || id.len() > MAX_BLOCK_ID_JSON_BYTES
        || id.first() != Some(&b'"')
        || id.last() != Some(&b'"')
    {
        return None;
    }
    Some(id)
}

fn set_response(block_id: Option<&[u8]>) -> u32 {
    TINE_RESULT.with(|slot| {
        let mut response = slot.borrow_mut();
        response.clear();
        if let Some(block_id) = block_id {
            response.reserve(RESPONSE_PREFIX.len() + block_id.len() + RESPONSE_SUFFIX.len());
            response.extend_from_slice(RESPONSE_PREFIX);
            response.extend_from_slice(block_id);
            response.extend_from_slice(RESPONSE_SUFFIX);
        } else {
            response.extend_from_slice(EMPTY_RESPONSE);
        }
        response.as_ptr() as usize as u32
    })
}

#[no_mangle]
pub extern "C" fn tine_alloc(length: u32) -> u32 {
    if length as usize > MAX_MESSAGE_BYTES {
        return 0;
    }
    let mut input = Vec::<u8>::with_capacity(length as usize);
    let pointer = input.as_mut_ptr() as usize as u32;
    std::mem::forget(input);
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn tine_handle(pointer: u32, length: u32) -> u32 {
    let input = if length as usize <= MAX_MESSAGE_BYTES {
        unsafe {
            Vec::from_raw_parts(
                pointer as usize as *mut u8,
                length as usize,
                length as usize,
            )
        }
    } else {
        Vec::new()
    };
    let command = command_event(&input);
    if command {
        let mut value = 1_u64;
        for index in 0..12_000_000_u64 {
            value = value.wrapping_mul(6364136223846793005).wrapping_add(index);
            unsafe { std::ptr::write_volatile(&mut value, value) };
        }
        std::hint::black_box(value);
    }
    set_response(command.then(|| focused_block_id(&input)).flatten())
}

#[no_mangle]
pub extern "C" fn tine_result_len() -> u32 {
    TINE_RESULT.with(|slot| slot.borrow().len() as u32)
}
