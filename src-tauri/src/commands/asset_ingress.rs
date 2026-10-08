//! Base64 asset ingress with a byte limit before allocation. The frontend sends
//! one string instead of a JSON number array, avoiding a large WebView parse.

const ASSET_INGRESS_MAX_BYTES: usize = 64 * 1024 * 1024;

fn decoded_base64_len(input: &str) -> Option<usize> {
    if input.len() % 4 != 0 {
        return None;
    }
    let padding = input
        .as_bytes()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'=')
        .count()
        .min(2);
    input
        .len()
        .checked_div(4)?
        .checked_mul(3)?
        .checked_sub(padding)
}

pub(crate) fn decode_asset_b64(b64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    let max_encoded = ASSET_INGRESS_MAX_BYTES.div_ceil(3) * 4;
    if b64.len() > max_encoded
        || decoded_base64_len(b64).is_some_and(|len| len > ASSET_INGRESS_MAX_BYTES)
    {
        return Err("asset payload exceeds 64 MiB ingress limit".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("bad base64 asset payload: {e}"))?;
    if decoded.len() > ASSET_INGRESS_MAX_BYTES {
        return Err("asset payload exceeds 64 MiB ingress limit".into());
    }
    Ok(decoded)
}

#[cfg(test)]
mod asset_ingress_tests {
    use super::{decoded_base64_len, ASSET_INGRESS_MAX_BYTES};

    #[test]
    fn base64_size_gate_accounts_for_padding_before_decode() {
        let encoded = ASSET_INGRESS_MAX_BYTES.div_ceil(3) * 4;
        assert!(encoded / 4 * 3 > ASSET_INGRESS_MAX_BYTES);
        assert_eq!(decoded_base64_len("AAAA"), Some(3));
        assert_eq!(decoded_base64_len("AA=="), Some(1));
        assert_eq!(decoded_base64_len("AAA="), Some(2));
    }
}
