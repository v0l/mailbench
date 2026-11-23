use mail_parser::MessageParser;
use mailbench::backend::{render_message, Address, Attachment, OutgoingMessage};
use mailbench::inline;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x * 7) as u8, (y * 13) as u8, (x ^ y) as u8, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn noisy_png(w: u32, h: u32) -> Vec<u8> {
    let mut seed = 12345u32;
    let img = image::RgbaImage::from_fn(w, h, |_, _| {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        image::Rgba([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8, 255])
    });
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 3) as u8, (y * 5) as u8, 90]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Jpeg).unwrap();
    out
}

fn with_gps(jpeg: &[u8]) -> Vec<u8> {
    let payload = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\0\0\0\0\0GPSMARKER-52.1N-4.3E";
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(payload);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&seg);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn message(body: &str, attachments: Vec<Attachment>, html: bool) -> OutgoingMessage {
    OutgoingMessage {
        from: Address { name: Some("Me".into()), address: "me@example.test".into() },
        to: vec![Address { name: None, address: "you@example.test".into() }],
        subject: "pic".into(),
        body: body.into(),
        attachments,
        html_alternative: html,
        ..Default::default()
    }
}

fn inline_attachment(cid: &str, name: &str, mime: &str, data: Vec<u8>) -> Attachment {
    Attachment { name: name.into(), mime: mime.into(), data, cid: Some(cid.into()) }
}

#[test]
fn sent_message_has_related_part_with_matching_content_id() {
    let data = png(8, 8);
    let m = message("look\n\n![shot.png](cid:abc123)\n\nbye", vec![inline_attachment("abc123", "shot.png", "image/png", data.clone())], true);
    let raw = render_message(&m, false).unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(text.contains("multipart/alternative"), "{text}");
    assert!(text.contains("multipart/related"), "{text}");
    assert!(text.find("multipart/alternative").unwrap() < text.find("multipart/related").unwrap());
    assert!(!text.contains("multipart/mixed"));
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let html = parsed.body_html(0).unwrap().to_string();
    assert!(html.contains("<img src=\"cid:abc123\""), "{html}");
    let part = parsed.attachments().next().unwrap();
    assert_eq!(part.content_id(), Some("abc123"));
    assert!(part.content_disposition().is_some_and(|d| d.is_inline()));
    assert_eq!(part.decoded().into_owned(), data);
    let plain = parsed.body_text(0).unwrap().to_string();
    assert!(plain.contains("[image: shot.png]"), "{plain}");
    assert!(!plain.contains("cid:"));
}

#[test]
fn regular_attachment_goes_beside_inline_in_mixed() {
    let m = message(
        "![a.png](cid:one)",
        vec![inline_attachment("one", "a.png", "image/png", png(4, 4)), Attachment { name: "n.txt".into(), mime: "text/plain".into(), data: b"x".to_vec(), cid: None }],
        true,
    );
    let raw = render_message(&m, false).unwrap();
    let s = String::from_utf8_lossy(&raw);
    assert!(s.find("multipart/mixed").unwrap() < s.find("multipart/alternative").unwrap());
    assert!(s.contains("multipart/related"));
}

#[test]
fn plain_text_only_sends_inline_images_as_attachments() {
    let m = message("see ![a.png](cid:one) here", vec![inline_attachment("one", "a.png", "image/png", png(4, 4))], false);
    let raw = render_message(&m, false).unwrap();
    let s = String::from_utf8_lossy(&raw);
    assert!(!s.contains("multipart/related") && !s.contains("text/html"));
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let part = parsed.attachments().next().unwrap();
    assert_eq!(part.attachment_name(), Some("a.png"));
    assert!(parsed.body_text(0).unwrap().contains("[image: a.png]"));
}

#[test]
fn exif_gps_is_gone_from_the_sent_jpeg() {
    let marked = with_gps(&jpeg(16, 16));
    assert!(marked.windows(10).any(|w| w == b"GPSMARKER-"));
    let m = message("![p.jpg](cid:j1)", vec![inline_attachment("j1", "p.jpg", "image/jpeg", marked)], true);
    let raw = render_message(&m, false).unwrap();
    let parsed = MessageParser::default().parse(&raw).unwrap();
    let sent = parsed.attachments().next().unwrap().decoded().into_owned();
    assert!(!sent.windows(10).any(|w| w == b"GPSMARKER-"));
    assert!(image::load_from_memory(&sent).is_ok());
}

#[test]
fn jpeg_orientation_survives_gps_stripping() {
    let payload = b"Exif\0\0MM\0\x2a\0\0\0\x08\0\x01\x01\x12\0\x03\0\0\0\x01\0\x06\0\0\0\0\0\0";
    let base = jpeg(16, 16);
    let mut seg = vec![0xFF, 0xE1];
    seg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    seg.extend_from_slice(payload);
    let mut tagged = base[..2].to_vec();
    tagged.extend_from_slice(&seg);
    tagged.extend_from_slice(&base[2..]);
    assert_eq!(inline::orientation(&tagged), Some(6));
    assert_eq!(inline::orientation(&inline::strip_exif(&tagged)), Some(6));
}

#[test]
fn magic_bytes_decide_not_the_name() {
    assert_eq!(inline::sniff(&jpeg(4, 4)), Some("image/jpeg"));
    let prepared = inline::prepare(jpeg(4, 4)).unwrap();
    assert_eq!(prepared.mime, "image/jpeg");
    assert!(inline::prepare(b"%PDF-1.4 hello".to_vec()).is_err());
}

#[test]
fn svg_is_refused() {
    let svg = b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>".to_vec();
    let err = inline::prepare(svg).unwrap_err();
    assert!(err.contains("SVG"), "{err}");
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

#[test]
fn a_20000_by_20000_image_is_refused_without_decoding_it() {
    let mut data = png(8, 8);
    data[16..20].copy_from_slice(&20000u32.to_be_bytes());
    data[20..24].copy_from_slice(&20000u32.to_be_bytes());
    let crc = crc32(&data[12..29]);
    data[29..33].copy_from_slice(&crc.to_be_bytes());
    let err = inline::prepare(data).unwrap_err();
    assert!(err.contains("megapixel"), "{err}");
}

#[test]
fn a_big_png_is_downscaled_under_five_megabytes() {
    let big = noisy_png(2400, 1800);
    assert!(big.len() > 12 * 1024 * 1024 / 2, "fixture is {} bytes", big.len());
    let prepared = inline::prepare(big).unwrap();
    assert!(prepared.data.len() <= inline::MAX_INLINE_BYTES);
    assert_eq!(prepared.mime, "image/png");
    assert!(prepared.note.is_some());
    let img = image::load_from_memory(&prepared.data).unwrap();
    assert!(img.width().max(img.height()) <= inline::MAX_EDGE);
}

#[test]
fn a_big_photo_becomes_a_jpeg() {
    let mut seed = 99u32;
    let img = image::RgbImage::from_fn(3000, 3000, |_, _| {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        image::Rgb([(seed >> 24) as u8, (seed >> 16) as u8, (seed >> 8) as u8])
    });
    let mut data = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut data, 100).encode_image(&img).unwrap();
    assert!(data.len() > inline::MAX_INLINE_BYTES, "fixture is {} bytes", data.len());
    let prepared = inline::prepare(data).unwrap();
    assert_eq!(prepared.mime, "image/jpeg");
    assert!(prepared.data.len() <= inline::MAX_INLINE_BYTES);
}

#[test]
fn a_small_image_is_left_alone() {
    let data = png(10, 10);
    let prepared = inline::prepare(data.clone()).unwrap();
    assert_eq!(prepared.data, data);
    assert!(prepared.note.is_none());
}

#[test]
fn image_references_parse_and_leave_links_alone() {
    let body = "a ![x.png](cid:ab12) b [link](https://e.test/) ![bad](https://e.test/x.png)";
    let found = mailbench::richtext::images(body);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].cid, "ab12");
    assert_eq!(found[0].alt, "x.png");
    assert_eq!(inline::plain_body(body), "a [image: x.png] b [link](https://e.test/) ![bad](https://e.test/x.png)");
}
