//! Persisted data (state.json). Everything here survives the app being closed.

use serde::{Deserialize, Serialize};

use crate::dsp::{CurveData, FilterKind};
use crate::engine::{Engine, Mode};

pub type Id = String;

pub const STORE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Store {
    pub version: u32,
    pub settings: AppSettings,
    #[serde(default)]
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub presets: Vec<Preset>,
    #[serde(default)]
    pub inbox: Vec<InboxItem>,
    #[serde(default)]
    pub active_session: Option<Id>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum Theme {
    System,
    Light,
    Dark,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct AppSettings {
    pub theme: Theme,
    pub default_bands: u32,
    pub default_steps: u32,
    pub default_mode: Mode,
    pub engine: Engine,
    /// align the measurement to the target after each autoeq run
    pub auto_align: bool,
    pub history_limit: u32,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            theme: Theme::System,
            default_bands: 20,
            default_steps: 3000,
            // the reference web tool always runs the algorithm in "i" mode
            default_mode: Mode::InEar,
            engine: Engine::BuiltinOrig,
            auto_align: true,
            history_limit: 50,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Folder {
    pub id: Id,
    pub name: String,
    pub parent: Option<Id>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Id,
    pub name: String,
    pub folder: Option<Id>,
    pub created_at: i64,
    /// deleted session, kept as a "zombie" so its bindings can be moved over
    #[serde(default)]
    pub deleted_at: Option<i64>,
    #[serde(default)]
    pub bands: Option<u32>,
    #[serde(default)]
    pub steps: Option<u32>,
    #[serde(default)]
    pub mode: Option<Mode>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub active_tab: Option<Id>,
    #[serde(default)]
    pub history: Vec<HistoryEntry>,
}

impl Session {
    pub fn is_zombie(&self) -> bool {
        self.deleted_at.is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub path: String,
    pub file_name: String,
    pub opened_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum PresetCategory {
    Measurement,
    Target,
    /// curve subtracted from the measurement before autoeq (e.g. a coupler compensation)
    Compensation,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PresetSource {
    /// points to a file, re-read on every use
    Path { path: String, file_name: String },
    /// embedded data, with the name of the file it came from
    Embedded { data: CurveData, file_name: String, hash: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum VisibilityMode {
    /// visible everywhere except in the listed sessions (empty list = the whole app)
    AllExcept,
    /// visible only in the listed sessions
    OnlyListed,
}

#[derive(Clone, Debug, Serialize, Deserialize, uniffi::Record)]
pub struct Visibility {
    pub mode: VisibilityMode,
    pub sessions: Vec<Id>,
}

impl Visibility {
    pub fn app_wide() -> Self {
        Visibility { mode: VisibilityMode::AllExcept, sessions: vec![] }
    }
    pub fn allows(&self, session: &str) -> bool {
        let listed = self.sessions.iter().any(|s| s == session);
        match self.mode {
            VisibilityMode::AllExcept => !listed,
            VisibilityMode::OnlyListed => listed,
        }
    }
    /// makes the preset visible in `session`, touching only that session
    pub fn grant(&mut self, session: &str) {
        match self.mode {
            VisibilityMode::AllExcept => self.sessions.retain(|s| s != session),
            VisibilityMode::OnlyListed => {
                if !self.sessions.iter().any(|s| s == session) {
                    self.sessions.push(session.to_string())
                }
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Preset {
    pub id: Id,
    pub name: String,
    pub category: PresetCategory,
    pub source: PresetSource,
    pub visibility: Visibility,
    /// sessions in which the preset is pinned
    #[serde(default)]
    pub pinned_in: Vec<Id>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
pub enum FilterOrigin {
    Auto,
    Manual,
    Imported,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Filter {
    pub id: Id,
    pub kind: FilterKind,
    pub f0: f64,
    pub gain: f64,
    pub q: f64,
    pub enabled: bool,
    pub origin: FilterOrigin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum CurveOrigin {
    File { path: String, file_name: String },
    Preset { preset_id: Id, name: String, file_name: String },
    Inbox { note: String, from_session: String },
    Result { tab_name: String },
    Other { label: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabCurve {
    pub id: Id,
    pub label: String,
    pub color: String,
    pub visible: bool,
    pub offset_db: f64,
    pub data: CurveData,
    pub origin: CurveOrigin,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct RunInfo {
    pub amp: f64,
    pub max: f64,
    pub mean: Option<f64>,
    pub duration_ms: f64,
    pub engine: String,
    pub bands: u32,
    pub steps: u32,
    pub mode: Mode,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: Id,
    pub name: String,
    pub curves: Vec<TabCurve>,
    pub filters: Vec<Filter>,
    pub measurement: Option<Id>,
    pub target: Option<Id>,
    /// subtracted from the measurement before autoeq and in the displayed result
    #[serde(default)]
    pub compensation: Option<Id>,
    pub bands: u32,
    pub steps: u32,
    pub mode: Mode,
    pub last_run: Option<RunInfo>,
    pub result_color: String,
    pub eq_color: String,
    pub show_result: bool,
    pub show_eq: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InboxItem {
    pub id: Id,
    pub note: String,
    pub label: String,
    pub from_session: Option<Id>,
    pub from_session_name: String,
    /// None = visible in all sessions
    pub to_session: Option<Id>,
    pub created_at: i64,
    pub data: CurveData,
    pub filters: Vec<Filter>,
}

impl Default for Store {
    fn default() -> Self {
        Store {
            version: STORE_VERSION,
            settings: AppSettings::default(),
            folders: vec![],
            sessions: vec![],
            presets: vec![],
            inbox: vec![],
            active_session: None,
        }
    }
}
