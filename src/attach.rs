use crate::mailstore::AttachmentData;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const OPEN_DIR_NAME: &str = "mailbench-open";

pub fn default_open_dir() -> PathBuf {
    std::env::temp_dir().join(OPEN_DIR_NAME)
}

pub fn safe_name(name: &str, fallback: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { fallback.to_string() } else { cleaned }
}

pub fn unique_names(items: &[AttachmentData]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        let base = safe_name(&item.name, "attachment");
        let mut candidate = base.clone();
        let mut n = 2;
        while seen.contains(&candidate) {
            candidate = match base.rsplit_once('.') {
                Some((stem, ext)) => format!("{stem} ({n}).{ext}"),
                None => format!("{base} ({n})"),
            };
            n += 1;
        }
        seen.push(candidate);
    }
    seen
}

pub fn zip_all(items: &[AttachmentData]) -> std::io::Result<Vec<u8>> {
    let names = unique_names(items);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (item, name) in items.iter().zip(names) {
        zip.start_file(name, options).map_err(std::io::Error::other)?;
        zip.write_all(&item.data)?;
    }
    Ok(zip.finish().map_err(std::io::Error::other)?.into_inner())
}

pub fn is_previewable_image(mime: &str) -> bool {
    matches!(mime.to_ascii_lowercase().as_str(), "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp")
}

pub fn decode_image(data: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(data).ok()?;
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()))
}

pub fn is_previewable(mime: &str, name: &str) -> bool {
    is_previewable_image(mime) || is_pdf(mime, name)
}

pub fn preview_image(mime: &str, name: &str, data: &[u8]) -> Option<egui::ColorImage> {
    if is_previewable_image(mime) {
        decode_image(data)
    } else if is_pdf(mime, name) {
        render_pdf_first_page(data, 720.0)
    } else {
        None
    }
}

pub fn is_pdf(mime: &str, name: &str) -> bool {
    mime.eq_ignore_ascii_case("application/pdf") || name.to_ascii_lowercase().ends_with(".pdf")
}

pub fn render_pdf_first_page(data: &[u8], target_width: f32) -> Option<egui::ColorImage> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{PixmapSettings, RenderCache, RenderSettings, render};
    let pdf = Pdf::new(data.to_vec()).ok()?;
    let pages = pdf.pages();
    let page = pages.first()?;
    let (width, height) = page.render_dimensions();
    if width < 1.0 || height < 1.0 {
        return None;
    }
    let scale = (target_width / width).clamp(0.05, 4.0);
    let pixmap = render(
        page,
        &RenderCache::new(),
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    decode_image(&pixmap.into_png().ok()?)
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

pub fn wipe_open_dir(dir: &Path) {
    let _ = std::fs::remove_dir_all(dir);
}

pub fn prepare_open_dir(dir: &Path) -> std::io::Result<()> {
    create_private_dir(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn write_private(dir: &Path, name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    prepare_open_dir(dir)?;
    let sub = dir.join(crate::accounts::new_id());
    create_private_dir(&sub)?;
    let path = sub.join(safe_name(name, "attachment"));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(&path)?.write_all(data)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY_PDF: &str = "%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 100]/Contents 4 0 R/Resources<<>>>>endobj\n4 0 obj<</Length 30>>stream\n1 0 0 rg 0 0 100 100 re f\nendstream\nendobj\ntrailer<</Root 1 0 R/Size 5>>\n";

    #[test]
    fn renders_pdf_first_page() {
        let img = render_pdf_first_page(TINY_PDF.as_bytes(), 100.0).unwrap();
        assert_eq!(img.size, [100, 50]);
        assert_eq!(img.pixels[0].r(), 255);
        assert_eq!(img.pixels[0].g(), 0);
        assert_eq!(img.pixels[99].g(), 255);
        assert!(render_pdf_first_page(b"nope", 100.0).is_none());
    }

    #[test]
    fn detects_pdf() {
        assert!(is_pdf("application/PDF", "x"));
        assert!(is_pdf("application/octet-stream", "A.PDF"));
        assert!(!is_pdf("image/png", "a.png"));
    }
    use std::sync::Arc;

    fn item(name: &str, data: &[u8]) -> AttachmentData {
        AttachmentData { name: name.into(), mime: "application/octet-stream".into(), data: Arc::new(data.to_vec()) }
    }

    #[test]
    fn names_cannot_escape_the_directory() {
        assert_eq!(safe_name("../../etc/passwd", "x"), "_.._etc_passwd");
        assert_eq!(safe_name("..", "x"), "x");
        assert_eq!(safe_name("", "x"), "x");
    }

    #[test]
    fn duplicate_names_get_a_counter() {
        let items = [item("a.txt", b"1"), item("a.txt", b"2"), item("b", b"3"), item("b", b"4")];
        assert_eq!(unique_names(&items), ["a.txt", "a (2).txt", "b", "b (2)"]);
    }

    #[test]
    fn zip_holds_every_attachment_byte_for_byte() {
        let items = [item("a.txt", b"alpha"), item("a.txt", b"beta")];
        let bytes = zip_all(&items).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2);
        let mut out = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("a (2).txt").unwrap(), &mut out).unwrap();
        assert_eq!(out, "beta");
    }

    #[test]
    fn open_dir_is_private_and_wiped() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(OPEN_DIR_NAME);
        let path = write_private(&dir, "x.txt", b"secret").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        wipe_open_dir(&dir);
        assert!(!dir.exists());
    }

    #[test]
    fn png_bytes_decode_for_preview() {
        let mut png = Vec::new();
        image::DynamicImage::new_rgba8(3, 2)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_image(&png).unwrap().size, [3, 2]);
        assert!(decode_image(b"nope").is_none());
    }

    #[test]
    fn only_raster_formats_preview() {
        assert!(is_previewable_image("image/PNG"));
        assert!(is_previewable_image("image/webp"));
        assert!(!is_previewable_image("image/svg+xml"));
        assert!(!is_previewable_image("application/pdf"));
    }
}
