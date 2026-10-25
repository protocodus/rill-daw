//! Bounded card previews. File access and PNG decoding stay off the UI thread.

use eframe::egui;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
};

const MAX_ENTRIES: usize = 64;
const MAX_JOBS: usize = 2;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// Cloned library UI snapshots share jobs and textures rather than duplicating
/// the queue. At most two decodes and 64 card-sized GPU textures are retained.
#[derive(Clone, Default)]
pub(crate) struct PreviewCache {
    inner: Arc<Mutex<Cache>>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    jobs: Vec<Job>,
    clock: u64,
    next_token: u64,
}

struct Entry {
    path: PathBuf,
    token: u64,
    touched: u64,
    // None covers both a pending decode and a memoized failure. Only a new path
    // or an explicit capture/refresh invalidation retries failed files.
    texture: Option<egui::TextureHandle>,
}

struct Job {
    fingerprint: String,
    token: u64,
    result: mpsc::Receiver<Option<egui::ColorImage>>,
}

impl PreviewCache {
    /// Request only cards intersecting the viewport. A cache miss starts work
    /// when a decode slot is available; there is no unbounded offscreen queue.
    pub(crate) fn request(
        &self,
        ctx: &egui::Context,
        fingerprint: &str,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        let mut cache = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        cache.poll(ctx);
        cache.clock = cache.clock.wrapping_add(1);
        let touched = cache.clock;
        if let Some(entry) = cache.entries.get_mut(fingerprint) {
            if entry.path == path {
                entry.touched = touched;
                return entry.texture.clone();
            }
            cache.entries.remove(fingerprint);
        }
        if cache.jobs.len() >= MAX_JOBS {
            return None;
        }
        if cache.entries.len() >= MAX_ENTRIES
            && let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| key.clone())
        {
            cache.entries.remove(&oldest);
        }
        cache.next_token = cache.next_token.wrapping_add(1);
        let token = cache.next_token;
        cache.entries.insert(
            fingerprint.to_owned(),
            Entry {
                path: path.to_owned(),
                token,
                touched,
                texture: None,
            },
        );
        let (sender, result) = mpsc::sync_channel(1);
        let path = path.to_owned();
        let repaint = ctx.clone();
        if std::thread::Builder::new()
            .name("rill-card-preview".into())
            .spawn(move || {
                let image = decode(&path);
                let _ = sender.send(image);
                repaint.request_repaint();
            })
            .is_ok()
        {
            cache.jobs.push(Job {
                fingerprint: fingerprint.to_owned(),
                token,
                result,
            });
        }
        None
    }

    /// A capture can replace the PNG without changing its fingerprint or path.
    /// Tokens prevent a decode started before refresh from publishing stale data.
    pub(crate) fn invalidate(&self, fingerprint: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entries
            .remove(fingerprint);
    }
}

impl Cache {
    fn poll(&mut self, ctx: &egui::Context) {
        let mut index = 0;
        while index < self.jobs.len() {
            let image = match self.jobs[index].result.try_recv() {
                Ok(image) => image,
                Err(mpsc::TryRecvError::Disconnected) => None,
                Err(mpsc::TryRecvError::Empty) => {
                    index += 1;
                    continue;
                }
            };
            let job = self.jobs.swap_remove(index);
            if let Some(entry) = self.entries.get_mut(&job.fingerprint)
                && entry.token == job.token
            {
                entry.texture = image.map(|image| {
                    ctx.load_texture(
                        format!("device-card-{}-{}", job.fingerprint, job.token),
                        image,
                        egui::TextureOptions::LINEAR,
                    )
                });
            }
        }
    }
}

fn decode(path: &Path) -> Option<egui::ColorImage> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_FILE_BYTES {
        return None;
    }
    let mut reader = image::ImageReader::open(path).ok()?;
    reader.set_format(image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(MAX_FILE_BYTES);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(512, 320).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn png(path: &Path, width: u32, height: u32) {
        image::RgbaImage::from_pixel(width, height, image::Rgba([40, 80, 160, 255]))
            .save(path)
            .unwrap();
    }

    fn loaded(
        cache: &PreviewCache,
        ctx: &egui::Context,
        key: &str,
        path: &Path,
    ) -> egui::TextureHandle {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(texture) = cache.request(ctx, key, path) {
                return texture;
            }
            assert!(Instant::now() < deadline, "Preview did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn decodes_card_sized_images_and_memoizes_failure_until_refresh() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.png");
        let cache = PreviewCache::default();
        let ctx = egui::Context::default();
        std::fs::write(&path, b"broken png").unwrap();
        assert!(cache.request(&ctx, "device", &path).is_none());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cache.inner.lock().unwrap().jobs.is_empty() {
            assert!(Instant::now() < deadline);
            assert!(cache.request(&ctx, "device", &path).is_none());
            std::thread::sleep(Duration::from_millis(2));
        }
        png(&path, 1024, 1024);
        for _ in 0..20 {
            assert!(cache.request(&ctx, "device", &path).is_none());
        }
        assert!(
            cache.inner.lock().unwrap().jobs.is_empty(),
            "Failure must not be decoded every frame"
        );
        cache.invalidate("device");
        assert_eq!(loaded(&cache, &ctx, "device", &path).size(), [320, 320]);
        let other = directory.path().join("replacement.png");
        png(&other, 640, 200);
        assert_eq!(loaded(&cache, &ctx, "device", &other).size(), [512, 160]);
    }

    #[test]
    fn clones_share_two_job_limit_and_refresh_discards_old_completions() {
        let cache = PreviewCache::default();
        let clone = cache.clone();
        let ctx = egui::Context::default();
        let mut senders = Vec::new();
        {
            let mut inner = cache.inner.lock().unwrap();
            for token in 1..=2 {
                let (sender, result) = mpsc::sync_channel(1);
                senders.push(sender);
                let key = format!("device-{token}");
                inner.entries.insert(
                    key.clone(),
                    Entry {
                        path: PathBuf::from("preview.png"),
                        token,
                        touched: token,
                        texture: None,
                    },
                );
                inner.jobs.push(Job {
                    fingerprint: key,
                    token,
                    result,
                });
            }
            inner.next_token = 2;
        }
        assert!(
            clone
                .request(&ctx, "third", Path::new("preview.png"))
                .is_none()
        );
        assert_eq!(cache.inner.lock().unwrap().jobs.len(), 2);
        assert!(!cache.inner.lock().unwrap().entries.contains_key("third"));
        clone.invalidate("device-1");
        senders[0]
            .send(Some(egui::ColorImage::new(
                [7, 5],
                vec![egui::Color32::RED; 35],
            )))
            .unwrap();
        {
            let mut inner = cache.inner.lock().unwrap();
            inner.entries.insert(
                "device-1".into(),
                Entry {
                    path: PathBuf::from("preview.png"),
                    token: 3,
                    touched: 3,
                    texture: None,
                },
            );
            inner.poll(&ctx);
            assert!(
                inner.entries["device-1"].texture.is_none(),
                "Stale image crossed refresh generation"
            );
            assert_eq!(inner.jobs.len(), 1);
        }
    }

    #[test]
    fn cache_evicts_one_old_entry_and_keeps_recent_visible_textures() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preview.png");
        png(&path, 8, 5);
        let cache = PreviewCache::default();
        let ctx = egui::Context::default();
        for index in 0..MAX_ENTRIES {
            let _ = loaded(&cache, &ctx, &format!("device-{index}"), &path);
        }
        let recent = cache.request(&ctx, "device-0", &path).unwrap();
        let _ = loaded(&cache, &ctx, "new-device", &path);
        let inner = cache.inner.lock().unwrap();
        assert_eq!(inner.entries.len(), MAX_ENTRIES);
        assert!(!inner.entries.contains_key("device-1"));
        assert_eq!(
            inner.entries["device-0"].texture.as_ref().unwrap().id(),
            recent.id()
        );
        assert_eq!(inner.jobs.len(), 0);
    }
}
