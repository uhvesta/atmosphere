/* Humata 2 Native Browser Co-Pilot — Shared Content Script (Read-Only)
 *
 * Strictly read-only: extracts structured accessibility/page snapshots
 * (`read_tab`). Never clicks, types, submits, highlights, mutates the DOM,
 * or executes page scripts.
 */

(() => {
  if (window.__humata2BrowserCopilotLoaded) {
    return;
  }
  window.__humata2BrowserCopilotLoaded = true;

  const api = typeof browser !== "undefined" ? browser : chrome;
  const DEFAULT_MAX_CHARS = 16000;
  const MAX_INTERACTIVE_ELEMENTS = 150;

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

  api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (!message || message.source !== "humata2-browser-copilot" || !message.action) {
      return undefined;
    }

    try {
      const { action } = message;
      if (action.type !== "read_tab") {
        throw new Error(
          `Unsupported or disallowed content action (read-only extension): ${String(action.type)}`
        );
      }
      const payload = {
        type: "snapshot",
        snapshot: extractPageSnapshot(action.max_chars),
      };
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
