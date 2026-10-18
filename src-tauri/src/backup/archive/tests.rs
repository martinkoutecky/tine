use super::*;
use std::borrow::Cow;

fn policy() -> super::super::SnapshotGraphTextPolicy {
    super::super::SnapshotGraphTextPolicy {
        version: super::super::GRAPH_TEXT_SCOPE_VERSION,
        hidden: vec!["private".into()],
        hidden_parse_failed_closed: false,
    }
}

fn manifest_for(files: &[(String, Vec<u8>)]) -> ZipManifest {
    ZipManifest {
        schema: ZIP_SCHEMA,
        writer: "og".into(),
        root: "/g".into(),
        journals_dir: "journals".into(),
        pages_dir: "pages".into(),
        graph_text_policy: policy(),
        files: files
            .iter()
            .map(|(path, bytes)| ZipFile {
                path: path.clone(),
                sha256: format!("{:x}", Sha256::digest(bytes)),
                len: bytes.len() as u64,
            })
            .collect(),
        complete: true,
    }
}

/// Payload with every shape the format must carry: nested, non-ASCII, a
/// real `graph/` directory, an empty file, incompressible bytes.
fn sample() -> Vec<(String, Vec<u8>)> {
    let mut noise = Vec::new();
    let mut x = 0x1234_5678u32;
    for _ in 0..4096 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        noise.push(x as u8);
    }
    let mut files = vec![
        ("assets/pic.edn".to_owned(), b"{:width 10}\n".to_vec()),
        (
            "graph/pages/inside graph dir.md".to_owned(),
            b"- graph dir\n".to_vec(),
        ),
        (
            "journals/2026_10_11.md".to_owned(),
            "- dnes ✓\n".repeat(50).into_bytes(),
        ),
        (
            "logseq/config.edn".to_owned(),
            b"{:meta/version 1}\n".to_vec(),
        ),
        ("pages/a/b/c/deep.md".to_owned(), b"- deep\n".to_vec()),
        ("pages/empty.md".to_owned(), Vec::new()),
        ("pages/noise.md".to_owned(), noise),
        (
            "pages/Příliš žluťoučký kůň.md".to_owned(),
            "- kůň\n".repeat(20).into_bytes(),
        ),
    ];
    files.sort();
    files
}

fn archive_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let manifest = encode_manifest(&manifest_for(files));
    write_archive(files, &manifest, "2026-10-11_09-30-15").unwrap()
}

fn verify_bytes(bytes: &[u8]) -> Result<Verified, Problem> {
    verify(&mut &bytes[..])
}

fn listing_bytes(bytes: &[u8]) -> Result<ZipManifest, Problem> {
    read_listing(&mut &bytes[..])
}

fn stored<'a>(name: &'a str, data: &'a [u8]) -> Encoded<'a> {
    let mut crc = flate2::Crc::new();
    crc.update(data);
    Encoded {
        name,
        method: STORED,
        crc: crc.sum(),
        size: data.len() as u64,
        stored: Cow::Borrowed(data),
    }
}

/// An archive assembled without the writer's checks, for malformed cases.
pub(in crate::backup) fn raw(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let encoded: Vec<Encoded> = entries
        .iter()
        .map(|(name, data)| stored(name, data))
        .collect();
    assemble(&encoded, dos_time("2026-10-11_09-30-15")).unwrap()
}

fn offset_of(haystack: &[u8], needle: &[u8], nth: usize) -> usize {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .nth(nth)
        .map(|(at, _)| at)
        .expect("needle present")
}

#[test]
fn a_written_archive_verifies_to_the_same_bytes() {
    let files = sample();
    let bytes = archive_of(&files);
    let verified = verify_bytes(&bytes).unwrap();
    assert_eq!(verified.manifest, manifest_for(&files));
    let got: Vec<_> = verified.files.iter().map(Vec::as_slice).collect();
    let want: Vec<_> = files.iter().map(|(_, data)| data.as_slice()).collect();
    assert_eq!(got, want);
    assert_eq!(listing_bytes(&bytes).unwrap(), manifest_for(&files));
    // Text deflates; incompressible bytes and the empty file are stored.
    let directory = read_directory(&mut &bytes[..]).unwrap();
    let method = |name: &str| {
        directory
            .entries
            .iter()
            .find(|e| e.name == name)
            .unwrap()
            .method
    };
    assert_eq!(method("journals/2026_10_11.md"), DEFLATE);
    assert_eq!(method("pages/noise.md"), STORED);
    assert_eq!(method("pages/empty.md"), STORED);
    assert_eq!(directory.entries.last().unwrap().name, MANIFEST_NAME);
}

/// Dossier "opens without Tine": Python's `zipfile` (an independent
/// implementation) lists and extracts a written archive, including
/// non-ASCII, nested and `graph/` paths, with identical bytes; `testzip`
/// checks every CRC.
#[test]
fn an_independent_reader_extracts_identical_bytes() {
    let files = sample();
    let dir = std::env::temp_dir().join(format!("tine-zip-python-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("backup.zip");
    std::fs::write(&path, archive_of(&files)).unwrap();
    let mut want: std::collections::BTreeMap<String, String> = files
        .iter()
        .map(|(name, data)| (name.clone(), format!("{:x}", Sha256::digest(data))))
        .collect();
    let read = python_zip_digests(&path);
    let manifest = read.get(MANIFEST_NAME).cloned().expect("manifest entry");
    want.insert(MANIFEST_NAME.into(), manifest);
    assert_eq!(read, want);
    let _ = std::fs::remove_dir_all(dir);
}

/// `{name: sha256}` of every entry as Python's `zipfile` reads it, after
/// `testzip()` found no bad CRC.
pub(in crate::backup) fn python_zip_digests(
    path: &std::path::Path,
) -> std::collections::BTreeMap<String, String> {
    let script = r#"
import hashlib, json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    assert z.testzip() is None, "bad CRC"
    out = {i.filename: hashlib.sha256(z.read(i)).hexdigest() for i in z.infolist()}
    assert len(out) == len(z.infolist()), "duplicate names"
sys.stdout.write(json.dumps(out))
"#;
    let output = ["python3", "python"]
        .iter()
        .find_map(|python| {
            std::process::Command::new(python)
                .arg("-c")
                .arg(script)
                .arg(path)
                .output()
                .ok()
        })
        .expect("the independent-reader gate needs python3 (or python) on PATH");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// ZIP64 appears only when the entry count needs it (R6), and both this
/// validator and Python read it.
#[test]
fn zip64_end_records_appear_only_for_large_entry_counts() {
    assert!(!contains(
        &archive_of(&sample()),
        &ZIP64_EOCD_SIG.to_le_bytes()
    ));
    let files: Vec<(String, Vec<u8>)> = (0..0xFFFF)
        .map(|i| (format!("pages/{i:05}.md"), format!("- {i}\n").into_bytes()))
        .collect();
    let bytes = archive_of(&files);
    assert!(contains(&bytes, &ZIP64_EOCD_SIG.to_le_bytes()));
    let verified = verify_bytes(&bytes).unwrap();
    assert_eq!(verified.files.len(), 0xFFFF);
    // D6: the end record's version-needed is 45, and nothing else.
    let record = bytes.len() - EOCD_LEN - ZIP64_LOCATOR_LEN - ZIP64_EOCD_LEN;
    let mut patched = bytes.clone();
    patched[record + 14..record + 16].copy_from_slice(&99u16.to_le_bytes());
    assert_eq!(
        listing_bytes(&patched).err(),
        Some(Problem::Unsupported("version needed"))
    );
    let dir = std::env::temp_dir().join(format!("tine-zip64-python-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("backup.zip");
    std::fs::write(&path, &bytes).unwrap();
    assert_eq!(python_zip_digests(&path).len(), 0x10000);
    let _ = std::fs::remove_dir_all(dir);
}

/// D6 (REVIEW-backup-zip F7): version-needed must be what this format
/// writes for the entry (Stored 10, Deflate 20; the ZIP64 end record 45),
/// even when the local and central records agree.
#[test]
fn an_unsupported_version_needed_is_refused() {
    let bytes = archive_of(&sample());
    let mut patched = bytes.clone();
    let mut at = 0;
    while at + 4 <= patched.len() {
        let sig = u32::from_le_bytes(patched[at..at + 4].try_into().unwrap());
        if sig == LOCAL_SIG {
            patched[at + 4..at + 6].copy_from_slice(&99u16.to_le_bytes());
        } else if sig == CENTRAL_SIG {
            patched[at + 6..at + 8].copy_from_slice(&99u16.to_le_bytes());
        }
        at += 1;
    }
    assert!(verify_bytes(&bytes).is_ok());
    assert_eq!(
        verify_bytes(&patched).err(),
        Some(Problem::Unsupported("version needed"))
    );
}

/// A ZIP64 end record where the count does not need one is refused.
#[test]
fn zip64_without_need_is_refused() {
    let files = sample();
    let bytes = archive_of(&files);
    let eocd = bytes.len() - EOCD_LEN;
    let (cd_size, cd_offset) = (u32_at(&bytes, eocd + 12), u32_at(&bytes, eocd + 16));
    let count = files.len() as u64 + 1;
    let mut forged = bytes[..eocd].to_vec();
    let record = forged.len() as u64;
    put32(&mut forged, ZIP64_EOCD_SIG);
    put64(&mut forged, 44);
    put16(&mut forged, 0x032D);
    put16(&mut forged, 45);
    put32(&mut forged, 0);
    put32(&mut forged, 0);
    put64(&mut forged, count);
    put64(&mut forged, count);
    put64(&mut forged, u64::from(cd_size));
    put64(&mut forged, u64::from(cd_offset));
    put32(&mut forged, ZIP64_LOCATOR_SIG);
    put32(&mut forged, 0);
    put64(&mut forged, record);
    put32(&mut forged, 1);
    put32(&mut forged, EOCD_SIG);
    put16(&mut forged, 0);
    put16(&mut forged, 0);
    put16(&mut forged, 0xFFFF);
    put16(&mut forged, 0xFFFF);
    put32(&mut forged, cd_size);
    put32(&mut forged, cd_offset);
    put16(&mut forged, 0);
    assert_eq!(
        verify_bytes(&forged).err(),
        Some(Problem::Damaged("ZIP64 end record"))
    );
}

fn damaged(bytes: &[u8]) -> bool {
    matches!(verify_bytes(bytes), Err(Problem::Damaged(_)))
}

/// Dossier restore gate, archive half: each damage is refused by full
/// verification (the restore path), never returned as bytes.
#[test]
fn damaged_archives_are_refused() {
    let files = sample();
    let good = archive_of(&files);
    assert!(verify_bytes(&good).is_ok());

    // Truncated: anywhere, including just the last byte.
    for cut in [1, EOCD_LEN, good.len() / 2, good.len() - 1] {
        assert!(damaged(&good[..good.len() - cut]), "cut {cut}");
    }
    // Trailing bytes after the end record.
    let mut trailing = good.clone();
    trailing.push(0);
    assert!(damaged(&trailing));

    // One flipped byte in a stored payload (CRC), and in a deflated one.
    let mut flipped = good.clone();
    let noise_at = offset_of(&good, b"pages/noise.md", 0) + b"pages/noise.md".len() + 100;
    flipped[noise_at] ^= 0x01;
    assert!(damaged(&flipped));
    let mut flipped = good.clone();
    let text_at =
        offset_of(&good, b"journals/2026_10_11.md", 0) + b"journals/2026_10_11.md".len() + 3;
    flipped[text_at] ^= 0x01;
    assert!(damaged(&flipped));
}

/// A manifest edit that keeps valid JSON (one digest character) fails its
/// checksum: damaged, also at listing time.
#[test]
fn a_manifest_edit_that_keeps_valid_json_fails_the_checksum() {
    let files = sample();
    let manifest = encode_manifest(&manifest_for(&files));
    let text = String::from_utf8(manifest).unwrap();
    let digest = &manifest_for(&files).files[0].sha256;
    let mut edited = digest.clone();
    edited.replace_range(0..1, if digest.starts_with('0') { "1" } else { "0" });
    let tampered = text.replacen(digest.as_str(), &edited, 1);
    assert!(serde_json::from_str::<serde_json::Value>(&tampered).is_ok());
    let bytes = write_archive(&files, tampered.as_bytes(), "2026-10-11_09-30-15").unwrap();
    assert_eq!(
        listing_bytes(&bytes).err(),
        Some(Problem::Damaged("manifest checksum"))
    );
    assert_eq!(
        verify_bytes(&bytes).err(),
        Some(Problem::Damaged("manifest checksum"))
    );
}

/// Entries and manifest must agree exactly: a listed file with no entry, an
/// unlisted extra entry, a duplicate entry, a `../` path.
#[test]
fn entry_and_manifest_mismatches_are_refused() {
    let files = sample();
    let manifest = encode_manifest(&manifest_for(&files));
    let entries: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice()))
        .collect();
    let with = |entries: Vec<(&str, &[u8])>| {
        let mut all = entries;
        all.push((MANIFEST_NAME, manifest.as_slice()));
        raw(&all)
    };
    assert!(verify_bytes(&with(entries.clone())).is_ok());
    let missing = with(entries[1..].to_vec());
    assert_eq!(
        listing_bytes(&missing).err(),
        Some(Problem::Damaged("manifest does not match the entries"))
    );
    let mut extra = entries.clone();
    extra.insert(0, ("aaa.md", b"unlisted"));
    assert!(damaged(&with(extra)));
    let mut duplicate = entries.clone();
    duplicate.insert(1, entries[0]);
    assert_eq!(
        verify_bytes(&with(duplicate)).err(),
        Some(Problem::Damaged("duplicate entry"))
    );
    let mut two_manifests = entries.clone();
    two_manifests.insert(0, (MANIFEST_NAME, manifest.as_slice()));
    assert!(damaged(&with(two_manifests)));

    let escape = vec![("../escape.md".to_owned(), b"- x\n".to_vec())];
    let escape_manifest = encode_manifest(&manifest_for(&escape));
    let bytes = raw(&[
        ("../escape.md", b"- x\n"),
        (MANIFEST_NAME, escape_manifest.as_slice()),
    ]);
    assert_eq!(
        verify_bytes(&bytes).err(),
        Some(Problem::Damaged("unsafe or duplicate path"))
    );
    assert_eq!(
        write_archive(&escape, &escape_manifest, "2026-10-11_09-30-15").err(),
        Some(Problem::Damaged("unsafe or duplicate path")),
        "the writer refuses what the reader refuses"
    );
}

/// Local headers must equal their central records: a reader that prefers
/// local names must not see a different path than the one verified.
#[test]
fn local_and_central_headers_must_agree() {
    let files = sample();
    let good = archive_of(&files);
    let name = b"pages/a/b/c/deep.md";
    let local = offset_of(&good, name, 0);
    let mut renamed = good.clone();
    renamed[local + 6] = b'X';
    assert_eq!(
        verify_bytes(&renamed).err(),
        Some(Problem::Damaged("local name"))
    );
    let header = local - LOCAL_LEN;
    let mut method = good.clone();
    method[header + 8] ^= 0x08;
    assert_eq!(
        verify_bytes(&method).err(),
        Some(Problem::Damaged("local header"))
    );
    let mut flags = good.clone();
    flags[header + 6] |= 0x08;
    assert_eq!(
        verify_bytes(&flags).err(),
        Some(Problem::Damaged("local header"))
    );
}

/// Zip features Tine never writes are refused as unsupported, in the
/// central directory, before any entry is read.
#[test]
fn features_tine_never_writes_are_unsupported() {
    let good = archive_of(&sample());
    let central = offset_of(&good, &CENTRAL_SIG.to_le_bytes(), 0);
    let edit = |at: usize, f: &dyn Fn(&mut u8)| {
        let mut bytes = good.clone();
        f(&mut bytes[central + at]);
        verify_bytes(&bytes).err()
    };
    // Data descriptor and encryption flags.
    assert_eq!(
        edit(8, &|b| *b |= 0x08),
        Some(Problem::Unsupported("zip flags"))
    );
    assert_eq!(
        edit(8, &|b| *b |= 0x01),
        Some(Problem::Unsupported("zip flags"))
    );
    // Another method (bzip2 = 12).
    assert_eq!(
        edit(10, &|b| *b = 12),
        Some(Problem::Unsupported("compression method"))
    );
    // A symlink's external attributes (0o120777 << 16).
    assert_eq!(
        edit(41, &|b| *b = 0xA1),
        Some(Problem::Unsupported("not a regular file"))
    );
    // A non-ASCII name without the UTF-8 flag.
    let mut bytes = good.clone();
    let utf8 = offset_of(&good, "Příliš".as_bytes(), 1);
    let record = (0..utf8)
        .rev()
        .find(|at| good[*at..].starts_with(&CENTRAL_SIG.to_le_bytes()))
        .unwrap();
    bytes[record + 9] &= !0x08;
    assert_eq!(
        verify_bytes(&bytes).err(),
        Some(Problem::Damaged("entry name encoding"))
    );
}

/// Offsets that overlap, leave gaps or point outside the entry region.
#[test]
fn entry_offsets_must_be_contiguous_and_in_bounds() {
    let good = archive_of(&sample());
    let second = offset_of(&good, &CENTRAL_SIG.to_le_bytes(), 1);
    let mut overlap = good.clone();
    overlap[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        verify_bytes(&overlap).err(),
        Some(Problem::Damaged("entries are not contiguous"))
    );
    let mut outside = good.clone();
    outside[second + 42..second + 46].copy_from_slice(&(good.len() as u32).to_le_bytes());
    assert!(damaged(&outside));
    // The listing alone reads the manifest entry: its bounds are checked too.
    let last = offset_of(&good, MANIFEST_NAME.as_bytes(), 1) - CENTRAL_LEN;
    let mut manifest_outside = good.clone();
    manifest_outside[last + 42..last + 46].copy_from_slice(&u32::MAX.wrapping_sub(1).to_le_bytes());
    assert!(matches!(
        listing_bytes(&manifest_outside),
        Err(Problem::Damaged(_))
    ));
}

/// A deflate stream that does not end exactly where recorded, and recorded
/// sizes larger than the limits, are refused before any large allocation.
#[test]
fn sizes_are_bounded_before_allocation() {
    let deflated = {
        let mut d = flate2::Compress::new(flate2::Compression::default(), false);
        Encoded::new("pages/a.md", b"- hello hello hello hello hello\n", &mut d)
    };
    assert_eq!(deflated.method, DEFLATE);
    let lie = |size: u64| Encoded {
        name: deflated.name,
        method: deflated.method,
        crc: deflated.crc,
        size,
        stored: deflated.stored.clone(),
    };
    let small = assemble(&[lie(5)], dos_time("")).unwrap();
    let directory = read_directory(&mut &small[..]).err();
    assert_eq!(
        directory,
        Some(Problem::Damaged("manifest is not the one last entry"))
    );
    let entry = |bytes: &[u8]| {
        let mut src = bytes;
        let directory = {
            // The single-entry archives below have no manifest; read the
            // entry directly.
            let size = bytes.len() as u64;
            let eocd = src
                .read_at(size - EOCD_LEN as u64, EOCD_LEN as u64)
                .unwrap();
            u64::from(u32_at(&eocd, 16))
        };
        let central = &bytes[directory as usize..];
        let parsed = Entry {
            name: "pages/a.md".into(),
            version_needed: u16_at(central, 6),
            flags: 0,
            method: DEFLATE,
            time: u16_at(central, 12),
            date: u16_at(central, 14),
            crc: u32_at(central, 16),
            compressed: u64::from(u32_at(central, 20)),
            size: u64::from(u32_at(central, 24)),
            offset: 0,
        };
        read_entry(&mut src, &parsed, directory, MAX_ENTRY_BYTES)
    };
    assert_eq!(
        entry(&small).err(),
        Some(Problem::Damaged("entry larger than recorded"))
    );
    let long = assemble(&[lie(4096)], dos_time("")).unwrap();
    assert_eq!(entry(&long).err(), Some(Problem::Damaged("deflate sizes")));
    let huge = assemble(&[lie(MAX_ENTRY_BYTES + 1)], dos_time("")).unwrap();
    assert_eq!(entry(&huge).err(), Some(Problem::TooLarge("entry")));
    // Aggregate: three entries each claiming 200 MiB.
    let claims: Vec<Encoded> = ["a.md", "b.md", "c.md"]
        .into_iter()
        .map(|name| Encoded {
            name,
            size: 200 << 20,
            ..stored(name, b"x")
        })
        .chain(std::iter::once(stored(MANIFEST_NAME, b"{}")))
        .collect();
    let bytes = assemble(&claims, dos_time("")).unwrap();
    assert_eq!(verify_bytes(&bytes).err(), Some(Problem::TooLarge("total")));
    // The writer applies the same per-entry limit.
    let too_big = vec![(
        "pages/big.md".to_owned(),
        vec![0u8; MAX_ENTRY_BYTES as usize + 1],
    )];
    assert_eq!(
        write_archive(&too_big, b"{}", "").err(),
        Some(Problem::TooLarge("entry"))
    );
}

/// The largest single allocation or reallocation made while `TRACK` is on,
/// by any thread: E1's plant observes allocation sizes, not the result.
struct Largest;
static TRACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LARGEST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

std::thread_local! {
    /// This thread's live heap bytes and their peak while `ON` is set
    /// ([`peak_heap_on_this_thread`]); other threads never disturb them.
    static THREAD_ON: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static THREAD_LIVE: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
    static THREAD_PEAK: std::cell::Cell<isize> = const { std::cell::Cell::new(0) };
}

impl Largest {
    fn note(size: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        if TRACK.load(Relaxed) {
            LARGEST.fetch_max(size, Relaxed);
        }
    }

    /// Count `delta` live bytes on this thread when it is tracking.
    fn live(delta: isize) {
        let _ = THREAD_ON.try_with(|on| {
            if on.get() {
                let live = THREAD_LIVE.with(|live| {
                    live.set(live.get() + delta);
                    live.get()
                });
                THREAD_PEAK.with(|peak| peak.set(peak.get().max(live)));
            }
        });
    }
}

/// The peak heap `run` held on this thread above what it held on entry
/// (bytes freed that were allocated before entry count against it).
pub(in crate::backup) fn peak_heap_on_this_thread<T>(run: impl FnOnce() -> T) -> (T, usize) {
    THREAD_LIVE.with(|live| live.set(0));
    THREAD_PEAK.with(|peak| peak.set(0));
    THREAD_ON.with(|on| on.set(true));
    let out = run();
    THREAD_ON.with(|on| on.set(false));
    (out, THREAD_PEAK.with(std::cell::Cell::get) as usize)
}

unsafe impl std::alloc::GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        Self::note(layout.size());
        Self::live(layout.size() as isize);
        unsafe { std::alloc::System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: std::alloc::Layout) -> *mut u8 {
        Self::note(layout.size());
        Self::live(layout.size() as isize);
        unsafe { std::alloc::System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        Self::live(-(layout.size() as isize));
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        Self::note(new_size);
        Self::live(new_size as isize - layout.size() as isize);
        unsafe { std::alloc::System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Largest = Largest;

/// E1 (review 2, R1): within every input limit, long names and a near-cap
/// manifest still encode past [`MAX_ARCHIVE_BYTES`] (16,000 payloads of
/// 33,554 bytes, 3,834-byte names, a 63,488,351-byte manifest: about
/// 707 MB). The writer refuses from the computed length, and no buffer over
/// the cap is ever allocated. The entries borrow one payload and one name,
/// which the layout does not distinguish from distinct ones.
#[test]
fn an_archive_over_the_cap_is_refused_before_its_buffer_exists() {
    use std::sync::atomic::Ordering::Relaxed;
    let payload = vec![0x5Au8; 33_554];
    let name = format!("pages/{}.md", "n".repeat(3_834 - "pages/.md".len()));
    let manifest = vec![b' '; 63_488_351];
    let entry = |name, data| Encoded {
        name,
        method: STORED,
        crc: 0,
        size: 0,
        stored: Cow::Borrowed(data),
    };
    let entries: Vec<Encoded> = (0..16_000)
        .map(|_| entry(name.as_str(), &payload[..]))
        .chain(std::iter::once(entry(MANIFEST_NAME, &manifest[..])))
        .collect();
    LARGEST.store(0, Relaxed);
    TRACK.store(true, Relaxed);
    let result = assemble(&entries, dos_time(""));
    TRACK.store(false, Relaxed);
    assert_eq!(result.err(), Some(Problem::TooLarge("archive")));
    let largest = LARGEST.load(Relaxed) as u64;
    assert!(
        largest <= MAX_ARCHIVE_BYTES,
        "E1/I-14: an over-cap archive is refused from its computed length before any \
         output buffer is allocated (exemplar archive.rs assemble); largest allocation \
         {largest} B"
    );
}

/// Item 7: listing never reads payload entries. Every payload byte is
/// corrupted; the listing still succeeds, full verification does not.
#[test]
fn listing_never_reads_payload_entries() {
    let files = sample();
    let mut bytes = archive_of(&files);
    let directory = read_directory(&mut &bytes[..]).unwrap();
    for entry in &directory.entries[..directory.entries.len() - 1] {
        let data = entry.offset as usize + LOCAL_LEN + entry.name.len();
        for byte in &mut bytes[data..data + entry.compressed as usize] {
            *byte ^= 0x5A;
        }
    }
    assert_eq!(listing_bytes(&bytes).unwrap(), manifest_for(&files));
    assert!(damaged(&bytes));
}

/// A checksum-valid manifest of another schema or field set is unsupported,
/// not damaged.
#[test]
fn a_valid_manifest_of_another_schema_is_unsupported() {
    let mut value = serde_json::to_value(manifest_for(&sample())).unwrap();
    value["schema"] = 6.into();
    let checksum = manifest_checksum(&value);
    value["checksum"] = checksum.into();
    assert_eq!(
        parse_manifest(&serde_json::to_vec(&value).unwrap()).err(),
        Some(Problem::Unsupported("manifest schema"))
    );
    let mut value = serde_json::to_value(manifest_for(&sample())).unwrap();
    value["incremental_base"] = "x".into();
    let checksum = manifest_checksum(&value);
    value["checksum"] = checksum.into();
    assert_eq!(
        parse_manifest(&serde_json::to_vec(&value).unwrap()).err(),
        Some(Problem::Unsupported("manifest fields"))
    );
}

/// The checksum's canonical form is an explicit recursive key sort, not
/// `serde_json`'s map order. Pinned vector ported from og-backup-cas,
/// cross-checked with Python `json.dumps(v, sort_keys=True,
/// separators=(",", ":"), ensure_ascii=False)`.
#[test]
fn the_manifest_checksum_canonical_form_is_pinned() {
    let value = serde_json::json!({
        "zeta": 1,
        "files": [{"path": "graph/pages/Č.md", "sha256": "a".repeat(64)}],
        "name": "Příliš žluťoučký kůň 🐎 \"quoted\" \\ tab\t ctrl\u{1}",
        "unknown_number": 12345,
        "nested": {"b": -7, "a": [true, null, "\u{2028}"]},
    });
    let mut bytes = Vec::new();
    canonical_json(&value, &mut bytes);
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        text.starts_with(r#"{"files":[{"path":"graph/pages/Č.md","#),
        "{text}"
    );
    assert_eq!(
        manifest_checksum(&value),
        "fc6ea8639cd4f2786ad10c743d3112ebc85fa741e72200c5d9325b439071ac20"
    );
}

/// The schema-5 manifest's checksum is pinned (a change of form or field
/// set is a deliberate format change). Independently recomputed by Python
/// in `the_manifest_checksum_matches_python`.
#[test]
fn the_schema_5_checksum_is_pinned() {
    let files = vec![("pages/A.md".to_owned(), b"- a\n".to_vec())];
    let encoded = encode_manifest(&manifest_for(&files));
    let value: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(value["checksum"], PINNED_SCHEMA_5_CHECKSUM);
    assert_eq!(parse_manifest(&encoded).unwrap(), manifest_for(&files));
}

/// Python: `json.dumps(m, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`.
const PINNED_SCHEMA_5_CHECKSUM: &str =
    "6980346b3a1bf14a4515e10080a1042761bfd7270e6110d90f0a09102fd476e9";

#[test]
fn the_manifest_checksum_matches_python() {
    let files = sample();
    let encoded = encode_manifest(&manifest_for(&files));
    let script = r#"
import hashlib, json, sys
m = json.loads(sys.stdin.buffer.read().decode("utf-8"))
c = m.pop("checksum")
s = json.dumps(m, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
assert hashlib.sha256(s.encode("utf-8")).hexdigest() == c, "checksum differs"
"#;
    use std::io::Write;
    let mut child = ["python3", "python"]
        .iter()
        .find_map(|python| {
            std::process::Command::new(python)
                .arg("-c")
                .arg(script)
                .stdin(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .ok()
        })
        .expect("needs python3 (or python) on PATH");
    child.stdin.take().unwrap().write_all(&encoded).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_path_policy_refuses_escapes_and_platform_aliases() {
    for bad in [
        "",
        "/abs.md",
        "../x.md",
        "a/../x.md",
        "a/./x.md",
        "a//x.md",
        "a/",
        "a\\b.md",
        "nul\0.md",
    ] {
        assert!(!path_is_safe(bad, false), "{bad:?}");
    }
    assert!(!path_is_safe(&"a".repeat(MAX_NAME_BYTES + 1), false));
    for ok in [
        "pages/a.md",
        "graph/x.md",
        "pages/aux.md",
        "pages/a:b.md",
        "pages/trailing. .md",
    ] {
        assert!(path_is_safe(ok, false), "{ok:?}");
    }
    for windows_bad in [
        "pages/aux.md",
        "pages/CON",
        "pages/com1.txt.md",
        "pages/LPT9.md",
        "pages/a:b.md",
        "C:/x.md",
        "pages/a.md:stream",
        "pages/trailing.",
        "pages/trailing ",
        "pages/q?.md",
        "pages/ctl\u{1}.md",
    ] {
        assert!(!path_is_safe(windows_bad, true), "{windows_bad:?}");
    }
    assert!(path_is_safe("pages/console.md", true));
    assert!(path_is_safe("pages/com10.md", true));
    // Case variants are distinct archive names; whether a destination
    // treats them as one file is Store's restore preflight (E2).
    assert!(paths_are_safe(["pages/A.md", "pages/a.md"], false));
    assert!(!paths_are_safe(["pages/a.md", "pages/a.md"], false));
}

#[test]
fn dos_time_encodes_the_stamp() {
    assert_eq!(
        dos_time("2026-10-11_09-30-15"),
        ((9 << 11) | (30 << 5) | 7, (46 << 9) | (10 << 5) | 11)
    );
    assert_eq!(dos_time(""), (0, (1 << 5) | 1));
}
