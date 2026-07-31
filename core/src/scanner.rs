use std::path::{Path, PathBuf};

const EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "bmp"];

pub fn list_wallpapers(folder: &Path) -> Vec<PathBuf> {
    let mut images: Vec<PathBuf> = match std::fs::read_dir(folder) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| is_supported_image(path))
            .collect(),
        Err(_) => Vec::new(),
    };
    images.sort();
    images
}

fn is_supported_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_only_top_level_supported_images_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.png"), b"x").unwrap();
        std::fs::write(dir.path().join("a.JPG"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("nested.png"), b"x").unwrap();

        let images = list_wallpapers(dir.path());

        assert_eq!(
            images,
            vec![dir.path().join("a.JPG"), dir.path().join("b.png")]
        );
    }

    #[test]
    fn returns_empty_vec_for_missing_folder() {
        let images = list_wallpapers(Path::new("/definitely/does/not/exist"));
        assert!(images.is_empty());
    }
}
