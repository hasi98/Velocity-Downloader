const LOCAL_API = "http://127.0.0.1:41420";
let isConnected = false;
let excludedSites = [];
const browserFallbackUrls = new Set();

// Load excluded sites from storage
chrome.storage.local.get(["excludedSites"], (result) => {
    if (result.excludedSites) {
        excludedSites = result.excludedSites;
    }
});

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

async function sendUrlToVelocity(url, referer = "") {
    await checkConnection();
    if (!isConnected) {
        throw new Error("Velocity Downloader is not running");
    }

    const cookieStr = await getCookieString(url);
    const userAgent = navigator.userAgent;

    const res = await fetch(`${LOCAL_API}/add_download`, {
        method: "POST",
        headers: {
            "Content-Type": "application/json",
        },
        body: JSON.stringify({
            url,
            cookies: cookieStr || null,
            referer: referer || null,
            user_agent: userAgent || null,
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

chrome.runtime.onInstalled.addListener(() => {
    chrome.contextMenus.create({
        id: "download-with-velocity",
        title: "Download with Velocity",
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

// Intercept downloads
chrome.downloads.onCreated.addListener((downloadItem) => {
    const targetUrl = downloadItem.finalUrl || downloadItem.url;

    if (!targetUrl || browserFallbackUrls.has(targetUrl)) {
        browserFallbackUrls.delete(targetUrl);
        return;
    }

    // Prevent infinite loops if our app initiates downloads that Chrome sees
    if (!isConnected) {
        console.log("Not connected to Velocity Downloader, allowing Chrome to download.");
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
        console.log("Local/private download URL ignored by Velocity:", targetUrl);
        return;
    }

    if (excludedSites.includes(domain)) {
        console.log(`Site ${domain} is in exclusion list. Using browser download.`);
        return;
    }

    const referer = downloadItem.referrer || "";
    cancelBrowserDownload(downloadItem.id);
    console.log("Canceled browser download and sending to Velocity Downloader:", targetUrl);

    sendUrlToVelocity(targetUrl, referer)
        .then(() => {
            console.log("Successfully sent to Velocity Downloader!");
        })
        .catch((e) => {
            console.error("Failed to send download to Velocity Downloader", e);
            restoreBrowserDownload(targetUrl);
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
        const domain = message.domain;
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
            sendResponse({ success: true, excluded: excludedSites.includes(domain) });
        });
        return true;
    }

    if (message.action === "downloadMedia") {
        checkConnection().then(() => {
            if (!isConnected) {
                console.log("Not connected to app. Cannot download media.");
                sendResponse({ success: false, error: "Not connected to Velocity Downloader App" });
                return;
            }

            const mediaUrl = message.url;
            const referer = message.referer || "";

            sendUrlToVelocity(mediaUrl, referer).then(() => {
                console.log("Successfully sent media URL to Velocity Downloader!");
                sendResponse({ success: true });
            }).catch(err => {
                console.error("Failed to send video download to Velocity Downloader", err);
                sendResponse({ success: false, error: err.toString() });
            });
        });

        return true; // Keep message channel open for async response
    }
});
