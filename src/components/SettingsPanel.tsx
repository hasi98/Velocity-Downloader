import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import type { AppSettings } from '../types';
import { open } from '@tauri-apps/plugin-dialog';
import { openChildWindow } from '../windowPlacement';
import '../native-ui.css';

export function SettingsPanel() {
    const [local, setLocal] = useState<AppSettings | null>(null);
    const [savedIndicator, setSavedIndicator] = useState(false);
    const [apiStatus, setApiStatus] = useState<'idle' | 'ok' | 'error'>('idle');

    useEffect(() => {
        invoke<AppSettings>('get_settings').then(setLocal).catch(console.error);
    }, []);

    useEffect(() => {
        const checkApi = async () => {
            try {
                const res = await fetch('http://127.0.0.1:41420/ping');
                setApiStatus(res.ok ? 'ok' : 'error');
            } catch {
                setApiStatus('error');
            }
        };
        checkApi();
        const interval = setInterval(checkApi, 5000);
        return () => clearInterval(interval);
    }, []);

    const updateSetting = async (updates: Partial<AppSettings>) => {
        if (!local) return;
        const newSettings = { ...local, ...updates };
        setLocal(newSettings);
        try {
            await invoke('update_settings', { settings: newSettings });
            await emit('settings-changed', newSettings);
            setSavedIndicator(true);
            setTimeout(() => setSavedIndicator(false), 2000);
        } catch (e) {
            console.error(e);
        }
    };

    const handlePicker = async () => {
        if (!local) return;
        try {
            const selected = await open({ directory: true, multiple: false, defaultPath: local.default_download_dir || undefined });
            if (selected) updateSetting({ default_download_dir: Array.isArray(selected) ? selected[0] : selected });
        } catch {}
    };

    if (!local) return <div className="ns-panel" style={{ color: '#888' }}>Loading settings...</div>;

    return (
        <div className="ns-panel">
            <div className="ns-title-row">
                <h2>Options</h2>
                {savedIndicator && <span style={{ color: '#4ec994', fontSize: '13px', display: 'flex', alignItems: 'center', gap: '6px' }}><span className="ns-status-dot" style={{ backgroundColor: '#4ec994' }} /> Saved</span>}
            </div>

            {/* Downloads */}
            <div className="ns-section">
                <div className="ns-section-title">Downloads</div>



                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Connections per File (Segments)</span>
                        <span className="ns-row-desc">Higher = faster for most servers (8–16 recommended)</span>
                    </div>
                    <div className="ns-input-suffix">
                        <input
                            type="number"
                            className="ns-input"
                            value={local.default_segments}
                            min={1} max={32}
                            onChange={e => updateSetting({ default_segments: parseInt(e.target.value) || 8 })}
                        />
                    </div>
                </div>

                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Global Speed Limit</span>
                        <span className="ns-row-desc">0 = unlimited</span>
                    </div>
                    <div className="ns-input-suffix">
                        <input
                            type="number"
                            className="ns-input"
                            value={local.speed_limit_bps ? Math.floor(local.speed_limit_bps / 1024) : 0}
                            min={0}
                            onChange={e => {
                                const val = parseInt(e.target.value) || 0;
                                updateSetting({ speed_limit_bps: val > 0 ? val * 1024 : null });
                            }}
                        />
                        <span className="ns-suffix-label">KB/s</span>
                    </div>
                </div>

                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Start with Windows</span>
                        <span className="ns-row-desc">Launch Velocity Downloader in the background when you sign in</span>
                    </div>
                    <label style={{ display: 'flex', alignItems: 'center', gap: '8px', color: '#ccc', fontSize: '13px' }}>
                        <input
                            type="checkbox"
                            checked={local.start_on_boot}
                            onChange={e => updateSetting({ start_on_boot: e.target.checked })}
                        />
                        Enabled
                    </label>
                </div>
            </div>

            {/* Storage */}
            <div className="ns-section">
                <div className="ns-section-title">Storage</div>
                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Default Save Location</span>
                        <span className="ns-row-desc">Where downloads are saved by default</span>
                    </div>
                    <div className="ns-path-row">
                        <div className="ns-path-display" onClick={handlePicker} title={local.default_download_dir}>
                            {local.default_download_dir || 'Not set'}
                        </div>
                        <button className="dw-btn dw-btn-secondary" onClick={handlePicker}>Browse</button>
                    </div>
                </div>
                
                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Temporary Download Directory</span>
                        <span className="ns-row-desc">Where segments are stored while downloading (leaves blank for same as save location)</span>
                    </div>
                    <div className="ns-path-row">
                        <div 
                            className="ns-path-display" 
                            onClick={async () => {
                                try {
                                    const selected = await open({ directory: true, multiple: false, defaultPath: local.temp_download_dir || undefined });
                                    if (selected) updateSetting({ temp_download_dir: Array.isArray(selected) ? selected[0] : selected });
                                } catch {}
                            }} 
                            title={local.temp_download_dir || 'Same as save location'}
                        >
                            {local.temp_download_dir || 'Same as save location'}
                        </div>
                        <button 
                            className="dw-btn dw-btn-secondary" 
                            onClick={async () => {
                                try {
                                    const selected = await open({ directory: true, multiple: false, defaultPath: local.temp_download_dir || undefined });
                                    if (selected) updateSetting({ temp_download_dir: Array.isArray(selected) ? selected[0] : selected });
                                } catch {}
                            }}
                        >
                            Browse
                        </button>
                        {local.temp_download_dir && (
                            <button 
                                className="dw-btn dw-btn-outline" 
                                style={{ padding: '5px 8px' }}
                                onClick={() => updateSetting({ temp_download_dir: null })}
                                title="Reset to default"
                            >
                                ✕
                            </button>
                        )}
                    </div>
                </div>
            </div>

            {/* Browser Integration */}
            <div className="ns-section">
                <div className="ns-section-title">Browser Integration</div>
                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Native Messenger Server</span>
                        <span className="ns-row-desc">Required for capturing browser downloads (port 41420)</span>
                    </div>
                    <span className={`ns-status ${apiStatus}`}>
                        <span className="ns-status-dot" />
                        {apiStatus === 'ok' ? 'Active' : apiStatus === 'error' ? 'Not Running' : 'Checking...'}
                    </span>
                </div>
                <div className="ns-row">
                    <div className="ns-row-info">
                        <span className="ns-row-label">Browser Extension</span>
                        <span className="ns-row-desc">Install the extension to intercept downloads from your browser</span>
                    </div>
                    <button
                        className="dw-btn dw-btn-secondary"
                        onClick={() => {
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
                        }}
                    >
                        🧩 Manage Extension
                    </button>
                </div>
            </div>
        </div>
    );
}
