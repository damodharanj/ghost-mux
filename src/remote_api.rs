use std::path::{Path, PathBuf};
use serde_json::Value;

/// Performs a synchronous JSON-RPC POST call over HTTP/HTTPS to the headless server.
pub fn call_remote_api(server_url: &str, method: &str, params: Value) -> Result<Value, String> {
    let mut url = server_url.to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        url = format!("http://{}", url);
    }

    // Ensure the path is routed to /api to align native client and web client request paths
    if !url.ends_with("/api") && !url.ends_with("/api/") {
        if url.ends_with('/') {
            url = format!("{}api", url);
        } else {
            url = format!("{}/api", url);
        }
    }

    let req = serde_json::json!({
        "method": method,
        "params": params,
    });

    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_json(req)
        .map_err(|e| format!("HTTP request to {} failed: {}", url, e))?;

    let resp_val: Value = resp.into_json()
        .map_err(|e| format!("Failed to parse JSON response: {}", e))?;

    if let Some(status) = resp_val.get("status").and_then(|s| s.as_str()) {
        if status == "success" {
            if let Some(res) = resp_val.get("result") {
                return Ok(res.clone());
            }
        } else if let Some(err) = resp_val.get("error").and_then(|e| e.as_str()) {
            return Err(err.to_string());
        }
    }

    Err("Invalid server response format".to_string())
}

pub struct RemoteDirEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

pub fn read_directory(dir: &Path, server_url: &Option<String>) -> Result<Vec<RemoteDirEntry>, String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({ "path": dir.to_string_lossy().to_string() });
        let res = call_remote_api(url, "fs.list_dir", params)?;
        let mut entries = Vec::new();
        if let Some(arr) = res.get("entries").and_then(|e| e.as_array()) {
            for entry_val in arr {
                if let (Some(name), Some(path_str), Some(is_dir)) = (
                    entry_val.get("name").and_then(|n| n.as_str()),
                    entry_val.get("path").and_then(|p| p.as_str()),
                    entry_val.get("is_dir").and_then(|d| d.as_bool()),
                ) {
                    let size = entry_val.get("size").and_then(|s| s.as_u64()).unwrap_or(0);
                    entries.push(RemoteDirEntry {
                        name: name.to_string(),
                        path: PathBuf::from(path_str),
                        is_dir,
                        size,
                    });
                }
            }
        }
        Ok(entries)
    } else {
        let std_entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for entry in std_entries {
            if let Ok(entry) = entry {
                if let Ok(meta) = entry.metadata() {
                    entries.push(RemoteDirEntry {
                        name: entry.file_name().to_string_lossy().to_string(),
                        path: entry.path(),
                        is_dir: meta.is_dir(),
                        size: meta.len(),
                    });
                }
            }
        }
        Ok(entries)
    }
}

pub fn read_file_content(path: &Path, server_url: &Option<String>) -> Result<String, String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({ "path": path.to_string_lossy().to_string() });
        let res = call_remote_api(url, "fs.read_file", params)?;
        res.get("content")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid response".to_string())
    } else {
        std::fs::read_to_string(path).map_err(|e| e.to_string())
    }
}

pub fn write_file_content(path: &Path, content: &str, server_url: &Option<String>) -> Result<(), String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({
            "path": path.to_string_lossy().to_string(),
            "content": content
        });
        let _ = call_remote_api(url, "fs.write_file", params)?;
        Ok(())
    } else {
        std::fs::write(path, content).map_err(|e| e.to_string())
    }
}

pub fn rename_file(source: &Path, target: &Path, server_url: &Option<String>) -> Result<(), String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({
            "src": source.to_string_lossy().to_string(),
            "dst": target.to_string_lossy().to_string()
        });
        let _ = call_remote_api(url, "fs.rename", params)?;
        Ok(())
    } else {
        std::fs::rename(source, target).map_err(|e| e.to_string())
    }
}

pub fn create_file(path: &Path, server_url: &Option<String>) -> Result<(), String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({ "path": path.to_string_lossy().to_string() });
        let _ = call_remote_api(url, "fs.create_file", params)?;
        Ok(())
    } else {
        std::fs::File::create(path).map(|_| ()).map_err(|e| e.to_string())
    }
}

pub fn create_dir(path: &Path, server_url: &Option<String>) -> Result<(), String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({ "path": path.to_string_lossy().to_string() });
        let _ = call_remote_api(url, "fs.create_dir", params)?;
        Ok(())
    } else {
        std::fs::create_dir_all(path).map_err(|e| e.to_string())
    }
}

pub fn delete_file_or_dir(path: &Path, recursive: bool, server_url: &Option<String>) -> Result<(), String> {
    if let Some(ref url) = server_url {
        let params = serde_json::json!({
            "path": path.to_string_lossy().to_string(),
            "recursive": recursive
        });
        let _ = call_remote_api(url, "fs.delete", params)?;
        Ok(())
    } else {
        if path.is_dir() {
            if recursive {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_dir(path).map_err(|e| e.to_string())
            }
        } else {
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }
    }
}

pub fn canonicalize_path(p: &Path, server_url: &Option<String>) -> PathBuf {
    if server_url.is_some() {
        p.to_path_buf()
    } else {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }
}
