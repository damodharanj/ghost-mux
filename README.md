# Ghost-mux: GPUI-based Dashboard Builder

Ghost-mux is a high-performance GPUI-based desktop dashboard builder. The application provides a highly responsive workspace starting with a single panel that can be split horizontally or vertically to build arbitrary, resizable grid layouts. It is styled with a custom dark-mode theme inspired by premium IDE interfaces and includes full terminal emulation, file exploration, git diffing, and live-adjustable configurations.

![Ghost-mux Design Reference](assets/design/reference-theme.png)

---

## Features

- **Dynamic Resizable Layouts**: Split any panel horizontally (`⬜→`) or vertically (`⬜↓`), resize layouts using draggable split dividers, or close panels. Built using a binary tree structure.
- **Embedded Terminal Emulator**: High-fidelity terminal integration powered by [libghostty-vt](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys) and `portable-pty`.
- **Integrated Git Panel**:
  - Displays current git branch.
  - Interactive changed files explorer (tree view or flat list toggles).
  - Built-in, side-by-side or unified interactive code diff views.
- **Integrated File Explorer**: Nested filesystem tree with collapsible and expandable directory nodes.
- **Live Settings Editor**:
  - Side-panel editor to adjust UI sizes, font families, font sizes, border radius, and terminal properties on-the-fly.
  - Automatically serializes to [settings.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/settings.yaml).
- **State Persistence**:
  - Automatically persists the entire layout tree structure, active tabs, current directories, and custom resize ratios to [dashboard_state.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/dashboard_state.yaml) so that the workspace state is restored seamlessly upon restart.

---

## Repository Structure

| File/Directory | Description |
|---|---|
| [src/main.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/main.rs) | Application entrypoint, GPUI window spawning, and theme initialization. |
| [src/dashboard.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs) | The main [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L161) containing the state, event listeners, and UI rendering logic. |
| [src/layout.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs) | Defines the binary tree layout structure ([PanelLayout](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L8)) and panel content enum ([PanelContent](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs#L124)). |
| [src/persist.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs) | Implements state serialization / deserialization using [DashboardPersistedState](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs#L144). |
| [src/settings.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs) | Defines the deserialization and default structure for user preferences ([AppSettings](file:///Users/saranyadamo/Downloads/ghost-mux/src/settings.rs#L7)). |
| [src/terminal/mod.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/terminal/mod.rs) | Embeds PTY master/slave using `portable-pty` and runs rendering routines under [TerminalModel](file:///Users/saranyadamo/Downloads/ghost-mux/src/terminal/mod.rs#L39). |
| [patches/gpui-component](file:///Users/saranyadamo/Downloads/ghost-mux/patches/gpui-component) | Local patched version of `gpui-component` with support for exact ratio-based splits layout recovery. |
| [patches/libghostty-vt-sys](file:///Users/saranyadamo/Downloads/ghost-mux/patches/libghostty-vt-sys) | Vendored wrapper around Ghostty's vt emulator using local Zig compiler boots. |
| [settings.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/settings.yaml) | Configuration options (fonts, sizes, border radii) loaded and updated dynamically. |
| [dashboard_state.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/dashboard_state.yaml) | State file where dashboard layouts and tabs are saved automatically. |

---

## Architectural Overview

### Layout Representation (`PanelLayout`)

The workspace layout is represented as a binary tree managed inside [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L161):

```
            [VSplit]  (stacked panels)
            /      \
       [HSplit]    Leaf(2) (e.g. Git Panel)
       /      \
   Leaf(0)   Leaf(1)
 (Terminal) (Editor)
```

The split nodes can be dynamically adjusted by the user. When a split is resized, ratios are stored in [DashboardView](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs#L161)'s `resizable_states` and mapped back to the unique split node IDs.

---

## Getting Started

### Prerequisites

Ghost-mux uses `libghostty-vt-sys`, which requires a Zig compiler for vendored compilation. Zig is automatically managed repo-locally within `.tools/zig/toolchain/`, so **no system-wide Zig installation is required**.

### Build & Run Commands

You can use the helper shell scripts located in `tools/` depending on your operating system:

#### macOS Development & Production Build

- **Ensure local Zig is cached & loaded**:
  ```bash
  ./tools/macos/ensure-zig.sh
  ```
- **Typecheck code**:
  ```bash
  cargo check
  ```
- **Run in Development mode**:
  ```bash
  ./tools/macos/dev-run.sh
  ```
- **Build Production Release Bundle**:
  ```bash
  ./tools/macos/build-production.sh
  ```
  This script outputs a standalone executable with its dependency libraries to `dist/gpui-components-demo/`.
- **Launch Production Release Bundle**:
  ```bash
  ./tools/macos/run-production.sh
  ```

#### Windows Development & Production Build (PowerShell)

- **Ensure local Zig is cached**:
  ```powershell
  .\tools\windows\ensure-zig.ps1
  ```
- **Run in Development mode**:
  ```powershell
  .\tools\windows\dev-run.ps1
  ```
- **Build Production Release Bundle**:
  ```powershell
  .\tools\windows\build-production.ps1
  ```
- **Launch Production Release Bundle**:
  ```powershell
  .\tools\windows\run-production.ps1
  ```

---

## Customizing Configuration

Configuration values are stored in [settings.yaml](file:///Users/saranyadamo/Downloads/ghost-mux/settings.yaml). A sample file might look like this:

```yaml
theme:
  font_family: .SystemUIFont
  font_size: 13.5
  mono_font_family: Menlo
  mono_font_size: 12.0
  radius: 13.5
  radius_lg: 10.5
layout:
  dashboard_title_height: 28.0
  sidebar_width: 260.0
  sidebar_min_width: 180.0
  sidebar_max_width: 420.0
terminal:
  font_family: Menlo
  font_size: 12.0
  line_height: 21.0
  char_width: 9.3
  resize_debounce_ms: 150
```

*Note: You can toggle and edit these settings directly within the app using the graphical Settings side-panel, which automatically updates this file.*
