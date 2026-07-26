use std::{fs::File, io::BufReader, path::Path, sync::atomic::AtomicBool};

use image::{AnimationDecoder, ImageDecoder, ImageFormat, ImageReader, metadata::Orientation};

use super::{
    source::{hash_file_cancellable, import_error, source_extension},
    types::{ImportLimits, RegisteredFile},
};
use crate::error::{PathIo, StorageError, StorageResult};

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
    if decoder.total_bytes() > limits.decoded_image_bytes {
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
