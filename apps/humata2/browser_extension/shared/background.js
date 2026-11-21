/* Humata 2 Native Browser Co-Pilot — Shared Background Router (Read-Only)
 *
 * Connects to the native messaging host `app.humata2.browser_host` and routes
 * strictly read-only BrowserRequest messages:
 *   - `list_tabs`: view open tabs
 *   - `read_tab`: read-only DOM/text snapshot via declarative content script
 *   - `screenshot`: read-only visible tab PNG screenshot
 *
 * Never clicks, types, navigates, mutates the DOM, or executes scripts.
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

async function readSnapshotFromContentScript(tabId, action) {
  return api.tabs.sendMessage(tabId, {
    source: "humata2-browser-copilot",
    action,
  });
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

    case "read_tab": {
      const tab = await resolveTargetTab(action.tab_id);
      const result = await readSnapshotFromContentScript(tab.id, {
        type: "read_tab",
        tab_id: action.tab_id,
        max_chars: action.max_chars,
      });
      if (!result || !result.ok) {
        postReply({
          request_id: requestId,
          ok: false,
          error: (result && result.error) || "Content script failed for read_tab",
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

    case "screenshot": {
      const tab = await resolveTargetTab(action.tab_id);
      const dataUrl = await api.tabs.captureVisibleTab(tab.windowId, {
        format: "png",
      });
      postReply({
        request_id: requestId,
        ok: true,
        payload: {
          type: "screenshot",
          screenshot: {
            tab_id: tab.id,
            window_id: tab.windowId,
            url: tab.url || "",
            title: tab.title || "",
            format: "png",
            data_url: dataUrl,
          },
        },
      });
      return;
    }

    default:
      throw new Error(
        `Unsupported or disallowed browser action type (read-only extension): ${String(action.type)}`
      );
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
  return payload;
}

if (api.runtime && api.runtime.onStartup) {
  api.runtime.onStartup.addListener(connectNativeHost);
}
if (api.runtime && api.runtime.onInstalled) {
  api.runtime.onInstalled.addListener(connectNativeHost);
}

connectNativeHost();
