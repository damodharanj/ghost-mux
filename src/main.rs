mod browser;
mod dashboard;
mod hook_server;
mod layout;
mod lsp;
mod persist;
mod settings;
mod terminal;

use dashboard::DashboardView;
use gpui::*;
use gpui_component::{ActiveTheme, Root, Theme, ThemeMode};
use settings::{AppSettings, ThemeSettings};
use std::path::Path;

fn apply_reference_theme(settings: &ThemeSettings, cx: &mut App) {
    let c = |hex| -> Hsla { rgb(hex).into() };
    let theme = Theme::global_mut(cx);
    theme.background = c(0x1e1f22);
    theme.secondary = c(0x1a1b1e);
    theme.sidebar = c(0x191a1d);
    theme.border = c(0x2b2d31);
    theme.sidebar_border = c(0x2b2d31);
    theme.title_bar = c(0x1a1b1e);
    theme.title_bar_border = c(0x2b2d31);
    theme.foreground = c(0xd4d4d4);
    theme.muted_foreground = c(0x9da1a6);
    theme.muted = c(0x26292e);
    theme.accent = c(0x007acc);
    theme.accent_foreground = c(0xffffff);
    theme.ring = theme.accent;
    theme.tab = c(0x1a1b1e);
    theme.tab_bar = c(0x1a1b1e);
    theme.tab_active = c(0x1f2329);
    theme.tab_foreground = c(0x9da1a6);
    theme.tab_active_foreground = c(0xd4d4d4);
    theme.font_family = settings.font_family.clone().into();
    theme.font_size = px(settings.font_size);
    theme.mono_font_family = settings.mono_font_family.clone().into();
    theme.mono_font_size = px(settings.mono_font_size);
    theme.radius = px(settings.radius);
    theme.radius_lg = px(settings.radius_lg);
}

fn main() {
    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        let settings = AppSettings::load_from_file(Path::new("settings.yaml")).unwrap_or_else(|err| {
            eprintln!("Unable to load settings.yaml, using defaults: {err:#}");
            AppSettings::default()
        });
        gpui_component::init(cx);
        Theme::change(ThemeMode::Dark, None, cx);
        apply_reference_theme(&settings.theme, cx);
        terminal::register_bindings(cx);
        dashboard::register_bindings(cx);

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), |window, cx| {
                let view = cx.new(|cx| DashboardView::new(window, settings.clone(), cx));
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}

