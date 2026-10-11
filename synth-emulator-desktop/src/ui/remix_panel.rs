//! The Remix tab: two decks, the mixer, the library and the keyboard map.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crossbeam_channel::{unbounded, Receiver, Sender};
use eframe::egui::{
    self, Color32, Key, Margin, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};
use remix::{
    Band, CrossfaderCurve, DeckCmd, DeckState, RemixCmd, RemixState, TapTempo, Track, DECKS,
    HOT_CUES, TEMPO_RANGES,
};

use super::theme::{self, ACCENT, ACCENT_FG, BG, ELEVATED, FG, MUTED, SUBTLE, SURFACE};
use super::widgets::{chip, knob};
use crate::settings::{Settings, TrackMemo};

const DECK_NAMES: [&str; DECKS] = ["A", "B"];
/// Seconds shown on each side of the playhead in the zoomed waveform.
const ZOOM_HALF_SECS: f64 = 3.0;
const LOW_COLOR: Color32 = Color32::from_rgb(236, 124, 84);
const MID_COLOR: Color32 = Color32::from_rgb(200, 207, 216);
const HIGH_COLOR: Color32 = Color32::from_rgb(112, 172, 255);
const DECK_COLORS: [Color32; DECKS] = [
    Color32::from_rgb(236, 124, 84),
    Color32::from_rgb(112, 172, 255),
];
const CUE_COLORS: [Color32; HOT_CUES] = [
    Color32::from_rgb(240, 84, 84),
    Color32::from_rgb(250, 190, 70),
    Color32::from_rgb(90, 210, 130),
    Color32::from_rgb(170, 120, 250),
];
const LOOP_BEATS: [f32; 4] = [1.0, 2.0, 4.0, 8.0];
const AUTO_BEATS: [f32; 4] = [4.0, 8.0, 16.0, 32.0];
/// Filter sweep speed while E/R or U/I is held (full range per second).
const FILTER_SWEEP_PER_SEC: f32 = 0.9;
const CROSSFADER_GLIDE_SECS: f64 = 0.12;

/// Where remix commands go. The app passes the audio host's sender.
pub trait RemixSink {
    fn send(&self, cmd: RemixCmd);
}

impl<F: Fn(RemixCmd)> RemixSink for F {
    fn send(&self, cmd: RemixCmd) {
        self(cmd)
    }
}

struct LoadResult {
    deck: usize,
    path: PathBuf,
    result: Result<Arc<Track>, String>,
}

#[derive(Default)]
struct DeckUi {
    track: Option<Arc<Track>>,
    loading: Option<PathBuf>,
    tap: TapTempo,
}

/// The UI's copy of every mixer control. It is the source of truth for the
/// knobs; the engine's state only confirms it.
#[derive(Clone, Copy, Debug)]
pub struct MixUi {
    pub eq: [[f32; 3]; DECKS],
    pub kills: [[bool; 3]; DECKS],
    pub filter: [f32; DECKS],
    pub volume: [f32; DECKS],
    pub crossfader: f32,
    pub master_filter: f32,
    pub echo_mix: f32,
    pub echo_feedback: f32,
    pub echo_beats: f32,
    pub master_gain: f32,
}

impl Default for MixUi {
    fn default() -> Self {
        Self {
            eq: [[1.0; 3]; DECKS],
            kills: [[false; 3]; DECKS],
            filter: [0.0; DECKS],
            volume: [1.0; DECKS],
            crossfader: 0.5,
            master_filter: 0.0,
            echo_mix: 0.0,
            echo_feedback: 0.4,
            echo_beats: 0.75,
            master_gain: 1.0,
        }
    }
}

struct Entry {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

#[derive(Default)]
struct Library {
    dir: PathBuf,
    entries: Vec<Entry>,
    filter: String,
    selected: Option<PathBuf>,
    error: Option<String>,
    path_edit: String,
}

impl Library {
    fn open(&mut self, dir: PathBuf) {
        self.path_edit = dir.to_string_lossy().into_owned();
        self.dir = dir;
        self.refresh();
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        match std::fs::read_dir(&self.dir) {
            Ok(rd) => {
                for e in rd.flatten() {
                    let path = e.path();
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let is_dir = path.is_dir();
                    if is_dir || remix::is_supported(&path) {
                        self.entries.push(Entry { name, path, is_dir });
                    }
                }
                self.entries.sort_by(|a, b| {
                    b.is_dir
                        .cmp(&a.is_dir)
                        .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                });
            }
            Err(e) => self.error = Some(format!("Não consegui abrir a pasta: {e}")),
        }
    }
}

pub struct RemixUi {
    sample_rate: u32,
    decks: [DeckUi; DECKS],
    load_tx: Sender<LoadResult>,
    load_rx: Receiver<LoadResult>,
    library: Library,
    pub mix: MixUi,
    /// [deck][0 = down (E/U), 1 = up (R/I)]
    filter_keys: [[bool; 2]; DECKS],
    xf_glide: Option<(f32, f32, f64)>,
    message: Option<(String, f64)>,
    last_time: f64,
}

impl RemixUi {
    pub fn new(sample_rate: u32, settings: &Settings) -> Self {
        let (load_tx, load_rx) = unbounded();
        let mut library = Library::default();
        if let Some(dir) = settings
            .music_dir
            .clone()
            .filter(|d| d.is_dir())
            .or_else(crate::settings::default_music_dir)
        {
            library.open(dir);
        }
        Self {
            sample_rate,
            decks: Default::default(),
            load_tx,
            load_rx,
            library,
            mix: MixUi::default(),
            filter_keys: [[false; 2]; DECKS],
            xf_glide: None,
            message: None,
            last_time: 0.0,
        }
    }

    fn say(&mut self, text: impl Into<String>, now: f64) {
        self.message = Some((text.into(), now));
    }

    /// Start decoding `path` for `deck` on a worker thread.
    pub fn request_load(&mut self, deck: usize, path: PathBuf, state: &RemixState, now: f64) {
        if deck >= DECKS {
            return;
        }
        if state.decks[deck].playing {
            self.say(
                format!(
                    "Deck {} está tocando. Pause antes de carregar outra faixa.",
                    DECK_NAMES[deck]
                ),
                now,
            );
            return;
        }
        if !remix::is_supported(&path) {
            self.say("Formato não suportado.", now);
            return;
        }
        self.decks[deck].loading = Some(path.clone());
        let tx = self.load_tx.clone();
        let rate = self.sample_rate;
        std::thread::spawn(move || {
            let result = remix::load_track(&path, rate)
                .map(Arc::new)
                .map_err(|e| e.to_string());
            let _ = tx.send(LoadResult { deck, path, result });
        });
    }

    /// Per-frame housekeeping: finished loads, held keys, glides, memos.
    pub fn tick(
        &mut self,
        now: f64,
        state: &RemixState,
        settings: &mut Settings,
        sink: &dyn RemixSink,
    ) -> bool {
        let mut settings_changed = false;
        let dt = (now - self.last_time).clamp(0.0, 0.1) as f32;
        self.last_time = now;

        while let Ok(done) = self.load_rx.try_recv() {
            let d = done.deck;
            if self.decks[d].loading.as_deref() != Some(done.path.as_path()) {
                continue; // superseded by a newer request
            }
            self.decks[d].loading = None;
            match done.result {
                Ok(track) => {
                    if state.decks[d].playing {
                        self.say(
                            format!("Deck {} começou a tocar; carga cancelada.", DECK_NAMES[d]),
                            now,
                        );
                        continue;
                    }
                    let memo = settings.memo(&track.path);
                    sink.send(RemixCmd::Deck(d, DeckCmd::Load(track.clone())));
                    if let Some(bpm) = memo.bpm {
                        sink.send(RemixCmd::Deck(d, DeckCmd::SetBpm(bpm)));
                    }
                    if memo.hot_cues.iter().any(Option::is_some) {
                        sink.send(RemixCmd::Deck(d, DeckCmd::SetHotCues(memo.hot_cues)));
                    }
                    let bpm = memo.bpm.or(track.analysis.bpm);
                    self.say(
                        match bpm {
                            Some(b) => format!("{} no deck {} ({b:.2} BPM)", track.title, DECK_NAMES[d]),
                            None => format!(
                                "{} no deck {} (sem BPM detectado: use TAP)",
                                track.title, DECK_NAMES[d]
                            ),
                        },
                        now,
                    );
                    // Keep our clone until the engine confirms the swap; the
                    // engine hands the old track back for dropping here.
                    self.decks[d].track = Some(track);
                    self.decks[d].tap.reset();
                }
                Err(e) => self.say(format!("Falha ao carregar: {e}"), now),
            }
        }

        // Remember hot cues per track once the engine is on our track.
        for d in 0..DECKS {
            let ds = &state.decks[d];
            let Some(track) = &self.decks[d].track else {
                continue;
            };
            if !ds.loaded || ds.track_id != track.id {
                continue;
            }
            let mut memo = settings.memo(&track.path);
            if memo.hot_cues != ds.hot_cues {
                memo.hot_cues = ds.hot_cues;
                settings_changed |= settings.set_memo(&track.path, memo);
            }
        }

        for d in 0..DECKS {
            let dir = match self.filter_keys[d] {
                [true, true] => {
                    if self.mix.filter[d] != 0.0 {
                        self.mix.filter[d] = 0.0;
                        sink.send(RemixCmd::SetFilter { deck: d, pos: 0.0 });
                    }
                    0.0
                }
                [true, false] => -1.0,
                [false, true] => 1.0,
                _ => 0.0,
            };
            if dir != 0.0 {
                let next = (self.mix.filter[d] + dir * FILTER_SWEEP_PER_SEC * dt).clamp(-1.0, 1.0);
                if next != self.mix.filter[d] {
                    self.mix.filter[d] = next;
                    sink.send(RemixCmd::SetFilter { deck: d, pos: next });
                }
            }
        }

        if let Some((from, to, t0)) = self.xf_glide {
            let t = ((now - t0) / CROSSFADER_GLIDE_SECS).clamp(0.0, 1.0) as f32;
            let v = from + (to - from) * t;
            self.mix.crossfader = v;
            sink.send(RemixCmd::SetCrossfader(v));
            if t >= 1.0 {
                self.xf_glide = None;
            }
        } else if state.auto.is_some() {
            // The engine is driving the crossfader; follow it.
            self.mix.crossfader = state.crossfader;
        }

        if let Some((_, t)) = &self.message {
            if now - t > 6.0 {
                self.message = None;
            }
        }
        settings_changed
    }

    /// Remix keyboard map. Returns true when the key was used.
    pub fn handle_key(
        &mut self,
        key: Key,
        pressed: bool,
        shift: bool,
        now: f64,
        state: &RemixState,
        settings: &mut Settings,
        sink: &dyn RemixSink,
    ) -> bool {
        let deck_cmd = |d: usize, c: DeckCmd| sink.send(RemixCmd::Deck(d, c));
        // Held keys first (they care about release).
        match key {
            Key::Z | Key::X | Key::N | Key::M => {
                let d = if matches!(key, Key::Z | Key::X) { 0 } else { 1 };
                let dir = if matches!(key, Key::Z | Key::N) { -1.0 } else { 1.0 };
                deck_cmd(d, DeckCmd::Nudge(if pressed { dir } else { 0.0 }));
                return true;
            }
            Key::E | Key::R | Key::U | Key::I => {
                let d = if matches!(key, Key::E | Key::R) { 0 } else { 1 };
                let up = matches!(key, Key::R | Key::I);
                self.filter_keys[d][up as usize] = pressed;
                return true;
            }
            _ => {}
        }
        if !pressed {
            return matches!(
                key,
                Key::Num1
                    | Key::Num2
                    | Key::Num3
                    | Key::Num4
                    | Key::Num7
                    | Key::Num8
                    | Key::Num9
                    | Key::Num0
                    | Key::Q
                    | Key::P
                    | Key::W
                    | Key::O
                    | Key::A
                    | Key::S
                    | Key::D
                    | Key::F
                    | Key::G
                    | Key::H
                    | Key::J
                    | Key::K
                    | Key::L
                    | Key::Semicolon
                    | Key::C
                    | Key::Comma
                    | Key::ArrowLeft
                    | Key::ArrowDown
                    | Key::ArrowRight
                    | Key::Space
            );
        }
        let hot = |k: Key| -> Option<(usize, usize)> {
            Some(match k {
                Key::Num1 => (0, 0),
                Key::Num2 => (0, 1),
                Key::Num3 => (0, 2),
                Key::Num4 => (0, 3),
                Key::Num7 => (1, 0),
                Key::Num8 => (1, 1),
                Key::Num9 => (1, 2),
                Key::Num0 => (1, 3),
                _ => return None,
            })
        };
        if let Some((d, i)) = hot(key) {
            deck_cmd(
                d,
                if shift {
                    DeckCmd::ClearHotCue(i)
                } else {
                    DeckCmd::HotCue(i)
                },
            );
            return true;
        }
        let loop_key = |k: Key| -> Option<(usize, f32)> {
            Some(match k {
                Key::A => (0, 1.0),
                Key::S => (0, 2.0),
                Key::D => (0, 4.0),
                Key::F => (0, 8.0),
                Key::H => (1, 1.0),
                Key::J => (1, 2.0),
                Key::K => (1, 4.0),
                Key::L => (1, 8.0),
                _ => return None,
            })
        };
        if let Some((d, beats)) = loop_key(key) {
            deck_cmd(d, DeckCmd::BeatLoop(beats));
            return true;
        }
        match key {
            Key::Q | Key::P => {
                let d = (key == Key::P) as usize;
                if shift {
                    if let Some(path) = self.library.selected.clone() {
                        self.request_load(d, path, state, now);
                    } else {
                        self.say("Selecione uma faixa na biblioteca primeiro.", now);
                    }
                } else {
                    deck_cmd(d, DeckCmd::TogglePlay);
                }
            }
            Key::W => deck_cmd(0, DeckCmd::Cue),
            Key::O => deck_cmd(1, DeckCmd::Cue),
            Key::G => deck_cmd(0, DeckCmd::ExitLoop),
            Key::Semicolon => deck_cmd(1, DeckCmd::ExitLoop),
            Key::C => sink.send(RemixCmd::SyncDeck(0)),
            Key::Comma => sink.send(RemixCmd::SyncDeck(1)),
            Key::ArrowLeft => self.glide_crossfader(0.0, now, sink),
            Key::ArrowDown => self.glide_crossfader(0.5, now, sink),
            Key::ArrowRight => self.glide_crossfader(1.0, now, sink),
            Key::Space => self.auto_transition(state, settings, now, sink),
            _ => return false,
        }
        true
    }

    /// Release everything that is held (tab switch, focus loss).
    pub fn release_keys(&mut self, sink: &dyn RemixSink) {
        self.filter_keys = [[false; 2]; DECKS];
        for d in 0..DECKS {
            sink.send(RemixCmd::Deck(d, DeckCmd::Nudge(0.0)));
        }
    }

    fn glide_crossfader(&mut self, to: f32, now: f64, sink: &dyn RemixSink) {
        sink.send(RemixCmd::CancelAutoTransition);
        self.xf_glide = Some((self.mix.crossfader, to, now));
    }

    fn auto_transition(
        &mut self,
        state: &RemixState,
        settings: &Settings,
        now: f64,
        sink: &dyn RemixSink,
    ) {
        if state.auto.is_some() {
            sink.send(RemixCmd::CancelAutoTransition);
            self.say("Transição cancelada.", now);
            return;
        }
        let to = if self.mix.crossfader < 0.5 { 1 } else { 0 };
        if !state.decks[to].loaded {
            self.say(
                format!("Carregue uma faixa no deck {} para a transição.", DECK_NAMES[to]),
                now,
            );
            return;
        }
        self.xf_glide = None;
        sink.send(RemixCmd::AutoTransition {
            to,
            beats: settings.auto_beats,
            fallback_secs: 8.0,
        });
        self.say(
            format!(
                "Transição para o deck {} em {} batidas.",
                DECK_NAMES[to], settings.auto_beats as u32
            ),
            now,
        );
    }

    pub fn tap(&mut self, d: usize, now: f64, settings: &mut Settings, sink: &dyn RemixSink) -> bool {
        let Some(bpm) = self.decks[d].tap.tap(now) else {
            return false;
        };
        sink.send(RemixCmd::Deck(d, DeckCmd::SetBpm(bpm)));
        if let Some(track) = &self.decks[d].track {
            let mut memo: TrackMemo = settings.memo(&track.path);
            memo.bpm = Some((bpm * 100.0).round() / 100.0);
            return settings.set_memo(&track.path, memo);
        }
        false
    }

    pub fn handle_dropped_files(
        &mut self,
        ctx: &egui::Context,
        state: &RemixState,
        now: f64,
    ) {
        let dropped: Vec<(PathBuf, Option<Pos2>)> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .map(|p| (p, i.pointer.latest_pos()))
                .collect()
        });
        let width = ctx.screen_rect().width();
        for (path, pos) in dropped {
            let d = match pos {
                Some(p) if p.x > width * 0.5 => 1,
                Some(_) => 0,
                None => usize::from(state.decks[0].loaded),
            };
            self.request_load(d, path, state, now);
        }
    }

    // ---------------------------------------------------------------- UI

    pub fn show(
        &mut self,
        ui: &mut Ui,
        state: &RemixState,
        settings: &mut Settings,
        sink: &dyn RemixSink,
        now: f64,
    ) -> bool {
        let mut settings_changed = false;
        for d in 0..DECKS {
            self.zoom_waveform(ui, d, &state.decks[d], sink);
            ui.add_space(4.0);
        }
        ui.add_space(6.0);

        let full = ui.available_width();
        let mixer_w = 300.0f32.min(full * 0.34);
        let deck_w = ((full - mixer_w - 24.0) * 0.5).max(240.0);
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
            for d in 0..DECKS {
                if d == 1 {
                    ui.add_space(8.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(mixer_w, 0.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| settings_changed |= self.mixer(ui, state, settings, sink, now),
                    );
                    ui.add_space(8.0);
                }
                ui.allocate_ui_with_layout(
                    Vec2::new(deck_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| settings_changed |= self.deck_panel(ui, d, &state.decks[d], settings, sink, now),
                );
            }
        });

        if let Some((text, _)) = &self.message {
            ui.add_space(6.0);
            ui.label(RichText::new(text).size(12.5).color(FG));
        }
        ui.add_space(8.0);
        settings_changed |= self.library_panel(ui, state, settings, now);
        ui.add_space(8.0);
        ui.label(
            RichText::new(
                "Teclado: 1-4 / 7-0 hot cues (Shift apaga) · Q / P play · W / O cue · A S D F / H J K L loop 1 2 4 8 · G / Ç sai do loop · Z X / N M nudge · C / , sync · E R / U I filtro (os dois juntos zeram) · ← ↓ → crossfader · Espaço transição automática · Shift+Q / Shift+P carrega a faixa selecionada · F10 XY pad · Shift+Esc para os dois decks",
            )
            .size(11.5)
            .color(MUTED),
        );
        settings_changed
    }

    fn zoom_waveform(&mut self, ui: &mut Ui, d: usize, ds: &DeckState, sink: &dyn RemixSink) {
        let height = 74.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 10.0, SURFACE);
        painter.text(
            rect.left_top() + Vec2::new(10.0, 6.0),
            egui::Align2::LEFT_TOP,
            DECK_NAMES[d],
            egui::FontId::proportional(13.0),
            DECK_COLORS[d],
        );
        let Some(track) = self.decks[d].track.clone().filter(|t| ds.loaded && t.id == ds.track_id) else {
            let text = match &self.decks[d].loading {
                Some(p) => format!(
                    "carregando {}…",
                    p.file_name().map(|f| f.to_string_lossy()).unwrap_or_default()
                ),
                None => "vazio: arraste um arquivo aqui ou use a biblioteca".to_string(),
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(12.0),
                SUBTLE,
            );
            return;
        };

        let mid_y = rect.center().y;
        let half_h = height * 0.5 - 6.0;
        let pos = ds.position_secs;
        let secs_per_px = 2.0 * ZOOM_HALF_SECS / rect.width() as f64;
        let x_of = |t: f64| rect.center().x + ((t - pos) / secs_per_px) as f32;

        if let Some(l) = ds.loop_state {
            let a = x_of(l.start_secs).max(rect.left());
            let b = x_of(l.end_secs).min(rect.right());
            if b > a {
                painter.rect_filled(
                    Rect::from_x_y_ranges(a..=b, rect.y_range()),
                    0.0,
                    Color32::from_rgba_unmultiplied(90, 210, 130, 28),
                );
            }
        }

        let ov = &track.overview;
        if ov.bins_per_sec > 0.0 && !ov.bins.is_empty() {
            let mut x = rect.left();
            while x < rect.right() {
                let t = pos + (x - rect.center().x) as f64 * secs_per_px;
                if let Some(bin) = ov.bin_at(t) {
                    let h = bin.peak.clamp(0.0, 1.0) * half_h;
                    painter.line_segment(
                        [Pos2::new(x, mid_y - h), Pos2::new(x, mid_y + h)],
                        Stroke::new(1.0, band_color(bin.low, bin.mid, bin.high)),
                    );
                }
                x += 1.0;
            }
        }

        if let Some(bpm) = ds.base_bpm.filter(|b| *b > 0.0) {
            let beat = 60.0 / bpm;
            let first = ((pos - ZOOM_HALF_SECS - ds.first_beat_secs) / beat).floor() as i64;
            let last = ((pos + ZOOM_HALF_SECS - ds.first_beat_secs) / beat).ceil() as i64;
            for n in first..=last {
                let x = x_of(ds.first_beat_secs + n as f64 * beat);
                if x < rect.left() || x > rect.right() {
                    continue;
                }
                let bar = n.rem_euclid(4) == 0;
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.top() + if bar { 12.0 } else { 6.0 })],
                    Stroke::new(if bar { 1.5 } else { 1.0 }, Color32::from_white_alpha(if bar { 150 } else { 70 })),
                );
            }
        }

        for (i, cue) in ds.hot_cues.iter().enumerate() {
            if let Some(t) = cue {
                let x = x_of(*t);
                if x >= rect.left() && x <= rect.right() {
                    painter.line_segment(
                        [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
                        Stroke::new(1.5, CUE_COLORS[i]),
                    );
                    painter.text(
                        Pos2::new(x + 3.0, rect.bottom() - 4.0),
                        egui::Align2::LEFT_BOTTOM,
                        format!("{}", i + 1),
                        egui::FontId::monospace(10.0),
                        CUE_COLORS[i],
                    );
                }
            }
        }
        let cx = x_of(ds.cue_secs);
        if cx >= rect.left() && cx <= rect.right() {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    Pos2::new(cx - 5.0, rect.bottom()),
                    Pos2::new(cx + 5.0, rect.bottom()),
                    Pos2::new(cx, rect.bottom() - 7.0),
                ],
                Color32::from_rgb(250, 190, 70),
                Stroke::NONE,
            ));
        }
        painter.line_segment(
            [
                Pos2::new(rect.center().x, rect.top()),
                Pos2::new(rect.center().x, rect.bottom()),
            ],
            Stroke::new(2.0, FG),
        );

        // Drag on the zoomed waveform scrubs (only while paused).
        if response.dragged() && !ds.playing && ds.duration_secs > 0.0 {
            let dt = -response.drag_delta().x as f64 * secs_per_px;
            let frac = ((pos + dt) / ds.duration_secs).clamp(0.0, 1.0);
            sink.send(RemixCmd::Deck(d, DeckCmd::SeekFraction(frac)));
        }
    }

    fn overview_strip(&self, ui: &mut Ui, d: usize, ds: &DeckState, sink: &dyn RemixSink) {
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 6.0, ELEVATED);
        let Some(track) = self.decks[d].track.as_ref().filter(|t| ds.loaded && t.id == ds.track_id) else {
            return;
        };
        let ov = &track.overview;
        let dur = ds.duration_secs.max(1e-6);
        if !ov.bins.is_empty() {
            let cols = rect.width().max(1.0) as usize;
            let per_col = ov.bins.len() as f32 / cols as f32;
            for c in 0..cols {
                let a = (c as f32 * per_col) as usize;
                let b = (((c + 1) as f32 * per_col) as usize).clamp(a + 1, ov.bins.len());
                let slice = &ov.bins[a.min(ov.bins.len() - 1)..b];
                let (mut peak, mut lo, mut mi, mut hi) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
                for bin in slice {
                    peak = peak.max(bin.peak);
                    lo = lo.max(bin.low);
                    mi = mi.max(bin.mid);
                    hi = hi.max(bin.high);
                }
                let x = rect.left() + c as f32 + 0.5;
                let h = peak.clamp(0.0, 1.0) * (rect.height() * 0.5 - 2.0);
                painter.line_segment(
                    [Pos2::new(x, rect.center().y - h), Pos2::new(x, rect.center().y + h)],
                    Stroke::new(1.0, band_color(lo, mi, hi).gamma_multiply(0.85)),
                );
            }
        }
        let x_of = |t: f64| rect.left() + (t / dur) as f32 * rect.width();
        let played = x_of(ds.position_secs);
        painter.rect_filled(
            Rect::from_min_max(rect.min, Pos2::new(played, rect.bottom())),
            6.0,
            Color32::from_black_alpha(110),
        );
        for (i, cue) in ds.hot_cues.iter().enumerate() {
            if let Some(t) = cue {
                let x = x_of(*t);
                painter.line_segment(
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.top() + 8.0)],
                    Stroke::new(2.0, CUE_COLORS[i]),
                );
            }
        }
        painter.line_segment(
            [Pos2::new(played, rect.top()), Pos2::new(played, rect.bottom())],
            Stroke::new(1.5, FG),
        );
        if (response.clicked() || response.dragged()) && ds.duration_secs > 0.0 {
            if let Some(p) = response.interact_pointer_pos() {
                let frac = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0) as f64;
                sink.send(RemixCmd::Deck(d, DeckCmd::SeekFraction(frac)));
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn deck_panel(
        &mut self,
        ui: &mut Ui,
        d: usize,
        ds: &DeckState,
        settings: &mut Settings,
        sink: &dyn RemixSink,
        now: f64,
    ) -> bool {
        let mut changed = false;
        let cmd = |c: DeckCmd| sink.send(RemixCmd::Deck(d, c));
        let keys = if d == 0 {
            DeckKeys { play: "Q", cue: "W", sync: "C", hot: ["1", "2", "3", "4"], loops: ["A", "S", "D", "F"], exit: "G", nudge: ["Z", "X"], filter: "E R" }
        } else {
            DeckKeys { play: "P", cue: "O", sync: ",", hot: ["7", "8", "9", "0"], loops: ["H", "J", "K", "L"], exit: "Ç", nudge: ["N", "M"], filter: "U I" }
        };
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(theme::panel_stroke())
            .corner_radius(theme::rounding_lg())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(DECK_NAMES[d]).size(18.0).color(DECK_COLORS[d]).strong());
                    let title = self.decks[d]
                        .track
                        .as_ref()
                        .filter(|t| ds.loaded && t.id == ds.track_id)
                        .map(|t| t.title.clone())
                        .unwrap_or_else(|| "—".into());
                    ui.add(egui::Label::new(RichText::new(title).size(13.5).color(FG)).truncate());
                });
                ui.horizontal(|ui| {
                    let bpm = ds.bpm.map(|b| format!("{b:.2}")).unwrap_or_else(|| "--.--".into());
                    ui.label(RichText::new(bpm).monospace().size(20.0).color(FG));
                    ui.label(RichText::new("BPM").size(10.5).color(SUBTLE));
                    ui.add_space(8.0);
                    let speed = (ds.speed - 1.0) * 100.0;
                    ui.label(RichText::new(format!("{speed:+.2}%")).monospace().size(12.0).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let remain = (ds.duration_secs - ds.position_secs).max(0.0);
                        ui.label(
                            RichText::new(format!("{} / -{}", fmt_time(ds.position_secs), fmt_time(remain)))
                                .monospace()
                                .size(12.0)
                                .color(MUTED),
                        );
                    });
                });
                self.overview_strip(ui, d, ds, sink);
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    let play_label = if ds.playing { "❚❚" } else { "▶" };
                    if big_button(ui, play_label, keys.play, ds.playing).clicked() {
                        cmd(DeckCmd::TogglePlay);
                    }
                    if big_button(ui, "CUE", keys.cue, false).clicked() {
                        cmd(DeckCmd::Cue);
                    }
                    if big_button(ui, "SYNC", keys.sync, false).clicked() {
                        sink.send(RemixCmd::SyncDeck(d));
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    for i in 0..HOT_CUES {
                        let set = ds.hot_cues[i].is_some();
                        let fill = if set { CUE_COLORS[i] } else { ELEVATED };
                        let text = if set { BG } else { MUTED };
                        let r = ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("{}\n{}", i + 1, keys.hot[i])).size(11.0).color(text),
                                )
                                .fill(fill)
                                .corner_radius(theme::rounding_sm())
                                .min_size(Vec2::new(44.0, 38.0)),
                            )
                            .on_hover_text("Clique: grava ou pula. Botão direito / Shift+tecla: apaga.");
                        if r.clicked() {
                            cmd(DeckCmd::HotCue(i));
                        }
                        if r.secondary_clicked() {
                            cmd(DeckCmd::ClearHotCue(i));
                        }
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::section_label("LOOP"));
                    for (i, beats) in LOOP_BEATS.iter().enumerate() {
                        let active = ds.loop_state.is_some_and(|l| (l.beats - beats).abs() < 1e-3);
                        if chip(ui, &format!("{} · {}", *beats as u32, keys.loops[i]), active).clicked() {
                            cmd(DeckCmd::BeatLoop(*beats));
                        }
                    }
                    if chip(ui, "½", false).clicked() {
                        cmd(DeckCmd::LoopHalve);
                    }
                    if chip(ui, "×2", false).clicked() {
                        cmd(DeckCmd::LoopDouble);
                    }
                    if chip(ui, &format!("sair · {}", keys.exit), false).clicked() {
                        cmd(DeckCmd::ExitLoop);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::section_label("TEMPO"));
                    let mut fader = ds.tempo_fader;
                    ui.spacing_mut().slider_width = 150.0;
                    if ui
                        .add(egui::Slider::new(&mut fader, -1.0..=1.0).show_value(false))
                        .on_hover_text("Duplo clique zera")
                        .changed()
                    {
                        cmd(DeckCmd::SetTempoFader(fader));
                    }
                    for range in TEMPO_RANGES {
                        let label = format!("±{}", (range * 100.0).round() as u32);
                        if chip(ui, &label, (ds.tempo_range - range).abs() < 1e-4).clicked() {
                            cmd(DeckCmd::SetTempoRange(range));
                        }
                    }
                    if chip(ui, "0", false).clicked() {
                        cmd(DeckCmd::SetTempoFader(0.0));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    for (i, (label, dir)) in [("−", -1.0f32), ("+", 1.0)].into_iter().enumerate() {
                        let r = ui.add(
                            egui::Button::new(RichText::new(format!("{label} {}", keys.nudge[i])).size(12.0))
                                .fill(ELEVATED)
                                .sense(Sense::click_and_drag()),
                        );
                        if r.is_pointer_button_down_on() {
                            cmd(DeckCmd::Nudge(dir));
                        } else if r.drag_stopped() || r.clicked() {
                            cmd(DeckCmd::Nudge(0.0));
                        }
                    }
                    if chip(ui, "TAP", false).clicked() && self.tap(d, now, settings, sink) {
                        changed = true;
                    }
                    if chip(ui, "grid aqui", false)
                        .on_hover_text("Marca a batida 1 na posição atual")
                        .clicked()
                    {
                        cmd(DeckCmd::SetGridAnchorHere);
                    }
                    if let Some(base) = ds.base_bpm {
                        let mut b = base;
                        if ui
                            .add(egui::DragValue::new(&mut b).speed(0.01).range(30.0..=300.0).max_decimals(2).suffix(" bpm"))
                            .on_hover_text("BPM original da faixa (arraste para corrigir)")
                            .changed()
                        {
                            cmd(DeckCmd::SetBpm(b));
                            if let Some(t) = &self.decks[d].track {
                                let mut memo = settings.memo(&t.path);
                                memo.bpm = Some((b * 100.0).round() / 100.0);
                                changed |= settings.set_memo(&t.path, memo);
                            }
                        }
                    }
                    ui.label(RichText::new(format!("filtro: {}", keys.filter)).size(11.0).color(SUBTLE));
                });
            });
        changed
    }

    #[allow(clippy::too_many_lines)]
    fn mixer(
        &mut self,
        ui: &mut Ui,
        state: &RemixState,
        settings: &mut Settings,
        sink: &dyn RemixSink,
        now: f64,
    ) -> bool {
        let mut changed = false;
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(theme::panel_stroke())
            .corner_radius(theme::rounding_lg())
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    for d in 0..DECKS {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(DECK_NAMES[d]).color(DECK_COLORS[d]).strong());
                            for (b, band) in [Band::High, Band::Mid, Band::Low].into_iter().enumerate() {
                                let _ = b;
                                let bi = band.index();
                                ui.horizontal(|ui| {
                                    let id = EQ_IDS[d][bi];
                                    let label = ["LOW", "MID", "HIGH"][bi];
                                    if knob(ui, id, label, &mut self.mix.eq[d][bi], 0.0, remix::dsp::EQ_MAX_GAIN, 0.0, false, fmt_gain_db) {
                                        if (self.mix.eq[d][bi] - 1.0).abs() < 0.04 {
                                            self.mix.eq[d][bi] = 1.0;
                                        }
                                        sink.send(RemixCmd::SetEq { deck: d, band, gain: self.mix.eq[d][bi] });
                                    }
                                    let k = self.mix.kills[d][bi];
                                    if ui
                                        .add(
                                            egui::Button::new(RichText::new("K").size(10.0).color(if k { ACCENT_FG } else { MUTED }))
                                                .fill(if k { Color32::from_rgb(240, 84, 84) } else { ELEVATED })
                                                .min_size(Vec2::splat(22.0)),
                                        )
                                        .on_hover_text("Kill: corta a banda")
                                        .clicked()
                                    {
                                        self.mix.kills[d][bi] = !k;
                                        sink.send(RemixCmd::SetKill { deck: d, band, on: !k });
                                    }
                                });
                            }
                            if knob(ui, FILTER_IDS[d], "FILTER", &mut self.mix.filter[d], -1.0, 1.0, 0.0, false, fmt_filter) {
                                if self.mix.filter[d].abs() < 0.03 {
                                    self.mix.filter[d] = 0.0;
                                }
                                sink.send(RemixCmd::SetFilter { deck: d, pos: self.mix.filter[d] });
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().slider_width = 110.0;
                                if ui
                                    .add(egui::Slider::new(&mut self.mix.volume[d], 0.0..=1.0).vertical().show_value(false))
                                    .changed()
                                {
                                    sink.send(RemixCmd::SetVolume { deck: d, gain: self.mix.volume[d] });
                                }
                                meter(ui, state.levels[d], 110.0);
                            });
                        });
                        if d == 0 {
                            ui.add_space(6.0);
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label(theme::section_label("CROSSFADER  ← ↓ →"));
                ui.spacing_mut().slider_width = ui.available_width() - 8.0;
                let mut xf = self.mix.crossfader;
                if ui.add(egui::Slider::new(&mut xf, 0.0..=1.0).show_value(false)).changed() {
                    self.xf_glide = None;
                    self.mix.crossfader = xf;
                    sink.send(RemixCmd::SetCrossfader(xf));
                }
                ui.horizontal_wrapped(|ui| {
                    let sharp = settings.sharp_crossfader;
                    if chip(ui, if sharp { "corte" } else { "suave" }, sharp)
                        .on_hover_text("Curva do crossfader")
                        .clicked()
                    {
                        settings.sharp_crossfader = !sharp;
                        sink.send(RemixCmd::SetCrossfaderCurve(if settings.sharp_crossfader {
                            CrossfaderCurve::Sharp
                        } else {
                            CrossfaderCurve::Smooth
                        }));
                        changed = true;
                    }
                    let auto_on = state.auto.is_some();
                    let label = match state.auto {
                        Some(a) => format!("AUTO {}%", (a.progress * 100.0).round() as u32),
                        None => "AUTO · espaço".into(),
                    };
                    if chip(ui, &label, auto_on).clicked() {
                        self.auto_transition(state, settings, now, sink);
                    }
                    egui::ComboBox::from_id_salt("auto-beats")
                        .width(70.0)
                        .selected_text(format!("{} beats", settings.auto_beats as u32))
                        .show_ui(ui, |ui| {
                            for b in AUTO_BEATS {
                                if ui
                                    .selectable_label((settings.auto_beats - b).abs() < 1e-3, format!("{} beats", b as u32))
                                    .clicked()
                                {
                                    settings.auto_beats = b;
                                    changed = true;
                                }
                            }
                        });
                });
                ui.add_space(6.0);
                ui.separator();
                ui.label(theme::section_label("MASTER"));
                ui.horizontal_wrapped(|ui| {
                    if knob(ui, "m-filter", "FILTER", &mut self.mix.master_filter, -1.0, 1.0, 0.0, false, fmt_filter) {
                        if self.mix.master_filter.abs() < 0.03 {
                            self.mix.master_filter = 0.0;
                        }
                        sink.send(RemixCmd::SetMasterFilter(self.mix.master_filter));
                    }
                    let mut echo_changed = false;
                    echo_changed |= knob(ui, "echo-mix", "ECHO", &mut self.mix.echo_mix, 0.0, 1.0, 0.01, false, |v| format!("{}%", (v * 100.0).round()));
                    echo_changed |= knob(ui, "echo-fb", "REPEAT", &mut self.mix.echo_feedback, 0.0, 0.95, 0.01, false, |v| format!("{}%", (v * 100.0).round()));
                    let mut idx = crate::xy::ECHO_BEATS
                        .iter()
                        .position(|b| (*b - self.mix.echo_beats).abs() < 1e-4)
                        .unwrap_or(4) as f32;
                    if knob(ui, "echo-time", "TIME", &mut idx, 0.0, (crate::xy::ECHO_BEATS.len() - 1) as f32, 1.0, false, |v| {
                        crate::xy::XyTarget::EchoTime.format(crate::xy::ECHO_BEATS[(v as usize).min(crate::xy::ECHO_BEATS.len() - 1)])
                    }) {
                        self.mix.echo_beats = crate::xy::ECHO_BEATS[(idx as usize).min(crate::xy::ECHO_BEATS.len() - 1)];
                        echo_changed = true;
                    }
                    if echo_changed {
                        self.send_echo(sink);
                    }
                    if knob(ui, "m-gain", "GAIN", &mut self.mix.master_gain, 0.0, 1.5, 0.01, false, |v| format!("{}%", (v * 100.0).round())) {
                        sink.send(RemixCmd::SetMasterGain(self.mix.master_gain));
                    }
                    ui.vertical(|ui| {
                        meter(ui, state.master_level, 70.0);
                        let limiting = state.limiter_reduction < 0.97;
                        ui.label(
                            RichText::new("LIM")
                                .size(9.5)
                                .color(if limiting { Color32::from_rgb(240, 84, 84) } else { SUBTLE }),
                        );
                    });
                });
            });
        changed
    }

    pub fn send_echo(&self, sink: &dyn RemixSink) {
        sink.send(RemixCmd::SetEcho {
            mix: self.mix.echo_mix,
            feedback: self.mix.echo_feedback,
            beats: self.mix.echo_beats,
        });
    }

    fn library_panel(&mut self, ui: &mut Ui, state: &RemixState, settings: &mut Settings, now: f64) -> bool {
        let mut changed = false;
        let mut load: Option<(usize, PathBuf)> = None;
        let mut enter: Option<PathBuf> = None;
        egui::Frame::new()
            .fill(SURFACE)
            .stroke(theme::panel_stroke())
            .corner_radius(theme::rounding_lg())
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(theme::section_label("BIBLIOTECA"));
                    if ui.button("↑").on_hover_text("Pasta de cima").clicked() {
                        if let Some(parent) = self.library.dir.parent() {
                            enter = Some(parent.to_path_buf());
                        }
                    }
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.library.path_edit)
                            .desired_width((ui.available_width() * 0.55).max(160.0))
                            .hint_text("pasta de músicas"),
                    );
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        enter = Some(PathBuf::from(self.library.path_edit.trim()));
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut self.library.filter)
                            .desired_width(140.0)
                            .hint_text("buscar"),
                    );
                    if ui.button("Atualizar").clicked() {
                        self.library.refresh();
                    }
                });
                if let Some(err) = &self.library.error {
                    ui.colored_label(Color32::from_rgb(220, 80, 80), err);
                }
                let needle = self.library.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .id_salt("library")
                    .max_height(220.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for e in &self.library.entries {
                            if !needle.is_empty() && !e.name.to_lowercase().contains(&needle) {
                                continue;
                            }
                            ui.horizontal(|ui| {
                                if e.is_dir {
                                    if ui
                                        .add(egui::Button::new(RichText::new(format!("▸ {}", e.name)).color(MUTED)).frame(false))
                                        .clicked()
                                    {
                                        enter = Some(e.path.clone());
                                    }
                                    return;
                                }
                                for d in 0..DECKS {
                                    let busy = state.decks[d].playing;
                                    if ui
                                        .add_enabled(
                                            !busy,
                                            egui::Button::new(RichText::new(DECK_NAMES[d]).size(11.0).color(DECK_COLORS[d]))
                                                .fill(ELEVATED)
                                                .min_size(Vec2::new(26.0, 22.0)),
                                        )
                                        .on_disabled_hover_text("Deck tocando")
                                        .clicked()
                                    {
                                        load = Some((d, e.path.clone()));
                                    }
                                }
                                let selected = self.library.selected.as_deref() == Some(e.path.as_path());
                                let memo = settings.memo(&e.path);
                                let label = match memo.bpm {
                                    Some(b) => format!("{}   {b:.1}", e.name),
                                    None => e.name.clone(),
                                };
                                let r = ui.selectable_label(selected, label);
                                if r.clicked() {
                                    self.library.selected = Some(e.path.clone());
                                }
                                if r.double_clicked() {
                                    let d = if self.mix.crossfader < 0.5 { 1 } else { 0 };
                                    load = Some((d, e.path.clone()));
                                }
                            });
                        }
                        if self.library.entries.is_empty() && self.library.error.is_none() {
                            ui.label(RichText::new("Nenhum áudio nesta pasta (mp3, flac, wav, ogg, m4a, aiff).").color(SUBTLE));
                        }
                    });
            });
        if let Some(dir) = enter {
            self.library.open(dir);
            if self.library.error.is_none() && settings.music_dir.as_deref() != Some(self.library.dir.as_path()) {
                settings.music_dir = Some(self.library.dir.clone());
                changed = true;
            }
        }
        if let Some((d, path)) = load {
            self.request_load(d, path, state, now);
        }
        changed
    }
}

struct DeckKeys {
    play: &'static str,
    cue: &'static str,
    sync: &'static str,
    hot: [&'static str; HOT_CUES],
    loops: [&'static str; 4],
    exit: &'static str,
    nudge: [&'static str; 2],
    filter: &'static str,
}

const EQ_IDS: [[&str; 3]; DECKS] = [["eq-a-low", "eq-a-mid", "eq-a-high"], ["eq-b-low", "eq-b-mid", "eq-b-high"]];
const FILTER_IDS: [&str; DECKS] = ["filter-a", "filter-b"];

fn big_button(ui: &mut Ui, label: &str, key: &str, active: bool) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("{label}\n{key}"))
                .size(13.0)
                .color(if active { ACCENT_FG } else { FG }),
        )
        .fill(if active { ACCENT } else { ELEVATED })
        .corner_radius(theme::rounding_sm())
        .min_size(Vec2::new(64.0, 42.0)),
    )
}

fn meter(ui: &mut Ui, level: f32, height: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(8.0, height), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, ELEVATED);
    let l = level.clamp(0.0, 1.2) / 1.2;
    let top = rect.bottom() - rect.height() * l;
    let color = if level > 0.98 {
        Color32::from_rgb(240, 84, 84)
    } else if level > 0.7 {
        Color32::from_rgb(250, 190, 70)
    } else {
        Color32::from_rgb(90, 210, 130)
    };
    p.rect_filled(Rect::from_min_max(Pos2::new(rect.left(), top), rect.max), 3.0, color);
}

fn band_color(low: f32, mid: f32, high: f32) -> Color32 {
    let sum = (low + mid + high).max(1e-4);
    let mix = |a: u8, b: u8, c: u8| ((a as f32 * low + b as f32 * mid + c as f32 * high) / sum) as u8;
    Color32::from_rgb(
        mix(LOW_COLOR.r(), MID_COLOR.r(), HIGH_COLOR.r()),
        mix(LOW_COLOR.g(), MID_COLOR.g(), HIGH_COLOR.g()),
        mix(LOW_COLOR.b(), MID_COLOR.b(), HIGH_COLOR.b()),
    )
}

fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0);
    let m = (s / 60.0).floor();
    format!("{}:{:04.1}", m as u32, s - m * 60.0)
}

fn fmt_gain_db(g: f32) -> String {
    if g <= 0.001 {
        "-∞".into()
    } else {
        format!("{:+.0} dB", 20.0 * g.log10())
    }
}

fn fmt_filter(p: f32) -> String {
    if p.abs() < 0.03 {
        "off".into()
    } else if p < 0.0 {
        format!("LP {}", (p.abs() * 100.0).round())
    } else {
        format!("HP {}", (p * 100.0).round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format() {
        assert_eq!(fmt_time(0.0), "0:00.0");
        assert_eq!(fmt_time(83.25), "1:23.2");
    }

    #[test]
    fn band_colors_follow_the_dominant_band() {
        assert_eq!(band_color(1.0, 0.0, 0.0), LOW_COLOR);
        assert_eq!(band_color(0.0, 0.0, 1.0), HIGH_COLOR);
    }

    #[test]
    fn gain_readout() {
        assert_eq!(fmt_gain_db(1.0), "+0 dB");
        assert_eq!(fmt_gain_db(0.0), "-∞");
    }
}
