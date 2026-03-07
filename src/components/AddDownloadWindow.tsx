import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { open } from '@tauri-apps/plugin-dialog';
import { formatBytes, getFileCategory, getCategorizedPath } from '../utils';
import type { ProbeResult, HttpContext } from '../types';
import '../native-ui.css';

export function AddDownloadWindow() {
    const [url, setUrl] = useState('');
    const [savePath, setSavePath] = useState('');
    const [defaultDir, setDefaultDir] = useState('');
    const [probing, setProbing] = useState(false);
    const [probeResult, setProbeResult] = useState<ProbeResult | null>(null);
    const [error, setError] = useState('');
    const [httpContext, setHttpContext] = useState<HttpContext>({});

    useEffect(() => {
        const init = async () => {
            try {
                const dir = await invoke<string>('get_default_download_dir');
                setDefaultDir(dir);
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
                        
                        // Auto-probe if URL is provided
                        if (payload.url) {
                            probeUrl(payload.url, {
                                cookies: payload.cookies,
                                referer: payload.referer,
                                user_agent: payload.user_agent,
                            }, dir);
                        }
                    }
                }
            } catch (e) {
                console.error(e);
            }
        };
        init();
    }, []);

    const probeUrl = async (targetUrl: string, ctx: HttpContext, defDir: string) => {
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
            setProbeResult(result);
            setSavePath(prev => prev === defDir ? getCategorizedPath(defDir, getFileCategory(result.filename)) : prev);
        } catch (e: any) {
            setError(typeof e === 'string' ? e : e.message || 'Failed to probe URL');
        } finally {
            setProbing(false);
        }
    };

    const handleProbe = () => {
        if (!url.trim()) return;
        probeUrl(url.trim(), httpContext, defaultDir);
    };

    const handlePicker = async () => {
        try {
            const selected = await open({ directory: true, multiple: false, defaultPath: savePath || undefined });
            if (selected) setSavePath(Array.isArray(selected) ? selected[0] : selected);
        } catch {}
    };

    const handleSubmit = async () => {
        if (!url.trim()) return;
        try {
            await invoke('add_download', {
                url: url.trim(),
                savePath: savePath || null,
                cookies: httpContext.cookies || null,
                referer: httpContext.referer || null,
                userAgent: httpContext.user_agent || null,
            });
            getCurrentWebviewWindow().close();
        } catch (e: any) {
            setError(typeof e === 'string' ? e : 'Failed to start download');
        }
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
        <div className="native-app" style={{ height: '100vh', display: 'flex', flexDirection: 'column', overflow: 'hidden', padding: '16px', boxSizing: 'border-box' }}>
            <div style={{ flex: 1, display: 'flex', flexDirection: 'column' }}>
                <div className="nd-body" style={{ flex: 1, padding: 0, overflowY: 'auto' }}>
                    <div className="nd-field">
                        <label className="nd-label">Download URL</label>
                        <div className="nd-row">
                            <input
                                type="text"
                                className="nd-input"
                                placeholder="https://example.com/file.zip"
                                value={url}
                                onChange={e => setUrl(e.target.value)}
                                onKeyDown={e => e.key === 'Enter' && handleProbe()}
                                autoFocus
                            />
                            <button className="dw-btn dw-btn-secondary" onClick={handleProbe} disabled={probing || !url.trim()}>
                                {probing ? '...' : '🔍 Analyze'}
                            </button>
                        </div>
                    </div>

                    {error && (
                        <div className="nd-error">⚠️ {error}</div>
                    )}

                    {probeResult && (
                        <div className="nd-probe-result">
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

                <div className="nd-footer" style={{ marginTop: 'auto', paddingTop: '16px', borderTop: '1px solid var(--border-light)' }}>
                    <button className="dw-btn dw-btn-secondary" onClick={handleClose}>Cancel</button>
                    <button className="dw-btn dw-btn-primary" onClick={handleSubmit} disabled={!url.trim()}>
                        ⬇ Start Download
                    </button>
                </div>
            </div>
        </div>
    );
}
