//! Object storage, behind a trait, with a filesystem implementation
//! (`docs/28` §Local deployment, `docs/48` Profile 1).
//!
//! # The failure this prevents
//!
//! `docs/28`: "**The pipeline is identical** — same states, same handshake,
//! same scan step. A deployment profile must not change the security model, or
//! the small profile becomes the insecure one."
//!
//! That is the whole reason this is a trait. The temptation on a single-node
//! profile is to skip the handshake — write the bytes through the API, mark the
//! row committed, serve it from the app origin — and every one of those
//! shortcuts removes a control. With one trait, the filesystem backend does the
//! same pre-sign, the same `head`, the same separate-origin download as S3, and
//! there is no code path that treats "small" as "trusted".
//!
//! # Why the API never streams bytes through itself
//!
//! `docs/28` opens with it: files never pass through the API process's memory.
//! [`ObjectStore`] has no `put` taking a body — the only way to write an object
//! is for the client to upload to the URL [`ObjectStore::presign_put`] mints.
//! A method that accepted bytes would be the one every future caller reached
//! for, and a 2 GB upload would then cost 2 GB of API memory.
//!
//! The filesystem backend still has to receive those bytes somewhere; it does
//! it in a dedicated handler on the attachment origin that streams with bounded
//! buffers, which is a different process concern from the API's permission
//! layer even when it is the same binary.

use std::future::Future;
use std::path::{Component, Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

/// What a stored object looks like from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectHead {
    pub byte_size: i64,
}

/// Why a storage operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("object not found")]
    NotFound,
    /// The key escaped its prefix, or was otherwise not addressable. Never
    /// surfaced to a client with detail: a key is not something a client names.
    #[error("invalid object key")]
    InvalidKey,
    #[error("storage backend failed: {0}")]
    Backend(String),
}

/// A boxed future, so the trait stays object-safe.
///
/// `Arc<dyn ObjectStore>` is what `AppState` carries — the same shape as
/// `Arc<dyn Mailer>` — so the backend is chosen once at startup from
/// `TF_STORAGE_BACKEND` and never branched on again.
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, StorageError>> + Send + 'a>>;

/// The seam between the pipeline and where bytes actually live.
pub trait ObjectStore: Send + Sync + std::fmt::Debug {
    /// A URL the client may `PUT` exactly this object to, valid for `ttl`.
    ///
    /// The returned URL must encode the key and the expiry such that a client
    /// cannot alter either — a client that could edit the key could write into
    /// another tenant's prefix.
    fn presign_put(&self, key: &str, ttl: Duration) -> Result<String, StorageError>;

    /// A URL the client may `GET` this object from, valid for `ttl`.
    ///
    /// Served from the attachment origin, never the application's
    /// (`docs/28` §Serving downloads).
    fn presign_get(&self, key: &str, ttl: Duration) -> Result<String, StorageError>;

    /// Size of the stored object, or [`StorageError::NotFound`].
    ///
    /// This is the commit step's evidence that an upload actually happened
    /// (`docs/28` step 3), which is why it is a `HEAD` and not a read.
    fn head<'a>(&'a self, key: &'a str) -> Fut<'a, ObjectHead>;

    /// The first `len` bytes, for the magic-byte sniff at commit.
    ///
    /// Bounded on purpose: this is the **only** method that returns file
    /// content, and it returns a prefix rather than a body so a large file
    /// cannot be pulled into the API process.
    fn read_prefix<'a>(&'a self, key: &'a str, len: usize) -> Fut<'a, Vec<u8>>;

    /// Remove an object. Idempotent — removing an absent object succeeds,
    /// because the sweeper and the infected path both re-run.
    fn delete<'a>(&'a self, key: &'a str) -> Fut<'a, ()>;

    /// Append `chunk` to the object at `key`, creating it if absent.
    ///
    /// # Why the trait needed a write at all
    ///
    /// Every other method here serves the attachment pipeline (`docs/28`),
    /// where the *client* uploads through a presigned URL and the server never
    /// touches the bytes. An export inverts that: the server generates the
    /// artefact, and `docs/38` requires it to stream "straight to object
    /// storage" so the API and worker processes never hold the result set.
    ///
    /// Append rather than `put(key, whole_body)` for exactly that reason. A
    /// single-shot put would mean building a 200,000-row file in memory first,
    /// which is the bound this method exists to avoid.
    ///
    /// **The cost, stated:** S3 has no append. A future S3 backend implements
    /// this as multipart upload, which imposes a 5 MiB minimum on every part
    /// but the last — so it will have to buffer to that size internally rather
    /// than issuing one request per call. That is a backend concern and it is
    /// written down here so the next implementer meets it in the contract
    /// rather than in production.
    fn append<'a>(&'a self, key: &'a str, chunk: &'a [u8]) -> Fut<'a, ()>;
}

/// The filesystem backend (`TF_STORAGE_BACKEND=fs`).
///
/// Objects live at `{root}/{key}`, and the key is `{workspace}/{task}/{id}` —
/// three UUIDs, so the tree is naturally partitioned by tenant and a directory
/// listing never crosses one.
#[derive(Debug, Clone)]
pub struct FilesystemStore {
    root: PathBuf,
    /// The origin signed URLs are issued against — `TF_ATTACHMENT_ORIGIN`,
    /// which config refuses to let equal `TF_PUBLIC_URL`.
    origin: String,
    /// Signs the URLs. The same `TF_SECRET_KEY` the CSRF token binds with.
    secret: String,
}

impl FilesystemStore {
    #[must_use]
    pub fn new(root: PathBuf, origin: String, secret: String) -> Self {
        Self {
            root,
            origin,
            secret,
        }
    }

    /// Resolve a key to a path, refusing anything that escapes the root.
    ///
    /// The key is built from UUIDs by `casual-task-attachment::object_key`, so
    /// this should never fire. It exists because "should never" is not a
    /// mechanism: if a key ever reaches here from somewhere else, the failure
    /// must be a refusal and not a write outside the tree.
    fn path_of(&self, key: &str) -> Result<PathBuf, StorageError> {
        let candidate = Path::new(key);
        let escapes = candidate
            .components()
            .any(|component| !matches!(component, Component::Normal(_)));
        if key.is_empty() || escapes {
            return Err(StorageError::InvalidKey);
        }
        Ok(self.root.join(candidate))
    }

    /// The signature over `(key, expiry)`.
    ///
    /// Keyed, so a client cannot mint its own URL or extend one: the expiry is
    /// inside the signed material, which is what stops "change the timestamp
    /// and keep using it".
    #[must_use]
    pub fn sign(secret: &str, key: &str, expires_at: i64, method: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        // Length-prefixed so ("a","bc") and ("ab","c") cannot collide into the
        // same signature — the same reason the idempotency hash does it.
        for part in [secret, method, key, &expires_at.to_string()] {
            hasher.update(u32::try_from(part.len()).unwrap_or(u32::MAX).to_be_bytes());
            hasher.update(part.as_bytes());
        }
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    /// Verify a signature in constant time.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidKey`] when the signature does not match or the
    /// URL has expired.
    pub fn verify(
        secret: &str,
        key: &str,
        expires_at: i64,
        method: &str,
        presented: &str,
        now: i64,
    ) -> Result<(), StorageError> {
        if now > expires_at {
            return Err(StorageError::InvalidKey);
        }
        let expected = Self::sign(secret, key, expires_at, method);
        // Constant time: a byte-by-byte comparison leaks the prefix length,
        // which is enough to forge a signature one character at a time.
        use subtle::ConstantTimeEq;
        if expected.as_bytes().ct_eq(presented.as_bytes()).into() {
            Ok(())
        } else {
            Err(StorageError::InvalidKey)
        }
    }

    fn url(&self, key: &str, ttl: Duration, method: &str) -> Result<String, StorageError> {
        let expires_at = now_unix()
            .checked_add(i64::try_from(ttl.as_secs()).unwrap_or(0))
            .ok_or(StorageError::InvalidKey)?;
        let signature = Self::sign(&self.secret, key, expires_at, method);
        Ok(format!(
            "{}/attachments/{key}?expires={expires_at}&signature={signature}",
            self.origin.trim_end_matches('/')
        ))
    }

    /// Where an object lives on disk. For the streaming handler.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidKey`] if the key escapes the root.
    pub fn resolve(&self, key: &str) -> Result<PathBuf, StorageError> {
        self.path_of(key)
    }

    /// Write an object, replacing whatever was there.
    ///
    /// # Why this is not [`ObjectStore::append`]
    ///
    /// An upload is idempotent by nature: a client that retries a `PUT` whose
    /// response it never saw is doing the right thing, and appending would
    /// double the bytes. The only thing that would notice is `commit`'s size
    /// check, so the symptom would be a refusal for a correct client. `append`
    /// exists for exports, which genuinely accumulate.
    ///
    /// Not on the [`ObjectStore`] trait: S3 has no equivalent to serve, because
    /// with S3 the *bucket* takes the `PUT` and this process never sees the
    /// bytes at all. It is the filesystem backend standing in for one.
    ///
    /// # Errors
    ///
    /// [`StorageError::InvalidKey`] if the key escapes the root, or
    /// [`StorageError::Backend`] for any I/O failure.
    pub async fn replace(&self, key: &str, bytes: &[u8]) -> Result<(), StorageError> {
        use tokio::io::AsyncWriteExt;
        let path = self.path_of(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| StorageError::Backend(error.to_string()))?;
        }
        let mut file = tokio::fs::File::create(&path)
            .await
            .map_err(|error| StorageError::Backend(error.to_string()))?;
        file.write_all(bytes)
            .await
            .map_err(|error| StorageError::Backend(error.to_string()))?;
        file.flush()
            .await
            .map_err(|error| StorageError::Backend(error.to_string()))?;
        Ok(())
    }
}

/// Seconds since the epoch.
fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

impl ObjectStore for FilesystemStore {
    fn presign_put(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        self.url(key, ttl, "PUT")
    }

    fn presign_get(&self, key: &str, ttl: Duration) -> Result<String, StorageError> {
        self.url(key, ttl, "GET")
    }

    fn head<'a>(&'a self, key: &'a str) -> Fut<'a, ObjectHead> {
        Box::pin(async move {
            let path = self.path_of(key)?;
            match tokio::fs::metadata(&path).await {
                Ok(meta) => Ok(ObjectHead {
                    byte_size: i64::try_from(meta.len()).unwrap_or(i64::MAX),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Err(StorageError::NotFound)
                }
                Err(error) => Err(StorageError::Backend(error.to_string())),
            }
        })
    }

    fn append<'a>(&'a self, key: &'a str, chunk: &'a [u8]) -> Fut<'a, ()> {
        Box::pin(async move {
            use tokio::io::AsyncWriteExt;
            let path = self.path_of(key)?;
            // The key is validated by `path_of`, but its parent directories are
            // this backend's own layout and may not exist yet. Created here
            // rather than at job start so a caller cannot forget.
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|error| StorageError::Backend(error.to_string()))?;
            }
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|error| StorageError::Backend(error.to_string()))?;
            file.write_all(chunk)
                .await
                .map_err(|error| StorageError::Backend(error.to_string()))?;
            // Flushed per chunk, not per file: a worker killed mid-export must
            // leave a partial artefact that `head` reports honestly, rather
            // than a file whose size depends on what the OS happened to flush.
            file.flush()
                .await
                .map_err(|error| StorageError::Backend(error.to_string()))?;
            Ok(())
        })
    }

    fn read_prefix<'a>(&'a self, key: &'a str, len: usize) -> Fut<'a, Vec<u8>> {
        Box::pin(async move {
            use tokio::io::AsyncReadExt;
            let path = self.path_of(key)?;
            let file = tokio::fs::File::open(&path).await.map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    StorageError::NotFound
                } else {
                    StorageError::Backend(error.to_string())
                }
            })?;
            // `take` bounds the read at the source: a caller cannot ask for a
            // prefix and receive a file.
            let mut buffer = Vec::with_capacity(len);
            file.take(len as u64)
                .read_to_end(&mut buffer)
                .await
                .map_err(|error| StorageError::Backend(error.to_string()))?;
            Ok(buffer)
        })
    }

    fn delete<'a>(&'a self, key: &'a str) -> Fut<'a, ()> {
        Box::pin(async move {
            let path = self.path_of(key)?;
            match tokio::fs::remove_file(&path).await {
                Ok(()) => Ok(()),
                // Idempotent: the sweeper and the infected path both re-run.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(StorageError::Backend(error.to_string())),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> FilesystemStore {
        FilesystemStore::new(
            PathBuf::from("/tmp/tf-test-objects"),
            "https://files.example.test".to_owned(),
            "a-secret-long-enough".to_owned(),
        )
    }

    #[test]
    fn a_key_cannot_escape_the_root() {
        // The keys this system mints are three UUIDs, so this should be
        // unreachable — which is exactly why it is asserted rather than assumed.
        let store = store();
        for key in ["../escape", "a/../../b", "/absolute", "", "./x"] {
            assert!(store.path_of(key).is_err(), "accepted {key:?}");
        }
        assert!(store.path_of("ws/task/att").is_ok());
    }

    #[test]
    fn a_signature_covers_the_key_the_expiry_and_the_method() {
        // Each one must change the signature, or it is not protected: a client
        // that could swap the key would read another object, one that could
        // move the expiry would hold a permanent URL, and one that could change
        // the method would turn a read grant into a write.
        let base = FilesystemStore::sign("s", "ws/t/a", 100, "GET");
        assert_ne!(base, FilesystemStore::sign("s", "ws/t/b", 100, "GET"));
        assert_ne!(base, FilesystemStore::sign("s", "ws/t/a", 101, "GET"));
        assert_ne!(base, FilesystemStore::sign("s", "ws/t/a", 100, "PUT"));
        assert_ne!(base, FilesystemStore::sign("t", "ws/t/a", 100, "GET"));
        assert_eq!(base, FilesystemStore::sign("s", "ws/t/a", 100, "GET"));
    }

    #[test]
    fn an_expired_or_forged_signature_is_refused() {
        let signature = FilesystemStore::sign("s", "ws/t/a", 100, "GET");
        assert!(FilesystemStore::verify("s", "ws/t/a", 100, "GET", &signature, 99).is_ok());
        // Expired.
        assert!(FilesystemStore::verify("s", "ws/t/a", 100, "GET", &signature, 101).is_err());
        // Forged.
        assert!(
            FilesystemStore::verify("s", "ws/t/a", 100, "GET", "0".repeat(64).as_str(), 99)
                .is_err()
        );
        // A GET signature does not authorise a PUT.
        assert!(FilesystemStore::verify("s", "ws/t/a", 100, "PUT", &signature, 99).is_err());
    }

    #[test]
    fn a_presigned_url_is_issued_against_the_attachment_origin() {
        // docs/28: "User content is served from a separate origin from the
        // application. This is the single most important control here."
        let url = store()
            .presign_get("ws/t/a", Duration::from_secs(300))
            .expect("signed");
        assert!(
            url.starts_with("https://files.example.test/attachments/ws/t/a?"),
            "{url}"
        );
        assert!(url.contains("signature="), "{url}");
        assert!(url.contains("expires="), "{url}");
    }

    #[test]
    fn the_trait_has_no_way_to_put_bytes_through_the_process() {
        // docs/28 opens with "files never pass through the API process's
        // memory". The guarantee is the ABSENCE of a method, so it is asserted
        // against the source: a `put` taking a body is what a future caller
        // would reach for.
        let source = include_str!("storage.rs");
        // Assembled rather than written as a literal: a literal needle appears
        // in this file and the check would match itself, which is the same
        // self-matching trap `casual-task-lint` hit with `OFFSET`.
        let needle = format!("fn {}(", "put");
        let declarations = source.matches(needle.as_str()).count();
        assert_eq!(
            declarations, 0,
            "ObjectStore grew a {needle} — uploads must go directly to storage, \
             or a 2 GB upload costs 2 GB of API memory"
        );
    }
}

#[cfg(test)]
mod append_tests {
    use super::*;

    fn store() -> FilesystemStore {
        let root = std::env::temp_dir().join(format!("tf-append-{}", uuid::Uuid::now_v7()));
        FilesystemStore::new(
            root,
            "https://files.example.test".to_owned(),
            "test-object-signing-secret".to_owned(),
        )
    }

    #[tokio::test]
    async fn appending_builds_a_file_without_holding_it_in_memory() {
        // The property an export depends on: the artefact grows a batch at a
        // time, so the process never holds the whole result set.
        let store = store();
        let key = format!("{}/export.csv", uuid::Uuid::now_v7());
        store.append(&key, b"one\n").await.expect("first chunk");
        store.append(&key, b"two\n").await.expect("second chunk");

        let head = store.head(&key).await.expect("the object exists");
        assert_eq!(head.byte_size, 8, "the chunks did not accumulate");
        let bytes = store.read_prefix(&key, 64).await.expect("readable");
        assert_eq!(String::from_utf8_lossy(&bytes), "one\ntwo\n");
    }

    #[tokio::test]
    async fn a_key_that_escapes_its_prefix_is_refused() {
        // The same guard the read paths have. A write is where it matters most:
        // a key that climbed out of the root would let an export overwrite
        // anything the process can write.
        let store = store();
        let escaped = store.append("../../etc/passwd", b"x").await;
        assert!(
            matches!(escaped, Err(StorageError::InvalidKey)),
            "a traversing key was accepted for writing"
        );
    }
}
