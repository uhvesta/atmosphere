/* Humata 2 Native Browser Co-Pilot — Shared Content Script
 *
 * - Extracts structured accessibility/page snapshots (`read_tab`)
 * - Executes approved DOM interactions (`click`, `type`, `submit`)
 * - Renders a clean, Shadow-DOM-isolated, dismissible highlight + callout
 *   bubble (`highlight_guide`) so agents can visually guide the user.
 */

(() => {
  if (window.__humata2BrowserCopilotLoaded) {
    return;
  }
  window.__humata2BrowserCopilotLoaded = true;

  const api = typeof browser !== "undefined" ? browser : chrome;
  const OVERLAY_HOST_ID = "humata2-copilot-guide-host";
  const DEFAULT_MAX_CHARS = 16000;
  const MAX_INTERACTIVE_ELEMENTS = 150;

  let activeCleanup = null;

  function cssEscapeIdent(value) {
    if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
      return CSS.escape(value);
    }
    return String(value).replace(/([^\w-])/g, "\\$1");
  }

  function buildSelector(el) {
    if (el.id) {
      return `#${cssEscapeIdent(el.id)}`;
    }
    const testId = el.getAttribute("data-testid") || el.getAttribute("data-test-id");
    if (testId) {
      return `${el.tagName.toLowerCase()}[data-testid="${testId.replace(/"/g, '\\"')}"]`;
    }
    const nameAttr = el.getAttribute("name");
    if (nameAttr) {
      return `${el.tagName.toLowerCase()}[name="${nameAttr.replace(/"/g, '\\"')}"]`;
    }
    const parts = [];
    let current = el;
    while (current && current.nodeType === Node.ELEMENT_NODE && parts.length < 5) {
      let segment = current.tagName.toLowerCase();
      if (current.id) {
        parts.unshift(`#${cssEscapeIdent(current.id)}`);
        break;
      }
      const parent = current.parentElement;
      if (parent) {
        const sameTagSiblings = Array.from(parent.children).filter(
          (child) => child.tagName === current.tagName
        );
        if (sameTagSiblings.length > 1) {
          const index = sameTagSiblings.indexOf(current) + 1;
          segment += `:nth-of-type(${index})`;
        }
      }
      parts.unshift(segment);
      current = parent;
    }
    return parts.join(" > ");
  }

  function isElementVisible(el) {
    const style = window.getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden" || style.opacity === "0") {
      return false;
    }
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }

  function extractAccessibleLabel(el) {
    const ariaLabel = el.getAttribute("aria-label");
    if (ariaLabel && ariaLabel.trim()) {
      return ariaLabel.trim();
    }
    const labelledBy = el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const refText = labelledBy
        .split(/\s+/)
        .map((id) => document.getElementById(id))
        .filter(Boolean)
        .map((node) => (node.textContent || "").trim())
        .join(" ")
        .trim();
      if (refText) {
        return refText;
      }
    }
    if (el.id) {
      const labelEl = document.querySelector(`label[for="${cssEscapeIdent(el.id)}"]`);
      if (labelEl && labelEl.textContent && labelEl.textContent.trim()) {
        return labelEl.textContent.trim();
      }
    }
    const placeholder = el.getAttribute("placeholder");
    if (placeholder && placeholder.trim()) {
      return placeholder.trim();
    }
    const title = el.getAttribute("title");
    if (title && title.trim()) {
      return title.trim();
    }
    const text = (el.innerText || el.textContent || "").replace(/\s+/g, " ").trim();
    return text.slice(0, 120);
  }

  function extractPageSnapshot(maxChars) {
    const limit = typeof maxChars === "number" && maxChars > 0 ? maxChars : DEFAULT_MAX_CHARS;
    const rawText = (document.body && document.body.innerText) || "";
    const visibleText = rawText.replace(/\n{3,}/g, "\n\n").trim().slice(0, limit);

    const interactiveQuery = [
      "a[href]",
      "button",
      "input",
      "textarea",
      "select",
      '[role="button"]',
      '[role="link"]',
      '[role="menuitem"]',
      '[role="tab"]',
      '[contenteditable="true"]',
    ].join(",");

    const interactiveElements = [];
    const nodes = document.querySelectorAll(interactiveQuery);
    for (const el of nodes) {
      if (interactiveElements.length >= MAX_INTERACTIVE_ELEMENTS) {
        break;
      }
      if (!isElementVisible(el)) {
        continue;
      }
      const tag = el.tagName.toLowerCase();
      interactiveElements.push({
        selector: buildSelector(el),
        tag,
        role: el.getAttribute("role") || null,
        label: extractAccessibleLabel(el),
        input_type: tag === "input" ? el.getAttribute("type") || "text" : null,
        disabled: Boolean(el.disabled || el.getAttribute("aria-disabled") === "true"),
      });
    }

    return {
      tab_id: 0,
      url: window.location.href,
      title: document.title || "",
      visible_text: visibleText,
      interactive_elements: interactiveElements,
    };
  }

  function findTargetElement(selector) {
    if (typeof selector !== "string" || selector.trim() === "") {
      throw new Error("Selector must be a non-empty string");
    }
    const el = document.querySelector(selector);
    if (!el) {
      throw new Error(`Element not found for selector: ${selector}`);
    }
    return el;
  }

  function performClick(selector) {
    const el = findTargetElement(selector);
    el.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
    if (typeof el.focus === "function") {
      el.focus();
    }
    el.click();
    return {
      type: "action_ack",
      tab_id: 0,
      url: window.location.href,
      detail: `Clicked ${selector}`,
    };
  }

  function setNativeInputValue(el, nextValue) {
    const proto =
      el.tagName === "TEXTAREA"
        ? window.HTMLTextAreaElement.prototype
        : window.HTMLInputElement.prototype;
    const descriptor = Object.getOwnPropertyDescriptor(proto, "value");
    if (descriptor && typeof descriptor.set === "function") {
      descriptor.set.call(el, nextValue);
    } else {
      el.value = nextValue;
    }
  }

  function performType(selector, text, clearFirst) {
    const el = findTargetElement(selector);
    el.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
    if (typeof el.focus === "function") {
      el.focus();
    }

    if (el.isContentEditable) {
      el.textContent = clearFirst ? text : `${el.textContent || ""}${text}`;
    } else {
      const current = typeof el.value === "string" ? el.value : "";
      const nextValue = clearFirst ? text : `${current}${text}`;
      setNativeInputValue(el, nextValue);
    }

    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));

    return {
      type: "action_ack",
      tab_id: 0,
      url: window.location.href,
      detail: `Typed ${text.length} characters into ${selector}`,
    };
  }

  function performSubmit(selector) {
    const el = findTargetElement(selector);
    const form = el.tagName === "FORM" ? el : el.closest("form");
    if (!form) {
      throw new Error(`No form associated with selector: ${selector}`);
    }
    if (typeof form.requestSubmit === "function") {
      form.requestSubmit();
    } else {
      form.submit();
    }
    return {
      type: "action_ack",
      tab_id: 0,
      url: window.location.href,
      detail: `Submitted form for ${selector}`,
    };
  }

  function dismissActiveGuide() {
    if (typeof activeCleanup === "function") {
      activeCleanup();
      activeCleanup = null;
    }
    const existing = document.getElementById(OVERLAY_HOST_ID);
    if (existing) {
      existing.remove();
    }
  }

  function renderHighlightGuide({ selector, title, body, step_label: stepLabel }) {
    dismissActiveGuide();

    const target = findTargetElement(selector);
    target.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });

    const host = document.createElement("div");
    host.id = OVERLAY_HOST_ID;
    host.style.position = "fixed";
    host.style.inset = "0";
    host.style.zIndex = "2147483647";
    host.style.pointerEvents = "none";

    const shadow = host.attachShadow({ mode: "open" });

    const style = document.createElement("style");
    style.textContent = `
      .ring {
        position: fixed;
        border: 3px solid #2563eb;
        border-radius: 10px;
        box-shadow: 0 0 0 4px rgba(37, 99, 235, 0.25), 0 12px 30px rgba(15, 23, 42, 0.25);
        pointer-events: none;
        transition: all 140ms ease-out;
        box-sizing: border-box;
      }
      .callout {
        position: fixed;
        width: 300px;
        max-width: calc(100vw - 24px);
        background: #0f172a;
        color: #f8fafc;
        border: 1px solid rgba(148, 163, 184, 0.35);
        border-radius: 12px;
        padding: 14px 16px;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
        box-shadow: 0 18px 40px rgba(15, 23, 42, 0.45);
        pointer-events: auto;
        box-sizing: border-box;
      }
      .header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 8px;
        margin-bottom: 6px;
      }
      .badge {
        display: inline-block;
        font-size: 11px;
        font-weight: 600;
        letter-spacing: 0.03em;
        text-transform: uppercase;
        color: #93c5fd;
      }
      .close-btn {
        background: transparent;
        border: none;
        color: #94a3b8;
        font-size: 16px;
        line-height: 1;
        cursor: pointer;
        padding: 2px 6px;
        border-radius: 6px;
      }
      .close-btn:hover {
        background: rgba(148, 163, 184, 0.16);
        color: #f8fafc;
      }
      .title {
        font-size: 14px;
        font-weight: 600;
        margin: 0 0 6px 0;
        color: #ffffff;
      }
      .body {
        font-size: 13px;
        line-height: 1.45;
        margin: 0 0 12px 0;
        color: #cbd5e1;
      }
      .footer {
        display: flex;
        justify-content: flex-end;
      }
      .dismiss-btn {
        background: #2563eb;
        color: #ffffff;
        border: none;
        border-radius: 8px;
        font-size: 12px;
        font-weight: 600;
        padding: 6px 12px;
        cursor: pointer;
      }
      .dismiss-btn:hover {
        background: #1d4ed8;
      }
    `;

    const ring = document.createElement("div");
    ring.className = "ring";

    const callout = document.createElement("div");
    callout.className = "callout";
    callout.setAttribute("role", "dialog");
    callout.setAttribute("aria-label", title || "Humata Co-Pilot Guide");

    const header = document.createElement("div");
    header.className = "header";

    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = stepLabel ? `Humata Guide · ${stepLabel}` : "Humata Guide";

    const closeBtn = document.createElement("button");
    closeBtn.className = "close-btn";
    closeBtn.type = "button";
    closeBtn.setAttribute("aria-label", "Dismiss guide");
    closeBtn.textContent = "×";
    closeBtn.addEventListener("click", dismissActiveGuide);

    header.appendChild(badge);
    header.appendChild(closeBtn);

    const titleEl = document.createElement("h3");
    titleEl.className = "title";
    titleEl.textContent = title || "";

    const bodyEl = document.createElement("p");
    bodyEl.className = "body";
    bodyEl.textContent = body || "";

    const footer = document.createElement("div");
    footer.className = "footer";

    const dismissBtn = document.createElement("button");
    dismissBtn.className = "dismiss-btn";
    dismissBtn.type = "button";
    dismissBtn.textContent = "Got it";
    dismissBtn.addEventListener("click", dismissActiveGuide);

    footer.appendChild(dismissBtn);
    callout.appendChild(header);
    callout.appendChild(titleEl);
    callout.appendChild(bodyEl);
    callout.appendChild(footer);

    shadow.appendChild(style);
    shadow.appendChild(ring);
    shadow.appendChild(callout);
    document.documentElement.appendChild(host);

    const updatePosition = () => {
      const rect = target.getBoundingClientRect();
      const pad = 6;
      ring.style.top = `${Math.max(0, rect.top - pad)}px`;
      ring.style.left = `${Math.max(0, rect.left - pad)}px`;
      ring.style.width = `${rect.width + pad * 2}px`;
      ring.style.height = `${rect.height + pad * 2}px`;

      const calloutRect = callout.getBoundingClientRect();
      const spaceBelow = window.innerHeight - rect.bottom;
      const top =
        spaceBelow >= calloutRect.height + 16
          ? rect.bottom + 12
          : Math.max(12, rect.top - calloutRect.height - 12);
      const left = Math.min(
        Math.max(12, rect.left),
        Math.max(12, window.innerWidth - calloutRect.width - 12)
      );
      callout.style.top = `${top}px`;
      callout.style.left = `${left}px`;
    };

    const onKeyDown = (event) => {
      if (event.key === "Escape") {
        dismissActiveGuide();
      }
    };

    window.addEventListener("scroll", updatePosition, true);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("keydown", onKeyDown, true);
    updatePosition();

    activeCleanup = () => {
      window.removeEventListener("scroll", updatePosition, true);
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("keydown", onKeyDown, true);
    };

    return {
      type: "highlight",
      highlight: {
        tab_id: 0,
        selector,
        title,
        body,
        step_label: stepLabel || null,
        displayed: true,
      },
    };
  }

  api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (!message || message.source !== "humata2-browser-copilot" || !message.action) {
      return undefined;
    }

    try {
      const { action } = message;
      let payload;
      switch (action.type) {
        case "read_tab":
          payload = {
            type: "snapshot",
            snapshot: extractPageSnapshot(action.max_chars),
          };
          break;
        case "click":
          payload = performClick(action.selector);
          break;
        case "type":
          payload = performType(action.selector, action.text || "", Boolean(action.clear_first));
          break;
        case "submit":
          payload = performSubmit(action.selector);
          break;
        case "highlight_guide":
          payload = renderHighlightGuide(action);
          break;
        default:
          throw new Error(`Unsupported content action: ${String(action.type)}`);
      }
      sendResponse({ ok: true, payload });
    } catch (err) {
      sendResponse({
        ok: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
    return false;
  });
})();
