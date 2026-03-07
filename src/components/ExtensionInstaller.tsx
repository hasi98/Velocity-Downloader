import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import { writeText } from '@tauri-apps/plugin-clipboard-manager';

interface ExtensionInstallerProps {
    onClose: () => void;
    inline?: boolean;
}

const BROWSERS = [
    { id: 'chrome', label: 'Chrome', icon: '🌐' },
    { id: 'edge', label: 'Edge', icon: '🔷' },
    { id: 'brave', label: 'Brave', icon: '🦁' },
    { id: 'opera', label: 'Opera', icon: '⭕' },
    { id: 'vivaldi', label: 'Vivaldi', icon: '🎵' },
];

export function ExtensionInstaller({ onClose, inline = false }: ExtensionInstallerProps) {
    const [selectedBrowser, setSelectedBrowser] = useState('chrome');
    const [status, setStatus] = useState<'idle' | 'preparing' | 'ready' | 'error'>('idle');
    const [extPath, setExtPath] = useState('');
    const [errorMsg, setErrorMsg] = useState('');
    const [copiedPath, setCopiedPath] = useState(false);
    const [copiedUrl, setCopiedUrl] = useState(false);

    const selectedLabel = BROWSERS.find(b => b.id === selectedBrowser)?.label ?? selectedBrowser;

    const handlePrepare = async () => {
        setStatus('preparing');
        setErrorMsg('');
        try {
            const path = await invoke<string>('install_extension', { browser: selectedBrowser });
            setExtPath(path);
            setStatus('ready');
        } catch (e: any) {
            setStatus('error');
            setErrorMsg(typeof e === 'string' ? e : (e?.message ?? 'Unknown error'));
        }
    };

    const handleCopyPath = async () => {
        try {
            await writeText(extPath);
            setCopiedPath(true);
            setTimeout(() => setCopiedPath(false), 2000);
        } catch {
            navigator.clipboard.writeText(extPath).catch(() => { });
            setCopiedPath(true);
            setTimeout(() => setCopiedPath(false), 2000);
        }
    };

    const handleCopyUrl = async (url: string) => {
        try {
            await writeText(url);
            setCopiedUrl(true);
            setTimeout(() => setCopiedUrl(false), 2000);
        } catch {
            navigator.clipboard.writeText(url).catch(() => { });
            setCopiedUrl(true);
            setTimeout(() => setCopiedUrl(false), 2000);
        }
    };

    const extPageUrl = {
        chrome: 'chrome://extensions',
        edge: 'edge://extensions',
        brave: 'brave://extensions',
        opera: 'opera://extensions',
        vivaldi: 'vivaldi://extensions',
    }[selectedBrowser] ?? 'chrome://extensions';

    const renderStep = (num: number, title: string, content: React.ReactNode) => (
        <div className="ext-step">
            <div className="ext-step-num">{num}</div>
            <div className="ext-step-body">
                <div className="ext-step-title">{title}</div>
                <div className="ext-step-content">{content}</div>
            </div>
        </div>
    );

    const handleInternalClose = async () => {
        if (onClose) {
            onClose();
        } else {
            try {
                await getCurrentWebviewWindow().close();
            } catch {
                window.close();
            }
        }
    };

    const content = (
        <div className="ext-installer-wrapper">
            {inline && (
                <div className="nd-titlebar" style={{ position: 'absolute', top: 0, left: 0, right: 0, borderTopLeftRadius: '0', borderTopRightRadius: '0' }}>
                    <span>Extension Setup</span>
                    <button className="nd-close-btn" onClick={handleInternalClose}>×</button>
                </div>
            )}
            <div className="ext-header" style={inline ? { marginTop: '20px' } : {}}>
                <div className="ext-icon-circle">🧩</div>
                <h1 className="ext-title">Browser Extension Setup</h1>
                <p className="ext-subtitle">Connect Velocity Downloader to your browser for one-click downloads.</p>
            </div>

            {status !== 'ready' && (
                <div className="ext-browser-selection">
                    <p className="ext-section-label">Select your primary browser:</p>
                    <div className="ext-browser-grid">
                        {BROWSERS.map(b => (
                            <button
                                key={b.id}
                                className={`ext-browser-card ${selectedBrowser === b.id ? 'active' : ''}`}
                                onClick={() => { setSelectedBrowser(b.id); setStatus('idle'); }}
                            >
                                <span className="ext-browser-icon">{b.icon}</span>
                                <span className="ext-browser-name">{b.label}</span>
                            </button>
                        ))}
                    </div>
                </div>
            )}

            {status === 'ready' && (
                <div className="ext-guide">
                    <div className="ext-status-banner">✅ Files prepared successfully!</div>
                    
                    {renderStep(1, "Open Extensions Page", (
                        <div className="ext-row">
                            <span className="ext-text-dim">Paste this in {selectedLabel} address bar:</span>
                            <div className="ext-copy-box">
                                <code>{extPageUrl}</code>
                                <button className="ext-mini-btn" onClick={() => handleCopyUrl(extPageUrl)}>
                                    {copiedUrl ? '✓' : '📋'}
                                </button>
                            </div>
                        </div>
                    ))}

                    {renderStep(2, "Enable Developer Mode", (
                        <p className="ext-text-dim">Turn on the <strong>Developer mode</strong> toggle switch at the top-right corner of the page.</p>
                    ))}

                    {renderStep(3, "Load Unpacked Folder", (
                        <div className="ext-column">
                            <p className="ext-text-dim">Click <strong>"Load unpacked"</strong> and select this folder:</p>
                            <div className="ext-copy-box full">
                                <code>{extPath}</code>
                                <button className="ext-copy-btn-large" onClick={handleCopyPath}>
                                    {copiedPath ? '✓ Path Copied' : '📋 Copy Path'}
                                </button>
                            </div>
                        </div>
                    ))}
                    
                    <div className="ext-pro-tip">
                        💡 <strong>Note:</strong> Browsers block automatic internal page opening for security. Please copy the link above manually if it didn't open.
                    </div>
                </div>
            )}

            {status === 'error' && (
                <div className="ext-error-box">
                    <span className="ext-error-icon">⚠️</span>
                    <div className="ext-error-text">
                        <strong>Installation Error</strong>
                        <p>{errorMsg}</p>
                    </div>
                </div>
            )}

            <div className="ext-footer">
                {status !== 'ready' ? (
                    <button 
                        className="ext-btn-primary" 
                        onClick={handlePrepare}
                        disabled={status === 'preparing'}
                    >
                        {status === 'preparing' ? 'Preparing...' : 'Next: Prepare Files'}
                    </button>
                ) : (
                    <button className="ext-btn-primary" onClick={handleInternalClose}>Finish Setup</button>
                )}
                <button className="ext-btn-ghost" onClick={handleInternalClose}>
                    {status === 'ready' ? 'Close' : 'Setup Later'}
                </button>
            </div>
        </div>
    );

    if (inline) {
        return <div className="ext-installer-container inline">{content}</div>;
    }

    return (
        <div className="ext-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
            <div className="ext-installer-container" id="ext-installer-modal">
                {content}
            </div>
        </div>
    );
}
