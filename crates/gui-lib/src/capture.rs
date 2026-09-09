//! Native-window screenshot encoding for visual-regression captures.
//!
//! The Iced window API supplies RGBA bytes. This module turns that exact buffer
//! into a PNG without involving OS-level screenshot tools, which avoids window
//! chrome and desktop-background nondeterminism.
use crc32fast::Hasher;
use flate2::{write::ZlibEncoder, Compression};
use iced::window::Screenshot;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

pub fn write_png(screenshot: Screenshot, destination: &Path) -> Result<PathBuf, String> {
    let width = screenshot.size.width;
    let height = screenshot.size.height;
    let rgba = screenshot.rgba;
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "screenshot dimensions overflow the PNG encoder".to_owned())?;
    if rgba.len() != expected {
        return Err(format!(
            "screenshot has {} RGBA bytes but {expected} are required for {width}×{height}",
            rgba.len()
        ));
    }

    let row_bytes = usize::try_from(width)
        .map_err(|_| "screenshot width does not fit this platform".to_owned())?
        .checked_mul(4)
        .ok_or_else(|| "screenshot row is too wide".to_owned())?;
    let mut scanlines = Vec::with_capacity(expected + usize::try_from(height).unwrap_or(0));
    for row in rgba.chunks_exact(row_bytes) {
        scanlines.push(0); // PNG filter type: None
        scanlines.extend_from_slice(row);
    }
    let mut compressed = ZlibEncoder::new(Vec::new(), Compression::best());
    compressed
        .write_all(&scanlines)
        .map_err(|error| format!("could not compress screenshot: {error}"))?;
    let compressed = compressed
        .finish()
        .map_err(|error| format!("could not finish screenshot compression: {error}"))?;

    let mut png = Vec::with_capacity(8 + 25 + compressed.len() + 12);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // RGBA8, no interlace
    append_chunk(&mut png, *b"IHDR", &ihdr);
    append_chunk(&mut png, *b"IDAT", &compressed);
    append_chunk(&mut png, *b"IEND", &[]);

    let parent = destination
        .parent()
        .ok_or_else(|| "capture destination has no parent directory".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create capture directory: {error}"))?;
    fs::write(destination, png).map_err(|error| format!("could not write PNG capture: {error}"))?;
    Ok(destination.to_owned())
}

fn append_chunk(output: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    output.extend_from_slice(&kind);
    output.extend_from_slice(data);
    let mut checksum = Hasher::new();
    checksum.update(&kind);
    checksum.update(data);
    output.extend_from_slice(&checksum.finalize().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::Size;

    #[test]
    fn writes_a_validly_framed_rgba_png() {
        let temporary = std::env::temp_dir().join(format!(
            "denon-avr-capture-{}-{}.png",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let screenshot = Screenshot::new(vec![0_u8, 0, 0, 255], Size::new(1, 1), 1.0);
        write_png(screenshot, &temporary).unwrap();
        let png = fs::read(&temporary).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        let _ = fs::remove_file(temporary);
    }
}
