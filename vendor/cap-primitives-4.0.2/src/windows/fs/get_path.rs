use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::{fs, io};

/// Calculates system path of `file`.
///
/// This function will automatically strip the extended prefix from the
/// resultant path to allow for joining this resultant path with relative
/// components.
pub(crate) fn get_path(file: &fs::File) -> io::Result<PathBuf> {
    // get system path to the handle
    let path = winx::file::get_file_path(file)?;

    // strip extended prefix; otherwise we will error out on any relative
    // components with `out_path`
    let wide: Vec<_> = path.as_os_str().encode_wide().collect();
    Ok(path_without_extended_prefix(&wide))
}

fn path_without_extended_prefix(wide: &[u16]) -> PathBuf {
    let unc_prefix = ['\\' as u16, '\\' as _, '?' as _, '\\' as _, 'U' as _, 'N' as _, 'C' as _, '\\' as _];
    if wide.starts_with(&unc_prefix) {
        // An extended UNC path must remain absolute: stripping only `\\?\`
        // would turn it into the relative path `UNC\server\share` (GH #533).
        let mut unc = vec!['\\' as u16, '\\' as _];
        unc.extend_from_slice(&wide[unc_prefix.len()..]);
        PathBuf::from(OsString::from_wide(&unc))
    } else {
        let wide_final = if wide.starts_with(&['\\' as u16, '\\' as _, '?' as _, '\\' as _]) {
            &wide[4..]
        } else {
            wide
        };
        PathBuf::from(OsString::from_wide(wide_final))
    }
}

/// Convenience function for calling `get_path` and concatenating the result
/// with `path`.
pub(super) fn concatenate(file: &fs::File, path: &Path) -> io::Result<PathBuf> {
    let file_path = get_path(file)?;
    Ok(file_path.join(path))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removing_extended_prefix_keeps_unc_paths_absolute() {
        for (source, expected) in [
            (r"\\?\UNC\server\share\graph", r"\\server\share\graph"),
            (r"\\?\UNC\server\share\nested folder\图谱", r"\\server\share\nested folder\图谱"),
            (r"\\?\C:\graph\nested", r"C:\graph\nested"),
            (r"C:\graph", r"C:\graph"),
            (r"\\server\share\graph", r"\\server\share\graph"),
        ] {
            let wide: Vec<u16> = OsStrExt::encode_wide(std::ffi::OsStr::new(source)).collect();
            let result = path_without_extended_prefix(&wide);
            assert_eq!(result, PathBuf::from(expected));
            assert!(result.is_absolute(), "{source:?} became relative");
        }
    }
}
