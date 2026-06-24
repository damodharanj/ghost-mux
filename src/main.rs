mod browser;
mod dashboard;
mod hook_server;
mod layout;
mod lsp;
mod persist;
mod remote_api;
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

fn setup_working_directory() {
    // 1. If settings.yaml exists in CWD, do nothing.
    if std::path::Path::new("settings.yaml").exists() {
        return;
    }

    // 2. If settings.yaml exists next to the executable, change CWD to that directory.
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(exe_dir) = current_exe.parent() {
            if exe_dir.join("settings.yaml").exists() {
                let _ = std::env::set_current_dir(exe_dir);
                return;
            }
        }
    }

    // 3. If running inside a macOS bundle or launched from root directory, use Application Support.
    #[cfg(target_os = "macos")]
    {
        let current_dir = std::env::current_dir().unwrap_or_default();
        let current_exe = std::env::current_exe().unwrap_or_default();
        let is_bundle = current_exe.to_string_lossy().contains(".app/Contents/MacOS/");
        let is_root_dir = current_dir == std::path::Path::new("/");

        if is_bundle || is_root_dir {
            if let Ok(home) = std::env::var("HOME") {
                let app_support = std::path::PathBuf::from(home)
                    .join("Library/Application Support/ghost-mux");
                
                let _ = std::fs::create_dir_all(&app_support);

                let dest_settings = app_support.join("settings.yaml");
                if !dest_settings.exists() {
                    // Try to copy from bundle's Resources directory
                    if let Some(macos_dir) = current_exe.parent() {
                        if let Some(contents_dir) = macos_dir.parent() {
                            let resources_settings = contents_dir.join("Resources/settings.yaml");
                            if resources_settings.exists() {
                                let _ = std::fs::copy(&resources_settings, &dest_settings);
                            }
                        }
                    }
                }

                let _ = std::env::set_current_dir(&app_support);
            }
        }
    }
}
actions!(app, [Quit]);

fn quit(_: &Quit, cx: &mut App) {
    cx.quit();
}

fn main() {
    setup_working_directory();

    // Parse --server or -s from command line arguments
    let args: Vec<String> = std::env::args().collect();
    let mut server_arg = None;
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--server" || args[i] == "-s" {
            if i + 1 < args.len() {
                server_arg = Some(args[i + 1].clone());
                i += 1;
            }
        }
        i += 1;
    }

    let app = gpui_platform::application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx| {
        cx.on_action(quit);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
        ]);
        cx.set_menus(vec![Menu {
            name: "Ghost-mux".into(),
            items: vec![MenuItem::action("Quit", Quit)],
            disabled: false,
        }]);
        let mut settings = AppSettings::load_from_file(Path::new("settings.yaml")).unwrap_or_else(|err| {
            eprintln!("Unable to load settings.yaml, using defaults: {err:#}");
            AppSettings::default()
        });
        if let Some(url) = server_arg {
            settings.server_url = Some(url);
        }
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

