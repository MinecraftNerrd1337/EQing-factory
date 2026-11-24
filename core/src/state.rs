//! Application logic: sessions, folders, presets, history,
//! tabs, filters, undo/redo, inbox. Independent of uniFFI.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::dsp::{self, add_response, CurveData, FilterKind};
use crate::engine::{self, AutoeqOutput, Engine, Mode};
use crate::error::CoreError;
use crate::formats::{self, ParsedFilter};
use crate::model::*;

pub const RESULT_ID: &str = "result";
pub const EQ_ID: &str = "eq";

/// Automatic colors: the hue advances by the golden angle (137.5°)
/// for each new curve, rendered in OKLCH at fixed lightness and chroma
/// (between typical light- and dark-theme settings, to stay readable in both themes).
pub const GOLDEN_ANGLE: f64 = 137.5;
const OKLCH_L: f64 = 0.62;
const OKLCH_C: f64 = 0.21;

const UNDO_LIMIT: usize = 200;
const COALESCE_MS: u128 = 1200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Role {
    Measurement,
    Target,
    /// subtracted from the measurement before autoeq
    Compensation,
    Compare,
}

impl Tab {
    fn slot(&mut self, role: Role) -> Option<&mut Option<Id>> {
        match role {
            Role::Measurement => Some(&mut self.measurement),
            Role::Target => Some(&mut self.target),
            Role::Compensation => Some(&mut self.compensation),
            Role::Compare => None,
        }
    }

    /// Role of a curve in the tab. Any curve can take any role.
    pub fn role_of(&self, curve: &str) -> Role {
        if self.measurement.as_deref() == Some(curve) {
            Role::Measurement
        } else if self.target.as_deref() == Some(curve) {
            Role::Target
        } else if self.compensation.as_deref() == Some(curve) {
            Role::Compensation
        } else {
            Role::Compare
        }
    }

    fn clear_roles_of(&mut self, curve: &str) {
        for r in [Role::Measurement, Role::Target, Role::Compensation] {
            if let Some(slot) = self.slot(r) {
                if slot.as_deref() == Some(curve) {
                    *slot = None;
                }
            }
        }
    }

    fn curve_in(&self, slot: &Option<Id>) -> Option<&TabCurve> {
        slot.as_ref().and_then(|id| self.curves.iter().find(|c| &c.id == id))
    }

    /// Compensation on a grid (zeros when there is no compensation). Its vertical offset
    /// is only visual: it is the shape that gets subtracted.
    pub fn compensation_on(&self, grid: &[f64]) -> Vec<f64> {
        match self.curve_in(&self.compensation) {
            Some(c) => c.data.resample(grid),
            None => vec![0.0; grid.len()],
        }
    }

    /// Compensated measurement (including the measurement's offset) + response of `filters`.
    pub fn corrected_on(&self, m: &TabCurve, filters: &[Filter], grid: &[f64], with_offset: bool) -> Vec<f64> {
        let eq = State::eq_response(filters, grid);
        let comp = self.compensation_on(grid);
        let off = if with_offset { m.offset_db } else { 0.0 };
        m.data.resample(grid).iter().zip(eq).zip(comp).map(|((v, e), c)| v + off - c + e).collect()
    }
}

#[derive(Clone, Debug, PartialEq, uniffi::Enum)]
pub enum CurveRef {
    Curve { id: Id },
    Result,
    Eq,
}

#[derive(Default)]
struct UndoStack {
    undo: Vec<Tab>,
    redo: Vec<Tab>,
    last_key: Option<String>,
    last_at: Option<Instant>,
}

/// What is needed to run autoeq outside the lock.
pub struct RunJob {
    pub session: Id,
    pub tab: Id,
    pub engine: Engine,
    pub mode: Mode,
    pub bands: u32,
    pub steps: u32,
    pub dst: Vec<f32>,
    pub src: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DuplicateReason {
    SamePath,
    SameContent,
}

#[derive(Clone, Debug)]
pub struct DuplicateMatch {
    pub preset_id: Id,
    pub preset_name: String,
    pub reason: DuplicateReason,
    pub visible_in_session: bool,
}

pub struct State {
    pub store: Store,
    path: Option<PathBuf>,
    undo: HashMap<Id, UndoStack>,
    rng: u64,
    counter: u64,
    /// hue of the next automatic color
    hue: f64,
    pub dirty: bool,
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn file_name_of(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn stem_of(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

pub fn read_curve_file(path: &str) -> Result<CurveData, CoreError> {
    if !Path::new(path).is_file() {
        return Err(CoreError::FileMissing { path: path.to_string() });
    }
    let bytes = fs::read(path).map_err(|e| CoreError::Io { detail: format!("{path}: {e}") })?;
    let text = String::from_utf8_lossy(&bytes);
    dsp::parse_curve_text(&text).map_err(|e| CoreError::Parse { detail: format!("{}: {e}", file_name_of(path)) })
}

/// "Measurement + EQ", or "Measurement − compensation + EQ".
pub fn result_label(t: &Tab, m: &TabCurve) -> String {
    match t.curve_in(&t.compensation) {
        Some(c) => format!("{} − {} + EQ", m.label, c.label),
        None => format!("{} + EQ", m.label),
    }
}

/// OKLCH -> sRGB hexadecimal; chroma is reduced if the color falls outside the gamut.
pub fn oklch_to_hex(l: f64, c: f64, h: f64) -> String {
    let lin = |v: f64| if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    let mut c = c;
    loop {
        let (a, b) = (c * h.to_radians().cos(), c * h.to_radians().sin());
        let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
        let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
        let s_ = (l - 0.0894841775 * a - 1.2914855480 * b).powi(3);
        let r = 4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_;
        let g = -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_;
        let bl = -0.0041960863 * l_ - 0.7034186147 * m_ + 1.7076147010 * s_;
        let ok = [r, g, bl].iter().all(|v| (-1e-4..=1.0001).contains(v));
        if ok || c <= 0.0 {
            let to = |v: f64| (lin(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
            return format!("#{:02X}{:02X}{:02X}", to(r), to(g), to(bl));
        }
        c -= 0.005;
    }
}

pub fn normalize_hex(s: &str) -> Option<String> {
    let t = s.trim().trim_start_matches('#');
    if t.len() == 6 && t.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(format!("#{}", t.to_ascii_uppercase()))
    } else {
        None
    }
}

impl State {
    // ------------------------------------------------------------ loading

    pub fn in_memory() -> State {
        let mut s = State {
            store: Store::default(),
            path: None,
            undo: HashMap::new(),
            rng: 0x9E3779B97F4A7C15 ^ now_ms() as u64,
            counter: 0,
            hue: 0.0,
            dirty: false,
        };
        s.hue = s.rand_f() * 360.0;
        s.ensure_session();
        s
    }

    /// Loads `<data_dir>/state.json`. An unreadable file is set aside
    /// (state.corrupt-<date>.json) rather than blocking the app.
    pub fn open(data_dir: &str) -> Result<State, CoreError> {
        let dir = PathBuf::from(data_dir);
        fs::create_dir_all(&dir).map_err(|e| CoreError::Io { detail: format!("{data_dir}: {e}") })?;
        let path = dir.join("state.json");
        let mut s = State::in_memory();
        s.path = Some(path.clone());
        if path.is_file() {
            let text = fs::read_to_string(&path).map_err(|e| CoreError::Io { detail: e.to_string() })?;
            match serde_json::from_str::<Store>(&text) {
                Ok(store) => s.store = store,
                Err(_) => {
                    let bak = dir.join(format!("state.corrupt-{}.json", now_ms()));
                    let _ = fs::rename(&path, bak);
                }
            }
        }
        s.ensure_session();
        s.dirty = true;
        s.save()?;
        Ok(s)
    }

    pub fn save(&mut self) -> Result<(), CoreError> {
        let Some(path) = &self.path else {
            self.dirty = false;
            return Ok(());
        };
        let json = serde_json::to_vec(&self.store).map_err(|e| CoreError::Io { detail: e.to_string() })?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json).map_err(|e| CoreError::Io { detail: e.to_string() })?;
        fs::rename(&tmp, path).map_err(|e| CoreError::Io { detail: e.to_string() })?;
        self.dirty = false;
        Ok(())
    }

    fn touch(&mut self) {
        self.dirty = true;
    }

    pub fn new_id(&mut self) -> Id {
        self.counter += 1;
        let r = self.next_rand();
        format!("{:x}{:04x}{:04x}", now_ms(), self.counter & 0xffff, r & 0xffff)
    }

    fn next_rand(&mut self) -> u64 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn rand_f(&mut self) -> f64 {
        (self.next_rand() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn ensure_session(&mut self) {
        if !self.store.sessions.iter().any(|s| !s.is_zombie()) {
            let id = self.create_session_raw("Session 1".into(), None);
            self.store.active_session = Some(id);
        }
        let active_ok = self
            .store
            .active_session
            .as_ref()
            .and_then(|a| self.store.sessions.iter().find(|s| &s.id == a))
            .is_some_and(|s| !s.is_zombie());
        if !active_ok {
            self.store.active_session = self.store.sessions.iter().find(|s| !s.is_zombie()).map(|s| s.id.clone());
        }
    }

    // ------------------------------------------------------------ access

    pub fn session(&self, id: &str) -> Result<&Session, CoreError> {
        self.store
            .sessions
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("session {id}") })
    }

    fn session_mut(&mut self, id: &str) -> Result<&mut Session, CoreError> {
        self.store
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("session {id}") })
    }

    fn live_session_mut(&mut self, id: &str) -> Result<&mut Session, CoreError> {
        let s = self.session_mut(id)?;
        if s.is_zombie() {
            return Err(CoreError::Invalid { detail: format!("session “{}” has been deleted", s.name) });
        }
        Ok(s)
    }

    pub fn find_tab(&self, tab: &str) -> Result<(&Session, &Tab), CoreError> {
        for s in &self.store.sessions {
            if let Some(t) = s.tabs.iter().find(|t| t.id == tab) {
                return Ok((s, t));
            }
        }
        Err(CoreError::NotFound { what: format!("tab {tab}") })
    }

    pub fn tab(&self, tab: &str) -> Result<&Tab, CoreError> {
        self.find_tab(tab).map(|(_, t)| t)
    }

    fn tab_mut(&mut self, tab: &str) -> Result<&mut Tab, CoreError> {
        for s in &mut self.store.sessions {
            if let Some(t) = s.tabs.iter_mut().find(|t| t.id == tab) {
                return Ok(t);
            }
        }
        Err(CoreError::NotFound { what: format!("tab {tab}") })
    }

    pub fn preset(&self, id: &str) -> Result<&Preset, CoreError> {
        self.store
            .presets
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("preset {id}") })
    }

    fn preset_mut(&mut self, id: &str) -> Result<&mut Preset, CoreError> {
        self.store
            .presets
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("preset {id}") })
    }

    // ------------------------------------------------------------ settings

    pub fn set_settings(&mut self, s: AppSettings) -> Result<(), CoreError> {
        if s.default_bands == 0 || s.default_bands as usize > engine::MAX_N {
            return Err(CoreError::Invalid { detail: format!("number of bands must be between 1 and {}", engine::MAX_N) });
        }
        if s.default_steps == 0 {
            return Err(CoreError::Invalid { detail: "invalid number of steps".into() });
        }
        self.store.settings = s;
        self.touch();
        Ok(())
    }

    /// Effective settings of a session (session override, otherwise the app default).
    pub fn effective_defaults(&self, session: &str) -> (u32, u32, Mode) {
        let st = &self.store.settings;
        match self.session(session) {
            Ok(s) => (
                s.bands.unwrap_or(st.default_bands),
                s.steps.unwrap_or(st.default_steps),
                s.mode.unwrap_or(st.default_mode),
            ),
            Err(_) => (st.default_bands, st.default_steps, st.default_mode),
        }
    }

    pub fn set_session_defaults(
        &mut self,
        session: &str,
        bands: Option<u32>,
        steps: Option<u32>,
        mode: Option<Mode>,
    ) -> Result<(), CoreError> {
        if let Some(b) = bands {
            check_bands(b)?;
        }
        if steps == Some(0) {
            return Err(CoreError::Invalid { detail: "invalid number of steps".into() });
        }
        let s = self.live_session_mut(session)?;
        s.bands = bands;
        s.steps = steps;
        s.mode = mode;
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ folders

    pub fn create_folder(&mut self, name: String, parent: Option<Id>) -> Result<Id, CoreError> {
        if let Some(p) = &parent {
            self.folder_exists(p)?;
        }
        let id = self.new_id();
        self.store.folders.push(Folder { id: id.clone(), name: clean_name(name, "Folder"), parent });
        self.touch();
        Ok(id)
    }

    fn folder_exists(&self, id: &str) -> Result<(), CoreError> {
        if self.store.folders.iter().any(|f| f.id == id) {
            Ok(())
        } else {
            Err(CoreError::NotFound { what: format!("folder {id}") })
        }
    }

    pub fn rename_folder(&mut self, id: &str, name: String) -> Result<(), CoreError> {
        let f = self
            .store
            .folders
            .iter_mut()
            .find(|f| f.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("folder {id}") })?;
        f.name = clean_name(name, &f.name.clone());
        self.touch();
        Ok(())
    }

    /// Moves a folder; refuses to put it inside one of its descendants.
    pub fn move_folder(&mut self, id: &str, parent: Option<Id>) -> Result<(), CoreError> {
        self.folder_exists(id)?;
        if let Some(p) = &parent {
            self.folder_exists(p)?;
            let mut cur = Some(p.clone());
            while let Some(c) = cur {
                if c == id {
                    return Err(CoreError::Invalid { detail: "a folder cannot be moved into itself".into() });
                }
                cur = self.store.folders.iter().find(|f| f.id == c).and_then(|f| f.parent.clone());
            }
        }
        if let Some(f) = self.store.folders.iter_mut().find(|f| f.id == id) {
            f.parent = parent;
        }
        self.touch();
        Ok(())
    }

    /// Deletes a folder: its contents move up one level.
    pub fn delete_folder(&mut self, id: &str) -> Result<(), CoreError> {
        let parent = self
            .store
            .folders
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| CoreError::NotFound { what: format!("folder {id}") })?
            .parent
            .clone();
        for f in &mut self.store.folders {
            if f.parent.as_deref() == Some(id) {
                f.parent = parent.clone();
            }
        }
        for s in &mut self.store.sessions {
            if s.folder.as_deref() == Some(id) {
                s.folder = parent.clone();
            }
        }
        self.store.folders.retain(|f| f.id != id);
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ sessions

    fn create_session_raw(&mut self, name: String, folder: Option<Id>) -> Id {
        let id = self.new_id();
        let mut s = Session {
            id: id.clone(),
            name,
            folder,
            created_at: now_ms(),
            deleted_at: None,
            bands: None,
            steps: None,
            mode: None,
            tabs: vec![],
            active_tab: None,
            history: vec![],
        };
        let st = &self.store.settings;
        let tab = self.blank_tab("Tab 1".into(), st.default_bands, st.default_steps, st.default_mode);
        s.active_tab = Some(tab.id.clone());
        s.tabs.push(tab);
        self.store.sessions.push(s);
        self.touch();
        id
    }

    pub fn create_session(&mut self, name: String, folder: Option<Id>) -> Result<Id, CoreError> {
        if let Some(f) = &folder {
            self.folder_exists(f)?;
        }
        let n = self.store.sessions.iter().filter(|s| !s.is_zombie()).count() + 1;
        let id = self.create_session_raw(clean_name(name, &format!("Session {n}")), folder);
        Ok(id)
    }

    pub fn rename_session(&mut self, id: &str, name: String) -> Result<(), CoreError> {
        let s = self.session_mut(id)?;
        s.name = clean_name(name, &s.name.clone());
        self.touch();
        Ok(())
    }

    pub fn move_session(&mut self, id: &str, folder: Option<Id>) -> Result<(), CoreError> {
        if let Some(f) = &folder {
            self.folder_exists(f)?;
        }
        self.live_session_mut(id)?.folder = folder;
        self.touch();
        Ok(())
    }

    pub fn set_active_session(&mut self, id: &str) -> Result<(), CoreError> {
        self.live_session_mut(id)?;
        self.store.active_session = Some(id.to_string());
        self.touch();
        Ok(())
    }

    /// Deletes a session: its tabs and its history are removed, but it remains
    /// as a "zombie" so that its links (preset visibility and pinning, inbox
    /// items) can be moved over to another session.
    pub fn delete_session(&mut self, id: &str) -> Result<(), CoreError> {
        let tabs: Vec<Id> = {
            let s = self.live_session_mut(id)?;
            s.deleted_at = Some(now_ms());
            s.folder = None;
            s.active_tab = None;
            s.history.clear();
            s.tabs.drain(..).map(|t| t.id).collect()
        };
        for t in tabs {
            self.undo.remove(&t);
        }
        self.ensure_session();
        self.touch();
        Ok(())
    }

    /// Number of presets and inbox items that still refer to a session.
    pub fn binding_count(&self, session: &str) -> u32 {
        let p = self
            .store
            .presets
            .iter()
            .filter(|p| p.visibility.sessions.iter().any(|s| s == session) || p.pinned_in.iter().any(|s| s == session))
            .count();
        let i = self.store.inbox.iter().filter(|i| i.to_session.as_deref() == Some(session)).count();
        (p + i) as u32
    }

    /// Moves all links of a zombie session over to `target` (merged
    /// with the ones `target` already had), then removes the zombie.
    pub fn rebind_zombie(&mut self, zombie: &str, target: &str) -> Result<u32, CoreError> {
        if !self.session(zombie)?.is_zombie() {
            return Err(CoreError::Invalid { detail: "only a deleted session can be rebound".into() });
        }
        self.live_session_mut(target)?;
        let mut n = 0;
        let swap = |v: &mut Vec<Id>| -> bool {
            if let Some(pos) = v.iter().position(|s| s == zombie) {
                v.remove(pos);
                if !v.iter().any(|s| s == target) {
                    v.push(target.to_string());
                }
                true
            } else {
                false
            }
        };
        for p in &mut self.store.presets {
            let a = swap(&mut p.visibility.sessions);
            let b = swap(&mut p.pinned_in);
            if a || b {
                n += 1;
            }
        }
        for i in &mut self.store.inbox {
            if i.to_session.as_deref() == Some(zombie) {
                i.to_session = Some(target.to_string());
                n += 1;
            }
        }
        self.store.sessions.retain(|s| s.id != zombie);
        self.touch();
        Ok(n)
    }

    /// Permanently removes a zombie and its links.
    pub fn purge_zombie(&mut self, zombie: &str) -> Result<(), CoreError> {
        if !self.session(zombie)?.is_zombie() {
            return Err(CoreError::Invalid { detail: "only a deleted session can be purged".into() });
        }
        for p in &mut self.store.presets {
            p.visibility.sessions.retain(|s| s != zombie);
            p.pinned_in.retain(|s| s != zombie);
        }
        self.store.inbox.retain(|i| i.to_session.as_deref() != Some(zombie));
        self.store.sessions.retain(|s| s.id != zombie);
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ history

    fn push_history(&mut self, session: &str, path: &str) {
        let limit = self.store.settings.history_limit.max(1) as usize;
        if let Ok(s) = self.session_mut(session) {
            s.history.retain(|h| h.path != path);
            s.history.insert(0, HistoryEntry { path: path.to_string(), file_name: file_name_of(path), opened_at: now_ms() });
            s.history.truncate(limit);
        }
        self.touch();
    }

    pub fn remove_history(&mut self, session: &str, path: &str) -> Result<(), CoreError> {
        self.session_mut(session)?.history.retain(|h| h.path != path);
        self.touch();
        Ok(())
    }

    pub fn clear_history(&mut self, session: &str, only_missing: bool) -> Result<(), CoreError> {
        let s = self.session_mut(session)?;
        if only_missing {
            s.history.retain(|h| Path::new(&h.path).is_file());
        } else {
            s.history.clear();
        }
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ tabs

    fn blank_tab(&mut self, name: String, bands: u32, steps: u32, mode: Mode) -> Tab {
        Tab {
            id: self.new_id(),
            name,
            curves: vec![],
            filters: vec![],
            measurement: None,
            target: None,
            compensation: None,
            bands,
            steps,
            mode,
            last_run: None,
            result_color: "#F97316".into(),
            eq_color: "#8E8E93".into(),
            show_result: true,
            show_eq: true,
            created_at: now_ms(),
        }
    }

    pub fn create_tab(&mut self, session: &str, name: Option<String>) -> Result<Id, CoreError> {
        let (b, st, m) = self.effective_defaults(session);
        let n = self.live_session_mut(session)?.tabs.len() + 1;
        let tab = self.blank_tab(clean_name(name.unwrap_or_default(), &format!("Tab {n}")), b, st, m);
        let id = tab.id.clone();
        let s = self.live_session_mut(session)?;
        s.tabs.push(tab);
        s.active_tab = Some(id.clone());
        self.touch();
        Ok(id)
    }

    pub fn duplicate_tab(&mut self, tab: &str) -> Result<Id, CoreError> {
        let (sid, mut copy) = {
            let (s, t) = self.find_tab(tab)?;
            (s.id.clone(), t.clone())
        };
        copy.id = self.new_id();
        copy.name = format!("{} (copy)", copy.name);
        copy.created_at = now_ms();
        let id = copy.id.clone();
        let s = self.live_session_mut(&sid)?;
        let pos = s.tabs.iter().position(|t| t.id == tab).map(|p| p + 1).unwrap_or(s.tabs.len());
        s.tabs.insert(pos, copy);
        s.active_tab = Some(id.clone());
        self.touch();
        Ok(id)
    }

    pub fn close_tab(&mut self, tab: &str) -> Result<(), CoreError> {
        let sid = self.find_tab(tab)?.0.id.clone();
        let s = self.session_mut(&sid)?;
        let pos = s.tabs.iter().position(|t| t.id == tab).unwrap_or(0);
        s.tabs.retain(|t| t.id != tab);
        if s.active_tab.as_deref() == Some(tab) {
            s.active_tab = s.tabs.get(pos.min(s.tabs.len().saturating_sub(1))).map(|t| t.id.clone());
        }
        self.undo.remove(tab);
        self.touch();
        Ok(())
    }

    pub fn move_tab(&mut self, tab: &str, index: u32) -> Result<(), CoreError> {
        let sid = self.find_tab(tab)?.0.id.clone();
        let s = self.session_mut(&sid)?;
        let pos = s.tabs.iter().position(|t| t.id == tab).unwrap();
        let t = s.tabs.remove(pos);
        let idx = (index as usize).min(s.tabs.len());
        s.tabs.insert(idx, t);
        self.touch();
        Ok(())
    }

    pub fn set_active_tab(&mut self, tab: &str) -> Result<(), CoreError> {
        let sid = self.find_tab(tab)?.0.id.clone();
        self.session_mut(&sid)?.active_tab = Some(tab.to_string());
        self.touch();
        Ok(())
    }

    pub fn rename_tab(&mut self, tab: &str, name: String) -> Result<(), CoreError> {
        let t = self.tab_mut(tab)?;
        t.name = clean_name(name, &t.name.clone());
        self.touch();
        Ok(())
    }

    pub fn set_tab_params(&mut self, tab: &str, bands: u32, steps: u32, mode: Mode) -> Result<(), CoreError> {
        check_bands(bands)?;
        if steps == 0 {
            return Err(CoreError::Invalid { detail: "invalid number of steps".into() });
        }
        self.checkpoint(tab, Some("params"))?;
        let t = self.tab_mut(tab)?;
        t.bands = bands;
        t.steps = steps;
        t.mode = mode;
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ undo / redo

    /// Records the tab's state before a change. Successive changes with the
    /// same key (dragging a slider…) are coalesced.
    pub fn checkpoint(&mut self, tab: &str, key: Option<&str>) -> Result<(), CoreError> {
        let snap = self.tab(tab)?.clone();
        let st = self.undo.entry(tab.to_string()).or_default();
        let now = Instant::now();
        let merge = match (key, &st.last_key, st.last_at) {
            (Some(k), Some(lk), Some(at)) => k == lk && now.duration_since(at).as_millis() < COALESCE_MS,
            _ => false,
        };
        if !merge {
            st.undo.push(snap);
            if st.undo.len() > UNDO_LIMIT {
                st.undo.remove(0);
            }
        }
        st.redo.clear();
        st.last_key = key.map(str::to_string);
        st.last_at = Some(now);
        Ok(())
    }

    pub fn can_undo(&self, tab: &str) -> (bool, bool) {
        self.undo.get(tab).map(|s| (!s.undo.is_empty(), !s.redo.is_empty())).unwrap_or((false, false))
    }

    pub fn undo(&mut self, tab: &str) -> Result<bool, CoreError> {
        let cur = self.tab(tab)?.clone();
        let Some(st) = self.undo.get_mut(tab) else { return Ok(false) };
        let Some(prev) = st.undo.pop() else { return Ok(false) };
        st.redo.push(cur);
        st.last_key = None;
        *self.tab_mut(tab)? = prev;
        self.touch();
        Ok(true)
    }

    pub fn redo(&mut self, tab: &str) -> Result<bool, CoreError> {
        let cur = self.tab(tab)?.clone();
        let Some(st) = self.undo.get_mut(tab) else { return Ok(false) };
        let Some(next) = st.redo.pop() else { return Ok(false) };
        st.undo.push(cur);
        st.last_key = None;
        *self.tab_mut(tab)? = next;
        self.touch();
        Ok(true)
    }

    // ------------------------------------------------------------ colors

    fn auto_color(&mut self, _tab: &str) -> String {
        let h = self.hue;
        self.hue = (self.hue + GOLDEN_ANGLE) % 360.0;
        oklch_to_hex(OKLCH_L, OKLCH_C, h)
    }

    pub fn random_color(&mut self) -> String {
        let h = self.rand_f() * 360.0;
        oklch_to_hex(OKLCH_L, OKLCH_C, h)
    }

    // ------------------------------------------------------------ curves

    fn add_curve(&mut self, tab: &str, label: String, data: CurveData, origin: CurveOrigin, role: Role) -> Result<Id, CoreError> {
        self.checkpoint(tab, None)?;
        let color = self.auto_color(tab);
        let id = self.new_id();
        let t = self.tab_mut(tab)?;
        // a new measurement (target, compensation) replaces the previous one
        let old = t.slot(role).and_then(|s| s.take());
        let mut color = color;
        if let Some(old) = old {
            // the replacement keeps the color of the old one
            if let Some(c) = t.curves.iter().find(|c| c.id == old) {
                color = c.color.clone();
            }
            t.curves.retain(|c| c.id != old);
        }
        t.curves.push(TabCurve { id: id.clone(), label, color, visible: true, offset_db: 0.0, data, origin });
        if let Some(slot) = t.slot(role) {
            *slot = Some(id.clone());
        }
        self.touch();
        Ok(id)
    }

    pub fn add_curve_from_file(&mut self, tab: &str, path: &str, role: Role) -> Result<Id, CoreError> {
        let sid = self.find_tab(tab)?.0.id.clone();
        let data = read_curve_file(path)?;
        self.push_history(&sid, path);
        let origin = CurveOrigin::File { path: path.to_string(), file_name: file_name_of(path) };
        self.add_curve(tab, stem_of(path), data, origin, role)
    }

    /// Default role of a preset added with one click: that of its category if
    /// the tab does not have one yet, otherwise comparison curve. The category is
    /// only used for this default: a preset can be added in any role.
    pub fn default_role_for(&self, tab: &str, category: PresetCategory) -> Role {
        match (self.tab(tab), category) {
            (Ok(t), PresetCategory::Measurement) if t.measurement.is_none() => Role::Measurement,
            (Ok(t), PresetCategory::Target) if t.target.is_none() => Role::Target,
            (Ok(t), PresetCategory::Compensation) if t.compensation.is_none() => Role::Compensation,
            _ => Role::Compare,
        }
    }

    pub fn add_curve_from_preset(&mut self, tab: &str, preset: &str, role: Option<Role>) -> Result<Id, CoreError> {
        let p = self.preset(preset)?.clone();
        let data = self.preset_data(&p)?;
        let role = role.unwrap_or_else(|| self.default_role_for(tab, p.category));
        let file_name = match &p.source {
            PresetSource::Path { file_name, .. } | PresetSource::Embedded { file_name, .. } => file_name.clone(),
        };
        if let PresetSource::Path { path, .. } = &p.source {
            let sid = self.find_tab(tab)?.0.id.clone();
            self.push_history(&sid, path);
        }
        let origin = CurveOrigin::Preset { preset_id: p.id.clone(), name: p.name.clone(), file_name };
        self.add_curve(tab, p.name.clone(), data, origin, role)
    }

    pub fn add_curve_from_inbox(&mut self, tab: &str, item: &str, role: Role) -> Result<Id, CoreError> {
        let it = self
            .store
            .inbox
            .iter()
            .find(|i| i.id == item)
            .cloned()
            .ok_or_else(|| CoreError::NotFound { what: format!("inbox item {item}") })?;
        let label = if it.note.trim().is_empty() { it.label.clone() } else { it.note.clone() };
        let origin = CurveOrigin::Inbox { note: it.note.clone(), from_session: it.from_session_name.clone() };
        self.add_curve(tab, label, it.data, origin, role)
    }

    pub fn add_curve_from_text(&mut self, tab: &str, label: String, text: &str, role: Role) -> Result<Id, CoreError> {
        let data = dsp::parse_curve_text(text).map_err(|e| CoreError::Parse { detail: e })?;
        let label = clean_name(label, "Curve");
        self.add_curve(tab, label.clone(), data, CurveOrigin::Other { label }, role)
    }

    /// Copies the current result (measurement + EQ) as a comparison curve.
    pub fn snapshot_result(&mut self, tab: &str) -> Result<Id, CoreError> {
        let data = self.curve_ref_data(tab, &CurveRef::Result)?;
        let name = self.tab(tab)?.name.clone();
        self.add_curve(tab, format!("Result — {name}"), data, CurveOrigin::Result { tab_name: name }, Role::Compare)
    }

    pub fn remove_curve(&mut self, tab: &str, curve: &str) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        let t = self.tab_mut(tab)?;
        t.curves.retain(|c| c.id != curve);
        t.clear_roles_of(curve);
        self.touch();
        Ok(())
    }

    pub fn set_curve_role(&mut self, tab: &str, curve: &str, role: Role) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        let t = self.tab_mut(tab)?;
        if !t.curves.iter().any(|c| c.id == curve) {
            return Err(CoreError::NotFound { what: format!("curve {curve}") });
        }
        t.clear_roles_of(curve);
        if let Some(slot) = t.slot(role) {
            *slot = Some(curve.to_string());
        }
        self.touch();
        Ok(())
    }

    fn curve_mut<'a>(t: &'a mut Tab, curve: &str) -> Result<&'a mut TabCurve, CoreError> {
        t.curves
            .iter_mut()
            .find(|c| c.id == curve)
            .ok_or_else(|| CoreError::NotFound { what: format!("curve {curve}") })
    }

    pub fn set_curve_visible(&mut self, tab: &str, curve: &str, visible: bool) -> Result<(), CoreError> {
        self.checkpoint(tab, Some(&format!("vis:{curve}")))?;
        let t = self.tab_mut(tab)?;
        match curve {
            RESULT_ID => t.show_result = visible,
            EQ_ID => t.show_eq = visible,
            _ => Self::curve_mut(t, curve)?.visible = visible,
        }
        self.touch();
        Ok(())
    }

    pub fn set_curve_color(&mut self, tab: &str, curve: &str, color: &str) -> Result<(), CoreError> {
        let color = normalize_hex(color).ok_or_else(|| CoreError::Invalid { detail: format!("invalid color: {color}") })?;
        self.checkpoint(tab, Some(&format!("color:{curve}")))?;
        let t = self.tab_mut(tab)?;
        match curve {
            RESULT_ID => t.result_color = color,
            EQ_ID => t.eq_color = color,
            _ => Self::curve_mut(t, curve)?.color = color,
        }
        self.touch();
        Ok(())
    }

    pub fn randomize_curve_color(&mut self, tab: &str, curve: &str) -> Result<String, CoreError> {
        let c = self.random_color();
        // no coalescing: each draw can be undone
        self.checkpoint(tab, None)?;
        let t = self.tab_mut(tab)?;
        match curve {
            RESULT_ID => t.result_color = c.clone(),
            EQ_ID => t.eq_color = c.clone(),
            _ => Self::curve_mut(t, curve)?.color = c.clone(),
        }
        self.touch();
        Ok(c)
    }

    pub fn rename_curve(&mut self, tab: &str, curve: &str, label: String) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        let c = Self::curve_mut(self.tab_mut(tab)?, curve)?;
        c.label = clean_name(label, &c.label.clone());
        self.touch();
        Ok(())
    }

    pub fn set_curve_offset(&mut self, tab: &str, curve: &str, offset: f64) -> Result<(), CoreError> {
        if !offset.is_finite() {
            return Err(CoreError::Invalid { detail: "invalid offset".into() });
        }
        self.checkpoint(tab, Some(&format!("offset:{curve}")))?;
        Self::curve_mut(self.tab_mut(tab)?, curve)?.offset_db = offset;
        self.touch();
        Ok(())
    }

    /// Aligns a curve to the target (least squares over 20 Hz – 20 kHz).
    pub fn align_curve_to_target(&mut self, tab: &str, curve: &str) -> Result<f64, CoreError> {
        let t = self.tab(tab)?;
        let target_id = t.target.clone().ok_or_else(|| CoreError::Invalid { detail: "no target in this tab".into() })?;
        let grid = dsp::log_grid(512);
        let target = t.curves.iter().find(|c| c.id == target_id).unwrap();
        let tv: Vec<f64> = target.data.resample(&grid).iter().map(|v| v + target.offset_db).collect();
        let c = t.curves.iter().find(|c| c.id == curve).ok_or_else(|| CoreError::NotFound { what: format!("curve {curve}") })?;
        let cv = if Some(curve) == t.measurement.as_deref() {
            // for the measurement, it is the corrected result (compensated + EQ) that gets aligned
            t.corrected_on(c, &t.filters, &grid, false)
        } else {
            c.data.resample(&grid)
        };
        let off = dsp::align_offset(&cv, &tv, &grid, engine::F_LO, engine::F_HI);
        self.set_curve_offset(tab, curve, off)?;
        Ok(off)
    }

    // ------------------------------------------------------------ filters

    pub fn add_filter(&mut self, tab: &str, kind: FilterKind, f0: f64, gain: f64, q: f64) -> Result<Id, CoreError> {
        check_filter(f0, gain, q)?;
        self.checkpoint(tab, None)?;
        let id = self.new_id();
        self.tab_mut(tab)?.filters.push(Filter { id: id.clone(), kind, f0, gain, q, enabled: true, origin: FilterOrigin::Manual });
        self.touch();
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_filter(
        &mut self,
        tab: &str,
        filter: &str,
        kind: FilterKind,
        f0: f64,
        gain: f64,
        q: f64,
        enabled: bool,
    ) -> Result<(), CoreError> {
        check_filter(f0, gain, q)?;
        self.checkpoint(tab, Some(&format!("filter:{filter}")))?;
        let t = self.tab_mut(tab)?;
        let f = t
            .filters
            .iter_mut()
            .find(|f| f.id == filter)
            .ok_or_else(|| CoreError::NotFound { what: format!("filter {filter}") })?;
        f.kind = kind;
        f.f0 = f0;
        f.gain = gain;
        f.q = q;
        f.enabled = enabled;
        self.touch();
        Ok(())
    }

    pub fn remove_filter(&mut self, tab: &str, filter: &str) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        self.tab_mut(tab)?.filters.retain(|f| f.id != filter);
        self.touch();
        Ok(())
    }

    /// "Freeze" an auto filter: it becomes manual and will be kept
    /// (and taken into account) on the next autoeq run.
    pub fn set_filter_origin(&mut self, tab: &str, filter: &str, origin: FilterOrigin) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        let t = self.tab_mut(tab)?;
        let f = t
            .filters
            .iter_mut()
            .find(|f| f.id == filter)
            .ok_or_else(|| CoreError::NotFound { what: format!("filter {filter}") })?;
        f.origin = origin;
        self.touch();
        Ok(())
    }

    pub fn clear_filters(&mut self, tab: &str, only_auto: bool) -> Result<(), CoreError> {
        self.checkpoint(tab, None)?;
        let t = self.tab_mut(tab)?;
        if only_auto {
            t.filters.retain(|f| f.origin != FilterOrigin::Auto);
        } else {
            t.filters.clear();
        }
        self.touch();
        Ok(())
    }

    /// Imports filters from a text; returns (filters read, lines skipped).
    pub fn import_filters(&mut self, tab: &str, text: &str, replace: bool) -> Result<(u32, Vec<String>), CoreError> {
        let parsed = formats::parse_filters(text);
        if parsed.filters.is_empty() {
            return Err(CoreError::Parse { detail: "no filter recognized in this text".into() });
        }
        for f in &parsed.filters {
            check_filter(f.f0, f.gain, f.q)?;
        }
        self.checkpoint(tab, None)?;
        let ids: Vec<Id> = parsed.filters.iter().map(|_| self.new_id()).collect();
        let t = self.tab_mut(tab)?;
        if replace {
            t.filters.clear();
        }
        for (f, id) in parsed.filters.iter().zip(ids) {
            t.filters.push(Filter {
                id,
                kind: f.kind,
                f0: f.f0,
                gain: f.gain,
                q: f.q,
                enabled: f.enabled,
                origin: FilterOrigin::Imported,
            });
        }
        self.touch();
        Ok((parsed.filters.len() as u32, parsed.skipped))
    }

    pub fn import_filters_file(&mut self, tab: &str, path: &str, replace: bool) -> Result<(u32, Vec<String>), CoreError> {
        if !Path::new(path).is_file() {
            return Err(CoreError::FileMissing { path: path.to_string() });
        }
        let text = fs::read_to_string(path).map_err(|e| CoreError::Io { detail: e.to_string() })?;
        self.import_filters(tab, &text, replace)
    }

    // ------------------------------------------------------------ computations

    /// Response of the enabled filters at frequencies `f`.
    pub fn eq_response(filters: &[Filter], f: &[f64]) -> Vec<f64> {
        let mut y = vec![0.0; f.len()];
        for fl in filters.iter().filter(|x| x.enabled) {
            add_response(fl.kind, fl.f0, fl.gain, fl.q, f, &mut y);
        }
        y
    }

    /// Exported preamp, (see `formats::export_preamp`).
    pub fn safe_preamp(filters: &[Filter]) -> f64 {
        let fl: Vec<ParsedFilter> = filters
            .iter()
            .map(|f| ParsedFilter { kind: f.kind, f0: f.f0, gain: f.gain, q: f.q, enabled: f.enabled })
            .collect();
        formats::export_preamp(&fl)
    }

    fn curve_ref_data_raw(&self, t: &Tab, r: &CurveRef, grid: &[f64]) -> Result<Vec<f64>, CoreError> {
        match r {
            CurveRef::Curve { id } => {
                let c = t.curves.iter().find(|c| &c.id == id).ok_or_else(|| CoreError::NotFound { what: format!("curve {id}") })?;
                Ok(c.data.resample(grid).iter().map(|v| v + c.offset_db).collect())
            }
            CurveRef::Eq => Ok(Self::eq_response(&t.filters, grid)),
            CurveRef::Result => {
                let m = t
                    .measurement
                    .as_ref()
                    .and_then(|id| t.curves.iter().find(|c| &c.id == id))
                    .ok_or_else(|| CoreError::Invalid { detail: "no measurement in this tab".into() })?;
                Ok(t.corrected_on(m, &t.filters, grid, true))
            }
        }
    }

    /// Data of a curve as displayed (offset included), on its own
    /// frequencies (the measurement's for the result, a dense grid for the EQ).
    pub fn curve_ref_data(&self, tab: &str, r: &CurveRef) -> Result<CurveData, CoreError> {
        let t = self.tab(tab)?;
        let freqs: Vec<f64> = match r {
            CurveRef::Curve { id } => {
                t.curves.iter().find(|c| &c.id == id).ok_or_else(|| CoreError::NotFound { what: format!("curve {id}") })?.data.freqs.clone()
            }
            CurveRef::Eq => dsp::log_grid(1024),
            CurveRef::Result => t
                .measurement
                .as_ref()
                .and_then(|id| t.curves.iter().find(|c| &c.id == id))
                .map(|c| c.data.freqs.clone())
                .ok_or_else(|| CoreError::Invalid { detail: "no measurement in this tab".into() })?,
        };
        let db = self.curve_ref_data_raw(t, r, &freqs)?;
        Ok(CurveData { freqs, db })
    }

    pub fn curve_ref_label(&self, tab: &str, r: &CurveRef) -> Result<String, CoreError> {
        let t = self.tab(tab)?;
        Ok(match r {
            CurveRef::Curve { id } => t.curves.iter().find(|c| &c.id == id).map(|c| c.label.clone()).unwrap_or_default(),
            CurveRef::Result => {
                let m = t.measurement.as_ref().and_then(|id| t.curves.iter().find(|c| &c.id == id));
                match m {
                    Some(m) => result_label(t, m),
                    None => "Result".into(),
                }
            }
            CurveRef::Eq => format!("EQ — {}", t.name),
        })
    }

    // ------------------------------------------------------------ autoeq

    /// Prepares a run: target and measurement (− compensation, + enabled
    /// manual/imported filters) resampled onto the algorithm's grid.
    pub fn prepare_run(&self, tab: &str) -> Result<RunJob, CoreError> {
        let (s, t) = self.find_tab(tab)?;
        let m = t
            .measurement
            .as_ref()
            .and_then(|id| t.curves.iter().find(|c| &c.id == id))
            .ok_or_else(|| CoreError::Invalid { detail: "choose a measurement first".into() })?;
        let tg = t
            .target
            .as_ref()
            .and_then(|id| t.curves.iter().find(|c| &c.id == id))
            .ok_or_else(|| CoreError::Invalid { detail: "choose a target first".into() })?;
        let grid = engine::grid_f64();
        let kept: Vec<Filter> = t.filters.iter().filter(|f| f.origin != FilterOrigin::Auto).cloned().collect();
        let src: Vec<f32> = t.corrected_on(m, &kept, &grid, false).iter().map(|v| *v as f32).collect();
        let dst = engine::resample_to_grid(&tg.data.freqs, &tg.data.db);
        Ok(RunJob {
            session: s.id.clone(),
            tab: tab.to_string(),
            engine: self.store.settings.engine.clone(),
            mode: t.mode,
            bands: t.bands,
            steps: t.steps,
            dst,
            src,
        })
    }

    pub fn apply_run(&mut self, job: &RunJob, out: &AutoeqOutput) -> Result<RunInfo, CoreError> {
        self.checkpoint(&job.tab, None)?;
        let ids: Vec<Id> = out.filters.iter().map(|_| self.new_id()).collect();
        let auto_align = self.store.settings.auto_align;
        let engine_label = match &job.engine {
            Engine::BuiltinOrig => "autoeq-c orig (built-in)".to_string(),
            Engine::External { path, .. } => file_name_of(path),
        };
        let info = RunInfo {
            amp: out.amp as f64,
            max: out.max as f64,
            mean: out.mean.map(|m| m as f64),
            duration_ms: out.duration_ms,
            engine: engine_label,
            bands: job.bands,
            steps: job.steps,
            mode: job.mode,
            at: now_ms(),
        };
        let t = self.tab_mut(&job.tab)?;
        t.filters.retain(|f| f.origin != FilterOrigin::Auto);
        // the auto filters go first, in the binary's order
        let auto: Vec<Filter> = out
            .filters
            .iter()
            .zip(ids)
            .map(|(f, id)| Filter {
                id,
                kind: f.kind,
                f0: f.f0 as f64,
                gain: f.gain as f64,
                q: f.q as f64,
                enabled: true,
                origin: FilterOrigin::Auto,
            })
            .collect();
        t.filters.splice(0..0, auto);
        t.last_run = Some(info.clone());

        if auto_align {
            let target_off = t.target.as_ref().and_then(|id| t.curves.iter().find(|c| &c.id == id)).map(|c| c.offset_db).unwrap_or(0.0);
            let off = match out.mean {
                // the algorithm's implicit offset: mean removed + global gain
                Some(mean) => target_off + mean as f64 + out.amp as f64,
                None => {
                    // external binary: least squares between result and target
                    let grid = engine::grid_f64();
                    let tg = t.target.as_ref().and_then(|id| t.curves.iter().find(|c| &c.id == id)).unwrap();
                    let m = t.measurement.as_ref().and_then(|id| t.curves.iter().find(|c| &c.id == id)).unwrap();
                    let res = t.corrected_on(m, &t.filters, &grid, false);
                    let tv: Vec<f64> = tg.data.resample(&grid).iter().map(|v| v + tg.offset_db).collect();
                    dsp::align_offset(&res, &tv, &grid, engine::F_LO, engine::F_HI)
                }
            };
            if let Some(mid) = t.measurement.clone() {
                if let Some(c) = t.curves.iter_mut().find(|c| c.id == mid) {
                    c.offset_db = off;
                }
            }
        }
        self.touch();
        Ok(info)
    }

    // ------------------------------------------------------------ presets

    pub fn visible_presets(&self, session: &str) -> Vec<&Preset> {
        let mut v: Vec<&Preset> = self.store.presets.iter().filter(|p| p.visibility.allows(session)).collect();
        v.sort_by(|a, b| {
            let pa = a.pinned_in.iter().any(|s| s == session);
            let pb = b.pinned_in.iter().any(|s| s == session);
            pb.cmp(&pa).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        v
    }

    pub fn preset_data(&self, p: &Preset) -> Result<CurveData, CoreError> {
        match &p.source {
            PresetSource::Path { path, .. } => read_curve_file(path),
            PresetSource::Embedded { data, .. } => Ok(data.clone()),
        }
    }

    /// Finds the presets that already match this file (same path or same content).
    pub fn find_duplicates_for_file(&self, session: &str, path: &str) -> Result<Vec<DuplicateMatch>, CoreError> {
        let data = read_curve_file(path)?;
        Ok(self.find_duplicates(session, Some(path), &data))
    }

    pub fn find_duplicates(&self, session: &str, path: Option<&str>, data: &CurveData) -> Vec<DuplicateMatch> {
        let hash = data.content_hash();
        let canon = |p: &str| fs::canonicalize(p).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string());
        let path_c = path.map(canon);
        let mut out = vec![];
        for p in &self.store.presets {
            let reason = match &p.source {
                PresetSource::Path { path: pp, .. } if path_c.as_deref() == Some(canon(pp).as_str()) => Some(DuplicateReason::SamePath),
                PresetSource::Path { path: pp, .. } => {
                    read_curve_file(pp).ok().filter(|d| d.content_hash() == hash).map(|_| DuplicateReason::SameContent)
                }
                PresetSource::Embedded { hash: h, .. } if *h == hash => Some(DuplicateReason::SameContent),
                _ => None,
            };
            if let Some(reason) = reason {
                out.push(DuplicateMatch {
                    preset_id: p.id.clone(),
                    preset_name: p.name.clone(),
                    reason,
                    visible_in_session: p.visibility.allows(session),
                });
            }
        }
        out
    }

    pub fn create_preset_from_file(
        &mut self,
        path: &str,
        name: Option<String>,
        category: PresetCategory,
        embed: bool,
        visibility: Visibility,
    ) -> Result<Id, CoreError> {
        let data = read_curve_file(path)?;
        let file_name = file_name_of(path);
        let source = if embed {
            let hash = data.content_hash();
            PresetSource::Embedded { data, file_name, hash }
        } else {
            PresetSource::Path { path: path.to_string(), file_name }
        };
        let name = clean_name(name.unwrap_or_default(), &stem_of(path));
        Ok(self.push_preset(name, category, source, visibility))
    }

    pub fn create_preset_from_curve(
        &mut self,
        tab: &str,
        r: &CurveRef,
        name: Option<String>,
        category: PresetCategory,
        visibility: Visibility,
    ) -> Result<Id, CoreError> {
        let data = self.curve_ref_data(tab, r)?;
        let label = self.curve_ref_label(tab, r)?;
        let file_name = match r {
            CurveRef::Curve { id } => match &self.tab(tab)?.curves.iter().find(|c| &c.id == id).unwrap().origin {
                CurveOrigin::File { file_name, .. } | CurveOrigin::Preset { file_name, .. } => file_name.clone(),
                _ => label.clone(),
            },
            _ => label.clone(),
        };
        let hash = data.content_hash();
        let source = PresetSource::Embedded { data, file_name, hash };
        Ok(self.push_preset(clean_name(name.unwrap_or_default(), &label), category, source, visibility))
    }

    fn push_preset(&mut self, name: String, category: PresetCategory, source: PresetSource, visibility: Visibility) -> Id {
        let id = self.new_id();
        let now = now_ms();
        self.store.presets.push(Preset { id: id.clone(), name, category, source, visibility, pinned_in: vec![], created_at: now, updated_at: now });
        self.touch();
        id
    }

    /// Updates a preset from a new file (new data or new path).
    pub fn update_preset_from_file(&mut self, id: &str, path: &str) -> Result<(), CoreError> {
        let data = read_curve_file(path)?;
        let p = self.preset_mut(id)?;
        let file_name = file_name_of(path);
        p.source = match &p.source {
            PresetSource::Embedded { .. } => {
                let hash = data.content_hash();
                PresetSource::Embedded { data, file_name, hash }
            }
            PresetSource::Path { .. } => PresetSource::Path { path: path.to_string(), file_name },
        };
        p.updated_at = now_ms();
        self.touch();
        Ok(())
    }

    /// Embeds the data of a "path" preset (the file can then go away).
    pub fn embed_preset(&mut self, id: &str) -> Result<(), CoreError> {
        let p = self.preset(id)?.clone();
        if let PresetSource::Path { path, file_name } = &p.source {
            let data = read_curve_file(path)?;
            let hash = data.content_hash();
            let pm = self.preset_mut(id)?;
            pm.source = PresetSource::Embedded { data, file_name: file_name.clone(), hash };
            pm.updated_at = now_ms();
            self.touch();
        }
        Ok(())
    }

    pub fn rename_preset(&mut self, id: &str, name: String) -> Result<(), CoreError> {
        let p = self.preset_mut(id)?;
        p.name = clean_name(name, &p.name.clone());
        p.updated_at = now_ms();
        self.touch();
        Ok(())
    }

    pub fn set_preset_category(&mut self, id: &str, c: PresetCategory) -> Result<(), CoreError> {
        self.preset_mut(id)?.category = c;
        self.touch();
        Ok(())
    }

    pub fn delete_preset(&mut self, id: &str) -> Result<(), CoreError> {
        self.preset(id)?;
        self.store.presets.retain(|p| p.id != id);
        self.touch();
        Ok(())
    }

    pub fn set_preset_visibility(&mut self, id: &str, v: Visibility) -> Result<(), CoreError> {
        let mut v = v;
        v.sessions.sort();
        v.sessions.dedup();
        self.preset_mut(id)?.visibility = v;
        self.touch();
        Ok(())
    }

    pub fn grant_preset_to_session(&mut self, id: &str, session: &str) -> Result<(), CoreError> {
        self.session(session)?;
        self.preset_mut(id)?.visibility.grant(session);
        self.touch();
        Ok(())
    }

    pub fn set_preset_pinned(&mut self, id: &str, session: &str, pinned: bool) -> Result<(), CoreError> {
        self.session(session)?;
        let p = self.preset_mut(id)?;
        p.pinned_in.retain(|s| s != session);
        if pinned {
            p.pinned_in.push(session.to_string());
        }
        self.touch();
        Ok(())
    }

    // ------------------------------------------------------------ inbox

    pub fn send_to_inbox(&mut self, tab: &str, r: &CurveRef, note: String, to: Option<Id>) -> Result<Id, CoreError> {
        if let Some(to) = &to {
            if self.session(to)?.is_zombie() {
                return Err(CoreError::Invalid { detail: "destination session has been deleted".into() });
            }
        }
        let data = self.curve_ref_data(tab, r)?;
        let label = self.curve_ref_label(tab, r)?;
        let (s, t) = self.find_tab(tab)?;
        let filters = if matches!(r, CurveRef::Result | CurveRef::Eq) { t.filters.clone() } else { vec![] };
        let (sid, sname) = (s.id.clone(), s.name.clone());
        let id = self.new_id();
        self.store.inbox.insert(
            0,
            InboxItem {
                id: id.clone(),
                note: note.trim().to_string(),
                label,
                from_session: Some(sid),
                from_session_name: sname,
                to_session: to,
                created_at: now_ms(),
                data,
                filters,
            },
        );
        self.touch();
        Ok(id)
    }

    pub fn inbox_for(&self, session: &str) -> Vec<&InboxItem> {
        self.store
            .inbox
            .iter()
            .filter(|i| i.to_session.is_none() || i.to_session.as_deref() == Some(session) || i.from_session.as_deref() == Some(session))
            .collect()
    }

    pub fn set_inbox_note(&mut self, item: &str, note: String) -> Result<(), CoreError> {
        let it = self.store.inbox.iter_mut().find(|i| i.id == item).ok_or_else(|| CoreError::NotFound { what: format!("inbox item {item}") })?;
        it.note = note.trim().to_string();
        self.touch();
        Ok(())
    }

    pub fn delete_inbox_item(&mut self, item: &str) -> Result<(), CoreError> {
        self.store.inbox.retain(|i| i.id != item);
        self.touch();
        Ok(())
    }

    /// Applies an inbox item's filters to a tab (as imported filters).
    pub fn apply_inbox_filters(&mut self, tab: &str, item: &str, replace: bool) -> Result<u32, CoreError> {
        let it = self.store.inbox.iter().find(|i| i.id == item).cloned().ok_or_else(|| CoreError::NotFound { what: format!("inbox item {item}") })?;
        if it.filters.is_empty() {
            return Err(CoreError::Invalid { detail: "this inbox item contains no filters".into() });
        }
        self.checkpoint(tab, None)?;
        let ids: Vec<Id> = it.filters.iter().map(|_| self.new_id()).collect();
        let t = self.tab_mut(tab)?;
        if replace {
            t.filters.clear();
        }
        for (f, id) in it.filters.iter().zip(ids) {
            t.filters.push(Filter { id, origin: FilterOrigin::Imported, ..f.clone() });
        }
        self.touch();
        Ok(it.filters.len() as u32)
    }

    // ------------------------------------------------------------ exports

    pub fn parsed_filters(t: &Tab) -> Vec<ParsedFilter> {
        t.filters
            .iter()
            .map(|f| ParsedFilter { kind: f.kind, f0: f.f0, gain: f.gain, q: f.q, enabled: f.enabled })
            .collect()
    }
}

fn clean_name(name: String, fallback: &str) -> String {
    let t = name.trim();
    if t.is_empty() { fallback.to_string() } else { t.to_string() }
}

fn check_bands(b: u32) -> Result<(), CoreError> {
    if b == 0 || b as usize > engine::MAX_N {
        Err(CoreError::Invalid { detail: format!("number of bands must be between 1 and {}", engine::MAX_N) })
    } else {
        Ok(())
    }
}

fn check_filter(f0: f64, gain: f64, q: f64) -> Result<(), CoreError> {
    if !(f0.is_finite() && f0 > 0.0 && f0 < engine::FS / 2.0) {
        return Err(CoreError::Invalid { detail: format!("invalid frequency: {f0}") });
    }
    if !(gain.is_finite() && gain.abs() <= 60.0) {
        return Err(CoreError::Invalid { detail: format!("invalid gain: {gain}") });
    }
    if !(q.is_finite() && q > 0.0 && q <= 100.0) {
        return Err(CoreError::Invalid { detail: format!("invalid Q: {q}") });
    }
    Ok(())
}

#[cfg(test)]
mod color_tests {
    use super::*;

    #[test]
    fn oklch_reference() {
        // oklch(0.628 0.2577 29.23) = #FF0000 (CSS Color 4 reference)
        assert_eq!(oklch_to_hex(0.627955, 0.257683, 29.2339), "#FF0000");
        assert_eq!(oklch_to_hex(1.0, 0.0, 0.0), "#FFFFFF");
    }

    #[test]
    fn golden_angle_hues() {
        let mut s = State::in_memory();
        let t = s.store.sessions[0].tabs.first().map(|t| t.id.clone()).unwrap_or_default();
        let a = s.auto_color(&t);
        let b = s.auto_color(&t);
        assert_ne!(a, b);
        assert_eq!(a.len(), 7);
    }
}

#[cfg(test)]
mod compensation_tests {
    use super::*;

    fn csv(f: impl Fn(f64) -> f64) -> String {
        dsp::log_grid(200).iter().map(|x| format!("{x},{}\n", f(*x))).collect()
    }

    #[test]
    fn compensation_is_subtracted_and_roles_are_free() {
        let mut s = State::in_memory();
        let sid = s.store.active_session.clone().unwrap();
        let tab = s.create_tab(&sid, None).unwrap();
        let m = s.add_curve_from_text(&tab, "M".into(), &csv(|f| (f / 1000.0).log10() * 6.0), Role::Measurement).unwrap();
        let tg = s.add_curve_from_text(&tab, "T".into(), &csv(|_| 0.0), Role::Target).unwrap();
        let c = s.add_curve_from_text(&tab, "C".into(), &csv(|f| (f / 1000.0).log10() * 6.0), Role::Compensation).unwrap();
        let t = s.tab(&tab).unwrap();
        assert_eq!(t.role_of(&c), Role::Compensation);
        // measurement − compensation = 0 everywhere
        let job = s.prepare_run(&tab).unwrap();
        assert!(job.src.iter().all(|v| v.abs() < 1e-3), "{:?}", &job.src[..4]);
        // the result (without filters) is the compensated measurement
        let r = s.curve_ref_data(&tab, &CurveRef::Result).unwrap();
        assert!(r.db.iter().all(|v| v.abs() < 1e-3));
        // no restriction: the target can become the measurement, the measurement the compensation
        s.set_curve_role(&tab, &tg, Role::Measurement).unwrap();
        s.set_curve_role(&tab, &m, Role::Compensation).unwrap();
        let t = s.tab(&tab).unwrap();
        assert_eq!(t.measurement.as_deref(), Some(tg.as_str()));
        assert_eq!(t.compensation.as_deref(), Some(m.as_str()));
        assert_eq!(t.role_of(&c), Role::Compare);
        assert!(t.target.is_none());
        // a preset category only sets the default role
        let p = s
            .create_preset_from_curve(&tab, &CurveRef::Curve { id: c.clone() }, None, PresetCategory::Target, Visibility::app_wide())
            .unwrap();
        let added = s.add_curve_from_preset(&tab, &p, Some(Role::Compensation)).unwrap();
        assert_eq!(s.tab(&tab).unwrap().compensation.as_deref(), Some(added.as_str()));
        assert_eq!(s.default_role_for(&tab, PresetCategory::Compensation), Role::Compare);
        s.remove_curve(&tab, &added).unwrap();
        assert!(s.tab(&tab).unwrap().compensation.is_none());
        assert_eq!(s.default_role_for(&tab, PresetCategory::Compensation), Role::Compensation);
    }
}
