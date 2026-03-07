const LOCAL_API = "http://127.0.0.1:41420";
let isConnected = false;
let excludedSites = [];

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

// Intercept downloads
chrome.downloads.onCreated.addListener(async (downloadItem) => {
    // Prevent infinite loops if our app initiates downloads that Chrome sees
    if (!isConnected) {
        console.log("Not connected to Velocity Downloader, allowing Chrome to download.");
        return;
    }

    // Check if the site is excluded
    const downloadUrl = new URL(downloadItem.url);
    const domain = downloadUrl.hostname;

    if (excludedSites.includes(domain)) {
        console.log(`Site ${domain} is in exclusion list. Using browser download.`);
        return;
    }

    chrome.downloads.pause(downloadItem.id, async () => {
        console.log("Paused download to send to Velocity Downloader:", downloadItem.url);

        // Gather cookies, referer, and user-agent
        const cookieStr = await getCookieString(downloadItem.url);
        const referer = downloadItem.referrer || "";
        const userAgent = navigator.userAgent;

        try {
            const res = await fetch(`${LOCAL_API}/add_download`, {
                method: "POST",
                headers: {
                    "Content-Type": "application/json",
                },
                body: JSON.stringify({
                    url: downloadItem.url,
                    cookies: cookieStr || null,
                    referer: referer || null,
                    user_agent: userAgent || null,
                })
            });

            if (res.ok) {
                console.log("Successfully sent to Velocity Downloader!");
                chrome.downloads.cancel(downloadItem.id);
                chrome.downloads.erase({ id: downloadItem.id });
            } else {
                console.error("Failed to send, resuming in Chrome.");
                chrome.downloads.resume(downloadItem.id);
            }
        } catch (e) {
            console.error("Failed to send download to Velocity Downloader", e);
            chrome.downloads.resume(downloadItem.id);
        }
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

            // Collect cookies + context for the media URL, then send to IDM
            const mediaUrl = message.url;
            const referer = message.referer || "";
            const userAgent = navigator.userAgent;

            getCookieString(mediaUrl).then(cookieStr => {
                return fetch(`${LOCAL_API}/add_download`, {
                    method: "POST",
                    headers: {
                        "Content-Type": "application/json",
                    },
                    body: JSON.stringify({
                        url: mediaUrl,
                        cookies: cookieStr || null,
                        referer: referer || null,
                        user_agent: userAgent || null,
                    })
                });
            }).then(res => {
                if (res.ok) {
                    console.log("Successfully sent media URL to Velocity Downloader!");
                    sendResponse({ success: true });
                } else {
                    console.error("Failed to send media URL to Velocity Downloader.");
                    sendResponse({ success: false, error: "Server error" });
                }
            }).catch(err => {
                console.error("Failed to send video download to Velocity Downloader", err);
                sendResponse({ success: false, error: err.toString() });
            });
        });

        return true; // Keep message channel open for async response
    }
});
