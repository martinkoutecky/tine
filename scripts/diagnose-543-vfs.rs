
// Diagnostic only. Install once BEFORE starting any SQLite connection/thread.
// Original sqlite3_file address/layout and all non-instrumented callbacks are preserved.
fn sql543_vfs_install() { sql543_vfs::install(); }
pub(crate) fn sql543_vfs_dump() { sql543_vfs::dump(); }
#[allow(unsafe_op_in_unsafe_fn)]
mod sql543_vfs {
    use rusqlite::ffi;
    use std::{collections::HashMap, ffi::{c_char,c_int,c_void}, sync::{Once,OnceLock,RwLock,atomic::{AtomicU64,AtomicUsize,Ordering}}, time::Instant};
    type Open = unsafe extern "C" fn(*mut ffi::sqlite3_vfs,*const c_char,*mut ffi::sqlite3_file,c_int,*mut c_int)->c_int;
    static INSTALL: Once = Once::new();
    static ORIGINAL_OPEN: AtomicUsize = AtomicUsize::new(0);
    #[derive(Clone,Copy)] struct File { original: usize, replacement: usize, kind: usize }
    static FILES: OnceLock<RwLock<HashMap<usize,File>>> = OnceLock::new();
    struct Stats { calls:AtomicU64, bytes:AtomicU64, nanos:AtomicU64, max_nanos:AtomicU64, errors:AtomicU64 }
    impl Stats { const fn new()->Self { Self { calls:AtomicU64::new(0),bytes:AtomicU64::new(0),nanos:AtomicU64::new(0),max_nanos:AtomicU64::new(0),errors:AtomicU64::new(0) } } }
    static STATS: [[Stats;4];3] = [const { [const { Stats::new() };4] };3];
    fn files()-> &'static RwLock<HashMap<usize,File>> { FILES.get_or_init(||RwLock::new(HashMap::new())) }
    fn file(p:*mut ffi::sqlite3_file)->File { *files().read().unwrap().get(&(p as usize)).unwrap() }
    fn record(f:File,op:usize,bytes:u64,start:Instant,rc:c_int) {
        let ns=start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let s=&STATS[f.kind][op];
        s.calls.fetch_add(1,Ordering::Relaxed); s.bytes.fetch_add(bytes,Ordering::Relaxed);
        s.nanos.fetch_add(ns,Ordering::Relaxed); s.max_nanos.fetch_max(ns,Ordering::Relaxed);
        if rc != ffi::SQLITE_OK {s.errors.fetch_add(1,Ordering::Relaxed);}
    }
    pub fn install() { INSTALL.call_once(||unsafe {
        let vfs=ffi::sqlite3_vfs_find(std::ptr::null());
        assert!(!vfs.is_null());
        let open=(*vfs).xOpen.expect("default VFS xOpen");
        ORIGINAL_OPEN.store(open as usize,Ordering::Release);
        (*vfs).xOpen=Some(open_file);
        eprintln!("SQL543 VFS installed name={}",std::ffi::CStr::from_ptr((*vfs).zName).to_string_lossy());
    }); }
    unsafe extern "C" fn open_file(v:*mut ffi::sqlite3_vfs,n:*const c_char,p:*mut ffi::sqlite3_file,flags:c_int,out:*mut c_int)->c_int {
        let original:Open=std::mem::transmute(ORIGINAL_OPEN.load(Ordering::Acquire));
        let rc=original(v,n,p,flags,out);
        // On failure SQLite may still call xClose if the original set pMethods.
        if !(*p).pMethods.is_null() {
            let orig=(*p).pMethods;
            // Copy only the advertised version's fields; do not read past a v1/v2 table.
            let mut methods:ffi::sqlite3_io_methods=std::mem::zeroed();
            let bytes=match (*orig).iVersion {
                1=>std::mem::offset_of!(ffi::sqlite3_io_methods,xShmMap),
                2=>std::mem::offset_of!(ffi::sqlite3_io_methods,xFetch),
                _=>std::mem::size_of::<ffi::sqlite3_io_methods>(),
            };
            std::ptr::copy_nonoverlapping(orig as *const u8,&mut methods as *mut _ as *mut u8,bytes);
            methods.xClose=Some(close); methods.xRead=Some(read); methods.xWrite=Some(write);
            methods.xSync=Some(sync); methods.xTruncate=Some(truncate);
            let replacement=Box::into_raw(Box::new(methods));
            let kind=if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 {0} else if flags & ffi::SQLITE_OPEN_WAL != 0 {1} else {2};
            files().write().unwrap().insert(p as usize,File{original:orig as usize,replacement:replacement as usize,kind});
            (*p).pMethods=replacement;
        }
        rc
    }
    unsafe extern "C" fn close(p:*mut ffi::sqlite3_file)->c_int {
        let f=file(p); let m=&*(f.original as *const ffi::sqlite3_io_methods);
        // Restore first in case the underlying xClose consults its methods.
        (*p).pMethods=f.original as *const _;
        let rc=(m.xClose.unwrap())(p);
        files().write().unwrap().remove(&(p as usize));
        drop(Box::from_raw(f.replacement as *mut ffi::sqlite3_io_methods));
        rc
    }
    unsafe extern "C" fn read(p:*mut ffi::sqlite3_file,b:*mut c_void,n:c_int,o:i64)->c_int {
        let f=file(p); let m=&*(f.original as *const ffi::sqlite3_io_methods); let start=Instant::now();
        let rc=(m.xRead.unwrap())(p,b,n,o); record(f,0,n.max(0) as u64,start,rc); rc
    }
    unsafe extern "C" fn write(p:*mut ffi::sqlite3_file,b:*const c_void,n:c_int,o:i64)->c_int {
        let f=file(p); let m=&*(f.original as *const ffi::sqlite3_io_methods); let start=Instant::now();
        let rc=(m.xWrite.unwrap())(p,b,n,o); record(f,1,n.max(0) as u64,start,rc); rc
    }
    unsafe extern "C" fn sync(p:*mut ffi::sqlite3_file,flags:c_int)->c_int {
        let f=file(p); let m=&*(f.original as *const ffi::sqlite3_io_methods); let start=Instant::now();
        let rc=(m.xSync.unwrap())(p,flags); record(f,2,0,start,rc); rc
    }
    unsafe extern "C" fn truncate(p:*mut ffi::sqlite3_file,size:i64)->c_int {
        let f=file(p); let m=&*(f.original as *const ffi::sqlite3_io_methods); let start=Instant::now();
        let rc=(m.xTruncate.unwrap())(p,size); record(f,3,0,start,rc); rc
    }
    pub fn dump() {
        for (kind,name) in ["main","wal","other"].iter().enumerate() {
            for (op,label) in ["read","write","sync","truncate"].iter().enumerate() {
                let s=&STATS[kind][op];
                eprintln!("SQL543 VFS file={} op={} calls={} requested_bytes={} elapsed_ms={:.3} max_ms={:.3} non_ok={}",name,label,
                    s.calls.load(Ordering::Relaxed),s.bytes.load(Ordering::Relaxed),s.nanos.load(Ordering::Relaxed) as f64/1e6,
                    s.max_nanos.load(Ordering::Relaxed) as f64/1e6,s.errors.load(Ordering::Relaxed));
            }
        }
    }
}
