(function () {
    let currentMedia = null;
    let panel = null;
    let hideTimer = null;
    const dismissedMedia = new WeakSet();

    function createPanel() {
        if (panel) return;

        panel = document.createElement("div");
        panel.className = "vdm-download-panel";

        const downloadButton = document.createElement("button");
        downloadButton.className = "vdm-download-button";
        downloadButton.type = "button";
        downloadButton.innerHTML = '<span class="vdm-icon">↓</span><span>Download with VDM</span>';
        downloadButton.addEventListener("click", handleDownloadClick);

        const closeButton = document.createElement("button");
        closeButton.className = "vdm-close-button";
        closeButton.type = "button";
        closeButton.title = "Hide";
        closeButton.textContent = "×";
        closeButton.addEventListener("click", (event) => {
            event.preventDefault();
            event.stopPropagation();
            if (currentMedia) dismissedMedia.add(currentMedia);
            hidePanel();
        });

        panel.appendChild(downloadButton);
        panel.appendChild(closeButton);
        document.documentElement.appendChild(panel);

        panel.addEventListener("mouseenter", () => {
            clearTimeout(hideTimer);
            panel.classList.add("visible");
        });
        panel.addEventListener("mouseleave", hidePanelSoon);
    }

    function handleDownloadClick(event) {
        event.preventDefault();
        event.stopPropagation();

        if (!currentMedia) return;

        const pageUrl = window.location.href;
        const mediaUrl = getMediaSource(currentMedia);
        const targetUrl = chooseDownloadUrl(mediaUrl, pageUrl);

        if (!targetUrl) {
            setButtonText("No media URL", 1600);
            return;
        }

        chrome.runtime.sendMessage(
            {
                action: "downloadMedia",
                url: targetUrl,
                referer: pageUrl,
                mediaUrl: mediaUrl || null,
                title: document.title || null,
            },
            (response) => {
                if (chrome.runtime.lastError) {
                    console.error("VDM extension error:", chrome.runtime.lastError.message);
                    setButtonText("VDM not available", 2200);
                    return;
                }
                if (response && response.success) {
                    setButtonText("Added to VDM", 1800);
                } else {
                    setButtonText("Send failed", 2200);
                    console.error("VDM media send failed:", response && response.error);
                }
            }
        );
    }

    function setButtonText(text, duration) {
        if (!panel) return;
        const button = panel.querySelector(".vdm-download-button");
        if (!button) return;

        const original = button.innerHTML;
        button.textContent = text;
        window.setTimeout(() => {
            if (button.isConnected) button.innerHTML = original;
        }, duration);
    }

    function getMediaSource(mediaElement) {
        let src = mediaElement.currentSrc || mediaElement.src || "";
        if (src) return src;

        const source = mediaElement.querySelector("source[src]");
        return source ? source.src : "";
    }

    function isLikelyMediaPage(url) {
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

    function chooseDownloadUrl(mediaUrl, pageUrl) {
        if (isLikelyMediaPage(pageUrl)) return pageUrl;
        if (!mediaUrl || mediaUrl.startsWith("blob:")) return pageUrl;
        return mediaUrl;
    }

    function showPanel(mediaElement) {
        if (dismissedMedia.has(mediaElement)) return;

        const rect = mediaElement.getBoundingClientRect();
        if (rect.width < 140 || rect.height < 80) return;

        currentMedia = mediaElement;
        createPanel();

        const panelWidth = panel.offsetWidth || 166;
        const top = Math.max(8, window.scrollY + rect.top + 10);
        const left = Math.max(8, window.scrollX + rect.right - panelWidth - 10);

        panel.style.top = `${top}px`;
        panel.style.left = `${left}px`;
        clearTimeout(hideTimer);
        panel.classList.add("visible");
    }

    function hidePanelSoon() {
        clearTimeout(hideTimer);
        hideTimer = window.setTimeout(hidePanel, 500);
    }

    function hidePanel() {
        clearTimeout(hideTimer);
        if (panel) panel.classList.remove("visible");
        currentMedia = null;
    }

    function attachMedia(mediaElement) {
        if (mediaElement.dataset.vdmAttached === "true") return;
        mediaElement.dataset.vdmAttached = "true";

        mediaElement.addEventListener("mouseenter", () => showPanel(mediaElement));
        mediaElement.addEventListener("mousemove", () => showPanel(mediaElement));
        mediaElement.addEventListener("mouseleave", hidePanelSoon);
    }

    function scan() {
        document.querySelectorAll("video, audio").forEach(attachMedia);
    }

    createPanel();
    scan();

    const observer = new MutationObserver(scan);
    observer.observe(document.documentElement, { childList: true, subtree: true });

    window.addEventListener("scroll", () => {
        if (currentMedia && panel && panel.classList.contains("visible")) {
            showPanel(currentMedia);
        }
    }, true);
})();
