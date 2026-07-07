// Ghost-mux App Controller

class LayoutNode {
    constructor(type, id, ptyId = null) {
        this.type = type; // 'leaf', 'hsplit', 'vsplit'
        this.id = id;
        this.ptyId = ptyId;
        this.ratio = 0.5;
        this.left = null;
        this.right = null;
    }
}

// Global Application State
let rootNode = null;
let nextNodeId = 1;
const activeTerminals = {}; // ptyId -> { term, fitAddon, pollInterval, container }
let isConnected = false;
const expandedPaths = new Set(['.']); // Tracks expanded folders in file tree
let editingFilePath = '';
let activeSidebarTab = 'explorer';

// Initialize Server URL
function initServerUrl() {
    const input = document.getElementById('server-url-input');
    const savedUrl = localStorage.getItem('ghost_mux_server_url');
    
    if (savedUrl) {
        input.value = savedUrl;
    } else if (window.location.protocol.startsWith('http')) {
        input.value = window.location.origin;
    } else {
        input.value = 'http://127.0.0.1:3030';
    }

    input.addEventListener('change', () => {
        localStorage.setItem('ghost_mux_server_url', input.value.trim());
        checkConnection();
    });
}

function getServerUrl() {
    return document.getElementById('server-url-input').value.trim() || 'http://127.0.0.1:3030';
}

// RPC Network Transport
async function callRpc(method, params = {}) {
    const url = getServerUrl();
    try {
        const response = await fetch(`${url}/api`, {
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
}

// Connection check & Session loading loop
async function checkConnection() {
    const statusBadge = document.getElementById('connection-status');
    const statusText = statusBadge.querySelector('.status-text');
    
    try {
        const result = await callRpc('pty.list');
        isConnected = true;
        statusBadge.className = 'status-badge status-online';
        statusText.textContent = 'Connected';
        
        // Update tabs according to active view
        if (activeSidebarTab === 'sessions') {
            updateSessionsList(result.sessions || []);
        }
    } catch (err) {
        isConnected = false;
        statusBadge.className = 'status-badge status-offline';
        statusText.textContent = 'Disconnected';
        document.getElementById('sessions-list').innerHTML = `
            <div class="sessions-placeholder" style="color: #f47067;">
                Unable to connect to server. Ensure it is running.
            </div>
        `;
    }
}

// xterm.js Terminal Instance Manager
function getOrCreateTerminal(ptyId) {
    if (activeTerminals[ptyId]) {
        return activeTerminals[ptyId];
    }
    
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

    const fitAddon = new FitAddon.FitAddon();
    term.loadAddon(fitAddon);

    term.onData(data => {
        if (isConnected) {
            callRpc('pty.write', { pty_id: ptyId, input: data }).catch(err => {
                console.error("Write error for PTY", ptyId, err);
            });
        }
    });

    let resizeTimeout;
    term.onResize(size => {
        clearTimeout(resizeTimeout);
        resizeTimeout = setTimeout(() => {
            if (isConnected) {
                callRpc('pty.resize', { pty_id: ptyId, cols: size.cols, rows: size.rows }).catch(err => {
                    console.error("Resize error for PTY", ptyId, err);
                });
            }
        }, 150);
    });

    const pollInterval = setInterval(async () => {
        if (!isConnected) return;
        
        try {
            const data = await callRpc('pty.read', { pty_id: ptyId });
            if (data.output) {
                term.write(data.output);
            }
            updatePtyStatusBadges(ptyId, data.running_agent, data.last_event);
        } catch (err) {
            if (err.message && err.message.includes("not found")) {
                cleanupTerminal(ptyId);
                closePtyInLayout(ptyId);
            }
        }
    }, 60);

    const container = document.createElement('div');
    container.className = 'terminal-container';
    
    activeTerminals[ptyId] = {
        term,
        fitAddon,
        pollInterval,
        container
    };

    term.open(container);
    return activeTerminals[ptyId];
}

function cleanupTerminal(ptyId) {
    const session = activeTerminals[ptyId];
    if (session) {
        clearInterval(session.pollInterval);
        session.term.dispose();
        delete activeTerminals[ptyId];
    }
}

// Tree Traversal Helpers
function findNodeById(node, id) {
    if (!node) return null;
    if (node.id === id) return node;
    if (node.type === 'hsplit' || node.type === 'vsplit') {
        return findNodeById(node.left, id) || findNodeById(node.right, id);
    }
    return null;
}

function findParentNode(root, id) {
    if (!root || root.id === id) return null;
    if (root.type === 'hsplit' || root.type === 'vsplit') {
        if ((root.left && root.left.id === id) || (root.right && root.right.id === id)) {
            return root;
        }
        return findParentNode(root.left, id) || findParentNode(root.right, id);
    }
    return null;
}

function findNodeByPtyId(node, ptyId) {
    if (!node) return null;
    if (node.type === 'leaf' && node.ptyId === ptyId) return node;
    if (node.type === 'hsplit' || node.type === 'vsplit') {
        return findNodeByPtyId(node.left, ptyId) || findNodeByPtyId(node.right, ptyId);
    }
    return null;
}

function findEmptyLeaf(node) {
    if (!node) return null;
    if (node.type === 'leaf') {
        return node.ptyId === null ? node : null;
    }
    return findEmptyLeaf(node.left) || findEmptyLeaf(node.right);
}

function findFirstLeaf(node) {
    if (!node) return null;
    if (node.type === 'leaf') return node;
    return findFirstLeaf(node.left);
}

function isPtyAttached(ptyId) {
    return checkPtyInTree(rootNode, ptyId);
}

function checkPtyInTree(node, ptyId) {
    if (!node) return false;
    if (node.type === 'leaf') {
        return node.ptyId === ptyId;
    }
    return checkPtyInTree(node.left, ptyId) || checkPtyInTree(node.right, ptyId);
}

// Layout Mutations
async function spawnRootTerminal() {
    try {
        const res = await callRpc('pty.spawn', { cwd: '/' });
        rootNode = new LayoutNode('leaf', nextNodeId++, res.pty_id);
        saveLayout();
        renderWorkspace();
    } catch (err) {
        alert("Failed to spawn root terminal: " + err.message);
    }
}

async function splitPanel(nodeId, splitType) {
    const node = findNodeById(rootNode, nodeId);
    if (!node) return;

    const oldPtyId = node.ptyId;
    const leftId = nextNodeId++;
    const rightId = nextNodeId++;

    node.type = splitType;
    node.ptyId = null;
    node.ratio = 0.5;
    node.left = new LayoutNode('leaf', leftId, oldPtyId);
    node.right = new LayoutNode('leaf', rightId, null);

    saveLayout();
    renderWorkspace();

    try {
        const res = await callRpc('pty.spawn', { cwd: '/' });
        node.right.ptyId = res.pty_id;
        saveLayout();
        renderWorkspace();
    } catch (err) {
        console.error("Failed to spawn split PTY", err);
    }
}

function closePanel(nodeId) {
    if (!rootNode) return;

    const node = findNodeById(rootNode, nodeId);
    if (!node) return;

    if (node.ptyId) {
        callRpc('pty.close', { pty_id: node.ptyId }).catch(err => console.error(err));
        cleanupTerminal(node.ptyId);
    }

    const parent = findParentNode(rootNode, nodeId);
    if (!parent) {
        rootNode = null;
        saveLayout();
        renderWorkspace();
        return;
    }

    const sibling = (parent.left.id === nodeId) ? parent.right : parent.left;
    const grandparent = findParentNode(rootNode, parent.id);

    if (!grandparent) {
        rootNode = sibling;
    } else {
        if (grandparent.left.id === parent.id) {
            grandparent.left = sibling;
        } else {
            grandparent.right = sibling;
        }
    }

    saveLayout();
    renderWorkspace();
}

function closePtyInLayout(ptyId) {
    const node = findNodeByPtyId(rootNode, ptyId);
    if (node) {
        closePanel(node.id);
    }
}

async function attachPtyToWorkspace(ptyId) {
    if (!rootNode) {
        rootNode = new LayoutNode('leaf', nextNodeId++, ptyId);
        saveLayout();
        renderWorkspace();
        return;
    }

    const leaf = findEmptyLeaf(rootNode) || findFirstLeaf(rootNode);
    if (leaf) {
        if (!leaf.ptyId) {
            leaf.ptyId = ptyId;
        } else {
            const oldPtyId = leaf.ptyId;
            const leftId = nextNodeId++;
            const rightId = nextNodeId++;

            leaf.type = 'hsplit';
            leaf.ptyId = null;
            leaf.ratio = 0.5;
            leaf.left = new LayoutNode('leaf', leftId, oldPtyId);
            leaf.right = new LayoutNode('leaf', rightId, ptyId);
        }
        saveLayout();
        renderWorkspace();
    }
}

async function closePtyFromServer(ptyId) {
    if (confirm(`Are you sure you want to terminate session ${ptyId}?`)) {
        try {
            await callRpc('pty.close', { pty_id: ptyId });
            cleanupTerminal(ptyId);
            closePtyInLayout(ptyId);
            checkConnection();
        } catch (err) {
            console.error("Failed to close PTY", err);
        }
    }
}

// DOM Rendering of the layout tree
function buildDomLayout(node) {
    if (!node) return document.createElement('div');

    if (node.type === 'leaf') {
        const panel = document.createElement('div');
        panel.className = 'panel-view';
        panel.id = `panel-${node.id}`;

        const header = document.createElement('div');
        header.className = 'panel-header';

        const title = document.createElement('div');
        title.className = 'panel-title';
        title.innerHTML = `
            <svg class="logo" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" style="width: 12px; height: 12px;">
                <rect x="2" y="3" width="20" height="14" rx="2" />
                <path d="M8 21h8M12 17v4" />
            </svg>
            <span class="panel-title-active">Terminal (${node.ptyId || 'Spawning...'})</span>
            <span id="agent-badge-${node.ptyId}" class="badge badge-primary" style="display: none;"></span>
            <span id="event-badge-${node.ptyId}" class="badge" style="display: none;"></span>
        `;
        header.appendChild(title);

        const actions = document.createElement('div');
        actions.className = 'panel-actions';

        const hSplit = document.createElement('button');
        hSplit.className = 'action-btn';
        hSplit.title = 'Split Horizontally (⬜→)';
        hSplit.textContent = '⬜→';
        hSplit.addEventListener('click', () => splitPanel(node.id, 'hsplit'));
        actions.appendChild(hSplit);

        const vSplit = document.createElement('button');
        vSplit.className = 'action-btn';
        vSplit.title = 'Split Vertically (⬜↓)';
        vSplit.textContent = '⬜↓';
        vSplit.addEventListener('click', () => splitPanel(node.id, 'vsplit'));
        actions.appendChild(vSplit);

        if (rootNode !== node) {
            const close = document.createElement('button');
            close.className = 'action-btn action-btn-danger';
            close.title = 'Close Panel (✕)';
            close.textContent = '✕';
            close.addEventListener('click', () => closePanel(node.id));
            actions.appendChild(close);
        }

        header.appendChild(actions);
        panel.appendChild(header);

        const content = document.createElement('div');
        content.className = 'panel-content';

        if (node.ptyId) {
            const session = getOrCreateTerminal(node.ptyId);
            content.appendChild(session.container);
        } else {
            content.innerHTML = `
                <div class="empty-state">
                    <div class="empty-content">
                        <div class="status-badge status-online">
                            <span class="status-indicator"></span>
                            <span class="status-text">Creating session...</span>
                        </div>
                    </div>
                </div>
            `;
        }

        panel.appendChild(content);
        return panel;
    }

    const container = document.createElement('div');
    container.className = `split-container ${node.type}`;
    container.id = `split-${node.id}`;

    const left = document.createElement('div');
    left.className = 'split-panel';
    left.style.flex = node.ratio;
    left.appendChild(buildDomLayout(node.left));

    const splitter = document.createElement('div');
    splitter.className = 'splitter';

    const right = document.createElement('div');
    right.className = 'split-panel';
    right.style.flex = 1 - node.ratio;
    right.appendChild(buildDomLayout(node.right));

    splitter.addEventListener('mousedown', (e) => {
        e.preventDefault();
        splitter.classList.add('dragging');
        
        const startX = e.clientX;
        const startY = e.clientY;
        const startRatio = node.ratio;
        const rect = container.getBoundingClientRect();
        
        const onMouseMove = (ev) => {
            if (node.type === 'hsplit') {
                const deltaX = ev.clientX - startX;
                node.ratio = Math.max(0.1, Math.min(0.9, startRatio + (deltaX / rect.width)));
            } else {
                const deltaY = ev.clientY - startY;
                node.ratio = Math.max(0.1, Math.min(0.9, startRatio + (deltaY / rect.height)));
            }
            left.style.flex = node.ratio;
            right.style.flex = 1 - node.ratio;
            
            fitAllTerminals(node);
        };
        
        const onMouseUp = () => {
            splitter.classList.remove('dragging');
            window.removeEventListener('mousemove', onMouseMove);
            window.removeEventListener('mouseup', onMouseUp);
            saveLayout();
            fitAllTerminals(node);
        };
        
        window.addEventListener('mousemove', onMouseMove);
        window.addEventListener('mouseup', onMouseUp);
    });

    container.appendChild(left);
    container.appendChild(splitter);
    container.appendChild(right);

    return container;
}

function fitAllTerminals(node) {
    if (!node) return;
    if (node.type === 'leaf') {
        if (node.ptyId) {
            const session = activeTerminals[node.ptyId];
            if (session) {
                session.fitAddon.fit();
            }
        }
    } else {
        fitAllTerminals(node.left);
        fitAllTerminals(node.right);
    }
}

function updatePtyStatusBadges(ptyId, runningAgent, lastEvent) {
    const agentBadge = document.getElementById(`agent-badge-${ptyId}`);
    const eventBadge = document.getElementById(`event-badge-${ptyId}`);
    
    if (agentBadge) {
        if (runningAgent) {
            agentBadge.textContent = runningAgent;
            agentBadge.style.display = 'inline-block';
        } else {
            agentBadge.style.display = 'none';
        }
    }
    
    if (eventBadge) {
        if (lastEvent) {
            eventBadge.textContent = lastEvent;
            if (lastEvent === 'Start') {
                eventBadge.style.backgroundColor = 'rgba(87, 201, 148, 0.2)';
                eventBadge.style.color = '#57c994';
                eventBadge.style.border = '1px solid rgba(87, 201, 148, 0.3)';
            } else if (lastEvent === 'Stop') {
                eventBadge.style.backgroundColor = 'rgba(244, 112, 103, 0.2)';
                eventBadge.style.color = '#f47067';
                eventBadge.style.border = '1px solid rgba(244, 112, 103, 0.3)';
            } else {
                eventBadge.style.backgroundColor = 'var(--muted)';
                eventBadge.style.color = 'var(--muted-foreground)';
                eventBadge.style.border = '1px solid var(--border)';
            }
            eventBadge.style.display = 'inline-block';
        } else {
            eventBadge.style.display = 'none';
        }
    }
}

// Workspace UI Render Trigger
function renderWorkspace() {
    const container = document.getElementById('workspace-root');
    container.innerHTML = '';
    
    if (!rootNode) {
        container.innerHTML = `
            <div class="empty-state">
                <div class="empty-content">
                    <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                        <rect x="3" y="3" width="18" height="18" rx="2" />
                        <path d="M9 3v18M3 9h18" />
                    </svg>
                    <h3>No Active Layout</h3>
                    <p>Connect to the headless IDE server or spawn a new terminal session to get started.</p>
                    <button id="empty-spawn-btn" class="btn btn-accent">Spawn New Session</button>
                </div>
            </div>
        `;
        document.getElementById('empty-spawn-btn').addEventListener('click', spawnRootTerminal);
        return;
    }
    
    const domNode = buildDomLayout(rootNode);
    container.appendChild(domNode);
    
    setTimeout(() => {
        fitAllTerminals(rootNode);
    }, 50);
}

// Session List UI Render
function updateSessionsList(sessions) {
    const list = document.getElementById('sessions-list');
    list.innerHTML = '';
    
    if (sessions.length === 0) {
        list.innerHTML = '<div class="sessions-placeholder">No active PTY sessions.</div>';
        return;
    }
    
    sessions.forEach(s => {
        const isAttached = isPtyAttached(s.pty_id);
        
        const card = document.createElement('div');
        card.className = `session-card ${isAttached ? 'active' : ''}`;
        card.innerHTML = `
            <div class="session-card-header">
                <span class="session-id">ID: ${s.pty_id}</span>
                <span class="session-size">${s.cols}x${s.rows}</span>
            </div>
            <div class="session-details">
                ${s.running_agent ? `<span class="badge badge-primary">${s.running_agent}</span>` : ''}
                ${s.last_event ? `<span class="badge" style="background-color: var(--muted); border: 1px solid var(--border);">${s.last_event}</span>` : ''}
                ${isAttached ? `<span class="badge" style="background-color: rgba(87, 201, 148, 0.2); color: #57c994; border: 1px solid rgba(87, 201, 148, 0.3)">Attached</span>` : ''}
            </div>
            <div class="session-card-footer">
                ${!isAttached ? `<button class="btn btn-primary attach-btn" data-id="${s.pty_id}">Attach</button>` : ''}
                <button class="btn btn-secondary close-btn action-btn-danger" data-id="${s.pty_id}">Close</button>
            </div>
        `;
        
        const attachBtn = card.querySelector('.attach-btn');
        if (attachBtn) {
            attachBtn.addEventListener('click', () => attachPtyToWorkspace(s.pty_id));
        }
        
        card.querySelector('.close-btn').addEventListener('click', (e) => {
            e.stopPropagation();
            closePtyFromServer(s.pty_id);
        });
        
        list.appendChild(card);
    });
}

// --- File Explorer Logic ---

async function renderFileTree() {
    if (!isConnected) return;
    const treeEl = document.getElementById('file-tree');
    treeEl.innerHTML = '';
    
    const rootNodes = await buildTreeNode('.', 0);
    rootNodes.forEach(node => treeEl.appendChild(node));
}

async function buildTreeNode(path, depth) {
    const list = [];
    try {
        const res = await callRpc('fs.list_dir', { path });
        const entries = res.entries || [];
        
        // Sort: directories first, then files alphabetically
        entries.sort((a, b) => {
            if (a.is_dir && !b.is_dir) return -1;
            if (!a.is_dir && b.is_dir) return 1;
            return a.name.localeCompare(b.name);
        });

        for (const entry of entries) {
            // Hide system/git metadata directories for clean visuals
            if (entry.name === '.git' || entry.name === '.DS_Store' || entry.name === 'target') {
                continue;
            }

            const nodeEl = document.createElement('div');
            nodeEl.className = 'tree-node';
            
            const rowEl = document.createElement('div');
            rowEl.className = 'tree-row';
            rowEl.style.paddingLeft = `${depth * 14 + 8}px`;
            
            const labelEl = document.createElement('div');
            labelEl.className = 'tree-label';
            
            const icon = entry.is_dir ? '📁' : '📄';
            labelEl.innerHTML = `<span class="tree-icon">${icon}</span><span>${entry.name}</span>`;
            rowEl.appendChild(labelEl);
            
            // Row Hover Actions
            const actionsEl = document.createElement('div');
            actionsEl.className = 'tree-actions';
            
            if (entry.is_dir) {
                // New File in directory
                const newFileBtn = document.createElement('button');
                newFileBtn.className = 'tree-action-btn';
                newFileBtn.textContent = '📄+';
                newFileBtn.title = 'New File';
                newFileBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    createNewFile(entry.path);
                });
                actionsEl.appendChild(newFileBtn);
                
                // New Folder in directory
                const newFolderBtn = document.createElement('button');
                newFolderBtn.className = 'tree-action-btn';
                newFolderBtn.textContent = '📁+';
                newFolderBtn.title = 'New Folder';
                newFolderBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    createNewFolder(entry.path);
                });
                actionsEl.appendChild(newFolderBtn);
            } else {
                // Edit File
                const editBtn = document.createElement('button');
                editBtn.className = 'tree-action-btn';
                editBtn.textContent = '✏️';
                editBtn.title = 'Edit File';
                editBtn.addEventListener('click', (e) => {
                    e.stopPropagation();
                    openEditor(entry.path);
                });
                actionsEl.appendChild(editBtn);
            }
            
            // Rename Button
            const renameBtn = document.createElement('button');
            renameBtn.className = 'tree-action-btn';
            renameBtn.textContent = '🔄';
            renameBtn.title = 'Rename';
            renameBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                renameEntry(entry.path, entry.name);
            });
            actionsEl.appendChild(renameBtn);
            
            // Delete Button
            const deleteBtn = document.createElement('button');
            deleteBtn.className = 'tree-action-btn tree-action-delete';
            deleteBtn.textContent = '✕';
            deleteBtn.title = 'Delete';
            deleteBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                deleteEntry(entry.path, entry.is_dir);
            });
            actionsEl.appendChild(deleteBtn);
            
            rowEl.appendChild(actionsEl);
            nodeEl.appendChild(rowEl);
            
            const isExpanded = expandedPaths.has(entry.path);
            if (entry.is_dir && isExpanded) {
                const childrenContainer = document.createElement('div');
                childrenContainer.className = 'tree-children';
                
                const childNodes = await buildTreeNode(entry.path, depth + 1);
                childNodes.forEach(child => childrenContainer.appendChild(child));
                nodeEl.appendChild(childrenContainer);
            }
            
            rowEl.addEventListener('click', async (e) => {
                if (entry.is_dir) {
                    if (isExpanded) {
                        expandedPaths.delete(entry.path);
                    } else {
                        expandedPaths.add(entry.path);
                    }
                    renderFileTree();
                } else {
                    openEditor(entry.path);
                }
            });
            
            list.push(nodeEl);
        }
    } catch (err) {
        console.error("Failed to build tree node for path:", path, err);
    }
    return list;
}

// File CRUD Commands
async function createNewFile(parentPath) {
    const name = prompt("Enter new file name:");
    if (!name) return;
    const fullPath = `${parentPath}/${name}`;
    try {
        await callRpc('fs.create_file', { path: fullPath });
        expandedPaths.add(parentPath);
        renderFileTree();
    } catch (err) {
        alert("Failed to create file: " + err.message);
    }
}

async function createNewFolder(parentPath) {
    const name = prompt("Enter new folder name:");
    if (!name) return;
    const fullPath = `${parentPath}/${name}`;
    try {
        await callRpc('fs.create_dir', { path: fullPath });
        expandedPaths.add(parentPath);
        renderFileTree();
    } catch (err) {
        alert("Failed to create directory: " + err.message);
    }
}

async function renameEntry(oldPath, oldName) {
    const newName = prompt("Enter new name:", oldName);
    if (!newName || newName === oldName) return;
    
    const segments = oldPath.split('/');
    segments[segments.length - 1] = newName;
    const newPath = segments.join('/');
    
    try {
        await callRpc('fs.rename', { src: oldPath, dst: newPath });
        renderFileTree();
    } catch (err) {
        alert("Failed to rename: " + err.message);
    }
}

async function deleteEntry(path, isDir) {
    if (confirm(`Are you sure you want to delete ${path}?` + (isDir ? " This will delete all contents." : ""))) {
        try {
            await callRpc('fs.delete', { path, recursive: isDir });
            renderFileTree();
        } catch (err) {
            alert("Failed to delete: " + err.message);
        }
    }
}

// File Editor UI Dialog
async function openEditor(filePath) {
    editingFilePath = filePath;
    document.getElementById('editor-file-title').textContent = `Editing: ${filePath}`;
    
    try {
        const res = await callRpc('fs.read_file', { path: filePath });
        document.getElementById('editor-textarea').value = res.content || '';
        document.getElementById('editor-modal').classList.remove('modal-closed');
    } catch (err) {
        alert("Failed to read file: " + err.message);
    }
}

function closeEditor() {
    document.getElementById('editor-modal').classList.add('modal-closed');
    editingFilePath = '';
}

async function saveFile() {
    if (!editingFilePath) return;
    const content = document.getElementById('editor-textarea').value;
    try {
        await callRpc('fs.write_file', { path: editingFilePath, content });
        closeEditor();
        renderFileTree();
    } catch (err) {
        alert("Failed to save file: " + err.message);
    }
}

// --- Git Source Control Logic ---

async function loadGitStatus() {
    if (!isConnected) return;
    try {
        const res = await callRpc('git.status', { cwd: '.' });
        document.getElementById('git-branch-name').textContent = res.branch || 'no branch';
        
        const listEl = document.getElementById('git-changes-list');
        listEl.innerHTML = '';
        
        const files = res.files || [];
        if (files.length === 0) {
            listEl.innerHTML = '<div class="sessions-placeholder">No changes detected.</div>';
            return;
        }
        
        files.forEach(f => {
            const item = document.createElement('div');
            item.className = 'git-change-item';
            
            let statusLabel = f.status;
            let statusClass = 'git-status-untracked';
            if (f.status === 'M') {
                statusClass = 'git-status-M';
            } else if (f.status === 'A') {
                statusClass = 'git-status-A';
            } else if (f.status === 'D') {
                statusClass = 'git-status-D';
            } else if (f.status === '??') {
                statusLabel = 'U';
                statusClass = 'git-status-untracked';
            }
            
            item.innerHTML = `
                <div class="git-file-info">
                    <span class="git-status-indicator ${statusClass}">${statusLabel}</span>
                    <span class="git-file-path" title="${f.path}">${f.path}</span>
                </div>
                <div class="git-item-actions">
                    <button class="btn-text stage-btn" title="Stage Change">+</button>
                </div>
            `;
            
            item.querySelector('.stage-btn').addEventListener('click', (e) => {
                e.stopPropagation();
                stageFile(f.path);
            });
            
            item.addEventListener('click', () => {
                openGitDiff(f.path);
            });
            
            listEl.appendChild(item);
        });
    } catch (err) {
        document.getElementById('git-branch-name').textContent = 'no repository';
        document.getElementById('git-changes-list').innerHTML = `
            <div class="sessions-placeholder" style="color: var(--muted-foreground)">
                ${err.message || 'Not a git repository'}
            </div>
        `;
    }
}

async function stageFile(path) {
    try {
        await callRpc('git.add', { cwd: '.', path });
        loadGitStatus();
    } catch (err) {
        alert("Failed to stage: " + err.message);
    }
}

async function stageAll() {
    try {
        await callRpc('git.add', { cwd: '.', path: '.' });
        loadGitStatus();
    } catch (err) {
        alert("Failed to stage all: " + err.message);
    }
}

async function commitChanges() {
    const msgInput = document.getElementById('git-commit-msg');
    const message = msgInput.value.trim();
    if (!message) {
        alert("Please enter a commit message.");
        return;
    }
    
    try {
        await callRpc('git.commit', { cwd: '.', message });
        msgInput.value = '';
        loadGitStatus();
        alert("Committed successfully!");
    } catch (err) {
        alert("Commit failed: " + err.message);
    }
}

async function pushChanges() {
    const pushBtn = document.getElementById('git-push-btn');
    try {
        pushBtn.disabled = true;
        pushBtn.textContent = 'Pushing...';
        await callRpc('git.push', { cwd: '.' });
        alert("Pushed successfully!");
    } catch (err) {
        alert("Push failed: " + err.message);
    } finally {
        pushBtn.disabled = false;
        pushBtn.textContent = 'Push';
        loadGitStatus();
    }
}

async function openGitDiff(filePath) {
    document.getElementById('diff-file-title').textContent = `Diff: ${filePath}`;
    
    try {
        const res = await callRpc('git.diff', { cwd: '.', path: filePath });
        const diffText = res.diff || '';
        
        const diffContainer = document.getElementById('diff-content-view');
        diffContainer.innerHTML = '';
        
        const lines = diffText.split('\n');
        lines.forEach(line => {
            const lineEl = document.createElement('span');
            lineEl.className = 'diff-line';
            
            if (line.startsWith('+') && !line.startsWith('+++')) {
                lineEl.classList.add('diff-line-add');
            } else if (line.startsWith('-') && !line.startsWith('---')) {
                lineEl.classList.add('diff-line-del');
            } else if (line.startsWith('@@') || line.startsWith('diff') || line.startsWith('index')) {
                lineEl.classList.add('diff-line-meta');
            }
            
            lineEl.textContent = line;
            diffContainer.appendChild(lineEl);
        });
        
        document.getElementById('diff-modal').classList.remove('modal-closed');
    } catch (err) {
        alert("Failed to load diff: " + err.message);
    }
}

function closeDiff() {
    document.getElementById('diff-modal').classList.add('modal-closed');
}

// Layout State Serialization
function serializeNode(node) {
    if (!node) return null;
    return {
        id: node.id,
        type: node.type,
        ratio: node.ratio,
        ptyId: node.ptyId,
        left: serializeNode(node.left),
        right: serializeNode(node.right)
    };
}

function deserializeNode(obj) {
    if (!obj) return null;
    const node = new LayoutNode(obj.type, obj.id, obj.ptyId);
    node.ratio = obj.ratio;
    node.left = deserializeNode(obj.left);
    node.right = deserializeNode(obj.right);
    
    if (obj.id >= nextNodeId) {
        nextNodeId = obj.id + 1;
    }
    
    return node;
}

function saveLayout() {
    localStorage.setItem('ghost_mux_layout', JSON.stringify(serializeNode(rootNode)));
}

function loadLayout() {
    const saved = localStorage.getItem('ghost_mux_layout');
    if (saved) {
        try {
            rootNode = deserializeNode(JSON.parse(saved));
        } catch (e) {
            console.error("Failed to load layout", e);
            rootNode = null;
        }
    }
}

// Window Event Listeners
window.addEventListener('resize', () => {
    if (rootNode) {
        fitAllTerminals(rootNode);
    }
});

// App Entrypoint
document.addEventListener('DOMContentLoaded', () => {
    initServerUrl();
    
    // Sidebar Tabs navigation click logic
    const railBtns = document.querySelectorAll('.rail-btn');
    const tabContents = document.querySelectorAll('.sidebar-content');
    const sidebarContainer = document.querySelector('.sidebar-container');
    
    railBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            const targetTab = btn.getAttribute('data-tab');
            const isAlreadyActive = btn.classList.contains('active');
            const isMobile = window.innerWidth <= 768;
            
            if (isMobile) {
                if (isAlreadyActive && sidebarContainer.classList.contains('open')) {
                    sidebarContainer.classList.remove('open');
                    btn.classList.remove('active');
                    return;
                } else {
                    sidebarContainer.classList.add('open');
                }
            }
            
            activeSidebarTab = targetTab;
            
            railBtns.forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            
            tabContents.forEach(c => {
                c.classList.remove('active');
                if (c.id === `tab-${targetTab}`) {
                    c.classList.add('active');
                }
            });
            
            // Trigger specific tab re-fetch
            if (targetTab === 'explorer') {
                renderFileTree();
            } else if (targetTab === 'git') {
                loadGitStatus();
            } else if (targetTab === 'sessions') {
                checkConnection();
            }
        });
    });

    // Close sidebar on mobile when workspace is clicked
    document.getElementById('workspace-root').addEventListener('click', () => {
        if (window.innerWidth <= 768 && sidebarContainer.classList.contains('open')) {
            sidebarContainer.classList.remove('open');
            railBtns.forEach(b => b.classList.remove('active'));
        }
    });

    // Fit terminals after sidebar transition completes
    sidebarContainer.addEventListener('transitionend', () => {
        if (rootNode) {
            fitAllTerminals(rootNode);
        }
    });

    // Explorer Header Actions
    document.getElementById('new-file-root-btn').addEventListener('click', () => createNewFile('.'));
    document.getElementById('new-folder-root-btn').addEventListener('click', () => createNewFolder('.'));
    document.getElementById('refresh-explorer-btn').addEventListener('click', renderFileTree);
    
    // Editor Modal Buttons
    document.getElementById('close-editor-modal-btn').addEventListener('click', closeEditor);
    document.getElementById('cancel-editor-btn').addEventListener('click', closeEditor);
    document.getElementById('save-editor-btn').addEventListener('click', saveFile);
    
    // Git Panel Actions
    document.getElementById('refresh-git-btn').addEventListener('click', loadGitStatus);
    document.getElementById('git-stage-all-btn').addEventListener('click', stageAll);
    document.getElementById('git-commit-btn').addEventListener('click', commitChanges);
    document.getElementById('git-push-btn').addEventListener('click', pushChanges);
    
    // Cmd+Enter inside commit box to commit
    document.getElementById('git-commit-msg').addEventListener('keydown', (e) => {
        if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
            e.preventDefault();
            commitChanges();
        }
    });

    // Git Diff Modal Buttons
    document.getElementById('close-diff-modal-btn').addEventListener('click', closeDiff);
    document.getElementById('close-diff-btn').addEventListener('click', closeDiff);

    // Sessions Panel Actions
    document.getElementById('new-session-btn').addEventListener('click', async () => {
        try {
            const res = await callRpc('pty.spawn', { cwd: '/' });
            checkConnection();
            attachPtyToWorkspace(res.pty_id);
        } catch (err) {
            alert("Failed to spawn PTY: " + err.message);
        }
    });

    // Spawn Root Terminal button (topbar fallback)
    document.getElementById('spawn-root-btn').addEventListener('click', spawnRootTerminal);

    // Initial connection verify and workspace render
    checkConnection();
    loadLayout();
    renderWorkspace();
    
    // Run initial data queries
    setTimeout(() => {
        renderFileTree();
        loadGitStatus();
    }, 200);
    
    // Periodically poll server connection, PTY lists, and active tab data
    setInterval(() => {
        checkConnection();
        if (isConnected) {
            if (activeSidebarTab === 'git') {
                loadGitStatus();
            }
        }
    }, 3000);
});
