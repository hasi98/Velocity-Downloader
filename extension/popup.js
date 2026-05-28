const statusEl = document.getElementById("status");
const checkBtn = document.getElementById("checkBtn");
const excludeBtn = document.getElementById("excludeBtn");
const siteLabel = document.getElementById("currentSite");

let currentDomain = "";

function updateExclusionButton(excludedSites) {
    if (!currentDomain) return;

    const apply = (list) => {
        const isExcluded = list.includes(currentDomain);
        excludeBtn.textContent = isExcluded ? "Enable VDM on this site" : "Disable VDM on this site";
        excludeBtn.style.background = isExcluded ? "#0d6b42" : "#26282b";
        excludeBtn.style.display = "block";
    };

    if (excludedSites) {
        apply(excludedSites);
        return;
    }

    chrome.storage.local.get(["excludedSites"], (result) => {
        apply(result.excludedSites || []);
    });
}

chrome.tabs.query({ active: true, currentWindow: true }, (tabs) => {
    if (!tabs[0] || !tabs[0].url) return;

    try {
        const url = new URL(tabs[0].url);
        if (url.protocol.startsWith("http")) {
            currentDomain = url.hostname;
            siteLabel.textContent = currentDomain;
            updateExclusionButton();
        } else {
            siteLabel.textContent = "Browser page";
            excludeBtn.style.display = "none";
        }
    } catch {
        siteLabel.textContent = "Invalid page";
        excludeBtn.style.display = "none";
    }
});

function checkStatus() {
    statusEl.textContent = "Checking...";
    statusEl.className = "";
    updateExclusionButton();

    chrome.runtime.sendMessage({ action: "getStatus" }, (response) => {
        if (response && response.connected) {
            statusEl.textContent = "Connected to VDM";
            statusEl.className = "connected";
        } else {
            statusEl.textContent = "Cannot find VDM";
            statusEl.className = "disconnected";
        }

        if (response && response.excludedSites) {
            updateExclusionButton(response.excludedSites);
        }
    });
}

excludeBtn.addEventListener("click", () => {
    if (!currentDomain) return;

    chrome.runtime.sendMessage({ action: "toggleExclusion", domain: currentDomain }, (response) => {
        if (response && response.success) {
            updateExclusionButton(response.excludedSites);
        } else {
            checkStatus();
        }
    });
});

checkBtn.addEventListener("click", checkStatus);
checkStatus();
