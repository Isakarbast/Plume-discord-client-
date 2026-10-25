//! Avatars, server icons, emoji and image attachments: fetched, decoded at
//! the size they're shown, and cached in memory (bounded) and on disk.
//!
//! Frontends get plain RGBA pixels ([`Image`]) and turn them into whatever
//! their toolkit wants (a `gdk::Texture` in GTK).

mod animated;
mod decode;
mod disk_cache;
mod fetch;
mod lru;
pub mod urls;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use thiserror::Error;
use tracing::debug;

pub use animated::{Frame, decode_animation};
pub use decode::{Image, decode};
use disk_cache::DiskCache;
use lru::ByteLru;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("download failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("not an image we can read: {0}")]
    Decode(#[from] image::ImageError),
    #[error("file is too big ({0} bytes)")]
    TooBig(usize),
    #[error("background task failed")]
    Task,
}

/// How much memory and disk the media cache may use.
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub memory_bytes: usize,
    /// `None` turns the disk cache off.
    pub disk: Option<(PathBuf, u64)>,
}

/// Shared by the whole app; cheap to clone.
#[derive(Clone)]
pub struct MediaStore {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    memory: Mutex<ByteLru<(String, u32), Arc<Image>>>,
    disk: Option<DiskCache>,
}

impl MediaStore {
    pub fn new(config: MediaConfig) -> Result<Self, MediaError> {
        Ok(MediaStore {
            inner: Arc::new(Inner {
                http: plume_rest::build_http_client()?,
                memory: Mutex::new(ByteLru::new(config.memory_bytes)),
                disk: config.disk.map(|(dir, max)| DiskCache::new(dir, max)),
            }),
        })
    }

    /// The image at `url`, scaled to fit in `max_size`×`max_size` pixels.
    pub async fn image(&self, url: &str, max_size: u32) -> Result<Arc<Image>, MediaError> {
        let key = (url.to_owned(), max_size);
        if let Some(image) = self.inner.memory.lock().expect("media lock").get(&key) {
            return Ok(image);
        }

        let bytes = match self.inner.disk.as_ref().and_then(|d| d.read(url)) {
            Some(bytes) => bytes,
            None => {
                debug!("fetching {url}");
                let bytes = fetch::fetch(&self.inner.http, url).await?;
                if let Some(disk) = &self.inner.disk {
                    disk.write(url, &bytes);
                }
                bytes
            }
        };

        // Decoding is CPU work, so keep it off the async threads.
        let image = tokio::task::spawn_blocking(move || decode(&bytes, max_size))
            .await
            .map_err(|_| MediaError::Task)??;
        let image = Arc::new(image);
        self.inner
            .memory
            .lock()
            .expect("media lock")
            .insert(key, image.clone(), image.byte_size());
        Ok(image)
    }

    /// Raw bytes (for animated images, which the caller decodes into frames).
    pub async fn bytes(&self, url: &str) -> Result<Vec<u8>, MediaError> {
        if let Some(bytes) = self.inner.disk.as_ref().and_then(|d| d.read(url)) {
            return Ok(bytes);
        }
        let bytes = fetch::fetch(&self.inner.http, url).await?;
        if let Some(disk) = &self.inner.disk {
            disk.write(url, &bytes);
        }
        Ok(bytes)
    }

    /// Memory the decoded images use right now, in bytes.
    pub fn memory_used(&self) -> usize {
        self.inner.memory.lock().expect("media lock").used()
    }

    /// Trims the disk cache to its limit. Run it now and then (e.g. at startup).
    pub fn prune_disk(&self) {
        if let Some(disk) = &self.inner.disk {
            disk.prune();
        }
    }
}
