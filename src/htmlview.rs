use anyrender::{PaintScene, render_to_buffer};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, local_name};
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MAX_HTML: usize = 3_000_000;
const MAX_HEIGHT: f32 = 400_000.0;
const MAX_REMOTE_REQUESTS: usize = 48;
const MAX_REMOTE_TOTAL: u64 = 32 * 1024 * 1024;
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
    pub zoom_milli: u32,
    pub dark: bool,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub rect: [f32; 4],
    pub href: String,
    pub text: String,
    pub mismatch: Option<(String, String)>,
}

pub struct Tile {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Glyph {
    pub start: usize,
    pub end: usize,
    pub rect: [f32; 4],
}

#[derive(Clone, Debug, Default)]
pub struct Block {
    pub text: String,
    pub glyphs: Vec<Glyph>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextPos {
    pub block: usize,
    pub glyph: usize,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub rects: Vec<[f32; 4]>,
}

pub struct Rendered {
    pub key: RenderKey,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub dark: bool,
    pub tile_rows: Vec<(u32, u32)>,
    pub tiles: Mutex<Vec<TileSlot>>,
    pub links: Vec<Link>,
    pub blocks: Vec<Block>,
}

impl Rendered {
    fn empty(key: RenderKey, scale: f32) -> Rendered {
        Rendered {
            width: key.width as f32,
            height: 1.0,
            scale,
            dark: false,
            tile_rows: Vec::new(),
            tiles: Mutex::new(Vec::new()),
            links: Vec::new(),
            blocks: Vec::new(),
            key,
        }
    }

    pub fn plain_text(&self) -> String {
        self.blocks.iter().map(|b| b.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n")
    }

    pub fn find(&self, query: &str) -> Vec<Hit> {
        find_in(&self.blocks, query)
    }

    pub fn pos_at(&self, x: f32, y: f32) -> Option<TextPos> {
        let mut best: Option<(f32, TextPos)> = None;
        for (b, block) in self.blocks.iter().enumerate() {
            for (g, glyph) in block.glyphs.iter().enumerate() {
                let [rx, ry, rw, rh] = glyph.rect;
                let dx = (rx - x).max(x - (rx + rw)).max(0.0);
                let dy = (ry - y).max(y - (ry + rh)).max(0.0);
                let d = (dy * 8.0).round() * 100_000.0 + dx;
                if best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, TextPos { block: b, glyph: g }));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    pub fn selection_rects(&self, a: TextPos, b: TextPos) -> Vec<[f32; 4]> {
        let (from, to) = if a <= b { (a, b) } else { (b, a) };
        let mut rects = Vec::new();
        for bi in from.block..=to.block.min(self.blocks.len().saturating_sub(1)) {
            let Some(block) = self.blocks.get(bi) else { continue };
            let lo = if bi == from.block { from.glyph } else { 0 };
            let hi = if bi == to.block { to.glyph + 1 } else { block.glyphs.len() };
            merge_rects(&mut rects, block.glyphs.get(lo..hi.min(block.glyphs.len())).unwrap_or(&[]).iter().map(|g| g.rect));
        }
        rects
    }

    pub fn selected_text(&self, a: TextPos, b: TextPos) -> String {
        let (from, to) = if a <= b { (a, b) } else { (b, a) };
        let mut parts = Vec::new();
        for bi in from.block..=to.block.min(self.blocks.len().saturating_sub(1)) {
            let Some(block) = self.blocks.get(bi) else { continue };
            if block.glyphs.is_empty() {
                continue;
            }
            let lo = if bi == from.block { from.glyph } else { 0 }.min(block.glyphs.len() - 1);
            let hi = if bi == to.block { to.glyph } else { block.glyphs.len() - 1 }.min(block.glyphs.len() - 1);
            let start = block.glyphs[lo].start;
            let end = block.glyphs[hi.max(lo)].end.min(block.text.len());
            if let Some(t) = block.text.get(start..end) {
                let t = t.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
        }
        parts.join("\n")
    }
}

fn merge_rects(out: &mut Vec<[f32; 4]>, rects: impl Iterator<Item = [f32; 4]>) {
    for r in rects {
        if let Some(last) = out.last_mut()
            && (last[1] - r[1]).abs() < 1.0
            && (last[3] - r[3]).abs() < 1.0
            && r[0] >= last[0] - 1.0
            && r[0] <= last[0] + last[2] + 1.0
        {
            let right = (last[0] + last[2]).max(r[0] + r[2]);
            last[2] = right - last[0];
            continue;
        }
        out.push(r);
    }
}

fn find_in(blocks: &[Block], query: &str) -> Vec<Hit> {
    let needle: String = query.trim().chars().flat_map(|c| c.to_lowercase()).collect();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for block in blocks {
        let mut low = String::new();
        let mut map: Vec<usize> = Vec::new();
        for (i, c) in block.text.char_indices() {
            for l in c.to_lowercase() {
                let before = low.len();
                low.push(l);
                map.extend(std::iter::repeat_n(i, low.len() - before));
            }
        }
        map.push(block.text.len());
        for (at, _) in low.match_indices(&needle) {
            let (start, end) = (map[at], map[at + needle.len()]);
            let mut rects = Vec::new();
            merge_rects(&mut rects, block.glyphs.iter().filter(|g| g.start < end && g.end > start).map(|g| g.rect));
            if !rects.is_empty() {
                hits.push(Hit { rects });
            }
        }
    }
    hits
}

pub fn link_mismatch(text: &str, href: &str) -> Option<(String, String)> {
    let real = url::Url::parse(href).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?.host_str()?.to_ascii_lowercase();
    let text = text.trim();
    if text.is_empty() || text.contains(char::is_whitespace) {
        return None;
    }
    let candidate = if text.contains("://") { text.to_string() } else { format!("https://{text}") };
    let shown = url::Url::parse(&candidate).ok()?.host_str()?.to_ascii_lowercase();
    let tld = shown.rsplit('.').next().unwrap_or("");
    if !shown.contains('.') || tld.len() < 2 || !tld.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let plain = |h: &str| h.strip_prefix("www.").unwrap_or(h).to_string();
    let (s, r) = (plain(&shown), plain(&real));
    let related = s == r || s.ends_with(&format!(".{r}")) || r.ends_with(&format!(".{s}"));
    (!related).then_some((s, r))
}

enum Job {
    Page { key: RenderKey, body: HtmlBody },
    Tile { key: RenderKey, index: usize },
}

struct Resources {
    inline: HashMap<String, Arc<Vec<u8>>>,
    remote: bool,
    pending: Mutex<usize>,
    requests: AtomicUsize,
    bytes: AtomicU64,
    max_requests: usize,
    max_bytes: u64,
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
            "http" | "https" if self.remote => self.fetch_capped(url.as_str()),
            _ => None,
        }
    }

    fn fetch_capped(&self, url: &str) -> Option<Vec<u8>> {
        if self.requests.fetch_add(1, Ordering::SeqCst) >= self.max_requests {
            return None;
        }
        let used = self.bytes.load(Ordering::SeqCst);
        let room = self.max_bytes.saturating_sub(used).min(REMOTE_MAX);
        if room == 0 {
            return None;
        }
        let out = fetch_remote(url, room)?;
        self.bytes.fetch_add(out.len() as u64, Ordering::SeqCst);
        Some(out)
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

fn fetch_remote(url: &str, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let agent = ureq::AgentBuilder::new().timeout(REMOTE_TIMEOUT).redirects(4).build();
    let resp = agent.get(url).call().ok()?;
    let mut out = Vec::new();
    resp.into_reader().take(limit).read_to_end(&mut out).ok()?;
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
            crate::runtime::handle().spawn_blocking(move || {
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

const MAIL_CSS: &str = "body { margin: 16px; color: #1a1a1a; font-family: sans-serif; font-size: 14px; line-height: 1.45; overflow-wrap: anywhere; } img { max-width: 100%; height: auto; } pre { white-space: pre-wrap; }";
const LIGHT_SHEET: &str = "html { background: #ffffff; }";
const DARK_SHEET: &str = "html { background: #16181c; } body { color: #d9dce1; } a { color: #82b4ff; } a:visited { color: #b9a2ff; }";
pub const DARK_BG: [u8; 3] = [0x16, 0x18, 0x1c];

pub fn sets_own_colours(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    for tag in ["<body", "<html"] {
        if let Some(at) = lower.find(tag) {
            let end = lower[at..].find('>').map_or(lower.len(), |e| at + e);
            let attrs = &lower[at..end];
            if attrs.contains("bgcolor") || attrs.contains("background") || attrs.contains("color") || attrs.contains("text=") {
                return true;
            }
        }
    }
    let mut rest = lower.as_str();
    while let Some(open) = rest.find("<style") {
        let Some(gt) = rest[open..].find('>') else { break };
        let from = open + gt + 1;
        let Some(close) = rest[from..].find("</style") else { break };
        if sheet_colours_root(&rest[from..from + close]) {
            return true;
        }
        rest = &rest[from + close..];
    }
    false
}

fn sheet_colours_root(sheet: &str) -> bool {
    let cleaned = sheet.replace("@media", " ");
    cleaned.split('}').any(|rule| {
        let Some((selectors, decls)) = rule.rsplit_once('{') else { return false };
        let selectors = selectors.rsplit('{').next().unwrap_or(selectors);
        let root = selectors.split(',').any(|s| {
            let s = s.trim();
            let last = s.rsplit([' ', '>']).next().unwrap_or(s);
            matches!(last, "body" | "html" | "*" | ":root")
        });
        root && (decls.contains("background") || decls.split(';').any(|d| d.trim_start().starts_with("color")))
    })
}

fn wrap(html: &str, dark: bool) -> String {
    let lower = html.to_ascii_lowercase();
    let sheet = if dark { DARK_SHEET } else { LIGHT_SHEET };
    let style = format!("<style>{sheet}{MAIL_CSS}</style>");
    if let Some(h) = lower.find("<head") {
        let at = lower[h..].find('>').map_or(html.len(), |e| h + e + 1);
        format!("{}{style}{}", &html[..at], &html[at..])
    } else {
        format!("<html><head>{style}</head><body>{html}</body></html>")
    }
}

fn scheme(dark: bool) -> ColorScheme {
    if dark { ColorScheme::Dark } else { ColorScheme::Light }
}

fn document(body: &HtmlBody, key: &RenderKey, dark: bool) -> (BaseDocument, Arc<Resources>) {
    let resources = Arc::new(Resources {
        inline: body.inline.iter().map(|(k, v)| (k.trim_matches(['<', '>']).to_ascii_lowercase(), v.clone())).collect(),
        remote: key.remote,
        pending: Mutex::new(0),
        requests: AtomicUsize::new(0),
        bytes: AtomicU64::new(0),
        max_requests: MAX_REMOTE_REQUESTS,
        max_bytes: MAX_REMOTE_TOTAL,
    });
    let net = Arc::new(MailNet { resources: resources.clone() });
    let scale = key.scale_milli as f32 / 1000.0 * key.zoom_milli as f32 / 1000.0;
    let layout_w = key.width as f32 / (key.zoom_milli as f32 / 1000.0);
    let doc = HtmlDocument::from_html(
        &wrap(&sanitize(&body.html), dark),
        DocumentConfig {
            base_url: Some("cid:mail/".to_string()),
            net_provider: Some(net as _),
            viewport: Some(Viewport::new((layout_w * scale) as u32, (800.0 * scale) as u32, scale, scheme(key.dark))),
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
        let text = node.text_content().trim().to_string();
        let mismatch = link_mismatch(&text, href);
        for r in doc.node_client_rects(id) {
            if r.width > 0.0 && r.height > 0.0 {
                out.push(Link {
                    rect: [r.x as f32, r.y as f32, r.width as f32, r.height as f32],
                    href: href.to_string(),
                    text: text.clone(),
                    mismatch: mismatch.clone(),
                });
            }
        }
    }
    out
}

fn text_blocks(doc: &BaseDocument) -> Vec<Block> {
    let mut out = Vec::new();
    let Some(root) = doc.try_root_element() else { return out };
    let mut stack = vec![root.id];
    while let Some(id) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        let kids: Vec<_> = match node.layout_children.borrow().as_ref() {
            Some(c) => c.iter().copied().collect(),
            None => node.children.iter().copied().collect(),
        };
        stack.extend(kids.into_iter().rev());
        if !node.flags.is_inline_root() {
            continue;
        }
        let Some(layout_data) = node.element_data().and_then(|d| d.inline_layout_data.as_ref()) else { continue };
        let Some(bounds) = doc.get_client_bounding_rect(id) else { continue };
        let l = node.unrounded_layout();
        let ox = bounds.x as f32 + l.border.left + l.padding.left;
        let oy = bounds.y as f32 + l.border.top + l.padding.top + layout_data.block_offset;
        let layout = &layout_data.layout;
        let scale = layout.scale();
        let mut glyphs = Vec::new();
        let mut covered = 0;
        for (i, _) in layout_data.text.char_indices() {
            if i < covered {
                continue;
            }
            let Some(cluster) = parley::Cluster::from_byte_index(layout, i) else { continue };
            let range = cluster.text_range();
            covered = range.end.max(i + 1);
            let Some(offset) = cluster.visual_offset() else { continue };
            let m = *cluster.line().metrics();
            glyphs.push(Glyph {
                start: range.start,
                end: range.end,
                rect: [
                    ox + offset / scale,
                    oy + m.block_min_coord / scale,
                    cluster.advance() / scale,
                    (m.block_max_coord - m.block_min_coord) / scale,
                ],
            });
        }
        if !glyphs.is_empty() {
            out.push(Block { text: layout_data.text.clone(), glyphs });
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

pub struct DocState {
    doc: BaseDocument,
    paint: f32,
    px_w: u32,
    px_h: u32,
    fill: [u8; 3],
}

pub fn render(body: &HtmlBody, key: &RenderKey) -> (Rendered, DocState) {
    let scale = key.scale_milli as f32 / 1000.0;
    let zoom = (key.zoom_milli as f32 / 1000.0).max(0.1);
    let z = scale * zoom;
    let width = key.width.max(64);
    let layout_width = width as f32 / zoom;
    let dark = key.dark && !sets_own_colours(&body.html);
    let (mut doc, resources) = document(body, key, dark);
    settle(&mut doc, &resources);
    let content_w = doc.try_root_element().map_or(0.0, |_| widest(&doc));
    let content_h = doc.try_root_element().map_or(0.0, |r| r.final_layout().size.height);
    let fit = if content_w > layout_width + 1.0 { layout_width / content_w } else { 1.0 };
    let layout_w = if fit < 1.0 { content_w.ceil() } else { layout_width };
    if fit < 1.0 {
        doc.set_viewport(Viewport::new((layout_w * z) as u32, (800.0 * z) as u32, z, scheme(key.dark)));
        doc.resolve(0.0);
    }
    let content_h = if fit < 1.0 { doc.try_root_element().map_or(content_h, |r| r.final_layout().size.height) } else { content_h };
    let height = content_h.clamp(1.0, MAX_HEIGHT);
    let paint = z * fit;
    let px_w = (width as f32 * scale).ceil() as u32;
    let px_h = (height * paint).ceil() as u32;
    doc.set_viewport(Viewport::new(px_w, px_h, paint, scheme(key.dark)));
    doc.resolve(0.0);
    let to_points = fit * zoom;
    let links = links(&doc)
        .into_iter()
        .map(|l| Link { rect: l.rect.map(|v| v * to_points), ..l })
        .collect();
    let blocks = text_blocks(&doc)
        .into_iter()
        .map(|mut b| {
            for g in &mut b.glyphs {
                g.rect = g.rect.map(|v| v * to_points);
            }
            b
        })
        .collect();
    let mut tile_rows = Vec::new();
    let mut top = 0;
    while top < px_h {
        let h = TILE.min(px_h - top);
        tile_rows.push((top, h));
        top += h;
    }
    let slots = tile_rows.iter().map(|_| TileSlot::Missing).collect();
    let page = Rendered {
        key: key.clone(),
        width: width as f32,
        height: height * to_points,
        scale,
        dark,
        tile_rows,
        tiles: Mutex::new(slots),
        links,
        blocks,
    };
    let fill = if dark { DARK_BG } else { [255, 255, 255] };
    (page, DocState { doc, paint, px_w, px_h, fill })
}

fn paint_tile(state: &mut DocState, top: u32, h: u32) -> Tile {
    let (paint, px_w, fill) = (state.paint, state.px_w, state.fill);
    let doc = &mut state.doc;
    let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            scene.fill(
                peniko::Fill::NonZero,
                Default::default(),
                peniko::Color::from_rgba8(fill[0], fill[1], fill[2], 255),
                None,
                &peniko::kurbo::Rect::new(0.0, 0.0, px_w as f64, h as f64),
            );
            blitz_paint::paint_scene(scene, doc, paint as f64, px_w, h, 0, top);
        },
        px_w,
        h,
    );
    Tile { width: px_w, height: h, rgba }
}

pub enum TileSlot {
    Missing,
    Requested,
    Ready(Arc<Tile>),
}

pub struct Renderer {
    jobs: Sender<Job>,
    done: Arc<Mutex<HashMap<RenderKey, Arc<Rendered>>>>,
    asked: Arc<Mutex<Vec<RenderKey>>>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new(None)
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Renderer {
    pub fn new(ctx: Option<egui::Context>) -> Renderer {
        let (jobs, rx): (Sender<Job>, Receiver<Job>) = channel();
        let done: Arc<Mutex<HashMap<RenderKey, Arc<Rendered>>>> = Arc::default();
        let asked: Arc<Mutex<Vec<RenderKey>>> = Arc::default();
        let out = done.clone();
        let asked_out = asked.clone();
        std::thread::Builder::new()
            .name("html-render".into())
            .spawn(move || {
                let mut current: Option<(RenderKey, DocState)> = None;
                while let Ok(first) = rx.recv() {
                    let mut page: Option<(RenderKey, HtmlBody)> = None;
                    let mut tiles: Vec<(RenderKey, usize)> = Vec::new();
                    let mut take = |job: Job, page: &mut Option<(RenderKey, HtmlBody)>| match job {
                        Job::Page { key, body } => {
                            if let Some((old, _)) = page.replace((key, body)) {
                                lock(&asked_out).retain(|k| *k != old);
                            }
                        }
                        Job::Tile { key, index } => {
                            if !tiles.contains(&(key.clone(), index)) {
                                tiles.push((key, index));
                            }
                        }
                    };
                    take(first, &mut page);
                    while let Ok(next) = rx.try_recv() {
                        take(next, &mut page);
                    }
                    if let Some((key, body)) = page {
                        let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&body, &key)));
                        match rendered {
                            Ok((rendered, state)) => {
                                current = Some((key.clone(), state));
                                let mut done = lock(&out);
                                if done.len() > 8 {
                                    done.clear();
                                }
                                done.insert(key.clone(), Arc::new(rendered));
                            }
                            Err(_) => {
                                let scale = key.scale_milli as f32 / 1000.0;
                                lock(&out).insert(key.clone(), Arc::new(Rendered::empty(key.clone(), scale)));
                            }
                        }
                        lock(&asked_out).retain(|k| *k != key);
                    }
                    for (key, index) in tiles {
                        let page = lock(&out).get(&key).cloned();
                        let Some(page) = page else { continue };
                        let Some((_, state)) = current.as_mut().filter(|(k, _)| *k == key) else {
                            lock(&out).remove(&key);
                            continue;
                        };
                        let Some(&(top, h)) = page.tile_rows.get(index) else { continue };
                        let tile = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| paint_tile(state, top, h)));
                        if let (Ok(tile), Some(slot)) = (tile, lock(&page.tiles).get_mut(index)) {
                            *slot = TileSlot::Ready(Arc::new(tile));
                        }
                    }
                    if let Some(ctx) = &ctx {
                        ctx.request_repaint();
                    }
                }
            })
            .expect("render thread");
        Renderer { jobs, done, asked }
    }

    pub fn get(&self, key: &RenderKey, body: &HtmlBody) -> Option<Arc<Rendered>> {
        if let Some(r) = lock(&self.done).get(key) {
            return Some(r.clone());
        }
        let mut asked = lock(&self.asked);
        if !asked.contains(key) {
            asked.push(key.clone());
            if asked.len() > 32 {
                asked.remove(0);
            }
            let _ = self.jobs.send(Job::Page { key: key.clone(), body: body.clone() });
        }
        None
    }

    pub fn tile(&self, page: &Rendered, index: usize) -> Option<Arc<Tile>> {
        let mut tiles = lock(&page.tiles);
        match tiles.get_mut(index)? {
            TileSlot::Ready(t) => Some(t.clone()),
            TileSlot::Requested => None,
            slot @ TileSlot::Missing => {
                *slot = TileSlot::Requested;
                let _ = self.jobs.send(Job::Tile { key: page.key.clone(), index });
                None
            }
        }
    }

    pub fn keep_tiles(&self, page: &Rendered, first: usize, last: usize) {
        for (i, slot) in lock(&page.tiles).iter_mut().enumerate() {
            if (i < first || i > last) && matches!(slot, TileSlot::Ready(_)) {
                *slot = TileSlot::Missing;
            }
        }
    }

    pub fn latest_for(&self, id: u64) -> Option<Arc<Rendered>> {
        lock(&self.done).iter().find(|(k, _)| k.id == id).map(|(_, r)| r.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Shot {
        page: Rendered,
        tiles: Vec<(u32, Tile)>,
    }

    impl std::ops::Deref for Shot {
        type Target = Rendered;
        fn deref(&self) -> &Rendered {
            &self.page
        }
    }

    fn key(width: u32, scale: f32, remote: bool) -> RenderKey {
        RenderKey { id: 1, width, scale_milli: (scale * 1000.0) as u32, remote, zoom_milli: 1000, dark: false }
    }

    fn shoot(html: &HtmlBody, key: RenderKey) -> Shot {
        let (page, mut state) = render(html, &key);
        let tiles = page.tile_rows.clone().into_iter().map(|(top, h)| (top, paint_tile(&mut state, top, h))).collect();
        Shot { page, tiles }
    }

    fn shot(html: &HtmlBody, width: u32, scale: f32, remote: bool) -> Shot {
        shoot(html, key(width, scale, remote))
    }

    fn pixel(r: &Shot, x: f32, y: f32) -> [u8; 3] {
        let px = (x * r.scale) as u32;
        let py = (y * r.scale) as u32;
        let (top, tile) = r.tiles.iter().find(|(top, t)| py >= *top && py < top + t.height).unwrap();
        let i = (((py - top) * tile.width + px) * 4) as usize;
        [tile.rgba[i], tile.rgba[i + 1], tile.rgba[i + 2]]
    }

    fn body(html: &str) -> HtmlBody {
        HtmlBody { html: html.into(), inline: Vec::new() }
    }

    #[test]
    fn a_mail_wider_than_the_pane_is_scaled_to_fit() {
        let r = shot(&body("<body style=\"margin:0\"><table width=\"600\" cellspacing=0 cellpadding=0><tr><td style=\"background:#0000ff;height:60px\"></td></tr></table></body>"), 300, 1.0, false);
        let edge = pixel(&r, 290.0, 10.0);
        assert!(edge[2] > 200 && edge[0] < 60, "right edge {edge:?}");
        assert!(r.height < 60.0 && r.height > 20.0, "height {}", r.height);
    }

    #[test]
    fn css_backgrounds_and_tables_are_laid_out() {
        let r = shot(
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
        let r = shot(
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
        let r = shot(&mail, 200, 1.0, false);
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
        let blocked = shot(&mail, 200, 1.0, false);
        assert_eq!(*hits.lock().unwrap(), 0, "a blocked image was fetched");
        assert!(pixel(&blocked, 10.0, 10.0)[0] > 200, "blocked image painted");
        let allowed = shot(&mail, 200, 1.0, true);
        assert_eq!(*hits.lock().unwrap(), 1);
        let shown = pixel(&allowed, 10.0, 10.0);
        assert!(shown[2] > 200 && shown[0] < 40, "remote image {shown:?}");
    }

    #[test]
    #[ignore = "writes MAILBENCH_FONT_PNG to show which system fonts Blitz picked"]
    fn system_font_sample_is_written_to_a_png() {
        let out = std::env::var("MAILBENCH_FONT_PNG").expect("MAILBENCH_FONT_PNG");
        let html = body("<body style=\"margin:8px;background:#fff;color:#000\"><p style=\"font:28px 'Segoe UI'\">Segoe UI: The quick brown fox 0123456789</p><p style=\"font:28px Arial\">Arial: The quick brown fox 0123456789</p><p style=\"font:28px 'Helvetica Neue'\">Helvetica Neue: The quick brown fox</p><p style=\"font:28px sans-serif\">sans-serif: The quick brown fox</p><p style=\"font:28px serif\">serif: The quick brown fox</p><p style=\"font:28px monospace\">monospace: The quick brown fox</p><p style=\"font:28px sans-serif\">Accents: caf\u{e9} na\u{ef}ve \u{fc}ber, \u{20ac}5 \u{2014} \u{201c}quotes\u{201d}</p></body>");
        let r = shot(&html, 900, 1.0, false);
        let height: u32 = r.tiles.iter().map(|(top, t)| top + t.height).max().unwrap();
        let mut img = image::RgbaImage::from_pixel(r.tiles[0].1.width, height, image::Rgba([255, 255, 255, 255]));
        for (top, t) in &r.tiles {
            let tile = image::RgbaImage::from_raw(t.width, t.height, t.rgba.clone()).unwrap();
            image::imageops::replace(&mut img, &tile, 0, *top as i64);
        }
        img.save(out).unwrap();
    }

    #[test]
    fn renderer_thread_returns_the_page() {
        let renderer = Renderer::new(None);
        let key = key(300, 1.0, false);
        let html = body("<h1>Hello</h1>");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(r) = renderer.get(&key, &html) {
                assert!(!r.tile_rows.is_empty());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn find_returns_rects_over_the_matching_text() {
        let r = shot(&body("<body style=\"margin:0\"><p style=\"margin:0\">alpha beta</p><p style=\"margin:0\">gamma Beta</p></body>"), 300, 1.0, false);
        let hits = r.find("beta");
        assert_eq!(hits.len(), 2);
        let first = hits[0].rects[0];
        let second = hits[1].rects[0];
        assert!(first[0] > 20.0 && first[2] > 10.0, "first {first:?}");
        assert!(second[1] > first[1] + 5.0, "second below first {first:?} {second:?}");
        assert!(r.find("zzz").is_empty());
        assert!(r.find("  ").is_empty());
    }

    #[test]
    fn selected_text_follows_the_drag() {
        let r = shot(&body("<body style=\"margin:0\"><p style=\"margin:0\">alpha beta</p><p style=\"margin:0\">gamma</p></body>"), 300, 1.0, false);
        assert_eq!(r.plain_text(), "alpha beta\ngamma");
        let a = r.pos_at(0.0, 5.0).unwrap();
        let b = r.pos_at(250.0, 25.0).unwrap();
        assert_eq!(r.selected_text(a, b), "alpha beta\ngamma");
        let hit = &r.find("beta")[0].rects[0];
        let from = r.pos_at(hit[0] + 1.0, hit[1] + 2.0).unwrap();
        assert_eq!(r.selected_text(from, from), "b");
    }

    #[test]
    fn dark_sheet_applies_only_when_the_mail_sets_no_colours() {
        let plain = body("<p>hello</p>");
        let dark = |b: &HtmlBody| shoot(b, RenderKey { dark: true, ..key(200, 1.0, false) });
        assert!(!sets_own_colours(&plain.html));
        let r = dark(&plain);
        assert!(r.dark);
        let px = pixel(&r, 150.0, 5.0);
        assert!(px[0] < 60 && px[1] < 60 && px[2] < 60, "dark page {px:?}");
        let own = body("<body bgcolor=\"#ffffff\"><p>hello</p></body>");
        assert!(sets_own_colours(&own.html));
        let r = dark(&own);
        assert!(!r.dark);
        assert!(pixel(&r, 150.0, 5.0)[0] > 200);
        assert!(sets_own_colours("<style>body { background: #fff }</style><p>x</p>"));
        assert!(sets_own_colours("<style>.a{color:red} html, body {color: #000}</style>"));
        assert!(!sets_own_colours("<style>.a{background:red}</style><p>x</p>"));
        let light = shot(&plain, 200, 1.0, false);
        assert!(pixel(&light, 150.0, 5.0)[0] > 200);
    }

    #[test]
    fn link_text_naming_another_domain_is_flagged() {
        assert_eq!(link_mismatch("paypal.com", "https://evil.example/login"), Some(("paypal.com".into(), "evil.example".into())));
        assert!(link_mismatch("https://www.paypal.com/x", "https://paypal.com/y").is_none());
        assert!(link_mismatch("login.paypal.com", "https://paypal.com/y").is_none());
        assert!(link_mismatch("Track your parcel", "https://evil.example").is_none());
        assert!(link_mismatch("Reset password", "https://evil.example").is_none());
        assert!(link_mismatch("evil.example", "https://evil.example/a").is_none());
        let r = shot(&body("<p><a href=\"https://evil.example/x\">bank.com</a></p>"), 300, 1.0, false);
        assert_eq!(r.links[0].mismatch, Some(("bank.com".into(), "evil.example".into())));
    }

    #[test]
    fn zoom_scales_the_page() {
        let html = body("<body style=\"margin:0\"><div style=\"height:50px\"></div></body>");
        let normal = shot(&html, 300, 1.0, false);
        let zoomed = shoot(&html, RenderKey { zoom_milli: 2000, ..key(300, 1.0, false) });
        assert!((zoomed.height / normal.height - 2.0).abs() < 0.05, "{} vs {}", zoomed.height, normal.height);
    }

    #[test]
    fn tall_mails_render_past_the_old_cap_as_lazy_tiles() {
        let r = shot(&body("<body style=\"margin:0\"><div style=\"height:40000px\"></div></body>"), 200, 1.0, false);
        assert!(r.height > 39_000.0, "height {}", r.height);
        assert!(r.tile_rows.len() > 10);
    }

    #[test]
    fn remote_fetches_stop_at_the_per_mail_cap() {
        let res = Resources {
            inline: HashMap::new(),
            remote: true,
            pending: Mutex::new(0),
            requests: AtomicUsize::new(0),
            bytes: AtomicU64::new(0),
            max_requests: 2,
            max_bytes: 10,
        };
        assert!(res.fetch_capped("http://127.0.0.1:1/a").is_none());
        assert!(res.fetch_capped("http://127.0.0.1:1/b").is_none());
        assert_eq!(res.requests.load(Ordering::SeqCst), 2);
        res.bytes.store(10, Ordering::SeqCst);
        assert!(res.fetch_capped("http://127.0.0.1:1/c").is_none());
    }
}
