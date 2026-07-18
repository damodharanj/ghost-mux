import React, { useState, useEffect, useRef, useContext, createContext } from 'react';
import { Terminal } from 'xterm';
import { FitAddon } from 'xterm-addon-fit';

// --- Workspace Layout tree node representation ---
class LayoutNode {
  constructor(type, id) {
    this.type = type; // 'leaf', 'hsplit', 'vsplit'
    this.id = id;
    this.ratio = 0.5;
    this.left = null;
    this.right = null;
    this.tabs = [];
    this.activeTabIndex = 0;
  }
}

// --- Serialization & Deserialization Helpers ---
let nextNodeId = 1;

function serializeNode(node) {
  if (!node) return null;
  if (node.type === 'leaf') {
    return {
      id: node.id,
      type: node.type,
      tabs: node.tabs,
      activeTabIndex: node.activeTabIndex
    };
  }
  return {
    id: node.id,
    type: node.type,
    ratio: node.ratio,
    left: serializeNode(node.left),
    right: serializeNode(node.right)
  };
}

function deserializeNode(obj) {
  if (!obj) return null;
  const node = new LayoutNode(obj.type, obj.id);
  node.ratio = obj.ratio;
  node.left = deserializeNode(obj.left);
  node.right = deserializeNode(obj.right);

  if (obj.type === 'leaf') {
    if (obj.tabs) {
      node.tabs = obj.tabs;
      node.activeTabIndex = obj.activeTabIndex || 0;
    } else {
      node.tabs = [];
      if (obj.ptyId) {
        node.tabs.push({
          id: nextNodeId++,
          type: 'terminal',
          title: `Terminal (${obj.ptyId})`,
          data: { ptyId: obj.ptyId }
        });
      } else {
        node.tabs.push({
          id: nextNodeId++,
          type: 'terminal',
          title: 'Terminal (New)',
          data: { ptyId: null }
        });
      }
      node.activeTabIndex = 0;
    }
  }

  if (obj.id >= nextNodeId) {
    nextNodeId = obj.id + 1;
  }

  return node;
}

function findNodeById(node, id) {
  if (!node) return null;
  if (node.id === id) return node;
  if (node.type !== 'leaf') {
    const left = findNodeById(node.left, id);
    if (left) return left;
    return findNodeById(node.right, id);
  }
  return null;
}

function findParentNode(root, id) {
  if (!root || root.type === 'leaf') return null;
  if ((root.left && root.left.id === id) || (root.right && root.right.id === id)) {
    return root;
  }
  const left = findParentNode(root.left, id);
  if (left) return left;
  return findParentNode(root.right, id);
}

function getAllLeaves(node, list = []) {
  if (!node) return list;
  if (node.type === 'leaf') {
    list.push(node);
  } else {
    getAllLeaves(node.left, list);
    getAllLeaves(node.right, list);
  }
  return list;
}

// Global Terminal Instances Registry
const activeTerminals = {};

// --- Workspace State Context ---
const WorkspaceContext = createContext(null);

// --- Component: Terminal Tab ---
function TerminalTab({ ptyId }) {
  const containerRef = useRef(null);
  const { serverUrl, isConnected, onPtyEvent } = useContext(WorkspaceContext);

  useEffect(() => {
    if (!ptyId) return;

    let session = activeTerminals[ptyId];
    if (!session) {
      const term = new Terminal({
        cursorBlink: true,
        fontFamily: 'Fira Code, Menlo, Monaco, monospace',
        fontSize: 12.5,
        lineHeight: 1.2,
        theme: {
          background: '#1e1f22',
          foreground: '#d4d4d4',
          cursor: '#007acc',
          cursorAccent: '#ffffff',
          selectionBackground: 'rgba(0, 122, 204, 0.3)',
          black: '#1e1f22',
          red: '#f47067',
          green: '#57c994',
          yellow: '#d4c270',
          blue: '#54b1ff',
          magenta: '#db82ff',
          cyan: '#5ffffe',
          white: '#ffffff'
        }
      });

      const fitAddon = new FitAddon();
      term.loadAddon(fitAddon);

      const terminalContainer = document.createElement('div');
      terminalContainer.className = 'terminal-container';
      term.open(terminalContainer);

      let ws = null;
      let reconnectTimeout = null;
      let shouldReconnect = true;

      const getWebSocketUrl = (id) => {
        const wsUrl = serverUrl.replace(/^http/, 'ws');
        return `${wsUrl}/ws?pty_id=${id}`;
      };

      const connectWs = () => {
        if (!shouldReconnect) return;
        const wsUrl = getWebSocketUrl(ptyId);
        ws = new WebSocket(wsUrl);

        ws.onmessage = (event) => {
          try {
            const data = JSON.parse(event.data);
            if (data.output) {
              term.write(data.output);
            }
            if (onPtyEvent) {
              onPtyEvent(ptyId, data.running_agent, data.last_event);
            }
          } catch (e) {
            console.error("Failed to parse WebSocket message:", e);
          }
        };

        ws.onclose = () => {
          if (shouldReconnect) {
            clearTimeout(reconnectTimeout);
            reconnectTimeout = setTimeout(connectWs, 2000);
          }
        };

        ws.onerror = (err) => {
          console.error(`WebSocket error for PTY ${ptyId}`, err);
        };
      };

      connectWs();

      term.onData((data) => {
        if (ws && ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        } else if (isConnected) {
          fetch(`${serverUrl}/api`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
              method: 'pty.write',
              params: { pty_id: ptyId, input: data }
            })
          }).catch(err => console.error("PTY Write failed", err));
        }
      });

      let resizeTimeout;
      term.onResize((size) => {
        clearTimeout(resizeTimeout);
        resizeTimeout = setTimeout(() => {
          if (isConnected) {
            fetch(`${serverUrl}/api`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                method: 'pty.resize',
                params: { pty_id: ptyId, cols: size.cols, rows: size.rows }
              })
            }).catch(err => console.error("PTY Resize failed", err));
          }
        }, 150);
      });

      session = {
        term,
        fitAddon,
        container: terminalContainer,
        close: () => {
          shouldReconnect = false;
          clearTimeout(reconnectTimeout);
          if (ws) ws.close();
          term.dispose();
        }
      };

      activeTerminals[ptyId] = session;
    }

    const container = containerRef.current;
    if (container) {
      container.appendChild(session.container);
      requestAnimationFrame(() => {
        try {
          session.fitAddon.fit();
        } catch (e) {}
      });
    }

    let resizeObserver = null;
    if (container) {
      resizeObserver = new ResizeObserver(() => {
        try {
          session.fitAddon.fit();
        } catch (e) {}
      });
      resizeObserver.observe(container);
    }

    return () => {
      if (container && session.container.parentNode === container) {
        container.removeChild(session.container);
      }
      if (resizeObserver) {
        resizeObserver.disconnect();
      }
    };
  }, [ptyId, serverUrl, isConnected]);

  return <div ref={containerRef} style={{ width: '100%', height: '100%', overflow: 'hidden' }} />;
}

// --- Component: Editor Tab ---
function EditorTab({ tab }) {
  const [content, setContent] = useState(tab.data.content || '');
  const textareaRef = useRef(null);
  const lineNumbersRef = useRef(null);
  const { callRpc, saveLayout } = useContext(WorkspaceContext);

  useEffect(() => {
    setContent(tab.data.content || '');
  }, [tab.data.path, tab.data.content]);

  const handleScroll = () => {
    if (lineNumbersRef.current && textareaRef.current) {
      lineNumbersRef.current.scrollTop = textareaRef.current.scrollTop;
    }
  };

  const handleInput = (e) => {
    const val = e.target.value;
    setContent(val);
    tab.data.content = val;
    saveLayout();
  };

  const handleKeyDown = (e) => {
    if (e.key === 'Tab') {
      e.preventDefault();
      const textarea = textareaRef.current;
      const start = textarea.selectionStart;
      const end = textarea.selectionEnd;
      const val = textarea.value;
      const newVal = val.substring(0, start) + "    " + val.substring(end);
      textarea.value = newVal;
      textarea.selectionStart = textarea.selectionEnd = start + 4;
      setContent(newVal);
      tab.data.content = newVal;
      saveLayout();
    }
  };

  const handleSave = async () => {
    try {
      await callRpc('fs.write_file', { path: tab.data.path, content });
      tab.data.originalContent = content;
      saveLayout();
      alert("File saved successfully!");
    } catch (err) {
      alert("Failed to save file: " + err.message);
    }
  };

  const lines = content.split('\n');

  return (
    <div className="editor-wrapper" style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <div className="editor-main" style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
        <div ref={lineNumbersRef} className="editor-line-numbers" style={{ overflow: 'hidden' }}>
          {lines.map((_, i) => <div key={i}>{i + 1}</div>)}
        </div>
        <textarea
          ref={textareaRef}
          className="editor-textarea"
          spellCheck={false}
          value={content}
          onScroll={handleScroll}
          onChange={handleInput}
          onKeyDown={handleKeyDown}
        />
      </div>
      <div className="editor-footer" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '6px 12px', borderTop: '1px solid var(--border)' }}>
        <span className="editor-file-path" style={{ fontSize: '11px', color: 'var(--muted-foreground)', fontFamily: 'var(--font-mono)' }}>{tab.data.path}</span>
        <div className="editor-footer-actions">
          <button className="btn btn-primary btn-sm" onClick={handleSave}>Save</button>
        </div>
      </div>
    </div>
  );
}

// --- Component: Diff Tab ---
function DiffTab({ tab }) {
  const lines = (tab.data.diff || '').split('\n');

  return (
    <div className="diff-wrapper pre-scrollable" style={{ width: '100%', height: '100%', overflow: 'auto', padding: '12px' }}>
      <pre className="diff-content-view" style={{ fontFamily: 'var(--font-mono)', fontSize: '12px', lineHeight: '1.4' }}>
        {lines.map((line, idx) => {
          let className = 'diff-line';
          if (line.startsWith('+') && !line.startsWith('+++')) {
            className += ' diff-line-add';
          } else if (line.startsWith('-') && !line.startsWith('---')) {
            className += ' diff-line-del';
          } else if (line.startsWith('@@') || line.startsWith('diff') || line.startsWith('index')) {
            className += ' diff-line-meta';
          }
          return <span key={idx} className={className} style={{ display: 'block' }}>{line}</span>;
        })}
      </pre>
    </div>
  );
}

// --- Component: Browser Tab ---
function BrowserTab({ tab }) {
  const [urlInput, setUrlInput] = useState(tab.data.url || 'https://google.com');
  const [iframeSrc, setIframeSrc] = useState(tab.data.url || 'https://google.com');
  const { saveLayout } = useContext(WorkspaceContext);

  const navigate = () => {
    let url = urlInput.trim();
    if (url) {
      if (!url.startsWith('http://') && !url.startsWith('https://')) {
        url = 'https://' + url;
      }
      setUrlInput(url);
      setIframeSrc(url);
      tab.data.url = url;
      saveLayout();
    }
  };

  const handleKeyDown = (e) => {
    if (e.key === 'Enter') {
      navigate();
    }
  };

  return (
    <div className="browser-wrapper" style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <div className="browser-navbar" style={{ display: 'flex', padding: '6px', gap: '6px', backgroundColor: 'var(--secondary)', borderBottom: '1px solid var(--border)' }}>
        <input
          className="browser-url-input"
          type="text"
          value={urlInput}
          onChange={(e) => setUrlInput(e.target.value)}
          onKeyDown={handleKeyDown}
          style={{ flex: 1, padding: '4px 8px', borderRadius: '4px', border: '1px solid var(--border)', backgroundColor: 'var(--background)', color: 'var(--foreground)' }}
        />
        <button className="btn btn-secondary btn-sm" onClick={navigate}>Go</button>
      </div>
      <iframe className="browser-iframe" src={iframeSrc} title="Web View" style={{ flex: 1, border: 'none', backgroundColor: '#fff' }} />
    </div>
  );
}

// --- Component: Recursive File Tree Node ---
function FileTreeNode({ entry, depth, expandedPaths, toggleExpanded, onOpenFile, onDelete, onRename, onCreateFile, onCreateFolder }) {
  const isExpanded = expandedPaths.has(entry.path);
  const [children, setChildren] = useState([]);
  const [loading, setLoading] = useState(false);
  const { callRpc } = useContext(WorkspaceContext);

  useEffect(() => {
    if (entry.is_dir && isExpanded) {
      let active = true;
      setLoading(true);
      callRpc('fs.list_dir', { path: entry.path })
        .then(res => {
          if (!active) return;
          const sorted = (res.entries || []).filter(e => e.name !== '.git' && e.name !== '.DS_Store' && e.name !== 'target');
          sorted.sort((a, b) => {
            if (a.is_dir && !b.is_dir) return -1;
            if (!a.is_dir && b.is_dir) return 1;
            return a.name.localeCompare(b.name);
          });
          setChildren(sorted);
        })
        .catch(err => console.error("List dir failed", err))
        .finally(() => {
          if (active) setLoading(false);
        });
      return () => { active = false; };
    }
  }, [entry.path, isExpanded, callRpc]);

  const handleRowClick = (e) => {
    if (entry.is_dir) {
      toggleExpanded(entry.path);
    } else {
      onOpenFile(entry.path);
    }
  };

  const icon = entry.is_dir ? '📁' : '📄';

  return (
    <div className="tree-node">
      <div className="tree-row" style={{ paddingLeft: `${depth * 14 + 8}px` }} onClick={handleRowClick}>
        <div className="tree-label">
          <span className="tree-icon" style={{ marginRight: '6px' }}>{icon}</span>
          <span>{entry.name}</span>
        </div>
        <div className="tree-actions">
          {entry.is_dir ? (
            <>
              <button className="tree-action-btn" title="New File" onClick={(e) => { e.stopPropagation(); onCreateFile(entry.path); }}>📄+</button>
              <button className="tree-action-btn" title="New Folder" onClick={(e) => { e.stopPropagation(); onCreateFolder(entry.path); }}>📁+</button>
            </>
          ) : (
            <button className="tree-action-btn" title="Edit File" onClick={(e) => { e.stopPropagation(); onOpenFile(entry.path); }}>✏️</button>
          )}
          <button className="tree-action-btn" title="Rename" onClick={(e) => { e.stopPropagation(); onRename(entry.path, entry.name); }}>🔄</button>
          <button className="tree-action-btn tree-action-delete" title="Delete" onClick={(e) => { e.stopPropagation(); onDelete(entry.path, entry.is_dir); }}>✕</button>
        </div>
      </div>
      {entry.is_dir && isExpanded && (
        <div className="tree-children">
          {loading && <div style={{ paddingLeft: `${(depth + 1) * 14 + 8}px`, color: 'var(--muted-foreground)', fontSize: '11px', padding: '4px' }}>Loading...</div>}
          {!loading && children.map(child => (
            <FileTreeNode
              key={child.path}
              entry={child}
              depth={depth + 1}
              expandedPaths={expandedPaths}
              toggleExpanded={toggleExpanded}
              onOpenFile={onOpenFile}
              onDelete={onDelete}
              onRename={onRename}
              onCreateFile={onCreateFile}
              onCreateFolder={onCreateFolder}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// --- Component: Explorer Tab (and Sidebar Pane) ---
function ExplorerTab({ onOpenFile }) {
  const { callRpc, activeWorkspacePath, activeWorkspaceName } = useContext(WorkspaceContext);
  const [expandedPaths, setExpandedPaths] = useState(new Set([activeWorkspacePath || '.']));
  const [refreshKey, setRefreshKey] = useState(0);

  useEffect(() => {
    setExpandedPaths(new Set([activeWorkspacePath || '.']));
  }, [activeWorkspacePath]);

  const toggleExpanded = (path) => {
    setExpandedPaths(prev => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  const handleRefresh = () => {
    setRefreshKey(prev => prev + 1);
  };

  const handleCreateFile = async (parentPath) => {
    const name = prompt("Enter file name:");
    if (!name) return;
    const fullPath = (parentPath === '.' || !parentPath) ? name : `${parentPath}/${name}`;
    try {
      await callRpc('fs.create_file', { path: fullPath });
      handleRefresh();
    } catch (err) {
      alert("Failed to create file: " + err.message);
    }
  };

  const handleCreateFolder = async (parentPath) => {
    const name = prompt("Enter folder name:");
    if (!name) return;
    const fullPath = (parentPath === '.' || !parentPath) ? name : `${parentPath}/${name}`;
    try {
      await callRpc('fs.create_dir', { path: fullPath });
      handleRefresh();
    } catch (err) {
      alert("Failed to create directory: " + err.message);
    }
  };

  const handleRename = async (oldPath, oldName) => {
    const newName = prompt("Rename entry to:", oldName);
    if (!newName || newName === oldName) return;
    const parts = oldPath.split('/');
    parts.pop();
    parts.push(newName);
    const newPath = parts.join('/');
    try {
      await callRpc('fs.rename', { src: oldPath, dst: newPath });
      handleRefresh();
    } catch (err) {
      alert("Failed to rename: " + err.message);
    }
  };

  const handleDelete = async (path, isDir) => {
    if (!confirm(`Are you sure you want to delete ${path}?`)) return;
    try {
      await callRpc('fs.delete', { path, recursive: isDir });
      handleRefresh();
    } catch (err) {
      alert("Failed to delete: " + err.message);
    }
  };

  const rootPath = activeWorkspacePath || '.';

  return (
    <div className="panel-explorer-wrapper" style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <div className="panel-explorer-header">
        <span>Workspace Directory Tree</span>
        <div className="explorer-actions">
          <button className="action-btn" title="New File" onClick={() => handleCreateFile(rootPath)}>📄+</button>
          <button className="action-btn" title="New Folder" onClick={() => handleCreateFolder(rootPath)}>📁+</button>
          <button className="action-btn" title="Refresh" onClick={handleRefresh}>🔄</button>
        </div>
      </div>
      <div className="file-tree" style={{ flex: 1, overflow: 'auto' }}>
        <FileTreeNode
          key={refreshKey + '-' + rootPath}
          entry={{ name: activeWorkspaceName, path: rootPath, is_dir: true }}
          depth={0}
          expandedPaths={expandedPaths}
          toggleExpanded={toggleExpanded}
          onOpenFile={onOpenFile}
          onDelete={handleDelete}
          onRename={handleRename}
          onCreateFile={handleCreateFile}
          onCreateFolder={handleCreateFolder}
        />
      </div>
    </div>
  );
}

// --- Component: Git Tab ---
function GitTab() {
  const [gitStatus, setGitStatus] = useState(null);
  const [commitMsg, setCommitMsg] = useState('');
  const [pushing, setPushing] = useState(false);
  const { callRpc, isConnected, openGitDiff, activeWorkspacePath } = useContext(WorkspaceContext);

  const loadStatus = async () => {
    if (!isConnected) return;
    try {
      const res = await callRpc('git.status', { cwd: activeWorkspacePath || '.' });
      setGitStatus(res);
    } catch (err) {
      setGitStatus({ error: err.message });
    }
  };

  useEffect(() => {
    loadStatus();
  }, [isConnected, activeWorkspacePath]);

  const handleStageFile = async (path) => {
    try {
      await callRpc('git.add', { cwd: activeWorkspacePath || '.', path });
      loadStatus();
    } catch (err) {
      alert(err.message);
    }
  };

  const handleStageAll = async () => {
    try {
      await callRpc('git.add', { cwd: activeWorkspacePath || '.', path: '.' });
      loadStatus();
    } catch (err) {
      alert(err.message);
    }
  };

  const handleCommit = async () => {
    const msg = commitMsg.trim();
    if (!msg) {
      alert("Please enter a commit message.");
      return;
    }
    try {
      await callRpc('git.commit', { cwd: activeWorkspacePath || '.', message: msg });
      setCommitMsg('');
      loadStatus();
      alert("Committed successfully!");
    } catch (err) {
      alert("Commit failed: " + err.message);
    }
  };

  const handlePush = async () => {
    try {
      setPushing(true);
      await callRpc('git.push', { cwd: activeWorkspacePath || '.' });
      alert("Pushed successfully!");
    } catch (err) {
      alert("Push failed: " + err.message);
    } finally {
      setPushing(false);
      loadStatus();
    }
  };

  if (!gitStatus) {
    return <div className="sessions-placeholder">Loading git status...</div>;
  }

  if (gitStatus.error) {
    return (
      <div className="panel-git-wrapper">
        <div className="panel-git-header">
          <span className="git-branch-title">Git: <strong>None</strong></span>
          <button className="action-btn" onClick={loadStatus}>🔄</button>
        </div>
        <div className="sessions-placeholder">{gitStatus.error}</div>
      </div>
    );
  }

  const files = gitStatus.files || [];

  return (
    <div className="panel-git-wrapper" style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
      <div className="panel-git-header">
        <span className="git-branch-title">Git: <strong>{gitStatus.branch || 'no branch'}</strong></span>
        <button className="action-btn" onClick={loadStatus}>🔄</button>
      </div>
      <div className="panel-git-changes-view" style={{ flex: 1, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div className="git-section-title" style={{ display: 'flex', justifyContent: 'space-between', padding: '4px 8px' }}>
          <span>Staged/Unstaged Changes</span>
          <button className="btn-text" onClick={handleStageAll}>Stage All</button>
        </div>
        <div className="git-changes-list" style={{ flex: 1, overflow: 'auto' }}>
          {files.length === 0 ? (
            <div className="sessions-placeholder">No changes detected.</div>
          ) : (
            files.map(f => {
              let statusLabel = f.status;
              let statusClass = 'git-status-untracked';
              if (f.status === 'M') statusClass = 'git-status-M';
              else if (f.status === 'A') statusClass = 'git-status-A';
              else if (f.status === 'D') statusClass = 'git-status-D';
              else if (f.status === '??') {
                statusLabel = 'U';
                statusClass = 'git-status-untracked';
              }
              return (
                <div key={f.path} className="git-change-item" onClick={() => openGitDiff(f.path)}>
                  <div className="git-file-info">
                    <span className={`git-status-indicator ${statusClass}`}>{statusLabel}</span>
                    <span className="git-file-path" title={f.path}>{f.path}</span>
                  </div>
                  <div className="git-item-actions">
                    <button className="btn-text stage-btn" title="Stage Change" onClick={(e) => { e.stopPropagation(); handleStageFile(f.path); }}>+</button>
                  </div>
                </div>
              );
            })
          )}
        </div>
        <div className="git-commit-box">
          <textarea
            value={commitMsg}
            onChange={(e) => setCommitMsg(e.target.value)}
            placeholder="Commit message... (Cmd+Enter to commit)"
            onKeyDown={(e) => {
              if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
                e.preventDefault();
                handleCommit();
              }
            }}
          />
          <div className="git-commit-actions">
            <button className="btn btn-primary btn-block" onClick={handleCommit}>Commit</button>
            <button className="btn btn-secondary btn-block" onClick={handlePush} disabled={pushing}>{pushing ? 'Pushing...' : 'Push'}</button>
          </div>
        </div>
      </div>
    </div>
  );
}

// --- Component: Diagnostics Tab ---
function DiagnosticsTab() {
  return (
    <div className="diagnostics-wrapper" style={{ width: '100%', height: '100%', display: 'flex', flexDirection: 'column', padding: '12px' }}>
      <div className="diagnostics-header" style={{ fontWeight: 600, marginBottom: '8px' }}>Workspace Diagnostics</div>
      <div className="diagnostics-list" style={{ flex: 1, overflow: 'auto' }}>
        <div className="diagnostics-item-placeholder">
          <span className="diag-icon">✓</span>
          <span className="diag-text">No diagnostics found in workspace. Code is looking good!</span>
        </div>
      </div>
    </div>
  );
}

// --- Component: Tab Content Router ---
function TabContentRenderer({ tab, nodeId, onOpenFile }) {
  const { type } = tab;

  switch (type) {
    case 'terminal':
      return <TerminalTab ptyId={tab.data.ptyId} />;
    case 'editor':
      return <EditorTab tab={tab} nodeId={nodeId} />;
    case 'diff':
      return <DiffTab tab={tab} />;
    case 'browser':
      return <BrowserTab tab={tab} />;
    case 'explorer':
      return <ExplorerTab onOpenFile={onOpenFile} />;
    case 'git':
      return <GitTab />;
    case 'diagnostics':
      return <DiagnosticsTab />;
    default:
      return <div className="sessions-placeholder">Unknown tab type: {type}</div>;
  }
}

// --- Component: Workspace Panel Leaf ---
function WorkspaceLeaf({ node, activePanelId, setActivePanelId, onSplit, onClosePanel, onTabChange, onAddTab, onCloseTab, onOpenFile }) {
  const isActive = activePanelId === node.id;
  const activeTab = node.tabs[node.activeTabIndex];
  const [showDropdown, setShowDropdown] = useState(false);
  const dropdownRef = useRef(null);

  useEffect(() => {
    function handleClickOutside(event) {
      if (dropdownRef.current && !dropdownRef.current.contains(event.target)) {
        setShowDropdown(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const handlePanelClick = () => {
    if (!isActive) {
      setActivePanelId(node.id);
    }
  };

  const handleAddTab = (type) => {
    setShowDropdown(false);
    onAddTab(node.id, type);
  };

  return (
    <div
      className={`workspace-panel ${isActive ? 'active' : ''}`}
      onClick={handlePanelClick}
      style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%', overflow: 'hidden' }}
    >
      {/* Panel Tabs Header */}
      <div className="panel-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <div className="panel-tabs" style={{ display: 'flex', flex: 1, overflowX: 'auto', overflowY: 'hidden' }}>
          {node.tabs.map((tab, idx) => {
            const isTabActive = node.activeTabIndex === idx;
            const isModified = tab.type === 'editor' && tab.data.content !== tab.data.originalContent;
            return (
              <div
                key={tab.id}
                className={`panel-tab-btn ${isTabActive ? 'active' : ''} ${isModified ? 'modified' : ''}`}
                onClick={(e) => {
                  e.stopPropagation();
                  setActivePanelId(node.id);
                  onTabChange(node.id, idx);
                }}
              >
                <span className="tab-title">{tab.title}</span>
                <span
                  className="tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    onCloseTab(node.id, idx);
                  }}
                >
                  &times;
                </span>
              </div>
            );
          })}
          {/* Add Tab Button */}
          <div ref={dropdownRef} className="add-tab-btn-container" style={{ position: 'relative', alignSelf: 'center' }}>
            <button className="add-tab-btn" onClick={(e) => { e.stopPropagation(); setShowDropdown(!showDropdown); }}>+</button>
            {showDropdown && (
              <div className="add-tab-dropdown show" style={{ position: 'absolute', top: '100%', left: 0, zIndex: 1000, minWidth: '120px' }}>
                <div className="dropdown-item" onClick={() => handleAddTab('terminal')}>Terminal</div>
                <div className="dropdown-item" onClick={() => handleAddTab('explorer')}>Explorer</div>
                <div className="dropdown-item" onClick={() => handleAddTab('git')}>Git Status</div>
                <div className="dropdown-item" onClick={() => handleAddTab('browser')}>Web Browser</div>
                <div className="dropdown-item" onClick={() => handleAddTab('diagnostics')}>Diagnostics</div>
              </div>
            )}
          </div>
        </div>

        {/* Panel Toolbar */}
        <div className="panel-toolbar">
          <button className="toolbar-btn" title="Split Horizontally" onClick={(e) => { e.stopPropagation(); onSplit(node.id, 'hsplit'); }}>⬜→</button>
          <button className="toolbar-btn" title="Split Vertically" onClick={(e) => { e.stopPropagation(); onSplit(node.id, 'vsplit'); }}>⬜↓</button>
          <button className="toolbar-btn close-btn" title="Close Panel" onClick={(e) => { e.stopPropagation(); onClosePanel(node.id); }}>✕</button>
        </div>
      </div>

      {/* Tab Content Container */}
      <div className="panel-content" style={{ flex: 1, overflow: 'hidden', position: 'relative' }}>
        {activeTab ? (
          <TabContentRenderer
            tab={activeTab}
            nodeId={node.id}
            onOpenFile={onOpenFile}
          />
        ) : (
          <div className="sessions-placeholder">No active tabs. Click + to add.</div>
        )}
      </div>
    </div>
  );
}

// --- Component: Workspace Panel Split ---
function WorkspaceSplit({ node, activePanelId, setActivePanelId, onSplit, onClosePanel, onTabChange, onAddTab, onCloseTab, onOpenFile }) {
  const containerRef = useRef(null);
  const [ratio, setRatio] = useState(node.ratio);
  const { saveLayout } = useContext(WorkspaceContext);

  useEffect(() => {
    setRatio(node.ratio);
  }, [node.ratio]);

  const handleMouseDown = (e) => {
    e.preventDefault();
    const startX = e.clientX;
    const startY = e.clientY;
    const startRatio = node.ratio;
    const rect = containerRef.current.getBoundingClientRect();

    const handleMouseMove = (ev) => {
      let newRatio;
      if (node.type === 'hsplit') {
        const deltaX = ev.clientX - startX;
        newRatio = Math.max(0.1, Math.min(0.9, startRatio + (deltaX / rect.width)));
      } else {
        const deltaY = ev.clientY - startY;
        newRatio = Math.max(0.1, Math.min(0.9, startRatio + (deltaY / rect.height)));
      }
      node.ratio = newRatio;
      setRatio(newRatio);
    };

    const handleMouseUp = () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
      saveLayout();
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);
  };

  const containerClass = node.type === 'hsplit' ? 'split-container hsplit' : 'split-container vsplit';
  const flexDirection = node.type === 'hsplit' ? 'row' : 'column';

  return (
    <div ref={containerRef} className={containerClass} style={{ display: 'flex', flexDirection, width: '100%', height: '100%', overflow: 'hidden' }}>
      <div className="split-panel" style={{ flex: ratio, overflow: 'hidden', position: 'relative' }}>
        <WorkspaceNode
          node={node.left}
          activePanelId={activePanelId}
          setActivePanelId={setActivePanelId}
          onSplit={onSplit}
          onClosePanel={onClosePanel}
          onTabChange={onTabChange}
          onAddTab={onAddTab}
          onCloseTab={onCloseTab}
          onOpenFile={onOpenFile}
        />
      </div>
      <div className={`splitter ${node.type}`} onMouseDown={handleMouseDown} />
      <div className="split-panel" style={{ flex: 1 - ratio, overflow: 'hidden', position: 'relative' }}>
        <WorkspaceNode
          node={node.right}
          activePanelId={activePanelId}
          setActivePanelId={setActivePanelId}
          onSplit={onSplit}
          onClosePanel={onClosePanel}
          onTabChange={onTabChange}
          onAddTab={onAddTab}
          onCloseTab={onCloseTab}
          onOpenFile={onOpenFile}
        />
      </div>
    </div>
  );
}

// --- Component: Workspace Node (Split/Leaf Router) ---
function WorkspaceNode({ node, activePanelId, setActivePanelId, onSplit, onClosePanel, onTabChange, onAddTab, onCloseTab, onOpenFile }) {
  if (!node) return null;

  if (node.type === 'leaf') {
    return (
      <WorkspaceLeaf
        node={node}
        activePanelId={activePanelId}
        setActivePanelId={setActivePanelId}
        onSplit={onSplit}
        onClosePanel={onClosePanel}
        onTabChange={onTabChange}
        onAddTab={onAddTab}
        onCloseTab={onCloseTab}
        onOpenFile={onOpenFile}
      />
    );
  }

  return (
    <WorkspaceSplit
      node={node}
      activePanelId={activePanelId}
      setActivePanelId={setActivePanelId}
      onSplit={onSplit}
      onClosePanel={onClosePanel}
      onTabChange={onTabChange}
      onAddTab={onAddTab}
      onCloseTab={onCloseTab}
      onOpenFile={onOpenFile}
    />
  );
}

// --- Main Application Shell ---
function App() {
  // Connection and Settings
  const [serverUrl, setServerUrl] = useState(() => {
    const saved = localStorage.getItem('ghost_mux_server_url');
    if (saved) return saved;
    if (window.location.protocol.startsWith('http')) return window.location.origin;
    return 'http://127.0.0.1:3030';
  });
  const [isConnected, setIsConnected] = useState(false);
  const [sessions, setSessions] = useState([]);
  const [ptyStatus, setPtyStatus] = useState({});

  // Workspaces State
  const [workspaces, setWorkspaces] = useState([]);
  const [activeWorkspacePath, setActiveWorkspacePath] = useState('');

  // Layout State
  const [rootNode, setRootNode] = useState(null);
  const [activePanelId, setActivePanelId] = useState(null);
  const [activeMobileNodeId, setActiveMobileNodeId] = useState(null);

  // Layout Rerender Trigger
  const [layoutTrigger, setLayoutTrigger] = useState(0);

  // Sidebars
  const [activeSidebarTab, setActiveSidebarTab] = useState('explorer');
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [sidebarMobileOpen, setSidebarMobileOpen] = useState(false);

  // Mobile layout state
  const [width, setWidth] = useState(window.innerWidth);

  useEffect(() => {
    const handleResize = () => setWidth(window.innerWidth);
    window.addEventListener('resize', handleResize);
    return () => window.removeEventListener('resize', handleResize);
  }, []);

  const isMobile = width <= 768;

  // Save/Load layout functions
  const persistLayout = (node) => {
    const path = activeWorkspacePath || 'default';
    const key = 'ghost_mux_layout_' + path;
    if (node) {
      localStorage.setItem(key, JSON.stringify(serializeNode(node)));
    } else {
      localStorage.removeItem(key);
    }
    setLayoutTrigger(prev => prev + 1);
  };

  const saveLayout = () => {
    if (rootNode) {
      persistLayout(rootNode);
    }
  };

  useEffect(() => {
    if (!activeWorkspacePath) {
      setRootNode(null);
      return;
    }
    let saved = localStorage.getItem('ghost_mux_layout_' + activeWorkspacePath);
    if (!saved) {
      const globalSaved = localStorage.getItem('ghost_mux_layout');
      if (globalSaved) {
        saved = globalSaved;
        localStorage.setItem('ghost_mux_layout_' + activeWorkspacePath, globalSaved);
        localStorage.removeItem('ghost_mux_layout');
      }
    }
    if (saved) {
      try {
        const rootObj = JSON.parse(saved);
        const node = deserializeNode(rootObj);
        setRootNode(node);
        const leaves = getAllLeaves(node);
        if (leaves.length > 0) {
          setActivePanelId(leaves[0].id);
          setActiveMobileNodeId(leaves[0].id);
        }
      } catch (e) {
        console.error("Failed to load layout from storage for workspace: " + activeWorkspacePath, e);
        setRootNode(null);
      }
    } else {
      setRootNode(null);
    }
  }, [activeWorkspacePath]);

  // JSON-RPC implementation
  const callRpc = async (method, params = {}) => {
    try {
      const response = await fetch(`${serverUrl}/api`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json'
        },
        body: JSON.stringify({ method, params })
      });
      if (!response.ok) {
        throw new Error(`HTTP Error ${response.status}`);
      }
      const data = await response.json();
      if (data.status === 'success') {
        return data.result;
      } else {
        throw new Error(data.error || 'Server error');
      }
    } catch (err) {
      throw new Error(err.message || 'Network error');
    }
  };

  // Connection & Session Checker
  const checkConnection = async () => {
    try {
      const result = await callRpc('pty.list');
      setIsConnected(true);
      setSessions(result.sessions || []);

      try {
        const wsResult = await callRpc('workspaces.list');
        setWorkspaces(wsResult.workspaces || []);
        setActiveWorkspacePath(prev => prev || wsResult.active_path);
      } catch (err) {
        console.error("Failed to fetch workspaces list", err);
      }
    } catch (err) {
      setIsConnected(false);
      setSessions([]);
    }
  };

  const handleWorkspaceChange = async (path) => {
    try {
      await callRpc('workspaces.set_active', { path });
      setActiveWorkspacePath(path);
    } catch (err) {
      alert("Failed to switch workspace: " + err.message);
    }
  };

  const handleAddWorkspace = async () => {
    const path = prompt("Enter absolute path for new repository/workspace:");
    if (!path) return;
    try {
      const res = await callRpc('workspaces.add', { path });
      setWorkspaces(prev => {
        if (!prev.some(w => w.path === res.workspace.path)) {
          return [...prev, res.workspace];
        }
        return prev;
      });
      setActiveWorkspacePath(res.workspace.path);
    } catch (err) {
      alert("Failed to add workspace: " + err.message);
    }
  };

  const handleRemoveWorkspace = async (pathToRemove) => {
    if (workspaces.length <= 1) {
      alert("Cannot remove the only workspace.");
      return;
    }
    if (!confirm("Are you sure you want to remove this workspace from the list?")) return;
    try {
      await callRpc('workspaces.remove', { path: pathToRemove });
      const wsResult = await callRpc('workspaces.list');
      setWorkspaces(wsResult.workspaces || []);
      setActiveWorkspacePath(wsResult.active_path);
    } catch (err) {
      alert("Failed to remove workspace: " + err.message);
    }
  };

  // Periodic Connection Polling
  useEffect(() => {
    checkConnection();
    const interval = setInterval(checkConnection, 3000);
    return () => clearInterval(interval);
  }, [serverUrl]);

  // Handle server url changes
  const handleServerUrlChange = (e) => {
    const val = e.target.value.trim();
    setServerUrl(val);
    localStorage.setItem('ghost_mux_server_url', val);
  };

  // Spawn Root Terminal (Fallback/First run)
  const spawnRootTerminal = async () => {
    try {
      const res = await callRpc('pty.spawn', { cwd: activeWorkspacePath || '.' });
      checkConnection();

      const newRoot = new LayoutNode('leaf', nextNodeId++);
      newRoot.tabs.push({
        id: nextNodeId++,
        type: 'terminal',
        title: `Terminal (${res.pty_id})`,
        data: { ptyId: res.pty_id }
      });
      newRoot.activeTabIndex = 0;

      setRootNode(newRoot);
      setActivePanelId(newRoot.id);
      setActiveMobileNodeId(newRoot.id);
      persistLayout(newRoot);
    } catch (err) {
      alert("Failed to spawn session: " + err.message);
    }
  };

  // Attach an existing PTY session to active workspace panel
  const attachPtyToWorkspace = (ptyId) => {
    let newRoot = rootNode;
    if (!newRoot) {
      newRoot = new LayoutNode('leaf', nextNodeId++);
      newRoot.tabs.push({
        id: nextNodeId++,
        type: 'terminal',
        title: `Terminal (${ptyId})`,
        data: { ptyId }
      });
      newRoot.activeTabIndex = 0;
      setRootNode(newRoot);
      setActivePanelId(newRoot.id);
      setActiveMobileNodeId(newRoot.id);
      persistLayout(newRoot);
      return;
    }

    let targetNode = findNodeById(newRoot, activePanelId);
    if (!targetNode) {
      const leaves = getAllLeaves(newRoot);
      targetNode = leaves.find(l => l.tabs.length === 0) || leaves[0];
      setActivePanelId(targetNode.id);
    }

    if (targetNode) {
      const existingIdx = targetNode.tabs.findIndex(t => t.type === 'terminal' && t.data.ptyId === ptyId);
      if (existingIdx !== -1) {
        targetNode.activeTabIndex = existingIdx;
      } else {
        targetNode.tabs.push({
          id: nextNodeId++,
          type: 'terminal',
          title: `Terminal (${ptyId})`,
          data: { ptyId }
        });
        targetNode.activeTabIndex = targetNode.tabs.length - 1;
      }
      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    }
  };

  // Close PTY session from server
  const closePtyFromServer = async (ptyId) => {
    try {
      await callRpc('pty.close', { pty_id: ptyId });
      if (activeTerminals[ptyId]) {
        activeTerminals[ptyId].close();
        delete activeTerminals[ptyId];
      }
      if (rootNode) {
        const cleanPtyFromNode = (n) => {
          if (!n) return;
          if (n.type === 'leaf') {
            const idx = n.tabs.findIndex(t => t.type === 'terminal' && t.data.ptyId === ptyId);
            if (idx !== -1) {
              closeTabInNode(n.id, idx);
            }
          } else {
            cleanPtyFromNode(n.left);
            cleanPtyFromNode(n.right);
          }
        };
        cleanPtyFromNode(rootNode);
      }
      checkConnection();
    } catch (err) {
      alert("Failed to close PTY: " + err.message);
    }
  };

  // Real-time terminal state updater
  const onPtyEvent = (ptyId, runningAgent, lastEvent) => {
    setPtyStatus(prev => ({
      ...prev,
      [ptyId]: { runningAgent, lastEvent }
    }));
  };

  // Split panel mutation
  const handleSplitPanel = async (nodeId, splitType) => {
    const newRoot = rootNode;
    const node = findNodeById(newRoot, nodeId);
    if (!node) return;

    const oldTabs = [...node.tabs];
    const oldActiveIndex = node.activeTabIndex;
    const leftId = nextNodeId++;
    const rightId = nextNodeId++;

    node.type = splitType;
    node.ratio = 0.5;

    node.left = new LayoutNode('leaf', leftId);
    node.left.tabs = oldTabs;
    node.left.activeTabIndex = oldActiveIndex;

    node.right = new LayoutNode('leaf', rightId);
    node.right.tabs = [{
      id: nextNodeId++,
      type: 'terminal',
      title: 'Terminal (Spawning...)',
      data: { ptyId: null }
    }];
    node.right.activeTabIndex = 0;

    setActivePanelId(rightId);
    setRootNode({ ...newRoot });
    persistLayout(newRoot);

    try {
      const res = await callRpc('pty.spawn', { cwd: activeWorkspacePath || '.' });
      node.right.tabs[0].title = `Terminal (${res.pty_id})`;
      node.right.tabs[0].data.ptyId = res.pty_id;
      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    } catch (err) {
      console.error("Failed to spawn split terminal PTY", err);
    }
  };

  // Close panel leaf mutation
  const handleClosePanel = (nodeId) => {
    if (!rootNode) return;
    const newRoot = rootNode;

    const node = findNodeById(newRoot, nodeId);
    if (!node) return;

    if (node.type === 'leaf') {
      node.tabs.forEach(tab => {
        if (tab.type === 'terminal' && tab.data.ptyId) {
          callRpc('pty.close', { pty_id: tab.data.ptyId }).catch(err => console.error(err));
          if (activeTerminals[tab.data.ptyId]) {
            activeTerminals[tab.data.ptyId].close();
            delete activeTerminals[tab.data.ptyId];
          }
        }
      });
    }

    const parent = findParentNode(newRoot, nodeId);
    if (!parent) {
      setRootNode(null);
      setActivePanelId(null);
      persistLayout(null);
      return;
    }

    const sibling = (parent.left.id === nodeId) ? parent.right : parent.left;
    const grandparent = findParentNode(newRoot, parent.id);

    if (!grandparent) {
      setRootNode(sibling);
      const leaves = getAllLeaves(sibling);
      if (leaves.length > 0) {
        setActivePanelId(leaves[0].id);
        setActiveMobileNodeId(leaves[0].id);
      }
    } else {
      if (grandparent.left.id === parent.id) {
        grandparent.left = sibling;
      } else {
        grandparent.right = sibling;
      }
      setRootNode({ ...newRoot });
      const leaves = getAllLeaves(newRoot);
      if (leaves.length > 0) {
        setActivePanelId(leaves[0].id);
        setActiveMobileNodeId(leaves[0].id);
      }
    }
    persistLayout(newRoot);
  };

  // Tab switching
  const handleTabChange = (nodeId, index) => {
    const newRoot = rootNode;
    const node = findNodeById(newRoot, nodeId);
    if (node) {
      node.activeTabIndex = index;
      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    }
  };

  // Add new Tab template
  const handleAddTab = async (nodeId, type) => {
    const newRoot = rootNode;
    const node = findNodeById(newRoot, nodeId);
    if (!node) return;

    if (type === 'terminal') {
      const tempTabId = nextNodeId++;
      const newTab = {
        id: tempTabId,
        type: 'terminal',
        title: 'Terminal (Spawning...)',
        data: { ptyId: null }
      };
      node.tabs.push(newTab);
      node.activeTabIndex = node.tabs.length - 1;
      setRootNode({ ...newRoot });
      saveLayout();

      try {
        const res = await callRpc('pty.spawn', { cwd: activeWorkspacePath || '.' });
        newTab.title = `Terminal (${res.pty_id})`;
        newTab.data.ptyId = res.pty_id;
        setRootNode({ ...newRoot });
        persistLayout(newRoot);
      } catch (err) {
        console.error("Spawning PTY tab failed", err);
      }
    } else {
      let title = 'Explorer';
      if (type === 'git') title = 'Git Status';
      else if (type === 'browser') title = 'Browser';
      else if (type === 'diagnostics') title = 'Diagnostics';

      node.tabs.push({
        id: nextNodeId++,
        type,
        title,
        data: type === 'browser' ? { url: 'https://google.com' } : {}
      });
      node.activeTabIndex = node.tabs.length - 1;
      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    }
  };

  // Close specific tab
  const handleCloseTab = (nodeId, index) => {
    closeTabInNode(nodeId, index);
  };

  const closeTabInNode = (nodeId, index) => {
    const newRoot = rootNode;
    const node = findNodeById(newRoot, nodeId);
    if (!node) return;

    const tab = node.tabs[index];
    if (tab && tab.type === 'terminal' && tab.data.ptyId) {
      callRpc('pty.close', { pty_id: tab.data.ptyId }).catch(err => console.error(err));
      if (activeTerminals[tab.data.ptyId]) {
        activeTerminals[tab.data.ptyId].close();
        delete activeTerminals[tab.data.ptyId];
      }
    }

    node.tabs.splice(index, 1);

    if (node.tabs.length === 0) {
      handleClosePanel(nodeId);
    } else {
      if (node.activeTabIndex >= node.tabs.length) {
        node.activeTabIndex = node.tabs.length - 1;
      }
      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    }
  };

  // Open file in editor tab helper
  const openFileInEditor = async (filePath) => {
    try {
      const res = await callRpc('fs.read_file', { path: filePath });
      const fileName = filePath.split('/').pop();

      let newRoot = rootNode;
      if (!newRoot) {
        newRoot = new LayoutNode('leaf', nextNodeId++);
        setRootNode(newRoot);
        setActivePanelId(newRoot.id);
        setActiveMobileNodeId(newRoot.id);
      }

      let targetNode = findNodeById(newRoot, activePanelId);
      if (!targetNode) {
        const leaves = getAllLeaves(newRoot);
        if (leaves.length > 0) {
          targetNode = leaves[0];
          setActivePanelId(targetNode.id);
        } else {
          newRoot = new LayoutNode('leaf', nextNodeId++);
          setRootNode(newRoot);
          targetNode = newRoot;
          setActivePanelId(newRoot.id);
          setActiveMobileNodeId(newRoot.id);
        }
      }

      const existingIdx = targetNode.tabs.findIndex(t => t.type === 'editor' && t.data.path === filePath);
      if (existingIdx !== -1) {
        targetNode.activeTabIndex = existingIdx;
      } else {
        targetNode.tabs.push({
          id: nextNodeId++,
          type: 'editor',
          title: fileName,
          data: {
            path: filePath,
            content: res.content || '',
            originalContent: res.content || ''
          }
        });
        targetNode.activeTabIndex = targetNode.tabs.length - 1;
      }

      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    } catch (err) {
      alert("Failed to read file: " + err.message);
    }
  };

  // Open git diff in diff tab helper
  const openGitDiff = async (filePath) => {
    try {
      const res = await callRpc('git.diff', { cwd: activeWorkspacePath || '.', path: filePath });
      const fileName = filePath.split('/').pop();

      let newRoot = rootNode;
      if (!newRoot) {
        newRoot = new LayoutNode('leaf', nextNodeId++);
        setRootNode(newRoot);
        setActivePanelId(newRoot.id);
        setActiveMobileNodeId(newRoot.id);
      }

      let targetNode = findNodeById(newRoot, activePanelId);
      if (!targetNode) {
        const leaves = getAllLeaves(newRoot);
        if (leaves.length > 0) {
          targetNode = leaves[0];
          setActivePanelId(targetNode.id);
        } else {
          newRoot = new LayoutNode('leaf', nextNodeId++);
          setRootNode(newRoot);
          targetNode = newRoot;
          setActivePanelId(newRoot.id);
          setActiveMobileNodeId(newRoot.id);
        }
      }

      const existingIdx = targetNode.tabs.findIndex(t => t.type === 'diff' && t.data.path === filePath);
      if (existingIdx !== -1) {
        targetNode.activeTabIndex = existingIdx;
        targetNode.tabs[existingIdx].data.diff = res.diff || '';
      } else {
        targetNode.tabs.push({
          id: nextNodeId++,
          type: 'diff',
          title: `diff: ${fileName}`,
          data: {
            path: filePath,
            diff: res.diff || ''
          }
        });
        targetNode.activeTabIndex = targetNode.tabs.length - 1;
      }

      setRootNode({ ...newRoot });
      persistLayout(newRoot);
    } catch (err) {
      alert("Failed to load git diff: " + err.message);
    }
  };

  const activeWorkspace = workspaces.find(w => w.path === activeWorkspacePath);
  const activeWorkspaceName = activeWorkspace ? activeWorkspace.name : 'Workspace Root';

  // Workspace context state
  const contextValue = {
    serverUrl,
    isConnected,
    callRpc,
    openGitDiff,
    onPtyEvent,
    saveLayout,
    activePanelId,
    setActivePanelId,
    activeWorkspacePath,
    activeWorkspaceName
  };

  // Mobile Workspace render elements
  const leaves = rootNode ? getAllLeaves(rootNode) : [];
  let activeMobileLeaf = leaves.find(l => l.id === activeMobileNodeId);
  if (leaves.length > 0 && !activeMobileLeaf) {
    activeMobileLeaf = leaves[0];
  }
  const activeMobileTab = activeMobileLeaf ? activeMobileLeaf.tabs[activeMobileLeaf.activeTabIndex] : null;

  const activeSessions = sessions.filter(s => s.cwd === activeWorkspacePath);

  return (
    <WorkspaceContext.Provider value={contextValue}>
      <div className={`app-container ${sidebarCollapsed ? 'sidebar-collapsed' : ''}`} key={layoutTrigger}>
        {/* Top brand header bar */}
        <header className="app-header">
          <div className="brand">
            <button
              className="sidebar-toggle-btn"
              title="Toggle Sidebar"
              onClick={() => {
                if (isMobile) {
                  setSidebarMobileOpen(!sidebarMobileOpen);
                } else {
                  setSidebarCollapsed(!sidebarCollapsed);
                }
              }}
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="3" y1="12" x2="21" y2="12"></line>
                <line x1="3" y1="6" x2="21" y2="6"></line>
                <line x1="3" y1="18" x2="21" y2="18"></line>
              </svg>
            </button>
            <svg className="logo" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M9 10h.01" />
              <path d="M15 10h.01" />
              <path d="M10 13a2 2 0 0 0 4 0" />
              <path d="M12 2a8 8 0 0 0-8 8v12l3-3 2.5 2.5L12 19l2.5 2.5L17 19l3 3V10a8 8 0 0 0-8-8z" />
            </svg>
            <span className="title">GHOST-MUX</span>
            <span className="badge badge-primary">Web Client</span>
          </div>

          <div className="header-controls">
            {/* Connection settings input */}
            <div className="input-group">
              <span className="input-label">Server</span>
              <input type="text" value={serverUrl} onChange={handleServerUrlChange} placeholder="http://127.0.0.1:3030" />
            </div>

            {/* Offline/Online connection status indicator */}
            <div className={`status-badge ${isConnected ? 'status-online' : 'status-offline'}`}>
              <span className="status-indicator"></span>
              <span className="status-text">{isConnected ? 'Connected' : 'Disconnected'}</span>
            </div>

            {/* Topbar Spawn Terminal fallback */}
            {isConnected && (
              <button className="btn btn-accent" onClick={spawnRootTerminal}>
                Spawn Terminal
              </button>
            )}
          </div>
        </header>

        {/* Main layout container */}
        <main className="app-workspace">
          {/* Workspace Tabs Rail (Vertical Tabs) */}
          {isConnected && workspaces.length > 0 && (
            <div className={`workspace-tabs-rail ${sidebarMobileOpen ? 'open' : ''}`}>
              <div className="workspace-tabs-list">
                {workspaces.map(ws => {
                  const isActive = ws.path === activeWorkspacePath;
                  const initials = ws.name 
                    ? ws.name.split(/[-_\s]+/).map(w => w[0]).join('').substring(0, 3).toUpperCase() 
                    : '?';
                  return (
                    <div 
                      key={ws.path}
                      className={`workspace-tab-item ${isActive ? 'active' : ''}`}
                      onClick={() => handleWorkspaceChange(ws.path)}
                    >
                      <div className="workspace-tab-avatar" title={`${ws.name} (${ws.path})`}>
                        {initials}
                      </div>
                      <span className="workspace-tab-tooltip">{ws.name}</span>
                      
                      {workspaces.length > 1 && (
                        <button 
                          className="workspace-tab-close-btn"
                          onClick={(e) => {
                            e.stopPropagation();
                            handleRemoveWorkspace(ws.path);
                          }}
                        >
                          ✕
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
              
              <button 
                className="workspace-tab-add-btn" 
                onClick={handleAddWorkspace}
                title="Add Workspace"
              >
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                  <line x1="12" y1="5" x2="12" y2="19"></line>
                  <line x1="5" y1="12" x2="19" y2="12"></line>
                </svg>
              </button>
            </div>
          )}

          {/* Responsive Left Sidebar */}
          <aside className={`sidebar-container ${sidebarMobileOpen ? 'open' : ''}`}>
            {/* Sidebar Rail */}
            <div className="sidebar-rail">
              <button className={`rail-btn ${activeSidebarTab === 'explorer' ? 'active' : ''}`} onClick={() => setActiveSidebarTab('explorer')} title="File Explorer">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
              </button>
              <button className={`rail-btn ${activeSidebarTab === 'git' ? 'active' : ''}`} onClick={() => setActiveSidebarTab('git')} title="Source Control">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <circle cx="18" cy="18" r="3" />
                  <circle cx="6" cy="6" r="3" />
                  <circle cx="6" cy="18" r="3" />
                  <path d="M18 15V9a4 4 0 0 0-4-4H9M6 9v6" />
                </svg>
              </button>
              <button className={`rail-btn ${activeSidebarTab === 'sessions' ? 'active' : ''}`} onClick={() => setActiveSidebarTab('sessions')} title="PTY Sessions">
                <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="2" y="3" width="20" height="14" rx="2" />
                  <path d="M8 21h8M12 17v4" />
                </svg>
              </button>
            </div>

            {/* Sidebar Active Panel content */}
            <div className="sidebar-panel">
              {activeSidebarTab === 'explorer' && (
                <div className="sidebar-content active" id="tab-explorer">
                  <div className="sidebar-header">
                    <h4>Explorer</h4>
                  </div>
                  <ExplorerTab onOpenFile={(p) => { setSidebarMobileOpen(false); openFileInEditor(p); }} />
                </div>
              )}

              {activeSidebarTab === 'git' && (
                <div className="sidebar-content active" id="tab-git">
                  <GitTab />
                </div>
              )}

              {activeSidebarTab === 'sessions' && (
                <div className="sidebar-content active" id="tab-sessions">
                  <div className="sidebar-header" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <h4>PTY Sessions</h4>
                    <button className="btn btn-accent btn-sm" onClick={async () => {
                      try {
                        const res = await callRpc('pty.spawn', { cwd: activeWorkspacePath || '.' });
                        checkConnection();
                        attachPtyToWorkspace(res.pty_id);
                      } catch (err) {
                        alert(err.message);
                      }
                    }}>+ New</button>
                  </div>
                  <div className="sessions-list" style={{ padding: '8px', overflowY: 'auto', flex: 1 }}>
                    {activeSessions.length === 0 ? (
                      <div className="sessions-placeholder">No active PTY sessions.</div>
                    ) : (
                      activeSessions.map(s => {
                        const isAttached = rootNode ? getAllLeaves(rootNode).some(leaf => leaf.tabs.some(t => t.type === 'terminal' && t.data.ptyId === s.pty_id)) : false;
                        const statusObj = ptyStatus[s.pty_id] || {};
                        const runningAgent = statusObj.runningAgent || s.running_agent;
                        const lastEvent = statusObj.lastEvent || s.last_event;

                        return (
                          <div key={s.pty_id} className={`session-card ${isAttached ? 'active' : ''}`}>
                            <div className="session-card-header">
                              <span className="session-id">ID: {s.pty_id}</span>
                              <span className="session-size">{s.cols}x{s.rows}</span>
                            </div>
                            <div className="session-details" style={{ margin: '6px 0', display: 'flex', gap: '4px', flexWrap: 'wrap' }}>
                              {runningAgent && <span className="badge badge-primary">{runningAgent}</span>}
                              {lastEvent && <span className="badge" style={{ backgroundColor: 'var(--muted)', border: '1px solid var(--border)' }}>{lastEvent}</span>}
                              {isAttached && <span className="badge" style={{ backgroundColor: 'rgba(87, 201, 148, 0.2)', color: '#57c994', border: '1px solid rgba(87, 201, 148, 0.3)' }}>Attached</span>}
                            </div>
                            <div className="session-card-footer" style={{ display: 'flex', gap: '6px' }}>
                              {!isAttached && <button className="btn btn-primary btn-sm attach-btn" onClick={() => attachPtyToWorkspace(s.pty_id)}>Attach</button>}
                              <button className="btn btn-secondary btn-sm close-btn action-btn-danger" onClick={() => closePtyFromServer(s.pty_id)}>Close</button>
                            </div>
                          </div>
                        );
                      })
                    )}
                  </div>
                </div>
              )}
            </div>
          </aside>

          {/* Sidebar drawer backdrop (only visible on mobile layout) */}
          {sidebarMobileOpen && (
            <div className="sidebar-backdrop active" id="sidebar-backdrop" onClick={() => setSidebarMobileOpen(false)} />
          )}

          {/* Core Panel layout/splits viewport */}
          <div id="workspace-root" className="workspace-viewport">
            {!rootNode ? (
              <div className="empty-state">
                <div className="empty-content">
                  <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                    <rect x="3" y="3" width="18" height="18" rx="2" />
                    <path d="M9 3v18M3 9h18" />
                  </svg>
                  <h3>No Active Layout</h3>
                  <p>Connect to the headless IDE server or spawn a new terminal session to get started.</p>
                  <button className="btn btn-accent" onClick={spawnRootTerminal}>Spawn New Session</button>
                </div>
              </div>
            ) : isMobile ? (
              // Mobile responsive tabbed panel view
              <div className="mobile-workspace-wrapper" style={{ display: 'flex', flexDirection: 'column', width: '100%', height: '100%' }}>
                <div className="mobile-tab-bar" style={{ display: 'flex', overflowX: 'auto', borderBottom: '1px solid var(--border)', backgroundColor: 'var(--secondary)' }}>
                  {leaves.map(leaf => {
                    const leafActiveTab = leaf.tabs[leaf.activeTabIndex];
                    const activePtyId = leafActiveTab && leafActiveTab.type === 'terminal' ? leafActiveTab.data.ptyId : null;
                    const statusObj = activePtyId ? ptyStatus[activePtyId] || {} : {};
                    const runningAgent = statusObj.runningAgent;
                    const lastEvent = statusObj.lastEvent;

                    return (
                      <div
                        key={leaf.id}
                        className={`mobile-tab ${leaf.id === activeMobileNodeId ? 'active' : ''}`}
                        onClick={() => setActiveMobileNodeId(leaf.id)}
                        style={{ display: 'flex', alignItems: 'center', padding: '8px 12px', borderRight: '1px solid var(--border)', cursor: 'pointer', whiteSpace: 'nowrap' }}
                      >
                        <span>{leafActiveTab ? leafActiveTab.title : `Panel ${leaf.id}`}</span>
                        {runningAgent && <span className="badge badge-primary" style={{ fontSize: '8px', padding: '1px 4px', marginLeft: '4px' }}>{runningAgent}</span>}
                        {lastEvent && <span className="badge" style={{ backgroundColor: 'var(--muted)', fontSize: '8px', padding: '1px 4px', marginLeft: '4px' }}>{lastEvent}</span>}
                        <button
                          className="mobile-tab-close"
                          onClick={(e) => { e.stopPropagation(); handleClosePanel(leaf.id); }}
                          style={{ marginLeft: '8px', background: 'transparent', border: 'none', color: 'var(--muted-foreground)', cursor: 'pointer' }}
                        >
                          ✕
                        </button>
                      </div>
                    );
                  })}
                  <button
                    className="mobile-tab-add"
                    onClick={async () => {
                      try {
                        const res = await callRpc('pty.spawn', { cwd: activeWorkspacePath || '.' });
                        attachPtyToWorkspace(res.pty_id);
                        const updated = rootNode ? getAllLeaves(rootNode) : [];
                        const leafWithPty = updated.find(l => l.tabs.some(t => t.data.ptyId === res.pty_id));
                        if (leafWithPty) {
                          setActiveMobileNodeId(leafWithPty.id);
                        }
                      } catch (err) {
                        alert(err.message);
                      }
                    }}
                    style={{ padding: '8px 16px', background: 'transparent', border: 'none', borderRight: '1px solid var(--border)', color: 'var(--accent)', cursor: 'pointer', fontWeight: 'bold' }}
                  >
                    +
                  </button>
                </div>
                <div className="panel-content" style={{ flex: 1, position: 'relative', overflow: 'hidden' }}>
                  {activeMobileTab ? (
                    <TabContentRenderer
                      tab={activeMobileTab}
                      nodeId={activeMobileNodeId}
                      onOpenFile={openFileInEditor}
                    />
                  ) : (
                    <div className="sessions-placeholder">No active tabs. Click + to add.</div>
                  )}
                </div>
              </div>
            ) : (
              // Desktop multi-panel split view
              <WorkspaceNode
                node={rootNode}
                activePanelId={activePanelId}
                setActivePanelId={setActivePanelId}
                onSplit={handleSplitPanel}
                onClosePanel={handleClosePanel}
                onTabChange={handleTabChange}
                onAddTab={handleAddTab}
                onCloseTab={handleCloseTab}
                onOpenFile={openFileInEditor}
              />
            )}
          </div>
        </main>
      </div>
    </WorkspaceContext.Provider>
  );
}

export default App;
