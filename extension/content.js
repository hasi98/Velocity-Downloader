(function () {
    // Keeps track of the currently hovered media element
    let currentMedia = null;
    let buttonWrapper = null;
    let hideTimeout = null;

    function createDownloadButton() {
        if (buttonWrapper) return;

        buttonWrapper = document.createElement("div");
        buttonWrapper.className = "myidm-download-btn-wrapper";

        const btn = document.createElement("button");
        btn.className = "myidm-download-btn";
        btn.innerHTML = `<span class="myidm-icon">⚡</span> Download with IDM`;

        btn.addEventListener("click", handleDownloadClick);
        buttonWrapper.appendChild(btn);
        document.body.appendChild(buttonWrapper);

        // Keep visible if hovering over the button itself
        buttonWrapper.addEventListener("mouseenter", () => {
            clearTimeout(hideTimeout);
            buttonWrapper.classList.add("visible");
        });

        buttonWrapper.addEventListener("mouseleave", () => {
            hideButtonWithDelay();
        });
    }

    function handleDownloadClick(e) {
        e.preventDefault();
        e.stopPropagation();

        if (!currentMedia) return;

        // Try to get the source
        let src = currentMedia.currentSrc || currentMedia.src;
        if (!src) {
            // Check for <source> elements inside
            const sources = currentMedia.querySelectorAll("source");
            for (let source of sources) {
                if (source.src) {
                    src = source.src;
                    break;
                }
            }
        }

        if (!src) {
            alert("No recognizable media source found to download.");
            return;
        }

        if (src.startsWith("blob:")) {
            alert("Blob URLs cannot be downloaded directly yet. (This is a protected stream).");
            return;
        }

        // Pass the current page URL as referer so the background script can attach it
        const referer = window.location.href;

        // We have a URL, send to background script
        try {
            chrome.runtime.sendMessage({ action: "downloadMedia", url: src, referer }, (response) => {
                if (chrome.runtime.lastError) {
                    console.error("Extension background error:", chrome.runtime.lastError.message);
                    alert("Extension connection error. Make sure My IDM Desktop is running and try refreshing the page. Details: " + chrome.runtime.lastError.message);
                    return;
                }
                if (response && response.success) {
                    // Temporarily show success on button
                    if (buttonWrapper) {
                        const btn = buttonWrapper.querySelector('button');
                        if (btn) {
                            const originalText = btn.innerHTML;
                            btn.innerHTML = `✅ Added to IDM`;
                            setTimeout(() => {
                                if (buttonWrapper && buttonWrapper.querySelector('button') === btn) {
                                    btn.innerHTML = originalText;
                                }
                            }, 2000);
                        }
                    }
                } else {
                    alert("Failed to send download to My IDM: " + (response ? response.error : "Unknown Error"));
                }
            });
        } catch (e) {
            console.error("SendMessage Error:", e);
            alert("The extension was reloaded or connection was lost. Please refresh this webpage and try again! Details: " + e.message);
        }
    }

    function showButtonOverMedia(mediaElement) {
        currentMedia = mediaElement;

        // Ensure button exists
        createDownloadButton();

        // Calculate position
        const rect = mediaElement.getBoundingClientRect();

        // Don't show if hidden or extremely small
        if (rect.width < 100 || rect.height < 50) return;

        // Position it at the top right of the media player
        buttonWrapper.style.top = `${window.scrollY + rect.top + 10}px`;
        buttonWrapper.style.left = `${window.scrollX + rect.right - buttonWrapper.offsetWidth - 10}px`;

        clearTimeout(hideTimeout);
        buttonWrapper.classList.add("visible");
    }

    function hideButtonWithDelay() {
        hideTimeout = setTimeout(() => {
            if (buttonWrapper) {
                buttonWrapper.classList.remove("visible");
                currentMedia = null;
            }
        }, 500); // Wait half a second before hiding
    }

    function attachMediaListeners(mediaElement) {
        // Only attach once
        if (mediaElement.dataset.myidmAttached) return;
        mediaElement.dataset.myidmAttached = "true";

        mediaElement.addEventListener("mouseenter", () => {
            showButtonOverMedia(mediaElement);
        });

        mediaElement.addEventListener("mouseleave", () => {
            hideButtonWithDelay();
        });
    }

    function scanForMedia() {
        // Find videos and audio elements
        const mediaElements = document.querySelectorAll("video, audio");
        mediaElements.forEach(attachMediaListeners);
    }

    // Initial scan
    createDownloadButton();
    scanForMedia();

    // Use MutationObserver to catch dynamically added video elements (e.g. YouTube, Twitter)
    const observer = new MutationObserver((mutations) => {
        let hasNewMedia = false;
        for (const mutation of mutations) {
            if (mutation.addedNodes.length > 0) {
                for (const node of mutation.addedNodes) {
                    if (node.nodeType === 1) { // Element node
                        if (node.tagName === "VIDEO" || node.tagName === "AUDIO") {
                            attachMediaListeners(node);
                        } else if (node.querySelectorAll) {
                            const children = node.querySelectorAll("video, audio");
                            if (children.length > 0) {
                                children.forEach(attachMediaListeners);
                            }
                        }
                    }
                }
            }
        }
    });

    observer.observe(document.body, { childList: true, subtree: true });

})();
