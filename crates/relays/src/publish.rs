//! Artefact publication to `file://` or `s3://` object stores.

use std::collections::HashMap;

use anyhow::{Context as _, Result, anyhow, bail};
use object_store::local::LocalFileSystem;
use object_store::path::Path;
use object_store::{
    Attribute, AttributeValue, Attributes, ObjectStore, ObjectStoreExt, PutOptions, PutPayload,
};
use sha2::{Digest, Sha256};

/// Cache-Control applied to published JSON artefacts on S3.
const CACHE_CONTROL: &str = "public, max-age=300, s-maxage=600, stale-while-revalidate=86400";

/// Publishes artefacts under a fixed key space, skipping uploads whose
/// SHA-256 matches the previously written content in this process.
#[derive(Debug)]
pub struct Publisher {
    store: Box<dyn ObjectStore>,
    /// Upload options for stores that support attributes (`LocalFileSystem`
    /// rejects them, so the decision is made once at construction).
    options: Option<PutOptions>,
    hashes: HashMap<String, [u8; 32]>,
}

impl Publisher {
    /// Open a store from a `file:///path` or `s3://bucket[/prefix]` URL.
    /// S3 credentials and configuration come from the environment.
    ///
    /// # Errors
    ///
    /// Returns an error for unknown schemes or unopenable stores.
    pub fn new(root: &str) -> Result<Self> {
        let url = url::Url::parse(root).with_context(|| format!("invalid store URL `{root}`"))?;
        match url.scheme() {
            "file" => {
                let root_path = url
                    .to_file_path()
                    .map_err(|()| anyhow!("store URL `{root}` is not a file path"))?;
                std::fs::create_dir_all(&root_path)
                    .context("failed to create local store directory")?;
                Ok(Self {
                    store: Box::new(
                        LocalFileSystem::new_with_prefix(root_path)
                            .context("failed to open local store")?,
                    ),
                    options: None,
                    hashes: HashMap::new(),
                })
            }
            "s3" => {
                let mut attributes = Attributes::new();
                attributes.insert(
                    Attribute::ContentType,
                    AttributeValue::from("application/json"),
                );
                attributes.insert(Attribute::CacheControl, AttributeValue::from(CACHE_CONTROL));
                Ok(Self {
                    store: Box::new(
                        object_store::aws::AmazonS3Builder::from_env()
                            .with_url(root)
                            .build()
                            .context("failed to configure S3 store")?,
                    ),
                    options: Some(PutOptions {
                        attributes,
                        ..PutOptions::default()
                    }),
                    hashes: HashMap::new(),
                })
            }
            scheme => bail!("unsupported store scheme `{scheme}` (expected file:// or s3://)"),
        }
    }

    /// Write `body` under `key`, unless identical content was already
    /// written through this publisher. Returns `true` when an upload
    /// happened.
    ///
    /// # Errors
    ///
    /// Returns an error when the store write fails.
    pub async fn put(&mut self, key: &str, body: Vec<u8>) -> Result<bool> {
        let digest: [u8; 32] = Sha256::digest(&body).into();
        if self.hashes.get(key) == Some(&digest) {
            return Ok(false);
        }
        let path = Path::from(key);
        let payload = PutPayload::from(body);
        match &self.options {
            Some(options) => {
                self.store
                    .put_opts(&path, payload, options.clone())
                    .await
                    .with_context(|| format!("failed to write {key}"))?;
            }
            None => {
                self.store
                    .put(&path, payload)
                    .await
                    .with_context(|| format!("failed to write {key}"))?;
            }
        }
        self.hashes.insert(key.to_owned(), digest);
        Ok(true)
    }
}
