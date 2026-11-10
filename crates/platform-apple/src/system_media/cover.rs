use std::{io::Read, ptr::NonNull, thread, time::Duration};

use objc2::{AllocAnyThread, rc::Retained};
use objc2_app_kit::NSImage;
use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFType, kCFBooleanTrue};
use objc2_core_graphics::CGImage;
use objc2_foundation::NSSize;
use objc2_image_io::{
    CGImageSource, kCGImageSourceCreateThumbnailFromImageAlways,
    kCGImageSourceCreateThumbnailWithTransform, kCGImageSourceShouldCacheImmediately,
    kCGImageSourceThumbnailMaxPixelSize,
};
use objc2_media_player::MPMediaItemArtwork;
use reqwest::{Url, blocking::Client, redirect::Policy};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_PIXELS: i32 = 256;

struct PendingCover {
    url: String,
    fetch: thread::JoinHandle<Option<CFRetained<CGImage>>>,
}

#[derive(Default)]
pub(super) struct Cover {
    url: Option<String>,
    attempted: bool,
    pending: Option<PendingCover>,
    pub(super) artwork: Option<Retained<MPMediaItemArtwork>>,
}

impl Cover {
    /// Poll from the existing desktop tick. Keep at most one fetch in flight,
    /// and publish its result only if it still belongs to the current cover.
    pub(super) fn update(&mut self, url: Option<&str>) -> bool {
        let mut changed = false;
        if self.url.as_deref() != url {
            self.url = url.map(str::to_owned);
            self.attempted = false;
            changed = self.artwork.take().is_some();
        }
        if self.pending.as_ref().is_some_and(|p| p.fetch.is_finished()) {
            let pending = self.pending.take().expect("finished cover fetch");
            let image = pending.fetch.join().ok().flatten();
            if self.url.as_deref() == Some(pending.url.as_str()) {
                self.attempted = true;
                self.artwork = image.map(artwork);
                changed |= self.artwork.is_some();
            }
        }
        if self.pending.is_none()
            && !self.attempted
            && let Some(url) = &self.url
        {
            self.attempted = true;
            let requested = url.clone();
            self.pending = thread::Builder::new()
                .name("musicx-cover".into())
                .spawn(move || fetch(&requested))
                .ok()
                .map(|fetch| PendingCover {
                    url: url.clone(),
                    fetch,
                });
        }
        changed
    }
}

fn fetch(value: &str) -> Option<CFRetained<CGImage>> {
    if Url::parse(value)
        .ok()?
        .fragment()
        .is_some_and(|f| f.starts_with("musicx-plugin="))
    {
        return thumbnail(&crate::player::artwork(value)?);
    }
    let url = musicx_core::metadata::public_url(value)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(Policy::custom(|attempt| {
            let next = attempt.url();
            if attempt.previous().len() < 4
                && next.scheme() == "https"
                && musicx_core::metadata::public_url(next.as_str()).is_some()
                && attempt.previous().first().and_then(Url::host_str) == next.host_str()
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > MAX_BYTES) {
        return None;
    }
    let mut bytes = Vec::new();
    response.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_BYTES {
        return None;
    }
    thumbnail(&bytes)
}

fn thumbnail(bytes: &[u8]) -> Option<CFRetained<CGImage>> {
    let data = CFData::from_bytes(bytes);
    // ImageIO option values are booleans and a maximum edge in pixels.
    unsafe {
        let source = CGImageSource::with_data(&data, None)?;
        let yes: &CFType = kCFBooleanTrue?;
        let max = CFNumber::new_i32(MAX_PIXELS);
        let options = CFDictionary::from_slices(
            &[
                kCGImageSourceCreateThumbnailFromImageAlways,
                kCGImageSourceCreateThumbnailWithTransform,
                kCGImageSourceShouldCacheImmediately,
                kCGImageSourceThumbnailMaxPixelSize,
            ],
            &[yes, yes, yes, &max],
        );
        source.thumbnail_at_index(0, Some(options.as_opaque()))
    }
}

fn artwork(image: CFRetained<CGImage>) -> Retained<MPMediaItemArtwork> {
    let bounds = NSSize::new(
        CGImage::width(Some(&image)) as f64,
        CGImage::height(Some(&image)) as f64,
    );
    let request = block2::RcBlock::new(move |size: NSSize| {
        let rendered = NSImage::initWithCGImage_size(NSImage::alloc(), &image, size);
        NonNull::new(Retained::autorelease_return(rendered)).expect("native artwork image")
    });
    unsafe {
        MPMediaItemArtwork::initWithBoundsSize_requestHandler(
            MPMediaItemArtwork::alloc(),
            bounds,
            &request,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_returns_the_native_card_size() {
        assert!(thumbnail(b"invalid image").is_none());
        let image = thumbnail(include_bytes!("../../../../assets/musicx-now-playing.png")).unwrap();
        assert!(CGImage::width(Some(&image)) <= MAX_PIXELS as usize);
        assert!(CGImage::height(Some(&image)) <= MAX_PIXELS as usize);
        let size = NSSize::new(224.0, 224.0);
        objc2::rc::autoreleasepool(|_| {
            let actual = unsafe { artwork(image).imageWithSize(size) }.unwrap();
            assert_eq!(actual.size(), size);
        });
    }
}
