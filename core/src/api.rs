//! Interface exposed through uniFFI: an `AppCore` object and simple "view"
//! records to display. Swift has no business logic to duplicate.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use crate::dsp::{self, FilterKind};
use crate::engine::{self, Mode};
use crate::error::CoreError;
use crate::formats;
use crate::model::*;
use crate::state::{self, CurveRef, DuplicateReason, Role, State, EQ_ID, RESULT_ID};

// ---------------------------------------------------------------- views

#[derive(Clone, Debug, uniffi::Record)]
pub struct FolderInfo {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub folder: Option<String>,
    pub is_zombie: bool,
    pub deleted_at: Option<i64>,
    /// presets and inbox items that still point to this session
    pub bindings: u32,
    pub tab_count: u32,
    pub bands: Option<u32>,
    pub steps: Option<u32>,
    pub mode: Option<Mode>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct SessionDefaults {
    pub bands: u32,
    pub steps: u32,
    pub mode: Mode,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct HistoryInfo {
    pub path: String,
    pub file_name: String,
    pub opened_at: i64,
    pub exists: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PresetInfo {
    pub id: String,
    pub name: String,
    pub category: PresetCategory,
    pub embedded: bool,
    /// path, for a "path" preset
    pub path: Option<String>,
    /// name of the file the data came from
    pub file_name: String,
    /// false if the file of a "path" preset has disappeared
    pub available: bool,
    pub pinned: bool,
    pub visible_here: bool,
    pub visibility: Visibility,
    pub point_count: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct TabSummary {
    pub id: String,
    pub name: String,
    pub has_measurement: bool,
    pub has_target: bool,
    pub filter_count: u32,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct CurveInfo {
    pub id: String,
    pub label: String,
    pub color: String,
    pub visible: bool,
    pub offset_db: f64,
    pub role: Role,
    /// where the curve comes from ("File", "Preset X", "Inbox: note"…)
    pub origin: String,
    pub file_name: Option<String>,
    pub path: Option<String>,
    pub point_count: u32,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct FilterInfo {
    pub id: String,
    pub kind: FilterKind,
    pub f0: f64,
    pub gain: f64,
    pub q: f64,
    pub enabled: bool,
    pub origin: FilterOrigin,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct TabInfo {
    pub id: String,
    pub name: String,
    pub curves: Vec<CurveInfo>,
    pub filters: Vec<FilterInfo>,
    pub measurement: Option<String>,
    pub target: Option<String>,
    /// curve subtracted from the measurement before autoeq
    pub compensation: Option<String>,
    pub bands: u32,
    pub steps: u32,
    pub mode: Mode,
    pub last_run: Option<RunInfo>,
    pub result_color: String,
    pub eq_color: String,
    pub show_result: bool,
    pub show_eq: bool,
    /// safe preamp (negative of the max gain of the active filter stack)
    pub preamp: f64,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SeriesKind {
    Measurement,
    Target,
    Compensation,
    Compare,
    Result,
    Eq,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PlotSeries {
    /// curve id, or "result" / "eq"
    pub key: String,
    pub label: String,
    pub color: String,
    pub kind: SeriesKind,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct PlotData {
    pub freqs: Vec<f64>,
    pub series: Vec<PlotSeries>,
    pub y_min: f64,
    pub y_max: f64,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct InboxInfo {
    pub id: String,
    pub note: String,
    pub label: String,
    pub from_session: Option<String>,
    pub from_session_name: String,
    pub to_session: Option<String>,
    pub to_session_name: Option<String>,
    pub created_at: i64,
    pub filter_count: u32,
    pub point_count: u32,
    /// sent from the requested session
    pub sent_from_here: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct DuplicateInfo {
    pub preset_id: String,
    pub preset_name: String,
    pub same_path: bool,
    pub visible_in_session: bool,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ImportReport {
    pub count: u32,
    pub skipped: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ExportFormat {
    /// Equalizer APO parametric text
    Parametric,
    /// GraphicEQ, for Wavelet (and Equalizer APO)
    GraphicEq,
    /// filters CSV (type, fc, gain, q, enabled)
    FiltersCsv,
}

// ---------------------------------------------------------------- object

#[derive(uniffi::Object)]
pub struct AppCore {
    state: Mutex<State>,
    last_save: Mutex<Instant>,
}

const AUTOSAVE_MS: u128 = 2000;

impl AppCore {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn read<T>(&self, f: impl FnOnce(&State) -> Result<T, CoreError>) -> Result<T, CoreError> {
        f(&self.lock())
    }

    /// State mutation, saving at most every 2 s
    /// (the UI also calls `save()` after a short pause and on close).
    fn mutate<T>(&self, f: impl FnOnce(&mut State) -> Result<T, CoreError>) -> Result<T, CoreError> {
        let mut s = self.lock();
        let r = f(&mut s)?;
        let mut last = self.last_save.lock().unwrap_or_else(|e| e.into_inner());
        if s.dirty && last.elapsed().as_millis() > AUTOSAVE_MS {
            s.save()?;
            *last = Instant::now();
        }
        Ok(r)
    }
}

fn session_info(s: &State, x: &Session) -> SessionInfo {
    SessionInfo {
        id: x.id.clone(),
        name: x.name.clone(),
        folder: x.folder.clone(),
        is_zombie: x.is_zombie(),
        deleted_at: x.deleted_at,
        bindings: s.binding_count(&x.id),
        tab_count: x.tabs.len() as u32,
        bands: x.bands,
        steps: x.steps,
        mode: x.mode,
    }
}

fn preset_info(p: &Preset, session: &str) -> PresetInfo {
    let (embedded, path, file_name, available, n) = match &p.source {
        PresetSource::Path { path, file_name } => (false, Some(path.clone()), file_name.clone(), Path::new(path).is_file(), 0),
        PresetSource::Embedded { data, file_name, .. } => (true, None, file_name.clone(), true, data.freqs.len() as u32),
    };
    PresetInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        category: p.category,
        embedded,
        path,
        file_name,
        available,
        pinned: p.pinned_in.iter().any(|s| s == session),
        visible_here: p.visibility.allows(session),
        visibility: p.visibility.clone(),
        point_count: n,
        created_at: p.created_at,
        updated_at: p.updated_at,
    }
}

fn origin_text(o: &CurveOrigin) -> (String, Option<String>, Option<String>) {
    match o {
        CurveOrigin::File { path, file_name } => ("File".into(), Some(file_name.clone()), Some(path.clone())),
        CurveOrigin::Preset { name, file_name, .. } => (format!("Preset “{name}”"), Some(file_name.clone()), None),
        CurveOrigin::Inbox { note, from_session } => {
            let n = if note.is_empty() { String::new() } else { format!(" “{note}”") };
            (format!("Inbox{n}, from {from_session}"), None, None)
        }
        CurveOrigin::Result { tab_name } => (format!("Result of “{tab_name}”"), None, None),
        CurveOrigin::Other { label } => (label.clone(), None, None),
    }
}

fn tab_info(s: &State, t: &Tab) -> TabInfo {
    let (can_undo, can_redo) = s.can_undo(&t.id);
    TabInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        curves: t
            .curves
            .iter()
            .map(|c| {
                let (origin, file_name, path) = origin_text(&c.origin);
                CurveInfo {
                    id: c.id.clone(),
                    label: c.label.clone(),
                    color: c.color.clone(),
                    visible: c.visible,
                    offset_db: c.offset_db,
                    role: t.role_of(&c.id),
                    origin,
                    file_name,
                    path,
                    point_count: c.data.freqs.len() as u32,
                }
            })
            .collect(),
        filters: t
            .filters
            .iter()
            .map(|f| FilterInfo { id: f.id.clone(), kind: f.kind, f0: f.f0, gain: f.gain, q: f.q, enabled: f.enabled, origin: f.origin })
            .collect(),
        measurement: t.measurement.clone(),
        target: t.target.clone(),
        compensation: t.compensation.clone(),
        bands: t.bands,
        steps: t.steps,
        mode: t.mode,
        last_run: t.last_run.clone(),
        result_color: t.result_color.clone(),
        eq_color: t.eq_color.clone(),
        show_result: t.show_result,
        show_eq: t.show_eq,
        preamp: State::safe_preamp(&t.filters),
        can_undo,
        can_redo,
    }
}

fn plot(t: &Tab, n: usize) -> PlotData {
    let freqs = dsp::log_grid(n.clamp(32, 4096));
    let mut series = vec![];
    for c in t.curves.iter().filter(|c| c.visible) {
        let kind = match t.role_of(&c.id) {
            Role::Measurement => SeriesKind::Measurement,
            Role::Target => SeriesKind::Target,
            Role::Compensation => SeriesKind::Compensation,
            Role::Compare => SeriesKind::Compare,
        };
        series.push(PlotSeries {
            key: c.id.clone(),
            label: c.label.clone(),
            color: c.color.clone(),
            kind,
            values: c.data.resample(&freqs).iter().map(|v| v + c.offset_db).collect(),
        });
    }
    let has_filters = t.filters.iter().any(|f| f.enabled);
    let eq = State::eq_response(&t.filters, &freqs);
    // the result is shown as soon as it differs from the measurement: active filters or compensation
    if t.show_result && (has_filters || t.compensation.is_some()) {
        if let Some(m) = t.measurement.as_ref().and_then(|id| t.curves.iter().find(|c| &c.id == id)) {
            series.push(PlotSeries {
                key: RESULT_ID.into(),
                label: state::result_label(t, m),
                color: t.result_color.clone(),
                kind: SeriesKind::Result,
                values: t.corrected_on(m, &t.filters, &freqs, true),
            });
        }
    }
    if t.show_eq && has_filters {
        series.push(PlotSeries { key: EQ_ID.into(), label: "EQ".into(), color: t.eq_color.clone(), kind: SeriesKind::Eq, values: eq });
    }
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for s in &series {
        for v in &s.values {
            lo = lo.min(*v);
            hi = hi.max(*v);
        }
    }
    if lo > hi {
        lo = -20.0;
        hi = 20.0;
    }
    let y_min = ((lo - 3.0) / 5.0).floor() * 5.0;
    let y_max = ((hi + 3.0) / 5.0).ceil() * 5.0;
    PlotData { freqs, series, y_min, y_max }
}

fn inbox_info(s: &State, i: &InboxItem, session: &str) -> InboxInfo {
    InboxInfo {
        id: i.id.clone(),
        note: i.note.clone(),
        label: i.label.clone(),
        from_session: i.from_session.clone(),
        from_session_name: i.from_session_name.clone(),
        to_session: i.to_session.clone(),
        to_session_name: i.to_session.as_ref().and_then(|t| s.session(t).ok()).map(|x| x.name.clone()),
        created_at: i.created_at,
        filter_count: i.filters.len() as u32,
        point_count: i.data.freqs.len() as u32,
        sent_from_here: i.from_session.as_deref() == Some(session),
    }
}

#[uniffi::export]
impl AppCore {
    /// Opens (or creates) the app state in `data_dir`.
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, CoreError> {
        let s = State::open(&data_dir)?;
        Ok(Arc::new(AppCore { state: Mutex::new(s), last_save: Mutex::new(Instant::now()) }))
    }

    /// In-memory state only (tests, previews).
    #[uniffi::constructor]
    pub fn in_memory() -> Arc<Self> {
        Arc::new(AppCore { state: Mutex::new(State::in_memory()), last_save: Mutex::new(Instant::now()) })
    }

    pub fn save(&self) -> Result<(), CoreError> {
        let mut s = self.lock();
        if s.dirty {
            s.save()?;
            *self.last_save.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        }
        Ok(())
    }

    pub fn version(&self) -> String {
        format!("eqing_core {} — autoeq-c orig, K={}", env!("CARGO_PKG_VERSION"), engine::c_grid_len())
    }

    // ------------------------------------------------------------ settings

    pub fn settings(&self) -> AppSettings {
        self.lock().store.settings.clone()
    }

    pub fn set_settings(&self, settings: AppSettings) -> Result<(), CoreError> {
        self.mutate(|s| s.set_settings(settings))
    }

    pub fn max_bands(&self) -> u32 {
        engine::MAX_N as u32
    }

    // ------------------------------------------------------------ folders

    pub fn folders(&self) -> Vec<FolderInfo> {
        self.lock()
            .store
            .folders
            .iter()
            .map(|f| FolderInfo { id: f.id.clone(), name: f.name.clone(), parent: f.parent.clone() })
            .collect()
    }

    pub fn create_folder(&self, name: String, parent: Option<String>) -> Result<String, CoreError> {
        self.mutate(|s| s.create_folder(name, parent))
    }

    pub fn rename_folder(&self, folder: String, name: String) -> Result<(), CoreError> {
        self.mutate(|s| s.rename_folder(&folder, name))
    }

    pub fn move_folder(&self, folder: String, parent: Option<String>) -> Result<(), CoreError> {
        self.mutate(|s| s.move_folder(&folder, parent))
    }

    pub fn delete_folder(&self, folder: String) -> Result<(), CoreError> {
        self.mutate(|s| s.delete_folder(&folder))
    }

    // ------------------------------------------------------------ sessions

    pub fn sessions(&self, include_zombies: bool) -> Vec<SessionInfo> {
        let s = self.lock();
        s.store
            .sessions
            .iter()
            .filter(|x| include_zombies || !x.is_zombie())
            .map(|x| session_info(&s, x))
            .collect()
    }

    pub fn active_session(&self) -> String {
        self.lock().store.active_session.clone().unwrap_or_default()
    }

    pub fn set_active_session(&self, session: String) -> Result<(), CoreError> {
        self.mutate(|s| s.set_active_session(&session))
    }

    pub fn create_session(&self, name: String, folder: Option<String>) -> Result<String, CoreError> {
        self.mutate(|s| s.create_session(name, folder))
    }

    pub fn rename_session(&self, session: String, name: String) -> Result<(), CoreError> {
        self.mutate(|s| s.rename_session(&session, name))
    }

    pub fn move_session(&self, session: String, folder: Option<String>) -> Result<(), CoreError> {
        self.mutate(|s| s.move_session(&session, folder))
    }

    /// The session becomes a zombie (see `rebind_zombie` and `purge_zombie`).
    pub fn delete_session(&self, session: String) -> Result<(), CoreError> {
        self.mutate(|s| s.delete_session(&session))
    }

    /// Moves the zombie's bindings over to `target`; returns the number of items changed.
    pub fn rebind_zombie(&self, zombie: String, target: String) -> Result<u32, CoreError> {
        self.mutate(|s| s.rebind_zombie(&zombie, &target))
    }

    pub fn purge_zombie(&self, zombie: String) -> Result<(), CoreError> {
        self.mutate(|s| s.purge_zombie(&zombie))
    }

    /// Session overrides (None = app default).
    pub fn set_session_defaults(
        &self,
        session: String,
        bands: Option<u32>,
        steps: Option<u32>,
        mode: Option<Mode>,
    ) -> Result<(), CoreError> {
        self.mutate(|s| s.set_session_defaults(&session, bands, steps, mode))
    }

    pub fn effective_defaults(&self, session: String) -> SessionDefaults {
        let (bands, steps, mode) = self.lock().effective_defaults(&session);
        SessionDefaults { bands, steps, mode }
    }

    // ------------------------------------------------------------ history

    pub fn history(&self, session: String) -> Vec<HistoryInfo> {
        self.read(|s| {
            Ok(s.session(&session)?
                .history
                .iter()
                .map(|h| HistoryInfo {
                    path: h.path.clone(),
                    file_name: h.file_name.clone(),
                    opened_at: h.opened_at,
                    exists: Path::new(&h.path).is_file(),
                })
                .collect())
        })
        .unwrap_or_default()
    }

    pub fn remove_history(&self, session: String, path: String) -> Result<(), CoreError> {
        self.mutate(|s| s.remove_history(&session, &path))
    }

    pub fn clear_history(&self, session: String, only_missing: bool) -> Result<(), CoreError> {
        self.mutate(|s| s.clear_history(&session, only_missing))
    }

    // ------------------------------------------------------------ tabs

    pub fn tabs(&self, session: String) -> Vec<TabSummary> {
        self.read(|s| {
            Ok(s.session(&session)?
                .tabs
                .iter()
                .map(|t| TabSummary {
                    id: t.id.clone(),
                    name: t.name.clone(),
                    has_measurement: t.measurement.is_some(),
                    has_target: t.target.is_some(),
                    filter_count: t.filters.len() as u32,
                })
                .collect())
        })
        .unwrap_or_default()
    }

    pub fn active_tab(&self, session: String) -> Option<String> {
        self.read(|s| Ok(s.session(&session)?.active_tab.clone())).ok().flatten()
    }

    pub fn set_active_tab(&self, tab: String) -> Result<(), CoreError> {
        self.mutate(|s| s.set_active_tab(&tab))
    }

    pub fn create_tab(&self, session: String, name: Option<String>) -> Result<String, CoreError> {
        self.mutate(|s| s.create_tab(&session, name))
    }

    pub fn duplicate_tab(&self, tab: String) -> Result<String, CoreError> {
        self.mutate(|s| s.duplicate_tab(&tab))
    }

    pub fn close_tab(&self, tab: String) -> Result<(), CoreError> {
        self.mutate(|s| s.close_tab(&tab))
    }

    pub fn move_tab(&self, tab: String, index: u32) -> Result<(), CoreError> {
        self.mutate(|s| s.move_tab(&tab, index))
    }

    pub fn rename_tab(&self, tab: String, name: String) -> Result<(), CoreError> {
        self.mutate(|s| s.rename_tab(&tab, name))
    }

    pub fn set_tab_params(&self, tab: String, bands: u32, steps: u32, mode: Mode) -> Result<(), CoreError> {
        self.mutate(|s| s.set_tab_params(&tab, bands, steps, mode))
    }

    pub fn tab(&self, tab: String) -> Result<TabInfo, CoreError> {
        self.read(|s| Ok(tab_info(s, s.tab(&tab)?)))
    }

    /// Visible curves of the tab on `points` log-spaced frequencies between 20 Hz and 20 kHz.
    pub fn plot(&self, tab: String, points: u32) -> Result<PlotData, CoreError> {
        self.read(|s| Ok(plot(s.tab(&tab)?, points as usize)))
    }

    pub fn undo(&self, tab: String) -> Result<bool, CoreError> {
        self.mutate(|s| s.undo(&tab))
    }

    pub fn redo(&self, tab: String) -> Result<bool, CoreError> {
        self.mutate(|s| s.redo(&tab))
    }

    // ------------------------------------------------------------ curves

    /// Opens a file in the tab (and adds it to the session history).
    pub fn add_curve_from_file(&self, tab: String, path: String, role: Role) -> Result<String, CoreError> {
        self.mutate(|s| s.add_curve_from_file(&tab, &path, role))
    }

    /// `role` absent: role inferred from the preset category.
    pub fn add_curve_from_preset(&self, tab: String, preset: String, role: Option<Role>) -> Result<String, CoreError> {
        self.mutate(|s| s.add_curve_from_preset(&tab, &preset, role))
    }

    pub fn add_curve_from_inbox(&self, tab: String, item: String, role: Role) -> Result<String, CoreError> {
        self.mutate(|s| s.add_curve_from_inbox(&tab, &item, role))
    }

    /// Curve pasted as text (CSV).
    pub fn add_curve_from_text(&self, tab: String, label: String, text: String, role: Role) -> Result<String, CoreError> {
        self.mutate(|s| s.add_curve_from_text(&tab, label, &text, role))
    }

    pub fn snapshot_result(&self, tab: String) -> Result<String, CoreError> {
        self.mutate(|s| s.snapshot_result(&tab))
    }

    pub fn remove_curve(&self, tab: String, curve: String) -> Result<(), CoreError> {
        self.mutate(|s| s.remove_curve(&tab, &curve))
    }

    pub fn set_curve_role(&self, tab: String, curve: String, role: Role) -> Result<(), CoreError> {
        self.mutate(|s| s.set_curve_role(&tab, &curve, role))
    }

    /// `curve` can also be "result" or "eq".
    pub fn set_curve_visible(&self, tab: String, curve: String, visible: bool) -> Result<(), CoreError> {
        self.mutate(|s| s.set_curve_visible(&tab, &curve, visible))
    }

    /// `#RRGGBB` color; `curve` can also be "result" or "eq".
    pub fn set_curve_color(&self, tab: String, curve: String, color: String) -> Result<(), CoreError> {
        self.mutate(|s| s.set_curve_color(&tab, &curve, &color))
    }

    pub fn randomize_curve_color(&self, tab: String, curve: String) -> Result<String, CoreError> {
        self.mutate(|s| s.randomize_curve_color(&tab, &curve))
    }

    pub fn rename_curve(&self, tab: String, curve: String, label: String) -> Result<(), CoreError> {
        self.mutate(|s| s.rename_curve(&tab, &curve, label))
    }

    pub fn set_curve_offset(&self, tab: String, curve: String, offset_db: f64) -> Result<(), CoreError> {
        self.mutate(|s| s.set_curve_offset(&tab, &curve, offset_db))
    }

    /// Aligns the curve to the target; returns the applied offset.
    pub fn align_curve_to_target(&self, tab: String, curve: String) -> Result<f64, CoreError> {
        self.mutate(|s| s.align_curve_to_target(&tab, &curve))
    }

    // ------------------------------------------------------------ filters

    pub fn add_filter(&self, tab: String, kind: FilterKind, f0: f64, gain: f64, q: f64) -> Result<String, CoreError> {
        self.mutate(|s| s.add_filter(&tab, kind, f0, gain, q))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_filter(
        &self,
        tab: String,
        filter: String,
        kind: FilterKind,
        f0: f64,
        gain: f64,
        q: f64,
        enabled: bool,
    ) -> Result<(), CoreError> {
        self.mutate(|s| s.update_filter(&tab, &filter, kind, f0, gain, q, enabled))
    }

    pub fn remove_filter(&self, tab: String, filter: String) -> Result<(), CoreError> {
        self.mutate(|s| s.remove_filter(&tab, &filter))
    }

    pub fn set_filter_origin(&self, tab: String, filter: String, origin: FilterOrigin) -> Result<(), CoreError> {
        self.mutate(|s| s.set_filter_origin(&tab, &filter, origin))
    }

    pub fn clear_filters(&self, tab: String, only_auto: bool) -> Result<(), CoreError> {
        self.mutate(|s| s.clear_filters(&tab, only_auto))
    }

    pub fn import_filters_text(&self, tab: String, text: String, replace: bool) -> Result<ImportReport, CoreError> {
        self.mutate(|s| s.import_filters(&tab, &text, replace)).map(|(count, skipped)| ImportReport { count, skipped })
    }

    pub fn import_filters_file(&self, tab: String, path: String, replace: bool) -> Result<ImportReport, CoreError> {
        self.mutate(|s| s.import_filters_file(&tab, &path, replace)).map(|(count, skipped)| ImportReport { count, skipped })
    }

    /// Runs autoeq on the tab (blocking: call it off the main thread).
    /// Auto filters are replaced; manual and imported filters are kept
    /// and taken into account in the measurement sent to the algorithm.
    pub fn run_autoeq(&self, tab: String) -> Result<RunInfo, CoreError> {
        let job = self.read(|s| s.prepare_run(&tab))?;
        let out = engine::run(&job.engine, job.mode, job.bands, job.steps, &job.dst, &job.src)
            .map_err(|detail| CoreError::Engine { detail })?;
        self.mutate(|s| s.apply_run(&job, &out))
    }

    // ------------------------------------------------------------ presets

    /// Presets visible in the session, pinned ones first.
    pub fn presets(&self, session: String) -> Vec<PresetInfo> {
        let s = self.lock();
        s.visible_presets(&session).into_iter().map(|p| preset_info(p, &session)).collect()
    }

    /// All presets (for management), with the pinned state for `session`.
    pub fn all_presets(&self, session: String) -> Vec<PresetInfo> {
        let s = self.lock();
        let mut v: Vec<PresetInfo> = s.store.presets.iter().map(|p| preset_info(p, &session)).collect();
        v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        v
    }

    pub fn find_duplicates_for_file(&self, session: String, path: String) -> Result<Vec<DuplicateInfo>, CoreError> {
        self.read(|s| {
            Ok(s.find_duplicates_for_file(&session, &path)?
                .into_iter()
                .map(|d| DuplicateInfo {
                    preset_id: d.preset_id,
                    preset_name: d.preset_name,
                    same_path: d.reason == DuplicateReason::SamePath,
                    visible_in_session: d.visible_in_session,
                })
                .collect())
        })
    }

    pub fn find_duplicates_for_curve(&self, session: String, tab: String, what: CurveRef) -> Result<Vec<DuplicateInfo>, CoreError> {
        self.read(|s| {
            let data = s.curve_ref_data(&tab, &what)?;
            Ok(s.find_duplicates(&session, None, &data)
                .into_iter()
                .map(|d| DuplicateInfo {
                    preset_id: d.preset_id,
                    preset_name: d.preset_name,
                    same_path: false,
                    visible_in_session: d.visible_in_session,
                })
                .collect())
        })
    }

    /// `embed`: embed the data (otherwise the preset keeps the file path).
    pub fn create_preset_from_file(
        &self,
        path: String,
        name: Option<String>,
        category: PresetCategory,
        embed: bool,
        visibility: Visibility,
    ) -> Result<String, CoreError> {
        self.mutate(|s| s.create_preset_from_file(&path, name, category, embed, visibility))
    }

    pub fn create_preset_from_curve(
        &self,
        tab: String,
        what: CurveRef,
        name: Option<String>,
        category: PresetCategory,
        visibility: Visibility,
    ) -> Result<String, CoreError> {
        self.mutate(|s| s.create_preset_from_curve(&tab, &what, name, category, visibility))
    }

    pub fn update_preset_from_file(&self, preset: String, path: String) -> Result<(), CoreError> {
        self.mutate(|s| s.update_preset_from_file(&preset, &path))
    }

    pub fn embed_preset(&self, preset: String) -> Result<(), CoreError> {
        self.mutate(|s| s.embed_preset(&preset))
    }

    pub fn rename_preset(&self, preset: String, name: String) -> Result<(), CoreError> {
        self.mutate(|s| s.rename_preset(&preset, name))
    }

    pub fn set_preset_category(&self, preset: String, category: PresetCategory) -> Result<(), CoreError> {
        self.mutate(|s| s.set_preset_category(&preset, category))
    }

    pub fn delete_preset(&self, preset: String) -> Result<(), CoreError> {
        self.mutate(|s| s.delete_preset(&preset))
    }

    pub fn set_preset_visibility(&self, preset: String, visibility: Visibility) -> Result<(), CoreError> {
        self.mutate(|s| s.set_preset_visibility(&preset, visibility))
    }

    /// Makes an existing preset visible in `session` (instead of creating a duplicate).
    pub fn grant_preset_to_session(&self, preset: String, session: String) -> Result<(), CoreError> {
        self.mutate(|s| s.grant_preset_to_session(&preset, &session))
    }

    pub fn set_preset_pinned(&self, preset: String, session: String, pinned: bool) -> Result<(), CoreError> {
        self.mutate(|s| s.set_preset_pinned(&preset, &session, pinned))
    }

    // ------------------------------------------------------------ inbox

    pub fn inbox(&self, session: String) -> Vec<InboxInfo> {
        let s = self.lock();
        s.inbox_for(&session).into_iter().map(|i| inbox_info(&s, i, &session)).collect()
    }

    /// Drops a curve (or the result, or the EQ) into the inbox; `to_session` absent = all sessions.
    pub fn send_to_inbox(&self, tab: String, what: CurveRef, note: String, to_session: Option<String>) -> Result<String, CoreError> {
        self.mutate(|s| s.send_to_inbox(&tab, &what, note, to_session))
    }

    pub fn set_inbox_note(&self, item: String, note: String) -> Result<(), CoreError> {
        self.mutate(|s| s.set_inbox_note(&item, note))
    }

    pub fn delete_inbox_item(&self, item: String) -> Result<(), CoreError> {
        self.mutate(|s| s.delete_inbox_item(&item))
    }

    pub fn apply_inbox_filters(&self, tab: String, item: String, replace: bool) -> Result<u32, CoreError> {
        self.mutate(|s| s.apply_inbox_filters(&tab, &item, replace))
    }

    // ------------------------------------------------------------ exports

    /// `frequency,raw` CSV of a curve as displayed (offset included).
    pub fn export_curve_csv(&self, tab: String, what: CurveRef) -> Result<String, CoreError> {
        self.read(|s| Ok(s.curve_ref_data(&tab, &what)?.to_csv(0.0)))
    }

    pub fn export_preset_csv(&self, preset: String) -> Result<String, CoreError> {
        self.read(|s| Ok(s.preset_data(s.preset(&preset)?)?.to_csv(0.0)))
    }

    pub fn export_inbox_csv(&self, item: String) -> Result<String, CoreError> {
        self.read(|s| {
            let it = s
                .store
                .inbox
                .iter()
                .find(|i| i.id == item)
                .ok_or_else(|| CoreError::NotFound { what: format!("inbox item {item}") })?;
            Ok(it.data.to_csv(0.0))
        })
    }

    pub fn export_eq(&self, tab: String, format: ExportFormat) -> Result<String, CoreError> {
        self.read(|s| {
            let t = s.tab(&tab)?;
            let fl = State::parsed_filters(t);
            let pre = formats::export_preamp(&fl);
            Ok(match format {
                ExportFormat::Parametric => formats::apo_text(&fl, pre),
                ExportFormat::GraphicEq => formats::graphic_eq(&fl),
                ExportFormat::FiltersCsv => formats::filters_csv(&fl, pre),
            })
        })
    }

    /// Reads a curve file without adding it anywhere (preview, validation).
    pub fn probe_curve_file(&self, path: String) -> Result<u32, CoreError> {
        Ok(state::read_curve_file(&path)?.freqs.len() as u32)
    }
}
