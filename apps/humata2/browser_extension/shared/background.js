/* Humata 2 Native Browser Co-Pilot — Shared Background Router
 *
 * Connects to the native messaging host `app.humata2.browser_host` and routes
 * typed BrowserRequest messages (`list_tabs`, `read_tab`, `click`, `type`,
 * `navigate`, `submit`, `highlight_guide`) to the active or targeted tab.
 */

const NATIVE_HOST_NAME = "app.humata2.browser_host";
const RECONNECT_BASE_MS = 1000;
const RECONNECT_MAX_MS = 30000;

const api = typeof browser !== "undefined" ? browser : chrome;

let nativePort = null;
let reconnectDelayMs = RECONNECT_BASE_MS;
let reconnectTimer = null;

function connectNativeHost() {
  if (nativePort) {
    return;
  }
  try {
    nativePort = api.runtime.connectNative(NATIVE_HOST_NAME);
    reconnectDelayMs = RECONNECT_BASE_MS;

    nativePort.onMessage.addListener((message) => {
      handleNativeRequest(message).catch((err) => {
        postReply({
          request_id: message && message.request_id ? String(message.request_id) : "unknown",
          ok: false,
          error: err instanceof Error ? err.message : String(err),
        });
      });
    });

    nativePort.onDisconnect.addListener(() => {
      nativePort = null;
      scheduleReconnect();
    });
  } catch (_err) {
    nativePort = null;
    scheduleReconnect();
  }
}

function scheduleReconnect() {
  if (reconnectTimer !== null) {
    return;
  }
  const delay = reconnectDelayMs;
  reconnectDelayMs = Math.min(reconnectDelayMs * 2, RECONNECT_MAX_MS);
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null;
    connectNativeHost();
  }, delay);
}

function postReply(reply) {
  if (!nativePort) {
    return;
  }
  try {
    nativePort.postMessage(reply);
  } catch (_err) {
    nativePort = null;
    scheduleReconnect();
  }
}

async function queryTabs(queryInfo) {
  return api.tabs.query(queryInfo);
}

async function resolveTargetTab(tabId) {
  if (typeof tabId === "number") {
    const tab = await api.tabs.get(tabId);
    if (!tab) {
      throw new Error(`Tab ${tabId} not found`);
    }
    return tab;
  }
  const activeTabs = await queryTabs({ active: true, lastFocusedWindow: true });
  if (activeTabs && activeTabs.length > 0) {
    return activeTabs[0];
  }
  const allTabs = await queryTabs({});
  if (allTabs && allTabs.length > 0) {
    return allTabs[0];
  }
  throw new Error("No open browser tabs available");
}

async function sendToContentScript(tabId, action) {
  try {
    return await api.tabs.sendMessage(tabId, {
      source: "humata2-browser-copilot",
      action,
    });
  } catch (_initialErr) {
    // Inject shared/content.js on demand if the tab was opened prior to extension load.
    if (api.scripting && typeof api.scripting.executeScript === "function") {
      await api.scripting.executeScript({
        target: { tabId },
        files: ["shared/content.js"],
      });
      return api.tabs.sendMessage(tabId, {
        source: "humata2-browser-copilot",
        action,
      });
    }
    throw _initialErr;
  }
}

async function handleNativeRequest(request) {
  if (!request || typeof request.request_id !== "string" || !request.action) {
    throw new Error("Malformed BrowserRequest: missing request_id or action");
  }
  const { request_id: requestId, action } = request;

  switch (action.type) {
    case "list_tabs": {
      const tabs = await queryTabs({});
      postReply({
        request_id: requestId,
        ok: true,
        payload: {
          type: "tabs",
          tabs: tabs.map((tab) => ({
            tab_id: tab.id,
            window_id: tab.windowId,
            title: tab.title || "",
            url: tab.url || "",
            active: Boolean(tab.active),
            pinned: Boolean(tab.pinned),
          })),
        },
      });
      return;
    }

    case "navigate": {
      if (typeof action.url !== "string" || action.url.trim() === "") {
        throw new Error("Navigate action requires a non-empty url");
      }
      const tab = await resolveTargetTab(action.tab_id);
      const updated = await api.tabs.update(tab.id, { url: action.url });
      postReply({
        request_id: requestId,
        ok: true,
        payload: {
          type: "action_ack",
          tab_id: updated && typeof updated.id === "number" ? updated.id : tab.id,
          url: action.url,
          detail: `Navigated tab ${tab.id} to ${action.url}`,
        },
      });
      return;
    }

    case "read_tab":
    case "click":
    case "type":
    case "submit":
    case "highlight_guide": {
      const tab = await resolveTargetTab(action.tab_id);
      const result = await sendToContentScript(tab.id, action);
      if (!result || !result.ok) {
        postReply({
          request_id: requestId,
          ok: false,
          error: (result && result.error) || `Content script failed for ${action.type}`,
        });
        return;
      }
      postReply({
        request_id: requestId,
        ok: true,
        payload: enrichPayloadWithTabId(result.payload, tab.id),
      });
      return;
    }

    default:
      throw new Error(`Unsupported browser action type: ${String(action.type)}`);
  }
}

function enrichPayloadWithTabId(payload, tabId) {
  if (!payload || typeof payload !== "object") {
    return payload;
  }
  if (payload.type === "snapshot" && payload.snapshot) {
    return {
      type: "snapshot",
      snapshot: { ...payload.snapshot, tab_id: tabId },
    };
  }
  if (payload.type === "highlight" && payload.highlight) {
    return {
      type: "highlight",
      highlight: { ...payload.highlight, tab_id: tabId },
    };
  }
  if (payload.type === "action_ack") {
    return { ...payload, tab_id: tabId };
  }
  return payload;
}

if (api.runtime && api.runtime.onStartup) {
  api.runtime.onStartup.addListener(connectNativeHost);
}
if (api.runtime && api.runtime.onInstalled) {
  api.runtime.onInstalled.addListener(connectNativeHost);
}

connectNativeHost();
