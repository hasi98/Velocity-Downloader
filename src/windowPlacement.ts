import { WebviewWindow } from '@tauri-apps/api/webviewWindow';
import { getCurrentWindow, Window as TauriWindow } from '@tauri-apps/api/window';
import { PhysicalPosition } from '@tauri-apps/api/dpi';

const MAIN_WINDOW_LABEL = 'main';

type WebviewWindowOptions = NonNullable<ConstructorParameters<typeof WebviewWindow>[1]>;

async function getMainWindow() {
  return (await TauriWindow.getByLabel(MAIN_WINDOW_LABEL).catch(() => null)) ?? getCurrentWindow();
}

export async function showWindowInFrontOfMain(webview: WebviewWindow) {
  try {
    const mainWindow = await getMainWindow();
    await mainWindow.unminimize().catch(() => {});
    await mainWindow.show().catch(() => {});

    const [mainPosition, mainSize, childSize] = await Promise.all([
      mainWindow.outerPosition(),
      mainWindow.outerSize(),
      webview.outerSize(),
    ]);

    const x = Math.round(mainPosition.x + (mainSize.width - childSize.width) / 2);
    const y = Math.round(mainPosition.y + (mainSize.height - childSize.height) / 2);
    await webview.setPosition(new PhysicalPosition(x, y));
  } catch (error) {
    console.warn('Failed to position child window over main window:', error);
  }

  await webview.show().catch(() => {});
  await webview.setFocus().catch(() => {});
}

export function openChildWindow(
  label: string,
  options: WebviewWindowOptions,
  onCreateError?: (event: unknown) => void
) {
  const webview = new WebviewWindow(label, {
    ...options,
    parent: MAIN_WINDOW_LABEL,
    center: false,
    visible: false,
    focus: false,
    preventOverflow: true,
  });

  webview.once('tauri://created', () => {
    showWindowInFrontOfMain(webview).catch(console.error);
  });

  webview.once('tauri://error', async (event) => {
    const existing = await WebviewWindow.getByLabel(label).catch(() => null);
    if (existing) {
      await showWindowInFrontOfMain(existing);
      return;
    }
    onCreateError?.(event);
  });

  return webview;
}
