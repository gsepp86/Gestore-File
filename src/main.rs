// Su Windows non mostra la finestra nera del terminale
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Rect, Sense, Stroke, Vec2};
use std::{
    cmp::Ordering,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::SystemTime,
};

// ───────────────────────── Tema Material ─────────────────────────

#[derive(Clone, Copy)]
struct Pal {
    bg: Color32,
    surface: Color32,
    container: Color32,
    primary: Color32,
    on_primary: Color32,
    primary_container: Color32,
    sec_container: Color32,
    text: Color32,
    weak: Color32,
    outline: Color32,
}

fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

fn palette(dark: bool) -> Pal {
    if dark {
        Pal {
            bg: rgb(0x141218),
            surface: rgb(0x1D1B20),
            container: rgb(0x2B2930),
            primary: rgb(0xD0BCFF),
            on_primary: rgb(0x381E72),
            primary_container: rgb(0x4F378B),
            sec_container: rgb(0x4A4458),
            text: rgb(0xE6E0E9),
            weak: rgb(0xCAC4D0),
            outline: rgb(0x49454F),
        }
    } else {
        Pal {
            bg: rgb(0xFEF7FF),
            surface: rgb(0xF7F2FA),
            container: rgb(0xECE6F0),
            primary: rgb(0x6750A4),
            on_primary: rgb(0xFFFFFF),
            primary_container: rgb(0xEADDFF),
            sec_container: rgb(0xE8DEF8),
            text: rgb(0x1D1B20),
            weak: rgb(0x49454F),
            outline: rgb(0xCAC4D0),
        }
    }
}

fn apply_theme(ctx: &egui::Context, dark: bool) {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.surface;
    v.window_fill = p.container;
    v.extreme_bg_color = p.container;
    v.faint_bg_color = p.surface;
    v.window_stroke = Stroke::new(1.0, p.outline);
    v.selection.bg_fill = p.primary_container;
    v.selection.stroke = Stroke::new(1.0, p.primary);
    v.hyperlink_color = p.primary;

    let pill = CornerRadius::same(20);
    let states = [
        (&mut v.widgets.noninteractive, p.surface, p.container),
        (&mut v.widgets.inactive, p.container, p.container),
        (&mut v.widgets.hovered, p.sec_container, p.sec_container),
        (&mut v.widgets.active, p.primary_container, p.primary_container),
    ];
    for (w, weak_bg, bg) in states {
        w.weak_bg_fill = weak_bg;
        w.bg_fill = bg;
        w.fg_stroke = Stroke::new(1.0, p.text);
        w.corner_radius = pill;
        w.bg_stroke = Stroke::new(1.0, Color32::TRANSPARENT);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.outline);
    v.widgets.noninteractive.corner_radius = CornerRadius::same(12);
    v.window_corner_radius = CornerRadius::same(24);
    v.menu_corner_radius = CornerRadius::same(16);
    ctx.set_visuals(v);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(14.0, 6.0);
    ctx.set_style(style);
}

// ───────────────────────── Dati ─────────────────────────

#[derive(Clone)]
struct Entry {
    name: String,
    lname: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    modified: Option<SystemTime>,
    /// Su Windows = data in cui il file è stato salvato/scaricato sul disco
    saved: Option<SystemTime>,
    kind: String,
    ext: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Saved,
    Modified,
    Name,
    Size,
    Kind,
}

impl SortKey {
    const ALL: [SortKey; 5] = [SortKey::Saved, SortKey::Modified, SortKey::Name, SortKey::Size, SortKey::Kind];
    fn label(self) -> &'static str {
        match self {
            SortKey::Saved => "Data download/salvataggio",
            SortKey::Modified => "Data modifica",
            SortKey::Name => "Nome",
            SortKey::Size => "Dimensione",
            SortKey::Kind => "Tipo",
        }
    }
    fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::Kind)
    }
}

struct Loaded {
    tab_id: u64,
    load_id: u64,
    path: PathBuf,
    result: Result<Vec<Entry>, String>,
}

// ───────────────────────── Utilità ─────────────────────────

fn is_hidden(md: &std::fs::Metadata, name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let _ = name;
        md.file_attributes() & 0x2 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = md;
        name.starts_with('.')
    }
}

fn read_entries(path: &Path, show_hidden: bool) -> Result<Vec<Entry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut out = Vec::with_capacity(256);
    for de in rd.flatten() {
        let Ok(md) = de.metadata() else { continue };
        let name = de.file_name().to_string_lossy().into_owned();
        if !show_hidden && is_hidden(&md, &name) {
            continue;
        }
        let is_dir = md.is_dir();
        let ext = if is_dir {
            String::new()
        } else {
            Path::new(&name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        };
        let kind = if is_dir {
            "Cartella".to_string()
        } else if ext.is_empty() {
            "File".to_string()
        } else {
            format!("File {}", ext.to_uppercase())
        };
        let modified = md.modified().ok();
        out.push(Entry {
            lname: name.to_lowercase(),
            name,
            path: de.path(),
            is_dir,
            size: if is_dir { 0 } else { md.len() },
            saved: md.created().ok().or(modified),
            modified,
            kind,
            ext,
        });
    }
    Ok(out)
}

/// Confronto "naturale": file2 < file10
fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (mut na, mut nb) = (String::new(), String::new());
                while let Some(&c) = ai.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    na.push(c);
                    ai.next();
                }
                while let Some(&c) = bi.peek() {
                    if !c.is_ascii_digit() {
                        break;
                    }
                    nb.push(c);
                    bi.next();
                }
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let o = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if o != Ordering::Equal {
                    return o;
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn fmt_size(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{} B", b)
    } else {
        format!("{:.1} {}", v, U[i])
    }
}

fn fmt_time(t: Option<SystemTime>) -> String {
    match t {
        Some(t) => chrono::DateTime::<chrono::Local>::from(t).format("%d/%m/%Y %H:%M").to_string(),
        None => "—".into(),
    }
}

/// Colore dell'icona in base al tipo di file (stile Material colorato)
fn kind_color(ext: &str) -> Color32 {
    match ext {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "heic" | "ico" => rgb(0x2E9E5B),
        "mp3" | "wav" | "flac" | "m4a" | "ogg" | "aac" => rgb(0xE91E63),
        "mp4" | "mkv" | "avi" | "mov" | "webm" => rgb(0x8E44AD),
        "zip" | "rar" | "7z" | "tar" | "gz" => rgb(0xF57C00),
        "exe" | "msi" | "bat" | "cmd" => rgb(0x546E7A),
        "pdf" => rgb(0xE53935),
        "doc" | "docx" | "odt" | "rtf" => rgb(0x1E88E5),
        "xls" | "xlsx" | "csv" | "ods" => rgb(0x2E7D32),
        "ppt" | "pptx" | "odp" => rgb(0xE64A19),
        "txt" | "md" | "log" | "ini" | "cfg" => rgb(0x78909C),
        "rs" | "py" | "js" | "ts" | "c" | "cpp" | "h" | "java" | "html" | "css" | "json" | "xml" | "toml" | "yml" | "yaml" => {
            rgb(0x00ACC1)
        }
        _ => rgb(0x7E57C2),
    }
}

fn draw_folder(painter: &egui::Painter, r: Rect) {
    let (w, h) = (r.width(), r.height());
    let back = rgb(0xFFA000);
    let front = rgb(0xFFCA28);
    painter.rect_filled(Rect::from_min_size(r.min + Vec2::new(0.0, h * 0.1), Vec2::new(w * 0.45, h * 0.3)), 2.0, back);
    painter.rect_filled(Rect::from_min_size(r.min + Vec2::new(0.0, h * 0.2), Vec2::new(w, h * 0.78)), 3.0, back);
    painter.rect_filled(Rect::from_min_size(r.min + Vec2::new(0.0, h * 0.34), Vec2::new(w, h * 0.66)), 3.0, front);
}

fn draw_file_icon(painter: &egui::Painter, r: Rect, ext: &str) {
    let col = kind_color(ext);
    painter.rect_filled(r, 5.0, col);
    let label: String = if ext.is_empty() { "•".into() } else { ext.chars().take(3).collect::<String>().to_uppercase() };
    painter.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(8.5), Color32::WHITE);
}

fn icon_chip(ui: &mut egui::Ui, color: Color32, glyph: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 12.0, color);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), Color32::WHITE);
}

// ───────────────────────── Anteprima ─────────────────────────

enum PreviewKind {
    Image(String),
    Text(String),
    Folder(usize),
    None(&'static str),
}

struct Preview {
    path: PathBuf,
    kind: PreviewKind,
}

const TEXT_EXT: &[&str] = &[
    "txt", "md", "rs", "toml", "json", "csv", "log", "ini", "xml", "html", "htm", "css", "js", "ts", "py", "bat", "cmd", "yaml", "yml", "c", "cpp", "h", "java", "cfg", "sh", "sql",
];
const IMG_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"];

fn build_preview(e: &Entry) -> Preview {
    let kind = if e.is_dir {
        let n = std::fs::read_dir(&e.path).map(|r| r.take(100_000).count()).unwrap_or(0);
        PreviewKind::Folder(n)
    } else if IMG_EXT.contains(&e.ext.as_str()) {
        if e.size > 40 * 1024 * 1024 {
            PreviewKind::None("Immagine troppo grande per l'anteprima")
        } else {
            PreviewKind::Image(format!("file://{}", e.path.display().to_string().replace('\\', "/")))
        }
    } else if TEXT_EXT.contains(&e.ext.as_str()) {
        let mut buf = Vec::new();
        match std::fs::File::open(&e.path).and_then(|f| f.take(32 * 1024).read_to_end(&mut buf)) {
            Ok(_) => PreviewKind::Text(String::from_utf8_lossy(&buf).into_owned()),
            Err(_) => PreviewKind::None("Impossibile leggere il file"),
        }
    } else {
        PreviewKind::None("Anteprima non disponibile per questo tipo di file")
    };
    Preview { path: e.path.clone(), kind }
}

// ───────────────────────── Scheda ─────────────────────────

struct Tab {
    id: u64,
    cwd: PathBuf,
    history: Vec<PathBuf>,
    hist_pos: usize,
    entries: Vec<Entry>,
    view: Vec<usize>,
    dirty: bool,
    loading: bool,
    error: Option<String>,
    load_id: u64,
    addr: String,
    filter: String,
    sort_key: SortKey,
    ascending: bool,
    folders_first: bool,
    selected: Option<PathBuf>,
}

impl Tab {
    fn new(id: u64, cwd: PathBuf) -> Self {
        Tab {
            id,
            addr: cwd.display().to_string(),
            history: vec![cwd.clone()],
            hist_pos: 0,
            cwd,
            entries: vec![],
            view: vec![],
            dirty: true,
            loading: false,
            error: None,
            load_id: 0,
            filter: String::new(),
            sort_key: SortKey::Saved,
            ascending: false, // più recenti in alto
            folders_first: false,
            selected: None,
        }
    }

    fn title(&self) -> String {
        match self.cwd.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => self.cwd.display().to_string(),
        }
    }

    fn load(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool) {
        self.load_id += 1;
        self.loading = true;
        self.error = None;
        self.selected = None;
        let (tab_id, load_id, path, tx, ctx) = (self.id, self.load_id, self.cwd.clone(), tx.clone(), ctx.clone());
        thread::spawn(move || {
            let result = read_entries(&path, hidden);
            let _ = tx.send(Loaded { tab_id, load_id, path, result });
            ctx.request_repaint();
        });
    }

    fn set_cwd(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool, p: PathBuf) {
        self.addr = p.display().to_string();
        self.cwd = p;
        self.filter.clear();
        self.load(ctx, tx, hidden);
    }

    fn go(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool, p: PathBuf) {
        if p == self.cwd {
            return;
        }
        self.history.truncate(self.hist_pos + 1);
        self.history.push(p.clone());
        self.hist_pos = self.history.len() - 1;
        self.set_cwd(ctx, tx, hidden, p);
    }

    fn back(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool) {
        if self.hist_pos > 0 {
            self.hist_pos -= 1;
            let p = self.history[self.hist_pos].clone();
            self.set_cwd(ctx, tx, hidden, p);
        }
    }

    fn forward(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool) {
        if self.hist_pos + 1 < self.history.len() {
            self.hist_pos += 1;
            let p = self.history[self.hist_pos].clone();
            self.set_cwd(ctx, tx, hidden, p);
        }
    }

    fn up(&mut self, ctx: &egui::Context, tx: &Sender<Loaded>, hidden: bool) {
        if let Some(p) = self.cwd.parent() {
            let p = p.to_path_buf();
            self.go(ctx, tx, hidden, p);
        }
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.ascending = !self.ascending;
        } else {
            self.sort_key = key;
            self.ascending = key.default_ascending();
        }
        self.dirty = true;
    }

    fn rebuild_view(&mut self) {
        let f = self.filter.to_lowercase();
        let mut idx: Vec<usize> = (0..self.entries.len())
            .filter(|&i| f.is_empty() || self.entries[i].lname.contains(&f))
            .collect();
        let (key, asc, ff) = (self.sort_key, self.ascending, self.folders_first);
        let e = &self.entries;
        idx.sort_by(|&a, &b| {
            let (x, y) = (&e[a], &e[b]);
            if ff && x.is_dir != y.is_dir {
                return y.is_dir.cmp(&x.is_dir);
            }
            let o = match key {
                SortKey::Saved => x.saved.cmp(&y.saved),
                SortKey::Modified => x.modified.cmp(&y.modified),
                SortKey::Name => natural_cmp(&x.lname, &y.lname),
                SortKey::Size => x.size.cmp(&y.size),
                SortKey::Kind => x.kind.cmp(&y.kind),
            }
            .then_with(|| natural_cmp(&x.lname, &y.lname));
            if asc {
                o
            } else {
                o.reverse()
            }
        });
        self.view = idx;
        self.dirty = false;
    }

    fn selected_entry(&self) -> Option<&Entry> {
        let s = self.selected.as_ref()?;
        self.entries.iter().find(|e| &e.path == s)
    }
}

// ───────────────────────── App ─────────────────────────

enum Action {
    Open(PathBuf),
    OpenNewTab(PathBuf),
    Select(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    CopyPath(PathBuf),
}

struct App {
    tabs: Vec<Tab>,
    active: usize,
    next_tab_id: u64,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,

    show_hidden: bool,
    show_preview: bool,
    dark: bool,
    pal: Pal,

    preview: Option<Preview>,
    rename: Option<(PathBuf, String)>,
    new_folder: Option<String>,
    confirm_delete: Option<PathBuf>,
    status: String,

    places: Vec<(String, Color32, PathBuf)>,
    drives: Arc<Mutex<Vec<PathBuf>>>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        egui_extras::install_image_loaders(&cc.egui_ctx);
        apply_theme(&cc.egui_ctx, true);

        let start = dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("."));

        let mut places = Vec::new();
        let mut add = |n: &str, c: u32, p: Option<PathBuf>| {
            if let Some(p) = p {
                places.push((n.to_string(), rgb(c), p));
            }
        };
        add("Download", 0x1E88E5, dirs::download_dir());
        add("Desktop", 0x00ACC1, dirs::desktop_dir());
        add("Documenti", 0xFFA000, dirs::document_dir());
        add("Immagini", 0x2E9E5B, dirs::picture_dir());
        add("Musica", 0xE91E63, dirs::audio_dir());
        add("Video", 0x8E44AD, dirs::video_dir());
        add("Home", 0x7E57C2, dirs::home_dir());

        // Elenco dischi in un thread: l'avvio non aspetta mai
        let drives = Arc::new(Mutex::new(Vec::new()));
        {
            let d = drives.clone();
            let ctx = cc.egui_ctx.clone();
            thread::spawn(move || {
                let mut found = Vec::new();
                #[cfg(windows)]
                for l in b'C'..=b'Z' {
                    let p = PathBuf::from(format!("{}:\\", l as char));
                    if p.exists() {
                        found.push(p);
                    }
                }
                #[cfg(not(windows))]
                found.push(PathBuf::from("/"));
                *d.lock().unwrap() = found;
                ctx.request_repaint();
            });
        }

        let (tx, rx) = channel();
        let mut app = App {
            tabs: vec![Tab::new(1, start)],
            active: 0,
            next_tab_id: 2,
            tx,
            rx,
            show_hidden: false,
            show_preview: true,
            dark: true,
            pal: palette(true),
            preview: None,
            rename: None,
            new_folder: None,
            confirm_delete: None,
            status: String::new(),
            places,
            drives,
        };
        let (tx, hidden) = (app.tx.clone(), app.show_hidden);
        app.tabs[0].load(&cc.egui_ctx, &tx, hidden);
        app
    }

    fn tab(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    fn reload(&mut self, ctx: &egui::Context) {
        let (tx, hidden) = (self.tx.clone(), self.show_hidden);
        self.tab().load(ctx, &tx, hidden);
    }

    fn go(&mut self, ctx: &egui::Context, p: PathBuf) {
        let (tx, hidden) = (self.tx.clone(), self.show_hidden);
        self.tab().go(ctx, &tx, hidden, p);
    }

    fn new_tab(&mut self, ctx: &egui::Context, p: PathBuf) {
        let mut t = Tab::new(self.next_tab_id, p);
        self.next_tab_id += 1;
        let (tx, hidden) = (self.tx.clone(), self.show_hidden);
        t.load(ctx, &tx, hidden);
        self.tabs.push(t);
        self.active = self.tabs.len() - 1;
    }

    fn close_tab(&mut self, i: usize) {
        if self.tabs.len() <= 1 {
            return;
        }
        self.tabs.remove(i);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if i < self.active {
            self.active -= 1;
        }
    }

    fn poll(&mut self) {
        while let Ok(l) = self.rx.try_recv() {
            let Some(t) = self.tabs.iter_mut().find(|t| t.id == l.tab_id) else { continue };
            if l.load_id != t.load_id || l.path != t.cwd {
                continue; // risultato vecchio
            }
            t.loading = false;
            match l.result {
                Ok(v) => {
                    t.entries = v;
                    t.error = None;
                }
                Err(e) => {
                    t.entries.clear();
                    t.error = Some(e);
                }
            }
            t.dirty = true;
        }
    }

    /// Aggiorna l'anteprima quando cambia la selezione
    fn sync_preview(&mut self, ctx: &egui::Context) {
        let sel = self.tabs[self.active].selected.clone();
        let current = self.preview.as_ref().map(|p| p.path.clone());
        if sel == current {
            return;
        }
        if let Some(Preview { kind: PreviewKind::Image(uri), .. }) = &self.preview {
            ctx.forget_image(uri); // libera la memoria dell'immagine precedente
        }
        self.preview = match (&sel, self.show_preview) {
            (Some(_), true) => self.tabs[self.active].selected_entry().map(build_preview),
            _ => None,
        };
        if self.preview.is_none() && sel.is_some() && self.show_preview {
            // elemento non più nell'elenco: nessuna anteprima
        }
    }

    fn do_action(&mut self, ctx: &egui::Context, a: Action) {
        match a {
            Action::Open(p) => {
                if p.is_dir() {
                    self.go(ctx, p);
                } else if let Err(e) = open::that(&p) {
                    self.status = format!("Impossibile aprire: {e}");
                }
            }
            Action::OpenNewTab(p) => self.new_tab(ctx, p),
            Action::Select(p) => self.tab().selected = Some(p),
            Action::Rename(p) => {
                let n = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                self.rename = Some((p, n));
            }
            Action::Delete(p) => self.confirm_delete = Some(p),
            Action::CopyPath(p) => {
                ctx.copy_text(p.display().to_string());
                self.status = "Percorso copiato".into();
            }
        }
    }

    // ───── Interfaccia ─────

    fn tab_bar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        let mut switch = None;
        let mut close = None;
        let mut add = false;
        ui.horizontal(|ui| {
            for (i, t) in self.tabs.iter().enumerate() {
                let active = i == self.active;
                let title: String = t.title().chars().take(22).collect();
                let (bg, fg) = if active { (pal.primary_container, pal.text) } else { (pal.container, pal.weak) };
                egui::Frame::new()
                    .fill(bg)
                    .corner_radius(CornerRadius::same(16))
                    .inner_margin(egui::Margin::symmetric(12, 4))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let r = ui.add(egui::Label::new(egui::RichText::new(format!("📁 {title}")).color(fg)).sense(Sense::click()));
                            if r.clicked() {
                                switch = Some(i);
                            }
                            if r.middle_clicked() {
                                close = Some(i);
                            }
                            if self.tabs.len() > 1 && ui.add(egui::Label::new(egui::RichText::new("✕").color(fg)).sense(Sense::click())).on_hover_text("Chiudi scheda").clicked() {
                                close = Some(i);
                            }
                        });
                    });
            }
            if ui.add(egui::Button::new("➕").corner_radius(CornerRadius::same(16))).on_hover_text("Nuova scheda (Ctrl+T)").clicked() {
                add = true;
            }
        });
        if let Some(i) = switch {
            self.active = i;
        }
        if let Some(i) = close {
            self.close_tab(i);
        }
        if add {
            let p = self.tabs[self.active].cwd.clone();
            self.new_tab(ctx, p);
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let (tx, hidden) = (self.tx.clone(), self.show_hidden);
        let mut reload = false;
        ui.horizontal(|ui| {
            let t = &mut self.tabs[self.active];
            if ui.add_enabled(t.hist_pos > 0, egui::Button::new("⬅")).on_hover_text("Indietro (Alt+←)").clicked() {
                t.back(ctx, &tx, hidden);
            }
            if ui.add_enabled(t.hist_pos + 1 < t.history.len(), egui::Button::new("➡")).on_hover_text("Avanti (Alt+→)").clicked() {
                t.forward(ctx, &tx, hidden);
            }
            if ui.add_enabled(t.cwd.parent().is_some(), egui::Button::new("⬆")).on_hover_text("Su (Backspace)").clicked() {
                t.up(ctx, &tx, hidden);
            }
            if ui.button("⟳").on_hover_text("Aggiorna (F5)").clicked() {
                reload = true;
            }

            let w = (ui.available_width() - 270.0).max(120.0);
            let r = ui.add_sized([w, 32.0], egui::TextEdit::singleline(&mut t.addr).hint_text("Percorso").margin(egui::Margin::symmetric(14, 6)));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let p = PathBuf::from(t.addr.trim());
                if p.is_dir() {
                    t.go(ctx, &tx, hidden, p);
                } else {
                    self.status = "Percorso non valido".into();
                    t.addr = t.cwd.display().to_string();
                }
            }
            let r = ui.add_sized(
                [ui.available_width(), 32.0],
                egui::TextEdit::singleline(&mut t.filter).hint_text("🔍 Cerca in questa cartella").margin(egui::Margin::symmetric(14, 6)),
            );
            if r.changed() {
                t.dirty = true;
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.label("Ordina per:");
            let t = &mut self.tabs[self.active];
            let mut chosen = None;
            egui::ComboBox::from_id_salt("sort").selected_text(t.sort_key.label()).show_ui(ui, |ui| {
                for k in SortKey::ALL {
                    if ui.selectable_label(t.sort_key == k, k.label()).clicked() {
                        chosen = Some(k);
                    }
                }
            });
            if let Some(k) = chosen {
                t.sort_key = k;
                t.ascending = k.default_ascending();
                t.dirty = true;
            }
            if ui.button(if t.ascending { "⬆ Crescente" } else { "⬇ Decrescente" }).clicked() {
                t.ascending = !t.ascending;
                t.dirty = true;
            }
            if ui.checkbox(&mut t.folders_first, "Cartelle prima").changed() {
                t.dirty = true;
            }
            if ui.checkbox(&mut self.show_hidden, "Nascosti").changed() {
                reload = true;
            }
            ui.checkbox(&mut self.show_preview, "Anteprima");
            ui.separator();
            if ui.button("➕ Nuova cartella").clicked() {
                self.new_folder = Some("Nuova cartella".into());
            }
            if ui.button(if self.dark { "☀" } else { "🌙" }).on_hover_text("Cambia tema").clicked() {
                self.dark = !self.dark;
                self.pal = palette(self.dark);
                apply_theme(ctx, self.dark);
            }
        });
        if reload {
            self.reload(ctx);
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.add_space(8.0);
        ui.label(egui::RichText::new("POSIZIONI").small().color(pal.weak));
        let cwd = self.tabs[self.active].cwd.clone();
        let mut target = None;
        let glyphs = ["⬇", "🖥", "📄", "🖼", "♫", "▶", "🏠"];
        for (i, (name, color, p)) in self.places.iter().enumerate() {
            let g = glyphs.get(i).copied().unwrap_or("•");
            if side_item(ui, pal, *color, g, name, *p == cwd) {
                target = Some(p.clone());
            }
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("DISCHI").small().color(pal.weak));
        let drives = self.drives.lock().unwrap().clone();
        for p in drives {
            if side_item(ui, pal, rgb(0x546E7A), "💾", &p.display().to_string(), p == cwd) {
                target = Some(p);
            }
        }
        if let Some(p) = target {
            self.go(ctx, p);
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        const ROW_H: f32 = 32.0;
        let pal = self.pal;
        let total_w = ui.available_width();
        let (w_date, w_mod, w_kind, w_size) = (150.0, 150.0, 110.0, 90.0);
        let w_name = (total_w - w_date - w_mod - w_kind - w_size - 24.0).max(150.0);
        let xs = [0.0, w_name, w_name + w_date, w_name + w_date + w_mod, w_name + w_date + w_mod + w_kind];

        let heads = [
            ("Nome", SortKey::Name),
            ("Download/salvataggio", SortKey::Saved),
            ("Modificato", SortKey::Modified),
            ("Tipo", SortKey::Kind),
            ("Dimensione", SortKey::Size),
        ];
        let widths = [w_name, w_date, w_mod, w_kind, w_size];
        let (hrect, _) = ui.allocate_exact_size(Vec2::new(total_w, 28.0), Sense::hover());
        let mut clicked_sort = None;
        {
            let t = &self.tabs[self.active];
            for (i, (label, key)) in heads.iter().enumerate() {
                let r = Rect::from_min_size(hrect.min + Vec2::new(xs[i] + 10.0, 0.0), Vec2::new(widths[i], 28.0));
                let resp = ui.interact(r, ui.id().with(("head", i)), Sense::click());
                let active = t.sort_key == *key;
                let arrow = if active {
                    if t.ascending { " ▲" } else { " ▼" }
                } else {
                    ""
                };
                let color = if active { pal.primary } else { pal.weak };
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, format!("{label}{arrow}"), FontId::proportional(13.0), color);
                if resp.clicked() {
                    clicked_sort = Some(*key);
                }
            }
        }
        if let Some(k) = clicked_sort {
            self.tab().set_sort(k);
        }
        ui.separator();

        let t = &self.tabs[self.active];
        if t.loading && t.entries.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        }
        if let Some(e) = &t.error {
            ui.add_space(20.0);
            ui.colored_label(Color32::from_rgb(0xF2, 0xB8, 0xB5), format!("Impossibile leggere la cartella: {e}"));
            return;
        }
        if t.view.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.label(egui::RichText::new("Nessun elemento").color(pal.weak)));
            return;
        }

        let mut action: Option<Action> = None;
        let n = t.view.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, n, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                let e = &t.entries[t.view[row]];
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(total_w - 12.0, ROW_H), Sense::click());
                let selected = t.selected.as_ref() == Some(&e.path);
                let inner = rect.shrink2(Vec2::new(0.0, 1.0));
                if selected {
                    ui.painter().rect_filled(inner, 16.0, pal.primary_container);
                } else if resp.hovered() {
                    ui.painter().rect_filled(inner, 16.0, pal.container);
                }
                let font = FontId::proportional(14.0);
                let p = ui.painter();

                let icon_r = Rect::from_min_size(rect.min + Vec2::new(12.0, (ROW_H - 18.0) / 2.0), Vec2::new(22.0, 18.0));
                if e.is_dir {
                    draw_folder(p, icon_r);
                } else {
                    draw_file_icon(p, Rect::from_center_size(icon_r.center(), Vec2::new(22.0, 18.0)), &e.ext);
                }
                let name_clip = Rect::from_min_size(rect.min + Vec2::new(40.0, 0.0), Vec2::new(w_name - 46.0, ROW_H));
                p.with_clip_rect(name_clip).text(
                    rect.min + Vec2::new(42.0, ROW_H / 2.0),
                    Align2::LEFT_CENTER,
                    &e.name,
                    font.clone(),
                    pal.text,
                );
                let cell = |i: usize, s: String| {
                    p.text(rect.min + Vec2::new(xs[i] + 10.0, ROW_H / 2.0), Align2::LEFT_CENTER, s, font.clone(), pal.weak);
                };
                cell(1, fmt_time(e.saved));
                cell(2, fmt_time(e.modified));
                cell(3, e.kind.clone());
                if !e.is_dir {
                    cell(4, fmt_size(e.size));
                }

                if resp.clicked() {
                    action = Some(Action::Select(e.path.clone()));
                }
                if resp.double_clicked() {
                    action = Some(Action::Open(e.path.clone()));
                }
                if resp.middle_clicked() && e.is_dir {
                    action = Some(Action::OpenNewTab(e.path.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Apri").clicked() {
                        action = Some(Action::Open(e.path.clone()));
                        ui.close_menu();
                    }
                    if e.is_dir && ui.button("Apri in nuova scheda").clicked() {
                        action = Some(Action::OpenNewTab(e.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Rinomina (F2)").clicked() {
                        action = Some(Action::Rename(e.path.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Copia percorso").clicked() {
                        action = Some(Action::CopyPath(e.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("Sposta nel cestino (Canc)").clicked() {
                        action = Some(Action::Delete(e.path.clone()));
                        ui.close_menu();
                    }
                });
            }
        });
        if let Some(a) = action {
            self.do_action(ctx, a);
        }
    }

    fn preview_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let pal = self.pal;
        ui.add_space(10.0);
        let t = &self.tabs[self.active];
        let (Some(entry), Some(prev)) = (t.selected_entry().cloned(), &self.preview) else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(egui::RichText::new("👁").size(36.0).color(pal.outline));
                ui.label(egui::RichText::new("Seleziona un file\nper vedere l'anteprima").color(pal.weak));
            });
            return;
        };

        let w = ui.available_width();
        match &prev.kind {
            PreviewKind::Image(uri) => {
                ui.vertical_centered(|ui| {
                    ui.add(egui::Image::from_uri(uri.clone()).max_size(Vec2::new(w - 8.0, 300.0)).corner_radius(12.0));
                });
            }
            PreviewKind::Text(txt) => {
                egui::Frame::new().fill(pal.container).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    egui::ScrollArea::both().max_height(300.0).id_salt("txtprev").show(ui, |ui| {
                        ui.add(egui::Label::new(egui::RichText::new(txt).monospace().size(12.0).color(pal.text)).wrap_mode(egui::TextWrapMode::Extend));
                    });
                });
            }
            PreviewKind::Folder(n) => {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(96.0, 78.0), Sense::hover());
                    draw_folder(ui.painter(), r);
                    ui.label(egui::RichText::new(format!("{n} elementi")).color(pal.weak));
                });
            }
            PreviewKind::None(msg) => {
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(72.0, 84.0), Sense::hover());
                    draw_file_icon(ui.painter(), r, &entry.ext);
                    ui.label(egui::RichText::new(*msg).color(pal.weak));
                });
            }
        }

        ui.add_space(14.0);
        ui.label(egui::RichText::new(&entry.name).strong().size(16.0).color(pal.text));
        ui.add_space(6.0);
        egui::Grid::new("info").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            let mut row = |k: &str, v: String| {
                ui.label(egui::RichText::new(k).color(pal.weak));
                ui.label(egui::RichText::new(v).color(pal.text));
                ui.end_row();
            };
            row("Tipo", entry.kind.clone());
            if !entry.is_dir {
                row("Dimensione", fmt_size(entry.size));
            }
            row("Salvato", fmt_time(entry.saved));
            row("Modificato", fmt_time(entry.modified));
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button("Apri").clicked() {
                self.do_action(ctx, Action::Open(entry.path.clone()));
            }
            if ui.button("Rinomina").clicked() {
                self.do_action(ctx, Action::Rename(entry.path.clone()));
            }
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Rinomina
        if let Some((path, mut name)) = self.rename.take() {
            let (mut keep, mut apply) = (true, false);
            egui::Window::new("Rinomina").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(320.0));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    apply = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Rinomina").clicked() {
                        apply = true;
                    }
                    if ui.button("Annulla").clicked() {
                        keep = false;
                    }
                });
            });
            if escape(ctx) {
                keep = false;
            }
            if apply {
                match std::fs::rename(&path, path.with_file_name(name.trim())) {
                    Ok(_) => {
                        self.status = "Rinominato".into();
                        self.reload(ctx);
                    }
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.rename = Some((path, name));
            }
        }

        // Nuova cartella
        if let Some(mut name) = self.new_folder.take() {
            let (mut keep, mut apply) = (true, false);
            egui::Window::new("Nuova cartella").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(320.0));
                r.request_focus();
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    apply = true;
                }
                ui.horizontal(|ui| {
                    if ui.button("Crea").clicked() {
                        apply = true;
                    }
                    if ui.button("Annulla").clicked() {
                        keep = false;
                    }
                });
            });
            if escape(ctx) {
                keep = false;
            }
            if apply {
                let dir = self.tabs[self.active].cwd.join(name.trim());
                match std::fs::create_dir(dir) {
                    Ok(_) => self.reload(ctx),
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.new_folder = Some(name);
            }
        }

        // Conferma cestino
        if let Some(path) = self.confirm_delete.take() {
            let (mut keep, mut apply) = (true, false);
            let nm = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            egui::Window::new("Sposta nel cestino").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
                ui.label(format!("Spostare «{nm}» nel cestino?"));
                ui.horizontal(|ui| {
                    if ui.button("Sì, sposta").clicked() {
                        apply = true;
                    }
                    if ui.button("Annulla").clicked() {
                        keep = false;
                    }
                });
            });
            if escape(ctx) {
                keep = false;
            }
            if apply {
                match trash::delete(&path) {
                    Ok(_) => {
                        self.status = "Spostato nel cestino".into();
                        self.reload(ctx);
                    }
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.confirm_delete = Some(path);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        let (ctrl_t, ctrl_w, ctrl_tab) = ctx.input(|i| {
            (
                i.modifiers.command && i.key_pressed(Key::T),
                i.modifiers.command && i.key_pressed(Key::W),
                i.modifiers.ctrl && i.key_pressed(Key::Tab),
            )
        });
        if ctrl_t {
            let p = self.tabs[self.active].cwd.clone();
            self.new_tab(ctx, p);
        }
        if ctrl_w {
            let i = self.active;
            self.close_tab(i);
        }
        if ctrl_tab {
            self.active = (self.active + 1) % self.tabs.len();
        }

        if ctx.wants_keyboard_input() || self.rename.is_some() || self.new_folder.is_some() || self.confirm_delete.is_some() {
            return;
        }
        let (f5, back, fwd, bksp, del, f2, enter) = ctx.input(|i| {
            (
                i.key_pressed(Key::F5),
                i.modifiers.alt && i.key_pressed(Key::ArrowLeft),
                i.modifiers.alt && i.key_pressed(Key::ArrowRight),
                i.key_pressed(Key::Backspace),
                i.key_pressed(Key::Delete),
                i.key_pressed(Key::F2),
                i.key_pressed(Key::Enter),
            )
        });
        let (tx, hidden) = (self.tx.clone(), self.show_hidden);
        if f5 {
            self.reload(ctx);
        }
        if back {
            self.tab().back(ctx, &tx, hidden);
        }
        if fwd {
            self.tab().forward(ctx, &tx, hidden);
        }
        if bksp {
            self.tab().up(ctx, &tx, hidden);
        }
        if let Some(p) = self.tabs[self.active].selected.clone() {
            if del {
                self.confirm_delete = Some(p.clone());
            }
            if f2 {
                self.do_action(ctx, Action::Rename(p.clone()));
            }
            if enter {
                self.do_action(ctx, Action::Open(p));
            }
        }
    }
}

fn side_item(ui: &mut egui::Ui, pal: Pal, color: Color32, glyph: &str, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 36.0), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 18.0, pal.sec_container);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 18.0, pal.container);
    }
    let c = rect.min + Vec2::new(22.0, 18.0);
    ui.painter().circle_filled(c, 12.0, color);
    ui.painter().text(c, Align2::CENTER_CENTER, glyph, FontId::proportional(13.0), Color32::WHITE);
    ui.painter().with_clip_rect(rect.shrink2(Vec2::new(8.0, 0.0))).text(
        rect.min + Vec2::new(44.0, 18.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.0),
        if selected { pal.text } else { pal.weak },
    );
    resp.clicked()
}

fn escape(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_pressed(Key::Escape))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        for t in &mut self.tabs {
            if t.dirty {
                t.rebuild_view();
            }
        }
        self.shortcuts(ctx);
        self.sync_preview(ctx);
        let pal = self.pal;

        egui::TopBottomPanel::top("top").frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::symmetric(12, 8))).show(ctx, |ui| {
            self.tab_bar(ui, ctx);
            ui.add_space(4.0);
            self.toolbar(ui, ctx);
        });
        egui::TopBottomPanel::bottom("status").frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::symmetric(14, 6))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{} elementi", self.tabs[self.active].view.len())).color(pal.weak));
                if self.tabs[self.active].loading {
                    ui.spinner();
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.label(egui::RichText::new(&self.status).color(pal.weak));
                }
            });
        });
        egui::SidePanel::left("side").resizable(false).exact_width(200.0).frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::symmetric(10, 4))).show(ctx, |ui| {
            self.sidebar(ui, ctx);
        });
        if self.show_preview {
            egui::SidePanel::right("preview")
                .resizable(true)
                .default_width(320.0)
                .width_range(240.0..=560.0)
                .frame(egui::Frame::new().fill(pal.bg).inner_margin(egui::Margin::symmetric(14, 4)))
                .show(ctx, |ui| {
                    self.preview_panel(ui, ctx);
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.surface).corner_radius(CornerRadius::same(24)).inner_margin(egui::Margin::symmetric(8, 8)))
            .show(ctx, |ui| {
                self.file_list(ui, ctx);
            });
        self.dialogs(ctx);
    }
}

fn load_icon() -> Option<egui::IconData> {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.into_rgba8();
    let (width, height) = img.dimensions();
    Some(egui::IconData { rgba: img.into_raw(), width, height })
}

fn main() -> eframe::Result {
    let mut vp = egui::ViewportBuilder::default()
        .with_title("Gestore File")
        .with_inner_size([1250.0, 740.0])
        .with_min_inner_size([800.0, 420.0]);
    if let Some(icon) = load_icon() {
        vp = vp.with_icon(Arc::new(icon));
    }
    let options = eframe::NativeOptions { viewport: vp, ..Default::default() };
    eframe::run_native("Gestore File", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
