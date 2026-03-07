import { useState } from 'react';
import type { DownloadTask } from '../types';
import { formatBytes, formatSpeed, formatEta, getFileIcon } from '../utils';

interface DownloadItemProps {
    task: DownloadTask;
    onPause: (id: string) => void;
    onResume: (id: string) => void;
    onRemove: (id: string) => void;
    onOpenWindow: (id: string, filename: string) => void;
}

export function DownloadItem({ task, onPause, onResume, onRemove, onOpenWindow }: DownloadItemProps) {
    const [showSegments, setShowSegments] = useState(false);

    const progress = task.total_size > 0
        ? Math.min((task.downloaded / task.total_size) * 100, 100)
        : 0;

    const isActive = task.status === 'downloading' || task.status === 'assembling';
    const icon = getFileIcon(task.filename);

    return (
        <div className="download-item" id={`download-${task.id}`}>
            <div className="download-item-header">
                <div className="download-file-info">
                    <div className="file-type-icon">{icon}</div>
                    <div className="file-details">
                        <div className="file-name" title={task.filename}>{task.filename}</div>
                        <div className="file-meta">
                            <span>{formatBytes(task.total_size)}</span>
                            <span className="separator" />
                            <span>{task.num_segments} segment{task.num_segments !== 1 ? 's' : ''}</span>
                            <span className="separator" />
                            <StatusBadge status={task.status} />
                        </div>
                    </div>
                </div>

                <div className="download-actions">
                    {task.status === 'downloading' && (
                        <button
                            className="btn-icon"
                            onClick={() => onPause(task.id)}
                            title="Pause"
                            id={`pause-${task.id}`}
                        >
                            ⏸
                        </button>
                    )}
                    {(task.status === 'paused' || task.status === 'failed') && (
                        <button
                            className="btn-icon"
                            onClick={() => onResume(task.id)}
                            title="Resume"
                            id={`resume-${task.id}`}
                        >
                            ▶️
                        </button>
                    )}
                    <button
                        className="btn-icon"
                        onClick={() => onOpenWindow(task.id, task.filename)}
                        title="Open Details Window"
                        id={`details-${task.id}`}
                    >
                        📁
                    </button>
                    <button
                        className="btn-icon danger"
                        onClick={() => onRemove(task.id)}
                        title="Remove"
                        id={`remove-${task.id}`}
                    >
                        ✕
                    </button>
                </div>
            </div>

            {/* Progress Section */}
            <div className="progress-section">
                <div className="progress-bar-wrapper">
                    <div className="progress-bar-track">
                        <div
                            className="progress-bar-fill"
                            style={{
                                width: `${progress}%`,
                                animationPlayState: isActive ? 'running' : 'paused'
                            }}
                        />
                    </div>
                    <span className="progress-text">{progress.toFixed(1)}%</span>
                </div>
            </div>

            {/* Speed & ETA Stats */}
            {isActive && (
                <div className="download-stats">
                    <div className="stat">
                        <span className="label">Speed:</span>
                        <span className="value speed-value">{formatSpeed(task.speed_bps)}</span>
                    </div>
                    <div className="stat">
                        <span className="label">ETA:</span>
                        <span className="value eta-value">{formatEta(task.eta_seconds)}</span>
                    </div>
                    <div className="stat">
                        <span className="label">Downloaded:</span>
                        <span className="value">{formatBytes(task.downloaded)} / {formatBytes(task.total_size)}</span>
                    </div>
                </div>
            )}

            {task.error && (
                <div className="download-stats" style={{ color: 'var(--accent-danger)' }}>
                    ⚠️ {task.error}
                </div>
            )}

            {/* Segments Section */}
            {task.segments.length > 1 && (
                <div className="segments-section">
                    <button
                        className="segments-toggle"
                        onClick={() => setShowSegments(!showSegments)}
                        id={`toggle-segments-${task.id}`}
                    >
                        {showSegments ? '▾' : '▸'} {task.segments.length} Segments
                    </button>

                    {showSegments && (
                        <div className="segments-grid">
                            {task.segments.map((segment: any) => {
                                const segProgress = segment.progress !== undefined ? segment.progress : (segment.end && segment.start ? ((segment.bytes_downloaded || 0) / (segment.end - segment.start + 1)) * 100 : 0);
                                const segDownloaded = segment.downloaded !== undefined ? segment.downloaded : (segment.bytes_downloaded || 0);
                                const segTotal = segment.total_size !== undefined ? segment.total_size : (segment.end && segment.start ? (segment.end - segment.start + 1) : 0);
                                return (
                                <div className="segment-item" key={segment.id}>
                                    <div className="segment-header">
                                        <span className="segment-label">Segment {segment.id + 1}</span>
                                        <span className="segment-speed">
                                            {segment.status === 'downloading' && segment.speed_bps !== undefined ? formatSpeed(segment.speed_bps) : ''}
                                        </span>
                                    </div>
                                    <div className="segment-progress-track">
                                        <div
                                            className={`segment-progress-fill s${segment.id % 16}`}
                                            style={{ width: `${Math.min(segProgress, 100)}%` }}
                                        />
                                    </div>
                                    <div className="segment-percent">
                                        {Math.min(segProgress, 100).toFixed(1)}% · {formatBytes(segDownloaded)} / {formatBytes(segTotal)}
                                    </div>
                                </div>
                            )})}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}

function StatusBadge({ status }: { status: string }) {
    const labels: Record<string, string> = {
        queued: 'Queued',
        downloading: 'Downloading',
        paused: 'Paused',
        completed: 'Completed',
        failed: 'Failed',
        assembling: 'Assembling',
    };

    return (
        <span className={`status-badge ${status}`}>
            <span className="status-dot" />
            {labels[status] || status}
        </span>
    );
}
