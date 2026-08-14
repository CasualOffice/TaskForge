use super::*;

fn with(extra: &[(&'static str, &'static str)]) -> Result<Config, ConfigError> {
    let base: Vec<(&'static str, &'static str)> = vec![
        ("DATABASE_URL", "postgres://localhost/tf"),
        ("TF_SECRET_KEY", "a-secret-key-long-enough-for-the-check"),
        ("TF_PUBLIC_URL", "https://tasks.example.com"),
        ("TF_ATTACHMENT_ORIGIN", "https://files.example.com"),
    ];
    Config::from_source(move |name| {
        extra
            .iter()
            .chain(base.iter())
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_owned())
    })
}

#[test]
fn the_backend_defaults_to_the_filesystem() {
    // docs/48: "TF_STORAGE_BACKEND fs | s3 (default fs)".
    let config = with(&[]).expect("defaults are valid");
    assert_eq!(config.storage.backend, StorageBackend::Filesystem);
    assert_eq!(config.storage.path, "./data/attachments");
}

#[test]
fn a_backend_this_build_does_not_have_refuses_to_start() {
    // The failure this parse exists for: `s3` is documented and not built,
    // so accepting it would store files on local disk while the operator
    // believed they were in a bucket.
    assert!(matches!(
        with(&[("TF_STORAGE_BACKEND", "s3")]).err(),
        Some(ConfigError::UnsupportedStorageBackend(_))
    ));
    assert!(with(&[("TF_STORAGE_BACKEND", "nonsense")]).is_err());
}

#[test]
fn an_empty_path_with_the_filesystem_backend_refuses_to_start() {
    assert!(matches!(
        with(&[("TF_STORAGE_PATH", "   ")]).err(),
        Some(ConfigError::MissingStoragePath)
    ));
}

#[test]
fn the_documented_spelling_is_accepted_case_insensitively() {
    assert!(with(&[("TF_STORAGE_BACKEND", "FS")]).is_ok());
    assert!(with(&[("TF_STORAGE_BACKEND", " fs ")]).is_ok());
}
