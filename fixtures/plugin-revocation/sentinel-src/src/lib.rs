// This test-only guest intentionally has no SDK dependency. Cargo includes a
// path dependency's absolute source identity in Rust crate disambiguation,
// which made otherwise locked builds vary between worktrees. The revocation
// journey proves host-side launch policy, not SDK conformance, so this minimal
// implementation keeps only the stable host ABI and a harmless empty response.
const MAX_MESSAGE_BYTES: usize = 256 * 1024;
const RESPONSE: &[u8] = br#"{"protocolVersion":2,"effects":[]}"#;

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
    if length as usize <= MAX_MESSAGE_BYTES {
        // SAFETY: Tine calls tine_alloc(length), fills exactly that region, and
        // passes the same pointer and length once. Reconstructing frees it.
        drop(unsafe {
            Vec::from_raw_parts(
                pointer as usize as *mut u8,
                length as usize,
                length as usize,
            )
        });
    }
    RESPONSE.as_ptr() as usize as u32
}

#[no_mangle]
pub extern "C" fn tine_result_len() -> u32 {
    RESPONSE.len() as u32
}
