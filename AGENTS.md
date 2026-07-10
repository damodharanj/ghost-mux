# Ghost-mux — Dashboard Builder

## Overview

Ghost-mux is a GPUI-based desktop **dashboard builder**. The app starts with a single full-screen panel. The user can split any panel horizontally or vertically to create arbitrary grid layouts. All splits are resizable via drag handles.

Additionally, a headless server is included that hosts a lightweight, mobile-responsive **Web Dashboard** served at `/`. The web client mirrors the desktop app's layout splitting capabilities and provides file exploration, text editing, terminal sessions, and Git operations over a custom JSON-RPC API.

## Design Reference

- Theme, colors, and font reference image: [assets/design/reference-theme1.png](file:///Users/saranyadamo/Downloads/ghost-mux/assets/design/reference-theme1.png)
- Preview:

![Ghost-mux design reference](file:///Users/saranyadamo/Downloads/ghost-mux/assets/design/reference-theme1.png)

---

## Architecture

### Layout Tree ([PanelLayout](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L8))

The entire UI is represented as a **binary tree** stored via [DashboardState::layout](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L53) inside [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L256):

```
Leaf(id)                     — a single panel with content
HSplit { left, right, id }   — two panels side-by-side (resizable)
VSplit { top, bottom, id }   — two panels stacked (resizable)
```

- [Leaf](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L9): a single panel with content.
- [HSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L10): two panels side-by-side (resizable).
- [VSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L15): two panels stacked (resizable).

Every node has a unique `usize` ID managed by [DashboardView::next_id](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L295).

### State — [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L256)

`DashboardView` maintains a map of [DashboardState](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L50) objects:

| Struct | Field | Type | Purpose |
|---|---|---|---|
| [DashboardState](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L50) | [layout](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L53) | [PanelLayout](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L8) | Root of the layout tree for the dashboard |
| [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L256) | [next_id](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L295) | `usize` | Monotonically increasing ID counter for nodes and tabs |

Mutations:
- **Layout Splits/Closes**:
  - [split_panel](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2249) — replace [Leaf](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L9) with an [HSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L10) or [VSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L15)
  - [close_panel](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2293) — remove [Leaf](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L9); sibling fills the space
- **Tab Management**:
  - [add_panel_tab](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2029) — add a new tab to a panel
  - [remove_panel_tab](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2058) — remove a tab from a panel
  - [switch_panel_tab](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2127) — activate a different tab in a panel
  - [set_panel_tab_content](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2169) — set a tab's component content
- **Dashboard CRUD**:
  - [add_dashboard](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L827) — add a new dashboard
  - [switch_dashboard](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L836) — switch active dashboard
  - [remove_dashboard](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L1017) — delete a dashboard

Mutations trigger `cx.notify()` so GPUI re-renders.

### Rendering — [render_layout](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2337)

Recursively traverses the tree:
- [Leaf](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L9) → [render_panel](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L2429) — shows panel content + toolbar
- [HSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L10) → [h_resizable](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component/crates/ui/src/resizable/mod.rs#L15)(...) with two [resizable_panel](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component/crates/ui/src/resizable/mod.rs#L25) children
- [VSplit](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L15) → [v_resizable](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component/crates/ui/src/resizable/mod.rs#L20)(...) with two [resizable_panel](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component/crates/ui/src/resizable/mod.rs#L25) children

Resizable split IDs are formed as `"h-{id}"` / `"v-{id}"` and must be unique across the whole tree (guaranteed because [DashboardView::next_id](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L295) is monotonic).

### Panel Toolbar

Each leaf panel shows a floating toolbar (top-right corner, absolute position) with:
- `⬜→` — split panel horizontally (side by side)
- `⬜↓` — split panel vertically (stacked)
- `✕` — close panel (hidden when only one panel remains)

Buttons dispatch mutations via `cx.listener(move |this, _, _, cx| { ... })`.

---

## Panel Content & Components

Dashboard panels can load different components represented by the [PanelContent](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L124) enum. Rendering is routed by [render_panel_content](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L5933) to the respective rendering functions:

- **Terminal**: Handled via [TerminalModel](file:///Users/saranyadamo/Downloads/ghost-mux/src/terminal/mod.rs#L39) which manages the PTY stream. Rendered via [render_terminal](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L7192) using `libghostty-vt` for terminal emulation.
- **FileExplorer**: Built-in tree explorer rendered via [render_explorer](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L5606) supporting file tree actions (create, delete, rename).
- **Git**: Sidebar/split component rendered via [render_git](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L6522) to track file diffs and tree changes.
- **Browser**: Integrates Cocoa's WKWebView wrapper [WebViewHandle](file:///Users/saranyadamo/Downloads/ghost-mux/src/browser.rs#L79) for macOS environments.
- **Editor**: Text/code file editor rendered via [render_panel_editor](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L8251) with file state management, file save/editing, and diff modes (supporting [render_modal_editor](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L7648), [render_side_by_side_line](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L8058), and [render_inline_diff_line](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L8204)). LSP-backed autocompletion, definitions, and hover actions are powered by [LspClient](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L31) using [GhostCompletionProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L351), [GhostHoverProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L405), and [GhostDefinitionProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L443).
- **Diagnostics**: Workspace diagnostics panel rendered via [render_diagnostics_panel](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L6146) displaying code compilation warnings and errors.

---

## Key Files

| File | Role |
|------|------|
| [src/main.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/main.rs) | App entry point — window setup, reference theme application via [apply_reference_theme](file:///Users/saranyadamo/Downloads/ghost-mux/src/main.rs#L16), and GPUI runner in [main](file:///Users/saranyadamo/Downloads/ghost-mux/src/main.rs#L100) |
| [src/dashboard.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs) | Main view [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L256) holding [DashboardState](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L50) elements, handling pane splits/tabs, and core layout rendering |
| [src/layout.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs) | Layout binary tree model [PanelLayout](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L8) and panel component enum [PanelContent](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L124) |
| [src/lsp.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs) | Language Server Protocol (LSP) integration: [LspClient](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L31) manages process and JSON-RPC lifecycle, while [GhostCompletionProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L351), [GhostHoverProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L405), and [GhostDefinitionProvider](file:///Users/saranyadamo/Downloads/ghost-mux/src/lsp.rs#L443) hook IDE functions into editor buffers |
| [src/settings.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs) | Application configurations: [AppSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L7), [ThemeSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L65), [LayoutSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L89), [TerminalSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L125), and [LspSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L155) |
| [src/persist.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs) | Layout serialisation/deserialisation via [DashboardPersistedState](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs#L152), [SerDashboard](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs#L136), [SerPanelLayout](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs#L62), and [SerPanelContent](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs#L19) |
| [src/browser.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/browser.rs) | WKWebView wrapper [WebViewHandle](file:///Users/saranyadamo/Downloads/ghost-mux/src/browser.rs#L79) integrating macOS Cocoa webview with GPUI |
| [src/hook_server.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/hook_server.rs) | TCP Server ([start_hook_server](file:///Users/saranyadamo/Downloads/ghost-mux/src/hook_server.rs#L13)) for routing agent lifecycle notifications, plus config auto-patches in [setup_agent_hooks](file:///Users/saranyadamo/Downloads/ghost-mux/src/hook_server.rs#L113) |
| [src/remote_api.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/remote_api.rs) | Synchronous JSON-RPC API client ([call_remote_api](file:///Users/saranyadamo/Downloads/ghost-mux/src/remote_api.rs#L5)) over HTTP/HTTPS to headless server for remote filesystem mutations |
| [ghost-mux-server/src/main.rs](file:///Users/saranyadamo/Downloads/ghost-mux/ghost-mux-server/src/main.rs) | Headless Server — processes JSON-RPC requests (such as [pty.list](file:///Users/saranyadamo/Downloads/ghost-mux/ghost-mux-server/src/main.rs#L520)), and serves embedded static files for the Web Dashboard |
| [src/terminal/mod.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/terminal/mod.rs) | PTY stream wrapper [TerminalModel](file:///Users/saranyadamo/Downloads/ghost-mux/src/terminal/mod.rs#L39) implementing terminal emulation via `libghostty-vt` bindings |
| [.github/workflows/build.yml](file:///Users/saranyadamo/Downloads/ghost-mux/.github/workflows/build.yml) | GitHub Actions CI build & release workflow |
| [.devcontainer/devcontainer.json](file:///Users/saranyadamo/ghost-mux/.devcontainer/devcontainer.json) | VS Code devcontainer configuration for development environment |
| [Cargo.toml](file:///Users/saranyadamo/Downloads/ghost-mux/Cargo.toml) | Cargo workspace manifest |
| [rust-toolchain.toml](file:///Users/saranyadamo/Downloads/ghost-mux/rust-toolchain.toml) | Workspace Rust compiler version override |
| [settings.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/settings.yaml) | Runtime fonts, styling boundaries, radius configurations |
| [dashboard_state.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/dashboard_state.yaml) | Auto-generated persistence of layouts, tab mappings, and ratios |
| [Todo.md](file:///Users/saranyadamo/Downloads/ghost-mux/Todo.md) | Ongoing roadmap goals (LSP support, Mobile, Syntax highlight, etc.) |
| [README.md](file:///Users/saranyadamo/ghost-mux/README.md) | User overview, instructions, macOS Gatekeeper troubleshooting, and Linux / Codespaces build guidelines |
| [Wasm_Compilation_Guide.md](file:///Users/saranyadamo/Downloads/ghost-mux/Wasm_Compilation_Guide.md) | Guide for client-server WebAssembly (Wasm) compilation, blockers, and architecture |
| [patches/libghostty-vt-sys](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys) | Vendored bindings of `libghostty-vt` with a custom Zig-based [build.rs](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys/build.rs#L194) detecting the repo-local Zig compiler |
| [patches/gpui-component](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component) | Local component toolkit fork modifying panel restoring behavior |
| [patches/gpui-component.patch](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component.patch) | Git patch applied to upstream gpui-component repository |
| [tools/setup-patches.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/setup-patches.sh) | Shell script to clone and patch gpui-component on Unix/macOS |
| [tools/setup-patches.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/setup-patches.ps1) | PowerShell script to clone and patch gpui-component on Windows |
| [tools/linux/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/ensure-zig.sh) | Zig installer tool for Unix systems |
| [tools/macos/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/ensure-zig.sh) | macOS wrapper script checking for Zig |
| [tools/windows/ensure-zig.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.ps1) | PowerShell script installing local Zig |
| [tools/windows/ensure-zig.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.cmd) | Windows Command Prompt wrapper for Zig setup |
| [tools/linux/build-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/build-production.sh) | Production build pipeline for Linux/Git Bash |
| [tools/macos/build-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/build-production.sh) | macOS packaging script for release bundles, including ad-hoc code signing |
| [tools/windows/build-production.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/build-production.ps1) | Windows PowerShell release bundle compilation script |
| [tools/windows/build-production.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/build-production.cmd) | Command wrapper triggering release compile |
| [tools/linux/run-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/run-production.sh) | Launch script for production build targets under Linux |
| [tools/macos/run-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/run-production.sh) | Release app launch wrapper for macOS environments |
| [tools/windows/run-production.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/run-production.ps1) | PowerShell wrapper targeting release executable launch |
| [tools/windows/run-production.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/run-production.cmd) | CMD launcher shim for production |
| [tools/linux/dev-run.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/dev-run.sh) | Quick development runner on Unix systems |
| [tools/linux/dev-run-server.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/dev-run-server.sh) | Quick development server runner on Unix systems |
| [tools/macos/dev-run.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/dev-run.sh) | Development runner command on macOS |
| [tools/macos/dev-run-server.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/dev-run-server.sh) | Development server runner command on macOS |
| [tools/windows/dev-run.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/dev-run.ps1) | PowerShell debug dev runner on Windows |
| [tools/windows/dev-run-server.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/dev-run-server.ps1) | PowerShell debug dev server runner on Windows |
| [tools/windows/dev-run.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/dev-run.cmd) | CMD shim for developer run command |
| [tools/windows/dev-run-server.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/dev-run-server.cmd) | CMD shim for developer server run command |
| [web/index.html](file:///Users/saranyadamo/Downloads/ghost-mux/web/index.html) | HTML entrypoint template for the mobile-responsive Web Dashboard |
| [web/app.js](file:///Users/saranyadamo/Downloads/ghost-mux/web/app.js) | Client-side application script for the Web Dashboard, handling layouts, RPC calls, and responsive state |
| [web/style.css](file:///Users/saranyadamo/Downloads/ghost-mux/web/style.css) | Custom stylesheets for the Web Dashboard including pane splits, diff highlights, and media queries |
| [AGENTS.md](file:///Users/saranyadamo/Downloads/ghost-mux/AGENTS.md) | Core repository documentation index (this file) |
| [.gitignore](file:///Users/saranyadamo/Downloads/ghost-mux/.gitignore) | Git ignore specifications |

---

## Framework Notes

- **GPUI** (zed-industries/zed) — reactive UI framework
- **gpui-component** (longbridge/gpui-component) — resizable panels, theme tokens
- **Theme**: always use `cx.theme()` tokens (`theme.background`, `theme.secondary`, `theme.foreground`, `theme.muted_foreground`, `theme.border`, `theme.accent`, `theme.muted`). Never hardcode colors except intentional palette values like green `rgb(0x57c994)` and red `rgb(0xf47067)`.
- **`AnyElement`**: every helper returns `.into_any_element()` for composability
- **`cx.listener`**: use `cx.listener(move |this, _, _window, cx| { ... })` for event handlers in render — captures by move, `this` is `&mut` [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L256)
- **Build**: `cargo run` (or `~/.cargo/bin/cargo run` if cargo is not on PATH)
- **Zig toolchain**: pinned at **0.15.2**, cached repo-locally at `.tools/zig/toolchain/` (gitignored). Auto-bootstrapped by [tools/linux/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/ensure-zig.sh) (or [tools/macos/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/ensure-zig.sh)) and [tools/windows/ensure-zig.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.ps1) — no global Zig install required.

---

## Build & Tooling

### Zig (required for `libghostty-vt-sys` vendored build)

Zig is managed **inside this repo** — no system-wide install needed.

| Script | Purpose |
|---|---|
| [tools/linux/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/ensure-zig.sh) | Install/reuse Zig 0.15.2 in `.tools/zig/toolchain/` |
| [tools/macos/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/ensure-zig.sh) | macOS wrapper → [tools/linux/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/ensure-zig.sh) |
| [tools/windows/ensure-zig.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.ps1) | Same, for Windows PowerShell |
| [tools/windows/ensure-zig.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.cmd) | Windows cmd shim → [tools/windows/ensure-zig.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/ensure-zig.ps1) |

- Called automatically by [tools/linux/build-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/build-production.sh) / [tools/windows/build-production.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/build-production.ps1).
- For plain `cargo check` / `cargo build`, the [build.rs](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys/build.rs#L194) in [patches/libghostty-vt-sys](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys) walks up from `CARGO_MANIFEST_DIR` to find `.tools/zig/toolchain/zig[.exe]` automatically — no env var needed.
- Pin a different version: `ZIG_VERSION=0.16.0 [./tools/linux/ensure-zig.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/ensure-zig.sh)`

### Production Bundle

The build scripts compile a `--release` binary, collect non-system runtime dylibs, and package the output:
- **macOS**: Generates a native self-contained app bundle at `dist/Ghost-mux.app` complete with `Info.plist`, compiled `AppIcon.icns`, and ad-hoc code signs all libraries and binaries if the `codesign` tool is available.
- **Linux**: Outputs a self-contained directory to `dist/ghost-mux/`.
- **Windows**: Outputs to `dist/ghost-mux/`.

| Script | OS |
|---|---|
| [tools/linux/build-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/build-production.sh) | Linux, Git Bash on Windows |
| [tools/macos/build-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/build-production.sh) | macOS |
| [tools/windows/build-production.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/build-production.ps1) | Windows PowerShell |
| [tools/windows/build-production.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/build-production.cmd) | Windows cmd (delegates to ps1) |

macOS: packages the executable, libraries, settings, web client files, and icon into `dist/Ghost-mux.app`. If the `codesign` tool is available, it performs ad-hoc code signing on all libraries and binaries (including the overall bundle). On startup, if launched as a bundle (or from `/`), it pivots working directory to `~/Library/Application Support/ghost-mux` where it copies `settings.yaml` and saves layout state.
Linux: uses `ldd` + `patchelf` (or `chrpath`, or a `run.sh` wrapper as fallback).  
Windows: no bundling needed; PE binaries link against system DLLs.

### Running the Bundled Executable

| Script | OS |
|---|---|
| [tools/linux/run-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/linux/run-production.sh) | Linux, Git Bash |
| [tools/macos/run-production.sh](file:///Users/saranyadamo/Downloads/ghost-mux/tools/macos/run-production.sh) | macOS |
| [tools/windows/run-production.ps1](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/run-production.ps1) | Windows PowerShell |
| [tools/windows/run-production.cmd](file:///Users/saranyadamo/Downloads/ghost-mux/tools/windows/run-production.cmd) | Windows cmd |

macOS: launches the `Ghost-mux.app` bundle via macOS `open`. Note that for pre-built release bundles downloaded directly from GitHub Releases, macOS Gatekeeper may block execution. The user can clear the quarantine attribute via `xattr -cr /Applications/Ghost-mux.app` to resolve this (see [README.md](file:///Users/saranyadamo/ghost-mux/README.md#L96-L109) for more details).
Linux: prefers `run.sh` wrapper inside the bundle (Linux fallback) over calling the binary directly.

---

## Development Workflow

### macOS

```bash
# Install repo-local Zig (first time or to update)
./tools/macos/ensure-zig.sh

# Quick type check (Zig auto-detected — no env var needed)
cargo check

# Dev run GUI app
./tools/macos/dev-run.sh

# Dev run GUI and server separately
./tools/macos/dev-run-server.sh

# Dev run GUI and server together
./tools/macos/dev-run-all.sh

# Production bundle → dist/<bin>/
./tools/macos/build-production.sh

# Launch bundled binary
./tools/macos/run-production.sh
```

### Linux (including GitHub Codespaces)

For Linux environments and GitHub Codespaces, install the LLVM C++ standard library prerequisites before setup:

```bash
# Install LLVM C++ prerequisites
sudo apt-get update
sudo apt-get install -y libc++-dev libc++abi-dev

# Install repo-local Zig (first time or to update)
./tools/linux/ensure-zig.sh

# Quick type check
cargo check

# Dev run GUI app
./tools/linux/dev-run.sh

# Dev run server separately
./tools/linux/dev-run-server.sh

# Production bundle → dist/<bin>/
./tools/linux/build-production.sh

# Launch bundled binary
./tools/linux/run-production.sh
```

### Windows (PowerShell)

```powershell
# Install repo-local Zig (first time or to update)
.\tools\windows\ensure-zig.ps1

# Quick type check
cargo check

# Dev run GUI app
.\tools\windows\dev-run.ps1

# Dev run server separately
.\tools\windows\dev-run-server.ps1

# Dev run GUI and server together
.\tools\windows\dev-run-all.ps1

# Production bundle → dist\<bin>\
.\tools\windows\build-production.ps1

# Launch bundled binary
.\tools\windows\run-production.ps1
```

### Windows (cmd)

```cmd
tools\windows\ensure-zig.cmd
tools\windows\dev-run.cmd
tools\windows\dev-run-server.cmd
tools\windows\dev-run-all.cmd
tools\windows\build-production.cmd
tools\windows\run-production.cmd
```
