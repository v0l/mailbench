use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use std::io::Cursor;

pub const MAX_PIXELS: u64 = 50_000_000;
pub const MAX_INLINE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_EDGE: u32 = 2000;
pub const MAX_MESSAGE_BYTES: usize = 25 * 1024 * 1024;
const JPEG_QUALITY: u8 = 85;
const THUMB_EDGE: u32 = 192;

#[derive(Debug)]
pub struct Prepared {
    pub mime: &'static str,
    pub data: Vec<u8>,
    pub note: Option<String>,
}

pub fn sniff(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

pub fn looks_like_svg(data: &[u8]) -> bool {
    let head = String::from_utf8_lossy(&data[..data.len().min(2048)]).to_ascii_lowercase();
    head.contains("<svg") || (head.trim_start().starts_with("<?xml") && head.contains("svg"))
}

fn format_of(mime: &str) -> ImageFormat {
    match mime {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/gif" => ImageFormat::Gif,
        _ => ImageFormat::WebP,
    }
}

pub fn prepare(data: Vec<u8>) -> Result<Prepared, String> {
    let Some(mime) = sniff(&data) else {
        return Err(if looks_like_svg(&data) {
            "SVG images are not accepted inline".to_string()
        } else {
            "Only PNG, JPEG, GIF and WebP images can go inline".to_string()
        });
    };
    let reader = ImageReader::with_format(Cursor::new(&data), format_of(mime));
    let (w, h) = reader.into_dimensions().map_err(|e| format!("Could not read the image: {e}"))?;
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(format!("The image is {w} by {h} pixels, over the {} megapixel limit", MAX_PIXELS / 1_000_000));
    }
    if data.len() <= MAX_INLINE_BYTES {
        return Ok(Prepared { mime, data, note: None });
    }
    let before = data.len();
    let (mime, data) = shrink(&data, mime)?;
    let note = format!("The image was {} MB, so it was shrunk to {} MB for sending", mb(before), mb(data.len()));
    Ok(Prepared { mime, data, note: Some(note) })
}

fn mb(bytes: usize) -> String {
    format!("{:.1}", bytes as f64 / 1_048_576.0)
}

fn shrink(data: &[u8], mime: &'static str) -> Result<(&'static str, Vec<u8>), String> {
    let mut decoder = ImageReader::with_format(Cursor::new(data), format_of(mime)).into_decoder().map_err(|e| format!("Could not read the image: {e}"))?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|e| format!("Could not read the image: {e}"))?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let as_jpeg = mime == "image/jpeg" || (mime == "image/webp" && !img.color().has_alpha());
    let mut edge = MAX_EDGE.min(img.width().max(img.height()));
    loop {
        let scaled = if img.width().max(img.height()) > edge { img.resize(edge, edge, image::imageops::FilterType::Triangle) } else { img.clone() };
        let bytes = if as_jpeg { encode_jpeg(&scaled)? } else { encode_png(&scaled)? };
        if bytes.len() <= MAX_INLINE_BYTES || edge <= 64 {
            return Ok((if as_jpeg { "image/jpeg" } else { "image/png" }, bytes));
        }
        edge = (edge as f32 * 0.75) as u32;
    }
}

fn encode_png(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), ImageFormat::Png).map_err(|e| format!("Could not re-encode the image: {e}"))?;
    Ok(out)
}

fn encode_jpeg(img: &DynamicImage) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| format!("Could not re-encode the image: {e}"))?;
    Ok(out)
}

pub fn thumbnail(data: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(data).ok()?;
    let small = img.thumbnail(THUMB_EDGE, THUMB_EDGE).to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw()))
}

pub fn new_cid() -> String {
    use rand::Rng;
    let n: u64 = rand::thread_rng().r#gen();
    format!("{n:016x}")
}

pub fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !matches!(c, '[' | ']' | '(' | ')' | '\n' | '\r')).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() { "image".to_string() } else { cleaned.to_string() }
}

pub fn reference(name: &str, cid: &str) -> String {
    format!("![{}](cid:{cid})", clean_name(name))
}

pub fn cids_in(body: &str) -> Vec<String> {
    crate::richtext::images(body).into_iter().map(|i| i.cid).collect()
}

pub fn plain_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut pos = 0;
    for image in crate::richtext::images(body) {
        out.push_str(&body[pos..image.start]);
        out.push_str(&format!("[image: {}]", image.alt));
        pos = image.end;
    }
    out.push_str(&body[pos..]);
    out
}

fn orientation_of(exif: &[u8]) -> Option<u16> {
    let tiff = exif.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b: [u8; 2] = tiff.get(o..o + 2)?.try_into().ok()?;
        Some(if little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b: [u8; 4] = tiff.get(o..o + 4)?.try_into().ok()?;
        Some(if little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    };
    let ifd = u32_at(4)? as usize;
    let count = u16_at(ifd)? as usize;
    (0..count).find_map(|n| {
        let at = ifd + 2 + n * 12;
        (u16_at(at)? == 0x0112).then(|| u16_at(at + 8)).flatten()
    })
}

fn orientation_segment(orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0\x2a\0\0\0\x08");
    tiff.extend_from_slice(&1u16.to_be_bytes());
    tiff.extend_from_slice(&0x0112u16.to_be_bytes());
    tiff.extend_from_slice(&3u16.to_be_bytes());
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes());
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(&payload);
    seg
}

pub fn strip_exif(jpeg: &[u8]) -> Vec<u8> {
    if !jpeg.starts_with(&[0xFF, 0xD8]) {
        return jpeg.to_vec();
    }
    let mut out = vec![0xFF, 0xD8];
    let mut orientation = None;
    let mut i = 2;
    while i + 4 <= jpeg.len() && jpeg[i] == 0xFF {
        let marker = jpeg[i + 1];
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        let end = (i + 2 + len).min(jpeg.len());
        let segment = &jpeg[i..end];
        match marker {
            0xE1 => {
                if let Some(o) = orientation_of(&jpeg[(i + 4).min(end)..end]) {
                    orientation = Some(o);
                }
            }
            0xED => {}
            _ => out.extend_from_slice(segment),
        }
        i = end;
    }
    if let Some(o) = orientation.filter(|o| *o > 1) {
        out.extend_from_slice(&orientation_segment(o));
    }
    out.extend_from_slice(&jpeg[i.min(jpeg.len())..]);
    out
}

pub fn strip_metadata(mime: &str, data: &[u8]) -> Vec<u8> {
    match mime {
        "image/jpeg" => strip_exif(data),
        "image/png" => strip_png(data),
        "image/webp" => strip_webp(data),
        _ => data.to_vec(),
    }
}

fn strip_png(data: &[u8]) -> Vec<u8> {
    if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return data.to_vec();
    }
    let mut out = data[..8].to_vec();
    let mut i = 8;
    while i + 12 <= data.len() {
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        let end = (i + 12 + len).min(data.len());
        if !matches!(&data[i + 4..i + 8], b"eXIf" | b"tEXt" | b"zTXt" | b"iTXt") {
            out.extend_from_slice(&data[i..end]);
        }
        i = end;
    }
    out.extend_from_slice(&data[i..]);
    out
}

fn strip_webp(data: &[u8]) -> Vec<u8> {
    if sniff(data) != Some("image/webp") {
        return data.to_vec();
    }
    let mut out = data[..12].to_vec();
    let mut i = 12;
    while i + 8 <= data.len() {
        let len = u32::from_le_bytes([data[i + 4], data[i + 5], data[i + 6], data[i + 7]]) as usize;
        let end = (i + 8 + len + (len & 1)).min(data.len());
        match &data[i..i + 4] {
            b"EXIF" | b"XMP " => {}
            b"VP8X" if end - i >= 9 => {
                let mut chunk = data[i..end].to_vec();
                chunk[8] &= !0x0C;
                out.extend_from_slice(&chunk);
            }
            _ => out.extend_from_slice(&data[i..end]),
        }
        i = end;
    }
    let riff = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    out
}

pub fn orientation(jpeg: &[u8]) -> Option<u16> {
    let mut i = 2;
    while i + 4 <= jpeg.len() && jpeg[i] == 0xFF && jpeg[i + 1] != 0xDA {
        let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
        let end = (i + 2 + len).min(jpeg.len());
        if jpeg[i + 1] == 0xE1 {
            if let Some(o) = orientation_of(&jpeg[i + 4..end]) {
                return Some(o);
            }
        }
        i = end;
    }
    None
}
