pub(crate) mod sandbox;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    write_atomically(path, contents.as_ref(), false)
}

/// Like [`atomic_write`], for files that hold credentials: the file is readable and writable
/// by its owner only, from the moment it exists.
pub fn atomic_write_private(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
    write_atomically(path, contents.as_ref(), true)
}

/// Removes group and other access from an existing file or directory. Does nothing where the
/// platform has no such permissions or the path does not exist.
pub fn restrict_to_owner(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = fs::metadata(path) {
            let mode = metadata.permissions().mode();
            if mode & 0o077 != 0 {
                let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o700));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn write_atomically(path: &Path, contents: &[u8], private: bool) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Path does not have a parent directory: {}", path.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to create directory {}: {error}", parent.display()))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("daily-state");
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut file = options.open(&temp_path).map_err(|error| {
            format!(
                "Failed to create temporary file {}: {error}",
                temp_path.display()
            )
        })?;
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!(
                    "Failed to write temporary file {}: {error}",
                    temp_path.display()
                )
            })?;
        drop(file);
        fs::rename(&temp_path, path).map_err(|error| {
            format!(
                "Failed to replace {} with {}: {error}",
                path.display(),
                temp_path.display()
            )
        })?;
        if let Ok(directory) = fs::File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub fn read_limited_utf8(path: &Path, limit: usize, label: &str) -> Result<String, String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("Failed to inspect {label} {}: {error}", path.display()))?;
    if metadata.len() > limit as u64 {
        return Err(format!(
            "{label} {} exceeds the {limit} byte limit.",
            path.display()
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| format!("Failed to read {label} {}: {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!(
            "{label} {} exceeds the {limit} byte limit.",
            path.display()
        ));
    }
    String::from_utf8(bytes)
        .map_err(|error| format!("{label} {} is not valid UTF-8: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn private_writes_and_restriction_leave_only_owner_access() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!(
            "daily-private-write-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let secret = root.join("settings.json");
        atomic_write_private(&secret, "{}").unwrap();
        assert_eq!(mode(&secret), 0o600);
        // Replacing a world-readable file leaves the private one in its place.
        fs::set_permissions(&secret, fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write_private(&secret, "{\"a\":1}").unwrap();
        assert_eq!(mode(&secret), 0o600);

        let existing = root.join("old.json");
        fs::write(&existing, "{}").unwrap();
        fs::set_permissions(&existing, fs::Permissions::from_mode(0o644)).unwrap();
        restrict_to_owner(&existing);
        assert_eq!(mode(&existing), 0o600);
        fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        restrict_to_owner(&root);
        assert_eq!(mode(&root), 0o700);
        restrict_to_owner(&root.join("missing"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_write_replaces_content_without_leaving_temp_files() {
        let root = std::env::temp_dir().join(format!(
            "daily-atomic-write-{}-{}",
            std::process::id(),
            NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let path = root.join("settings.json");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
