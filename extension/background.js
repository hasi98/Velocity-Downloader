const LOCAL_API = "http://127.0.0.1:41420";
let isConnected = false;
let excludedSites = [];
const browserFallbackUrls = new Set();

function normalizeExcludedHost(hostname) {
    return (hostname || "").toLowerCase().replace(/^www\./, "");
}

// Load excluded sites from storage
chrome.storage.local.get(["excludedSites"], (result) => {
    if (result.excludedSites) {
        excludedSites = result.excludedSites.map(normalizeExcludedHost);
    }
});

chrome.storage.onChanged.addListener((changes, areaName) => {
    if (areaName !== "local" || !changes.excludedSites) return;
    excludedSites = (changes.excludedSites.newValue || []).map(normalizeExcludedHost);
});

async function refreshExcludedSites() {
    const result = await chrome.storage.local.get(["excludedSites"]);
    excludedSites = (result.excludedSites || []).map(normalizeExcludedHost);
    return excludedSites;
}

// Check connection to the desktop app
async function checkConnection() {
    try {
        const res = await fetch(`${LOCAL_API}/ping`);
        if (res.ok) {
            isConnected = true;
        } else {
            isConnected = false;
        }
    } catch (e) {
        isConnected = false;
    }
}

// Check every 5 seconds
setInterval(checkConnection, 5000);
checkConnection(); // Initial check

/**
 * Collect all cookies for a given URL and return them as a
 * "name=value; name2=value2" string suitable for a Cookie header.
 */
async function getCookieString(url) {
    try {
        const cookies = await chrome.cookies.getAll({ url });
        return cookies.map(c => `${c.name}=${c.value}`).join("; ");
    } catch (e) {
        console.warn("Could not collect cookies:", e);
        return "";
    }
}

function isLikelyMediaPageUrl(url) {
    try {
        const parsed = new URL(url);
        const host = parsed.hostname.toLowerCase();
        const path = parsed.pathname.toLowerCase();
        if (path.endsWith(".m3u8") || path.endsWith(".mpd")) return true;

        return [
            "youtube.com",
            "youtu.be",
            "vimeo.com",
            "dailymotion.com",
            "tiktok.com",
            "instagram.com",
            "facebook.com",
            "fb.watch",
            "x.com",
            "twitter.com",
            "twitch.tv",
            "soundcloud.com",
            "reddit.com",
            "streamable.com",
            "bilibili.com",
        ].some((domain) => host === domain || host.endsWith(`.${domain}`));
    } catch {
        return false;
    }
}

async function sendUrlToVelocity(url, referer = "") {
    if (isUrlExcluded(url) || isUrlExcluded(referer)) {
        throw new Error("VDM is disabled on this site");
    }

    await checkConnection();
    if (!isConnected) {
        throw new Error("Velocity Download Manager is not running");
    }

    const isMediaPage = isLikelyMediaPageUrl(url);
    const cookieStr = isMediaPage ? "" : await getCookieString(url);
    const userAgent = isMediaPage ? "" : navigator.userAgent;

    const res = await fetch(`${LOCAL_API}/add_download`, {
        method: "POST",
        headers: {
            "Content-Type": "application/json",
        },
        body: JSON.stringify({
            url,
            cookies: cookieStr || null,
            referer: isMediaPage ? null : (referer || null),
            user_agent: userAgent || null,
            source: "extension",
        })
    });

    if (!res.ok) {
        throw new Error(`Velocity server returned ${res.status}`);
    }

    const data = await res.json().catch(() => null);
    if (data && data.success === false) {
        throw new Error(data.message || "Velocity rejected the download");
    }

    return res;
}

function cancelBrowserDownload(downloadId) {
    chrome.downloads.cancel(downloadId, () => {
        const cancelError = chrome.runtime.lastError;
        if (cancelError) {
            console.warn("Could not cancel browser download:", cancelError.message);
        }

        chrome.downloads.erase({ id: downloadId }, () => {
            const eraseError = chrome.runtime.lastError;
            if (eraseError) {
                console.warn("Could not erase canceled browser download:", eraseError.message);
            }
        });
    });
}

function restoreBrowserDownload(url) {
    browserFallbackUrls.add(url);
    setTimeout(() => browserFallbackUrls.delete(url), 60000);

    chrome.downloads.download({ url, saveAs: true }, () => {
        const error = chrome.runtime.lastError;
        if (error) {
            console.error("Could not restore browser download:", error.message);
            browserFallbackUrls.delete(url);
        }
    });
}

function normalizeHost(hostname) {
    return hostname.toLowerCase().replace(/^\[(.*)\]$/, "$1");
}

function hostFromUrl(url) {
    try {
        const parsed = new URL(url);
        if (!["http:", "https:"].includes(parsed.protocol)) return "";
        return normalizeExcludedHost(parsed.hostname);
    } catch {
        return "";
    }
}

function isHostExcluded(hostname) {
    const host = normalizeExcludedHost(hostname);
    if (!host) return false;
    return excludedSites.some((excluded) => {
        const normalized = normalizeExcludedHost(excluded);
        return host === normalized || host.endsWith(`.${normalized}`);
    });
}

function isUrlExcluded(url) {
    return isHostExcluded(hostFromUrl(url));
}

async function getActivePageUrl() {
    try {
        const tabs = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
        return tabs && tabs[0] && tabs[0].url ? tabs[0].url : "";
    } catch {
        return "";
    }
}

async function isDownloadFromDisabledSite(targetUrl, referer) {
    await refreshExcludedSites();
    if (isUrlExcluded(targetUrl) || isUrlExcluded(referer)) {
        return true;
    }

    const activePageUrl = await getActivePageUrl();
    return isUrlExcluded(activePageUrl);
}

function isPrivateIpv4(hostname) {
    const parts = hostname.split(".");
    if (parts.length !== 4) return false;

    const nums = parts.map(part => Number(part));
    if (nums.some(num => !Number.isInteger(num) || num < 0 || num > 255)) {
        return false;
    }

    return nums[0] === 10 ||
        nums[0] === 127 ||
        nums[0] === 0 ||
        (nums[0] === 169 && nums[1] === 254) ||
        (nums[0] === 172 && nums[1] >= 16 && nums[1] <= 31) ||
        (nums[0] === 192 && nums[1] === 168);
}

function isLocalOrPrivateHost(hostname) {
    const host = normalizeHost(hostname);
    return host === "localhost" ||
        host === "::1" ||
        host.startsWith("fe80:") ||
        host.startsWith("fc") ||
        host.startsWith("fd") ||
        host.endsWith(".localhost") ||
        host.endsWith(".local") ||
        isPrivateIpv4(host);
}

function extensionFromUrlOrFilename(url, filename = "") {
    const fromFilename = filename.split(/[\\/]/).pop() || "";
    const filenameExt = fromFilename.includes(".")
        ? fromFilename.split(".").pop().toLowerCase()
        : "";
    if (filenameExt) return filenameExt;

    try {
        const path = new URL(url).pathname;
        const name = path.split("/").pop() || "";
        return name.includes(".") ? name.split(".").pop().toLowerCase() : "";
    } catch {
        return "";
    }
}

function isImageDownload(downloadItem, targetUrl) {
    const mime = (downloadItem.mime || "").toLowerCase();
    if (mime.startsWith("image/")) return true;

    const ext = extensionFromUrlOrFilename(targetUrl, downloadItem.filename || "");
    return ["png", "jpg", "jpeg", "webp", "gif", "svg", "ico", "bmp", "avif"].includes(ext);
}

chrome.runtime.onInstalled.addListener(() => {
    chrome.contextMenus.create({
        id: "download-with-velocity",
        title: "Download with VDM",
        contexts: ["link", "image", "video", "audio"]
    });
});

chrome.contextMenus.onClicked.addListener((info, tab) => {
    if (info.menuItemId !== "download-with-velocity") return;

    const targetUrl = info.linkUrl || info.srcUrl;
    if (!targetUrl) return;

    sendUrlToVelocity(targetUrl, info.pageUrl || tab?.url || "")
        .then(() => console.log("Context menu URL sent to Velocity:", targetUrl))
        .catch(err => console.error("Context menu send failed:", err));
});

async function handleDownloadCreated(downloadItem) {
    const targetUrl = downloadItem.finalUrl || downloadItem.url;

    if (!targetUrl || browserFallbackUrls.has(targetUrl)) {
        browserFallbackUrls.delete(targetUrl);
        return;
    }

    if (downloadItem.saveAs) {
        console.log("Browser Save As download ignored by VDM:", targetUrl);
        return;
    }

    if (isImageDownload(downloadItem, targetUrl)) {
        console.log("Image download ignored by VDM. Use the context menu to send it manually:", targetUrl);
        return;
    }

    // Check if the site is excluded
    let domain = "";
    try {
        const downloadUrl = new URL(targetUrl);
        if (!["http:", "https:"].includes(downloadUrl.protocol)) {
            return;
        }
        domain = downloadUrl.hostname;
    } catch (e) {
        console.warn("Could not parse download URL, allowing browser download:", targetUrl);
        return;
    }

    if (isLocalOrPrivateHost(domain)) {
        console.log("Local/private download URL ignored by VDM:", targetUrl);
        return;
    }

    const referer = downloadItem.referrer || "";
    if (await isDownloadFromDisabledSite(targetUrl, referer)) {
        console.log(`VDM is disabled for this site. Using browser download: ${targetUrl}`);
        return;
    }

    cancelBrowserDownload(downloadItem.id);
    console.log("Canceled browser download and sending to Velocity Download Manager:", targetUrl);

    sendUrlToVelocity(targetUrl, referer)
        .then(() => {
            console.log("Successfully sent to Velocity Download Manager!");
        })
        .catch((e) => {
            console.error("Failed to send download to Velocity Download Manager", e);
            restoreBrowserDownload(targetUrl);
        });
}

// Intercept downloads
chrome.downloads.onCreated.addListener((downloadItem) => {
    handleDownloadCreated(downloadItem).catch((error) => {
        console.error("VDM download interception failed:", error);
    });
});

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (message.action === "getStatus") {
        checkConnection().then(() => sendResponse({
            connected: isConnected,
            excludedSites: excludedSites
        }));
        return true;
    }

    if (message.action === "toggleExclusion") {
        const domain = normalizeExcludedHost(message.domain);
        const index = excludedSites.indexOf(domain);

        if (index > -1) {
            excludedSites.splice(index, 1);
            console.log("Removing domain from exclusion:", domain);
        } else {
            excludedSites.push(domain);
            console.log("Adding domain to exclusion:", domain);
        }

        chrome.storage.local.set({ excludedSites: excludedSites }, () => {
            console.log("Exclusion list saved:", excludedSites);
            sendResponse({ success: true, excluded: excludedSites.includes(domain), excludedSites });
        });
        return true;
    }

    if (message.action === "downloadMedia") {
        checkConnection().then(() => {
            if (!isConnected) {
                console.log("Not connected to app. Cannot download media.");
                sendResponse({ success: false, error: "Not connected to Velocity Download Manager" });
                return;
            }

            const mediaUrl = message.url;
            const referer = message.referer || "";

            sendUrlToVelocity(mediaUrl, referer).then(() => {
                console.log("Successfully sent media URL to Velocity Download Manager!");
                sendResponse({ success: true });
            }).catch(err => {
                console.error("Failed to send video download to Velocity Download Manager", err);
                sendResponse({ success: false, error: err.toString() });
            });
        });

        return true; // Keep message channel open for async response
    }
});
