use bg_spec_core::Config;
use std::{fs, path::Path};
use tempfile::TempDir;
use walkdir::WalkDir;

pub fn fixture() -> (TempDir, Config) {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus = temp.path().join("corpus");
    for entry in WalkDir::new(root.join("corpus")) {
        let entry = entry.unwrap();
        let relative = entry.path().strip_prefix(root.join("corpus")).unwrap();
        let target = corpus.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(target).unwrap();
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
    let mut config = Config::load(&root.join("config/config.toml")).unwrap();
    config.corpus_dir = corpus;
    config.data_dir = temp.path().join("data");
    (temp, config)
}
