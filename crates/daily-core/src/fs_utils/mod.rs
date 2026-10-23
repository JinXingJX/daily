pub(crate) mod sandbox;

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> Result<(), String> {
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
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| {
                format!(
                    "Failed to create temporary file {}: {error}",
                    temp_path.display()
                )
            })?;
        file.write_all(contents.as_ref())
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!(
                    "Failed to write temporary file {}: {error}",
                    temp_path.display()
                )
            })?;
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
