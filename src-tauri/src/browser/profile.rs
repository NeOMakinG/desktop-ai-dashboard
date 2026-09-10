//! Only profile identity metadata lives here. WebKit owns its isolated website data.
//! Never read/copy another browser profile or repair/delete existing metadata.
use crate::types::{AppError, AppResult};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};
use uuid::Uuid;

fn error() -> AppError {
    AppError::new("browser_profile_unavailable", super::GATE_ERROR)
}

#[cfg(target_os = "macos")]
pub(super) fn load_or_create(app_data: &Path) -> AppResult<Uuid> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    if !app_data.is_absolute()
        || !fs::symlink_metadata(app_data)
            .map_err(|_| error())?
            .is_dir()
    {
        return Err(error());
    }
    let directory = app_data.join("owned-browser");
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(error()),
    }
    let metadata = fs::symlink_metadata(&directory).map_err(|_| error())?;
    // Refuse insecure/symlinked existing metadata rather than silently changing it.
    if !metadata.is_dir()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.uid() != unsafe { libc::geteuid() }
    {
        return Err(error());
    }
    let path = directory.join("profile-id");
    let read_existing = || {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
            .map_err(|_| error())?;
        let metadata = file.metadata().map_err(|_| error())?;
        if !metadata.is_file()
            || metadata.len() != 36
            || metadata.nlink() != 1
            || metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(error());
        }
        let mut text = String::new();
        (&mut file)
            .take(37)
            .read_to_string(&mut text)
            .map_err(|_| error())?;
        parse_id(&text)
    };
    match fs::symlink_metadata(&path) {
        Ok(_) => return read_existing(),
        Err(failure) if failure.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(error()),
    }
    let id = Uuid::new_v4();
    let temporary = directory.join(format!(".profile-{}", Uuid::new_v4()));
    let result = (|| {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| error())?;
        file.write_all(id.to_string().as_bytes())
            .map_err(|_| error())?;
        file.sync_all().map_err(|_| error())?;
        drop(file);
        let from = CString::new(temporary.as_os_str().as_bytes()).map_err(|_| error())?;
        let to = CString::new(path.as_os_str().as_bytes()).map_err(|_| error())?;
        let renamed = unsafe {
            libc::renameatx_np(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_EXCL,
            )
        };
        if renamed != 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
                return read_existing();
            }
            return Err(error());
        }
        fs::File::open(&directory)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| error())?;
        Ok(id)
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn parse_id(text: &str) -> AppResult<Uuid> {
    let id = Uuid::parse_str(text).map_err(|_| error())?;
    if text.len() != 36 || id.get_version_num() != 4 || id.to_string() != text {
        return Err(error());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_canonical_bounded_v4_only() {
        let id = Uuid::new_v4();
        assert_eq!(parse_id(&id.to_string()).unwrap(), id);
        for value in [
            "",
            "00000000-0000-0000-0000-000000000000",
            "not-a-uuid",
            &format!("{}\n", id),
            &id.to_string().to_uppercase(),
        ] {
            assert!(parse_id(value).is_err());
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn metadata_persists_and_corruption_is_not_repaired() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let first = load_or_create(root.path()).unwrap();
        assert_eq!(first, load_or_create(root.path()).unwrap());
        let path = root.path().join("owned-browser/profile-id");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::write(&path, "bad").unwrap();
        assert!(load_or_create(root.path()).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "bad");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn interrupted_temporary_creation_and_concurrent_creation_are_safe() {
        use std::os::unix::fs::DirBuilderExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("owned-browser");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .unwrap();
        fs::write(directory.join(".profile-abandoned"), "partial").unwrap();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = root.path().to_path_buf();
                std::thread::spawn(move || load_or_create(&path).unwrap())
            })
            .collect();
        let ids: Vec<_> = handles
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert!(ids.iter().all(|id| *id == ids[0]));
        assert_eq!(
            fs::read_to_string(directory.join("profile-id")).unwrap(),
            ids[0].to_string()
        );
        assert_eq!(
            fs::read_to_string(directory.join(".profile-abandoned")).unwrap(),
            "partial"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn symlinks_and_shared_metadata_fail_closed() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        symlink(other.path(), root.path().join("owned-browser")).unwrap();
        assert!(load_or_create(root.path()).is_err());
        let root = tempfile::tempdir().unwrap();
        load_or_create(root.path()).unwrap();
        let path = root.path().join("owned-browser/profile-id");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create(root.path()).is_err());
        fs::remove_file(&path).unwrap();
        symlink(other.path().join("unread"), &path).unwrap();
        assert!(load_or_create(root.path()).is_err());
    }
}
