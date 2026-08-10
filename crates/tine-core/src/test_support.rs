use std::fs;
use std::path::Path;

pub(crate) const TEST_DEEP_STACK_BYTES: usize = 32 * 1024 * 1024;

#[track_caller]
pub(crate) fn remove_dir_all(path: impl AsRef<Path>) {
    let path = path.as_ref();
    match fs::remove_dir_all(path) {
        Ok(()) => {}
        #[cfg(windows)]
        Err(error)
            if error.raw_os_error()
                == Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION as i32) => {}
        Err(error) => panic!(
            "failed to remove test directory {}: {error}",
            path.display()
        ),
    }
}

/// Run one test body on a deliberately deep stack.
///
/// Promoted/runtime opens in production already run on much larger stacks than
/// libtest's default worker threads provide, so this keeps stack-sensitive test
/// bodies aligned with the production contract without changing assertions or
/// production code paths.
pub(crate) fn run_on_deep_stack(body: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(TEST_DEEP_STACK_BYTES)
        .spawn(body)
        .expect("the deep-stack test thread spawns")
        .join()
        .expect("the deep-stack test body must not panic");
}
