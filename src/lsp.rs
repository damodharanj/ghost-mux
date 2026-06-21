use anyhow::{anyhow, Result};
use async_channel::{bounded, Receiver, Sender};
use futures::channel::oneshot;
use gpui::{App, Context, Task, Window};
use gpui_component::input::{
    CompletionProvider, DefinitionProvider, HoverProvider, InputState, RopeExt, Rope,
};
use lsp_types::{
    CompletionContext, CompletionResponse, InitializeParams, ClientCapabilities, TraceValue, Uri,
};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use url::Url;

pub fn path_to_uri(path: &Path) -> Result<Uri> {
    let url = Url::from_file_path(path).map_err(|_| anyhow!("invalid path"))?;
    let uri: Uri = url.as_str().parse().map_err(|e| anyhow!("{}", e))?;
    Ok(uri)
}

pub fn uri_to_path(uri: &Uri) -> Result<PathBuf> {
    let url = Url::parse(uri.as_str()).map_err(|e| anyhow!("{}", e))?;
    let path = url.to_file_path().map_err(|_| anyhow!("invalid file url"))?;
    Ok(path)
}

pub struct LspClient {
    child: Mutex<Option<std::process::Child>>,
    writer: Arc<Mutex<Option<std::process::ChildStdin>>>,
    next_id: Arc<Mutex<i64>>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>>,
    diagnostics_tx: Sender<(Uri, Vec<lsp_types::Diagnostic>)>,
    pub diagnostics_rx: Receiver<(Uri, Vec<lsp_types::Diagnostic>)>,
    is_initialized: std::sync::atomic::AtomicBool,
    outgoing_queue: Mutex<Vec<String>>,
    spawn_error: Option<String>,
}

fn log_msg(msg: &str) {
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("lsp.log")
    {
        let _ = writeln!(file, "[{}] {}", std::thread::current().name().unwrap_or("unknown"), msg);
    }
}

impl LspClient {
    pub fn start(
        workspace_root: &Path,
        cmd: &[String],
        _language: &str,
    ) -> Result<Arc<Self>> {
        if cmd.is_empty() {
            return Err(anyhow!("Empty server command"));
        }

        log_msg(&format!("Starting LSP server command: {:?} in {:?}", cmd, workspace_root));

        let spawn_res = Command::new(&cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(workspace_root)
            .spawn();

        let (child, stdin, stdout, stderr, spawn_error) = match spawn_res {
            Ok(mut c) => {
                let stdin = c.stdin.take();
                let stdout = c.stdout.take();
                let stderr = c.stderr.take();
                (Some(c), stdin, stdout, stderr, None)
            }
            Err(e) => {
                let err_msg = format!("{}", e);
                log_msg(&format!("Failed to spawn command {:?}: {}", cmd, e));
                (None, None, None, None, Some(err_msg))
            }
        };

        let writer = Arc::new(Mutex::new(stdin));
        let next_id = Arc::new(Mutex::new(0));
        let pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value>>>>> = Arc::new(Mutex::new(HashMap::new()));
        let (diagnostics_tx, diagnostics_rx) = bounded(128);

        let pending_clone = pending.clone();
        let diagnostics_tx_clone = diagnostics_tx.clone();

        // Stdout reader thread
        if let Some(stdout) = stdout {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line.is_empty() {
                        log_msg("LSP stdout connection closed.");
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

                    // Read exact body
                    let mut body = vec![0u8; len];
                    if reader.read_exact(&mut body).is_err() {
                        break;
                    }

                    let Ok(json) = serde_json::from_slice::<Value>(&body) else {
                        continue;
                    };

                    log_msg(&format!("LSP Stdout JSON: {}", json.to_string()));

                    // If response, resolve pending future
                    if let Some(id) = json.get("id").and_then(|v| v.as_i64()) {
                        let mut pending = pending_clone.lock().unwrap();
                        if let Some(tx) = pending.remove(&id) {
                            if let Some(error) = json.get("error") {
                                let _ = tx.send(Err(anyhow!("{}", error)));
                            } else if let Some(result) = json.get("result") {
                                let _ = tx.send(Ok(result.clone()));
                            } else {
                                let _ = tx.send(Ok(Value::Null));
                            }
                        }
                    } else if let Some(method) = json.get("method").and_then(|v| v.as_str()) {
                        if method == "textDocument/publishDiagnostics" {
                            if let Some(params) = json.get("params") {
                                if let Ok(published) = serde_json::from_value::<lsp_types::PublishDiagnosticsParams>(params.clone()) {
                                    log_msg(&format!("Publishing diagnostics for URI: {:?}, count: {}", published.uri, published.diagnostics.len()));
                                    let _ = diagnostics_tx_clone.send_blocking((published.uri, published.diagnostics));
                                }
                            }
                        }
                    }
                }
            });
        }

        // Stderr reader thread (consume to prevent buffer block)
        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok() && !line.is_empty() {
                    log_msg(&format!("LSP Stderr: {}", line.trim()));
                    line.clear();
                }
            });
        }

        let client = Arc::new(Self {
            child: Mutex::new(child),
            writer,
            next_id,
            pending,
            diagnostics_tx,
            diagnostics_rx,
            is_initialized: std::sync::atomic::AtomicBool::new(false),
            outgoing_queue: Mutex::new(vec![]),
            spawn_error,
        });

        // Async Handshake (send initialize)
        if client.spawn_error.is_none() {
            let root_uri = path_to_uri(workspace_root).ok();
            let init_params = InitializeParams {
                process_id: Some(std::process::id()),
                root_path: Some(workspace_root.to_string_lossy().to_string()),
                root_uri,
                initialization_options: None,
                capabilities: ClientCapabilities {
                    text_document: Some(lsp_types::TextDocumentClientCapabilities {
                        completion: Some(lsp_types::CompletionClientCapabilities {
                            completion_item: Some(lsp_types::CompletionItemCapability {
                                snippet_support: Some(true),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            };

            let client_clone = client.clone();
            std::thread::spawn(move || {
                if let Ok(rx) = client_clone.send_request("initialize", init_params) {
                    // Wait for initialize response
                    if futures::executor::block_on(rx).is_ok() {
                        let _ = client_clone.send_notification("initialized", Value::Object(serde_json::Map::new()));
                        
                        // Mark as initialized and flush queued outgoing messages
                        client_clone.is_initialized.store(true, std::sync::atomic::Ordering::SeqCst);
                        let mut queue = client_clone.outgoing_queue.lock().unwrap();
                        let mut stdin_guard = client_clone.writer.lock().unwrap();
                        if let Some(ref mut stdin) = *stdin_guard {
                            for content in queue.drain(..) {
                                if let Err(e) = stdin.write_all(content.as_bytes()).and_then(|_| stdin.flush()) {
                                    log_msg(&format!("Failed to flush queued message: {}", e));
                                }
                            }
                        }
                    }
                }
            });
        }

        Ok(client)
    }

    pub fn send_request<T: serde::Serialize>(
        &self,
        method: &str,
        params: T,
    ) -> Result<oneshot::Receiver<Result<Value>>> {
        let id = {
            let mut next_id = self.next_id.lock().unwrap();
            *next_id += 1;
            *next_id
        };

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&req)?;
        log_msg(&format!("Sending Request ID {}: {}", id, payload));
        let content = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            pending.insert(id, tx);
        }

        if self.spawn_error.is_some() {
            return Err(anyhow!("Cannot send request: LSP failed to start: {:?}", self.spawn_error));
        }

        if self.is_initialized.load(std::sync::atomic::Ordering::SeqCst) || method == "initialize" {
            let mut stdin_guard = self.writer.lock().unwrap();
            if let Some(ref mut stdin) = *stdin_guard {
                stdin.write_all(content.as_bytes())?;
                stdin.flush()?;
            } else {
                return Err(anyhow!("stdin not available"));
            }
        } else {
            let mut queue = self.outgoing_queue.lock().unwrap();
            queue.push(content);
        }

        Ok(rx)
    }

    pub fn send_notification<T: serde::Serialize>(
        &self,
        method: &str,
        params: T,
    ) -> Result<()> {
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&req)?;
        log_msg(&format!("Sending Notification ({}): {}", method, payload));
        let content = format!("Content-Length: {}\r\n\r\n{}", payload.len(), payload);

        if self.spawn_error.is_some() {
            return Err(anyhow!("Cannot send notification: LSP failed to start: {:?}", self.spawn_error));
        }

        if self.is_initialized.load(std::sync::atomic::Ordering::SeqCst) || method == "initialized" {
            let mut stdin_guard = self.writer.lock().unwrap();
            if let Some(ref mut stdin) = *stdin_guard {
                stdin.write_all(content.as_bytes())?;
                stdin.flush()?;
            } else {
                return Err(anyhow!("stdin not available"));
            }
        } else {
            let mut queue = self.outgoing_queue.lock().unwrap();
            queue.push(content);
        }

        Ok(())
    }

    pub fn status(&self) -> String {
        if let Some(ref err) = self.spawn_error {
            return format!("Failed to start ({})", err);
        }
        if let Ok(mut guard) = self.child.lock() {
            if let Some(ref mut child) = *guard {
                match child.try_wait() {
                    Ok(None) => "Running".to_string(),
                    Ok(Some(status)) => {
                        if let Some(code) = status.code() {
                            format!("Exited ({})", code)
                        } else {
                            "Exited".to_string()
                        }
                    }
                    Err(e) => format!("Error: {}", e),
                }
            } else {
                "Not started".to_string()
            }
        } else {
            "Locked".to_string()
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|g| g.as_ref().map(|c| c.id()))
    }

    pub fn is_initialized(&self) -> bool {
        self.is_initialized.load(std::sync::atomic::Ordering::SeqCst)
    }
}

// --- Providers ---

#[derive(Clone)]
pub struct GhostCompletionProvider {
    pub client: Arc<LspClient>,
    pub file_path: PathBuf,
}

impl CompletionProvider for GhostCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let uri = match path_to_uri(&self.file_path) {
            Ok(u) => u,
            Err(_) => return Task::ready(Err(anyhow!("Invalid file path"))),
        };
        let position = text.offset_to_position(offset);
        let client = self.client.clone();

        cx.background_executor().spawn(async move {
            let params = lsp_types::CompletionParams {
                text_document_position: lsp_types::TextDocumentPositionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri },
                    position,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            };

            let rx = client.send_request("textDocument/completion", params)?;
            let val = rx.await??;
            let resp: CompletionResponse = serde_json::from_value(val)?;
            Ok(resp)
        })
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        new_text
            .chars()
            .last()
            .map(|c| c.is_alphanumeric() || c == '.' || c == ':')
            .unwrap_or(false)
    }
}

#[derive(Clone)]
pub struct GhostHoverProvider {
    pub client: Arc<LspClient>,
    pub file_path: PathBuf,
}

impl HoverProvider for GhostHoverProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>> {
        let uri = match path_to_uri(&self.file_path) {
            Ok(u) => u,
            Err(_) => return Task::ready(Err(anyhow!("Invalid file path"))),
        };
        let position = text.offset_to_position(offset);
        let client = self.client.clone();

        cx.background_executor().spawn(async move {
            let params = lsp_types::HoverParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri },
                    position,
                },
                work_done_progress_params: Default::default(),
            };

            let rx = client.send_request("textDocument/hover", params)?;
            let val = rx.await??;
            let resp: Option<lsp_types::Hover> = serde_json::from_value(val)?;
            Ok(resp)
        })
    }
}

#[derive(Clone)]
pub struct GhostDefinitionProvider {
    pub client: Arc<LspClient>,
    pub file_path: PathBuf,
}

impl DefinitionProvider for GhostDefinitionProvider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<lsp_types::LocationLink>>> {
        let uri = match path_to_uri(&self.file_path) {
            Ok(u) => u,
            Err(_) => return Task::ready(Err(anyhow!("Invalid file path"))),
        };
        let position = text.offset_to_position(offset);
        let client = self.client.clone();

        cx.background_executor().spawn(async move {
            let params = lsp_types::GotoDefinitionParams {
                text_document_position_params: lsp_types::TextDocumentPositionParams {
                    text_document: lsp_types::TextDocumentIdentifier { uri },
                    position,
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            };

            let rx = client.send_request("textDocument/definition", params)?;
            let val = rx.await??;
            let resp: lsp_types::GotoDefinitionResponse = serde_json::from_value(val)?;

            let links = match resp {
                lsp_types::GotoDefinitionResponse::Scalar(loc) => vec![lsp_types::LocationLink {
                    origin_selection_range: None,
                    target_uri: loc.uri,
                    target_range: loc.range,
                    target_selection_range: loc.range,
                }],
                lsp_types::GotoDefinitionResponse::Array(locs) => locs
                    .into_iter()
                    .map(|loc| lsp_types::LocationLink {
                        origin_selection_range: None,
                        target_uri: loc.uri,
                        target_range: loc.range,
                        target_selection_range: loc.range,
                    })
                    .collect(),
                lsp_types::GotoDefinitionResponse::Link(links) => links,
            };
            Ok(links)
        })
    }
}

// --- Compound Providers ---

pub struct CompoundCompletionProvider {
    pub providers: Vec<GhostCompletionProvider>,
}

impl CompletionProvider for CompoundCompletionProvider {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let mut tasks = vec![];
        for provider in &self.providers {
            tasks.push(provider.completions(text, offset, trigger.clone(), window, cx));
        }

        cx.background_executor().spawn(async move {
            let mut all_items = vec![];
            for task in tasks {
                if let Ok(resp) = task.await {
                    match resp {
                        CompletionResponse::Array(items) => all_items.extend(items),
                        CompletionResponse::List(list) => all_items.extend(list.items),
                    }
                }
            }
            Ok(CompletionResponse::Array(all_items))
        })
    }

    fn is_completion_trigger(
        &self,
        offset: usize,
        new_text: &str,
        cx: &mut Context<InputState>,
    ) -> bool {
        self.providers
            .iter()
            .any(|p| p.is_completion_trigger(offset, new_text, cx))
    }
}

pub struct CompoundHoverProvider {
    pub providers: Vec<GhostHoverProvider>,
}

impl HoverProvider for CompoundHoverProvider {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<lsp_types::Hover>>> {
        let mut tasks = vec![];
        for provider in &self.providers {
            tasks.push(provider.hover(text, offset, window, cx));
        }

        cx.background_executor().spawn(async move {
            let mut final_hover = None;
            for task in tasks {
                if let Ok(Some(hover)) = task.await {
                    if final_hover.is_none() {
                        final_hover = Some(hover);
                    } else if let Some(ref mut fh) = final_hover {
                        match (&mut fh.contents, hover.contents) {
                            (
                                lsp_types::HoverContents::Markup(m1),
                                lsp_types::HoverContents::Markup(m2),
                            ) => {
                                m1.value = format!("{}\n\n---\n\n{}", m1.value, m2.value);
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(final_hover)
        })
    }
}

pub struct CompoundDefinitionProvider {
    pub providers: Vec<GhostDefinitionProvider>,
}

impl DefinitionProvider for CompoundDefinitionProvider {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<lsp_types::LocationLink>>> {
        let mut tasks = vec![];
        for provider in &self.providers {
            tasks.push(provider.definitions(text, offset, window, cx));
        }

        cx.background_executor().spawn(async move {
            let mut all_links = vec![];
            for task in tasks {
                if let Ok(links) = task.await {
                    all_links.extend(links);
                }
            }
            Ok(all_links)
        })
    }
}
