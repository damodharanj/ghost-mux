//! Dashboard layout persistence.
//!
//! Saves / loads the full dashboard configuration (layout tree, panel tabs,
//! active dashboard, and the `next_id` counter) to/from a YAML file named
//! `dashboard_state.yaml` in the same directory as `settings.yaml`.

use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::{fs, path::{Path, PathBuf}};

use crate::layout::{PanelContent, PanelLayout};

// ---------------------------------------------------------------------------
// Serialisable mirror types
// ---------------------------------------------------------------------------

/// Serialisable mirror of [`PanelContent`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SerPanelContent {
    Terminal,
    Editor {
        path: std::path::PathBuf,
        is_diff: bool,
        status: Option<String>,
    },
    FileExplorer,
    Git,
    Browser {
        url: String,
    },
    Diagnostics,
}

impl From<PanelContent> for SerPanelContent {
    fn from(c: PanelContent) -> Self {
        match c {
            PanelContent::Terminal => SerPanelContent::Terminal,
            PanelContent::FileExplorer => SerPanelContent::FileExplorer,
            PanelContent::Git => SerPanelContent::Git,
            PanelContent::Browser { url } => SerPanelContent::Browser { url },
            PanelContent::Editor { path, is_diff, status } => SerPanelContent::Editor { path, is_diff, status },
            PanelContent::Diagnostics => SerPanelContent::Diagnostics,
        }
    }
}

impl From<SerPanelContent> for PanelContent {
    fn from(c: SerPanelContent) -> Self {
        match c {
            SerPanelContent::Terminal => PanelContent::Terminal,
            SerPanelContent::Editor { path, is_diff, status } => PanelContent::Editor { path, is_diff, status },
            SerPanelContent::FileExplorer => PanelContent::FileExplorer,
            SerPanelContent::Git => PanelContent::Git,
            SerPanelContent::Browser { url } => PanelContent::Browser { url },
            SerPanelContent::Diagnostics => PanelContent::Diagnostics,
        }
    }
}

/// Serialisable mirror of [`PanelLayout`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub enum SerPanelLayout {
    Leaf(usize),
    HSplit {
        left: Box<SerPanelLayout>,
        right: Box<SerPanelLayout>,
        id: usize,
    },
    VSplit {
        top: Box<SerPanelLayout>,
        bot: Box<SerPanelLayout>,
        id: usize,
    },
}

impl From<PanelLayout> for SerPanelLayout {
    fn from(l: PanelLayout) -> Self {
        match l {
            PanelLayout::Leaf(id) => SerPanelLayout::Leaf(id),
            PanelLayout::HSplit { left, right, id } => SerPanelLayout::HSplit {
                left: Box::new((*left).into()),
                right: Box::new((*right).into()),
                id,
            },
            PanelLayout::VSplit { top, bot, id } => SerPanelLayout::VSplit {
                top: Box::new((*top).into()),
                bot: Box::new((*bot).into()),
                id,
            },
        }
    }
}

impl From<SerPanelLayout> for PanelLayout {
    fn from(l: SerPanelLayout) -> Self {
        match l {
            SerPanelLayout::Leaf(id) => PanelLayout::Leaf(id),
            SerPanelLayout::HSplit { left, right, id } => PanelLayout::HSplit {
                left: Box::new((*left).into()),
                right: Box::new((*right).into()),
                id,
            },
            SerPanelLayout::VSplit { top, bot, id } => PanelLayout::VSplit {
                top: Box::new((*top).into()),
                bot: Box::new((*bot).into()),
                id,
            },
        }
    }
}

/// One tab inside a panel.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SerPanelTab {
    pub id: usize,
    pub title: String,
    pub content: SerPanelContent,
}

/// All tabs for a panel, plus which is active.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SerPanelTabs {
    pub tabs: Vec<SerPanelTab>,
    pub active_tab: usize,
}

/// Panel-id → tabs mapping for a single dashboard.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SerPanelEntry {
    pub panel_id: usize,
    pub tabs: SerPanelTabs,
}

/// Full state of one dashboard.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SerDashboard {
    pub id: usize,
    pub title: String,
    pub layout: SerPanelLayout,
    pub panels: Vec<SerPanelEntry>,
    /// Per-split panel size ratios.  Key = split-node ID, value = ordered
    /// panel ratios (one per child, summing to ~1.0).  Missing entries default
    /// to an even split on restore.
    #[serde(default)]
    pub split_size_ratios: std::collections::HashMap<usize, Vec<f32>>,
    #[serde(default)]
    pub current_dir: Option<PathBuf>,
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub is_local: Option<bool>,
}

/// Root document written to `dashboard_state.yaml`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DashboardPersistedState {
    /// Monotonically-increasing ID counter — must be restored so new IDs
    /// never collide with persisted ones.
    pub next_id: usize,
    pub active_dashboard_id: usize,
    /// Ordered list of dashboard IDs (defines sidebar order).
    pub dashboard_order: Vec<usize>,
    pub dashboards: Vec<SerDashboard>,
    #[serde(default)]
    pub remote_workspace_root: Option<std::path::PathBuf>,
}

// ---------------------------------------------------------------------------
// Load / save helpers
// ---------------------------------------------------------------------------

impl DashboardPersistedState {
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read dashboard state at {}", path.display()))?;
        let state: DashboardPersistedState =
            serde_yaml::from_str(&raw).context("failed to parse dashboard_state.yaml")?;
        Ok(state)
    }

    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        let raw =
            serde_yaml::to_string(self).context("failed to serialise dashboard_state.yaml")?;
        fs::write(path, raw)
            .with_context(|| format!("failed to write dashboard state at {}", path.display()))?;
        Ok(())
    }
}
