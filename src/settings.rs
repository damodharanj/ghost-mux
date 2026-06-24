use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AppSettings {
    pub theme: ThemeSettings,
    pub layout: LayoutSettings,
    pub terminal: TerminalSettings,
    pub lsp: LspSettings,
    pub agents: Vec<AgentConfig>,
    pub server_url: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: ThemeSettings::default(),
            layout: LayoutSettings::default(),
            terminal: TerminalSettings::default(),
            lsp: LspSettings::default(),
            agents: vec![
                AgentConfig {
                    name: "claude".to_string(),
                    config_file: "~/.claude/settings.json".to_string(),
                    hook_type: "nested".to_string(),
                    enabled: true,
                },
                AgentConfig {
                    name: "cursor".to_string(),
                    config_file: "~/.cursor/hooks.json".to_string(),
                    hook_type: "flat".to_string(),
                    enabled: true,
                },
                AgentConfig {
                    name: "gemini".to_string(),
                    config_file: "~/.gemini/settings.json".to_string(),
                    hook_type: "nested".to_string(),
                    enabled: true,
                },
            ],
            server_url: None,
        }
    }
}

impl AppSettings {
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read settings file at {}", path.display()))?;
        let settings: AppSettings =
            serde_yaml::from_str(&raw).context("failed to parse settings yaml")?;
        Ok(settings)
    }

    pub fn save_to_file(&self, path: &Path) -> anyhow::Result<()> {
        let raw = serde_yaml::to_string(self).context("failed to serialize settings yaml")?;
        fs::write(path, raw)
            .with_context(|| format!("failed to write settings file at {}", path.display()))?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ThemeSettings {
    pub font_family: String,
    pub font_size: f32,
    pub mono_font_family: String,
    pub mono_font_size: f32,
    pub radius: f32,
    pub radius_lg: f32,
}

impl Default for ThemeSettings {
    fn default() -> Self {
        Self {
            font_family: ".SystemUIFont".to_string(),
            font_size: 13.0,
            mono_font_family: "Menlo".to_string(),
            mono_font_size: 13.5,
            radius: 4.0,
            radius_lg: 6.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LayoutSettings {
    pub dashboard_title_height: f32,
    pub sidebar_width: f32,
    pub sidebar_min_width: f32,
    pub sidebar_max_width: f32,
    pub sidebar_row_height: f32,
    pub sidebar_header_height: f32,
    pub panel_header_height: f32,
    pub panel_tab_height: f32,
    pub icon_button_height: f32,
    pub panel_tab_close_height: f32,
    pub panel_tab_close_width: f32,
    pub sidebar_close_button_size: f32,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        Self {
            dashboard_title_height: 28.0,
            sidebar_width: 220.0,
            sidebar_min_width: 180.0,
            sidebar_max_width: 420.0,
            sidebar_row_height: 28.0,
            sidebar_header_height: 24.0,
            panel_header_height: 24.0,
            panel_tab_height: 22.0,
            icon_button_height: 18.0,
            panel_tab_close_height: 18.0,
            panel_tab_close_width: 16.0,
            sidebar_close_button_size: 18.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct TerminalSettings {
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub char_width: f32,
    pub resize_debounce_ms: u64,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: "Menlo".to_string(),
            font_size: 14.0,
            line_height: 17.0,
            char_width: 8.4,
            resize_debounce_ms: 150,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AgentConfig {
    pub name: String,
    pub config_file: String,
    pub hook_type: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct LspSettings {
    pub enabled: bool,
    pub servers: std::collections::HashMap<String, Vec<String>>,
}

impl Default for LspSettings {
    fn default() -> Self {
        let mut servers = std::collections::HashMap::new();
        servers.insert("rust".to_string(), vec!["rust-analyzer".to_string()]);
        servers.insert("python".to_string(), vec!["pyright-langserver".to_string(), "--stdio".to_string()]);
        servers.insert("typescript".to_string(), vec!["typescript-language-server".to_string(), "--stdio".to_string()]);
        servers.insert("javascript".to_string(), vec!["typescript-language-server".to_string(), "--stdio".to_string()]);
        servers.insert("html".to_string(), vec!["vscode-html-language-server".to_string(), "--stdio".to_string()]);
        servers.insert("css".to_string(), vec!["vscode-css-language-server".to_string(), "--stdio".to_string()]);
        servers.insert("tailwind".to_string(), vec!["tailwindcss-language-server".to_string(), "--stdio".to_string()]);
        Self {
            enabled: true,
            servers,
        }
    }
}
