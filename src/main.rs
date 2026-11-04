// Su Windows non mostra la finestra nera del terminale
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align2, Color32, FontId, Key, Sense, Vec2};
use std::{
    cmp::Ordering,
    path::{Path, PathBuf},
    sync::{
        mpsc::{channel, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::SystemTime,
};

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
    const ALL: [SortKey; 5] = [
        SortKey::Saved,
        SortKey::Modified,
        SortKey::Name,
        SortKey::Size,
        SortKey::Kind,
    ];
    fn label(self) -> &'static str {
        match self {
            SortKey::Saved => "Data download/salvataggio",
            SortKey::Modified => "Data modifica",
            SortKey::Name => "Nome",
            SortKey::Size => "Dimensione",
            SortKey::Kind => "Tipo",
        }
    }
    /// Direzione naturale: date e dimensioni dal più grande/recente, nomi dalla A
    fn default_ascending(self) -> bool {
        matches!(self, SortKey::Name | SortKey::Kind)
    }
}

struct Loaded {
    id: u64,
    path: PathBuf,
    result: Result<Vec<Entry>, String>,
}

// ───────────────────────── Utilità ─────────────────────────

fn is_hidden(md: &std::fs::Metadata, name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        let _ = name;
        md.file_attributes() & 0x2 != 0 // FILE_ATTRIBUTE_HIDDEN
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
        let kind = if is_dir {
            "Cartella".to_string()
        } else {
            match Path::new(&name).extension() {
                Some(e) => format!("File {}", e.to_string_lossy().to_uppercase()),
                None => "File".to_string(),
            }
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
                let mut na = String::new();
                let mut nb = String::new();
                while let Some(&c) = ai.peek() {
                    if c.is_ascii_digit() {
                        na.push(c);
                        ai.next();
                    } else {
                        break;
                    }
                }
                while let Some(&c) = bi.peek() {
                    if c.is_ascii_digit() {
                        nb.push(c);
                        bi.next();
                    } else {
                        break;
                    }
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
        Some(t) => chrono::DateTime::<chrono::Local>::from(t)
            .format("%d/%m/%Y %H:%M")
            .to_string(),
        None => "—".into(),
    }
}

fn icon_for(e: &Entry) -> &'static str {
    if e.is_dir {
        return "📁";
    }
    let ext = Path::new(&e.name)
        .extension()
        .map(|x| x.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" | "svg" | "heic" => "🖼",
        "mp3" | "wav" | "flac" | "m4a" | "ogg" => "🎵",
        "mp4" | "mkv" | "avi" | "mov" | "webm" => "🎬",
        "zip" | "rar" | "7z" | "tar" | "gz" => "🗜",
        "exe" | "msi" | "bat" => "⚙",
        _ => "📄",
    }
}

// ───────────────────────── App ─────────────────────────

enum Action {
    Open(PathBuf),
    Select(PathBuf),
    Rename(PathBuf),
    Delete(PathBuf),
    CopyPath(PathBuf),
}

struct App {
    cwd: PathBuf,
    history: Vec<PathBuf>,
    hist_pos: usize,

    entries: Vec<Entry>,
    view: Vec<usize>,
    dirty: bool,
    loading: bool,
    error: Option<String>,
    load_id: u64,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,

    addr: String,
    filter: String,
    sort_key: SortKey,
    ascending: bool,
    folders_first: bool,
    show_hidden: bool,
    dark: bool,

    selected: Option<PathBuf>,
    rename: Option<(PathBuf, String)>,
    new_folder: Option<String>,
    confirm_delete: Option<PathBuf>,
    status: String,

    places: Vec<(String, PathBuf)>,
    drives: Arc<Mutex<Vec<PathBuf>>>,
}

const ACCENT: Color32 = Color32::from_rgb(96, 165, 250);

fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        v.panel_fill = Color32::from_rgb(24, 26, 31);
        v.window_fill = Color32::from_rgb(30, 33, 39);
        v.extreme_bg_color = Color32::from_rgb(18, 19, 23);
        v.faint_bg_color = Color32::from_rgb(29, 32, 38);
    }
    v.selection.bg_fill = ACCENT.linear_multiply(0.35);
    v.selection.stroke.color = ACCENT;
    v.hyperlink_color = ACCENT;
    let r = egui::CornerRadius::same(8);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.corner_radius = r;
    v.window_corner_radius = egui::CornerRadius::same(12);
    ctx.set_visuals(v);
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        apply_theme(&cc.egui_ctx, true);
        let mut style = (*cc.egui_ctx.style()).clone();
        style.spacing.item_spacing = Vec2::new(8.0, 6.0);
        style.spacing.button_padding = Vec2::new(10.0, 5.0);
        cc.egui_ctx.set_style(style);

        let start = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));

        let mut places = Vec::new();
        let mut add = |n: &str, p: Option<PathBuf>| {
            if let Some(p) = p {
                places.push((n.to_string(), p));
            }
        };
        add("⬇  Download", dirs::download_dir());
        add("🖥  Desktop", dirs::desktop_dir());
        add("📄  Documenti", dirs::document_dir());
        add("🖼  Immagini", dirs::picture_dir());
        add("🎵  Musica", dirs::audio_dir());
        add("🎬  Video", dirs::video_dir());
        add("🏠  Home", dirs::home_dir());

        // Elenco dischi in un thread, così l'avvio non aspetta mai
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
            cwd: start.clone(),
            history: vec![start.clone()],
            hist_pos: 0,
            entries: vec![],
            view: vec![],
            dirty: true,
            loading: false,
            error: None,
            load_id: 0,
            tx,
            rx,
            addr: start.display().to_string(),
            filter: String::new(),
            sort_key: SortKey::Saved,
            ascending: false, // più recenti in alto
            folders_first: false,
            show_hidden: false,
            dark: true,
            selected: None,
            rename: None,
            new_folder: None,
            confirm_delete: None,
            status: String::new(),
            places,
            drives,
        };
        app.load(&cc.egui_ctx);
        app
    }

    fn load(&mut self, ctx: &egui::Context) {
        self.load_id += 1;
        self.loading = true;
        self.error = None;
        self.selected = None;
        let (id, path, hidden, tx, ctx) = (
            self.load_id,
            self.cwd.clone(),
            self.show_hidden,
            self.tx.clone(),
            ctx.clone(),
        );
        thread::spawn(move || {
            let result = read_entries(&path, hidden);
            let _ = tx.send(Loaded { id, path, result });
            ctx.request_repaint();
        });
    }

    fn go(&mut self, ctx: &egui::Context, p: PathBuf) {
        if p == self.cwd {
            return;
        }
        self.history.truncate(self.hist_pos + 1);
        self.history.push(p.clone());
        self.hist_pos = self.history.len() - 1;
        self.set_cwd(ctx, p);
    }

    fn set_cwd(&mut self, ctx: &egui::Context, p: PathBuf) {
        self.addr = p.display().to_string();
        self.cwd = p;
        self.filter.clear();
        self.load(ctx);
    }

    fn back(&mut self, ctx: &egui::Context) {
        if self.hist_pos > 0 {
            self.hist_pos -= 1;
            let p = self.history[self.hist_pos].clone();
            self.set_cwd(ctx, p);
        }
    }

    fn forward(&mut self, ctx: &egui::Context) {
        if self.hist_pos + 1 < self.history.len() {
            self.hist_pos += 1;
            let p = self.history[self.hist_pos].clone();
            self.set_cwd(ctx, p);
        }
    }

    fn up(&mut self, ctx: &egui::Context) {
        if let Some(p) = self.cwd.parent() {
            self.go(ctx, p.to_path_buf());
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

    fn poll(&mut self) {
        while let Ok(l) = self.rx.try_recv() {
            if l.id != self.load_id || l.path != self.cwd {
                continue; // risultato vecchio, ignorato
            }
            self.loading = false;
            match l.result {
                Ok(v) => {
                    self.entries = v;
                    self.error = None;
                }
                Err(e) => {
                    self.entries.clear();
                    self.error = Some(e);
                }
            }
            self.dirty = true;
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
            Action::Select(p) => self.selected = Some(p),
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

    fn toolbar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            if ui.add_enabled(self.hist_pos > 0, egui::Button::new("⬅")).on_hover_text("Indietro (Alt+←)").clicked() {
                self.back(ctx);
            }
            if ui.add_enabled(self.hist_pos + 1 < self.history.len(), egui::Button::new("➡")).on_hover_text("Avanti (Alt+→)").clicked() {
                self.forward(ctx);
            }
            if ui.add_enabled(self.cwd.parent().is_some(), egui::Button::new("⬆")).on_hover_text("Su (Backspace)").clicked() {
                self.up(ctx);
            }
            if ui.button("⟳").on_hover_text("Aggiorna (F5)").clicked() {
                self.load(ctx);
            }

            let w = (ui.available_width() - 260.0).max(120.0);
            let r = ui.add_sized([w, 28.0], egui::TextEdit::singleline(&mut self.addr).hint_text("Percorso"));
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let p = PathBuf::from(self.addr.trim());
                if p.is_dir() {
                    self.go(ctx, p);
                } else {
                    self.status = "Percorso non valido".into();
                    self.addr = self.cwd.display().to_string();
                }
            }

            let r = ui.add_sized([ui.available_width(), 28.0], egui::TextEdit::singleline(&mut self.filter).hint_text("🔍 Cerca in questa cartella"));
            if r.changed() {
                self.dirty = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Ordina per:");
            let mut chosen = None;
            egui::ComboBox::from_id_salt("sort")
                .selected_text(self.sort_key.label())
                .show_ui(ui, |ui| {
                    for k in SortKey::ALL {
                        if ui.selectable_label(self.sort_key == k, k.label()).clicked() {
                            chosen = Some(k);
                        }
                    }
                });
            if let Some(k) = chosen {
                self.sort_key = k;
                self.ascending = k.default_ascending();
                self.dirty = true;
            }
            let arrow = if self.ascending { "⬆ Crescente" } else { "⬇ Decrescente" };
            if ui.button(arrow).clicked() {
                self.ascending = !self.ascending;
                self.dirty = true;
            }
            if ui.checkbox(&mut self.folders_first, "Cartelle prima").changed() {
                self.dirty = true;
            }
            if ui.checkbox(&mut self.show_hidden, "File nascosti").changed() {
                self.load(ctx);
            }
            ui.separator();
            if ui.button("➕ Nuova cartella").clicked() {
                self.new_folder = Some("Nuova cartella".into());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(if self.dark { "☀" } else { "🌙" }).on_hover_text("Cambia tema").clicked() {
                    self.dark = !self.dark;
                    apply_theme(ctx, self.dark);
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("POSIZIONI").small().weak());
        let mut target = None;
        for (name, p) in &self.places {
            let sel = *p == self.cwd;
            if ui.add_sized([ui.available_width(), 28.0], egui::SelectableLabel::new(sel, name)).clicked() {
                target = Some(p.clone());
            }
        }
        ui.add_space(10.0);
        ui.label(egui::RichText::new("DISCHI").small().weak());
        let drives = self.drives.lock().unwrap().clone();
        for p in drives {
            let sel = p == self.cwd;
            let label = format!("💾  {}", p.display());
            if ui.add_sized([ui.available_width(), 28.0], egui::SelectableLabel::new(sel, label)).clicked() {
                target = Some(p);
            }
        }
        if let Some(p) = target {
            self.go(ctx, p);
        }
    }

    fn file_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        const ROW_H: f32 = 28.0;
        let total_w = ui.available_width();
        let (w_date, w_mod, w_kind, w_size) = (150.0, 150.0, 110.0, 90.0);
        let w_name = (total_w - w_date - w_mod - w_kind - w_size - 24.0).max(150.0);
        let xs = [0.0, w_name, w_name + w_date, w_name + w_date + w_mod, w_name + w_date + w_mod + w_kind];

        // Intestazione cliccabile
        let heads = [
            ("Nome", SortKey::Name),
            ("Download/salvataggio", SortKey::Saved),
            ("Modificato", SortKey::Modified),
            ("Tipo", SortKey::Kind),
            ("Dimensione", SortKey::Size),
        ];
        let widths = [w_name, w_date, w_mod, w_kind, w_size];
        let (hrect, _) = ui.allocate_exact_size(Vec2::new(total_w, 26.0), Sense::hover());
        let mut clicked_sort = None;
        for (i, (label, key)) in heads.iter().enumerate() {
            let r = egui::Rect::from_min_size(
                hrect.min + Vec2::new(xs[i] + 10.0, 0.0),
                Vec2::new(widths[i], 26.0),
            );
            let resp = ui.interact(r, ui.id().with(("head", i)), Sense::click());
            let active = self.sort_key == *key;
            let arrow = if active { if self.ascending { " ▲" } else { " ▼" } } else { "" };
            let color = if active { ACCENT } else { ui.visuals().weak_text_color() };
            ui.painter().text(r.left_center(), Align2::LEFT_CENTER, format!("{label}{arrow}"), FontId::proportional(13.0), color);
            if resp.clicked() {
                clicked_sort = Some(*key);
            }
        }
        if let Some(k) = clicked_sort {
            self.set_sort(k);
        }
        ui.separator();

        if self.loading && self.entries.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.spinner());
            return;
        }
        if let Some(e) = &self.error {
            ui.add_space(20.0);
            ui.colored_label(Color32::LIGHT_RED, format!("Impossibile leggere la cartella: {e}"));
            return;
        }
        if self.view.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| ui.weak("Nessun elemento"));
            return;
        }

        let mut action: Option<Action> = None;
        let n = self.view.len();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, ROW_H, n, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                let e = &self.entries[self.view[row]];
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(total_w - 12.0, ROW_H), Sense::click());
                let selected = self.selected.as_ref() == Some(&e.path);
                if selected {
                    ui.painter().rect_filled(rect, 6.0, ACCENT.linear_multiply(0.25));
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, ui.visuals().widgets.hovered.weak_bg_fill);
                }
                let txt = ui.visuals().text_color();
                let weak = ui.visuals().weak_text_color();
                let font = FontId::proportional(14.0);
                let p = ui.painter();

                let name_clip = egui::Rect::from_min_size(rect.min, Vec2::new(w_name - 6.0, ROW_H));
                p.with_clip_rect(name_clip).text(
                    rect.min + Vec2::new(10.0, ROW_H / 2.0),
                    Align2::LEFT_CENTER,
                    format!("{}  {}", icon_for(e), e.name),
                    font.clone(),
                    txt,
                );
                let cell = |i: usize, s: String, col: Color32| {
                    p.text(rect.min + Vec2::new(xs[i] + 10.0, ROW_H / 2.0), Align2::LEFT_CENTER, s, font.clone(), col);
                };
                cell(1, fmt_time(e.saved), weak);
                cell(2, fmt_time(e.modified), weak);
                cell(3, e.kind.clone(), weak);
                if !e.is_dir {
                    cell(4, fmt_size(e.size), weak);
                }

                if resp.clicked() {
                    action = Some(Action::Select(e.path.clone()));
                }
                if resp.double_clicked() {
                    action = Some(Action::Open(e.path.clone()));
                }
                resp.context_menu(|ui| {
                    if ui.button("Apri").clicked() {
                        action = Some(Action::Open(e.path.clone()));
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

    fn dialogs(&mut self, ctx: &egui::Context) {
        // Rinomina
        if let Some((path, mut name)) = self.rename.take() {
            let mut keep = true;
            let mut apply = false;
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
            if ui_escape(ctx) {
                keep = false;
            }
            if apply {
                let new = path.with_file_name(name.trim());
                match std::fs::rename(&path, &new) {
                    Ok(_) => {
                        self.status = "Rinominato".into();
                        self.load(ctx);
                    }
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.rename = Some((path, name));
            }
        }

        // Nuova cartella
        if let Some(mut name) = self.new_folder.take() {
            let mut keep = true;
            let mut apply = false;
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
            if ui_escape(ctx) {
                keep = false;
            }
            if apply {
                match std::fs::create_dir(self.cwd.join(name.trim())) {
                    Ok(_) => self.load(ctx),
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.new_folder = Some(name);
            }
        }

        // Conferma cestino
        if let Some(path) = self.confirm_delete.take() {
            let mut keep = true;
            let mut apply = false;
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
            if ui_escape(ctx) {
                keep = false;
            }
            if apply {
                match trash::delete(&path) {
                    Ok(_) => {
                        self.status = "Spostato nel cestino".into();
                        self.load(ctx);
                    }
                    Err(e) => self.status = format!("Errore: {e}"),
                }
            } else if keep {
                self.confirm_delete = Some(path);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
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
        if f5 {
            self.load(ctx);
        }
        if back {
            self.back(ctx);
        }
        if fwd {
            self.forward(ctx);
        }
        if bksp {
            self.up(ctx);
        }
        if let Some(p) = self.selected.clone() {
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

fn ui_escape(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.key_pressed(Key::Escape))
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        if self.dirty {
            self.rebuild_view();
        }
        self.shortcuts(ctx);

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.add_space(6.0);
            self.toolbar(ui, ctx);
            ui.add_space(4.0);
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.weak(format!("{} elementi", self.view.len()));
                if self.loading {
                    ui.spinner();
                }
                if !self.status.is_empty() {
                    ui.separator();
                    ui.weak(&self.status);
                }
            });
        });
        egui::SidePanel::left("side").resizable(false).exact_width(190.0).show(ctx, |ui| {
            self.sidebar(ui, ctx);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.file_list(ui, ctx);
        });
        self.dialogs(ctx);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gestore File")
            .with_inner_size([1100.0, 700.0])
            .with_min_inner_size([700.0, 400.0]),
        ..Default::default()
    };
    eframe::run_native("Gestore File", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
