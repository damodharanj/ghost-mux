use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;


use serde_json::Value;

// PTY dependencies
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use libghostty_vt::{
    render::{CellIterator, RowIterator},
    RenderState, Terminal, TerminalOptions,
};


// --- Constants ---
const DEFAULT_COLS: usize = 80;
const DEFAULT_ROWS: usize = 24;


// Normalize agent hooks events like in hook_server.rs
fn normalize_event(raw: &str) -> &'static str {
    match raw.to_lowercase().as_str() {
        "start" | "started" | "sessionstart" | "session_start" | "task_started" | "userpromptsubmit" => "Start",
        "stop" | "completed" | "sessionend" | "session_end" | "agent-turn-complete" | "task_complete" => "Stop",
        "permissionrequest" | "waiting-for-input" | "exec_approval_request" | "apply_patch_approval_request" | "request_user_input" => "PermissionRequest",
        _ => "Unknown",
    }
}

// --- Struct definitions ---

// Wrapper to satisfy thread-safety Send/Sync requirements for the underlying C/Zig terminal emulator objects
struct TerminalEmulator {
    terminal: Terminal<'static, 'static>,
    render_state: RenderState<'static>,
    row_iter: RowIterator<'static>,
    cell_iter: CellIterator<'static>,
}

unsafe impl Send for TerminalEmulator {}
unsafe impl Sync for TerminalEmulator {}

// Represents an active PTY process and terminal emulator session
struct PtySession {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    pty_master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    emulator: Arc<Mutex<TerminalEmulator>>,
    cols: usize,
    rows: usize,
    running_agent: Option<String>,
    last_event: Option<String>,
}

// Represents a language server workspace session
struct LspSession {
    _child: std::process::Child,
    writer: Arc<Mutex<std::process::ChildStdin>>,
    next_id: Arc<Mutex<i64>>,
    pending: Arc<Mutex<HashMap<i64, std::sync::mpsc::Sender<Result<Value, String>>>>>,
    diagnostics: Arc<Mutex<HashMap<String, Value>>>,
}

impl LspSession {
    pub fn start(cmd: &[String], workspace_root: &Path) -> Result<Self, String> {
        if cmd.is_empty() {
            return Err("Empty command".into());
        }
        let mut child = std::process::Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(workspace_root)
            .spawn()
            .map_err(|e| e.to_string())?;

        let stdin = child.stdin.take().ok_or("Failed to get stdin")?;
        let stdout = child.stdout.take().ok_or("Failed to get stdout")?;
        let stderr = child.stderr.take().ok_or("Failed to get stderr")?;

        let writer = Arc::new(Mutex::new(stdin));
        let next_id = Arc::new(Mutex::new(0));
        let pending: Arc<Mutex<HashMap<i64, std::sync::mpsc::Sender<Result<Value, String>>>>> = Arc::new(Mutex::new(HashMap::new()));
        let diagnostics: Arc<Mutex<HashMap<String, Value>>> = Arc::new(Mutex::new(HashMap::new()));

        // Stdout reader thread
        let pending_clone = pending.clone();
        let diagnostics_clone = diagnostics.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).is_err() || line.is_empty() {
                    break;
                }
                if !line.starts_with("Content-Length:") {
                    continue;
                }
                let len_str = line.trim_start_matches("Content-Length:").trim();
                let Ok(len) = len_str.parse::<usize>() else {
                    continue;
                };

                // Read empty line
                line.clear();
                if reader.read_line(&mut line).is_err() {
                    break;
                }

                // Read body
                let mut body = vec![0u8; len];
                if reader.read_exact(&mut body).is_err() {
                    break;
                }

                if let Ok(json) = serde_json::from_slice::<Value>(&body) {
                    if let Some(id) = json.get("id").and_then(|v| v.as_i64()) {
                        let mut p = pending_clone.lock().unwrap();
                        if let Some(tx) = p.remove(&id) {
                            if let Some(error) = json.get("error") {
                                let _ = tx.send(Err(error.to_string()));
                            } else if let Some(result) = json.get("result") {
                                let _ = tx.send(Ok(result.clone()));
                            } else {
                                let _ = tx.send(Ok(Value::Null));
                            }
                        }
                    } else if let Some(method) = json.get("method").and_then(|v| v.as_str()) {
                        if method == "textDocument/publishDiagnostics" {
                            if let Some(params) = json.get("params") {
                                if let (Some(uri), Some(diags)) = (params.get("uri"), params.get("diagnostics")) {
                                    let mut d = diagnostics_clone.lock().unwrap();
                                    d.insert(uri.as_str().unwrap_or("").to_string(), diags.clone());
                                }
                            }
                        }
                    }
                }
            }
        });

        // Stderr reader thread (just consume to prevent blocking)
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok() && !line.is_empty() {
                line.clear();
            }
        });

        Ok(Self {
            _child: child,
            writer,
            next_id,
            pending,
            diagnostics,
        })
    }

    pub fn send_request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = {
            let mut nid = self.next_id.lock().unwrap();
            *nid += 1;
            *nid
        };

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let content = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);

        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut p = self.pending.lock().unwrap();
            p.insert(id, tx);
        }

        {
            let mut w = self.writer.lock().unwrap();
            w.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
            w.flush().map_err(|e| e.to_string())?;
        }

        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("LSP request timeout/error: {}", e))?
    }

    pub fn send_notification(&self, method: &str, params: Value) -> Result<(), String> {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&req).map_err(|e| e.to_string())?;
        let content = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);

        let mut w = self.writer.lock().unwrap();
        w.write_all(content.as_bytes()).map_err(|e| e.to_string())?;
        w.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn get_diagnostics(&self) -> HashMap<String, Value> {
        self.diagnostics.lock().unwrap().clone()
    }
}

// Global server context holding active states
struct ServerState {
    ptys: Mutex<HashMap<String, PtySession>>,
    lsps: Mutex<HashMap<String, LspSession>>,
    next_pty_id: std::sync::atomic::AtomicUsize,
    next_lsp_id: std::sync::atomic::AtomicUsize,
    port: u16,
}

impl ServerState {
    fn new(port: u16) -> Self {
        Self {
            ptys: Mutex::new(HashMap::new()),
            lsps: Mutex::new(HashMap::new()),
            next_pty_id: std::sync::atomic::AtomicUsize::new(1),
            next_lsp_id: std::sync::atomic::AtomicUsize::new(1),
            port,
        }
    }

    fn execute_method(&self, method: &str, params: Value) -> Result<Value, String> {
        match method {
            // --- File System Methods ---
            "fs.list_dir" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                let p = Path::new(path_str);
                let mut entries = Vec::new();
                let dir_entries = fs::read_dir(p).map_err(|e| e.to_string())?;
                for entry in dir_entries {
                    if let Ok(entry) = entry {
                        let meta = entry.metadata().map_err(|e| e.to_string())?;
                        entries.push(serde_json::json!({
                            "name": entry.file_name().to_string_lossy().to_string(),
                            "path": entry.path().to_string_lossy().to_string(),
                            "is_dir": meta.is_dir(),
                            "size": meta.len(),
                        }));
                    }
                }
                Ok(serde_json::json!({ "entries": entries }))
            }
            "fs.read_file" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                let content = fs::read_to_string(path_str).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "content": content }))
            }
            "fs.write_file" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                let content = params.get("content").and_then(|c| c.as_str()).ok_or("Missing content")?;
                fs::write(path_str, content).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "success": true }))
            }
            "fs.create_file" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                fs::File::create(path_str).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "success": true }))
            }
            "fs.create_dir" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                fs::create_dir_all(path_str).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "success": true }))
            }
            "fs.rename" => {
                let src = params.get("src").and_then(|p| p.as_str()).ok_or("Missing src")?;
                let dst = params.get("dst").and_then(|p| p.as_str()).ok_or("Missing dst")?;
                fs::rename(src, dst).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "success": true }))
            }
            "fs.delete" => {
                let path_str = params.get("path").and_then(|p| p.as_str()).ok_or("Missing path")?;
                let recursive = params.get("recursive").and_then(|r| r.as_bool()).unwrap_or(false);
                let p = Path::new(path_str);
                if p.is_dir() {
                    if recursive {
                        fs::remove_dir_all(p).map_err(|e| e.to_string())?;
                    } else {
                        fs::remove_dir(p).map_err(|e| e.to_string())?;
                    }
                } else {
                    fs::remove_file(p).map_err(|e| e.to_string())?;
                }
                Ok(serde_json::json!({ "success": true }))
            }

            // --- PTY Methods ---
            "pty.spawn" => {
                let cwd_str = params.get("cwd").and_then(|c| c.as_str());
                let command_args: Option<Vec<String>> = params.get("command")
                    .and_then(|c| c.as_array())
                    .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect());

                let cols = params.get("cols").and_then(|c| c.as_u64()).map(|c| c as usize).unwrap_or(DEFAULT_COLS);
                let rows = params.get("rows").and_then(|r| r.as_u64()).map(|r| r as usize).unwrap_or(DEFAULT_ROWS);

                let id = self.next_pty_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string();

                let pty_system = native_pty_system();
                let pair = pty_system
                    .openpty(PtySize {
                        rows: rows as u16,
                        cols: cols as u16,
                        pixel_width: 0,
                        pixel_height: 0,
                    })
                    .map_err(|e| e.to_string())?;

                let mut cmd = if let Some(ref args) = command_args {
                    if args.is_empty() {
                        CommandBuilder::new_default_prog()
                    } else {
                        let mut cb = CommandBuilder::new(&args[0]);
                        cb.args(&args[1..]);
                        cb
                    }
                } else {
                    CommandBuilder::new_default_prog()
                };

                if let Some(ref dir) = cwd_str {
                    let path = Path::new(dir);
                    if path.exists() {
                        cmd.cwd(path);
                    }
                }

                // Setup Environment variables for Agent hook detection
                cmd.env("TERM", "xterm-256color");
                cmd.env("COLUMNS", cols.to_string());
                cmd.env("LINES", rows.to_string());
                cmd.env("GHOST_MUX_HOOK_URL", &format!("http://127.0.0.1:{}/hook/{}", self.port, id));
                cmd.env("GHOST_MUX_TERMINAL_ID", &id);
                cmd.env("SUPERSET_HOST_AGENT_HOOK_URL", &format!("http://127.0.0.1:{}/hook/{}", self.port, id));
                cmd.env("SUPERSET_TERMINAL_ID", &id);
                cmd.env("SUPERSET_WORKSPACE_ID", "1");
                cmd.env("SUPERSET_TAB_ID", &id);
                cmd.env("SUPERSET_PANE_ID", &id);
                if let Ok(home) = std::env::var("HOME") {
                    cmd.env("SUPERSET_HOME_DIR", &format!("{}/.ghost-mux", home));
                } else if let Ok(home) = std::env::var("USERPROFILE") {
                    cmd.env("SUPERSET_HOME_DIR", &format!("{}/.ghost-mux", home));
                }

                let child = pair.slave.spawn_command(cmd).map_err(|e| e.to_string())?;
                drop(pair.slave);

                let writer_raw = pair.master.take_writer().map_err(|e| e.to_string())?;
                let writer: Arc<Mutex<Box<dyn Write + Send>>> = Arc::new(Mutex::new(writer_raw));
                let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
                let pty_master = Arc::new(Mutex::new(pair.master));

                let terminal = Terminal::new(TerminalOptions {
                    cols: cols as u16,
                    rows: rows as u16,
                    max_scrollback: 5000,
                })
                .map_err(|e| e.to_string())?;

                let render_state = RenderState::new().map_err(|e| e.to_string())?;
                let row_iter = RowIterator::new().map_err(|e| e.to_string())?;
                let cell_iter = CellIterator::new().map_err(|e| e.to_string())?;

                let emulator = Arc::new(Mutex::new(TerminalEmulator {
                    terminal,
                    render_state,
                    row_iter,
                    cell_iter,
                }));

                let output_buffer = Arc::new(Mutex::new(Vec::new()));

                // PTY reader thread
                let output_buffer_clone = output_buffer.clone();
                let emulator_clone = emulator.clone();
                thread::spawn(move || {
                    let _child = child;
                    let mut buf = [0u8; 4096];
                    loop {
                        match reader.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let bytes = &buf[..n];
                                output_buffer_clone.lock().unwrap().extend_from_slice(bytes);
                                emulator_clone.lock().unwrap().terminal.vt_write(bytes);
                            }
                        }
                    }
                });

                let session = PtySession {
                    writer,
                    pty_master,
                    output_buffer,
                    emulator,
                    cols,
                    rows,
                    running_agent: None,
                    last_event: None,
                };

                self.ptys.lock().unwrap().insert(id.clone(), session);
                Ok(serde_json::json!({ "pty_id": id }))
            }
            "pty.write" => {
                let pty_id = params.get("pty_id").and_then(|p| p.as_str()).ok_or("Missing pty_id")?;
                let input = params.get("input").and_then(|i| i.as_str()).ok_or("Missing input")?;
                
                let ptys = self.ptys.lock().unwrap();
                let pty = ptys.get(pty_id).ok_or("PTY session not found")?;
                let mut w = pty.writer.lock().unwrap();
                w.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
                w.flush().map_err(|e| e.to_string())?;
                Ok(serde_json::json!({ "success": true }))
            }
            "pty.read" => {
                let pty_id = params.get("pty_id").and_then(|p| p.as_str()).ok_or("Missing pty_id")?;
                let ptys = self.ptys.lock().unwrap();
                let pty = ptys.get(pty_id).ok_or("PTY session not found")?;
                let mut buf = pty.output_buffer.lock().unwrap();
                let output = String::from_utf8_lossy(&buf).into_owned();
                buf.clear();
                Ok(serde_json::json!({
                    "output": output,
                    "running_agent": pty.running_agent,
                    "last_event": pty.last_event
                }))
            }
            "pty.get_screen" => {
                let pty_id = params.get("pty_id").and_then(|p| p.as_str()).ok_or("Missing pty_id")?;
                let ptys = self.ptys.lock().unwrap();
                let pty = ptys.get(pty_id).ok_or("PTY session not found")?;

                let em = &mut *pty.emulator.lock().unwrap();
                let snap = em.render_state.update(&em.terminal).map_err(|e| e.to_string())?;
                let mut row_it = em.row_iter.update(&snap).map_err(|e| e.to_string())?;
                
                let mut rows_list = Vec::new();
                while let Some(row) = row_it.next() {
                    let mut cell_it = em.cell_iter.update(row).map_err(|e| e.to_string())?;
                    let mut row_str = String::new();
                    while let Some(cell) = cell_it.next() {
                        let grapheme_len = cell.graphemes_len().unwrap_or(0);
                        let ch = if grapheme_len > 0 {
                            cell.graphemes()
                                .ok()
                                .and_then(|g| g.into_iter().next())
                                .unwrap_or(' ')
                        } else {
                            ' '
                        };
                        row_str.push(ch);
                    }
                    if row_str.len() < pty.cols {
                        row_str.extend(std::iter::repeat(' ').take(pty.cols - row_str.len()));
                    }
                    rows_list.push(row_str);
                }

                let cursor_viewport = snap.cursor_viewport().ok().flatten();
                let cursor_x = cursor_viewport.map(|cv| cv.x as usize).unwrap_or(0);
                let cursor_y = cursor_viewport.map(|cv| cv.y as usize).unwrap_or(0);

                Ok(serde_json::json!({
                    "rows": rows_list,
                    "cursor_x": cursor_x,
                    "cursor_y": cursor_y,
                    "cols": pty.cols,
                    "rows_count": pty.rows,
                    "running_agent": pty.running_agent,
                    "last_event": pty.last_event,
                }))
            }
            "pty.resize" => {
                let pty_id = params.get("pty_id").and_then(|p| p.as_str()).ok_or("Missing pty_id")?;
                let cols = params.get("cols").and_then(|c| c.as_u64()).ok_or("Missing cols")? as usize;
                let rows = params.get("rows").and_then(|r| r.as_u64()).ok_or("Missing rows")? as usize;

                let mut ptys = self.ptys.lock().unwrap();
                let pty = ptys.get_mut(pty_id).ok_or("PTY session not found")?;
                pty.pty_master.lock().unwrap().resize(PtySize {
                    rows: rows as u16,
                    cols: cols as u16,
                    pixel_width: 0,
                    pixel_height: 0,
                }).map_err(|e| e.to_string())?;

                let _ = pty.emulator.lock().unwrap().terminal.resize(cols as u16, rows as u16, 0, 0);
                pty.cols = cols;
                pty.rows = rows;
                Ok(serde_json::json!({ "success": true }))
            }
            "pty.close" => {
                let pty_id = params.get("pty_id").and_then(|p| p.as_str()).ok_or("Missing pty_id")?;
                let mut ptys = self.ptys.lock().unwrap();
                if ptys.remove(pty_id).is_some() {
                    Ok(serde_json::json!({ "success": true }))
                } else {
                    Err("PTY session not found".to_string())
                }
            }
            "pty.list" => {
                let ptys = self.ptys.lock().unwrap();
                let list: Vec<Value> = ptys.iter().map(|(id, pty)| {
                    serde_json::json!({
                        "pty_id": id,
                        "cols": pty.cols,
                        "rows": pty.rows,
                        "running_agent": pty.running_agent,
                        "last_event": pty.last_event
                    })
                }).collect();
                Ok(serde_json::json!({ "sessions": list }))
            }

            // --- Git Methods ---
            "git.status" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;
                let is_git = std::process::Command::new("git")
                    .arg("rev-parse")
                    .arg("--is-inside-work-tree")
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                if !is_git.status.success() {
                    return Err("Not a git repository".to_string());
                }

                let branch_output = std::process::Command::new("git")
                    .arg("rev-parse")
                    .arg("--abbrev-ref")
                    .arg("HEAD")
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                let branch = if branch_output.status.success() {
                    String::from_utf8_lossy(&branch_output.stdout).trim().to_string()
                } else {
                    "HEAD".to_string()
                };

                let mut files = Vec::new();
                let status_output = std::process::Command::new("git")
                    .arg("status")
                    .arg("--porcelain")
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                if status_output.status.success() {
                    let stdout = String::from_utf8_lossy(&status_output.stdout);
                    for line in stdout.lines() {
                        if line.len() > 3 {
                            let status = line[0..2].trim().to_string();
                            let path = line[3..].to_string();
                            files.push(serde_json::json!({
                                "status": status,
                                "path": path
                            }));
                        }
                    }
                }

                Ok(serde_json::json!({
                    "branch": branch,
                    "files": files
                }))
            }
            "git.diff" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;
                let path = params.get("path").and_then(|p| p.as_str());

                let mut args = vec!["diff"];
                if let Some(ref file_path) = path {
                    args.push("--");
                    args.push(file_path);
                }

                let diff_output = std::process::Command::new("git")
                    .args(&args)
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                let stdout = String::from_utf8_lossy(&diff_output.stdout).into_owned();
                Ok(serde_json::json!({ "diff": stdout }))
            }
            "git.add" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;
                let path = params.get("path").and_then(|p| p.as_str()).unwrap_or(".");

                let add_output = std::process::Command::new("git")
                    .arg("add")
                    .arg(path)
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                if add_output.status.success() {
                    Ok(serde_json::json!({ "success": true }))
                } else {
                    Err(String::from_utf8_lossy(&add_output.stderr).into_owned())
                }
            }
            "git.commit" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;
                let message = params.get("message").and_then(|m| m.as_str()).ok_or("Missing message")?;

                let commit_output = std::process::Command::new("git")
                    .arg("commit")
                    .arg("-m")
                    .arg(message)
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                if commit_output.status.success() {
                    Ok(serde_json::json!({ "success": true }))
                } else {
                    Err(String::from_utf8_lossy(&commit_output.stderr).into_owned())
                }
            }
            "git.push" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;

                let push_output = std::process::Command::new("git")
                    .arg("push")
                    .current_dir(cwd)
                    .output()
                    .map_err(|e| e.to_string())?;

                if push_output.status.success() {
                    Ok(serde_json::json!({ "success": true }))
                } else {
                    Err(String::from_utf8_lossy(&push_output.stderr).into_owned())
                }
            }

            // --- LSP Methods ---
            "lsp.start" => {
                let cwd = params.get("cwd").and_then(|c| c.as_str()).ok_or("Missing cwd")?;
                let command_args: Vec<String> = params.get("command")
                    .and_then(|c| c.as_array())
                    .ok_or("Missing or invalid command arguments list")?
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();

                let id = self.next_lsp_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string();
                let session = LspSession::start(&command_args, Path::new(cwd))?;
                
                // Perform LSP handshake initialize
                let init_params = serde_json::json!({
                    "processId": std::process::id(),
                    "rootPath": cwd,
                    "rootUri": format!("file://{}", cwd),
                    "capabilities": {
                        "textDocument": {
                            "completion": {
                                "completionItem": {
                                    "snippetSupport": true
                                }
                            }
                        }
                    }
                });

                session.send_request("initialize", init_params)?;
                session.send_notification("initialized", serde_json::json!({}))?;

                self.lsps.lock().unwrap().insert(id.clone(), session);
                Ok(serde_json::json!({ "lsp_id": id }))
            }
            "lsp.request" => {
                let lsp_id = params.get("lsp_id").and_then(|p| p.as_str()).ok_or("Missing lsp_id")?;
                let lsp_method = params.get("method").and_then(|m| m.as_str()).ok_or("Missing method")?;
                let req_params = params.get("params").cloned().unwrap_or(Value::Null);

                let lsps = self.lsps.lock().unwrap();
                let lsp = lsps.get(lsp_id).ok_or("LSP session not found")?;
                let resp = lsp.send_request(lsp_method, req_params)?;
                Ok(resp)
            }
            "lsp.notify" => {
                let lsp_id = params.get("lsp_id").and_then(|p| p.as_str()).ok_or("Missing lsp_id")?;
                let lsp_method = params.get("method").and_then(|m| m.as_str()).ok_or("Missing method")?;
                let req_params = params.get("params").cloned().unwrap_or(Value::Null);

                let lsps = self.lsps.lock().unwrap();
                let lsp = lsps.get(lsp_id).ok_or("LSP session not found")?;
                lsp.send_notification(lsp_method, req_params)?;
                Ok(serde_json::json!({ "success": true }))
            }
            "lsp.get_diagnostics" => {
                let lsp_id = params.get("lsp_id").and_then(|p| p.as_str()).ok_or("Missing lsp_id")?;
                let lsps = self.lsps.lock().unwrap();
                let lsp = lsps.get(lsp_id).ok_or("LSP session not found")?;
                let diags = lsp.get_diagnostics();
                Ok(serde_json::to_value(&diags).map_err(|e| e.to_string())?)
            }

            _ => Err(format!("Unknown method: {}", method)),
        }
    }
}

// Handles dynamic agent callbacks
fn handle_hook(path: &str, req_body: &str, server_state: &ServerState) -> Result<(), String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() || segments[0] != "hook" {
        return Err("Not a hook endpoint".into());
    }

    let pty_id = segments.get(1).ok_or("Missing terminal/pty id in hook path")?.to_string();
    let mut event = segments.get(2).map(|s| s.to_string());

    // Extract event type from JSON body if not present in path or as validation
    if event.is_none() && !req_body.trim().is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(req_body) {
            if let Some(ev) = v.get("eventType").and_then(|x| x.as_str()) {
                event = Some(ev.to_string());
            }
        }
    }

    let event_name = event.ok_or("Missing hook event type")?;
    let normalized = normalize_event(&event_name);

    let mut ptys = server_state.ptys.lock().unwrap();
    if let Some(pty) = ptys.get_mut(&pty_id) {
        pty.last_event = Some(normalized.to_string());
        if normalized == "Start" {
            pty.running_agent = Some("ActiveAgent".to_string());
        } else if normalized == "Stop" {
            pty.running_agent = None;
        }
        Ok(())
    } else {
        Err(format!("PTY session {} not found for hook", pty_id))
    }
}

const INDEX_HTML: &[u8] = include_bytes!("../../web/index.html");
const APP_JS: &[u8] = include_bytes!("../../web/app.js");
const STYLE_CSS: &[u8] = include_bytes!("../../web/style.css");

// HTTP Connection handling
fn handle_connection(mut stream: TcpStream, server_state: Arc<ServerState>) {
    let mut reader = BufReader::new(&mut stream);
    let mut headers = Vec::new();
    let mut content_length = 0;
    let mut is_post = false;
    let mut is_options = false;
    let mut is_get = false;
    let mut path = String::new();

    loop {
        let mut line = String::new();
        if let Err(_) = reader.read_line(&mut line) {
            return;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        headers.push(line.clone());

        if line.starts_with("POST ") {
            is_post = true;
            if let Some(p) = line.split_whitespace().nth(1) {
                path = p.to_string();
            }
        } else if line.starts_with("GET ") {
            is_get = true;
            if let Some(p) = line.split_whitespace().nth(1) {
                path = p.to_string();
            }
        } else if line.starts_with("OPTIONS ") {
            is_options = true;
            if let Some(p) = line.split_whitespace().nth(1) {
                path = p.to_string();
            }
        }
        if line.to_lowercase().starts_with("content-length:") {
            if let Some(len_str) = line.split(':').nth(1) {
                if let Ok(len) = len_str.trim().parse::<usize>() {
                    content_length = len;
                }
            }
        }
    }

    if is_options {
        let response = "HTTP/1.1 200 OK\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\nContent-Length: 0\r\n\r\n";
        let _ = stream.write_all(response.as_bytes());
        return;
    }

    if is_get {
        let file_path = path.split('?').next().unwrap_or("/");

        if file_path == "/ws" {
            let pty_id = if let Some(query_idx) = path.find('?') {
                let query = &path[query_idx + 1..];
                query.split('&')
                    .find(|pair| pair.starts_with("pty_id="))
                    .and_then(|pair| pair.split('=').nth(1))
                    .map(|val| val.to_string())
            } else {
                None
            };

            if let Some(id) = pty_id {
                let mut raw_headers = Vec::new();
                for h in &headers {
                    raw_headers.extend_from_slice(h.as_bytes());
                }
                raw_headers.extend_from_slice(b"\r\n");

                let prefixed = PrefixedStream {
                    prefix: std::io::Cursor::new(raw_headers),
                    stream,
                };

                handle_websocket(prefixed, id, server_state);
            } else {
                let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\nMissing pty_id parameter");
            }
            return;
        }

        if file_path.contains("..") {
            send_html_response(stream, 403, "Forbidden", "<h1>403 Forbidden</h1>");
            return;
        }

        let target_file = if file_path == "/" {
            "index.html"
        } else {
            file_path.strip_prefix('/').unwrap_or(file_path)
        };

        match target_file {
            "index.html" => {
                send_file_response(stream, 200, "OK", "text/html", INDEX_HTML);
            }
            "app.js" => {
                send_file_response(stream, 200, "OK", "application/javascript", APP_JS);
            }
            "style.css" => {
                send_file_response(stream, 200, "OK", "text/css", STYLE_CSS);
            }
            _ => {
                send_html_response(stream, 404, "Not Found", "<h1>404 Not Found</h1>");
            }
        }
        return;
    }

    if !is_post {
        send_response(stream, 405, "Method Not Allowed", &serde_json::json!({ "error": "Only POST, GET or OPTIONS methods are supported" }));
        return;
    }

    let mut body_buf = vec![0u8; content_length];
    if content_length > 0 {
        if let Err(_) = reader.read_exact(&mut body_buf) {
            send_response(stream, 400, "Bad Request", &serde_json::json!({ "error": "Failed to read request body" }));
            return;
        }
    }

    let req_body = String::from_utf8_lossy(&body_buf);

    // Check if it's an agent lifecycle hook endpoint
    if path.starts_with("/hook/") {
        match handle_hook(&path, &req_body, &server_state) {
            Ok(_) => {
                send_response(stream, 200, "OK", &serde_json::json!({ "success": true }));
            }
            Err(err) => {
                send_response(stream, 400, "Bad Request", &serde_json::json!({ "error": err }));
            }
        }
        return;
    }

    // Standard RPC API Endpoint
    let req_val: Value = match serde_json::from_slice(&body_buf) {
        Ok(v) => v,
        Err(e) => {
            send_response(stream, 400, "Bad Request", &serde_json::json!({ "error": format!("Invalid JSON: {}", e) }));
            return;
        }
    };

    let method = match req_val.get("method").and_then(|m| m.as_str()) {
        Some(m) => m,
        None => {
            send_response(stream, 400, "Bad Request", &serde_json::json!({ "error": "Missing 'method' field" }));
            return;
        }
    };

    let params = req_val.get("params").cloned().unwrap_or(Value::Null);
    let result = server_state.execute_method(method, params);

    match result {
        Ok(res) => {
            send_response(stream, 200, "OK", &serde_json::json!({
                "status": "success",
                "result": res
            }));
        }
        Err(err) => {
            send_response(stream, 500, "Internal Error", &serde_json::json!({
                "status": "error",
                "error": err
            }));
        }
    }
}

fn send_response(mut stream: TcpStream, status_code: u16, status_text: &str, body: &Value) {
    let body_str = match serde_json::to_string(body) {
        Ok(s) => s,
        Err(_) => "{\"status\":\"error\",\"error\":\"Serialization failure\"}".to_string(),
    };
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type\r\n\r\n{}",
        status_code, status_text, body_str.len(), body_str
    );
    let _ = stream.write_all(response.as_bytes());
}

fn send_file_response(mut stream: TcpStream, status_code: u16, status_text: &str, mime_type: &str, content: &[u8]) {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        status_code, status_text, mime_type, content.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(content);
}

fn send_html_response(mut stream: TcpStream, status_code: u16, status_text: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n{}",
        status_code, status_text, body.len(), body
    );
    let _ = stream.write_all(response.as_bytes());
}

struct PrefixedStream {
    prefix: std::io::Cursor<Vec<u8>>,
    stream: TcpStream,
}

impl std::io::Read for PrefixedStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.prefix.position() < self.prefix.get_ref().len() as u64 {
            self.prefix.read(buf)
        } else {
            self.stream.read(buf)
        }
    }
}

impl std::io::Write for PrefixedStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

fn handle_websocket(prefixed: PrefixedStream, pty_id: String, server_state: Arc<ServerState>) {
    let _ = prefixed.stream.set_read_timeout(Some(Duration::from_millis(20)));

    let mut ws = match tungstenite::accept(prefixed) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("WebSocket handshake error: {:?}", e);
            return;
        }
    };

    let mut last_agent = None;
    let mut last_event = None;

    loop {
        let (output, running_agent, last_event_val) = {
            let ptys = server_state.ptys.lock().unwrap();
            if let Some(pty) = ptys.get(&pty_id) {
                let mut buf = pty.output_buffer.lock().unwrap();
                let out = String::from_utf8_lossy(&buf).into_owned();
                buf.clear();
                (out, pty.running_agent.clone(), pty.last_event.clone())
            } else {
                break;
            }
        };

        let agent_changed = running_agent != last_agent;
        let event_changed = last_event_val != last_event;
        if !output.is_empty() || agent_changed || event_changed {
            last_agent = running_agent.clone();
            last_event = last_event_val.clone();

            let payload = serde_json::json!({
                "output": output,
                "running_agent": last_agent,
                "last_event": last_event
            });
            if let Ok(msg_str) = serde_json::to_string(&payload) {
                if let Err(_) = ws.send(tungstenite::Message::Text(msg_str)) {
                    break;
                }
            }
        }

        match ws.read() {
            Ok(msg) => {
                match msg {
                    tungstenite::Message::Text(text) => {
                        let ptys = server_state.ptys.lock().unwrap();
                        if let Some(pty) = ptys.get(&pty_id) {
                            let mut w = pty.writer.lock().unwrap();
                            let _ = w.write_all(text.as_bytes());
                            let _ = w.flush();
                        } else {
                            break;
                        }
                    }
                    tungstenite::Message::Binary(bin) => {
                        let ptys = server_state.ptys.lock().unwrap();
                        if let Some(pty) = ptys.get(&pty_id) {
                            let mut w = pty.writer.lock().unwrap();
                            let _ = w.write_all(&bin);
                            let _ = w.flush();
                        } else {
                            break;
                        }
                    }
                    tungstenite::Message::Close(_) => {
                        break;
                    }
                    _ => {}
                }
            }
            Err(tungstenite::Error::Io(ref err)) if err.kind() == std::io::ErrorKind::WouldBlock || err.kind() == std::io::ErrorKind::TimedOut => {
                // Timeout expected, just loop
            }
            Err(_) => {
                break;
            }
        }
    }
}


fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut port = 3030;
    let mut host = "127.0.0.1".to_string();

    // Command line args parser
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" | "-p" => {
                if i + 1 < args.len() {
                    if let Ok(p) = args[i + 1].parse::<u16>() {
                        port = p;
                    }
                    i += 1;
                }
            }
            "--host" | "-h" => {
                if i + 1 < args.len() {
                    host = args[i + 1].clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let listener_addr = format!("{}:{}", host, port);
    let listener = TcpListener::bind(&listener_addr).expect("Failed to bind TCP listener");
    println!("Headless IDE server running on http://{}", listener_addr);

    let state = Arc::new(ServerState::new(port));

    for stream in listener.incoming() {
        if let Ok(stream) = stream {
            let state_clone = state.clone();
            thread::spawn(move || {
                handle_connection(stream, state_clone);
            });
        }
    }
}

// --- Automated Test Cases Module ---
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // Helper to send HTTP request
    fn post_rpc(port: u16, method: &str, params: Value) -> Value {
        let payload = serde_json::json!({
            "method": method,
            "params": params
        });
        let body = serde_json::to_string(&payload).unwrap();
        let request = format!(
            "POST /api HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            port, body.len(), body
        );

        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.flush().unwrap();

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let mut content_len = 0;
        
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if line.to_lowercase().starts_with("content-length:") {
                content_len = line.split(':').nth(1).unwrap().trim().parse::<usize>().unwrap();
            }
        }

        let mut body_buf = vec![0u8; content_len];
        reader.read_exact(&mut body_buf).unwrap();
        serde_json::from_slice(&body_buf).unwrap()
    }

    // Helper to spin up a server instance on a random port for testing
    fn spawn_test_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(ServerState::new(port));
        
        thread::spawn(move || {
            for stream in listener.incoming() {
                if let Ok(stream) = stream {
                    let state_clone = state.clone();
                    thread::spawn(move || {
                        handle_connection(stream, state_clone);
                    });
                }
            }
        });
        port
    }

    #[test]
    fn test_file_system_operations() {
        let port = spawn_test_server();
        let tmp_dir = std::env::temp_dir().join(format!("ghost_mux_test_{}", port));
        let _ = fs::remove_dir_all(&tmp_dir);
        fs::create_dir_all(&tmp_dir).unwrap();

        // 1. Create a file
        let test_file = tmp_dir.join("hello.txt");
        let create_res = post_rpc(port, "fs.create_file", serde_json::json!({
            "path": test_file.to_string_lossy().to_string()
        }));
        assert_eq!(create_res.get("status").unwrap().as_str().unwrap(), "success");

        // 2. Write to the file
        let write_res = post_rpc(port, "fs.write_file", serde_json::json!({
            "path": test_file.to_string_lossy().to_string(),
            "content": "Hello Headless IDE!"
        }));
        assert_eq!(write_res.get("status").unwrap().as_str().unwrap(), "success");

        // 3. Read from the file
        let read_res = post_rpc(port, "fs.read_file", serde_json::json!({
            "path": test_file.to_string_lossy().to_string()
        }));
        assert_eq!(read_res.get("status").unwrap().as_str().unwrap(), "success");
        let content = read_res.get("result").unwrap().get("content").unwrap().as_str().unwrap();
        assert_eq!(content, "Hello Headless IDE!");

        // 4. List directory entries
        let list_res = post_rpc(port, "fs.list_dir", serde_json::json!({
            "path": tmp_dir.to_string_lossy().to_string()
        }));
        assert_eq!(list_res.get("status").unwrap().as_str().unwrap(), "success");
        let entries = list_res.get("result").unwrap().get("entries").unwrap().as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].get("name").unwrap().as_str().unwrap(), "hello.txt");

        // Cleanup
        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_pty_spawning_and_screen() {
        let port = spawn_test_server();

        // 1. Spawn PTY
        let spawn_res = post_rpc(port, "pty.spawn", serde_json::json!({
            "cols": 80,
            "rows": 24
        }));
        assert_eq!(spawn_res.get("status").unwrap().as_str().unwrap(), "success");
        let pty_id = spawn_res.get("result").unwrap().get("pty_id").unwrap().as_str().unwrap();

        // 2. Write command "echo 'headless'" into PTY
        let write_res = post_rpc(port, "pty.write", serde_json::json!({
            "pty_id": pty_id,
            "input": "echo 'headless'\n"
        }));
        assert_eq!(write_res.get("status").unwrap().as_str().unwrap(), "success");

        // Allow shell process to execute command and populate screen buffer
        thread::sleep(Duration::from_millis(500));

        // 3. Get screen content and verify output
        let screen_res = post_rpc(port, "pty.get_screen", serde_json::json!({
            "pty_id": pty_id
        }));
        assert_eq!(screen_res.get("status").unwrap().as_str().unwrap(), "success");
        let rows = screen_res.get("result").unwrap().get("rows").unwrap().as_array().unwrap();
        
        let mut found = false;
        for row in rows {
            let row_str = row.as_str().unwrap();
            if row_str.contains("headless") {
                found = true;
                break;
            }
        }
        assert!(found, "PTY screen did not contain word 'headless'. Entire screen: {:?}", rows);

        // 4. Test agent hook triggering
        let hook_request = format!(
            "POST /hook/{}/Start HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            pty_id
        );
        let mut stream = TcpStream::connect(format!("127.0.0.1:{}", port)).unwrap();
        stream.write_all(hook_request.as_bytes()).unwrap();
        stream.flush().unwrap();

        // Read response
        let mut hook_res = String::new();
        stream.read_to_string(&mut hook_res).unwrap();
        assert!(hook_res.contains("200 OK"));

        // Query screen again and verify the agent status is active
        let screen_res_2 = post_rpc(port, "pty.get_screen", serde_json::json!({
            "pty_id": pty_id
        }));
        let running_agent = screen_res_2.get("result").unwrap().get("running_agent");
        assert_eq!(running_agent.unwrap().as_str().unwrap(), "ActiveAgent");

        // 5. Close PTY
        let close_res = post_rpc(port, "pty.close", serde_json::json!({
            "pty_id": pty_id
        }));
        assert_eq!(close_res.get("status").unwrap().as_str().unwrap(), "success");
    }

    #[test]
    fn test_git_status() {
        let port = spawn_test_server();
        let git_res = post_rpc(port, "git.status", serde_json::json!({
            "cwd": std::env::current_dir().unwrap().to_string_lossy().to_string()
        }));
        assert_eq!(git_res.get("status").unwrap().as_str().unwrap(), "success");
        let branch = git_res.get("result").unwrap().get("branch").unwrap().as_str().unwrap();
        assert!(!branch.is_empty());
    }
}
