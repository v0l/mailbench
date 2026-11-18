use anyrender::{PaintScene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_HTML: usize = 3_000_000;
const MAX_HEIGHT: f32 = 30_000.0;
const TILE: u32 = 2048;
const REMOTE_TIMEOUT: Duration = Duration::from_secs(8);
const REMOTE_MAX: u64 = 8 * 1024 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlBody {
    pub html: String,
    pub inline: Vec<(String, Arc<Vec<u8>>)>,
}

impl HtmlBody {
    pub fn is_empty(&self) -> bool {
        self.html.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderKey {
    pub id: u64,
    pub width: u32,
    pub scale_milli: u32,
    pub remote: bool,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub rect: [f32; 4],
    pub href: String,
}

pub struct Tile {
    pub top: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct Rendered {
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub tiles: Vec<Tile>,
    pub links: Vec<Link>,
}

struct Job {
    key: RenderKey,
    body: HtmlBody,
}

struct Resources {
    inline: HashMap<String, Arc<Vec<u8>>>,
    remote: bool,
    pending: Mutex<usize>,
}

impl Resources {
    fn lookup(&self, url: &url::Url) -> Option<Vec<u8>> {
        match url.scheme() {
            "cid" => {
                let id = percent_encoding::percent_decode_str(url.path()).decode_utf8_lossy();
                let id = id.trim_matches(['<', '>']).to_ascii_lowercase();
                self.inline.get(&id).map(|b| b.to_vec())
            }
            "data" => data_url(url.as_str()),
            "http" | "https" if self.remote => fetch_remote(url.as_str()),
            _ => None,
        }
    }
}

fn data_url(url: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let rest = url.strip_prefix("data:")?;
    let (meta, data) = rest.split_once(',')?;
    if meta.ends_with(";base64") {
        let clean: String = data.chars().filter(|c| !c.is_whitespace()).collect();
        base64::engine::general_purpose::STANDARD.decode(clean).ok()
    } else {
        Some(percent_encoding::percent_decode_str(data).collect())
    }
}

fn fetch_remote(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new().timeout(REMOTE_TIMEOUT).redirects(4).build();
    let resp = agent.get(url).call().ok()?;
    let mut out = Vec::new();
    resp.into_reader().take(REMOTE_MAX).read_to_end(&mut out).ok()?;
    Some(out)
}

struct MailNet {
    resources: Arc<Resources>,
}

impl NetProvider for MailNet {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.clone();
        if matches!(url.scheme(), "http" | "https") && self.resources.remote {
            *self.resources.pending.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            let resources = self.resources.clone();
            std::thread::spawn(move || {
                let bytes = resources.lookup(&url).unwrap_or_default();
                handler.bytes(url.to_string(), Bytes::from(bytes));
                *resources.pending.lock().unwrap_or_else(|e| e.into_inner()) -= 1;
            });
            return;
        }
        let bytes = self.resources.lookup(&url).unwrap_or_default();
        handler.bytes(url.to_string(), Bytes::from(bytes));
    }
}

fn sanitize(html: &str) -> String {
    let mut out = String::with_capacity(html.len().min(MAX_HTML));
    let lower = html.to_ascii_lowercase();
    let mut i = 0;
    let bytes = html.as_bytes();
    while i < html.len() && out.len() < MAX_HTML {
        if bytes[i] == b'<' {
            let rest = &lower[i..];
            let blocked = ["<script", "<iframe", "<object", "<embed", "<frame", "<form", "<input", "<button", "<textarea", "<select", "<meta", "<base", "<link"]
                .into_iter()
                .find(|t| rest.starts_with(t) && rest[t.len()..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/'));
            if let Some(tag) = blocked {
                let name = &tag[1..];
                let open_end = rest.find('>').map_or(html.len(), |e| i + e + 1);
                let paired = matches!(name, "script" | "iframe" | "object" | "textarea" | "select" | "button" | "form");
                i = if paired && name != "form" {
                    let close = format!("</{name}");
                    lower[open_end..].find(&close).map_or(html.len(), |c| {
                        let after = open_end + c;
                        lower[after..].find('>').map_or(html.len(), |e| after + e + 1)
                    })
                } else {
                    open_end
                };
                continue;
            }
            if rest.starts_with("</form") {
                i = rest.find('>').map_or(html.len(), |e| i + e + 1);
                continue;
            }
        }
        let ch = html[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

const MAIL_CSS: &str = "html { background: #ffffff; } body { margin: 16px; color: #1a1a1a; font-family: sans-serif; font-size: 14px; line-height: 1.45; overflow-wrap: anywhere; } img { max-width: 100%; height: auto; } pre { white-space: pre-wrap; }";

fn wrap(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let style = format!("<style>{MAIL_CSS}</style>");
    if let Some(h) = lower.find("<head") {
        let at = lower[h..].find('>').map_or(html.len(), |e| h + e + 1);
        format!("{}{style}{}", &html[..at], &html[at..])
    } else {
        format!("<html><head>{style}</head><body>{html}</body></html>")
    }
}

fn document(body: &HtmlBody, width: u32, scale: f32, remote: bool) -> (BaseDocument, Arc<Resources>) {
    let resources = Arc::new(Resources {
        inline: body.inline.iter().map(|(k, v)| (k.trim_matches(['<', '>']).to_ascii_lowercase(), v.clone())).collect(),
        remote,
        pending: Mutex::new(0),
    });
    let net = Arc::new(MailNet { resources: resources.clone() });
    let height = 800;
    let doc = HtmlDocument::from_html(
        &wrap(&sanitize(&body.html)),
        DocumentConfig {
            base_url: Some("cid:mail/".to_string()),
            net_provider: Some(net as _),
            viewport: Some(Viewport::new((width as f32 * scale) as u32, (height as f32 * scale) as u32, scale, ColorScheme::Light)),
            ..Default::default()
        },
    );
    (doc.into_inner(), resources)
}

fn settle(doc: &mut BaseDocument, resources: &Resources) {
    let deadline = Instant::now() + REMOTE_TIMEOUT + Duration::from_secs(1);
    doc.resolve(0.0);
    while *resources.pending.lock().unwrap_or_else(|e| e.into_inner()) > 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
        doc.resolve(0.0);
    }
    doc.resolve(0.0);
}

fn links(doc: &BaseDocument) -> Vec<Link> {
    let mut out = Vec::new();
    for (id, node) in doc.tree().iter() {
        if !node.data.is_element_with_tag_name(&local_name!("a")) {
            continue;
        }
        let Some(href) = node.attr(local_name!("href")) else { continue };
        let href = href.trim();
        let lower = href.to_ascii_lowercase();
        if !(lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:")) {
            continue;
        }
        for r in doc.node_client_rects(id) {
            if r.width > 0.0 && r.height > 0.0 {
                out.push(Link { rect: [r.x as f32, r.y as f32, r.width as f32, r.height as f32], href: href.to_string() });
            }
        }
    }
    out
}

fn widest(doc: &BaseDocument) -> f32 {
    doc.tree()
        .iter()
        .filter(|(_, n)| n.is_element())
        .filter_map(|(id, _)| doc.get_client_bounding_rect(id))
        .map(|r| (r.x + r.width) as f32)
        .fold(0.0, f32::max)
}

pub fn render(body: &HtmlBody, width: u32, scale: f32, remote: bool) -> Rendered {
    let width = width.max(64);
    let (mut doc, resources) = document(body, width, scale, remote);
    settle(&mut doc, &resources);
    let content_w = doc.try_root_element().map_or(0.0, |_| widest(&doc));
    let content_h = doc.try_root_element().map_or(0.0, |r| r.final_layout().size.height);
    let fit = if content_w > width as f32 + 1.0 { width as f32 / content_w } else { 1.0 };
    let layout_w = if fit < 1.0 { content_w.ceil() as u32 } else { width };
    if fit < 1.0 {
        doc.set_viewport(Viewport::new((layout_w as f32 * scale) as u32, (800.0 * scale) as u32, scale, ColorScheme::Light));
        doc.resolve(0.0);
    }
    let content_h = if fit < 1.0 { doc.try_root_element().map_or(content_h, |r| r.final_layout().size.height) } else { content_h };
    let height = content_h.clamp(1.0, MAX_HEIGHT);
    let paint = scale * fit;
    let px_w = (width as f32 * scale).ceil() as u32;
    let px_h = (height * paint).ceil() as u32;
    doc.set_viewport(Viewport::new(px_w, px_h, paint, ColorScheme::Light));
    doc.resolve(0.0);
    let links = links(&doc).into_iter().map(|l| Link { rect: l.rect.map(|v| v * fit), href: l.href }).collect();
    let mut tiles = Vec::new();
    let mut top = 0;
    while top < px_h {
        let h = TILE.min(px_h - top);
        let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| {
                scene.fill(
                    peniko::Fill::NonZero,
                    Default::default(),
                    peniko::color::palette::css::WHITE,
                    None,
                    &peniko::kurbo::Rect::new(0.0, 0.0, px_w as f64, h as f64),
                );
                blitz_paint::paint_scene(scene, &mut doc, paint as f64, px_w, h, 0, top);
            },
            px_w,
            h,
        );
        tiles.push(Tile { top, width: px_w, height: h, rgba });
        top += h;
    }
    Rendered { width: width as f32, height: height * fit, scale, tiles, links }
}

pub struct Renderer {
    jobs: Sender<Job>,
    done: Arc<Mutex<HashMap<RenderKey, Arc<Rendered>>>>,
    asked: Mutex<Vec<RenderKey>>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(None)
    }
}

impl Renderer {
    pub fn new(ctx: Option<egui::Context>) -> Renderer {
        let (jobs, rx): (Sender<Job>, Receiver<Job>) = channel();
        let done: Arc<Mutex<HashMap<RenderKey, Arc<Rendered>>>> = Arc::default();
        let out = done.clone();
        std::thread::Builder::new()
            .name("html-render".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        job = newer;
                    }
                    let key = job.key.clone();
                    let scale = key.scale_milli as f32 / 1000.0;
                    let body = job.body;
                    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&body, key.width, scale, key.remote)))
                        .unwrap_or_else(|_| Rendered { width: key.width as f32, height: 1.0, scale, tiles: Vec::new(), links: Vec::new() });
                    let mut done = out.lock().unwrap_or_else(|e| e.into_inner());
                    if done.len() > 8 {
                        done.clear();
                    }
                    done.insert(key, Arc::new(rendered));
                    drop(done);
                    if let Some(ctx) = &ctx {
                        ctx.request_repaint();
                    }
                }
            })
            .expect("render thread");
        Renderer { jobs, done, asked: Mutex::new(Vec::new()) }
    }

    pub fn get(&self, key: &RenderKey, body: &HtmlBody) -> Option<Arc<Rendered>> {
        if let Some(r) = self.done.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return Some(r.clone());
        }
        let mut asked = self.asked.lock().unwrap_or_else(|e| e.into_inner());
        if !asked.contains(key) {
            asked.push(key.clone());
            if asked.len() > 32 {
                asked.remove(0);
            }
            let _ = self.jobs.send(Job { key: key.clone(), body: body.clone() });
        }
        None
    }

    pub fn latest_for(&self, id: u64) -> Option<Arc<Rendered>> {
        self.done.lock().unwrap_or_else(|e| e.into_inner()).iter().find(|(k, _)| k.id == id).map(|(_, r)| r.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(r: &Rendered, x: f32, y: f32) -> [u8; 3] {
        let px = (x * r.scale) as u32;
        let py = (y * r.scale) as u32;
        let tile = r.tiles.iter().find(|t| py >= t.top && py < t.top + t.height).unwrap();
        let i = (((py - tile.top) * tile.width + px) * 4) as usize;
        [tile.rgba[i], tile.rgba[i + 1], tile.rgba[i + 2]]
    }

    fn body(html: &str) -> HtmlBody {
        HtmlBody { html: html.into(), inline: Vec::new() }
    }

    #[test]
    fn a_mail_wider_than_the_pane_is_scaled_to_fit() {
        let r = render(&body("<body style=\"margin:0\"><table width=\"600\" cellspacing=0 cellpadding=0><tr><td style=\"background:#0000ff;height:60px\"></td></tr></table></body>"), 300, 1.0, false);
        let edge = pixel(&r, 290.0, 10.0);
        assert!(edge[2] > 200 && edge[0] < 60, "right edge {edge:?}");
        assert!(r.height < 60.0 && r.height > 20.0, "height {}", r.height);
    }

    #[test]
    fn css_backgrounds_and_tables_are_laid_out() {
        let r = render(
            &body("<table width=\"100%\"><tr><td style=\"background:#ff0000;height:40px\">a</td><td style=\"background:#0000ff;height:40px\">b</td></tr></table>"),
            400,
            1.0,
            false,
        );
        assert!(r.height >= 40.0);
        let left = pixel(&r, 60.0, 36.0);
        let right = pixel(&r, 340.0, 36.0);
        assert!(left[0] > 200 && left[2] < 60, "left cell {left:?}");
        assert!(right[2] > 200 && right[0] < 60, "right cell {right:?}");
    }

    #[test]
    fn links_have_rects_and_only_safe_schemes() {
        let r = render(
            &body("<p><a href=\"https://example.com/track\">Track your parcel</a> <a href=\"javascript:alert(1)\">bad</a></p>"),
            400,
            1.0,
            false,
        );
        assert_eq!(r.links.len(), 1);
        assert_eq!(r.links[0].href, "https://example.com/track");
        assert!(r.links[0].rect[2] > 20.0 && r.links[0].rect[3] > 5.0);
    }

    #[test]
    fn scripts_forms_and_frames_are_dropped() {
        let html = sanitize("<p>ok</p><script>alert(1)</script><iframe src=\"https://x\"></iframe><form action=\"https://x\"><input name=a><p>kept</p></form>");
        assert!(!html.contains("script") && !html.contains("iframe") && !html.contains("input") && !html.contains("form"));
        assert!(html.contains("ok") && html.contains("kept"));
    }

    #[test]
    fn cid_images_load_and_remote_images_do_not_by_default() {
        let png = {
            let mut out = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 200, 0, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let mail = HtmlBody {
            html: "<body style=\"margin:0\"><img src=\"cid:logo@x\" width=20 height=20><img src=\"https://127.0.0.1:1/p.png\" width=20 height=20></body>".into(),
            inline: vec![("<logo@x>".into(), Arc::new(png))],
        };
        let r = render(&mail, 200, 1.0, false);
        let shown = pixel(&r, 10.0, 10.0);
        assert!(shown[1] > 150 && shown[0] < 60, "cid image {shown:?}");
    }

    #[test]
    fn remote_images_load_only_when_allowed() {
        let png = {
            let mut out = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(20, 20, image::Rgba([0, 0, 220, 255])).write_to(&mut out, image::ImageFormat::Png).unwrap();
            out.into_inner()
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(Mutex::new(0));
        let seen = hits.clone();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                *seen.lock().unwrap() += 1;
                let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", png.len());
                let _ = s.write_all(&png);
            }
        });
        let mail = body(&format!("<body style=\"margin:0\"><div style=\"height:40px\"><img src=\"http://127.0.0.1:{port}/p.png\" width=20 height=20></div></body>"));
        let blocked = render(&mail, 200, 1.0, false);
        assert_eq!(*hits.lock().unwrap(), 0, "a blocked image was fetched");
        assert!(pixel(&blocked, 10.0, 10.0)[0] > 200, "blocked image painted");
        let allowed = render(&mail, 200, 1.0, true);
        assert_eq!(*hits.lock().unwrap(), 1);
        let shown = pixel(&allowed, 10.0, 10.0);
        assert!(shown[2] > 200 && shown[0] < 40, "remote image {shown:?}");
    }

    #[test]
    fn renderer_thread_returns_the_page() {
        let renderer = Renderer::new(None);
        let key = RenderKey { id: 1, width: 300, scale_milli: 1000, remote: false };
        let html = body("<h1>Hello</h1>");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(r) = renderer.get(&key, &html) {
                assert!(!r.tiles.is_empty());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
