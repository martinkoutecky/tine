//! Normalize only fixture path prefixes, including their postcard lengths.
//! The golden image still uses the original 15-byte /tmp/.tmpXXXXXX stand-in.
use std::path::Path;

pub(super) fn normalize(bytes: &mut Vec<u8>, root: &Path) {
    let root = root.to_str().unwrap().as_bytes();
    let mut at = 0;
    while let Some(found) = bytes[at..].windows(root.len()).position(|w| w == root) {
        let start = at + found;
        let (prefix, length) = (1..=10)
            .find_map(|width| {
                let prefix = start.checked_sub(width)?;
                let length = postcard::from_bytes::<usize>(&bytes[prefix..start]).ok()?;
                (length >= root.len() && start + length <= bytes.len()).then_some((prefix, length))
            })
            .expect("a checkpoint path has its postcard length prefix");
        let mut replacement = postcard::to_stdvec(&(length - root.len() + 15)).unwrap();
        replacement.extend_from_slice(&[b'r'; 15]);
        at = prefix + replacement.len();
        bytes.splice(prefix..start + root.len(), replacement);
    }
}

#[test]
fn path_normalization_preserves_the_original_pin_across_length_prefix_boundaries() {
    let expected = vec![
        format!("{}/pages/A.md", "r".repeat(15)),
        format!("{}/journals/B.md", "r".repeat(15)),
    ];
    for length in [15, 100, 130, 260] {
        let root = format!("/{}", "x".repeat(length - 1));
        let paths = vec![
            format!("{root}/pages/A.md"),
            format!("{root}/journals/B.md"),
        ];
        let mut bytes = postcard::to_stdvec(&paths).unwrap();
        normalize(&mut bytes, Path::new(&root));
        assert_eq!(bytes, postcard::to_stdvec(&expected).unwrap());
    }
}
