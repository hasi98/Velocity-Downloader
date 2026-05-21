export type DownloadStatus = 
  | 'queued' 
  | 'downloading' 
  | 'paused' 
  | 'completed' 
  | 'failed' 
  | 'assembling';

export type SegmentStatus = 
  | 'pending' 
  | 'downloading' 
  | 'paused' 
  | 'completed' 
  | 'failed';

export interface SegmentProgress {
  id: number;
  downloaded: number;
  total_size: number;
  speed_bps: number;
  status: SegmentStatus;
  progress: number;
}

export interface DownloadTask {
  id: string;
  url: string;
  filename: string;
  save_path: string;
  total_size: number;
  downloaded: number;
  status: DownloadStatus;
  segments: SegmentProgress[];
  supports_range: boolean;
  num_segments: number;
  speed_bps: number;
  eta_seconds: number;
  created_at: string;
  updated_at: string;
  error: string | null;
  content_type: string | null;
  http_context: {
    cookies: string | null;
    referer: string | null;
    user_agent: string | null;
  };
  speed_limit_bps: number | null;
}

export interface ProgressEvent {
  download_id: string;
  total_size: number;
  downloaded: number;
  speed_bps: number;
  eta_seconds: number;
  status: DownloadStatus;
  segments: SegmentProgress[];
  speed_limit_bps: number | null;
}

export interface AppSettings {
  default_segments: number;
  default_download_dir: string;
  temp_download_dir: string | null;
  speed_limit_bps: number | null;
  start_on_boot: boolean;
}

export interface ProbeResult {
  size: number;
  supports_range: boolean;
  content_type: string | null;
  filename: string;
}

export interface Toast {
  id: string;
  type: 'success' | 'error' | 'info';
  message: string;
}

/** The payload sent by the extension via the show-add-download Tauri event */
export interface ShowAddDownloadPayload {
  url: string;
  cookies: string | null;
  referer: string | null;
  user_agent: string | null;
}

/** Browser HTTP context forwarded from the extension */
export interface HttpContext {
  cookies?: string | null;
  referer?: string | null;
  user_agent?: string | null;
}
