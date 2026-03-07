/** Format bytes to human-readable string */
export function formatBytes(bytes: number, decimals = 2): string {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

/** Format bytes per second to speed string */
export function formatSpeed(bytesPerSecond: number): string {
    if (bytesPerSecond <= 0) return '0 B/s';
    const k = 1024;
    const sizes = ['B/s', 'KB/s', 'MB/s', 'GB/s'];
    const i = Math.floor(Math.log(bytesPerSecond) / Math.log(k));
    return parseFloat((bytesPerSecond / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
}

/** Format seconds to human-readable ETA */
export function formatEta(seconds: number): string {
    if (seconds <= 0 || !isFinite(seconds)) return '—';

    const hrs = Math.floor(seconds / 3600);
    const mins = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);

    if (hrs > 0) {
        return `${hrs}h ${mins}m ${secs}s`;
    } else if (mins > 0) {
        return `${mins}m ${secs}s`;
    } else {
        return `${secs}s`;
    }
}

/** Get file extension from filename */
export function getFileExtension(filename: string): string {
    const parts = filename.split('.');
    return parts.length > 1 ? parts[parts.length - 1].toLowerCase() : '';
}

/** Get file type icon based on extension */
export function getFileIcon(filename: string): string {
    const ext = getFileExtension(filename);
    const iconMap: Record<string, string> = {
        // Video
        mp4: '🎬', mkv: '🎬', avi: '🎬', mov: '🎬', wmv: '🎬', flv: '🎬', webm: '🎬',
        // Audio
        mp3: '🎵', wav: '🎵', flac: '🎵', aac: '🎵', ogg: '🎵', m4a: '🎵',
        // Image
        jpg: '🖼️', jpeg: '🖼️', png: '🖼️', gif: '🖼️', svg: '🖼️', webp: '🖼️', bmp: '🖼️',
        // Archive
        zip: '📦', rar: '📦', '7z': '📦', tar: '📦', gz: '📦',
        // Document
        pdf: '📄', doc: '📄', docx: '📄', xls: '📄', xlsx: '📄', ppt: '📄', pptx: '📄', txt: '📄',
        // Code
        js: '💻', ts: '💻', py: '💻', rs: '💻', cpp: '💻', java: '💻',
        // Executable
        exe: '⚙️', msi: '⚙️', dmg: '⚙️', deb: '⚙️', rpm: '⚙️',
        // ISO
        iso: '💿',
    };
    return iconMap[ext] || '📄';
}

/** Get file category based on extension */
export function getFileCategory(filename: string): string {
    const ext = getFileExtension(filename);
    const categories: Record<string, string[]> = {
        'Video': ['mp4', 'mkv', 'avi', 'mov', 'wmv', 'flv', 'webm', 'ts', 'm3u8'],
        'Music': ['mp3', 'wav', 'flac', 'aac', 'ogg', 'm4a'],
        'Compressed': ['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'iso'],
        'Programs': ['exe', 'msi', 'apk', 'dmg', 'pkg', 'deb', 'rpm', 'appimage'],
        'Documents': ['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'csv']
    };
    
    for (const [category, exts] of Object.entries(categories)) {
        if (exts.includes(ext)) {
            return category;
        }
    }
    return 'General';
}

/** Generate categorized path */
export function getCategorizedPath(basePath: string, category: string): string {
    if (category === 'General' || !basePath) return basePath;
    const separator = basePath.includes('\\') ? '\\' : '/';
    // Remove trailing slash if exists
    let cleanBase = basePath;
    if (cleanBase.endsWith(separator)) cleanBase = cleanBase.slice(0, -1);
    
    // Check if it already ends with the category folder
    if (cleanBase.endsWith(`${separator}${category}`)) {
        return cleanBase;
    }
    
    return `${cleanBase}${separator}${category}`;
}

/** Generate a unique ID */
export function generateId(): string {
    return Date.now().toString(36) + Math.random().toString(36).substr(2);
}
