//! Windows account boundary (PROTOCOL §1): protected DACLs on new objects,
//! effective directory grants checked on every use (including inherited ACLs),
//! and a private connected pipe checked before sending a hello.
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, LocalFree, GENERIC_EXECUTE, GENERIC_READ, GENERIC_WRITE, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetAce, GetSecurityDescriptorControl, GetTokenInformation, TokenUser, ACCESS_ALLOWED_ACE,
    ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, INHERIT_ONLY_ACE, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_EXECUTE,
    FILE_GENERIC_READ, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use super::OwnerError;

fn unsafe_io(context: &str) -> OwnerError {
    OwnerError::Unsafe(format!("{context}: {}", std::io::Error::last_os_error()))
}

pub(super) fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

/// Own an OS-allocated security descriptor until CreateDirectory/NamedPipe
/// has copied it. No inheritable handles are introduced.
pub(super) struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

impl Descriptor {
    pub(super) fn private(directory: bool) -> Result<Self, OwnerError> {
        let sid = user_sid()?;
        let inherit = if directory { "OICI" } else { "" };
        Self::from_sddl(&format!("O:{sid}D:P(A;{inherit};FA;;;{sid})"))
    }

    pub(super) fn from_sddl(sddl: &str) -> Result<Self, OwnerError> {
        let sddl = wide(std::ffi::OsStr::new(sddl));
        let mut descriptor = null_mut();
        // SAFETY: NUL-terminated input; the returned LocalAlloc buffer is owned.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        } == 0
        {
            return Err(unsafe_io("cannot construct private owner permissions"));
        }
        Ok(Self(descriptor))
    }

    pub(super) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

fn sid_text(sid: *mut c_void) -> Result<String, OwnerError> {
    let mut raw = null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut raw) } == 0 {
        return Err(unsafe_io("cannot read owner SID"));
    }
    let text = unsafe {
        let mut len = 0;
        while *raw.add(len) != 0 {
            len += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(raw, len));
        LocalFree(raw.cast());
        text
    };
    Ok(text)
}

pub(super) fn user_sid() -> Result<String, OwnerError> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(unsafe_io("cannot read account token"));
    }
    // TOKEN_USER contains a pointer: use a pointer-aligned buffer, not Vec<u8>.
    let result = (|| {
        let mut needed = 0;
        unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(unsafe_io("cannot read account SID"));
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        sid_text(user.User.Sid)
    })();
    unsafe { CloseHandle(token) };
    result
}

pub(super) fn pipe_name(key: &str) -> Result<std::path::PathBuf, OwnerError> {
    let name = format!(r"\\.\pipe\tine-{}-{key}", user_sid()?);
    // Windows' full pipe name limit is 256 characters, including its prefix.
    if name.encode_utf16().count() >= 256
        || key.len() != 16
        || !key.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(OwnerError::Unsafe("invalid owner pipe name".into()));
    }
    Ok(name.into())
}

pub(super) fn create_directory(path: &Path) -> Result<(), OwnerError> {
    let descriptor = Descriptor::private(true)?;
    let attributes = descriptor.attributes();
    let name = wide(path.as_os_str());
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(OwnerError::Unsafe(format!(
                "cannot create {}: {error}",
                path.display()
            )));
        }
    }
    check_directory(path)
}

/// OPEN_ALWAYS never replaces the lock object. A fresh file gets a protected
/// user-only DACL, even if the existing directory grants inheritable read access
/// to another account (read access alone suffices to hold a Windows file lock).
/// Windows ignores these attributes when opening an existing file.
pub(super) fn open_lock_file(path: &Path) -> Result<std::fs::File, OwnerError> {
    let descriptor = Descriptor::private(false)?;
    let attributes = descriptor.attributes();
    let name = wide(path.as_os_str());
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            OPEN_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(OwnerError::Lock(format!(
            "cannot open {}: {}",
            path.display(),
            std::io::Error::last_os_error()
        )));
    }
    Ok(unsafe { std::fs::File::from_raw_handle(raw) })
}

pub(super) fn check_directory(path: &Path) -> Result<(), OwnerError> {
    use std::os::windows::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| OwnerError::Unsafe(format!("cannot inspect {}: {e}", path.display())))?;
    if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(OwnerError::Unsafe(format!(
            "{} must be a directory, not a reparse point",
            path.display()
        )));
    }
    let name = wide(path.as_os_str());
    let raw = unsafe {
        CreateFileW(
            name.as_ptr(),
            READ_CONTROL | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(unsafe_io("cannot inspect owner directory security"));
    }
    let file = unsafe { std::fs::File::from_raw_handle(raw) };
    // Check the opened object too, so a reparse replacement cannot escape via
    // the gap between the metadata read and CreateFile.
    let opened = file
        .metadata()
        .map_err(|e| OwnerError::Unsafe(e.to_string()))?;
    if !opened.is_dir() || opened.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(OwnerError::Unsafe(
            "owner directory changed during inspection".into(),
        ));
    }
    check_handle(file.as_raw_handle(), true)
}

pub(super) fn check_handle(handle: HANDLE, directory: bool) -> Result<(), OwnerError> {
    let mut owner = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(OwnerError::Unsafe(format!(
            "cannot inspect owner permissions: {}",
            std::io::Error::from_raw_os_error(status as i32)
        )));
    }
    let _descriptor = Descriptor(descriptor);
    let sid = user_sid()?;
    if owner.is_null() {
        return Err(OwnerError::Unsafe("owner object has no owner".into()));
    }
    let owner = sid_text(owner)?;
    if owner != sid && !(directory && matches!(owner.as_str(), "S-1-5-18" | "S-1-5-32-544")) {
        return Err(OwnerError::Unsafe(
            "owner object must belong to the current account or a trusted directory administrator"
                .into(),
        ));
    }
    let mut control = 0;
    let mut revision = 0;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
        || (!directory && control & SE_DACL_PROTECTED == 0)
        || dacl.is_null()
    {
        return Err(OwnerError::Unsafe(
            "owner object must have a non-null DACL (protected for pipes)".into(),
        ));
    }
    for index in 0..unsafe { (*dacl).AceCount } as u32 {
        let mut raw = null_mut();
        if unsafe { GetAce(dacl, index, &mut raw) } == 0 {
            return Err(unsafe_io("cannot inspect owner access rule"));
        }
        // Evaluate inherited and explicit grants identically. Reject unfamiliar
        // ACE types rather than overlooking object or callback ACEs. Ordinary
        // denials cannot grant another account access.
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        if header.AceType == 1 {
            continue;
        } // ACCESS_DENIED_ACE_TYPE
        if header.AceType != 0 {
            return Err(OwnerError::Unsafe("unrecognized owner access rule".into()));
        }
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        // SID is variable-length and extends past ACCESS_ALLOWED_ACE. Derive
        // its pointer from the whole ACE buffer, not a struct-sized reference.
        let principal =
            sid_text(unsafe { raw.byte_add(std::mem::offset_of!(ACCESS_ALLOWED_ACE, SidStart)) })?;
        let trusted = principal == sid
            || (directory && matches!(principal.as_str(), "S-1-5-18" | "S-1-5-32-544"))
            || (directory
                && principal == "S-1-3-0"
                && u32::from(header.AceFlags) & INHERIT_ONLY_ACE != 0);
        let unsafe_access = if directory {
            // Every bit outside ordinary read/execute rights is refused; this
            // includes generic write/all, delete-child, WRITE_DAC/OWNER.
            ace.Mask & !(FILE_GENERIC_READ | FILE_GENERIC_EXECUTE | GENERIC_READ | GENERIC_EXECUTE)
                != 0
        } else {
            ace.Mask != 0
        };
        if !trusted && unsafe_access {
            return Err(OwnerError::Unsafe(
                "owner permissions grant another account access".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_with_dacl(path: &Path, dacl: &str) {
        let sid = user_sid().unwrap();
        let descriptor = Descriptor::from_sddl(&format!("O:{sid}{dacl}")).unwrap();
        let name = wide(path.as_os_str());
        assert_ne!(
            unsafe { CreateDirectoryW(name.as_ptr(), &descriptor.attributes()) },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }

    fn assert_inherited_dacl(path: &Path) {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Security::INHERITED_ACE;

        let file = std::fs::OpenOptions::new()
            .access_mode(READ_CONTROL)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
            .unwrap();
        let mut dacl = null_mut();
        let mut descriptor = null_mut();
        assert_eq!(
            unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    &mut dacl,
                    null_mut(),
                    &mut descriptor,
                )
            },
            0
        );
        let _descriptor = Descriptor(descriptor);
        let mut control = 0;
        let mut revision = 0;
        assert_ne!(
            unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
            0
        );
        assert_eq!(control & SE_DACL_PROTECTED, 0);
        assert!(!dacl.is_null());
        assert!(unsafe { (*dacl).AceCount } > 0);
        for index in 0..unsafe { (*dacl).AceCount } as u32 {
            let mut raw = null_mut();
            assert_ne!(unsafe { GetAce(dacl, index, &mut raw) }, 0);
            assert_ne!(
                u32::from(unsafe { (*raw.cast::<ACE_HEADER>()).AceFlags }) & INHERITED_ACE,
                0
            );
        }
    }

    #[test]
    fn existing_inherited_owner_directories_keep_arbitration() {
        let data = tempfile::tempdir().unwrap();
        let sid = user_sid().unwrap();
        let profile = data.path().join("private-profile");
        create_with_dacl(
            &profile,
            &format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
        );
        // Reproduce the older lock-only implementation's ordinary directory
        // creation: neither tine nor owners receives an explicit private DACL.
        let owners = profile.join("tine/owners");
        std::fs::create_dir_all(&owners).unwrap();
        assert_inherited_dacl(owners.parent().unwrap());
        assert_inherited_dacl(&owners);
        let namespace = crate::owner::namespace::Namespace::at(owners.clone()).unwrap();
        let paths = namespace.graph(Path::new("graph")).unwrap();
        let lock = crate::owner::lock::OwnerLock::try_take(&paths.lock, "app")
            .unwrap()
            .expect("an existing safe namespace must retain owner arbitration");
        assert!(
            crate::owner::lock::OwnerLock::try_take(&paths.lock, "serve")
                .unwrap()
                .is_none()
        );
        drop(lock);
        // Checking a safe inherited directory does not rewrite its ACL.
        assert_inherited_dacl(&owners);
    }

    #[test]
    fn new_locks_are_private_even_in_a_directory_with_inheritable_foreign_read() {
        let data = tempfile::tempdir().unwrap();
        let sid = user_sid().unwrap();
        let owners = data.path().join("tine-owners");
        create_with_dacl(&owners, &format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FR;;;WD)"));
        check_directory(&owners).unwrap();
        let path = owners.join("graph.lock");
        let lock = crate::owner::lock::OwnerLock::try_take(&path, "app")
            .unwrap()
            .unwrap();
        let file = std::fs::File::open(&path).unwrap();
        // The pipe-level check requires a protected DACL and no foreign grant.
        check_handle(file.as_raw_handle(), false).unwrap();
        assert!(crate::owner::lock::OwnerLock::try_take(&path, "serve")
            .unwrap()
            .is_none());
        drop(lock);
        assert!(crate::owner::lock::OwnerLock::try_take(&path, "serve")
            .unwrap()
            .is_some());

        let existing = owners.join("existing.lock");
        std::fs::write(&existing, b"previous lock file").unwrap();
        let before = std::fs::File::open(&existing).unwrap();
        assert!(check_handle(before.as_raw_handle(), false).is_err());
        let reopened = open_lock_file(&existing).unwrap();
        assert!(
            check_handle(reopened.as_raw_handle(), false).is_err(),
            "existing DACL must not be rewritten"
        );
    }

    #[test]
    fn only_inherit_only_creator_owner_grants_are_safe_on_directories() {
        let data = tempfile::tempdir().unwrap();
        let sid = user_sid().unwrap();
        for (name, principal, flags, safe) in [
            ("creator-owner-inherit-only", "CO", "OICIIO", true),
            ("creator-owner-effective", "CO", "OICI", false),
            ("creator-group-inherit-only", "CG", "OICIIO", false),
        ] {
            let path = data.path().join(name);
            create_with_dacl(
                &path,
                &format!("D:P(A;OICI;FA;;;{sid})(A;{flags};GA;;;{principal})"),
            );
            let result = check_directory(&path);
            if safe {
                result.unwrap();
            } else {
                assert!(matches!(result, Err(OwnerError::Unsafe(_))), "{name}");
            }
        }
    }

    #[test]
    fn inherited_everyone_or_users_write_grants_refuse() {
        let data = tempfile::tempdir().unwrap();
        let sid = user_sid().unwrap();
        for principal in ["WD", "BU"] {
            let profile = data.path().join(principal);
            create_with_dacl(
                &profile,
                &format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FW;;;{principal})"),
            );
            let owners = profile.join("tine/owners");
            std::fs::create_dir_all(&owners).unwrap();
            assert_inherited_dacl(&owners);
            assert!(matches!(
                check_directory(&owners),
                Err(OwnerError::Unsafe(_))
            ));
            assert!(matches!(
                crate::owner::namespace::Namespace::at(owners),
                Err(OwnerError::Unsafe(_))
            ));
        }
    }

    #[test]
    fn directory_checks_reject_foreign_write_delete_and_null_dacl() {
        let data = tempfile::tempdir().unwrap();
        let sid = user_sid().unwrap();
        for (index, acl) in [
            format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FW;;;WD)"),
            format!("D:P(A;OICI;FA;;;{sid})(A;;0x40;;;WD)"), // delete-child
            format!("D:P(A;OICI;FA;;;{sid})(A;;WD;;;WD)"),   // WRITE_DAC
            "D:NO_ACCESS_CONTROL".into(),                    // NULL DACL grants everyone access
        ]
        .iter()
        .enumerate()
        {
            let path = data.path().join(format!("unsafe-{index}"));
            let descriptor = Descriptor::from_sddl(&format!("O:{sid}{acl}")).unwrap();
            let name = wide(path.as_os_str());
            assert_ne!(
                unsafe { CreateDirectoryW(name.as_ptr(), &descriptor.attributes()) },
                0
            );
            assert!(
                matches!(check_directory(&path), Err(OwnerError::Unsafe(_))),
                "{acl}"
            );
        }
        let private = data.path().join("private");
        create_directory(&private).unwrap();
        check_directory(&private).unwrap();
        // The protocol permits directory read/execute rights; only a pipe
        // must exclude every grant to another account.
        let readable = data.path().join("readable");
        let descriptor =
            Descriptor::from_sddl(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;;GRGX;;;WD)")).unwrap();
        let name = wide(readable.as_os_str());
        assert_ne!(
            unsafe { CreateDirectoryW(name.as_ptr(), &descriptor.attributes()) },
            0
        );
        check_directory(&readable).unwrap();
    }

    #[test]
    fn namespace_rechecks_an_existing_directory_after_its_dacl_is_loosened() {
        use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
        use windows_sys::Win32::Security::{
            GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        };
        let data = tempfile::tempdir().unwrap();
        let owners = data.path().join("tine-owners");
        let namespace = crate::owner::namespace::Namespace::at(owners.clone()).unwrap();
        let sid = user_sid().unwrap();
        let descriptor =
            Descriptor::from_sddl(&format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;;FW;;;WD)")).unwrap();
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl = null_mut();
        assert_ne!(
            unsafe {
                GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted)
            },
            0
        );
        let name = wide(owners.as_os_str());
        assert_eq!(
            unsafe {
                SetNamedSecurityInfoW(
                    name.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                    null_mut(),
                    null_mut(),
                    dacl,
                    null_mut(),
                )
            },
            0
        );
        assert!(matches!(
            namespace.graph(Path::new("graph")),
            Err(OwnerError::Unsafe(_))
        ));
        assert!(matches!(
            crate::owner::namespace::Namespace::at(owners),
            Err(OwnerError::Unsafe(_))
        ));
    }
}
