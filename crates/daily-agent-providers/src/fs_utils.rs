use std::{fs, path::Path};
pub(crate) fn read_limited_utf8(path: &Path, limit: usize, label: &str) -> Result<String, String> {
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
