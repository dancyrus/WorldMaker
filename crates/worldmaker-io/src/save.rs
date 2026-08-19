//! Save/load and export stubs. Real formats arrive with Phase 6 (painter and
//! branches) and Phase 7 (exports); these exist so call sites and error
//! handling can be built against stable signatures now.

use std::path::Path;

use worldmaker_core::FieldStore;

/// Save a world to disk. Not implemented until the painter phase needs it.
pub fn save_world(_fields: &FieldStore, _path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("saving worlds is not implemented yet (arrives with Phase 6)")
}

/// Load a world from disk. Not implemented until the painter phase needs it.
pub fn load_world(_path: &Path) -> anyhow::Result<FieldStore> {
    anyhow::bail!("loading worlds is not implemented yet (arrives with Phase 6)")
}

/// Export the visible map as an image. Arrives with Phase 7 (style & export).
pub fn export_png(_path: &Path) -> anyhow::Result<()> {
    anyhow::bail!("PNG export is not implemented yet (arrives with Phase 7)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stubs_fail_with_helpful_messages() {
        let err = save_world(&FieldStore::new(1), Path::new("x")).unwrap_err();
        assert!(err.to_string().contains("Phase 6"));
        assert!(load_world(Path::new("x")).is_err());
        assert!(export_png(Path::new("x")).is_err());
    }
}
