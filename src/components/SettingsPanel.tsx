import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { emit } from '@tauri-apps/api/event';
import { getVersion } from '@tauri-apps/api/app';
import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import type { AppSettings } from '../types';
import { open } from '@tauri-apps/plugin-dialog';
import { openChildWindow } from '../windowPlacement';
import '../native-ui.css';

type PendingUpdate = NonNullable<Awaited<ReturnType<typeof check>>>;
type UpdateStatus = 'idle' | 'checking' | 'available' | 'current' | 'downloading' | 'installed' | 'error';
type SettingsTab = 'downloads' | 'storage' | 'browser' | 'updates';

const SETTINGS_TABS: Array<{ id: SettingsTab; label: string }> = [
    { id: 'downloads', label: 'Downloads' },
    { id: 'storage', label: 'Storage' },
    { id: 'browser', label: 'Browser Extension' },
    { id: 'updates', label: 'Updates' },
];

function getUpdateErrorMessage(error: unknown) {
    const message = error instanceof Error ? error.message : String(error);
    if (message.includes('latest.json') || message.includes('404')) {
        return 'Update information is not available yet.';
    }
    return message || 'Could not check for updates.';
}

function UpdaterSection() {
    const [currentVersion, setCurrentVersion] = useState('');
    const [pendingUpdate, setPendingUpdate] = useState<PendingUpdate | null>(null);
    const [status, setStatus] = useState<UpdateStatus>('idle');
    const [message, setMessage] = useState('Check for a newer version of Velocity Downloader.');
    const [progress, setProgress] = useState(0);

    useEffect(() => {
        getVersion().then(setCurrentVersion).catch(() => {});
    }, []);

    const handleCheck = async () => {
        setStatus('checking');
        setPendingUpdate(null);
        setProgress(0);
        setMessage('Checking for updates...');

        try {
            const update = await check({ timeout: 30000 });
            if (!update) {
                setStatus('current');
                setMessage('Velocity Downloader is up to date.');
                return;
            }

            setPendingUpdate(update);
            setStatus('available');
            setMessage(`Version ${update.version} is available.`);
        } catch (error) {
            setStatus('error');
            setMessage(getUpdateErrorMessage(error));
        }
    };

    const handleInstall = async () => {
        if (!pendingUpdate) return;

        setStatus('downloading');
        setProgress(0);
        setMessage('Downloading update...');

        let downloaded = 0;
        let contentLength = 0;

        try {
            await pendingUpdate.downloadAndInstall((event) => {
                switch (event.event) {
                    case 'Started':
                        downloaded = 0;
                        contentLength = event.data.contentLength ?? 0;
                        setProgress(0);
                        break;
                    case 'Progress':
                        downloaded += event.data.chunkLength;
                        if (contentLength > 0) {
                            setProgress(Math.min(100, Math.round((downloaded / contentLength) * 100)));
                        }
                        break;
                    case 'Finished':
                        setProgress(100);
                        setMessage('Update installed. Restarting...');
                        break;
                }
            });

            setStatus('installed');
            await relaunch();
        } catch (error) {
            setStatus('error');
            setMessage(getUpdateErrorMessage(error));
        }
    };

    const busy = status === 'checking' || status === 'downloading';
    const statusClass = status === 'available' || status === 'installed'
        ? 'ok'
        : status === 'error'
            ? 'error'
            : 'idle';
    const statusLabel = status === 'checking'
        ? 'Checking'
        : status === 'downloading'
            ? `${progress}%`
            : status === 'available'
                ? 'Update available'
                : status === 'installed'
                    ? 'Installed'
                    : status === 'error'
                        ? 'Problem'
                        : null;

    return (
        <div className="ns-section">
            <div className="ns-row">
                <div className="ns-row-info">
                    <span className="ns-row-label">Velocity Updates</span>
                    <span className="ns-row-desc">
                        {currentVersion ? `Installed version: ${currentVersion}. ` : ''}
                        {message}
                    </span>
                    {pendingUpdate?.body && (
                        <span className="ns-row-desc ns-update-notes">{pendingUpdate.body}</span>
                    )}
                    {status === 'downloading' && (
                        <div className="ns-update-progress">
                            <div className="ns-update-progress-fill" style={{ width: `${progress}%` }} />
                        </div>
                    )}
                </div>
                <div className="ns-update-actions">
                    {statusLabel && (
                        <span className={`ns-status ${statusClass}`}>
                            <span className="ns-status-dot" />
                            {statusLabel}
                        </span>
                    )}
                    <button className="dw-btn dw-btn-secondary" disabled={busy} onClick={handleCheck}>
                        Check now
                    </button>
                    <button className="dw-btn dw-btn-primary" disabled={!pendingUpdate || busy} onClick={handleInstall}>
                        Install update
                    </button>
                </div>
            </div>
        </div>
    );
}

export function SettingsPanel() {
    const [local, setLocal] = useState<AppSettings | null>(null);
    const [savedIndicator, setSavedIndicator] = useState(false);
    const [apiStatus, setApiStatus] = useState<'idle' | 'ok' | 'error'>('idle');
    const [activeTab, setActiveTab] = useState<SettingsTab>('downloads');

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

    const handleTempPicker = async () => {
        if (!local) return;
        try {
            const selected = await open({ directory: true, multiple: false, defaultPath: local.temp_download_dir || undefined });
            if (selected) updateSetting({ temp_download_dir: Array.isArray(selected) ? selected[0] : selected });
        } catch {}
    };

    const openExtensionManager = () => {
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
    };

    if (!local) return <div className="ns-panel" style={{ color: '#888' }}>Loading settings...</div>;

    return (
        <div className="ns-panel">
            <div className="ns-title-row">
                <h2>Options</h2>
                {savedIndicator && (
                    <span style={{ color: '#4ec994', fontSize: '13px', display: 'flex', alignItems: 'center', gap: '6px' }}>
                        <span className="ns-status-dot" style={{ backgroundColor: '#4ec994' }} /> Saved
                    </span>
                )}
            </div>

            <div className="ns-tabs" role="tablist" aria-label="Settings sections">
                {SETTINGS_TABS.map(tab => (
                    <button
                        key={tab.id}
                        className={`ns-tab ${activeTab === tab.id ? 'active' : ''}`}
                        role="tab"
                        aria-selected={activeTab === tab.id}
                        onClick={() => setActiveTab(tab.id)}
                    >
                        {tab.label}
                    </button>
                ))}
            </div>

            <div className="ns-tab-body">
                {activeTab === 'downloads' && (
                    <div className="ns-tab-panel" role="tabpanel">
                        <div className="ns-row">
                            <div className="ns-row-info">
                                <span className="ns-row-label">Connections per File</span>
                                <span className="ns-row-desc">More connections can make supported downloads faster.</span>
                            </div>
                            <div className="ns-input-suffix">
                                <input
                                    type="number"
                                    className="ns-input"
                                    value={local.default_segments}
                                    min={1}
                                    max={32}
                                    onChange={e => updateSetting({ default_segments: parseInt(e.target.value) || 8 })}
                                />
                            </div>
                        </div>

                        <div className="ns-row">
                            <div className="ns-row-info">
                                <span className="ns-row-label">Download Speed Limit</span>
                                <span className="ns-row-desc">Set to 0 for unlimited speed.</span>
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
                                <span className="ns-row-desc">Launch Velocity Downloader in the background when you sign in.</span>
                            </div>
                            <label className="ns-check">
                                <input
                                    type="checkbox"
                                    checked={local.start_on_boot}
                                    onChange={e => updateSetting({ start_on_boot: e.target.checked })}
                                />
                                Enabled
                            </label>
                        </div>
                    </div>
                )}

                {activeTab === 'storage' && (
                    <div className="ns-tab-panel" role="tabpanel">
                        <div className="ns-row">
                            <div className="ns-row-info">
                                <span className="ns-row-label">Default Save Location</span>
                                <span className="ns-row-desc">Where downloads are saved by default.</span>
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
                                <span className="ns-row-label">Temporary Files Location</span>
                                <span className="ns-row-desc">Used while files are downloading. Leave empty to use the save location.</span>
                            </div>
                            <div className="ns-path-row">
                                <div
                                    className="ns-path-display"
                                    onClick={handleTempPicker}
                                    title={local.temp_download_dir || 'Same as save location'}
                                >
                                    {local.temp_download_dir || 'Same as save location'}
                                </div>
                                <button className="dw-btn dw-btn-secondary" onClick={handleTempPicker}>
                                    Browse
                                </button>
                                {local.temp_download_dir && (
                                    <button
                                        className="dw-btn dw-btn-outline"
                                        style={{ padding: '5px 8px' }}
                                        onClick={() => updateSetting({ temp_download_dir: null })}
                                        title="Reset to default"
                                    >
                                        x
                                    </button>
                                )}
                            </div>
                        </div>
                    </div>
                )}

                {activeTab === 'browser' && (
                    <div className="ns-tab-panel" role="tabpanel">
                        <div className="ns-row">
                            <div className="ns-row-info">
                                <span className="ns-row-label">Extension Connection</span>
                                <span className="ns-row-desc">
                                    {apiStatus === 'ok'
                                        ? 'Velocity can receive downloads from your browser.'
                                        : apiStatus === 'error'
                                            ? 'Open the extension manager and make sure the extension is installed and enabled.'
                                            : 'Checking browser extension connection...'}
                                </span>
                            </div>
                            <span className={`ns-status ${apiStatus}`}>
                                <span className="ns-status-dot" />
                                {apiStatus === 'ok' ? 'Extension connected' : apiStatus === 'error' ? 'Not connected' : 'Checking'}
                            </span>
                        </div>

                        <div className="ns-row">
                            <div className="ns-row-info">
                                <span className="ns-row-label">Browser Extension</span>
                                <span className="ns-row-desc">Install or manage the browser extension.</span>
                            </div>
                            <button className="dw-btn dw-btn-secondary" onClick={openExtensionManager}>
                                Manage Extension
                            </button>
                        </div>
                    </div>
                )}

                {activeTab === 'updates' && (
                    <div className="ns-tab-panel" role="tabpanel">
                        <UpdaterSection />
                    </div>
                )}
            </div>
        </div>
    );
}
