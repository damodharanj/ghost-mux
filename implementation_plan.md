# Implementation Plan — Repository Level Search (Cmd-Shift-F)

Provide first-class support for repository-wide search in `ghost-mux`. Users will be able to trigger a search view via `Cmd-Shift-F` globally, which opens a dedicated repository search tab inside the active panel. From there, users can type a query, search all files (excluding common ignore folders like `.git`, `node_modules`, `target`, etc.), and click a search result to open that file at the specific matching line in the Editor.

## Proposed Changes

### Core Elements & State

#### [MODIFY] [layout.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/layout.rs)
- Add `Search` variant to `PanelContent` enum.
- Add `collect_leaf_ids` helper to `PanelLayout` to easily identify all panel layout leaf nodes.

#### [MODIFY] [persist.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/persist.rs)
- Add `Search` variant to `SerPanelContent`.
- Implement mapping for `Search` to/from `PanelContent`.

#### [MODIFY] [dashboard.rs](file:///Users/saranyadamo/Downloads/ghost-mux/src/dashboard.rs)
- Define `SearchMatch` and `SearchState` structs.
- Define `FindInFiles` action.
- Update `DashboardView` to hold `search_states` (mapping tab ID to `SearchState`) and `search_scroll_handles`.
- Register the `FindInFiles` action and bind it to `cmd-shift-f` globally.
- Implement the `trigger_find_in_files` method to:
  - Detect the active/focused panel.
  - Switch to an existing `Search` tab if one exists in the panel, and focus its search input.
  - Create a new `Search` tab if none exists, and focus its search input.
- Implement the `perform_search` method:
  - Triggers a background search task on the app's background executor to avoid locking the UI main thread.
  - Recursively searches file contents inside the active directory, ignoring standard directories like `.git`, `node_modules`, `target`, `.tools`, and `.gemini`.
  - Groups results and matches up to 1000 items.
  - Updates the tab's `SearchState` on the main thread and notifies the UI.
- Implement `open_file_at_line` to:
  - Open a file in the editor.
  - Use `select_text_range` on the editor's `InputState` to position the cursor on the matching line.
- Implement `render_search` layout and styling:
  - Add search bar with input element.
  - Group search results by file.
  - Support hover effects and cursor-pointer for clicking matches.
- Route `PanelContent::Search` in:
  - `ensure_content_entity` (allocates and subscribes to search text inputs).
  - `render_panel_content` (renders search results via `render_search`).
  - `display_title` and `content_title` (return `"search"`).
  - Dropdown menu items for switching tab content (adds "Search" option).

---

## Verification Plan

### Automated Tests
- Run `cargo check` to ensure compiles and type-checks correctly.
- Run `cargo test` to ensure existing tests pass.

### Manual Verification
1. Run `cargo run` to start `ghost-mux`.
2. Press `Cmd-Shift-F` to open the search tab.
3. Verify that focus is placed on the search input.
4. Type a search query (e.g. `fn ` or `struct `) and press Enter.
5. Verify the search happens in the background and matching lines are grouped by file and listed below the input.
6. Hover over a search result and verify it has a hover background.
7. Click a search result and verify it opens the correct file at the exact matching line in the Editor.
8. Verify that switching dashboard or layout persistence persists the search tab if configured to.
