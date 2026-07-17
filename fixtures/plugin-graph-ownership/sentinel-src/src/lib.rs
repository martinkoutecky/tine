// Test-only guest with no SDK dependency: it returns one fixed, host-validated
// effect and burns a bounded amount of worker time for command events so the
// native journey can switch graphs while the response is genuinely pending.
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const RESPONSE: &[u8] = br#"{"protocolVersion":2,"effects":[{"kind":"replace-block-text","blockId":"shared-id","expectedRaw":"same raw\nid:: shared-id","raw":"plugin result\nid:: shared-id"}]}"#;

#[no_mangle]
pub extern "C" fn tine_alloc(length: u32) -> u32 {
    if length as usize > MAX_MESSAGE_BYTES { return 0; }
    let mut input = Vec::<u8>::with_capacity(length as usize);
    let pointer = input.as_mut_ptr() as usize as u32;
    std::mem::forget(input);
    pointer
}

#[no_mangle]
pub unsafe extern "C" fn tine_handle(pointer: u32, length: u32) -> u32 {
    let input = if length as usize <= MAX_MESSAGE_BYTES {
        unsafe { Vec::from_raw_parts(pointer as usize as *mut u8, length as usize, length as usize) }
    } else {
        Vec::new()
    };
    const COMMAND: &[u8] = br#""kind":"command""#;
    if input.windows(COMMAND.len()).any(|window| window == COMMAND) {
        let mut value = 1_u64;
        for index in 0..12_000_000_u64 {
            value = value.wrapping_mul(6364136223846793005).wrapping_add(index);
            unsafe { std::ptr::write_volatile(&mut value, value) };
        }
        std::hint::black_box(value);
    }
    RESPONSE.as_ptr() as usize as u32
}

#[no_mangle]
pub extern "C" fn tine_result_len() -> u32 { RESPONSE.len() as u32 }
