import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { DownloadWindow } from "./components/DownloadWindow";
import { SettingsPanel } from "./components/SettingsPanel";
import { AddDownloadWindow } from "./components/AddDownloadWindow";
import { ExtensionInstaller } from "./components/ExtensionInstaller";
import { BatchDownloadWindow } from "./components/BatchDownloadWindow";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

// Block right-click / inspect globally
document.addEventListener('contextmenu', e => e.preventDefault());
document.addEventListener('keydown', e => {
  if (e.key === 'F12') e.preventDefault();
  if (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'J' || e.key === 'C')) e.preventDefault();
  if (e.ctrlKey && e.key === 'U') e.preventDefault();
});

// Try to get params from search first, then fallback to hash if needed
const getParam = (name: string) => {
  const searchParams = new URLSearchParams(window.location.search);
  if (searchParams.has(name)) return searchParams.get(name);
  
  // Fallback for some routers/environments using hashes
  const hash = window.location.hash.split('?')[1];
  if (hash) {
    const hashParams = new URLSearchParams(hash);
    return hashParams.get(name);
  }
  return null;
};

const windowParam = getParam('window');

if (windowParam === 'add-download') {
  root.render(
    <React.StrictMode>
      <AddDownloadWindow />
    </React.StrictMode>
  );
} else if (windowParam === 'download') {
  const id = getParam('id');
  root.render(
    <React.StrictMode>
      <DownloadWindow id={id || ''} />
    </React.StrictMode>
  );
} else if (windowParam === 'settings') {
  root.render(
    <React.StrictMode>
      <div className="native-app" style={{ overflowY: 'auto' }}>
        <SettingsPanel />
      </div>
    </React.StrictMode>
  );
} else if (windowParam === 'batch-download') {
  root.render(
    <React.StrictMode>
      <BatchDownloadWindow />
    </React.StrictMode>
  );
} else if (windowParam === 'extensions') {
  root.render(
    <React.StrictMode>
      <div className="native-app" style={{ overflowY: 'auto', height: '100vh', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
        <ExtensionInstaller 
            inline={true} 
            onClose={() => {
                getCurrentWebviewWindow().close().catch(e => {
                    console.error('Failed to close window:', e);
                    window.close(); // Fallback
                });
            }} 
        />
      </div>
    </React.StrictMode>
  );
} else {
  root.render(
    <React.StrictMode>
      <App />
    </React.StrictMode>
  );
}
