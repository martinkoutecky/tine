//! Name the platform step an I/O failure came from (master 678830a086af;
//! GH #538, #590). A failed save used to reach the app as `io:InvalidInput`
//! alone, so an Android `EINVAL` from the no-replace rename could not be told
//! from one raised by creating, writing or syncing the temporary file.
//!
//! [`at`] wraps an error with a fixed operation name, keeping its `ErrorKind`
//! and its display text; [`step_of`] reads the name and the OS error back.
//! Operation names are `&'static str` literals, so no path or page content can
//! travel with them (I-5). Diagnosis only: nothing here refuses or retries.

use std::io;

#[derive(Debug)]
struct PlatformStepFailure {
    operation: &'static str,
    source: io::Error,
}

impl std::fmt::Display for PlatformStepFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for PlatformStepFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Label an error with the platform step that raised it. An error that
/// already names its step keeps the innermost name. O(1), no I/O.
pub(crate) fn at(operation: &'static str) -> impl FnOnce(io::Error) -> io::Error {
    move |error| match step_of(&error) {
        Some(_) => error,
        None => io::Error::new(
            error.kind(),
            PlatformStepFailure {
                operation,
                source: error,
            },
        ),
    }
}

/// The fixed step name and OS error code an error carries, if [`at`]
/// labelled it. O(1), no I/O.
pub(crate) fn step_of(error: &io::Error) -> Option<(&'static str, Option<i32>)> {
    let step = error.get_ref()?.downcast_ref::<PlatformStepFailure>()?;
    Some((step.operation, step.source.raw_os_error()))
}
