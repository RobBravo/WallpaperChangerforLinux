use std::path::Path;

/// Writes `contents` to `path` atomically: writes to a sibling temp file first, then
/// renames it over `path`. A reader can never observe a partially-written or empty
/// file, because `rename` on Linux is atomic at the filesystem level - the file at
/// `path` is always either the complete old content or the complete new content,
/// never a torn mix of both.
///
/// The temp file's name includes this process's PID so two processes writing the
/// same `path` concurrently (e.g. the GUI's "Guardar" and the tray's pause toggle,
/// both targeting config.toml) never write into the same temp file.
pub fn atomic_write(path: &Path, contents: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp_path, contents)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_parent_dir_and_writes_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.toml");

        atomic_write(&path, "hello = 1").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello = 1");
    }

    #[test]
    fn atomic_write_replaces_existing_content_and_leaves_no_temp_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.toml");
        std::fs::write(&path, "old").unwrap();

        atomic_write(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let leftover: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftover.is_empty(), "temp file left behind: {leftover:?}");
    }
}
