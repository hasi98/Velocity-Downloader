import { useState, useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { LogicalSize } from '@tauri-apps/api/dpi';
import { open } from '@tauri-apps/plugin-dialog';
import { formatBytes } from '../utils';
import type { AppSettings, DownloadTask, ProbeResult, HttpContext } from '../types';
import '../native-ui.css';

const ADD_WINDOW_WIDTH = 600;
const ADD_COMPACT_HEIGHT = 360;
const ADD_EXPANDED_HEIGHT = 500;

export function AddDownloadWindow() {
    const [url, setUrl] = useState('');
    const [savePath, setSavePath] = useState('');
    const [filename, setFilename] = useState('');
    const [probing, setProbing] = useState(false);
    const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
    const [error, setError] = useState('');
    const [httpContext, setHttpContext] = useState<HttpContext>({});
    const probeRequestId = useRef(0);
    const prefetchId = useRef<string | null>(null);

    useEffect(() => {
        const init = async () => {
            try {
                const settings = await invoke<AppSettings>('get_settings');
                const dir = settings.default_download_dir;
                setSavePath(dir);

                // Load payload from local storage
                const pendingId = new URLSearchParams(window.location.search).get('id');
                if (pendingId) {
                    const payloadStr = localStorage.getItem(`add-dl-${pendingId}`);
                    if (payloadStr) {
                        const payload = JSON.parse(payloadStr);
                        setUrl(payload.url || '');
                        setHttpContext({
                            cookies: payload.cookies,
                            referer: payload.referer,
                            user_agent: payload.user_agent,
                        });
                        localStorage.removeItem(`add-dl-${pendingId}`);
                    }
                }
            } catch (e) {
                console.error(e);
            }
        };
        init();
    }, []);

    const cancelPrefetch = async () => {
        const id = prefetchId.current;
        if (!id) return;
        prefetchId.current = null;
        try {
            await invoke('remove_download', { downloadId: id });
        } catch {}
    };

    useEffect(() => {
        return () => {
            void cancelPrefetch();
        };
    }, []);

    useEffect(() => {
        const nextHeight = probeResult || error ? ADD_EXPANDED_HEIGHT : ADD_COMPACT_HEIGHT;
        getCurrentWebviewWindow()
            .setSize(new LogicalSize(ADD_WINDOW_WIDTH, nextHeight))
            .catch(() => {});
    }, [probeResult, error]);

    const probeUrl = async (targetUrl: string, ctx: HttpContext, targetSavePath: string) => {
        const requestId = ++probeRequestId.current;
        setProbing(true);
        setError('');
        setProbeResult(null);
        try {
            const result = await invoke<ProbeResult>('probe_url', { 
                url: targetUrl,
                cookies: ctx.cookies || null,
                referer: ctx.referer || null,
                userAgent: ctx.user_agent || null
            });
            if (requestId === probeRequestId.current) {
                setProbeResult(result);
                const effectiveFilename = filename.trim() || result.filename;
                setFilename(effectiveFilename);
                if (targetSavePath && result.supports_range) {
                    try {
                        const task = await invoke<DownloadTask>('prefetch_download', {
                            url: targetUrl,
                            savePath: targetSavePath,
                            filename: effectiveFilename,
                            cookies: ctx.cookies || null,
                            referer: ctx.referer || null,
                            userAgent: ctx.user_agent || null,
                        });
                        if (requestId === probeRequestId.current) {
                            prefetchId.current = task.id;
                        } else {
                            await invoke('remove_download', { downloadId: task.id });
                        }
                    } catch (e: any) {
                        if (requestId === probeRequestId.current) {
                            setError(typeof e === 'string' ? e : e.message || 'Failed to start prefetch');
                        }
                    }
                }
            }
        } catch (e: any) {
            if (requestId === probeRequestId.current) {
                setError(typeof e === 'string' ? e : e.message || 'Failed to probe URL');
            }
        } finally {
            if (requestId === probeRequestId.current) {
                setProbing(false);
            }
        }
    };

    useEffect(() => {
        const targetUrl = url.trim();
        probeRequestId.current += 1;
        cancelPrefetch();
        setProbeResult(null);
        setError('');

        if (!targetUrl || !savePath || !/^(https?|ftp):\/\//i.test(targetUrl)) {
            setProbing(false);
            return;
        }

        setProbing(true);
        const timer = window.setTimeout(() => {
            probeUrl(targetUrl, httpContext, savePath);
        }, 650);

        return () => window.clearTimeout(timer);
    }, [url, savePath, httpContext]);

    const handlePicker = async () => {
        try {
            const selected = await open({ directory: true, multiple: false, defaultPath: savePath || undefined });
            if (selected) setSavePath(Array.isArray(selected) ? selected[0] : selected);
        } catch {}
    };

    const handleSubmit = async () => {
        if (!url.trim() || probing || !probeResult) return;
        try {
            if (prefetchId.current) {
                const id = prefetchId.current;
                prefetchId.current = null;
                await invoke('reveal_download', { downloadId: id });
            } else {
                await invoke('add_download', {
                    url: url.trim(),
                    savePath: savePath || null,
                    filename: filename.trim() || null,
                    cookies: httpContext.cookies || null,
                    referer: httpContext.referer || null,
                    userAgent: httpContext.user_agent || null,
                });
            }
            getCurrentWebviewWindow().close();
        } catch (e: any) {
            setError(typeof e === 'string' ? e : 'Failed to start download');
        }
    };

    const handleClose = async () => {
        try {
            await cancelPrefetch();
            await getCurrentWebviewWindow().close();
        } catch (e) {
            console.error('Failed to close window via Tauri:', e);
            window.close(); // Browser fallback
        }
    };

    return (
        <div className="native-app" style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden', padding: '16px', boxSizing: 'border-box' }}>
            <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
                <div className="nd-body" style={{ flex: 1, minHeight: 0, padding: 0, overflowY: 'auto' }}>
                    <div className="nd-field">
                        <label className="nd-label">Download URL</label>
                        <div className="nd-row">
                            <input
                                type="text"
                                className="nd-input"
                                placeholder="https://example.com/file.zip"
                                value={url}
                                onChange={e => {
                                    setUrl(e.target.value);
                                    setFilename('');
                                }}
                                autoFocus
                            />
                        </div>
                    </div>

                    {error && (
                        <div className="nd-error">⚠️ {error}</div>
                    )}

                    {probeResult && (
                        <div className="nd-probe-result">
                            <div className="nd-field" style={{ gap: '6px' }}>
                                <label className="nd-label">File Name</label>
                                <input
                                    type="text"
                                    className="nd-input"
                                    value={filename}
                                    onChange={e => {
                                        setFilename(e.target.value);
                                        cancelPrefetch();
                                    }}
                                />
                            </div>
                            <div className="nd-probe-row">
                                <span>Filename</span>
                                <span className="nd-probe-value">{probeResult.filename}</span>
                            </div>
                            <div className="nd-probe-row">
                                <span>Size</span>
                                <span className="nd-probe-value">{formatBytes(probeResult.size)}</span>
                            </div>
                            <div className="nd-probe-row">
                                <span>Multi-part</span>
                                <span className={`nd-probe-value ${probeResult.supports_range ? 'nd-good' : 'nd-warn'}`}>
                                    {probeResult.supports_range ? '✓ Supported' : '✕ Not supported'}
                                </span>
                            </div>
                            {probeResult.content_type && (
                                <div className="nd-probe-row">
                                    <span>Type</span>
                                    <span className="nd-probe-value">{probeResult.content_type}</span>
                                </div>
                            )}
                        </div>
                    )}

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
                            <button className="dw-btn dw-btn-secondary" onClick={handlePicker} title="Browse">📂</button>
                        </div>
                    </div>
                </div>

                <div className="nd-footer" style={{ flexShrink: 0, marginTop: '12px', paddingTop: '12px', borderTop: '1px solid var(--border-light)' }}>
                    <button className="dw-btn dw-btn-secondary" onClick={handleClose}>Cancel</button>
                    <button className="dw-btn dw-btn-primary" onClick={handleSubmit} disabled={!url.trim() || probing || !probeResult}>
                        {probing ? 'Analyzing...' : '⬇ Start Download'}
                    </button>
                </div>
            </div>
        </div>
    );
}
