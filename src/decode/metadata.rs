//! Finding a file's EXIF without reading its image data. kamadak-exif's own
//! search reads through the file, all of it when there's no EXIF to find
//! (most wallpapers), which over a network takes long enough to see. JPEG,
//! PNG and WebP say how long each part is, so the parts are skipped here.

use exif::Exif;
use std::io::{BufRead, Read, Seek, SeekFrom};

const EXIF_ID: &[u8] = b"Exif\0\0";

/// The EXIF in `container` (read from its start), if it has any.
pub fn exif_in(container: &mut (impl BufRead + Seek)) -> Option<Exif> {
    let mut magic = [0; 12];
    container.rewind().ok()?;
    let read = container.by_ref().take(12).read(&mut magic).ok()?;
    let magic = &magic[..read];
    let raw = if magic.starts_with(&[0xff, 0xd8]) {
        container.seek(SeekFrom::Start(2)).ok()?;
        jpeg(container)?
    } else if magic.starts_with(b"\x89PNG\r\n\x1a\n") {
        container.seek(SeekFrom::Start(8)).ok()?;
        png(container)?
    } else if magic.len() == 12 && &magic[..4] == b"RIFF" && &magic[8..] == b"WEBP" {
        container.seek(SeekFrom::Start(12)).ok()?;
        webp(container)?
    } else {
        // TIFF and HEIF, which kamadak-exif doesn't read through.
        container.rewind().ok()?;
        return exif::Reader::new().read_from_container(container).ok();
    };
    exif::Reader::new().read_raw(raw).ok()
}

// The segments before the scan data, where EXIF has to be.
fn jpeg(container: &mut (impl BufRead + Seek)) -> Option<Vec<u8>> {
    loop {
        // To the next marker, past any fill bytes.
        container.read_until(0xff, &mut Vec::new()).ok()?;
        let mut code = 0xff;
        while code == 0xff {
            code = byte(container)?;
        }
        match code {
            // Markers without a segment.
            0x00 | 0x01 | 0xd0..=0xd7 => continue,
            // The file starts again or ends, or the image starts (SOS).
            0xd8..=0xda => return None,
            _ => {}
        }
        let mut length = [0; 2];
        container.read_exact(&mut length).ok()?;
        let length = u64::from(u16::from_be_bytes(length).checked_sub(2)?);
        if code == 0xe1 {
            let segment = exactly(container, length)?;
            if let Some(exif) = segment.strip_prefix(EXIF_ID) {
                return Some(exif.to_vec());
            }
        } else {
            container.seek_relative(length as i64).ok()?;
        }
    }
}

// The eXIf chunk, which may come after the image data.
fn png(container: &mut (impl BufRead + Seek)) -> Option<Vec<u8>> {
    loop {
        let mut header = [0; 8];
        container.read_exact(&mut header).ok()?;
        let length = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        match &header[4..] {
            b"eXIf" => return exactly(container, length.into()),
            b"IEND" => return None,
            // The chunk and its CRC.
            _ => container.seek_relative(i64::from(length) + 4).ok()?,
        }
    }
}

// The EXIF chunk, which comes after the image data.
fn webp(container: &mut (impl BufRead + Seek)) -> Option<Vec<u8>> {
    loop {
        let mut header = [0; 8];
        container.read_exact(&mut header).ok()?;
        let length = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        if &header[..4] == b"EXIF" {
            return exactly(container, length.into());
        }
        // Chunks are padded to an even length.
        container
            .seek_relative(i64::from(length) + i64::from(length & 1))
            .ok()?;
    }
}

fn byte(container: &mut impl Read) -> Option<u8> {
    let mut byte = [0];
    container.read_exact(&mut byte).ok()?;
    Some(byte[0])
}

// `length` bytes, or none if the file ends first (a length is only what the
// file claims, so nothing is allocated ahead of reading).
fn exactly(container: &mut impl Read, length: u64) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    container
        .by_ref()
        .take(length)
        .read_to_end(&mut data)
        .ok()?;
    (data.len() as u64 == length).then_some(data)
}
