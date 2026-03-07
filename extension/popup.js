const statusEl = document.getElementById("status");
const checkBtn = document.getElementById("checkBtn");
const excludeBtn = document.getElementById("excludeBtn");
const siteLabel = document.getElementById("currentSite");

let currentDomain = "";

// Function to update the exclusion button text and style
function updateExclusionButton(excludedSites) {
    if (!currentDomain) return;

    if (excludedSites) {
        const isExcluded = excludedSites.includes(currentDomain);
        excludeBtn.textContent = isExcluded ? "✅ Enable IDM" : "❌ Disable IDM";
        excludeBtn.style.background = isExcluded ? "#059669" : "#1e293b";
        excludeBtn.style.display = "block";
    } else {
        // Fetch if not provided
        chrome.storage.local.get(["excludedSites"], (result) => {
            const list = result.excludedSites || [];
            const isExcluded = list.includes(currentDomain);
            excludeBtn.textContent = isExcluded ? "✅ Enable IDM" : "❌ Disable IDM";
            excludeBtn.style.background = isExcluded ? "#059669" : "#1e293b";
            excludeBtn.style.display = "block";
        });
    }
}

// Get current tab domain
chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
    if (tabs[0] && tabs[0].url) {
        try {
            const url = new URL(tabs[0].url);
            if (url.protocol.startsWith('http')) {
                currentDomain = url.hostname;
                siteLabel.textContent = currentDomain;
                updateExclusionButton();
            } else {
                siteLabel.textContent = "Browser Page";
                excludeBtn.style.display = "none";
            }
        } catch (e) {
            siteLabel.textContent = "Invalid Page";
            excludeBtn.style.display = "none";
        }
    }
});

function checkStatus() {
    statusEl.textContent = "Checking...";
    statusEl.className = "";

    // Update button immediately from local storage for speed
    updateExclusionButton();

    chrome.runtime.sendMessage({ action: "getStatus" }, (response) => {
        if (response && response.connected) {
            statusEl.textContent = "✅ Connected to App";
            statusEl.className = "connected";
        } else {
            statusEl.textContent = "❌ Cannot find App";
            statusEl.className = "disconnected";
        }
        if (response && response.excludedSites) {
            updateExclusionButton(response.excludedSites);
        }
    });
}

excludeBtn.addEventListener("click", () => {
    if (!currentDomain) return;

    chrome.runtime.sendMessage({
        action: "toggleExclusion",
        domain: currentDomain
    }, (response) => {
        if (response && response.success) {
            // Background script sends full list back in getStatus, 
            // but we can just toggle locally for instant UI update
            const isNowExcluded = excludeBtn.textContent.includes("Disable");
            excludeBtn.textContent = isNowExcluded ? "✅ Enable IDM" : "❌ Disable IDM";
            excludeBtn.style.background = isNowExcluded ? "#059669" : "#1e293b";
        }
    });
});

checkBtn.addEventListener("click", checkStatus);
checkStatus(); // Initial check
