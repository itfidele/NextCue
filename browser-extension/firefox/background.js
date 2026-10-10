"use strict";

if (typeof importScripts === "function") importScripts("routes.js");
const extensionApi = typeof browser !== "undefined" ? browser : chrome;
const stateStorage = extensionApi.storage.session || extensionApi.storage.local;
let operations = Promise.resolve();

function queue(work) {
  operations = operations.then(work).catch(() => {
    // Connection failures are reported by the local options page. Never log a
    // URL, call identifier, pairing token, response body, or request payload.
  });
}

async function sendHint(tabId, value, reset) {
  const stored = await extensionApi.storage.local.get(["connection", "enabled"]);
  if (!stored.enabled || !stored.connection) return;
  let connection;
  try {
    connection = NextCueRoutes.parseConnection(stored.connection);
  } catch (_) {
    return;
  }
  const state = await stateStorage.get("meetingTabs");
  const tabs = state.meetingTabs || {};
  const key = String(tabId);
  if (reset) delete tabs[key];
  const app = NextCueRoutes.providerForUrl(value);
  if (!app) {
    delete tabs[key];
    await stateStorage.set({ meetingTabs: tabs });
    return;
  }
  if (tabs[key]?.app === app && tabs[key]?.sent) return;
  const page = tabs[key]?.app === app ? tabs[key] : { app, id: crypto.randomUUID(), sent: false };
  tabs[key] = page;
  await stateStorage.set({ meetingTabs: tabs });
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), 3000);
  try {
    const response = await fetch(`http://127.0.0.1:${connection.port}/v1/meeting`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${connection.token}` },
      body: JSON.stringify({ app, id: page.id, kind: "browser-page" }),
      signal: controller.signal,
      redirect: "error",
      credentials: "omit",
      cache: "no-store",
    });
    if (!response.ok) {
      await extensionApi.storage.local.set({ connectionStatus: "NextCue rejected the connection. Copy the connection code again." });
      return;
    }
    const result = await response.json();
    if (result.accepted || result.reason === "duplicate") {
      page.sent = true;
      await stateStorage.set({ meetingTabs: tabs });
      await extensionApi.storage.local.set({ connectionStatus: "Connected to NextCue on this computer." });
    } else if (result.reason === "disabled") {
      await extensionApi.storage.local.set({ connectionStatus: "Connected. Turn on meeting hints in NextCue Settings when needed." });
    }
  } catch (_) {
    await extensionApi.storage.local.set({ connectionStatus: "NextCue is unavailable. Open it, then check the connection code." });
  } finally {
    clearTimeout(timer);
  }
}

extensionApi.tabs.onUpdated.addListener((tabId, change, tab) => {
  // No tabs/history permission is requested. URL access is limited to the
  // meeting-site host permissions listed in the manifest.
  if (change.status === "loading") {
    queue(async () => {
      const state = await stateStorage.get("meetingTabs");
      const tabs = state.meetingTabs || {};
      delete tabs[String(tabId)];
      await stateStorage.set({ meetingTabs: tabs });
    });
  }
  if (change.status === "complete" || (change.url && tab.status === "complete")) {
    queue(() => sendHint(tabId, change.url || tab.url || "", false));
  }
});

extensionApi.tabs.onRemoved.addListener((tabId) => {
  queue(async () => {
    const state = await stateStorage.get("meetingTabs");
    const tabs = state.meetingTabs || {};
    delete tabs[String(tabId)];
    await stateStorage.set({ meetingTabs: tabs });
  });
});

extensionApi.runtime.onMessage.addListener((message, sender, reply) => {
  if (sender.id !== extensionApi.runtime.id || message?.type !== "rescan") return;
  queue(async () => {
    const tabs = await extensionApi.tabs.query({});
    // User explicitly saved a connection/enabled setting or clicked Check now.
    // Emit a new page hint for an already-open matching page when requested.
    for (const tab of tabs) {
      if (typeof tab.id === "number" && tab.url) await sendHint(tab.id, tab.url, true);
    }
    reply({ ok: true });
  });
  return true;
});

extensionApi.action.onClicked.addListener(() => extensionApi.runtime.openOptionsPage());
extensionApi.runtime.onInstalled.addListener(() => {
  extensionApi.storage.local.setAccessLevel?.({ accessLevel: "TRUSTED_CONTEXTS" });
});
