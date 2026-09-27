//! Write a file beside its destination and move it into place only when it
//! is complete.
//!
//! The temporary file lives in the destination's directory, so the final
//! `rename` stays on one filesystem and replaces the destination in a single
//! step (POSIX `rename`; `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on
//! Windows, which is what `std::fs::rename` uses). Until [`AtomicFile::commit`]
//! succeeds, an existing destination is untouched. Dropping or discarding an
//! uncommitted file deletes the temporary.
//!
//! Limits: the replaced file's permissions are not copied (the new file gets
//! the process defaults). On Windows the rename fails while another program
//! holds the destination open; the error is returned and the destination is
//! kept.

use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A temporary file that becomes `dest` on [`commit`](AtomicFile::commit).
#[derive(Debug)]
pub struct AtomicFile {
    dest: PathBuf,
    temp: PathBuf,
    file: Option<File>,
}

impl AtomicFile {
    /// Create the temporary file. Fails early, before any rendering work, if
    /// the destination's directory is missing or not writable, or if the
    /// destination is a directory.
    pub fn create(dest: &Path) -> io::Result<AtomicFile> {
        if dest.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::IsADirectory,
                "the destination is a directory",
            ));
        }
        let name = dest.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "the destination has no file name",
            )
        })?;
        let dir = match dest.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        let mut last = None;
        for _ in 0..16 {
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let mut temp_name = std::ffi::OsString::from(".");
            temp_name.push(name);
            temp_name.push(format!(".{}-{n}.pigment-tmp", std::process::id()));
            let temp = dir.join(temp_name);
            match OpenOptions::new().write(true).create_new(true).open(&temp) {
                Ok(file) => {
                    return Ok(AtomicFile {
                        dest: dest.to_path_buf(),
                        temp,
                        file: Some(file),
                    });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| io::Error::other("no free temporary name")))
    }

    pub fn destination(&self) -> &Path {
        &self.dest
    }

    /// Path of the temporary file (for tests and diagnostics).
    pub fn temp_path(&self) -> &Path {
        &self.temp
    }

    /// The open temporary file. `None` after commit or discard.
    pub fn file(&mut self) -> Option<&mut File> {
        self.file.as_mut()
    }

    /// Flush to stable storage and rename over the destination. Every other
    /// handle to the temporary file must already be closed (Windows cannot
    /// rename an open file). On failure the temporary is removed and the
    /// destination is unchanged.
    pub fn commit(mut self) -> io::Result<()> {
        let file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("already committed or discarded"))?;
        let result = file.sync_all().and_then(|()| {
            drop(file);
            fs::rename(&self.temp, &self.dest)
        });
        match result {
            // The file is in place; a failed directory sync only weakens
            // durability across a power cut, so it is not reported.
            Ok(()) => {
                let _ = sync_dir(&self.dest);
            }
            Err(_) => {
                let _ = fs::remove_file(&self.temp);
            }
        }
        // Renamed or removed: nothing for Drop to clean up.
        self.temp = PathBuf::new();
        result
    }

    /// Delete the temporary file. The destination is unchanged.
    pub fn discard(mut self) {
        self.remove_temp();
    }

    fn remove_temp(&mut self) {
        self.file = None;
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
            self.temp = PathBuf::new();
        }
    }
}

impl Drop for AtomicFile {
    fn drop(&mut self) {
        self.remove_temp();
    }
}

/// Persist the directory entry after a rename (POSIX). Windows has no
/// directory handle to sync; `MoveFileExW` is durable enough there.
#[cfg(unix)]
fn sync_dir(dest: &Path) -> io::Result<()> {
    let dir = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn commit_replaces_and_discard_keeps_the_destination() {
        let dir = TestDir::new("atomic");
        let dest = dir.path().join("out.bin");
        fs::write(&dest, b"old").unwrap();

        let mut f = AtomicFile::create(&dest).unwrap();
        f.file().unwrap().write_all(b"new").unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"old", "untouched until commit");
        let temp = f.temp_path().to_path_buf();
        assert!(temp.exists());
        f.discard();
        assert!(!temp.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"old");

        let mut f = AtomicFile::create(&dest).unwrap();
        f.file().unwrap().write_all(b"new").unwrap();
        f.commit().unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new");
        assert_eq!(dir.entries(), vec!["out.bin".to_string()]);
    }

    #[test]
    fn dropping_an_uncommitted_file_removes_it() {
        let dir = TestDir::new("atomic-drop");
        let dest = dir.path().join("ünïcødé 画.png");
        {
            let mut f = AtomicFile::create(&dest).unwrap();
            f.file().unwrap().write_all(b"partial").unwrap();
        }
        assert!(dir.entries().is_empty());
        assert!(!dest.exists());
    }

    #[test]
    fn bad_destinations_fail_before_writing() {
        let dir = TestDir::new("atomic-bad");
        assert!(AtomicFile::create(dir.path()).is_err(), "a directory");
        assert!(AtomicFile::create(&dir.path().join("missing/out.png")).is_err());
        assert!(dir.entries().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn read_only_directory_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TestDir::new("atomic-ro");
        let ro = dir.path().join("ro");
        fs::create_dir(&ro).unwrap();
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o555)).unwrap();
        let probe = ro.join("probe");
        // Root ignores directory permissions; the check is meaningless then.
        if fs::write(&probe, b"").is_ok() {
            fs::remove_file(&probe).unwrap();
        } else {
            let e = AtomicFile::create(&ro.join("out.png")).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
        }
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    }
}
