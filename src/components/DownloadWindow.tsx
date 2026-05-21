import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { currentMonitor, getCurrentWindow } from '@tauri-apps/api/window';
import { LogicalSize } from '@tauri-apps/api/dpi';
import type { DownloadTask, ProgressEvent } from '../types';
import { formatBytes, formatSpeed } from '../utils';
import '../index.css';
import '../native-ui.css';

interface DownloadWindowProps {
  id: string;
}

const WINDOW_WIDTH = 650;
const COMPACT_HEIGHT = 350;
const MIN_EXPANDED_HEIGHT = 330;

export function DownloadWindow({ id }: DownloadWindowProps) {
  const [task, setTask] = useState<DownloadTask | null>(null);
  const [limitInput, setLimitInput] = useState<string>('');
  const [showSegments, setShowSegments] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    invoke<DownloadTask | null>('get_download', { downloadId: id }).then(t => {
      if (t) {
        setTask(t);
        setLimitInput(t.speed_limit_bps ? (t.speed_limit_bps / 1024).toString() : '');
      }
    });

    const unlisten = listen<ProgressEvent>('download-progress', (event) => {
      if (event.payload.download_id === id) {
        setTask(prev => {
          if (!prev) return null;
          return {
            ...prev,
            downloaded: event.payload.downloaded,
            speed_bps: event.payload.speed_bps,
            eta_seconds: event.payload.eta_seconds,
            status: event.payload.status,
            segments: event.payload.segments.length > 0 ? event.payload.segments : prev.segments,
            speed_limit_bps: event.payload.speed_limit_bps,
          };
        });
      }
    });

    return () => { unlisten.then(f => f()); };
  }, [id]);

  const handlePause = async () => {
    try { await invoke('pause_download', { downloadId: id }); } catch {}
  };

  const handleStop = async () => {
    try { 
      await invoke('pause_download', { downloadId: id }); 
      try {
          await getCurrentWebviewWindow().close();
      } catch {
          window.close();
      }
    } catch {}
  };

  const handleResume = async () => {
    try { await invoke('resume_download', { downloadId: id }); } catch {}
  };

  const handleSetLimit = async () => {
    const limitKbps = parseFloat(limitInput);
    const limitBps = isNaN(limitKbps) || limitKbps <= 0 ? null : Math.floor(limitKbps * 1024);
    try { await invoke('set_task_speed_limit', { downloadId: id, limitBps }); } catch {}
  };

  const handleToggleSegments = async () => {
    if (!task) return;
    setShowSegments(prev => !prev);
  };

  useEffect(() => {
    if (!task) return;

    let cancelled = false;
    const resizeToContent = async () => {
      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      if (cancelled) return;

      const appWindow = getCurrentWindow();

      if (!showSegments) {
        await appWindow.setSize(new LogicalSize(WINDOW_WIDTH, COMPACT_HEIGHT)).catch(() => {});
        return;
      }

      const root = rootRef.current;
      if (!root) return;

      const rootStyle = window.getComputedStyle(root);
      const gap = parseFloat(rootStyle.rowGap || rootStyle.gap || '0') || 0;
      const paddingY =
        (parseFloat(rootStyle.paddingTop || '0') || 0) +
        (parseFloat(rootStyle.paddingBottom || '0') || 0);
      const children = Array.from(root.children) as HTMLElement[];
      const contentHeight = children.reduce((sum, child) => {
        const childHeight = child.classList.contains('dw-segments')
          ? child.scrollHeight
          : child.offsetHeight;
        return sum + childHeight;
      }, paddingY + Math.max(0, children.length - 1) * gap);

      const [outer, inner, monitor] = await Promise.all([
        appWindow.outerSize().catch(() => null),
        appWindow.innerSize().catch(() => null),
        currentMonitor().catch(() => null),
      ]);
      const scaleFactor = monitor?.scaleFactor || window.devicePixelRatio || 1;
      const frameDelta = outer && inner ? Math.max(0, (outer.height - inner.height) / scaleFactor) : 40;
      const maxHeight = monitor
        ? Math.floor(monitor.workArea.size.height / scaleFactor) - 32
        : 760;
      const nextHeight = Math.ceil(
        Math.min(maxHeight, Math.max(MIN_EXPANDED_HEIGHT, contentHeight + frameDelta + 8))
      );

      await appWindow.setSize(new LogicalSize(WINDOW_WIDTH, nextHeight)).catch(() => {});
    };

    resizeToContent();

    return () => {
      cancelled = true;
    };
  }, [showSegments, task?.segments.length, task?.error]);

  if (!task) {
    return <div className="dw-loading">Loading...</div>;
  }

  const progress = task.total_size > 0 ? Math.min((task.downloaded / task.total_size) * 100, 100) : 0;
  const isActive = task.status === 'downloading' || task.status === 'assembling';
  const isFinished = task.status === 'completed';
  const remainingBytes = task.total_size > 0 ? Math.max(task.total_size - task.downloaded, 0) : 0;

  return (
    <div className="dw-root" ref={rootRef}>

      {/* File info strip */}
      <div className="dw-file-strip">
        <span className="dw-file-icon">📄</span>
        <div className="dw-file-details">
          <div className="dw-filename" title={task.filename}>{task.filename}</div>
          <div className="dw-file-url" title={task.url}>{task.url}</div>
        </div>
        <div className={`dw-status-badge dw-status-${task.status}`}>
          {task.status.charAt(0).toUpperCase() + task.status.slice(1)}
        </div>
      </div>

      {/* Progress area */}
      <div className="dw-progress-section">
        <div className="dw-progress-header">
          <span>{formatBytes(task.downloaded)} / {task.total_size > 0 ? formatBytes(task.total_size) : '?'}</span>
          <span className="dw-percent">{progress.toFixed(1)}%</span>
        </div>
        <div className="dw-progress-track">
          <div className="dw-progress-fill" style={{ width: `${progress}%` }} />
        </div>
      </div>

      {/* Stats row */}
      <div className="dw-stats-row">
        <div className="dw-stat">
          <span className="dw-stat-label">Speed</span>
          <span className="dw-stat-value dw-speed">{isActive ? formatSpeed(task.speed_bps) : '—'}</span>
        </div>
        <div className="dw-stat">
          <span className="dw-stat-label">Time Left</span>
          <span className="dw-stat-value">{isActive && task.eta_seconds > 0 ? `${Math.floor(task.eta_seconds)}s` : '—'}</span>
        </div>
        <div className="dw-stat">
          <span className="dw-stat-label">Segments</span>
          <span className="dw-stat-value">{task.num_segments}</span>
        </div>
        <div className="dw-stat">
          <span className="dw-stat-label">Remaining</span>
          <span className="dw-stat-value">{task.total_size > 0 ? formatBytes(remainingBytes) : '—'}</span>
        </div>
        <div className="dw-stat">
          <span className="dw-stat-label">Resume</span>
          <span className="dw-stat-value">{task.supports_range ? 'Yes' : 'No'}</span>
        </div>
      </div>

      {/* Save path */}
      <div className="dw-save-row">
        <span className="dw-save-label">Save to:</span>
        <span className="dw-save-path" title={task.save_path}>{task.save_path}</span>
      </div>

      {task.error && (
        <div className="dw-error-row" title={task.error}>
          {task.error}
        </div>
      )}

      {/* Segments */}
      {task.segments.length > 1 && showSegments && (
        <div className="dw-segments">
          {task.segments.map((seg: any) => {
            const segProgress = seg.progress !== undefined
              ? seg.progress
              : (seg.end && seg.start ? ((seg.downloaded || 0) / (seg.end - seg.start + 1)) * 100 : 0);
            return (
              <div key={seg.id} className="dw-segment">
                <div className="dw-segment-top">
                  <span>Seg {seg.id + 1}</span>
                  <span className="dw-segment-speed">{seg.status === 'downloading' && seg.speed_bps ? formatSpeed(seg.speed_bps) : ''}</span>
                </div>
                <div className="dw-segment-track">
                  <div className={`dw-segment-fill seg-color-${seg.id % 8}`} style={{ width: `${Math.min(segProgress, 100)}%` }} />
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Bottom controls */}
      <div className="dw-controls">
        <div className="dw-speed-limit">
          <input
            type="number"
            value={limitInput}
            onChange={e => setLimitInput(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleSetLimit()}
            placeholder="Unlimited"
            title="Speed limit in KB/s"
            className="dw-limit-input"
          />
          <span className="dw-limit-unit">KB/s</span>
          <button className="dw-btn dw-btn-outline" onClick={handleSetLimit}>Apply</button>
        </div>
        <div className="dw-actions">
          {task.segments.length > 1 && (
            <button className="dw-btn dw-btn-outline" onClick={handleToggleSegments}>
              {showSegments ? 'Less' : 'More'}
            </button>
          )}
          {isActive && <button className="dw-btn dw-btn-secondary" onClick={handlePause}>⏸ Pause</button>}
          {isActive && <button className="dw-btn dw-btn-secondary" onClick={handleStop}>⏹ Stop</button>}
          {!isActive && !isFinished && <button className="dw-btn dw-btn-primary" onClick={handleResume}>▶ Resume</button>}
        </div>
      </div>

    </div>
  );
}
