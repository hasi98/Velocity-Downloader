import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type { ProgressEvent } from '../types';
import '../native-ui.css';

interface QueueItem {
    id: string; // The download ID if started
    url: string;
    status: 'pending' | 'downloading' | 'completed' | 'failed';
    error?: string;
    progress?: number;
}

export function BatchDownloadWindow() {
    const [urlsInput, setUrlsInput] = useState('');
    const [mode, setMode] = useState<'all' | 'sequential'>('all');
    const [savePath, setSavePath] = useState('');
    const [queue, setQueue] = useState<QueueItem[]>([]);
    const [isStarted, setIsStarted] = useState(false);
    const [error, setError] = useState('');

    const [currentIdx, setCurrentIdx] = useState(-1);
    const queueRef = useRef<QueueItem[]>([]);
    const activeListener = useRef<UnlistenFn | null>(null);

    useEffect(() => {
        const init = async () => {
            try {
                const dir = await invoke<string>('get_default_download_dir');
                setSavePath(dir);
            } catch (e) {
                console.error(e);
            }
        };
        init();
        return () => {
            if (activeListener.current) activeListener.current();
        };
    }, []);

    useEffect(() => {
        queueRef.current = queue;
    }, [queue]);

    const handlePicker = async () => {
        try {
            const selected = await open({ directory: true, multiple: false, defaultPath: savePath || undefined });
            if (selected) setSavePath(Array.isArray(selected) ? selected[0] : selected);
        } catch { }
    };

    const startBatch = async () => {
        if (!urlsInput.trim()) return;

        const urls = urlsInput.split('\n')
            .map(u => u.trim())
            .filter(u => u.length > 0 && (u.startsWith('http') || u.startsWith('ftp')));

        if (urls.length === 0) {
            setError('No valid URLs found.');
            return;
        }

        const initialQueue: QueueItem[] = urls.map(u => ({
            id: '',
            url: u,
            status: 'pending'
        }));

        setQueue(initialQueue);
        setIsStarted(true);
        setError('');

        if (mode === 'all') {
            // Parallel: just start them all
            for (let i = 0; i < urls.length; i++) {
                try {
                    await invoke('add_download', {
                        url: urls[i],
                        savePath: savePath || null
                    });
                    updateQueueItem(i, { status: 'downloading' });
                } catch (e: any) {
                    updateQueueItem(i, { status: 'failed', error: typeof e === 'string' ? e : 'Start failed' });
                }
            }
        } else {
            // Sequential
            runSequential(initialQueue, 0);
        }
    };

    const runSequential = async (currentQueue: QueueItem[], index: number) => {
        if (index >= currentQueue.length) {
            console.log('Batch complete');
            return;
        }

        setCurrentIdx(index);
        const url = currentQueue[index].url;
        updateQueueItem(index, { status: 'downloading' });

        try {
            const task: any = await invoke('add_download', {
                url,
                savePath: savePath || null
            });
            
            // Wait for completion
            let unlisten: UnlistenFn | null = null;
            const promise = new Promise<void>((resolve) => {
                listen<ProgressEvent>('download-progress', (event) => {
                    const p = event.payload;
                    if (p.download_id === task.id) {
                        const progress = p.total_size > 0 ? (p.downloaded / p.total_size) * 100 : 0;
                        updateQueueItem(index, { progress });

                        if (p.status === 'completed') {
                            updateQueueItem(index, { status: 'completed', progress: 100 });
                            if (unlisten) unlisten();
                            resolve();
                        } else if (p.status === 'failed') {
                            updateQueueItem(index, { status: 'failed', error: 'Download failed' });
                            if (unlisten) unlisten();
                            alert(`Download failed for: ${url}\nMoving to next file.`);
                            resolve();
                        }
                    }
                }).then(u => { unlisten = u; activeListener.current = u; });
            });

            await promise;
            // Move to next
            runSequential(queueRef.current, index + 1);

        } catch (e: any) {
            const ErrorMsg = typeof e === 'string' ? e : 'Failed to probe/start';
            updateQueueItem(index, { status: 'failed', error: ErrorMsg });
            alert(`Could not start download for: ${url}\nError: ${ErrorMsg}\nMoving to next file.`);
            runSequential(queueRef.current, index + 1);
        }
    };

    const updateQueueItem = (index: number, updates: Partial<QueueItem>) => {
        setQueue(prev => {
            const next = [...prev];
            next[index] = { ...next[index], ...updates };
            return next;
        });
    };

    const handleClose = async () => {
        try {
            await getCurrentWebviewWindow().close();
        } catch (e) {
            console.error('Failed to close window via Tauri:', e);
            window.close(); // Browser fallback
        }
    };

    return (
        <div className="native-app dw-root">
            <div className="nd-titlebar" style={{ borderRadius: '4px 4px 0 0' }}>
                <span>Batch Downloader</span>
                <button className="nd-close-btn" onClick={handleClose}>×</button>
            </div>

            {!isStarted ? (
                <div className="nd-body" style={{ flex: 1, padding: '16px', overflowY: 'auto' }}>
                    <div className="nd-field">
                        <label className="nd-label">Enter URLs (one per line)</label>
                        <textarea
                            className="nd-input"
                            style={{ height: '140px', resize: 'none', fontFamily: 'monospace' }}
                            placeholder="https://example.com/file1.zip&#10;https://example.com/file2.mp4"
                            value={urlsInput}
                            onChange={e => setUrlsInput(e.target.value)}
                            autoFocus
                        />
                    </div>

                    <div className="nd-field">
                        <label className="nd-label">Download Mode</label>
                        <div className="nd-row">
                            <label className="batch-mode-opt">
                                <input type="radio" checked={mode === 'all'} onChange={() => setMode('all')} />
                                All at once (Parallel)
                            </label>
                            <label className="batch-mode-opt">
                                <input type="radio" checked={mode === 'sequential'} onChange={() => setMode('sequential')} />
                                1 by 1 (Sequential)
                            </label>
                        </div>
                    </div>

                    <div className="nd-field">
                        <label className="nd-label">Save to Directory</label>
                        <div className="nd-row">
                            <input
                                type="text"
                                className="nd-input"
                                value={savePath}
                                readOnly
                                onClick={handlePicker}
                                style={{ cursor: 'pointer' }}
                            />
                            <button className="dw-btn dw-btn-secondary" onClick={handlePicker}>📂</button>
                        </div>
                    </div>

                    {error && <div className="nd-error">{error}</div>}
                </div>
            ) : (
                <div className="nd-body" style={{ flex: 1, padding: '16px', overflowY: 'auto' }}>
                    <div className="batch-queue">
                        {queue.map((item, i) => (
                            <div key={i} className={`batch-item ${currentIdx === i ? 'active' : ''} ${item.status}`}>
                                <div className="batch-item-info">
                                    <span className="batch-item-idx">{i + 1}</span>
                                    <span className="batch-item-url">{item.url}</span>
                                    <span className={`batch-item-status status-${item.status}`}>
                                        {item.status === 'downloading' && item.progress !== undefined ? `${Math.round(item.progress)}%` : item.status}
                                    </span>
                                </div>
                                {item.status === 'downloading' && (
                                    <div className="batch-item-progress">
                                        <div className="batch-item-fill" style={{ width: `${item.progress || 0}%` }} />
                                    </div>
                                )}
                                {item.error && <div className="batch-item-error">{item.error}</div>}
                            </div>
                        ))}
                    </div>
                </div>
            )}

            <div className="nd-footer">
                {!isStarted ? (
                    <>
                        <button className="dw-btn dw-btn-secondary" onClick={handleClose}>Cancel</button>
                        <button className="dw-btn dw-btn-primary" onClick={startBatch} disabled={!urlsInput.trim()}>
                            🚀 Start Batch Download
                        </button>
                    </>
                ) : (
                    <button className="dw-btn dw-btn-primary" onClick={handleClose}>Close Window</button>
                )}
            </div>

            <style>{`
                .batch-mode-opt {
                    display: flex;
                    align-items: center;
                    gap: 8px;
                    font-size: 13px;
                    color: #ccc;
                    cursor: pointer;
                    background: #252526;
                    padding: 8px 12px;
                    border-radius: 4px;
                    border: 1px solid #3f3f46;
                    flex: 1;
                }
                .batch-mode-opt:hover {
                    background: #2d2d30;
                }
                .batch-queue {
                    display: flex;
                    flex-direction: column;
                    gap: 8px;
                }
                .batch-item {
                    background: #252526;
                    border: 1px solid #3f3f46;
                    border-radius: 4px;
                    padding: 10px;
                    display: flex;
                    flex-direction: column;
                    gap: 6px;
                }
                .batch-item.active {
                    border-color: #0078d4;
                    background: #2d2d30;
                    box-shadow: 0 0 10px rgba(0, 120, 212, 0.2);
                }
                .batch-item-info {
                    display: flex;
                    align-items: center;
                    gap: 10px;
                    font-size: 12px;
                }
                .batch-item-idx {
                    color: #555;
                    font-weight: 700;
                    width: 16px;
                    text-align: right;
                }
                .batch-item-url {
                    flex: 1;
                    white-space: nowrap;
                    overflow: hidden;
                    text-overflow: ellipsis;
                    color: #bbb;
                }
                .batch-item-status {
                    font-weight: 600;
                    text-transform: uppercase;
                    font-size: 10px;
                    padding: 2px 6px;
                    border-radius: 3px;
                }
                .status-pending { background: #333; color: #888; }
                .status-downloading { background: #004a7c; color: #6ec6f5; }
                .status-completed { background: #0b3d2b; color: #4ec994; }
                .status-failed { background: #4d0b0b; color: #f07070; }
                
                .batch-item-progress {
                    height: 4px;
                    background: #1e1e1e;
                    border-radius: 2px;
                    overflow: hidden;
                }
                .batch-item-fill {
                    height: 100%;
                    background: #0078d4;
                    transition: width 0.3s;
                }
                .batch-item-error {
                    font-size: 11px;
                    color: #f07070;
                    padding-left: 26px;
                }
            `}</style>
        </div>
    );
}
