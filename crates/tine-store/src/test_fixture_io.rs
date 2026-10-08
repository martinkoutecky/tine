use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn atomic_write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = path.as_ref();
    let root = path.parent().and_then(Path::parent).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "fixture path has no graph root",
        )
    })?;
    let temp = root.join(format!(
        ".fixture-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&temp, bytes)?;
    std::fs::rename(temp, path)
}
