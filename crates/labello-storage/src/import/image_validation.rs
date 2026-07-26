use std::{
    collections::VecDeque, fs::File, io::BufReader, path::Path, sync::atomic::AtomicBool,
    time::Duration,
};

use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};
use parking_lot::{Condvar, Mutex};

use super::{
    source::{hash_file_cancellable, import_error, source_extension},
    types::{ImportLimits, RegisteredFile},
};
use crate::error::{PathIo, StorageError, StorageResult};

const MEMORY_WAIT_POLL: Duration = Duration::from_millis(25);

pub(super) struct DecodedImageMemoryLimiter {
    capacity: u64,
    state: Mutex<DecodedImageMemoryState>,
    changed: Condvar,
}

#[derive(Default)]
struct DecodedImageMemoryState {
    used: u64,
    next_waiter: u64,
    waiters: VecDeque<u64>,
}

struct DecodedImageMemoryPermit<'a> {
    limiter: &'a DecodedImageMemoryLimiter,
    bytes: u64,
}

impl DecodedImageMemoryLimiter {
    pub(super) fn new(capacity: u64) -> Self {
        Self {
            capacity,
            state: Mutex::new(DecodedImageMemoryState::default()),
            changed: Condvar::new(),
        }
    }

    fn acquire(
        &self,
        bytes: u64,
        cancelled: &AtomicBool,
    ) -> StorageResult<DecodedImageMemoryPermit<'_>> {
        if bytes > self.capacity {
            return Err(import_error(
                "image_decoded_memory_limit",
                "decoded image exceeds the shared memory budget",
            ));
        }
        let mut state = self.state.lock();
        let waiter = state.next_waiter;
        state.next_waiter = state.next_waiter.wrapping_add(1);
        state.waiters.push_back(waiter);
        loop {
            if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some(position) = state.waiters.iter().position(|queued| *queued == waiter) {
                    state.waiters.remove(position);
                }
                self.changed.notify_all();
                return Err(import_error(
                    "parser_cancelled",
                    "decoded image memory wait was cancelled",
                ));
            }
            if state.waiters.front() == Some(&waiter)
                && bytes <= self.capacity.saturating_sub(state.used)
            {
                state.waiters.pop_front();
                state.used += bytes;
                self.changed.notify_all();
                return Ok(DecodedImageMemoryPermit {
                    limiter: self,
                    bytes,
                });
            }
            self.changed.wait_for(&mut state, MEMORY_WAIT_POLL);
        }
    }

    #[cfg(test)]
    fn state(&self) -> (u64, usize) {
        let state = self.state.lock();
        (state.used, state.waiters.len())
    }
}

impl Drop for DecodedImageMemoryPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.limiter.state.lock();
        state.used -= self.bytes;
        self.limiter.changed.notify_all();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ValidatedImage {
    pub blake3: String,
    pub byte_size: u64,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
    pub extension: String,
}

pub(super) fn validate_image(
    path: &Path,
    source_name: &str,
    registered: &RegisteredFile,
    limits: &ImportLimits,
    decoded_memory: &DecodedImageMemoryLimiter,
    cancelled: &AtomicBool,
) -> StorageResult<ValidatedImage> {
    let metadata = std::fs::metadata(path).with_path(path)?;
    if metadata.len() != registered.byte_size {
        return Err(import_error(
            "source_file_size_mismatch",
            "staged source file size changed after verification",
        ));
    }
    if metadata.len() > limits.single_source_file_bytes {
        return Err(import_error(
            "image_encoded_bytes_limit",
            "image exceeds the encoded byte limit",
        ));
    }
    let reader = ImageReader::open(path)
        .map_err(|source| StorageError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .with_guessed_format()
        .map_err(|source| StorageError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let format = reader.format().ok_or_else(|| {
        import_error(
            "image_format_unsupported",
            "image format could not be identified",
        )
    })?;
    if !matches!(
        format,
        ImageFormat::Png
            | ImageFormat::Jpeg
            | ImageFormat::Gif
            | ImageFormat::WebP
            | ImageFormat::Bmp
    ) {
        return Err(import_error(
            "image_format_unsupported",
            "image format is not supported by the import profile",
        ));
    }
    let decoded_buffers = if format == ImageFormat::Gif { 2 } else { 1 };
    let reservation = limits
        .decoded_image_bytes
        .checked_mul(decoded_buffers)
        .and_then(|decoded| decoded.checked_add(metadata.len()))
        .ok_or_else(|| {
            import_error(
                "image_decoded_memory_limit",
                "image validation memory reservation overflowed",
            )
        })?;
    let permit = decoded_memory.acquire(reservation, cancelled)?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|source| StorageError::Image {
            path: path.to_path_buf(),
            source,
        })?;
    decoder
        .set_limits(decoder_limits(limits))
        .map_err(|source| StorageError::Image {
            path: path.to_path_buf(),
            source,
        })?;
    let (width, height) = decoder.dimensions();
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| import_error("image_pixel_limit", "decoded image dimensions overflow"))?;
    if width == 0 || height == 0 || pixels > limits.decoded_image_pixels {
        return Err(import_error(
            "image_pixel_limit",
            "decoded image exceeds the pixel limit",
        ));
    }
    let decoded_bytes = decoder.total_bytes();
    if decoded_bytes > limits.decoded_image_bytes {
        return Err(import_error(
            "image_decoded_bytes_limit",
            "decoded image exceeds the memory limit",
        ));
    }
    reject_animation(path, format, limits)?;
    let orientation = decoder
        .orientation()
        .map_err(|source| StorageError::Image {
            path: path.to_path_buf(),
            source,
        })?;
    if orientation != Orientation::NoTransforms {
        return Err(import_error(
            "image_exif_orientation",
            "non-identity image orientation is not supported",
        ));
    }
    image::DynamicImage::from_decoder(decoder).map_err(|source| StorageError::Image {
        path: path.to_path_buf(),
        source,
    })?;
    drop(permit);

    let (extension, media_type) = match format {
        ImageFormat::Png => ("png", "image/png"),
        ImageFormat::Jpeg => ("jpg", "image/jpeg"),
        ImageFormat::Gif => ("gif", "image/gif"),
        ImageFormat::WebP => ("webp", "image/webp"),
        ImageFormat::Bmp => ("bmp", "image/bmp"),
        _ => unreachable!("validated above"),
    };
    let declared = source_extension(source_name).unwrap_or_default();
    let extension_matches = match format {
        ImageFormat::Jpeg => matches!(declared.as_str(), "jpg" | "jpeg"),
        _ => declared == extension,
    };
    if !extension_matches {
        return Err(import_error(
            "image_extension_mismatch",
            "image extension does not match decoded format",
        ));
    }
    let blake3 = hash_file_cancellable(path, cancelled)?;
    if blake3 != registered.blake3 {
        return Err(import_error(
            "source_file_digest_mismatch",
            "staged source file changed during image validation",
        ));
    }
    Ok(ValidatedImage {
        blake3,
        byte_size: metadata.len(),
        width,
        height,
        media_type: media_type.to_string(),
        extension: extension.to_string(),
    })
}

fn decoder_limits(limits: &ImportLimits) -> image::Limits {
    let mut decoder_limits = image::Limits::default();
    decoder_limits.max_alloc = Some(limits.decoded_image_bytes);
    decoder_limits
}

fn reject_animation(path: &Path, format: ImageFormat, limits: &ImportLimits) -> StorageResult<()> {
    let animated = match format {
        ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(BufReader::new(
                File::open(path).with_path(path)?,
            ))
            .map_err(|source| StorageError::Image {
                path: path.to_path_buf(),
                source,
            })?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(|source| StorageError::Image {
                    path: path.to_path_buf(),
                    source,
                })?;
            decoder.into_frames().take(2).count() > 1
        }
        ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(BufReader::new(
                File::open(path).with_path(path)?,
            ))
            .map_err(|source| StorageError::Image {
                path: path.to_path_buf(),
                source,
            })?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(|source| StorageError::Image {
                    path: path.to_path_buf(),
                    source,
                })?;
            decoder.has_animation()
        }
        ImageFormat::Png => {
            let mut decoder = image::codecs::png::PngDecoder::new(BufReader::new(
                File::open(path).with_path(path)?,
            ))
            .map_err(|source| StorageError::Image {
                path: path.to_path_buf(),
                source,
            })?;
            decoder
                .set_limits(decoder_limits(limits))
                .map_err(|source| StorageError::Image {
                    path: path.to_path_buf(),
                    source,
                })?;
            decoder.is_apng().map_err(|source| StorageError::Image {
                path: path.to_path_buf(),
                source,
            })?
        }
        _ => false,
    };
    if animated {
        Err(import_error(
            "image_animated",
            "animated or multi-frame images are not supported",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use super::*;

    fn wait_for_state(limiter: &DecodedImageMemoryLimiter, expected: (u64, usize)) {
        for _ in 0..1000 {
            if limiter.state() == expected {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("memory limiter did not reach state {expected:?}");
    }

    #[test]
    fn decoded_memory_capacity_is_released_and_accepts_equality() {
        let limiter = DecodedImageMemoryLimiter::new(10);
        let cancelled = AtomicBool::new(false);
        {
            let _permit = limiter.acquire(10, &cancelled).unwrap();
            assert_eq!(limiter.state(), (10, 0));
        }
        assert_eq!(limiter.state(), (0, 0));
        assert!(limiter.acquire(10, &cancelled).is_ok());
    }

    #[test]
    fn decoded_memory_waiters_are_fifo_and_cancellable() {
        let limiter = Arc::new(DecodedImageMemoryLimiter::new(10));
        let cancelled = Arc::new(AtomicBool::new(false));
        let held = limiter.acquire(6, &cancelled).unwrap();
        let (large_acquired_tx, large_acquired_rx) = mpsc::channel();
        let (large_release_tx, large_release_rx) = mpsc::channel();
        let (small_acquired_tx, small_acquired_rx) = mpsc::channel();

        std::thread::scope(|scope| {
            {
                let limiter = limiter.clone();
                let cancelled = cancelled.clone();
                scope.spawn(move || {
                    let _permit = limiter.acquire(10, &cancelled).unwrap();
                    large_acquired_tx.send(()).unwrap();
                    large_release_rx.recv().unwrap();
                });
            }
            wait_for_state(&limiter, (6, 1));
            {
                let limiter = limiter.clone();
                let cancelled = cancelled.clone();
                scope.spawn(move || {
                    let _permit = limiter.acquire(4, &cancelled).unwrap();
                    small_acquired_tx.send(()).unwrap();
                });
            }
            wait_for_state(&limiter, (6, 2));
            drop(held);
            large_acquired_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
            assert!(small_acquired_rx.try_recv().is_err());
            large_release_tx.send(()).unwrap();
            small_acquired_rx
                .recv_timeout(Duration::from_secs(1))
                .unwrap();
        });

        let held = limiter.acquire(10, &cancelled).unwrap();
        let waiter_cancelled = Arc::new(AtomicBool::new(false));
        std::thread::scope(|scope| {
            let waiter_limiter = limiter.clone();
            let waiter_cancelled_for_thread = waiter_cancelled.clone();
            let waiter = scope.spawn(move || {
                waiter_limiter
                    .acquire(1, &waiter_cancelled_for_thread)
                    .err()
            });
            wait_for_state(&limiter, (10, 1));
            waiter_cancelled.store(true, Ordering::Relaxed);
            drop(held);
            let error = waiter.join().unwrap().unwrap();
            assert!(matches!(
                error,
                StorageError::Import { ref code, .. } if code == "parser_cancelled"
            ));
        });
        assert_eq!(limiter.state(), (0, 0));
    }
}
