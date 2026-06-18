use gpui::*;
use gpui::prelude::FluentBuilder;
use gpui::{InteractiveElement, ParentElement, Styled, StatefulInteractiveElement};
use gpui_component::{
    input::{Input, InputState},
    resizable::{h_resizable, resizable_panel, v_resizable, ResizableState, ResizablePanelEvent},
    ActiveTheme, *,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::layout::{PanelContent, PanelLayout, SplitDir};
use crate::persist::{
    DashboardPersistedState, SerDashboard, SerPanelContent, SerPanelEntry, SerPanelLayout,
    SerPanelTab, SerPanelTabs,
};
use crate::settings::{AppSettings, LayoutSettings, TerminalSettings};
use crate::terminal::{TerminalModel, TerminalShiftTab, TerminalTab};

#[derive(Clone, Debug)]
pub struct PanelTab {
    pub id: usize,
    pub title: String,
    pub content: PanelContent,
}

#[derive(Clone, Debug)]
pub struct PanelTabs {
    pub tabs: Vec<PanelTab>,
    pub active_tab: usize,
}

#[derive(Clone, Debug)]
pub struct DashboardState {
    pub id: usize,
    pub title: String,
    pub layout: PanelLayout,
    pub panel_tabs: HashMap<usize, PanelTabs>,
    pub current_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct GitDiffFile {
    pub status: String,
    pub path: String,
}

#[derive(Clone, Debug)]
pub struct GitDiffState {
    pub branch: String,
    pub files: Vec<GitDiffFile>,
    pub diff: String,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffLineClass {
    Unchanged,
    Addition,
    Deletion,
    Empty,
    Header,
}

#[derive(Clone, Debug)]
pub struct SideBySideLine {
    pub left_line_num: Option<usize>,
    pub left_text: String,
    pub left_class: DiffLineClass,
    pub right_line_num: Option<usize>,
    pub right_text: String,
    pub right_class: DiffLineClass,
}

#[derive(Clone)]
pub struct ModalEditorState {
    pub path: PathBuf,
    pub editor: Entity<InputState>,
    pub is_diff: bool,
    pub side_by_side: bool,
    pub scroll_handle: UniformListScrollHandle,
}

impl std::fmt::Debug for ModalEditorState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModalEditorState")
            .field("path", &self.path)
            .field("is_diff", &self.is_diff)
            .field("side_by_side", &self.side_by_side)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct GitTreeNode {
    name: String,
    path: PathBuf,
    is_dir: bool,
    depth: usize,
    is_expanded: bool,
    status: Option<String>,
}

struct GitTreeBuilderNode {
    name: String,
    relative_path: PathBuf,
    is_dir: bool,
    status: Option<String>,
    children: std::collections::BTreeMap<String, GitTreeBuilderNode>,
}

fn flatten_git_tree(
    builder_node: &GitTreeBuilderNode,
    depth: usize,
    tab_collapsed: &std::collections::HashSet<PathBuf>,
    root_path: &std::path::Path,
    nodes: &mut Vec<GitTreeNode>,
) {
    let mut sorted_children: Vec<&GitTreeBuilderNode> = builder_node.children.values().collect();
    sorted_children.sort_by(|a, b| {
        match (a.is_dir, b.is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        }
    });

    for child in sorted_children {
        let child_abs_path = root_path.join(&child.relative_path);
        let is_expanded = !tab_collapsed.contains(&child_abs_path);
        
        nodes.push(GitTreeNode {
            name: child.name.clone(),
            path: child_abs_path.clone(),
            is_dir: child.is_dir,
            depth,
            is_expanded,
            status: child.status.clone(),
        });

        if child.is_dir && is_expanded {
            flatten_git_tree(child, depth + 1, tab_collapsed, root_path, nodes);
        }
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TerminalMemoryStat {
    pid: u32,
    rss_kb: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
struct MemorySnapshot {
    app_rss_kb: u64,
    shells_rss_kb: u64,
}

pub struct BrowserState {
    pub url_editor: Entity<InputState>,
    pub handle: Option<std::rc::Rc<crate::browser::WebViewHandle>>,
}

pub struct DashboardView {
    pub dashboards: HashMap<usize, DashboardState>,
    pub dashboard_order: Vec<usize>,
    pub active_dashboard_id: usize,
    pub terminals: HashMap<usize, Entity<TerminalModel>>,
    pub editors: HashMap<usize, Entity<InputState>>,
    pub browsers: HashMap<usize, BrowserState>,
    /// Externally-owned ResizableState entities, keyed by split-node ID.
    /// Keeping them here (not inside the window keyed state) ensures that
    /// panel sizes survive dashboard switching.
    resizable_states: HashMap<usize, Entity<ResizableState>>,
    resizable_subscriptions: HashMap<usize, Subscription>,
    /// Saved size ratios for split nodes that have not yet been rendered.
    /// On the first render of a split, these ratios seed the ResizableState
    /// via `ResizableState::new_with_ratios` so the user's saved layout is
    /// restored without a visible 50/50 → correct-split flash.
    pending_split_ratios: HashMap<usize, Vec<f32>>,
    pub settings: AppSettings,
    pub show_settings_panel: bool,
    pub show_sidebar: bool,
    pub show_memory_stats: bool,
    pub settings_status: Option<String>,
    pub settings_path: PathBuf,
    /// Path where dashboard layout is persisted (alongside settings.yaml).
    pub persist_path: PathBuf,
    terminal_memory: HashMap<usize, TerminalMemoryStat>,
    memory_snapshot: MemorySnapshot,
    pub terminal_cwds: HashMap<usize, PathBuf>,
    pub expanded_paths: HashMap<usize, std::collections::HashSet<PathBuf>>,
    pub git_diffs: HashMap<usize, GitDiffState>,
    pub git_tree_view: HashMap<usize, bool>,
    pub git_diff_side_by_side: HashMap<usize, bool>,
    pub git_diff_wrap: HashMap<usize, bool>,
    pub git_collapsed_paths: HashMap<usize, std::collections::HashSet<PathBuf>>,
    pub git_diff_scroll_handles: std::cell::RefCell<HashMap<usize, UniformListScrollHandle>>,
    pub git_diff_div_scroll_handles: std::cell::RefCell<HashMap<usize, gpui::ScrollHandle>>,
    pub next_id: usize,
    pub modal_editor: Option<ModalEditorState>,
    pub editor_panels: std::collections::HashSet<usize>,
    pub panel_focus_handles: HashMap<usize, FocusHandle>,
    pub open_menu: Option<(usize, usize)>, // (panel_id, tab_idx)
}

#[derive(Clone, Copy, Debug)]
enum SettingsNumberField {
    ThemeFontSize,
    ThemeMonoFontSize,
    ThemeRadius,
    ThemeRadiusLg,
    SidebarWidth,
    SidebarMinWidth,
    SidebarMaxWidth,
    PanelHeaderHeight,
    PanelTabHeight,
    IconButtonHeight,
    TerminalFontSize,
    TerminalLineHeight,
    TerminalCharWidth,
}

impl DashboardView {
    pub fn new(window: &mut Window, settings: AppSettings, cx: &mut Context<Self>) -> Self {
        let persist_path = PathBuf::from("dashboard_state.yaml");
        let mut view = Self {
            dashboards: HashMap::new(),
            dashboard_order: vec![],
            active_dashboard_id: 0,
            terminals: HashMap::new(),
            editors: HashMap::new(),
            browsers: HashMap::new(),
            resizable_states: HashMap::new(),
            pending_split_ratios: HashMap::new(),
            resizable_subscriptions: HashMap::new(),
            settings,
            show_settings_panel: false,
            show_sidebar: true,
            show_memory_stats: false,
            settings_status: None,
            settings_path: PathBuf::from("settings.yaml"),
            persist_path: persist_path.clone(),
            terminal_memory: HashMap::new(),
            memory_snapshot: MemorySnapshot::default(),
            terminal_cwds: HashMap::new(),
            expanded_paths: HashMap::new(),
            git_diffs: HashMap::new(),
            git_tree_view: HashMap::new(),
            git_diff_side_by_side: HashMap::new(),
            git_diff_wrap: HashMap::new(),
            git_collapsed_paths: HashMap::new(),
            git_diff_scroll_handles: std::cell::RefCell::new(HashMap::new()),
            git_diff_div_scroll_handles: std::cell::RefCell::new(HashMap::new()),
            next_id: 0,
            modal_editor: None,
            editor_panels: std::collections::HashSet::new(),
            panel_focus_handles: HashMap::new(),
            open_menu: None,
        };

        // Try to restore previously-saved layout. Fall back to a fresh dashboard.
        let restored = if persist_path.exists() {
            match DashboardPersistedState::load_from_file(&persist_path) {
                Ok(state) => {
                    view.restore_from_persisted(state, window, cx);
                    true
                }
                Err(err) => {
                    eprintln!("Could not restore dashboard state, starting fresh: {err:#}");
                    false
                }
            }
        } else {
            false
        };

        if !restored {
            let first_dashboard = view.create_dashboard("Dashboard 1".to_string(), window, cx);
            view.active_dashboard_id = first_dashboard;
        }

        view.refresh_terminal_memory(cx);
        cx.spawn(async move |entity, cx| loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            entity
                .update(cx, |this, cx| this.refresh_terminal_memory(cx))
                .ok();
        })
        .detach();
        view
    }

    // -----------------------------------------------------------------------
    // Persistence helpers
    // -----------------------------------------------------------------------

    /// Serialize current dashboard layout to `dashboard_state.yaml`.
    /// Errors are printed to stderr and silently ignored so they never
    /// interrupt normal operation.
    fn save_dashboard_state(&self, cx: &App) {
        let dashboards: Vec<SerDashboard> = self
            .dashboard_order
            .iter()
            .filter_map(|id| self.dashboards.get(id))
            .map(|d| {
                let panels: Vec<SerPanelEntry> = d
                    .panel_tabs
                    .iter()
                    .map(|(panel_id, panel_tabs)| SerPanelEntry {
                        panel_id: *panel_id,
                        tabs: SerPanelTabs {
                            active_tab: panel_tabs.active_tab,
                            tabs: panel_tabs
                                .tabs
                                .iter()
                                .map(|t| SerPanelTab {
                                    id: t.id,
                                    title: t.title.clone(),
                                    content: SerPanelContent::from(t.content.clone()),
                                })
                                .collect(),
                        },
                    })
                    .collect();

                // Collect size ratios for all known split nodes in this dashboard.
                let mut split_size_ratios: std::collections::HashMap<usize, Vec<f32>> =
                    std::collections::HashMap::new();
                for split_id in d.layout.collect_split_ids() {
                    if let Some(state_entity) = self.resizable_states.get(&split_id) {
                        let state = state_entity.read(cx);
                        let sizes = state.sizes();
                        let total: f32 = sizes.iter().map(|s| s.as_f32()).sum();
                        if total > 0.0 && !sizes.is_empty() {
                            let ratios: Vec<f32> =
                                sizes.iter().map(|s| s.as_f32() / total).collect();
                            split_size_ratios.insert(split_id, ratios);
                        }
                    }
                }

                SerDashboard {
                    id: d.id,
                    title: d.title.clone(),
                    layout: SerPanelLayout::from(d.layout.clone()),
                    panels,
                    split_size_ratios,
                    current_dir: Some(d.current_dir.clone()),
                }
            })
            .collect();

        let state = DashboardPersistedState {
            next_id: self.next_id,
            active_dashboard_id: self.active_dashboard_id,
            dashboard_order: self.dashboard_order.clone(),
            dashboards,
        };

        if let Err(err) = state.save_to_file(&self.persist_path) {
            eprintln!("Failed to save dashboard state: {err:#}");
        }
    }

    // Helper called by every mutation that needs to persist state.
    fn persist(&self, cx: &App) {
        self.save_dashboard_state(cx);
    }

    /// Rebuild in-memory state from a previously-saved [`DashboardPersistedState`].
    ///
    /// Terminal processes are always re-spawned fresh (we cannot serialise a
    /// running PTY), so the layout structure is restored but new terminal
    /// entities are created for each tab.
    fn restore_from_persisted(
        &mut self,
        state: DashboardPersistedState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.next_id = state.next_id;
        self.active_dashboard_id = state.active_dashboard_id;
        self.dashboard_order = state.dashboard_order;

        for ser_dashboard in state.dashboards {
            let layout: PanelLayout = ser_dashboard.layout.into();
            let current_dir = ser_dashboard.current_dir.clone().unwrap_or_else(|| {
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
            });

            let mut panel_tabs: HashMap<usize, PanelTabs> = HashMap::new();
            for entry in ser_dashboard.panels {
                let tabs: Vec<PanelTab> = entry
                    .tabs
                    .tabs
                    .into_iter()
                    .map(|t| {
                        let content: PanelContent = t.content.clone().into();
                        // Always create a fresh content entity — PTYs cannot
                        // be serialised.
                        self.ensure_content_entity(t.id, content.clone(), Some(current_dir.clone()), window, cx);
                        PanelTab {
                            id: t.id,
                            title: if t.title.starts_with("Tab ") {
                                content_title(&content)
                            } else {
                                t.title
                            },
                            content,
                        }
                    })
                    .collect();

                let active_tab = entry.tabs.active_tab.min(tabs.len().saturating_sub(1));
                panel_tabs.insert(
                    entry.panel_id,
                    PanelTabs { tabs, active_tab },
                );
            }

            // Load saved split-size ratios so the first render can pre-seed
            // ResizableState via new_with_ratios.
            for (split_id, ratios) in ser_dashboard.split_size_ratios {
                self.pending_split_ratios.insert(split_id, ratios);
            }

            self.dashboards.insert(
                ser_dashboard.id,
                DashboardState {
                    id: ser_dashboard.id,
                    title: ser_dashboard.title,
                    layout,
                    panel_tabs,
                    current_dir,
                },
            );
        }

        // Validate active_dashboard_id in case the state file is stale.
        if !self.dashboards.contains_key(&self.active_dashboard_id) {
            if let Some(first) = self.dashboard_order.first().copied() {
                self.active_dashboard_id = first;
            }
        }
    }

    fn create_dashboard(
        &mut self,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> usize {
        let dashboard_id = self.next_id;
        let panel_id = self.next_id + 1;
        let tab_id = self.next_id + 2;
        self.next_id += 3;

        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        self.ensure_content_entity(tab_id, PanelContent::Terminal, Some(current_dir.clone()), window, cx);

        let mut panel_tabs = HashMap::new();
        panel_tabs.insert(
            panel_id,
            PanelTabs {
                tabs: vec![PanelTab {
                    id: tab_id,
                    title: "terminal".to_string(),
                    content: PanelContent::Terminal,
                }],
                active_tab: 0,
            },
        );

        self.dashboards.insert(
            dashboard_id,
            DashboardState {
                id: dashboard_id,
                title,
                layout: PanelLayout::Leaf(panel_id),
                panel_tabs,
                current_dir,
            },
        );
        self.dashboard_order.push(dashboard_id);

        dashboard_id
    }

    fn active_dashboard(&self) -> Option<&DashboardState> {
        self.dashboards.get(&self.active_dashboard_id)
    }

    pub fn add_dashboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let title = format!("Dashboard {}", self.dashboard_order.len() + 1);
        let new_id = self.create_dashboard(title, window, cx);
        self.active_dashboard_id = new_id;
        self.persist(cx);
        cx.notify();
    }

    pub fn switch_dashboard(&mut self, dashboard_id: usize, cx: &mut Context<Self>) {
        self.open_menu = None;
        if dashboard_id != self.active_dashboard_id && self.dashboards.contains_key(&dashboard_id) {
            self.active_dashboard_id = dashboard_id;

            // Clear job_done and needs_attention for all terminals in the newly active dashboard
            if let Some(dashboard) = self.dashboards.get(&dashboard_id) {
                for panel_tabs in dashboard.panel_tabs.values() {
                    for tab in &panel_tabs.tabs {
                        if let Some(terminal) = self.terminals.get(&tab.id) {
                            terminal.update(cx, |m, _| {
                                m.job_done = false;
                                m.needs_attention = false;
                            });
                        }
                    }
                }
            }

            self.persist(cx);
            cx.notify();
        }
    }

    pub fn remove_dashboard(&mut self, dashboard_id: usize, cx: &mut Context<Self>) {
        if self.dashboard_order.len() <= 1 || !self.dashboards.contains_key(&dashboard_id) {
            return;
        }

        if let Some(removed) = self.dashboards.remove(&dashboard_id) {
            // Clean up resizable states belonging to this dashboard's splits.
            for split_id in removed.layout.collect_split_ids() {
                self.resizable_states.remove(&split_id);
                self.resizable_subscriptions.remove(&split_id);
            }
            for panel in removed.panel_tabs.values() {
                for tab in &panel.tabs {
                    self.terminals.remove(&tab.id);
                    self.editors.remove(&tab.id);
                }
            }
        }

        self.dashboard_order.retain(|id| *id != dashboard_id);
        if self.active_dashboard_id == dashboard_id {
            if let Some(first) = self.dashboard_order.first().copied() {
                self.active_dashboard_id = first;
            }
        }

        self.persist(cx);
        cx.notify();
    }

    fn toggle_settings_panel(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.show_settings_panel = !self.show_settings_panel;
        cx.notify();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.show_sidebar = !self.show_sidebar;
        cx.notify();
    }

    fn toggle_memory_stats(&mut self, cx: &mut Context<Self>) {
        self.open_menu = None;
        self.show_memory_stats = !self.show_memory_stats;
        cx.notify();
    }

    fn adjust_settings_number(
        &mut self,
        field: SettingsNumberField,
        delta: f32,
        cx: &mut Context<Self>,
    ) {
        match field {
            SettingsNumberField::ThemeFontSize => {
                self.settings.theme.font_size = (self.settings.theme.font_size + delta).max(8.0);
            }
            SettingsNumberField::ThemeMonoFontSize => {
                self.settings.theme.mono_font_size =
                    (self.settings.theme.mono_font_size + delta).max(8.0);
            }
            SettingsNumberField::ThemeRadius => {
                self.settings.theme.radius = (self.settings.theme.radius + delta).max(0.0);
            }
            SettingsNumberField::ThemeRadiusLg => {
                self.settings.theme.radius_lg = (self.settings.theme.radius_lg + delta).max(0.0);
            }
            SettingsNumberField::SidebarWidth => {
                self.settings.layout.sidebar_width =
                    (self.settings.layout.sidebar_width + delta).max(80.0);
            }
            SettingsNumberField::SidebarMinWidth => {
                self.settings.layout.sidebar_min_width =
                    (self.settings.layout.sidebar_min_width + delta).max(80.0);
            }
            SettingsNumberField::SidebarMaxWidth => {
                self.settings.layout.sidebar_max_width =
                    (self.settings.layout.sidebar_max_width + delta).max(80.0);
            }
            SettingsNumberField::PanelHeaderHeight => {
                self.settings.layout.panel_header_height =
                    (self.settings.layout.panel_header_height + delta).max(16.0);
            }
            SettingsNumberField::PanelTabHeight => {
                self.settings.layout.panel_tab_height =
                    (self.settings.layout.panel_tab_height + delta).max(16.0);
            }
            SettingsNumberField::IconButtonHeight => {
                self.settings.layout.icon_button_height =
                    (self.settings.layout.icon_button_height + delta).max(14.0);
            }
            SettingsNumberField::TerminalFontSize => {
                self.settings.terminal.font_size =
                    (self.settings.terminal.font_size + delta).max(8.0);
            }
            SettingsNumberField::TerminalLineHeight => {
                self.settings.terminal.line_height =
                    (self.settings.terminal.line_height + delta).max(10.0);
            }
            SettingsNumberField::TerminalCharWidth => {
                self.settings.terminal.char_width =
                    (self.settings.terminal.char_width + delta).max(4.0);
            }
        }
        self.normalize_layout_sizes();
        self.apply_theme_settings(cx);
        cx.notify();
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        match self.settings.save_to_file(&self.settings_path) {
            Ok(()) => {
                self.settings_status = Some("Saved settings.yaml".to_string());
            }
            Err(err) => {
                self.settings_status = Some(format!("Failed to save settings.yaml: {err:#}"));
            }
        }
        cx.notify();
    }

    fn normalize_layout_sizes(&mut self) {
        let layout = &mut self.settings.layout;
        if layout.sidebar_min_width > layout.sidebar_max_width {
            layout.sidebar_max_width = layout.sidebar_min_width;
        }
        if layout.sidebar_width < layout.sidebar_min_width {
            layout.sidebar_width = layout.sidebar_min_width;
        }
        if layout.sidebar_width > layout.sidebar_max_width {
            layout.sidebar_width = layout.sidebar_max_width;
        }
    }

    fn apply_theme_settings(&mut self, cx: &mut Context<Self>) {
        let theme = Theme::global_mut(cx);
        theme.font_family = self.settings.theme.font_family.clone().into();
        theme.font_size = px(self.settings.theme.font_size);
        theme.mono_font_family = self.settings.theme.mono_font_family.clone().into();
        theme.mono_font_size = px(self.settings.theme.mono_font_size);
        theme.radius = px(self.settings.theme.radius);
        theme.radius_lg = px(self.settings.theme.radius_lg);
    }

    fn refresh_terminal_memory(&mut self, cx: &mut Context<Self>) {
        let mut updated = HashMap::new();
        let mut cwds = HashMap::new();
        for (tab_id, terminal) in &self.terminals {
            if let Some(pid) = terminal.read(cx).shell_pid() {
                if let Some(rss_kb) = read_shell_rss_kb(pid) {
                    updated.insert(*tab_id, TerminalMemoryStat { pid, rss_kb });
                }
                if let Some(cwd) = read_terminal_cwd(pid) {
                    cwds.insert(*tab_id, cwd);
                }
            }
        }

        let shell_pids: Vec<u32> = self.terminals.values().filter_map(|t| t.read(cx).shell_pid()).collect();
        let descendants_map = get_all_terminal_descendants(&shell_pids);
        let mut attention_changed = false;

        for (tab_id, terminal) in &self.terminals {
            let mut check_needs_attention = false;
            let mut agent_name = String::new();
            let mut has_descendants = false;
            if let Some(pid) = terminal.read(cx).shell_pid() {
                if let Some(descendants) = descendants_map.get(&pid) {
                    if !descendants.is_empty() {
                        has_descendants = true;
                    }
                    for (_child_pid, cmd) in descendants {
                        if is_llm_cli_agent(cmd) {
                            check_needs_attention = true;
                            agent_name = extract_agent_name(cmd);
                            break;
                        }
                    }
                }
            }

            if check_needs_attention {
                let is_idle = std::time::Instant::now()
                    .duration_since(terminal.read(cx).last_output_time)
                    > Duration::from_millis(1500);
                if is_idle {
                    let last_line = terminal.update(cx, |m, _| m.get_last_nonempty_line());
                    let has_prompt = if let Some(ref line) = last_line {
                        line_looks_like_prompt(line)
                    } else {
                        false
                    };

                    if has_prompt {
                        let was_needed = terminal.read(cx).needs_attention;
                        if !was_needed {
                            terminal.update(cx, |m, _| {
                                m.needs_attention = true;
                            });
                            attention_changed = true;
                            let notification_msg = format!("{} in Tab {} is waiting for your input.", agent_name, tab_id);
                            send_desktop_notification("Ghost-mux Attention Needed", &notification_msg);
                        }
                    }
                }
            }

            let ongoing = has_descendants && !terminal.read(cx).needs_attention;
            let was_ongoing = terminal.read(cx).process_ongoing;
            if was_ongoing != ongoing {
                terminal.update(cx, |m, _| {
                    m.process_ongoing = ongoing;
                    if was_ongoing && !ongoing {
                        m.job_done = true;
                    }
                });
                attention_changed = true;
            }
        }

        let mut dashboard_cwds_changed = false;
        for d in self.dashboards.values_mut() {
            let mut new_dir = None;
            for panel in d.panel_tabs.values() {
                if let Some(active_tab) = panel.tabs.get(panel.active_tab) {
                    if let Some(cwd) = cwds.get(&active_tab.id) {
                        new_dir = Some(cwd.clone());
                        break;
                    }
                }
            }
            if new_dir.is_none() {
                for panel in d.panel_tabs.values() {
                    for tab in &panel.tabs {
                        if let Some(cwd) = cwds.get(&tab.id) {
                            new_dir = Some(cwd.clone());
                            break;
                        }
                    }
                    if new_dir.is_some() {
                        break;
                    }
                }
            }
            if let Some(dir) = new_dir {
                if d.current_dir != dir {
                    d.current_dir = dir;
                    dashboard_cwds_changed = true;
                }
            }
        }

        let mut cwds_changed = false;
        for (k, v) in cwds {
            if self.terminal_cwds.get(&k) != Some(&v) {
                self.terminal_cwds.insert(k, v);
                cwds_changed = true;
            }
        }

        // Refresh git diffs for active Git tabs
        let mut git_tabs_to_refresh = Vec::new();
        for d in self.dashboards.values() {
            for panel in d.panel_tabs.values() {
                if let Some(active_tab) = panel.tabs.get(panel.active_tab) {
                    if active_tab.content == PanelContent::Git {
                        git_tabs_to_refresh.push(active_tab.id);
                    }
                }
            }
        }
        let mut git_changed = false;
        for tab_id in git_tabs_to_refresh {
            if self.refresh_git_diff(tab_id, cx) {
                git_changed = true;
            }
        }

        let shells_rss_kb = updated.values().map(|s| s.rss_kb).sum();
        let app_rss_kb = read_app_phys_footprint_kb().unwrap_or(0);
        let snapshot = MemorySnapshot { app_rss_kb, shells_rss_kb };
        let changed = updated != self.terminal_memory
            || snapshot != self.memory_snapshot
            || cwds_changed
            || git_changed
            || dashboard_cwds_changed
            || attention_changed;
        self.terminal_memory = updated;
        self.memory_snapshot = snapshot;
        if dashboard_cwds_changed {
            self.persist(cx);
        }
        if changed {
            cx.notify();
        }
    }

    fn panel_active_content(&self, dashboard_id: usize, panel_id: usize) -> PanelContent {
        self.dashboards
            .get(&dashboard_id)
            .and_then(|d| d.panel_tabs.get(&panel_id))
            .and_then(|p| p.tabs.get(p.active_tab))
            .map(|t| t.content.clone())
            .unwrap_or(PanelContent::Terminal)
    }

    pub fn refresh_git_diff(&mut self, tab_id: usize, _cx: &mut Context<Self>) -> bool {
        let cwd = self.terminal_cwds.get(&tab_id).cloned().unwrap_or_else(|| {
            if let Some(dashboard) = self.dashboards.get(&self.active_dashboard_id) {
                dashboard.current_dir.clone()
            } else {
                std::env::current_dir().unwrap_or_default()
            }
        });

        let is_git = std::process::Command::new("git")
            .arg("rev-parse")
            .arg("--is-inside-work-tree")
            .current_dir(&cwd)
            .output();

        let new_state = match is_git {
            Ok(output) if output.status.success() => {
                let branch_output = std::process::Command::new("git")
                    .arg("rev-parse")
                    .arg("--abbrev-ref")
                    .arg("HEAD")
                    .current_dir(&cwd)
                    .output();
                let branch = if let Ok(out) = branch_output {
                    if out.status.success() {
                        String::from_utf8_lossy(&out.stdout).trim().to_string()
                    } else {
                        "HEAD".to_string()
                    }
                } else {
                    "HEAD".to_string()
                };

                let mut files = Vec::new();
                if let Ok(status_output) = std::process::Command::new("git")
                    .arg("status")
                    .arg("--porcelain")
                    .current_dir(&cwd)
                    .output()
                {
                    if status_output.status.success() {
                        let stdout = String::from_utf8_lossy(&status_output.stdout);
                        for line in stdout.lines() {
                            if line.len() > 3 {
                                let status = line[0..2].trim().to_string();
                                let path = line[3..].to_string();
                                files.push(GitDiffFile { status, path });
                            }
                        }
                    }
                }

                GitDiffState {
                    branch,
                    files,
                    diff: String::new(),
                    error: None,
                }
            }
            Ok(_) => {
                GitDiffState {
                    branch: String::new(),
                    files: Vec::new(),
                    diff: String::new(),
                    error: Some("Not a git repository".to_string()),
                }
            }
            Err(_) => {
                GitDiffState {
                    branch: String::new(),
                    files: Vec::new(),
                    diff: String::new(),
                    error: Some("git command not found".to_string()),
                }
            }
        };

        let mut changed = true;
        if let Some(old_state) = self.git_diffs.get(&tab_id) {
            if old_state.branch == new_state.branch
                && old_state.error == new_state.error
                && old_state.diff == new_state.diff
                && old_state.files.len() == new_state.files.len()
            {
                let files_match = old_state.files.iter().zip(new_state.files.iter()).all(|(a, b)| {
                    a.status == b.status && a.path == b.path
                });
                if files_match {
                    changed = false;
                }
            }
        }

        if changed {
            self.git_diffs.insert(tab_id, new_state);
        }
        changed
    }

    fn ensure_content_entity(
        &mut self,
        tab_id: usize,
        content: PanelContent,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ref path) = cwd {
            self.terminal_cwds.entry(tab_id).or_insert_with(|| path.clone());
        }
        match content {
            PanelContent::Terminal => {
                if !self.terminals.contains_key(&tab_id) {
                    let terminal = cx.new(|cx| TerminalModel::new(cwd, cx));
                    self.terminals.insert(tab_id, terminal);
                }
            }
            PanelContent::FileExplorer => {
                if !self.terminals.contains_key(&tab_id) {
                    let terminal = cx.new(|cx| TerminalModel::new(cwd, cx));
                    self.terminals.insert(tab_id, terminal);
                }
            }
            PanelContent::Git => {
                self.refresh_git_diff(tab_id, cx);
            }
            PanelContent::Browser { url } => {
                if !self.browsers.contains_key(&tab_id) {
                    let editor = cx.new(|cx| {
                        let mut e = InputState::new(window, cx)
                            .multi_line(false);
                        e.set_value(url.clone(), window, cx);
                        e
                    });
                    
                    cx.subscribe(&editor, move |this, editor, event, cx| {
                        if let gpui_component::input::InputEvent::PressEnter { .. } = event {
                            let new_url = editor.read(cx).value().to_string();
                            this.navigate_browser(tab_id, &new_url, cx);
                        }
                    }).detach();
                    
                    let (tx, rx) = std::sync::mpsc::channel::<String>();
                    let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
                    let handle = crate::browser::WebViewHandle::new(
                        window,
                        &url,
                        Box::new(move |new_url: String| {
                            let _ = tx.send(new_url);
                        }),
                    ).map(std::rc::Rc::new);

                    let entity = cx.weak_entity();
                    let window_handle = window.window_handle();
                    let tab_id_clone = tab_id;
                    cx.spawn(async move |_entity, cx| {
                        loop {
                            let rx_clone = rx.clone();
                            let res: Option<String> = cx.background_executor().spawn(async move {
                                rx_clone.lock().unwrap().recv().ok()
                            }).await;
                            
                            if let Some(new_url) = res {
                                let _ = cx.update(|cx| {
                                    let _ = cx.update_window(window_handle, |_, window, cx| {
                                        let _ = entity.update(cx, |this, cx| {
                                            this.on_browser_url_changed(tab_id_clone, &new_url, window, cx);
                                        });
                                    });
                                });
                            } else {
                                break;
                            }
                        }
                    }).detach();

                    self.browsers.insert(tab_id, BrowserState {
                        url_editor: editor,
                        handle,
                    });
                }
            }
            PanelContent::Editor { path, is_diff, status } => {
                if !self.editors.contains_key(&tab_id) {
                    let editor = if is_diff {
                        let cwd = cwd.clone().unwrap_or_else(|| {
                            std::env::current_dir().unwrap_or_default()
                        });
                        let status_str = status.clone().unwrap_or_default();
                        let diff_content = self.get_file_diff(&path, &status_str, &cwd);
                        cx.new(|cx| {
                            let mut e = InputState::new(window, cx)
                                .multi_line(true)
                                .code_editor("diff")
                                .line_number(true)
                                .disabled(true);
                            e.set_value(diff_content, window, cx);
                            e
                        })
                    } else {
                        let content = std::fs::read_to_string(&path).unwrap_or_default();
                        let lang = detect_language(&path);
                        cx.new(|cx| {
                            let mut e = InputState::new(window, cx)
                                .multi_line(true)
                                .code_editor(lang)
                                .line_number(true);
                            e.set_value(content, window, cx);
                            e
                        })
                    };
                    self.editors.insert(tab_id, editor);
                }
            }
        }
    }

    pub fn navigate_browser(&mut self, tab_id: usize, url: &str, cx: &mut Context<Self>) {
        if let Some(browser) = self.browsers.get_mut(&tab_id) {
            if let Some(ref handle) = browser.handle {
                handle.load_url(url);
            }
            self.update_browser_tab_url(tab_id, url, cx);
        }
    }

    fn update_browser_tab_url(&mut self, tab_id: usize, url: &str, cx: &mut Context<Self>) {
        for state in self.dashboards.values_mut() {
            for panel_tabs in state.panel_tabs.values_mut() {
                for tab in &mut panel_tabs.tabs {
                    if tab.id == tab_id {
                        if let PanelContent::Browser { url: ref mut tab_url } = tab.content {
                            *tab_url = url.to_string();
                            self.persist(cx);
                            cx.notify();
                            return;
                        }
                    }
                }
            }
        }
    }

    pub fn on_browser_url_changed(&mut self, tab_id: usize, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(browser) = self.browsers.get_mut(&tab_id) {
            browser.url_editor.update(cx, |editor, cx| {
                editor.set_value(url.to_string(), window, cx);
            });
            self.update_browser_tab_url(tab_id, url, cx);
        }
    }

    fn hide_inactive_browsers(&self) {
        let active_dashboard = match self.dashboards.get(&self.active_dashboard_id) {
            Some(d) => d,
            None => {
                for browser in self.browsers.values() {
                    if let Some(ref handle) = browser.handle {
                        handle.set_visible(false);
                    }
                }
                return;
            }
        };

        for (&tab_id, browser) in &self.browsers {
            let mut is_active = false;
            for panel_tabs in active_dashboard.panel_tabs.values() {
                if let Some(active_tab) = panel_tabs.tabs.get(panel_tabs.active_tab) {
                    if active_tab.id == tab_id {
                        if let PanelContent::Browser { .. } = active_tab.content {
                            is_active = true;
                        }
                    }
                }
            }

            if let Some(ref handle) = browser.handle {
                handle.set_visible(is_active);
            }
        }
    }

    pub fn add_panel_tab(
        &mut self,
        dashboard_id: usize,
        panel_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = self.panel_active_content(dashboard_id, panel_id);
        let tab_id = self.next_id;
        self.next_id += 1;
        let cwd = self.dashboards.get(&dashboard_id).map(|d| d.current_dir.clone());
        self.ensure_content_entity(tab_id, content.clone(), cwd, window, cx);

        self.open_menu = None;
        if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get_mut(&panel_id) {
                let title = content_title(&content);
                panel.tabs.push(PanelTab {
                    id: tab_id,
                    title,
                    content,
                });
                panel.active_tab = panel.tabs.len() - 1;
                self.persist(cx);
                cx.notify();
            }
        }
    }

    pub fn remove_panel_tab(
        &mut self,
        dashboard_id: usize,
        panel_id: usize,
        tab_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        let removed_tab_id = if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get_mut(&panel_id) {
                if panel.tabs.len() <= 1 || tab_idx >= panel.tabs.len() {
                    return;
                }
                let removed = panel.tabs.remove(tab_idx).id;
                if panel.active_tab >= panel.tabs.len() {
                    panel.active_tab = panel.tabs.len() - 1;
                } else if tab_idx < panel.active_tab {
                    panel.active_tab -= 1;
                }
                Some(removed)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(tab_id) = removed_tab_id {
            self.terminals.remove(&tab_id);
            self.editors.remove(&tab_id);
            self.browsers.remove(&tab_id);
            self.terminal_cwds.remove(&tab_id);
            self.expanded_paths.remove(&tab_id);
            self.git_diffs.remove(&tab_id);
            self.git_tree_view.remove(&tab_id);
            self.git_diff_side_by_side.remove(&tab_id);
            self.git_diff_wrap.remove(&tab_id);
            self.git_collapsed_paths.remove(&tab_id);
            self.persist(cx);
            cx.notify();
        }
    }

    pub fn switch_panel_tab(
        &mut self,
        dashboard_id: usize,
        panel_id: usize,
        tab_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get_mut(&panel_id) {
                if tab_idx < panel.tabs.len() && tab_idx != panel.active_tab {
                    panel.active_tab = tab_idx;
                    if let Some(tab) = panel.tabs.get(tab_idx) {
                        if let Some(terminal) = self.terminals.get(&tab.id) {
                            terminal.update(cx, |m, _| {
                                m.needs_attention = false;
                                m.job_done = false;
                            });
                        }
                    }
                    self.persist(cx);
                    cx.notify();
                }
            }
        }
    }

    pub fn set_panel_tab_content(
        &mut self,
        dashboard_id: usize,
        panel_id: usize,
        content: PanelContent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu = None;
        let tab_id = if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get_mut(&panel_id) {
                if let Some(active_tab) = panel.tabs.get_mut(panel.active_tab) {
                    if active_tab.content == content {
                        return;
                    }
                    active_tab.content = content.clone();
                    active_tab.title = content_title(&content);
                    active_tab.id
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            return;
        };

        let cwd = self.dashboards.get(&dashboard_id).map(|d| d.current_dir.clone());
        self.ensure_content_entity(tab_id, content, cwd, window, cx);
        self.persist(cx);
        cx.notify();
    }

    pub fn toggle_tab_menu(&mut self, panel_id: usize, tab_idx: usize, cx: &mut Context<Self>) {
        if self.open_menu == Some((panel_id, tab_idx)) {
            self.open_menu = None;
        } else {
            self.open_menu = Some((panel_id, tab_idx));
        }
        cx.notify();
    }

    pub fn split_panel(
        &mut self,
        dashboard_id: usize,
        panel_id: usize,
        dir: SplitDir,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = self.panel_active_content(dashboard_id, panel_id);
        let Some(dashboard) = self.dashboards.get(&dashboard_id) else {
            return;
        };
        if !dashboard.panel_tabs.contains_key(&panel_id) {
            return;
        }

        let new_panel = self.next_id;
        let new_split = self.next_id + 1;
        let new_tab = self.next_id + 2;
        self.next_id += 3;

        let cwd = self.dashboards.get(&dashboard_id).map(|d| d.current_dir.clone());
        self.ensure_content_entity(new_tab, content.clone(), cwd, window, cx);
        if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            dashboard.layout = dashboard
                .layout
                .clone()
                .split(panel_id, dir, new_panel, new_split);
            dashboard.panel_tabs.insert(
                new_panel,
                PanelTabs {
                    tabs: vec![PanelTab {
                        id: new_tab,
                        title: content_title(&content),
                        content,
                    }],
                    active_tab: 0,
                },
            );
        }
        self.persist(cx);
        cx.notify();
    }

    pub fn close_panel(&mut self, dashboard_id: usize, panel_id: usize, cx: &mut Context<Self>) {
        let (updated_layout, old_split_ids, removed_tabs) =
            if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
                let old_split_ids = dashboard.layout.collect_split_ids();
                if let Some(layout) = dashboard.layout.clone().close(panel_id) {
                    let removed_tabs = dashboard
                        .panel_tabs
                        .remove(&panel_id)
                        .map(|panel| panel.tabs)
                        .unwrap_or_default();
                    (Some(layout), old_split_ids, removed_tabs)
                } else {
                    (None, vec![], vec![])
                }
            } else {
                (None, vec![], vec![])
            };

        if let Some(layout) = updated_layout {
            // Remove resizable states for split nodes that no longer exist.
            let new_split_ids: std::collections::HashSet<usize> =
                layout.collect_split_ids().into_iter().collect();
            for split_id in old_split_ids {
                if !new_split_ids.contains(&split_id) {
                    self.resizable_states.remove(&split_id);
                    self.resizable_subscriptions.remove(&split_id);
                }
            }
            if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
                dashboard.layout = layout;
            }
            for tab in removed_tabs {
                self.terminals.remove(&tab.id);
                self.editors.remove(&tab.id);
                self.browsers.remove(&tab.id);
            }
            self.panel_focus_handles.remove(&panel_id);
            self.persist(cx);
            cx.notify();
        }
    }

    pub fn render_layout(
        &mut self,
        dashboard_id: usize,
        layout: &PanelLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match layout {
            PanelLayout::Leaf(id) => self.render_panel(dashboard_id, *id, window, cx),
            PanelLayout::HSplit { left, right, id } => {
                let sid = *id;
                // Get-or-create an externally-owned ResizableState.  On the
                // very first render after a restore, use new_with_ratios so
                // the user's saved sizes are applied without a visible 50/50 → correct-split flash.
                // Extract the pending ratio before calling entry() to avoid
                // a simultaneous mutable borrow of two separate fields.
                let pending_ratios = self.pending_split_ratios.remove(&sid);
                let state_exists = self.resizable_states.contains_key(&sid);
                let state = self
                    .resizable_states
                    .entry(sid)
                    .or_insert_with(|| {
                        let init = if let Some(ratios) = pending_ratios {
                            ResizableState::new_with_ratios(&ratios)
                        } else {
                            ResizableState::default()
                        };
                        cx.new(|_| init)
                    })
                    .clone();

                if !state_exists {
                    let sub = cx.subscribe(&state, move |this, _emitter, event: &ResizablePanelEvent, cx| {
                        match event {
                            ResizablePanelEvent::Resized => {
                                this.persist(cx);
                            }
                        }
                    });
                    self.resizable_subscriptions.insert(sid, sub);
                }

                let left_el = self.render_layout(dashboard_id, left, window, cx);
                let right_el = self.render_layout(dashboard_id, right, window, cx);
                h_resizable(format!("h-{sid}"))
                    .with_state(&state)
                    .child(resizable_panel().child(left_el))
                    .child(resizable_panel().child(right_el))
                    .into_any_element()
            }
            PanelLayout::VSplit { top, bot, id } => {
                let sid = *id;
                // Extract the pending ratio before calling entry() to avoid
                // a simultaneous mutable borrow of two separate fields.
                let pending_ratios = self.pending_split_ratios.remove(&sid);
                let state_exists = self.resizable_states.contains_key(&sid);
                let state = self
                    .resizable_states
                    .entry(sid)
                    .or_insert_with(|| {
                        let init = if let Some(ratios) = pending_ratios {
                            ResizableState::new_with_ratios(&ratios)
                        } else {
                            ResizableState::default()
                        };
                        cx.new(|_| init)
                    })
                    .clone();

                if !state_exists {
                    let sub = cx.subscribe(&state, move |this, _emitter, event: &ResizablePanelEvent, cx| {
                        match event {
                            ResizablePanelEvent::Resized => {
                                this.persist(cx);
                            }
                        }
                    });
                    self.resizable_subscriptions.insert(sid, sub);
                }

                let top_el = self.render_layout(dashboard_id, top, window, cx);
                let bot_el = self.render_layout(dashboard_id, bot, window, cx);
                v_resizable(format!("v-{sid}"))
                    .with_state(&state)
                    .child(resizable_panel().child(top_el))
                    .child(resizable_panel().child(bot_el))
                    .into_any_element()
            }
        }
    }


    fn render_panel(&mut self, dashboard_id: usize, panel_id: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(dashboard) = self.dashboards.get(&dashboard_id) else {
            return div().size_full().bg(theme.background).into_any_element();
        };
        let can_close = dashboard.layout.leaf_count() > 1;
        let Some(panel_tabs) = dashboard.panel_tabs.get(&panel_id) else {
            return div().size_full().bg(theme.background).into_any_element();
        };
        let Some(active_tab) = panel_tabs.tabs.get(panel_tabs.active_tab) else {
            return div().size_full().bg(theme.background).into_any_element();
        };

        let is_editor_on = self.editor_panels.contains(&panel_id);

        let focus_handle = self
            .panel_focus_handles
            .entry(panel_id)
            .or_insert_with(|| cx.focus_handle())
            .clone();

        let is_focused = focus_handle.contains_focused(window, cx);
        let border_color = if is_focused {
            theme.accent
        } else {
            theme.border
        };

        let fh_click = focus_handle.clone();

        let click_catcher = if self.open_menu.is_some() {
            Some(
                div()
                    .id(ElementId::Integer(2_900_000 + panel_id as u64))
                    .absolute()
                    .top(px(self.settings.layout.panel_header_height))
                    .left_0()
                    .size_full()
                    .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
                        this.open_menu = None;
                        cx.notify();
                    }))
            )
        } else {
            None
        };

        let header_height = self.settings.layout.panel_header_height;

        div()
            .id(ElementId::Integer(1_000_000 + panel_id as u64))
            .size_full()
            .relative()
            .bg(theme.background)
            .border_1()
            .border_color(border_color)
            .rounded_sm()
            .overflow_hidden()
            .track_focus(&focus_handle)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&fh_click, cx);
                crate::browser::restore_gpui_focus(window);
            })
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().h(px(header_height)).w_full())
                    .child(render_panel_content(
                        active_tab.id,
                        active_tab.content.clone(),
                        dashboard_id,
                        panel_id,
                        dashboard.current_dir.clone(),
                        &self.terminals,
                        &self.editors,
                        &self.browsers,
                        &self.terminal_cwds,
                        &self.expanded_paths,
                        &self.git_diffs,
                        &self.git_tree_view,
                        &self.git_diff_side_by_side,
                        &self.git_diff_wrap,
                        &self.git_collapsed_paths,
                        &self.git_diff_scroll_handles,
                        &self.git_diff_div_scroll_handles,
                        &self.settings.terminal,
                        &self.settings.layout,
                        window,
                        cx,
                    ))
            )
            .children(click_catcher)
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(header_height))
                    .child(panel_header(
                        dashboard_id,
                        panel_id,
                        panel_tabs,
                        can_close,
                        is_editor_on,
                        &self.settings.layout,
                        self.open_menu,
                        &self.terminals,
                        cx,
                    ))
            )
            .into_any_element()
    }

    pub fn toggle_explorer_dir(&mut self, tab_id: usize, path: PathBuf, cx: &mut Context<Self>) {
        let expanded = self.expanded_paths.entry(tab_id).or_default();
        if expanded.contains(&path) {
            expanded.remove(&path);
        } else {
            expanded.insert(path);
        }
        cx.notify();
    }

    pub fn open_explorer_file(
        &mut self,
        _dashboard_id: usize,
        _panel_id: usize,
        _tab_id: usize,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor_panel_id) = self.editor_panels.iter().cloned().next() {
            self.open_file_in_panel(editor_panel_id, path, false, None, window, cx);
        } else {
            if let Ok(content) = std::fs::read_to_string(&path) {
                let lang = detect_language(&path);
                let editor = cx.new(|cx| {
                    let mut e = InputState::new(window, cx)
                        .multi_line(true)
                        .code_editor(lang)
                        .line_number(true);
                    e.set_value(content, window, cx);
                    e
                });
                let focus_handle = editor.focus_handle(cx);
                window.on_next_frame(move |window, cx| {
                    window.focus(&focus_handle, cx);
                    crate::browser::restore_gpui_focus(window);
                });
                self.modal_editor = Some(ModalEditorState {
                    path,
                    editor,
                    is_diff: false,
                    side_by_side: false,
                    scroll_handle: UniformListScrollHandle::new(),
                });
                cx.notify();
            }
        }
    }

    pub fn close_modal_editor(&mut self, cx: &mut Context<Self>) {
        self.modal_editor = None;
        cx.notify();
    }

    pub fn get_file_diff(&self, path: &std::path::Path, status: &str, cwd: &std::path::Path) -> String {
        if status == "??" {
            if let Ok(out) = std::process::Command::new("git")
                .arg("diff")
                .arg("--no-index")
                .arg("--")
                .arg("/dev/null")
                .arg(path)
                .current_dir(cwd)
                .output()
            {
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() || out.status.code() == Some(1) {
                    return stdout;
                } else {
                    return format!("Error running git diff: {}\n{}", stdout, stderr);
                }
            }
        }

        let has_head = std::process::Command::new("git")
            .arg("rev-parse")
            .arg("HEAD")
            .current_dir(cwd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        let diff_output = if has_head {
            std::process::Command::new("git")
                .arg("diff")
                .arg("HEAD")
                .arg("--")
                .arg(path)
                .current_dir(cwd)
                .output()
        } else {
            let unstaged = std::process::Command::new("git")
                .arg("diff")
                .arg("--")
                .arg(path)
                .current_dir(cwd)
                .output();
            let staged = std::process::Command::new("git")
                .arg("diff")
                .arg("--cached")
                .arg("--")
                .arg(path)
                .current_dir(cwd)
                .output();
            
            match (unstaged, staged) {
                (Ok(u), Ok(s)) => {
                    let mut combined = u.stdout;
                    combined.extend_from_slice(&s.stdout);
                    Ok(std::process::Output {
                        status: if u.status.success() && s.status.success() {
                            u.status
                        } else {
                            u.status
                        },
                        stdout: combined,
                        stderr: [u.stderr, s.stderr].concat(),
                    })
                }
                (Err(e), _) => Err(e),
                (_, Err(e)) => Err(e),
            }
        };

        match diff_output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
                let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
                if out.status.success() || out.status.code() == Some(1) {
                    stdout
                } else {
                    format!("Error running git diff: {}\n{}", stdout, stderr)
                }
            }
            Err(e) => format!("Failed to run git diff: {}", e),
        }
    }

    pub fn open_git_file_diff(
        &mut self,
        path: PathBuf,
        status: String,
        tab_id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(editor_panel_id) = self.editor_panels.iter().cloned().next() {
            self.open_file_in_panel(editor_panel_id, path, true, Some(status), window, cx);
        } else {
            let cwd = self.terminal_cwds.get(&tab_id).cloned().unwrap_or_else(|| {
                if let Some(dashboard) = self.dashboards.get(&self.active_dashboard_id) {
                    dashboard.current_dir.clone()
                } else {
                    std::env::current_dir().unwrap_or_default()
                }
            });
            
            let diff_content = self.get_file_diff(&path, &status, &cwd);
            let editor = cx.new(|cx| {
                let mut e = InputState::new(window, cx)
                    .multi_line(true)
                    .code_editor("diff")
                    .line_number(true)
                    .disabled(true);
                e.set_value(diff_content, window, cx);
                e
            });
            let focus_handle = editor.focus_handle(cx);
            window.on_next_frame(move |window, cx| {
                window.focus(&focus_handle, cx);
                crate::browser::restore_gpui_focus(window);
            });
            self.modal_editor = Some(ModalEditorState {
                path,
                editor,
                is_diff: true,
                side_by_side: false,
                scroll_handle: UniformListScrollHandle::new(),
            });
            cx.notify();
        }
    }

    pub fn toggle_editor_panel(&mut self, panel_id: usize, cx: &mut Context<Self>) {
        if self.editor_panels.contains(&panel_id) {
            self.editor_panels.remove(&panel_id);
        } else {
            self.editor_panels.clear();
            self.editor_panels.insert(panel_id);
        }
        cx.notify();
    }

    pub fn open_file_in_panel(
        &mut self,
        panel_id: usize,
        path: PathBuf,
        is_diff: bool,
        status: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut found_dashboard_id = None;
        for (db_id, db) in &self.dashboards {
            if db.panel_tabs.contains_key(&panel_id) {
                found_dashboard_id = Some(*db_id);
                break;
            }
        }
        let Some(dashboard_id) = found_dashboard_id else { return; };

        let mut existing_tab_idx = None;
        if let Some(dashboard) = self.dashboards.get(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get(&panel_id) {
                for (idx, tab) in panel.tabs.iter().enumerate() {
                    if let PanelContent::Editor { path: ref p, is_diff: d, .. } = tab.content {
                        if p == &path && d == is_diff {
                            existing_tab_idx = Some(idx);
                            break;
                        }
                    }
                }
            }
        }

        if let Some(idx) = existing_tab_idx {
            self.switch_panel_tab(dashboard_id, panel_id, idx, cx);
            if let Some(dashboard) = self.dashboards.get(&dashboard_id) {
                if let Some(panel) = dashboard.panel_tabs.get(&panel_id) {
                    if let Some(tab) = panel.tabs.get(idx) {
                        if let Some(editor) = self.editors.get(&tab.id) {
                            let focus_handle = editor.focus_handle(cx);
                            window.on_next_frame(move |window, cx| {
                                window.focus(&focus_handle, cx);
                                crate::browser::restore_gpui_focus(window);
                            });
                        }
                    }
                }
            }
            cx.notify();
            return;
        }

        let tab_id = self.next_id;
        self.next_id += 1;
        let title = path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "File".to_string());
        let title = if is_diff { format!("Diff: {}", title) } else { title };

        let cwd = self.terminal_cwds.get(&tab_id).cloned().unwrap_or_else(|| {
            if let Some(dashboard) = self.dashboards.get(&dashboard_id) {
                dashboard.current_dir.clone()
            } else {
                std::env::current_dir().unwrap_or_default()
            }
        });

        let editor = if is_diff {
            let status_str = status.clone().unwrap_or_default();
            let diff_content = self.get_file_diff(&path, &status_str, &cwd);
            cx.new(|cx| {
                let mut e = InputState::new(window, cx)
                    .multi_line(true)
                    .code_editor("diff")
                    .line_number(true)
                    .disabled(true);
                e.set_value(diff_content, window, cx);
                e
            })
        } else {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            let lang = detect_language(&path);
            cx.new(|cx| {
                let mut e = InputState::new(window, cx)
                    .multi_line(true)
                    .code_editor(lang)
                    .line_number(true);
                e.set_value(content, window, cx);
                e
            })
        };

        self.editors.insert(tab_id, editor.clone());

        let new_tab = PanelTab {
            id: tab_id,
            title,
            content: PanelContent::Editor {
                path,
                is_diff,
                status,
            },
        };

        if let Some(dashboard) = self.dashboards.get_mut(&dashboard_id) {
            if let Some(panel) = dashboard.panel_tabs.get_mut(&panel_id) {
                panel.tabs.push(new_tab);
                panel.active_tab = panel.tabs.len() - 1;
            }
        }

        let focus_handle = editor.focus_handle(cx);
        window.on_next_frame(move |window, cx| {
            window.focus(&focus_handle, cx);
            crate::browser::restore_gpui_focus(window);
        });

        self.persist(cx);
        cx.notify();
    }

    pub fn toggle_git_dir(&mut self, tab_id: usize, path: PathBuf, cx: &mut Context<Self>) {
        let collapsed = self.git_collapsed_paths.entry(tab_id).or_default();
        if collapsed.contains(&path) {
            collapsed.remove(&path);
        } else {
            collapsed.insert(path);
        }
        cx.notify();
    }

    pub fn save_modal_file(
        &mut self,
        path: &std::path::Path,
        editor: &Entity<InputState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let content = editor.read(cx).text().to_string();
        if let Err(err) = std::fs::write(path, content) {
            eprintln!("Failed to save file: {:?}", err);
        } else {
            println!("Saved file successfully: {:?}", path);
        }
    }
}

impl Render for DashboardView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.hide_inactive_browsers();
        let active = self
            .active_dashboard()
            .map(|dashboard| (dashboard.id, dashboard.layout.clone(), dashboard.title.clone()));

        let show_sidebar = self.show_sidebar;
        let sidebar_toggle_btn = {
            let theme = cx.theme();
            let layout_settings = &self.settings.layout;
            div()
                .id(ElementId::Integer(800_001))
                .h(px(layout_settings.icon_button_height))
                .px_1()
                .mr_2()
                .rounded_sm()
                .bg(if show_sidebar { theme.accent } else { theme.background })
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(if show_sidebar {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .hover(move |s| s.bg(theme.accent).text_color(theme.foreground))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_sidebar(cx);
                }))
                .child(Icon::new(IconName::PanelLeft).size_3())
        };

        let main = if let Some((dashboard_id, layout, title)) = active {
            let layout_el = self.render_layout(dashboard_id, &layout, _window, cx);
            let theme = cx.theme();
            div()
                .flex_1()
                .h_full()
                .overflow_hidden()
                .child(
                    div()
                        .w_full()
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(self.settings.layout.dashboard_title_height))
                                .px_3()
                                .flex()
                                .items_center()
                                .text_sm()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .bg(theme.secondary)
                                .border_b_1()
                                .border_color(theme.border)
                                .child(sidebar_toggle_btn)
                                .child(title),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .overflow_hidden()
                                .child(layout_el),
                        ),
                )
                .into_any_element()
        } else {
            let theme = cx.theme();
            div()
                .flex_1()
                .h_full()
                .bg(theme.background)
                .items_center()
                .justify_center()
                .child("No dashboards")
                .into_any_element()
        };

        let theme = cx.theme();
        let layout_settings = &self.settings.layout;
        let main_view = if show_sidebar {
            div()
                .size_full()
                .bg(theme.background)
                .child(
                    h_resizable("dashboard-main-split")
                        .child(
                            resizable_panel()
                                .size(px(layout_settings.sidebar_width))
                                .size_range(
                                    px(layout_settings.sidebar_min_width)
                                        ..px(layout_settings.sidebar_max_width),
                                )
                                .flex_none()
                                .child(dashboard_sidebar(self, cx)),
                        )
                        .child(resizable_panel().child(main)),
                )
        } else {
            div()
                .size_full()
                .bg(theme.background)
                .flex()
                .child(main)
        };

        let mut root = div().relative().size_full().child(main_view);

        if let Some(ref modal) = self.modal_editor {
            root = root.child(render_modal_editor(&modal.path, &modal.editor, modal.is_diff, self, cx));
        }

        root.into_any()
    }
}

fn dashboard_sidebar(view: &DashboardView, cx: &mut Context<DashboardView>) -> AnyElement {
    let theme = cx.theme();
    let settings = &view.settings.layout;
    let can_remove_dashboard = view.dashboard_order.len() > 1;

    let dashboard_rows = view
        .dashboard_order
        .iter()
        .filter_map(|dashboard_id| view.dashboards.get(dashboard_id))
        .map(|dashboard| {
            let selected = dashboard.id == view.active_dashboard_id;
            let folder_name = dashboard.current_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| dashboard.current_dir.to_string_lossy().to_string());

            let has_attention = {
                let mut attention = false;
                for panel_tabs in dashboard.panel_tabs.values() {
                    for tab in &panel_tabs.tabs {
                        if let Some(terminal) = view.terminals.get(&tab.id) {
                            if terminal.read(cx).needs_attention {
                                attention = true;
                                break;
                            }
                        }
                    }
                    if attention {
                        break;
                    }
                }
                attention
            };

            let has_ongoing = {
                let mut ongoing = false;
                for panel_tabs in dashboard.panel_tabs.values() {
                    for tab in &panel_tabs.tabs {
                        if let Some(terminal) = view.terminals.get(&tab.id) {
                            if terminal.read(cx).process_ongoing {
                                ongoing = true;
                                break;
                            }
                        }
                    }
                    if ongoing {
                        break;
                    }
                }
                ongoing
            };

            let has_done = {
                let mut done = false;
                for panel_tabs in dashboard.panel_tabs.values() {
                    for tab in &panel_tabs.tabs {
                        if let Some(terminal) = view.terminals.get(&tab.id) {
                            if terminal.read(cx).job_done {
                                done = true;
                                break;
                            }
                        }
                    }
                    if done {
                        break;
                    }
                }
                done
            };

            let badge = if has_attention {
                Some(
                    div()
                        .w(px(8.))
                        .h(px(8.))
                        .rounded_full()
                        .bg(rgb(0xf47067))
                        .ml_2()
                )
            } else {
                None
            };

            let spinner = if has_ongoing {
                Some(
                    div()
                        .ml_2()
                        .child(gpui_component::spinner::Spinner::new().xsmall())
                )
            } else {
                None
            };

            let done_badge = if has_done {
                Some(
                    div()
                        .ml_2()
                        .text_color(rgb(0x57c994))
                        .child(Icon::new(IconName::Check).size_3())
                )
            } else {
                None
            };

            let row = div()
                .id(ElementId::Integer(1_000_000 + dashboard.id as u64))
                .w_full()
                .h(px(settings.sidebar_row_height + 14.))
                .px_2()
                .py_1()
                .rounded_sm()
                .flex()
                .flex_row()
                .items_center()
                .cursor_pointer()
                .bg(if selected {
                    theme.accent
                } else {
                    gpui::transparent_black()
                })
                .text_color(if selected {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                .on_click(cx.listener({
                    let dashboard_id = dashboard.id;
                    move |this, _: &ClickEvent, _window, cx| {
                        this.switch_dashboard(dashboard_id, cx);
                    }
                }))
                .child(
                    div()
                        .flex_1()
                        .flex_col()
                        .overflow_hidden()
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_semibold()
                                        .child(dashboard.title.clone())
                                )
                                .children(badge)
                                .children(spinner)
                                .children(done_badge),
                        )
                        .child(
                            div()
                                .text_size(px(10.))
                                .font_normal()
                                .text_color(if selected {
                                    theme.foreground
                                } else {
                                    theme.muted_foreground
                                })
                                .child(folder_name),
                        ),
                );

            if can_remove_dashboard {
                row.child(
                    div()
                        .id(ElementId::Integer(1_200_000 + dashboard.id as u64))
                        .h(px(settings.sidebar_close_button_size))
                        .w(px(settings.sidebar_close_button_size))
                        .rounded_sm()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(rgb(0xf47067))
                        .hover(|s| s.bg(theme.background))
                        .on_click(cx.listener({
                            let dashboard_id = dashboard.id;
                            move |this, _: &ClickEvent, _window, cx| {
                                this.remove_dashboard(dashboard_id, cx);
                            }
                        }))
                        .child(Icon::new(IconName::Close).size_3()),
                )
            } else {
                row
            }
            .into_any_element()
        });

    let mut terminal_tabs: Vec<usize> = view.terminals.keys().copied().collect();
    terminal_tabs.sort_unstable();
    let terminal_memory_rows: Vec<AnyElement> = terminal_tabs
        .into_iter()
        .map(|tab_id| {
            let row = if let Some(stat) = view.terminal_memory.get(&tab_id) {
                format!(
                    "Tab {tab_id}  PID {}  RSS {}",
                    stat.pid,
                    format_kb_as_mb(stat.rss_kb)
                )
            } else {
                format!("Tab {tab_id}  PID --  RSS --")
            };
            div()
                .w_full()
                .h(px(settings.sidebar_row_height))
                .px_2()
                .rounded_sm()
                .flex()
                .items_center()
                .overflow_hidden()
                .text_xs()
                .text_color(theme.muted_foreground)
                .bg(theme.background)
                .child(row)
                .into_any_element()
        })
        .collect();

    div()
        .w_full()
        .h_full()
        .px_2()
        .py_2()
        .bg(theme.secondary)
        .border_r_1()
        .border_color(theme.border)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .w_full()
                .h(px(settings.sidebar_header_height))
                .px_1()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child("Dashboards"),
                )
                .child(sidebar_text_button(
                    ElementId::Integer(900_003),
                    "Settings",
                    view.show_settings_panel,
                    theme,
                    cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.toggle_settings_panel(cx);
                    }),
                ))
                .child(div().w(px(6.)))
                .child(action_icon_button(
                    ElementId::Integer(900_001),
                    IconName::Plus,
                    false,
                    settings,
                    theme,
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.add_dashboard(window, cx);
                    }),
                )),
        )
        .children(dashboard_rows)
        .child(div().h(px(6.)))
        .child(
            div()
                .w_full()
                .px_1()
                .flex()
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child("Memory"),
                )
                .child(
                    div()
                        .id(ElementId::Integer(900_010))
                        .h(px(16.))
                        .w(px(16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .text_color(theme.muted_foreground)
                        .hover(|s| s.text_color(theme.foreground))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.toggle_memory_stats(cx);
                        }))
                        .child(if view.show_memory_stats {
                            Icon::new(IconName::ChevronDown).size_3()
                        } else {
                            Icon::new(IconName::ChevronRight).size_3()
                        }),
                ),
        )
        .children(if view.show_memory_stats {
            Some(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(memory_stat_row(
                        "App (total)",
                        format_kb_as_mb(view.memory_snapshot.app_rss_kb),
                        theme.accent.into(),
                        theme,
                    ))
                    .child(memory_stat_row(
                        "VT + UI",
                        format_kb_as_mb(
                            view.memory_snapshot
                                .app_rss_kb
                                .saturating_sub(view.memory_snapshot.shells_rss_kb),
                        ),
                        rgb(0x57c994).into(),
                        theme,
                    ))
                    .child(memory_stat_row(
                        "Shells (sum)",
                        format_kb_as_mb(view.memory_snapshot.shells_rss_kb),
                        theme.muted_foreground,
                        theme,
                    ))
                    .child(div().h(px(4.)))
                    .child(
                        div()
                            .w_full()
                            .px_1()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child("Per terminal"),
                    )
                    .children(terminal_memory_rows)
                    .into_any_element(),
            )
        } else {
            None
        })
        .children(if view.show_settings_panel {
            Some(settings_panel(view, cx))
        } else {
            None
        })
        .into_any_element()
}

/// App phys_footprint — matches Activity Monitor "Memory" column exactly.
fn read_app_phys_footprint_kb() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        use libc::{c_int, c_void, proc_pid_rusage, rusage_info_v2, RUSAGE_INFO_V2};
        let pid = std::process::id();
        let mut info: rusage_info_v2 = unsafe { std::mem::zeroed() };
        // proc_pid_rusage takes *mut rusage_info_t which is *mut *mut c_void.
        // We point that at a *mut c_void that in turn points at our struct.
        let mut buf: *mut c_void = &mut info as *mut _ as *mut c_void;
        let ret = unsafe {
            proc_pid_rusage(pid as c_int, RUSAGE_INFO_V2, &mut buf as *mut *mut c_void)
        };
        if ret == 0 {
            return Some(info.ri_phys_footprint / 1024);
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        read_shell_rss_kb(std::process::id())
    }
}

/// Shell child RSS via `ps` — lightweight, no permissions needed.
fn read_shell_rss_kb(pid: u32) -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    raw.trim().parse::<u64>().ok()
}

fn read_terminal_cwd(pid: u32) -> Option<PathBuf> {
    let output = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    for line in raw.lines() {
        if line.starts_with('n') {
            let path_str = &line[1..];
            let path = PathBuf::from(path_str);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

fn get_all_terminal_descendants(shell_pids: &[u32]) -> HashMap<u32, Vec<(u32, String)>> {
    let mut result = HashMap::new();
    if shell_pids.is_empty() {
        return result;
    }
    let output = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,command="])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            if let Ok(raw) = String::from_utf8(out.stdout) {
                let mut parent_to_children = HashMap::new();
                let mut pid_to_command = HashMap::new();
                for line in raw.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        if let (Ok(pid), Ok(ppid)) = (parts[0].parse::<u32>(), parts[1].parse::<u32>()) {
                            let cmd = parts[2..].join(" ");
                            pid_to_command.insert(pid, cmd);
                            parent_to_children.entry(ppid).or_insert_with(Vec::new).push(pid);
                        }
                    }
                }
                for &shell_pid in shell_pids {
                    let mut descendants = Vec::new();
                    let mut queue = vec![shell_pid];
                    let mut visited = std::collections::HashSet::new();
                    visited.insert(shell_pid);
                    while let Some(current_pid) = queue.pop() {
                        if let Some(children) = parent_to_children.get(&current_pid) {
                            for &child in children {
                                if !visited.contains(&child) {
                                    visited.insert(child);
                                    if let Some(cmd) = pid_to_command.get(&child) {
                                        descendants.push((child, cmd.clone()));
                                    }
                                    queue.push(child);
                                }
                            }
                        }
                    }
                    result.insert(shell_pid, descendants);
                }
            }
        }
    }
    result
}

fn is_llm_cli_agent(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    if lower.contains("claude-code") 
       || lower.contains("opencode") 
       || lower.contains("open-code") 
       || lower.contains("aider") 
       || lower.contains("mentat") 
       || lower.contains("copilot-cli")
       || lower.contains("gh-copilot")
       || lower.contains("pi-coding-agent")
       || lower.contains("earendil-works/pi")
    {
        return true;
    }
    
    let parts: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').collect();
    for part in parts {
        if part == "claude" || part == "pi" {
            return true;
        }
    }
    false
}

fn extract_agent_name(cmd: &str) -> String {
    let lower = cmd.to_lowercase();
    if lower.contains("claude-code") || lower.contains("claude") {
        "Claude Code".to_string()
    } else if lower.contains("opencode") || lower.contains("open-code") {
        "OpenCode".to_string()
    } else if lower.contains("aider") {
        "Aider".to_string()
    } else if lower.contains("mentat") {
        "Mentat".to_string()
    } else if lower.contains("copilot") {
        "Copilot CLI".to_string()
    } else if lower.contains("pi") {
        "Pi CLI".to_string()
    } else {
        "LLM Agent".to_string()
    }
}

fn line_looks_like_prompt(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let is_prompt = trimmed.ends_with('>')
        || trimmed.ends_with('?')
        || trimmed.ends_with(':')
        || trimmed.ends_with("❯")
        || trimmed.ends_with("$")
        || trimmed.ends_with("#")
        || trimmed.ends_with("%")
        || trimmed.ends_with('π')
        || trimmed.ends_with('›')
        || trimmed.ends_with('»');
    
    if is_prompt {
        return true;
    }

    let line_lower = trimmed.to_lowercase();
    line_lower.contains("enter a message")
        || line_lower.contains("input")
        || line_lower.contains("prompt")
        || line_lower.contains("press enter")
        || line_lower.contains("user:")
}

fn send_desktop_notification(title: &str, message: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .arg("-e")
            .arg(format!(
                "display notification {:?} with title {:?}",
                message, title
            ))
            .spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .arg(title)
            .arg(message)
            .spawn();
    }
}

fn format_kb_as_mb(kb: u64) -> String {
    format!("{:.1} MB", kb as f64 / 1024.0)
}

fn memory_stat_row(
    label: &'static str,
    value: String,
    value_color: Hsla,
    theme: &gpui_component::theme::Theme,
) -> AnyElement {
    div()
        .w_full()
        .h(px(20.))
        .px_2()
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(value_color)
                .child(value),
        )
        .into_any_element()
}

fn sidebar_text_button(
    eid: ElementId,
    label: &'static str,
    active: bool,
    theme: &gpui_component::theme::Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(eid)
        .h(px(18.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(if active { theme.accent } else { theme.background })
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .text_xs()
        .font_semibold()
        .cursor_pointer()
        .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
        .on_click(handler)
        .child(label)
}

fn settings_action_button(
    eid: ElementId,
    label: &'static str,
    theme: &gpui_component::theme::Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(eid)
        .h(px(20.))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_color(theme.muted_foreground)
        .text_xs()
        .font_semibold()
        .cursor_pointer()
        .hover(|s| s.bg(theme.accent).text_color(theme.foreground))
        .on_click(handler)
        .child(label)
        .into_any_element()
}

fn settings_number_row(
    label: &'static str,
    value: f32,
    dec_handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    inc_handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    theme: &gpui_component::theme::Theme,
    id_base: u64,
) -> AnyElement {
    div()
        .w_full()
        .h(px(22.))
        .flex()
        .items_center()
        .child(
            div()
                .flex_1()
                .overflow_hidden()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .id(ElementId::Integer(id_base))
                .h(px(18.))
                .w(px(18.))
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                .on_click(dec_handler)
                .child("-"),
        )
        .child(
            div()
                .w(px(56.))
                .text_center()
                .text_xs()
                .text_color(theme.foreground)
                .child(format!("{value:.2}")),
        )
        .child(
            div()
                .id(ElementId::Integer(id_base + 1))
                .h(px(18.))
                .w(px(18.))
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                .on_click(inc_handler)
                .child("+"),
        )
        .into_any_element()
}

fn settings_panel(view: &DashboardView, cx: &mut Context<DashboardView>) -> AnyElement {
    let theme = cx.theme();
    let settings = &view.settings;
    div()
        .w_full()
        .mt_2()
        .p_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_xs()
                .font_semibold()
                .text_color(theme.foreground)
                .child("settings.yaml"),
        )
        .child(settings_number_row(
            "theme.font_size",
            settings.theme.font_size,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeFontSize, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeFontSize, 0.5, cx);
            }),
            theme,
            910_000,
        ))
        .child(settings_number_row(
            "theme.mono_font_size",
            settings.theme.mono_font_size,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeMonoFontSize, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeMonoFontSize, 0.5, cx);
            }),
            theme,
            910_010,
        ))
        .child(settings_number_row(
            "theme.radius",
            settings.theme.radius,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeRadius, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeRadius, 0.5, cx);
            }),
            theme,
            910_020,
        ))
        .child(settings_number_row(
            "theme.radius_lg",
            settings.theme.radius_lg,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeRadiusLg, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::ThemeRadiusLg, 0.5, cx);
            }),
            theme,
            910_030,
        ))
        .child(settings_number_row(
            "layout.sidebar_width",
            settings.layout.sidebar_width,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarWidth, -10.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarWidth, 10.0, cx);
            }),
            theme,
            910_040,
        ))
        .child(settings_number_row(
            "layout.sidebar_min_width",
            settings.layout.sidebar_min_width,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarMinWidth, -10.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarMinWidth, 10.0, cx);
            }),
            theme,
            910_050,
        ))
        .child(settings_number_row(
            "layout.sidebar_max_width",
            settings.layout.sidebar_max_width,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarMaxWidth, -10.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::SidebarMaxWidth, 10.0, cx);
            }),
            theme,
            910_060,
        ))
        .child(settings_number_row(
            "layout.panel_header_height",
            settings.layout.panel_header_height,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::PanelHeaderHeight, -1.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::PanelHeaderHeight, 1.0, cx);
            }),
            theme,
            910_070,
        ))
        .child(settings_number_row(
            "layout.panel_tab_height",
            settings.layout.panel_tab_height,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::PanelTabHeight, -1.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::PanelTabHeight, 1.0, cx);
            }),
            theme,
            910_080,
        ))
        .child(settings_number_row(
            "layout.icon_button_height",
            settings.layout.icon_button_height,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::IconButtonHeight, -1.0, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::IconButtonHeight, 1.0, cx);
            }),
            theme,
            910_090,
        ))
        .child(settings_number_row(
            "terminal.font_size",
            settings.terminal.font_size,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalFontSize, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalFontSize, 0.5, cx);
            }),
            theme,
            910_100,
        ))
        .child(settings_number_row(
            "terminal.line_height",
            settings.terminal.line_height,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalLineHeight, -0.5, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalLineHeight, 0.5, cx);
            }),
            theme,
            910_110,
        ))
        .child(settings_number_row(
            "terminal.char_width",
            settings.terminal.char_width,
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalCharWidth, -0.1, cx);
            }),
            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.adjust_settings_number(SettingsNumberField::TerminalCharWidth, 0.1, cx);
            }),
            theme,
            910_120,
        ))
        .child(
            div()
                .w_full()
                .mt_1()
                .flex()
                .gap_1()
                .child(settings_action_button(
                    ElementId::Integer(920_000),
                    "Save YAML",
                    theme,
                    cx.listener(move |this, _: &ClickEvent, _window, cx| {
                        this.save_settings(cx);
                    }),
                )),
        )
        .children(view.settings_status.as_ref().map(|msg| {
            div()
                .w_full()
                .mt_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(msg.clone())
                .into_any_element()
        }))
        .into_any_element()
}

// --- Panel Header ---

fn panel_header(
    dashboard_id: usize,
    panel_id: usize,
    panel_tabs: &PanelTabs,
    can_close: bool,
    is_editor_on: bool,
    settings: &LayoutSettings,
    open_menu: Option<(usize, usize)>,
    terminals: &HashMap<usize, Entity<TerminalModel>>,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let theme = cx.theme();

    let tab_buttons: Vec<AnyElement> = panel_tabs
        .tabs
        .iter()
        .enumerate()
        .map(|(idx, tab)| {
            let is_active = idx == panel_tabs.active_tab;
            let display_title = match &tab.content {
                PanelContent::Terminal => "terminal".to_string(),
                PanelContent::FileExplorer => "explorer".to_string(),
                PanelContent::Git => "git".to_string(),
                PanelContent::Browser { url } => {
                    if url.is_empty() {
                        "browser".to_string()
                    } else {
                        url.trim_start_matches("https://")
                            .trim_start_matches("http://")
                            .trim_start_matches("www.")
                            .split('/')
                            .next()
                            .unwrap_or(url)
                            .to_string()
                    }
                }
                PanelContent::Editor { path, is_diff, .. } => {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| "editor".to_string());
                    if *is_diff {
                        format!("diff: {name}")
                    } else {
                        name
                    }
                }
            };

            let is_menu_open = open_menu == Some((panel_id, idx));

            let tab_needs_attention = match &tab.content {
                PanelContent::Terminal => {
                    terminals.get(&tab.id).map_or(false, |term| term.read(cx).needs_attention)
                }
                _ => false,
            };

            let tab_process_ongoing = match &tab.content {
                PanelContent::Terminal => {
                    terminals.get(&tab.id).map_or(false, |term| term.read(cx).process_ongoing)
                }
                _ => false,
            };

            let tab_job_done = match &tab.content {
                PanelContent::Terminal => {
                    terminals.get(&tab.id).map_or(false, |term| term.read(cx).job_done)
                }
                _ => false,
            };

            let tab_badge = if tab_needs_attention {
                Some(
                    div()
                        .w(px(6.))
                        .h(px(6.))
                        .rounded_full()
                        .bg(rgb(0xf47067))
                        .ml_1()
                )
            } else {
                None
            };

            let tab_spinner = if tab_process_ongoing {
                Some(
                    div()
                        .ml_1()
                        .child(gpui_component::spinner::Spinner::new().xsmall())
                )
            } else {
                None
            };

            let tab_done_badge = if tab_job_done {
                Some(
                    div()
                        .ml_1()
                        .text_color(rgb(0x57c994))
                        .child(Icon::new(IconName::Check).size_3())
                )
            } else {
                None
            };

            let tab_select = div()
                .id(ElementId::Integer(2_000_000 + tab.id as u64))
                .flex_1()
                .h_full()
                .flex()
                .flex_row()
                .items_center()
                .px_2()
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.switch_panel_tab(dashboard_id, panel_id, idx, cx);
                }))
                .child(div().child(display_title.clone()))
                .children(tab_badge)
                .children(tab_spinner)
                .children(tab_done_badge);

            let menu_btn = div()
                .id(ElementId::Integer(2_200_000 + tab.id as u64))
                .h(px(settings.panel_tab_close_height))
                .w(px(settings.panel_tab_close_width))
                .rounded_sm()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(theme.muted_foreground)
                .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                    this.toggle_tab_menu(panel_id, idx, cx);
                }))
                .child(Icon::new(IconName::Menu).size_3());

            let mut tab_el = div()
                .h(px(settings.panel_tab_height))
                .flex()
                .flex_row()
                .items_center()
                .bg(if is_active { theme.background } else { theme.secondary })
                .border_1()
                .border_color(theme.border)
                .rounded_sm()
                .text_color(if is_active { theme.foreground } else { theme.muted_foreground })
                .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                .child(tab_select)
                .child(menu_btn)
                .child(div().w(px(2.)));

            if panel_tabs.tabs.len() > 1 {
                tab_el = tab_el
                    .child(div().w(px(1.)))
                    .child(
                        div()
                            .id(ElementId::Integer(2_100_000 + tab.id as u64))
                            .h(px(settings.panel_tab_close_height))
                            .w(px(settings.panel_tab_close_width))
                            .rounded_sm()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_color(theme.muted_foreground)
                            .hover(|s| s.bg(theme.muted).text_color(theme.foreground))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                this.remove_panel_tab(dashboard_id, panel_id, idx, cx);
                            }))
                            .child(Icon::new(IconName::Close).size_3())
                    )
                    .child(div().w(px(2.)));
            }

            div()
                .relative()
                .child(tab_el)
                .children(if is_menu_open {
                    Some(
                        div()
                            .absolute()
                            .top_full()
                            .left_0()
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.border)
                            .rounded_sm()
                            .p_1()
                            .min_w(px(100.))
                            .flex()
                            .flex_col()
                            .shadow_md()
                            .child(
                                dropdown_item(
                                    ElementId::Integer(2_300_000 + tab.id as u64 * 10 + 1),
                                    "Terminal",
                                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_menu = None;
                                        this.set_panel_tab_content(dashboard_id, panel_id, PanelContent::Terminal, window, cx);
                                    }),
                                    theme
                                )
                            )
                            .child(
                                dropdown_item(
                                    ElementId::Integer(2_300_000 + tab.id as u64 * 10 + 2),
                                    "Explorer",
                                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_menu = None;
                                        this.set_panel_tab_content(dashboard_id, panel_id, PanelContent::FileExplorer, window, cx);
                                    }),
                                    theme
                                )
                            )
                            .child(
                                dropdown_item(
                                    ElementId::Integer(2_300_000 + tab.id as u64 * 10 + 3),
                                    "Git",
                                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_menu = None;
                                        this.set_panel_tab_content(dashboard_id, panel_id, PanelContent::Git, window, cx);
                                    }),
                                    theme
                                )
                            )
                            .child(
                                dropdown_item(
                                    ElementId::Integer(2_300_000 + tab.id as u64 * 10 + 4),
                                    "Browser",
                                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.open_menu = None;
                                        this.set_panel_tab_content(
                                            dashboard_id,
                                            panel_id,
                                            PanelContent::Browser { url: "https://google.com".to_string() },
                                            window,
                                            cx
                                        );
                                    }),
                                    theme
                                )
                            )
                    )
                } else {
                    None
                })
                .into_any_element()
        })
        .collect();

    let add_panel_tab_btn = action_icon_button(
        ElementId::Integer(3_000_000 + (dashboard_id as u64 * 1_000) + panel_id as u64 * 100 + 1),
        IconName::Plus,
        false,
        settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.add_panel_tab(dashboard_id, panel_id, window, cx);
        }),
    );
    let split_h = action_icon_button(
        ElementId::Integer(3_000_000 + (dashboard_id as u64 * 1_000) + panel_id as u64 * 100 + 5),
        IconName::PanelLeft,
        false,
        settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.split_panel(dashboard_id, panel_id, SplitDir::Horizontal, window, cx);
        }),
    );
    let split_v = action_icon_button(
        ElementId::Integer(3_000_000 + (dashboard_id as u64 * 1_000) + panel_id as u64 * 100 + 6),
        IconName::PanelBottom,
        false,
        settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.split_panel(dashboard_id, panel_id, SplitDir::Vertical, window, cx);
        }),
    );
    let close_btn = action_icon_button(
        ElementId::Integer(3_000_000 + (dashboard_id as u64 * 1_000) + panel_id as u64 * 100 + 2),
        IconName::Close,
        true,
        settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.close_panel(dashboard_id, panel_id, cx);
        }),
    );


    let editor_toggle = action_text_button(
        ElementId::Integer(3_000_000 + (dashboard_id as u64 * 1_000) + panel_id as u64 * 100 + 9),
        "Editor",
        is_editor_on,
        settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.toggle_editor_panel(panel_id, cx);
        }),
    );

    div()
        .w_full()
        .h(px(settings.panel_header_height))
        .bg(theme.secondary)
        .border_b_1()
        .border_color(theme.border)
        .flex()
        .flex_row()
        .items_center()
        .px_2()
        .gap_px()
        .children(tab_buttons)
        .child(add_panel_tab_btn)
        .child(div().flex_1())
        .child(editor_toggle)
        .child(div().w(px(8.)))
        .child(split_h)
        .child(div().w(px(3.)))
        .child(split_v)
        .child(div().w(px(6.)))
        .children(if can_close { Some(close_btn) } else { None })
        .child(div().w(px(2.)))
        .into_any_element()
}


fn action_icon_button(
    eid: ElementId,
    icon: IconName,
    danger: bool,
    settings: &LayoutSettings,
    theme: &gpui_component::theme::Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let danger_color: Hsla = rgb(0xf47067).into();
    div()
        .id(eid)
        .h(px(settings.icon_button_height))
        .px_2()
        .rounded_sm()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_center()
        .text_color(if danger {
            danger_color
        } else {
            theme.muted_foreground
        })
        .hover(move |s| {
            if danger {
                s.bg(theme.muted).text_color(danger_color)
            } else {
                s.bg(theme.accent).text_color(theme.foreground)
            }
        })
        .on_click(handler)
        .child(Icon::new(icon).size_3())
}

fn browser_nav_button(
    eid: ElementId,
    label: &'static str,
    settings: &LayoutSettings,
    theme: &gpui_component::theme::Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(eid)
        .h(px(settings.icon_button_height))
        .w(px(settings.icon_button_height))
        .rounded_sm()
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.muted_foreground)
        .hover(move |s| s.bg(theme.muted).text_color(theme.foreground))
        .on_click(handler)
        .child(label)
}

fn action_text_button(
    eid: ElementId,
    label: &'static str,
    active: bool,
    settings: &LayoutSettings,
    theme: &gpui_component::theme::Theme,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    div()
        .id(eid)
        .h(px(settings.icon_button_height))
        .px_2()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(if active { theme.accent } else { theme.background })
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .text_xs()
        .font_semibold()
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_center()
        .hover(move |s| {
            if active {
                s.bg(theme.accent)
            } else {
                s.bg(theme.muted).text_color(theme.foreground)
            }
        })
        .on_click(handler)
        .child(label)
}

// --- Panel Content Router ---

struct ExplorerNode {
    path: PathBuf,
    name: String,
    is_dir: bool,
    depth: usize,
    is_expanded: bool,
}

fn build_tree(
    dir: &std::path::Path,
    depth: usize,
    expanded_paths: &std::collections::HashSet<PathBuf>,
    nodes: &mut Vec<ExplorerNode>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    let mut dir_entries = Vec::new();
    let mut file_entries = Vec::new();

    for entry in entries {
        if let Ok(entry) = entry {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();

            // Skip common ignored files/directories to match VS Code and keep it fast
            if name.starts_with('.') || name == "node_modules" || name == "target" {
                continue;
            }

            let is_dir = path.is_dir();
            if is_dir {
                dir_entries.push((path, name));
            } else {
                file_entries.push((path, name));
            }
        }
    }

    // Sort alphabetically
    dir_entries.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    file_entries.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    // Add directories first
    for (path, name) in dir_entries {
        let is_expanded = expanded_paths.contains(&path);
        nodes.push(ExplorerNode {
            path: path.clone(),
            name,
            is_dir: true,
            depth,
            is_expanded,
        });

        if is_expanded {
            build_tree(&path, depth + 1, expanded_paths, nodes);
        }
    }

    // Add files
    for (path, name) in file_entries {
        nodes.push(ExplorerNode {
            path,
            name,
            is_dir: false,
            depth,
            is_expanded: false,
        });
    }
}

fn detect_language(path: &std::path::Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("rs") => "rust",
        Some("toml") => "toml",
        Some("md") => "markdown",
        Some("json") => "json",
        Some("js") => "javascript",
        Some("ts") => "typescript",
        Some("html") => "html",
        Some("css") => "css",
        Some("yaml") | Some("yml") => "yaml",
        Some("py") => "python",
        Some("sh") => "shell",
        _ => "text",
    }
}

fn render_explorer(
    tab_id: usize,
    dashboard_id: usize,
    panel_id: usize,
    dashboard_cwd: PathBuf,
    terminal_cwds: &HashMap<usize, PathBuf>,
    expanded_paths: &HashMap<usize, std::collections::HashSet<PathBuf>>,
    layout_settings: &LayoutSettings,
    _window: &mut Window,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let theme = cx.theme();
    let root_path = terminal_cwds
        .get(&tab_id)
        .cloned()
        .unwrap_or(dashboard_cwd);

    let root_name = root_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root_path.to_string_lossy().to_string())
        .to_uppercase();

    let default_expanded = std::collections::HashSet::new();
    let tab_expanded = expanded_paths.get(&tab_id).unwrap_or(&default_expanded);

    let mut nodes = Vec::new();
    build_tree(&root_path, 0, tab_expanded, &mut nodes);

    let items: Vec<AnyElement> = nodes
        .into_iter()
        .enumerate()
        .map(|(idx, node)| {
            let depth = node.depth;
            let path_for_click = node.path.clone();

            let chevron = if node.is_dir {
                let icon_name = if node.is_expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                };
                div()
                    .w(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Icon::new(icon_name).size_3().text_color(theme.muted_foreground))
                    .into_any_element()
            } else {
                div().w(px(14.)).into_any_element()
            };

            let item_icon = if node.is_dir {
                let icon_name = if node.is_expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                };
                Icon::new(icon_name)
                    .size_3p5()
                    .text_color(theme.accent)
            } else {
                Icon::new(IconName::File)
                    .size_3p5()
                    .text_color(theme.muted_foreground)
            };

            let on_click_handler = cx.listener(move |this, _: &ClickEvent, window, cx| {
                if node.is_dir {
                    this.toggle_explorer_dir(tab_id, path_for_click.clone(), cx);
                } else {
                    this.open_explorer_file(
                        dashboard_id,
                        panel_id,
                        tab_id,
                        path_for_click.clone(),
                        window,
                        cx,
                    );
                }
            });

            div()
                .id(ElementId::Integer(4_000_000 + tab_id as u64 * 10_000 + idx as u64))
                .w_full()
                .h(px(22.))
                .pl(px(8. + depth as f32 * 12.))
                .flex()
                .flex_row()
                .items_center()
                .cursor_pointer()
                .hover(move |s| s.bg(theme.accent).text_color(theme.foreground))
                .on_click(on_click_handler)
                .child(chevron)
                .child(item_icon)
                .child(div().w(px(6.)))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .text_xs()
                        .font_medium()
                        .child(node.name),
                )
                .into_any_element()
        })
        .collect();

    div()
        .w_full()
        .h_full()
        .bg(theme.background)
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .h(px(layout_settings.panel_tab_height))
                .px_3()
                .bg(theme.secondary)
                .border_b_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .font_bold()
                        .text_color(theme.muted_foreground)
                        .child(root_name),
                ),
        )
        .child(
            div()
                .id(ElementId::Integer(4_200_000 + tab_id as u64))
                .flex_1()
                .overflow_y_scroll()
                .py_1()
                .children(items),
        )
        .into_any_element()
}

fn render_panel_content(
    id: usize,
    content: PanelContent,
    dashboard_id: usize,
    panel_id: usize,
    dashboard_cwd: PathBuf,
    terminals: &HashMap<usize, Entity<TerminalModel>>,
    editors: &HashMap<usize, Entity<InputState>>,
    browsers: &HashMap<usize, BrowserState>,
    terminal_cwds: &HashMap<usize, PathBuf>,
    expanded_paths: &HashMap<usize, std::collections::HashSet<PathBuf>>,
    git_diffs: &HashMap<usize, GitDiffState>,
    git_tree_view: &HashMap<usize, bool>,
    git_diff_side_by_side: &HashMap<usize, bool>,
    git_diff_wrap: &HashMap<usize, bool>,
    git_collapsed_paths: &HashMap<usize, std::collections::HashSet<PathBuf>>,
    git_diff_scroll_handles: &std::cell::RefCell<HashMap<usize, UniformListScrollHandle>>,
    git_diff_div_scroll_handles: &std::cell::RefCell<HashMap<usize, gpui::ScrollHandle>>,
    terminal_settings: &TerminalSettings,
    layout_settings: &LayoutSettings,
    window: &mut Window,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    match content {
        PanelContent::Terminal => {
            if let Some(term) = terminals.get(&id) {
                render_terminal(id, term, terminal_settings, window, cx)
            } else {
                div().flex_1().child("No terminal").into_any_element()
            }
        }
        PanelContent::FileExplorer => {
            render_explorer(
                id,
                dashboard_id,
                panel_id,
                dashboard_cwd,
                terminal_cwds,
                expanded_paths,
                layout_settings,
                window,
                cx,
            )
        }
        PanelContent::Git => {
            render_git(
                id,
                dashboard_id,
                panel_id,
                dashboard_cwd,
                terminal_cwds,
                git_diffs,
                git_tree_view,
                git_diff_side_by_side,
                git_collapsed_paths,
                layout_settings,
                window,
                cx,
            )
        }
        PanelContent::Editor { path, is_diff, status } => {
            if let Some(editor) = editors.get(&id) {
                render_panel_editor(id, &path, editor, is_diff, status.as_deref(), git_diff_side_by_side, git_diff_wrap, git_diff_scroll_handles, git_diff_div_scroll_handles, layout_settings, window, cx)
            } else {
                div().flex_1().child("No editor").into_any_element()
            }
        }
        PanelContent::Browser { .. } => {
            if let Some(state) = browsers.get(&id) {
                let theme = cx.theme();
                let handle_opt = state.handle.clone();
                let tab_id = id;

                let toolbar = div()
                    .h(px(32.0))
                    .bg(theme.secondary)
                    .border_b_1()
                    .border_color(theme.border)
                    .flex()
                    .items_center()
                    .px_2()
                    .gap_2()
                    .child(
                        browser_nav_button(
                            ElementId::Integer(950_000 + tab_id as u64 * 10 + 1),
                            "←",
                            layout_settings,
                            theme,
                            cx.listener(move |this, _, _, _| {
                                if let Some(ref b) = this.browsers.get(&tab_id) {
                                    if let Some(ref h) = b.handle {
                                        h.go_back();
                                    }
                                }
                            })
                        )
                    )
                    .child(
                        browser_nav_button(
                            ElementId::Integer(950_000 + tab_id as u64 * 10 + 2),
                            "→",
                            layout_settings,
                            theme,
                            cx.listener(move |this, _, _, _| {
                                if let Some(ref b) = this.browsers.get(&tab_id) {
                                    if let Some(ref h) = b.handle {
                                        h.go_forward();
                                    }
                                }
                            })
                        )
                    )
                    .child(
                        browser_nav_button(
                            ElementId::Integer(950_000 + tab_id as u64 * 10 + 3),
                            "↻",
                            layout_settings,
                            theme,
                            cx.listener(move |this, _, _, _| {
                                if let Some(ref b) = this.browsers.get(&tab_id) {
                                    if let Some(ref h) = b.handle {
                                        h.reload();
                                    }
                                }
                            })
                        )
                    )
                    .child(
                        div()
                            .flex_1()
                            .h(px(24.0))
                            .on_mouse_down(MouseButton::Left, {
                                let focus_handle = state.url_editor.focus_handle(cx);
                                move |_, window, cx| {
                                    window.focus(&focus_handle, cx);
                                    crate::browser::restore_gpui_focus(window);
                                    cx.stop_propagation();
                                }
                            })
                            .child(
                                Input::new(&state.url_editor)
                                    .h_full()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(theme.mono_font_size)
                                    .font_normal()
                            )
                    );

                let body = div()
                    .flex_1()
                    .child(
                        canvas(
                            move |bounds, _, _| bounds,
                            move |bounds, _prepaint_val, window, _cx| {
                                if let Some(ref h) = handle_opt {
                                    h.set_bounds(window, bounds);
                                    h.set_visible(true);
                                }
                            }
                        )
                        .size_full()
                    );

                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(toolbar)
                    .child(body)
                    .into_any_element()
            } else {
                div().flex_1().child("No browser state").into_any_element()
            }
        }
    }
}

fn render_git(
    tab_id: usize,
    _dashboard_id: usize,
    _panel_id: usize,
    dashboard_cwd: PathBuf,
    terminal_cwds: &HashMap<usize, PathBuf>,
    git_diffs: &HashMap<usize, GitDiffState>,
    git_tree_view: &HashMap<usize, bool>,
    _git_diff_side_by_side: &HashMap<usize, bool>,
    git_collapsed_paths: &HashMap<usize, std::collections::HashSet<PathBuf>>,
    layout_settings: &LayoutSettings,
    _window: &mut Window,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let theme = cx.theme();
    let root_path = terminal_cwds
        .get(&tab_id)
        .cloned()
        .unwrap_or(dashboard_cwd);

    let root_name = root_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| root_path.to_string_lossy().to_string())
        .to_uppercase();

    let (branch, files, error) = if let Some(state) = git_diffs.get(&tab_id) {
        (
            state.branch.clone(),
            state.files.clone(),
            state.error.clone(),
        )
    } else {
        (
            String::new(),
            Vec::new(),
            Some("Loading git diff...".to_string()),
        )
    };

    let is_tree_view = *git_tree_view.get(&tab_id).unwrap_or(&false);

    let body = if let Some(err) = error {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.muted_foreground)
            .text_xs()
            .child(err)
            .into_any_element()
    } else {
        let mut changes_section = Vec::new();
        
        if files.is_empty() {
            changes_section.push(
                div()
                    .w_full()
                    .py_4()
                    .px_3()
                    .flex()
                    .justify_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No changes")
                    .into_any_element()
            );
        } else if is_tree_view {
            // Changed files list header (Tree mode)
            changes_section.push(
                div()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(theme.secondary)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(theme.muted_foreground)
                            .child("CHANGED FILES")
                    )
                    .into_any_element()
            );

            // Construct tree hierarchy from changes
            let mut root_node = GitTreeBuilderNode {
                name: "".to_string(),
                relative_path: PathBuf::new(),
                is_dir: true,
                status: None,
                children: std::collections::BTreeMap::new(),
            };

            for f in &files {
                let path_buf = PathBuf::from(&f.path);
                let mut current = &mut root_node;
                let mut accumulated_path = PathBuf::new();
                
                let components: Vec<_> = path_buf.components().collect();
                let total_components = components.len();
                
                for (i, comp) in components.iter().enumerate() {
                    let name = comp.as_os_str().to_string_lossy().to_string();
                    accumulated_path.push(&name);
                    
                    let is_last = i == total_components - 1;
                    
                    current = current.children.entry(name.clone()).or_insert_with(|| {
                        GitTreeBuilderNode {
                            name,
                            relative_path: accumulated_path.clone(),
                            is_dir: !is_last,
                            status: if is_last { Some(f.status.clone()) } else { None },
                            children: std::collections::BTreeMap::new(),
                        }
                    });
                    
                    if !is_last {
                        current.is_dir = true;
                    }
                }
            }

            let default_collapsed = std::collections::HashSet::new();
            let tab_collapsed = git_collapsed_paths.get(&tab_id).unwrap_or(&default_collapsed);

            let mut tree_nodes = Vec::new();
            flatten_git_tree(&root_node, 0, tab_collapsed, &root_path, &mut tree_nodes);

            for (idx, node) in tree_nodes.into_iter().enumerate() {
                let depth = node.depth;
                let path_for_click = node.path.clone();
                let is_dir = node.is_dir;

                let chevron = if is_dir {
                    let icon_name = if node.is_expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    };
                    div()
                        .w(px(14.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(Icon::new(icon_name).size_3().text_color(theme.muted_foreground))
                        .into_any_element()
                } else {
                    div().w(px(14.)).into_any_element()
                };

                let item_icon = if is_dir {
                    let icon_name = if node.is_expanded {
                        IconName::FolderOpen
                    } else {
                        IconName::Folder
                    };
                    Icon::new(icon_name)
                        .size_3p5()
                        .text_color(theme.accent)
                } else {
                    Icon::new(IconName::File)
                        .size_3p5()
                        .text_color(theme.muted_foreground)
                };

                let badge = if !is_dir {
                    if let Some(ref status) = node.status {
                        let badge_color: Hsla = match status.as_str() {
                            "M" => rgb(0xcca700).into(), // modified
                            "A" => rgb(0x57c994).into(), // added
                            "D" => rgb(0xf47067).into(), // deleted
                            "??" => theme.muted_foreground.into(), // untracked
                            _ => theme.foreground.into(),
                        };
                        div()
                            .w(px(20.))
                            .text_color(badge_color)
                            .font_bold()
                            .text_xs()
                            .child(status.clone())
                    } else {
                        div().w(px(20.))
                    }
                } else {
                    div().w(px(20.))
                };

                let on_click_handler = cx.listener({
                    let path = path_for_click.clone();
                    let status = node.status.clone().unwrap_or_default();
                    move |this, _: &ClickEvent, window, cx| {
                        if is_dir {
                            this.toggle_git_dir(tab_id, path.clone(), cx);
                        } else {
                            this.open_git_file_diff(path.clone(), status.clone(), tab_id, window, cx);
                        }
                    }
                });

                changes_section.push(
                    div()
                        .id(ElementId::Integer(5_500_000 + tab_id as u64 * 10_000 + idx as u64))
                        .w_full()
                        .h(px(22.))
                        .pl(px(8. + depth as f32 * 12.))
                        .flex()
                        .flex_row()
                        .items_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(theme.accent.opacity(0.1)).text_color(theme.foreground))
                        .on_click(on_click_handler)
                        .child(chevron)
                        .child(badge)
                        .child(div().w(px(4.)))
                        .child(item_icon)
                        .child(div().w(px(6.)))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_color(theme.foreground)
                                .font_medium()
                                .text_xs()
                                .child(node.name),
                        )
                        .into_any_element()
                );
            }
        } else {
            // Changed files list header (Flat mode)
            changes_section.push(
                div()
                    .w_full()
                    .px_3()
                    .py_1()
                    .bg(theme.secondary)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_xs()
                            .font_bold()
                            .text_color(theme.muted_foreground)
                            .child("CHANGED FILES")
                    )
                    .into_any_element()
            );

            // File items
            for (idx, f) in files.iter().enumerate() {
                let badge_color: Hsla = match f.status.as_str() {
                    "M" => rgb(0xcca700).into(), // modified (gold/yellow)
                    "A" => rgb(0x57c994).into(), // added (green)
                    "D" => rgb(0xf47067).into(), // deleted (red)
                    "??" => theme.muted_foreground.into(), // untracked (gray)
                    _ => theme.foreground.into(),
                };

                let on_click_handler = cx.listener({
                    let path = root_path.join(&f.path);
                    let status = f.status.clone();
                    move |this, _: &ClickEvent, window, cx| {
                        this.open_git_file_diff(path.clone(), status.clone(), tab_id, window, cx);
                    }
                });

                changes_section.push(
                    div()
                        .id(ElementId::Integer(5_000_000 + tab_id as u64 * 10_000 + idx as u64))
                        .w_full()
                        .h(px(22.))
                        .px_3()
                        .flex()
                        .flex_row()
                        .items_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(theme.accent.opacity(0.1)).text_color(theme.foreground))
                        .on_click(on_click_handler)
                        .child(
                            div()
                                .w(px(20.))
                                .text_color(badge_color)
                                .font_bold()
                                .text_xs()
                                .child(f.status.clone())
                        )
                        .child(div().w(px(8.)))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .text_color(theme.foreground)
                                .font_medium()
                                .text_xs()
                                .child(f.path.clone())
                        )
                        .into_any_element()
                );
            }
        }

        div()
            .id(ElementId::Integer(8_000_000 + tab_id as u64))
            .flex_1()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .children(changes_section)
            .into_any_element()
    };

    let refresh_btn = action_icon_button(
        ElementId::Integer(5_000_000 + tab_id as u64),
        IconName::LoaderCircle,
        false,
        layout_settings,
        theme,
        cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.refresh_git_diff(tab_id, cx);
            cx.notify();
        }),
    );

    let flat_btn = div()
        .id(ElementId::Integer(5_100_000 + tab_id as u64))
        .h(px(layout_settings.icon_button_height))
        .px_2()
        .rounded_sm()
        .bg(if !is_tree_view { theme.accent.opacity(0.15) } else { theme.background })
        .border_1()
        .border_color(if !is_tree_view { theme.accent } else { theme.border })
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_medium()
        .text_color(if !is_tree_view { theme.accent } else { theme.muted_foreground })
        .cursor_pointer()
        .hover(move |s| s.text_color(theme.foreground))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.git_tree_view.insert(tab_id, false);
            cx.notify();
        }))
        .child("Flat");

    let tree_btn = div()
        .id(ElementId::Integer(5_200_000 + tab_id as u64))
        .h(px(layout_settings.icon_button_height))
        .px_2()
        .rounded_sm()
        .bg(if is_tree_view { theme.accent.opacity(0.15) } else { theme.background })
        .border_1()
        .border_color(if is_tree_view { theme.accent } else { theme.border })
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_medium()
        .text_color(if is_tree_view { theme.accent } else { theme.muted_foreground })
        .cursor_pointer()
        .hover(move |s| s.text_color(theme.foreground))
        .on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
            this.git_tree_view.insert(tab_id, true);
            cx.notify();
        }))
        .child("Tree");

    div()
        .w_full()
        .h_full()
        .bg(theme.background)
        .flex()
        .flex_col()
        .child(
            div()
                .w_full()
                .h(px(layout_settings.panel_tab_height))
                .px_3()
                .bg(theme.secondary)
                .border_b_1()
                .border_color(theme.border)
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_bold()
                                .text_color(theme.accent)
                                .child(format!("GIT: {}", root_name))
                        )
                        .child(
                            if !branch.is_empty() {
                                div()
                                    .ml_2()
                                    .px_1()
                                    .rounded_sm()
                                    .bg(theme.muted)
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("branch: {}", branch))
                            } else {
                                div()
                            }
                        )
                )
                .child({
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(flat_btn)
                        .child(tree_btn)
                        .child(refresh_btn)
                }),
        )
        .child(body)
        .into_any_element()
}

// --- Terminal Renderer ---

fn is_terminal_copy_shortcut(keystroke: &Keystroke) -> bool {
    keystroke.key.eq_ignore_ascii_case("c")
        && ((keystroke.modifiers.platform && !keystroke.modifiers.control && !keystroke.modifiers.alt)
            || (keystroke.modifiers.control
                && keystroke.modifiers.shift
                && !keystroke.modifiers.alt
                && !keystroke.modifiers.platform))
}

fn terminal_rows_to_text(rows: &[Vec<(char, u32, u32)>]) -> String {
    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let mut line: String = row.iter().map(|(ch, _, _)| *ch).collect();
        while line.ends_with(' ') {
            line.pop();
        }
        lines.push(line);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

fn terminal_position_to_cell(
    position: Point<Pixels>,
    bounds: Option<Bounds<Pixels>>,
    line_height: f32,
    char_width: f32,
    rows: usize,
    cols: usize,
    clamp: bool,
) -> Option<(usize, usize)> {
    let bounds = bounds?;
    if rows == 0 || cols == 0 || line_height <= 0.0 || char_width <= 0.0 {
        return None;
    }

    let mut x = (position.x - bounds.origin.x).as_f32();
    let mut y = (position.y - bounds.origin.y).as_f32();
    let max_x = (bounds.size.width.as_f32() - 1.0).max(0.0);
    let max_y = (bounds.size.height.as_f32() - 1.0).max(0.0);

    if clamp {
        x = x.clamp(0.0, max_x);
        y = y.clamp(0.0, max_y);
    } else if x < 0.0 || y < 0.0 || x > max_x || y > max_y {
        return None;
    }

    let row = ((y / line_height).floor() as usize).min(rows.saturating_sub(1));
    let col = ((x / char_width).floor() as usize).min(cols.saturating_sub(1));
    Some((row, col))
}

fn is_selected_cell(
    row_idx: usize,
    col_idx: usize,
    selection: Option<((usize, usize), (usize, usize))>,
) -> bool {
    let Some(((start_row, start_col), (end_row, end_col))) = selection else {
        return false;
    };
    if row_idx < start_row || row_idx > end_row {
        return false;
    }
    if start_row == end_row {
        return row_idx == start_row && col_idx >= start_col && col_idx <= end_col;
    }
    if row_idx == start_row {
        return col_idx >= start_col;
    }
    if row_idx == end_row {
        return col_idx <= end_col;
    }
    true
}

fn render_terminal(
    id: usize,
    term: &Entity<TerminalModel>,
    terminal_settings: &TerminalSettings,
    _window: &mut Window,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let focus_handle = term.read(cx).focus_handle.clone();
    let is_focused = focus_handle.contains_focused(_window, cx);
    let (row_data, _cursor, term_bg) = term.update(cx, |m, _| m.collect_rows(is_focused));

    let model = term.read(cx);
    let scroll_handle = model.scroll_handle.clone();
    let selection = model.selection_range();

    let item_count = row_data.len();
    let row_data = Arc::new(row_data);
    let row_data_for_list = Arc::clone(&row_data);
    let row_data_for_copy = Arc::clone(&row_data);

    let term_entity_copy = term.clone();
    let term_entity = term.clone();
    let term_entity_scroll = term.clone();
    let term_entity_tab = term.clone();
    let term_entity_shift_tab = term.clone();
    let term_entity_select_start = term.clone();
    let term_entity_select_move = term.clone();
    let term_entity_select_end = term.clone();
    let term_entity_select_end_out = term.clone();
    let term_entity_click = term.clone();
    let fh_click = focus_handle.clone();
    let fh_click_mouse_down = focus_handle.clone();
    let term_font_family = terminal_settings.font_family.clone();
    let term_font_size = terminal_settings.font_size;
    let term_line_height = terminal_settings.line_height;
    let term_char_width = terminal_settings.char_width;
    let resize_debounce_ms = terminal_settings.resize_debounce_ms;

    let list = uniform_list(
        ElementId::Integer(id as u64 * 100 + 52),
        item_count,
        move |range, _window, _cx| {
            range
                .map(|i| {
                    render_term_row(
                        i,
                        &row_data_for_list[i],
                        &term_font_family,
                        term_font_size,
                        term_line_height,
                        selection,
                    )
                })
                .collect::<Vec<AnyElement>>()
        },
    )
    .size_full()
    .bg(rgb(term_bg))
    .track_scroll(&scroll_handle);

    let inner = div()
        .id(ElementId::Integer(id as u64 * 100 + 51))
        .size_full()
        .key_context("Terminal")
        .track_focus(&focus_handle)
        .on_click(move |_, window, cx| {
            window.focus(&fh_click, cx);
            crate::browser::restore_gpui_focus(window);
            term_entity_click.update(cx, |m, _| {
                m.needs_attention = false;
                m.job_done = false;
            });
        })
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            window.focus(&fh_click_mouse_down, cx);
            crate::browser::restore_gpui_focus(window);
            term_entity_select_start.update(cx, |m, inner_cx| {
                m.needs_attention = false;
                m.job_done = false;
                if let Some((row, col)) = terminal_position_to_cell(
                    event.position,
                    m.viewport_bounds.clone(),
                    term_line_height,
                    term_char_width,
                    m.rows,
                    m.cols,
                    true,
                ) {
                    m.begin_selection(row, col);
                }
                inner_cx.notify();
            });
            cx.stop_propagation();
        })
        .on_mouse_move(move |event, _window, cx| {
            if !event.dragging() {
                return;
            }
            term_entity_select_move.update(cx, |m, inner_cx| {
                if let Some((row, col)) = terminal_position_to_cell(
                    event.position,
                    m.viewport_bounds.clone(),
                    term_line_height,
                    term_char_width,
                    m.rows,
                    m.cols,
                    true,
                ) {
                    m.update_selection(row, col);
                    inner_cx.notify();
                }
            });
        })
        .on_mouse_up(MouseButton::Left, move |event, _window, cx| {
            term_entity_select_end.update(cx, |m, inner_cx| {
                if let Some((row, col)) = terminal_position_to_cell(
                    event.position,
                    m.viewport_bounds.clone(),
                    term_line_height,
                    term_char_width,
                    m.rows,
                    m.cols,
                    true,
                ) {
                    m.update_selection(row, col);
                }
                m.end_selection();
                inner_cx.notify();
            });
        })
        .on_mouse_up_out(MouseButton::Left, move |event, _window, cx| {
            term_entity_select_end_out.update(cx, |m, inner_cx| {
                if let Some((row, col)) = terminal_position_to_cell(
                    event.position,
                    m.viewport_bounds.clone(),
                    term_line_height,
                    term_char_width,
                    m.rows,
                    m.cols,
                    true,
                ) {
                    m.update_selection(row, col);
                }
                m.end_selection();
                inner_cx.notify();
            });
        })
        .on_action(move |_: &TerminalTab, _window, cx| {
            term_entity_tab.update(cx, |m, _| {
                m.send_tab();
            });
        })
        .on_action(move |_: &TerminalShiftTab, _window, cx| {
            term_entity_shift_tab.update(cx, |m, cx| {
                m.send_key(b"\x1b[Z");
                cx.notify();
            });
        })
        .on_scroll_wheel(move |event, _window, cx| {
            let lines = match event.delta {
                ScrollDelta::Lines(p) => p.y,
                ScrollDelta::Pixels(p) => p.y.as_f32() / term_line_height,
            };
            let delta = -(lines.round() as isize);
            if delta != 0 {
                term_entity_scroll.update(cx, |m, inner_cx| {
                    m.scroll_by_lines(delta);
                    inner_cx.notify();
                });
            }
        })
        .on_key_down(move |event, _window, cx| {
            let ks = event.keystroke.clone();
            if is_terminal_copy_shortcut(&ks) {
                let text = term_entity_copy
                    .update(cx, |m, _| m.selected_text(row_data_for_copy.as_slice()))
                    .filter(|text| !text.is_empty())
                    .unwrap_or_else(|| terminal_rows_to_text(row_data_for_copy.as_slice()));
                if !text.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                cx.stop_propagation();
                return;
            }
            term_entity.update(cx, |m, _| {
                let bytes = m.encode_keystroke(&ks);
                if !bytes.is_empty() {
                    m.send_key(&bytes);
                }
            });
        })
        .child(list);

    let term_for_resize = term.clone();
    let terminal_display = div()
        .on_children_prepainted(move |child_bounds, _window, cx| {
            if let Some(b) = child_bounds.first() {
                let new_rows = ((b.size.height.as_f32() / term_line_height) as usize).max(10);
                let new_cols = ((b.size.width.as_f32() / term_char_width) as usize).max(20);
                let maybe_gen = term_for_resize.update(cx, |m, _| {
                    m.set_viewport_bounds(b.clone());
                    m.set_pending_resize(new_rows, new_cols)
                });
                if let Some(gen) = maybe_gen {
                    let term_debounce = term_for_resize.clone();
                    cx.spawn(async move |cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(resize_debounce_ms))
                            .await;
                        cx.update(|cx| {
                            term_debounce.update(cx, |m, _| m.apply_pending_resize(gen));
                        });
                    })
                    .detach();
                }
            }
        })
        .id(ElementId::Integer(id as u64 * 100 + 50))
        .flex_1()
        .min_h_0()
        .relative()
        .child(inner);

    div()
        .id(ElementId::Integer(id as u64 * 100 + 59))
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .child(terminal_display)
        .into_any_element()
}

fn render_term_row(
    row_idx: usize,
    row: &[(char, u32, u32)],
    term_font_family: &str,
    term_font_size: f32,
    term_line_height: f32,
    selection: Option<((usize, usize), (usize, usize))>,
) -> AnyElement {
    if row.is_empty() {
        return div().h(px(term_line_height)).into_any_element();
    }
    let mut runs: Vec<(String, u32, u32)> = vec![];
    let mut txt = String::new();
    let mut r_fg = row[0].1;
    let mut r_bg = row[0].2;
    for (col_idx, &(ch, fg, bg)) in row.iter().enumerate() {
        let (fg, bg) = if is_selected_cell(row_idx, col_idx, selection) {
            (bg, fg)
        } else {
            (fg, bg)
        };
        if fg == r_fg && bg == r_bg {
            txt.push(ch);
        } else {
            if !txt.is_empty() {
                runs.push((txt.clone(), r_fg, r_bg));
                txt.clear();
            }
            txt.push(ch);
            r_fg = fg;
            r_bg = bg;
        }
    }
    if !txt.is_empty() {
        runs.push((txt, r_fg, r_bg));
    }
    div()
        .flex()
        .flex_row()
        .h(px(term_line_height))
        .children(runs.into_iter().map(|(text, fg, bg)| {
            div()
                .font_family(term_font_family)
                .text_size(px(term_font_size))
                .text_color(rgb(fg))
                .bg(rgb(bg))
                .child(text)
                .into_any_element()
        }))
        .into_any_element()
}

// --- Editor Renderer ---



fn render_modal_editor(
    path: &PathBuf,
    editor: &Entity<InputState>,
    is_diff: bool,
    view: &DashboardView,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let theme = cx.theme();
    let settings = &view.settings.layout;

    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "Unknown File".to_string());

    let relative_path = path.to_string_lossy().to_string();

    let title = if is_diff {
        format!("Diff: {}", file_name)
    } else {
        file_name
    };

    // Dark transparent backdrop
    div()
        .id(ElementId::Integer(999_000))
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(gpui::rgba(0x00000077))
        .flex()
        .items_center()
        .justify_center()
        .p_12()
        .on_click(cx.listener(|this, _: &ClickEvent, _window, cx| {
            this.close_modal_editor(cx);
        }))
        .on_scroll_wheel(|_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            // The Modal Dialog Box
            div()
                .id(ElementId::Integer(999_010))
                .w_full()
                .max_w(if is_diff { px(1200.) } else { px(900.) })
                .h(gpui::relative(0.8))
                .flex()
                .flex_col()
                .rounded_md()
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .shadow_lg()
                .on_click(|_, _, cx| {
                    cx.stop_propagation();
                })
                .on_mouse_down(MouseButton::Left, {
                    let editor = editor.clone();
                    move |_, window, cx| {
                        cx.stop_propagation();
                        editor.update(cx, |e, cx| {
                            e.focus(window, cx);
                        });
                    }
                })
                .child(
                    // Modal Header
                    div()
                        .h(px(settings.panel_header_height))
                        .px_4()
                        .bg(theme.secondary)
                        .border_b_1()
                        .border_color(theme.border)
                        .rounded_t_md()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::File)
                                        .size_3p5()
                                        .text_color(theme.accent)
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .font_bold()
                                        .text_color(theme.foreground)
                                        .child(title)
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("({})", relative_path))
                                )
                        )
                        .child({
                            let mut right_container = div()
                                .flex()
                                .items_center()
                                .gap_2();
                            if !is_diff {
                                right_container = right_container.child(
                                    sidebar_text_button(
                                        ElementId::Integer(999_001),
                                        "Save",
                                        false,
                                        theme,
                                        cx.listener({
                                            let path = path.clone();
                                            let editor = editor.clone();
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.save_modal_file(&path, &editor, window, cx);
                                            }
                                        })
                                    )
                                );
                            } else {
                                let side_by_side = if let Some(ref modal) = view.modal_editor {
                                    modal.side_by_side
                                } else {
                                    false
                                };
                                right_container = right_container
                                    .child(
                                        sidebar_text_button(
                                            ElementId::Integer(999_003),
                                            "Inline",
                                            !side_by_side,
                                            theme,
                                            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                                if let Some(ref mut modal) = this.modal_editor {
                                                    modal.side_by_side = false;
                                                }
                                                cx.notify();
                                            })
                                        )
                                    )
                                    .child(
                                        sidebar_text_button(
                                            ElementId::Integer(999_004),
                                            "Side-by-Side",
                                            side_by_side,
                                            theme,
                                            cx.listener(move |this, _: &ClickEvent, _window, cx| {
                                                if let Some(ref mut modal) = this.modal_editor {
                                                    modal.side_by_side = true;
                                                }
                                                cx.notify();
                                            })
                                        )
                                    );
                            }
                            right_container.child(
                                // Close Button
                                action_icon_button(
                                    ElementId::Integer(999_002),
                                    IconName::Close,
                                    true,
                                    settings,
                                    theme,
                                    cx.listener(|this, _: &ClickEvent, _window, cx| {
                                        this.close_modal_editor(cx);
                                    })
                                )
                            )
                        })
                )
                .child(
                    // Editor Container
                    if is_diff {
                        let content = editor.read(cx).text().to_string();
                        let side_by_side = if let Some(ref modal) = view.modal_editor {
                            modal.side_by_side
                        } else {
                            false
                        };

                        let scroll_handle = view
                            .modal_editor
                            .as_ref()
                            .map(|m| m.scroll_handle.clone())
                            .unwrap_or_else(UniformListScrollHandle::new);

                        let list_element = if side_by_side {
                            let sbs_lines = parse_side_by_side_diff(&content);
                            let item_count = sbs_lines.len();
                            let sbs_lines = Arc::new(sbs_lines);

                            uniform_list(
                                ElementId::Integer(999_400),
                                item_count,
                                move |range, _window, cx| {
                                    let theme = cx.theme();
                                    range
                                        .map(|i| {
                                            render_side_by_side_line(
                                                i,
                                                999_500,
                                                sbs_lines[i].clone(),
                                                theme,
                                                false,
                                            )
                                        })
                                        .collect::<Vec<AnyElement>>()
                                },
                            )
                            .size_full()
                            .track_scroll(&scroll_handle)
                        } else {
                            let inline_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
                            let item_count = inline_lines.len();
                            let inline_lines = Arc::new(inline_lines);

                            uniform_list(
                                ElementId::Integer(999_400),
                                item_count,
                                move |range, _window, cx| {
                                    let theme = cx.theme();
                                    range
                                        .map(|i| {
                                            render_inline_diff_line(
                                                i,
                                                999_500,
                                                inline_lines[i].clone(),
                                                theme,
                                                false,
                                            )
                                        })
                                        .collect::<Vec<AnyElement>>()
                                },
                            )
                            .size_full()
                            .track_scroll(&scroll_handle)
                        };

                        div()
                            .flex_1()
                            .p_2()
                            .overflow_hidden()
                            .child(list_element)
                    } else {
                        div()
                            .flex_1()
                            .p_2()
                            .overflow_hidden()
                            .child(
                                Input::new(editor)
                                    .h_full()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(theme.mono_font_size)
                                    .font_normal(),
                            )
                    }
                )
        )
        .into_any_element()
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    if !line.starts_with("@@") {
        return None;
    }
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let left_part = parts[1].strip_prefix('-')?;
    let right_part = parts[2].strip_prefix('+')?;

    let left_start = left_part.split(',').next()?.parse::<usize>().ok()?;
    let right_start = right_part.split(',').next()?.parse::<usize>().ok()?;

    Some((left_start, right_start))
}

fn parse_side_by_side_diff(diff_content: &str) -> Vec<SideBySideLine> {
    let mut result = Vec::new();
    let mut left_line_num = 0;
    let mut right_line_num = 0;

    let lines = diff_content.lines().collect::<Vec<_>>();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("diff --git") || line.starts_with("index ") || line.starts_with("--- ") || line.starts_with("+++ ") {
            result.push(SideBySideLine {
                left_line_num: None,
                left_text: line.to_string(),
                left_class: DiffLineClass::Header,
                right_line_num: None,
                right_text: String::new(),
                right_class: DiffLineClass::Empty,
            });
            i += 1;
        } else if line.starts_with("@@") {
            if let Some((left_start, right_start)) = parse_hunk_header(line) {
                left_line_num = left_start;
                right_line_num = right_start;
            }
            result.push(SideBySideLine {
                left_line_num: None,
                left_text: line.to_string(),
                left_class: DiffLineClass::Header,
                right_line_num: None,
                right_text: String::new(),
                right_class: DiffLineClass::Empty,
            });
            i += 1;
        } else {
            let mut deletes = Vec::new();
            let mut adds = Vec::new();

            while i < lines.len() {
                let current_line = lines[i];
                if current_line.starts_with('-') && !current_line.starts_with("---") {
                    deletes.push(current_line);
                    i += 1;
                } else if current_line.starts_with('+') && !current_line.starts_with("+++") {
                    adds.push(current_line);
                    i += 1;
                } else {
                    break;
                }
            }

            if !deletes.is_empty() || !adds.is_empty() {
                let max_len = std::cmp::max(deletes.len(), adds.len());
                for idx in 0..max_len {
                    let del_opt = deletes.get(idx);
                    let add_opt = adds.get(idx);

                    let (left_num, left_txt, left_cls) = if let Some(del_line) = del_opt {
                        let num = left_line_num;
                        left_line_num += 1;
                        let txt = if del_line.len() > 1 { &del_line[1..] } else { "" };
                        (Some(num), txt.to_string(), DiffLineClass::Deletion)
                    } else {
                        (None, String::new(), DiffLineClass::Empty)
                    };

                    let (right_num, right_txt, right_cls) = if let Some(add_line) = add_opt {
                        let num = right_line_num;
                        right_line_num += 1;
                        let txt = if add_line.len() > 1 { &add_line[1..] } else { "" };
                        (Some(num), txt.to_string(), DiffLineClass::Addition)
                    } else {
                        (None, String::new(), DiffLineClass::Empty)
                    };

                    result.push(SideBySideLine {
                        left_line_num: left_num,
                        left_text: left_txt,
                        left_class: left_cls,
                        right_line_num: right_num,
                        right_text: right_txt,
                        right_class: right_cls,
                    });
                }
            } else {
                let current_line = lines[i];
                if current_line.starts_with('\\') {
                    result.push(SideBySideLine {
                        left_line_num: None,
                        left_text: current_line.to_string(),
                        left_class: DiffLineClass::Header,
                        right_line_num: None,
                        right_text: String::new(),
                        right_class: DiffLineClass::Empty,
                    });
                } else {
                    let left_num = left_line_num;
                    let right_num = right_line_num;
                    left_line_num += 1;
                    right_line_num += 1;
                    let txt = if current_line.starts_with(' ') {
                        if current_line.len() > 1 { &current_line[1..] } else { "" }
                    } else {
                        current_line
                    };
                    result.push(SideBySideLine {
                        left_line_num: Some(left_num),
                        left_text: txt.to_string(),
                        left_class: DiffLineClass::Unchanged,
                        right_line_num: Some(right_num),
                        right_text: txt.to_string(),
                        right_class: DiffLineClass::Unchanged,
                    });
                }
                i += 1;
            }
        }
    }
    result
}

fn render_side_by_side_line(
    idx: usize,
    base_id: u64,
    sline: SideBySideLine,
    theme: &gpui_component::theme::Theme,
    wrap: bool,
) -> AnyElement {
    if sline.left_class == DiffLineClass::Header {
        let line_text = sline.left_text;
        let text_color: Hsla;
        let bg_color: Hsla;
        if line_text.starts_with("@@") {
            text_color = theme.accent;
            bg_color = theme.accent.opacity(0.05);
        } else if line_text.starts_with("diff --git") || line_text.starts_with("index ") {
            text_color = theme.foreground;
            bg_color = theme.muted.opacity(0.3);
        } else {
            text_color = theme.muted_foreground;
            bg_color = theme.background;
        }

        div()
            .id(ElementId::Integer(base_id + idx as u64))
            .w_full()
            .bg(bg_color)
            .px_3()
            .flex()
            .when(wrap, |this| this.items_start())
            .when(!wrap, |this| this.items_center())
            .child(
                div()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .text_color(text_color)
                    .when(wrap, |this| this.whitespace_normal())
                    .when(!wrap, |this| this.whitespace_nowrap())
                    .child(line_text)
            )
            .into_any_element()
    } else {
        let left_text_color = match sline.left_class {
            DiffLineClass::Deletion => rgb(0xf47067).into(),
            DiffLineClass::Empty => theme.muted_foreground,
            _ => theme.foreground,
        };
        let left_bg_color = match sline.left_class {
            DiffLineClass::Deletion => Hsla::from(rgb(0xf47067)).opacity(0.1),
            DiffLineClass::Empty => theme.muted.opacity(0.1),
            _ => theme.background,
        };

        let right_text_color = match sline.right_class {
            DiffLineClass::Addition => rgb(0x57c994).into(),
            DiffLineClass::Empty => theme.muted_foreground,
            _ => theme.foreground,
        };
        let right_bg_color = match sline.right_class {
            DiffLineClass::Addition => Hsla::from(rgb(0x57c994)).opacity(0.1),
            DiffLineClass::Empty => theme.muted.opacity(0.1),
            _ => theme.background,
        };

        let left_line_num_str = sline.left_line_num
            .map(|n| n.to_string())
            .unwrap_or_default();
        let right_line_num_str = sline.right_line_num
            .map(|n| n.to_string())
            .unwrap_or_default();

        div()
            .id(ElementId::Integer(base_id + idx as u64))
            .w_full()
            .flex()
            .flex_row()
            .child(
                div()
                    .flex_1()
                    .bg(left_bg_color)
                    .px_3()
                    .flex()
                    .flex_row()
                    .when(wrap, |this| this.items_start())
                    .when(!wrap, |this| this.items_center())
                    .child(
                        div()
                            .w(px(32.))
                            .flex()
                            .justify_end()
                            .pr_2()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .text_color(theme.muted_foreground.opacity(0.6))
                            .child(left_line_num_str)
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .text_color(left_text_color)
                            .when(wrap, |this| this.whitespace_normal())
                            .when(!wrap, |this| this.whitespace_nowrap())
                            .child(sline.left_text)
                    )
            )
            .child(
                div()
                    .w(px(1.))
                    .bg(theme.border)
            )
            .child(
                div()
                    .flex_1()
                    .bg(right_bg_color)
                    .px_3()
                    .flex()
                    .flex_row()
                    .when(wrap, |this| this.items_start())
                    .when(!wrap, |this| this.items_center())
                    .child(
                        div()
                            .w(px(32.))
                            .flex()
                            .justify_end()
                            .pr_2()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .text_color(theme.muted_foreground.opacity(0.6))
                            .child(right_line_num_str)
                    )
                    .child(
                        div()
                            .flex_1()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(theme.mono_font_size)
                            .text_color(right_text_color)
                            .when(wrap, |this| this.whitespace_normal())
                            .when(!wrap, |this| this.whitespace_nowrap())
                            .child(sline.right_text)
                    )
            )
            .into_any_element()
    }
}

fn render_inline_diff_line(
    idx: usize,
    base_id: u64,
    line: String,
    theme: &gpui_component::theme::Theme,
    wrap: bool,
) -> AnyElement {
    let text_color: Hsla;
    let bg_color: Hsla;

    if line.starts_with('+') && !line.starts_with("+++") {
        text_color = rgb(0x57c994).into();
        bg_color = Hsla::from(rgb(0x57c994)).opacity(0.1);
    } else if line.starts_with('-') && !line.starts_with("---") {
        text_color = rgb(0xf47067).into();
        bg_color = Hsla::from(rgb(0xf47067)).opacity(0.1);
    } else if line.starts_with("@@") {
        text_color = theme.accent;
        bg_color = theme.accent.opacity(0.05);
    } else if line.starts_with("diff --git") || line.starts_with("index ") {
        text_color = theme.foreground;
        bg_color = theme.muted.opacity(0.3);
    } else {
        text_color = theme.muted_foreground;
        bg_color = theme.background;
    }

    div()
        .id(ElementId::Integer(base_id + idx as u64))
        .w_full()
        .bg(bg_color)
        .px_3()
        .flex()
        .when(wrap, |this| this.items_start())
        .when(!wrap, |this| this.items_center())
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.mono_font_size)
                .text_color(text_color)
                .when(wrap, |this| this.whitespace_normal())
                .when(!wrap, |this| this.whitespace_nowrap())
                .child(line)
        )
        .into_any_element()
}

fn render_panel_editor(
    tab_id: usize,
    path: &PathBuf,
    editor: &Entity<InputState>,
    is_diff: bool,
    _status: Option<&str>,
    git_diff_side_by_side: &HashMap<usize, bool>,
    git_diff_wrap: &HashMap<usize, bool>,
    git_diff_scroll_handles: &std::cell::RefCell<HashMap<usize, UniformListScrollHandle>>,
    git_diff_div_scroll_handles: &std::cell::RefCell<HashMap<usize, gpui::ScrollHandle>>,
    layout_settings: &LayoutSettings,
    _window: &mut Window,
    cx: &mut Context<DashboardView>,
) -> AnyElement {
    let theme = cx.theme();
    let relative_path = path.to_string_lossy().to_string();

    let mut toolbar = div()
        .h(px(layout_settings.panel_header_height))
        .px_4()
        .bg(theme.secondary)
        .border_b_1()
        .border_color(theme.border)
        .flex()
        .flex_row()
        .items_center()
        .justify_between();

    toolbar = toolbar.child(
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(Icon::new(IconName::File).size_3p5().text_color(theme.accent))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(relative_path)
            )
    );

    if !is_diff {
        toolbar = toolbar.child(
            sidebar_text_button(
                ElementId::Integer(999_100 + tab_id as u64),
                "Save",
                false,
                theme,
                cx.listener({
                    let path = path.clone();
                    let editor = editor.clone();
                    move |this, _: &ClickEvent, window, cx| {
                        this.save_modal_file(&path, &editor, window, cx);
                    }
                })
            )
        );
    } else {
        let side_by_side = git_diff_side_by_side.get(&tab_id).cloned().unwrap_or(false);
        let wrap = git_diff_wrap.get(&tab_id).cloned().unwrap_or(false);
        
        toolbar = toolbar.child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    sidebar_text_button(
                        ElementId::Integer(999_101 + tab_id as u64),
                        "Inline",
                        !side_by_side,
                        theme,
                        cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.git_diff_side_by_side.insert(tab_id, false);
                            cx.notify();
                        })
                    )
                )
                .child(
                    sidebar_text_button(
                        ElementId::Integer(999_102 + tab_id as u64),
                        "Side-by-Side",
                        side_by_side,
                        theme,
                        cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            this.git_diff_side_by_side.insert(tab_id, true);
                            cx.notify();
                        })
                    )
                )
                .child(
                    div()
                        .w(px(1.))
                        .h(px(14.))
                        .bg(theme.border)
                )
                .child(
                    sidebar_text_button(
                        ElementId::Integer(999_103 + tab_id as u64),
                        "Wrap Text",
                        wrap,
                        theme,
                        cx.listener(move |this, _: &ClickEvent, _window, cx| {
                            let current = this.git_diff_wrap.get(&tab_id).cloned().unwrap_or(false);
                            this.git_diff_wrap.insert(tab_id, !current);
                            cx.notify();
                        })
                    )
                )
        );
    }

    let body = if is_diff {
        let content = editor.read(cx).text().to_string();
        let side_by_side = git_diff_side_by_side.get(&tab_id).cloned().unwrap_or(false);
        let wrap = git_diff_wrap.get(&tab_id).cloned().unwrap_or(false);

        let list_element = if side_by_side {
            let sbs_lines = parse_side_by_side_diff(&content);
            let sbs_lines = Arc::new(sbs_lines);

            if wrap {
                let div_scroll_handle = git_diff_div_scroll_handles
                    .borrow_mut()
                    .entry(tab_id)
                    .or_insert_with(gpui::ScrollHandle::new)
                    .clone();

                div()
                    .id(ElementId::Integer(999_800 + tab_id as u64))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&div_scroll_handle)
                    .child(
                        div()
                            .v_flex()
                            .children(sbs_lines.iter().enumerate().map(|(i, sline)| {
                                render_side_by_side_line(
                                    i,
                                    999_700 + tab_id as u64,
                                    sline.clone(),
                                    theme,
                                    wrap,
                                )
                            }))
                    )
                    .into_any_element()
            } else {
                let scroll_handle = git_diff_scroll_handles
                    .borrow_mut()
                    .entry(tab_id)
                    .or_insert_with(UniformListScrollHandle::new)
                    .clone();
                let item_count = sbs_lines.len();

                uniform_list(
                    ElementId::Integer(999_600 + tab_id as u64),
                    item_count,
                    move |range, _window, cx| {
                        let theme = cx.theme();
                        range
                            .map(|i| {
                                render_side_by_side_line(
                                    i,
                                    999_700 + tab_id as u64,
                                    sbs_lines[i].clone(),
                                    theme,
                                    wrap,
                                )
                            })
                            .collect::<Vec<AnyElement>>()
                    },
                )
                .size_full()
                .track_scroll(&scroll_handle)
                .into_any_element()
            }
        } else {
            let inline_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
            let inline_lines = Arc::new(inline_lines);

            if wrap {
                let div_scroll_handle = git_diff_div_scroll_handles
                    .borrow_mut()
                    .entry(tab_id)
                    .or_insert_with(gpui::ScrollHandle::new)
                    .clone();

                div()
                    .id(ElementId::Integer(999_900 + tab_id as u64))
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&div_scroll_handle)
                    .child(
                        div()
                            .v_flex()
                            .children(inline_lines.iter().enumerate().map(|(i, line)| {
                                render_inline_diff_line(
                                    i,
                                    999_700 + tab_id as u64,
                                    line.clone(),
                                    theme,
                                    wrap,
                                )
                            }))
                    )
                    .into_any_element()
            } else {
                let scroll_handle = git_diff_scroll_handles
                    .borrow_mut()
                    .entry(tab_id)
                    .or_insert_with(UniformListScrollHandle::new)
                    .clone();
                let item_count = inline_lines.len();

                uniform_list(
                    ElementId::Integer(999_600 + tab_id as u64),
                    item_count,
                    move |range, _window, cx| {
                        let theme = cx.theme();
                        range
                            .map(|i| {
                                render_inline_diff_line(
                                    i,
                                    999_700 + tab_id as u64,
                                    inline_lines[i].clone(),
                                    theme,
                                    wrap,
                                )
                            })
                            .collect::<Vec<AnyElement>>()
                    },
                )
                .size_full()
                .track_scroll(&scroll_handle)
                .into_any_element()
            }
        };

        div()
            .flex_1()
            .p_2()
            .overflow_hidden()
            .child(list_element)
    } else {
        div()
            .flex_1()
            .p_2()
            .overflow_hidden()
            .child(
                Input::new(editor)
                    .h_full()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .font_normal(),
            )
    };

    div()
        .size_full()
        .flex()
        .flex_col()
        .child(toolbar)
        .child(body)
        .into_any_element()
}

fn content_title(content: &PanelContent) -> String {
    match content {
        PanelContent::Terminal => "terminal".to_string(),
        PanelContent::FileExplorer => "explorer".to_string(),
        PanelContent::Git => "git".to_string(),
        PanelContent::Browser { .. } => "browser".to_string(),
        PanelContent::Editor { path, is_diff, .. } => {
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "editor".to_string());
            if *is_diff {
                format!("diff: {name}")
            } else {
                name
            }
        }
    }
}

fn dropdown_item(
    eid: ElementId,
    label: &'static str,
    handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .id(eid)
        .px_2()
        .py_1()
        .rounded_sm()
        .flex()
        .items_center()
        .text_color(theme.foreground)
        .hover(|s| s.bg(theme.muted).text_color(theme.accent))
        .on_click(handler)
        .child(label)
}

