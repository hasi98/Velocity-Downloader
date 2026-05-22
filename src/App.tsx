import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { check } from '@tauri-apps/plugin-updater';
import type { DownloadTask, ProgressEvent, Toast, HttpContext, ShowAddDownloadPayload } from './types';
import { formatBytes, formatSpeed, formatEta, generateId, getFileIcon, getFileCategory } from './utils';
import { openChildWindow } from './windowPlacement';
import './index.css';
import './native-ui.css';

type ContextMenuState = { x: number; y: number; id: string } | null;

function App() {
  const [downloads, setDownloads] = useState<DownloadTask[]>([]);
  const [category, setCategory] = useState<string>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [selectionAnchorId, setSelectionAnchorId] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<ContextMenuState>(null);

  const [toasts, setToasts] = useState<Toast[]>([]);
  const [httpContext, setHttpContext] = useState<HttpContext>({});

  const downloadsRef = useRef<DownloadTask[]>([]);
  const tableContainerRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => { downloadsRef.current = downloads; }, [downloads]);

  // Close context menu on click outside
  useEffect(() => {
    const handler = () => setContextMenu(null);
    window.addEventListener('click', handler);
    return () => window.removeEventListener('click', handler);
  }, []);

  const addToast = useCallback((type: Toast['type'], message: string) => {
    const id = generateId();
    setToasts(prev => [...prev, { id, type, message }]);
    setTimeout(() => setToasts(prev => prev.filter(t => t.id !== id)), 4000);
  }, []);

  const openDownloadWindow = useCallback(async (id: string, filename: string) => {
    const label = `dl-${id.replace(/[^a-zA-Z0-9]/g, '')}-${Date.now()}`;
    openChildWindow(label, {
      url: `?window=download&id=${id}`,
      title: `Downloading - ${filename}`,
      width: 650,
      height: 350,
      minWidth: 500,
      minHeight: 330,
      resizable: true,
      decorations: true,
      alwaysOnTop: false,
    }, (e) => console.error('Download window error:', e));
  }, []);

  const openSettingsWindow = useCallback(async () => {
    openChildWindow('settings-window', {
      url: '?window=settings',
      title: 'Options',
      width: 780,
      height: 640,
      minWidth: 600,
      minHeight: 500,
      resizable: true,
      decorations: true,
    }, (e) => console.error('Settings window error:', e));
  }, []);

  const openAddDownloadWindow = useCallback((payload?: any) => {
    const id = Date.now().toString();
    if (payload) {
      localStorage.setItem(`add-dl-${id}`, JSON.stringify(payload));
    }
    openChildWindow(`add-download-${id}`, {
      url: `?window=add-download&id=${id}`,
      title: 'Add New Download',
      width: 600,
      height: 360,
      minWidth: 500,
      minHeight: 320,
      resizable: true,
      decorations: true,
      alwaysOnTop: false,
    }, (e) => console.error('Add window error:', e));
  }, []);

  const openExtensionWindow = useCallback(() => {
    openChildWindow('extensions-window', {
      url: '?window=extensions',
      title: 'Install Extension',
      width: 800,
      height: 750,
      minWidth: 700,
      minHeight: 600,
      resizable: true,
      decorations: true,
      alwaysOnTop: false,
    }, (e) => console.error('Ext window error:', e));
  }, []);

  const openBatchDownloadWindow = useCallback(() => {
    openChildWindow(`batch-download-${Date.now()}`, {
      url: '?window=batch-download',
      title: 'Batch Download',
      width: 700,
      height: 600,
      minWidth: 500,
      minHeight: 400,
      resizable: true,
      decorations: true,
      alwaysOnTop: false,
    }, (e) => console.error('Batch window error:', e));
  }, []);

  // Load initial data
  useEffect(() => {
    const init = async () => {
      try {
        const [allDownloads] = await Promise.all([
          invoke<DownloadTask[]>('get_all_downloads'),
        ]);
        setDownloads(allDownloads);

        const chromeInstalled = await invoke<boolean>('check_extension_installed', { browser: 'chrome' }).catch(() => false);
        const edgeInstalled = await invoke<boolean>('check_extension_installed', { browser: 'edge' }).catch(() => false);
        if (!chromeInstalled && !edgeInstalled) openExtensionWindow();
      } catch (e) {
        console.error('Failed to initialize:', e);
      }
    };
    init();
  }, []);

  useEffect(() => {
    const key = 'velocity-last-update-check';
    const lastCheck = Number(localStorage.getItem(key) || 0);
    const sixHours = 6 * 60 * 60 * 1000;
    if (Date.now() - lastCheck < sixHours) return;

    localStorage.setItem(key, Date.now().toString());
    check({ timeout: 15000 })
      .then(update => {
        if (update) addToast('info', `Update ${update.version} available. Open Options to install.`);
      })
      .catch(() => {});
  }, [addToast]);

  // Event listeners
  useEffect(() => {
    const unlistenProgress = listen<ProgressEvent>('download-progress', (event) => {
      const p = event.payload;
      setDownloads(prev => prev.map(d => d.id === p.download_id
        ? { ...d, downloaded: p.downloaded, speed_bps: p.speed_bps, eta_seconds: p.eta_seconds, status: p.status, segments: p.segments.length > 0 ? p.segments : d.segments }
        : d
      ));
    });

    const unlistenAdded = listen<DownloadTask>('download-added', (event) => {
      // Use ref for existence check — React 18 batches state updaters asynchronously
      const alreadyExists = downloadsRef.current.some(d => d.id === event.payload.id);
      if (!alreadyExists) {
        setDownloads(prev => prev.some(d => d.id === event.payload.id) ? prev : [event.payload, ...prev]);
        openDownloadWindow(event.payload.id, event.payload.filename);
      }
    });

    const unlistenShowModal = listen<ShowAddDownloadPayload>('show-add-download', (event) => {
      openAddDownloadWindow({
        url: event.payload.url,
        cookies: event.payload.cookies,
        referer: event.payload.referer,
        user_agent: event.payload.user_agent
      });
    });

    return () => {
      unlistenProgress.then(f => f());
      unlistenAdded.then(f => f());
      unlistenShowModal.then(f => f());
    };
  }, [openDownloadWindow]);

  // Handlers
  const handleAddDownload = async (url: string, savePath?: string, ctx?: HttpContext) => {
    const context = ctx ?? httpContext;
    try {
      await invoke<DownloadTask>('add_download', {
        url,
        savePath: savePath || null,
        cookies: context.cookies ?? null,
        referer: context.referer ?? null,
        userAgent: context.user_agent ?? null,
      });
      setHttpContext({});
    } catch (e: any) {
      addToast('error', typeof e === 'string' ? e : 'Failed to start download');
    }
  };

  const handlePause = async (id: string) => {
    try {
      await invoke('pause_download', { downloadId: id });
      setDownloads(prev => prev.map(d => d.id === id ? { ...d, status: 'paused' as const, speed_bps: 0 } : d));
    } catch {}
  };

  const handleResume = async (id: string) => {
    try { await invoke('resume_download', { downloadId: id }); } catch {}
  };

  const handleRemoveMany = async (ids: string[]) => {
    const uniqueIds = [...new Set(ids)];
    if (uniqueIds.length === 0) return;

    try {
      await Promise.all(uniqueIds.map(id => invoke('remove_download', { downloadId: id })));
      setDownloads(prev => prev.filter(d => !uniqueIds.includes(d.id)));
      setSelectedIds(prev => prev.filter(id => !uniqueIds.includes(id)));
      if (selectedId && uniqueIds.includes(selectedId)) setSelectedId(null);
      if (selectionAnchorId && uniqueIds.includes(selectionAnchorId)) setSelectionAnchorId(null);
    } catch (e) {
      addToast('error', typeof e === 'string' ? e : 'Failed to delete selected downloads');
    }
  };

  const handleStopAll = async () => {
    downloads.forEach(d => {
      if (d.status === 'downloading' || d.status === 'assembling') handlePause(d.id);
    });
  };

  const displayedDownloads = downloads.filter(d => {
    if (category === 'unfinished') return d.status !== 'completed';
    if (category === 'finished') return d.status === 'completed';
    if (['Compressed', 'Documents', 'Music', 'Programs', 'Video'].includes(category)) {
      return getFileCategory(d.filename) === category;
    }
    return true;
  });

  const displayedIds = displayedDownloads.map(d => d.id);
  const selectedIdSet = new Set(selectedIds);
  const selectedDownloads = downloads.filter(d => selectedIdSet.has(d.id));
  const selectedDownload = downloads.find(d => d.id === selectedId) ?? selectedDownloads[0] ?? null;
  const hasSelection = selectedIds.length > 0;
  const hasActiveSelection = selectedDownloads.some(d => d.status === 'downloading' || d.status === 'assembling');
  const hasResumableSelection = selectedDownloads.some(d =>
    d.status !== 'downloading' && d.status !== 'assembling' && d.status !== 'completed'
  );

  useEffect(() => {
    const visibleIds = new Set(displayedDownloads.map(d => d.id));
    setSelectedIds(prev => prev.filter(id => visibleIds.has(id)));
    setSelectedId(prev => (prev && visibleIds.has(prev) ? prev : null));
    setSelectionAnchorId(prev => (prev && visibleIds.has(prev) ? prev : null));
  }, [downloads, category]);

  const focusTable = () => {
    tableContainerRef.current?.focus({ preventScroll: true });
  };

  const scrollRowIntoView = (id: string) => {
    requestAnimationFrame(() => {
      document.querySelector(`[data-download-row="${CSS.escape(id)}"]`)?.scrollIntoView({
        block: 'nearest',
        inline: 'nearest',
      });
    });
  };

  const getRangeIds = (fromId: string, toId: string) => {
    const fromIndex = displayedIds.indexOf(fromId);
    const toIndex = displayedIds.indexOf(toId);
    if (fromIndex === -1 || toIndex === -1) return [toId];

    const start = Math.min(fromIndex, toIndex);
    const end = Math.max(fromIndex, toIndex);
    return displayedIds.slice(start, end + 1);
  };

  const selectSingle = (id: string) => {
    setSelectedId(id);
    setSelectedIds([id]);
    setSelectionAnchorId(id);
    focusTable();
    scrollRowIntoView(id);
  };

  const selectRange = (id: string, additive = false) => {
    const anchor = selectionAnchorId && displayedIds.includes(selectionAnchorId)
      ? selectionAnchorId
      : selectedId && displayedIds.includes(selectedId)
        ? selectedId
        : id;
    const rangeIds = getRangeIds(anchor, id);
    setSelectedId(id);
    setSelectedIds(prev => additive ? [...new Set([...prev, ...rangeIds])] : rangeIds);
    if (!selectionAnchorId) setSelectionAnchorId(anchor);
    focusTable();
    scrollRowIntoView(id);
  };

  const toggleSelection = (id: string) => {
    setSelectedIds(prev => {
      const next = prev.includes(id) ? prev.filter(selected => selected !== id) : [...prev, id];
      setSelectedId(next.includes(id) ? id : next[next.length - 1] ?? null);
      return next;
    });
    setSelectionAnchorId(id);
    focusTable();
    scrollRowIntoView(id);
  };

  const handleRowClick = (e: React.MouseEvent, id: string) => {
    if (e.shiftKey) {
      selectRange(id, e.ctrlKey || e.metaKey);
    } else if (e.ctrlKey || e.metaKey) {
      toggleSelection(id);
    } else {
      selectSingle(id);
    }
  };

  const handleRowContextMenu = (e: React.MouseEvent, id: string) => {
    e.preventDefault();
    e.stopPropagation();
    if (!selectedIdSet.has(id)) {
      setSelectedId(id);
      setSelectedIds([id]);
      setSelectionAnchorId(id);
    }
    setContextMenu({ x: e.clientX, y: e.clientY, id });
  };

  const handleTableKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    if (displayedIds.length === 0) return;

    const currentIndex = selectedId ? displayedIds.indexOf(selectedId) : -1;
    const fallbackIndex = currentIndex === -1 ? 0 : currentIndex;
    let nextIndex: number | null = null;

    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'a') {
      e.preventDefault();
      setSelectedIds(displayedIds);
      setSelectedId(displayedIds[displayedIds.length - 1] ?? null);
      setSelectionAnchorId(displayedIds[0] ?? null);
      return;
    }

    switch (e.key) {
      case 'ArrowDown':
        nextIndex = currentIndex === -1 ? 0 : Math.min(fallbackIndex + 1, displayedIds.length - 1);
        break;
      case 'ArrowUp':
        nextIndex = Math.max(fallbackIndex - 1, 0);
        break;
      case 'Home':
        nextIndex = 0;
        break;
      case 'End':
        nextIndex = displayedIds.length - 1;
        break;
      case 'PageDown':
        nextIndex = currentIndex === -1 ? 0 : Math.min(fallbackIndex + 10, displayedIds.length - 1);
        break;
      case 'PageUp':
        nextIndex = Math.max(fallbackIndex - 10, 0);
        break;
      case ' ':
        e.preventDefault();
        toggleSelection(displayedIds[fallbackIndex]);
        return;
      case 'Enter':
        e.preventDefault();
        if (selectedDownload) openDownloadWindow(selectedDownload.id, selectedDownload.filename);
        return;
      case 'Delete':
      case 'Backspace':
        e.preventDefault();
        handleRemoveMany(selectedIds);
        return;
      case 'Escape':
        e.preventDefault();
        setSelectedId(null);
        setSelectedIds([]);
        setSelectionAnchorId(null);
        return;
      default:
        return;
    }

    e.preventDefault();
    const nextId = displayedIds[nextIndex];
    if (e.shiftKey) {
      selectRange(nextId, e.ctrlKey || e.metaKey);
    } else {
      selectSingle(nextId);
    }
  };

  const ctxDownload = contextMenu ? downloads.find(d => d.id === contextMenu.id) : null;
  const contextSelectionIds = contextMenu
    ? selectedIdSet.has(contextMenu.id) ? selectedIds : [contextMenu.id]
    : [];

  return (
    <div className="native-app">
      {/* Top Toolbar */}
      <div className="native-toolbar">
        <button className="toolbar-btn" onClick={() => { openAddDownloadWindow(); }}>
          <span className="toolbar-icon">➕</span>
          <span className="toolbar-text">Add URL</span>
        </button>
        <button className="toolbar-btn" onClick={openBatchDownloadWindow}>
          <span className="toolbar-icon">📚</span>
          <span className="toolbar-text">Batch</span>
        </button>
        <button
          className="toolbar-btn"
          disabled={!hasResumableSelection}
          onClick={() => selectedDownloads
            .filter(d => d.status !== 'downloading' && d.status !== 'assembling' && d.status !== 'completed')
            .forEach(d => handleResume(d.id))}
        >
          <span className="toolbar-icon">▶️</span>
          <span className="toolbar-text">Resume</span>
        </button>
        <button
          className="toolbar-btn"
          disabled={!hasActiveSelection}
          onClick={() => selectedDownloads
            .filter(d => d.status === 'downloading' || d.status === 'assembling')
            .forEach(d => handlePause(d.id))}
        >
          <span className="toolbar-icon">⏹️</span>
          <span className="toolbar-text">Stop</span>
        </button>
        <button className="toolbar-btn" onClick={handleStopAll}>
          <span className="toolbar-icon">⏸️</span>
          <span className="toolbar-text">Stop All</span>
        </button>
        <button
          className="toolbar-btn"
          disabled={!hasSelection}
          onClick={() => handleRemoveMany(selectedIds)}
        >
          <span className="toolbar-icon">🗑️</span>
          <span className="toolbar-text">Delete</span>
        </button>

        <div className="toolbar-separator" />

        <button className="toolbar-btn" onClick={openSettingsWindow}>
          <span className="toolbar-icon">⚙️</span>
          <span className="toolbar-text">Options</span>
        </button>
      </div>

      <div className="native-main">
        {/* Left Sidebar */}
            <div className="native-sidebar">
              <div className={`sidebar-item ${category === 'all' ? 'active' : ''}`} onClick={() => setCategory('all')}>
                <span className="sidebar-icon">📥</span> All Downloads
              </div>
              <div className={`sidebar-item sub-item ${category === 'unfinished' ? 'active' : ''}`} onClick={() => setCategory('unfinished')}>
                <span className="sidebar-icon">◷</span> Unfinished
              </div>
              <div className={`sidebar-item sub-item ${category === 'finished' ? 'active' : ''}`} onClick={() => setCategory('finished')}>
                <span className="sidebar-icon">✓</span> Finished
              </div>
              <div className="sidebar-group">Categories</div>
              <div className={`sidebar-item sub-item ${category === 'Compressed' ? 'active' : ''}`} onClick={() => setCategory('Compressed')}><span className="sidebar-icon">📂</span> Compressed</div>
              <div className={`sidebar-item sub-item ${category === 'Documents' ? 'active' : ''}`} onClick={() => setCategory('Documents')}><span className="sidebar-icon">📄</span> Documents</div>
              <div className={`sidebar-item sub-item ${category === 'Music' ? 'active' : ''}`} onClick={() => setCategory('Music')}><span className="sidebar-icon">🎵</span> Music</div>
              <div className={`sidebar-item sub-item ${category === 'Programs' ? 'active' : ''}`} onClick={() => setCategory('Programs')}><span className="sidebar-icon">⚙️</span> Programs</div>
              <div className={`sidebar-item sub-item ${category === 'Video' ? 'active' : ''}`} onClick={() => setCategory('Video')}><span className="sidebar-icon">🎬</span> Video</div>
            </div>

            {/* Right Pane Table */}
            <div
              className="native-table-container"
              ref={tableContainerRef}
              tabIndex={0}
              onKeyDown={handleTableKeyDown}
              aria-label="Download history"
            >
              <table className="native-table" aria-multiselectable="true">
                <thead>
                  <tr>
                    <th>File Name</th>
                    <th>Size</th>
                    <th>Status</th>
                    <th>Time Left</th>
                    <th>Transfer Rate</th>
                    <th>Added</th>
                  </tr>
                </thead>
                <tbody>
                  {displayedDownloads.map(d => {
                    const isActive = d.status === 'downloading' || d.status === 'assembling';
                    const isCompleted = d.status === 'completed';
                    let statusLabel = d.status.charAt(0).toUpperCase() + d.status.slice(1);
                    if (isCompleted) statusLabel = 'Complete';

                    return (
                      <tr
                        key={d.id}
                        data-download-row={d.id}
                        className={selectedIdSet.has(d.id) ? 'selected' : ''}
                        aria-selected={selectedIdSet.has(d.id)}
                        onClick={e => handleRowClick(e, d.id)}
                        onDoubleClick={() => openDownloadWindow(d.id, d.filename)}
                        onContextMenu={e => handleRowContextMenu(e, d.id)}
                      >
                        <td className="cell-filename">
                          <span className="file-icon" style={{ opacity: isCompleted ? 1 : 0.7 }}>{getFileIcon(d.filename)}</span>
                          {d.filename}
                        </td>
                        <td>{formatBytes(d.total_size || d.downloaded)}</td>
                        <td>{statusLabel}</td>
                        <td>{isActive ? formatEta(d.eta_seconds) : ''}</td>
                        <td>{isActive && d.speed_bps > 0 ? formatSpeed(d.speed_bps) : ''}</td>
                        <td>{new Date().toLocaleDateString()}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              {displayedDownloads.length === 0 && (
                <div className="empty-table-msg">There are no downloads to display.</div>
              )}
            </div>
      </div>

      {/* Right-click Context Menu */}
      {contextMenu && ctxDownload && (
        <div
          className="ctx-menu"
          style={{ top: contextMenu.y, left: contextMenu.x }}
          onClick={e => e.stopPropagation()}
        >
          <div className="ctx-menu-header">{ctxDownload.filename.length > 30 ? ctxDownload.filename.slice(0, 30) + '…' : ctxDownload.filename}</div>
          <div className="ctx-divider" />
          <button 
            className="ctx-item" 
            disabled={ctxDownload.status !== 'completed'}
            style={ctxDownload.status !== 'completed' ? { opacity: 0.5, cursor: 'not-allowed' } : {}}
            onClick={() => { if (ctxDownload.status === 'completed') invoke('open_file', { path: ctxDownload.save_path }).catch(console.error); setContextMenu(null); }}
          >
            📄 Open File
          </button>
          <button 
            className="ctx-item" 
            onClick={() => { invoke('open_folder', { path: ctxDownload.save_path }).catch(console.error); setContextMenu(null); }}
          >
            📂 Open File Location
          </button>
          <div className="ctx-divider" />
          <button className="ctx-item" onClick={() => { openDownloadWindow(contextMenu.id, ctxDownload.filename); setContextMenu(null); }}>
            📊 Open Progress Window
          </button>
          {(ctxDownload.status === 'paused' || ctxDownload.status === 'failed') && (
            <button className="ctx-item" onClick={() => { handleResume(contextMenu.id); setContextMenu(null); }}>
              ▶ Resume Download
            </button>
          )}
          {(ctxDownload.status === 'downloading' || ctxDownload.status === 'assembling') && (
            <>
              <button className="ctx-item" onClick={() => { handlePause(contextMenu.id); setContextMenu(null); }}>
                ⏸ Pause
              </button>
              <button className="ctx-item" onClick={() => { handlePause(contextMenu.id); setContextMenu(null); }}>
                ⏹ Stop
              </button>
            </>
          )}
          <button className="ctx-item" onClick={() => { 
            const dirOnly = ctxDownload.save_path.substring(0, Math.max(ctxDownload.save_path.lastIndexOf('\\'), ctxDownload.save_path.lastIndexOf('/')));
            handleAddDownload(ctxDownload.url, dirOnly, ctxDownload.http_context); 
            setContextMenu(null); 
          }}>
            🔄 Redownload
          </button>
          <div className="ctx-divider" />
          <button className="ctx-item" onClick={() => { navigator.clipboard.writeText(ctxDownload.url).catch(() => {}); setContextMenu(null); }}>
            📋 Copy URL
          </button>
          <button className="ctx-item" onClick={() => { navigator.clipboard.writeText(ctxDownload.save_path).catch(() => {}); setContextMenu(null); }}>
            📁 Copy Save Path
          </button>
          <div className="ctx-divider" />
          <button className="ctx-item ctx-danger" onClick={() => { handleRemoveMany(contextSelectionIds); setContextMenu(null); }}>
            🗑 Delete
          </button>
        </div>
      )}


      {toasts.length > 0 && (
        <div className="toast-container">
          {toasts.map(toast => (
            <div key={toast.id} className={`toast ${toast.type}`}>
              {toast.message}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default App;
