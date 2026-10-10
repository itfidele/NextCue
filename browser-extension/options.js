"use strict";
const extensionApi = typeof browser !== "undefined" ? browser : chrome;
const connectionInput = document.getElementById("connection");
const enabledInput = document.getElementById("enabled");
const status = document.getElementById("status");
const paired = document.getElementById("paired");

async function refresh() {
  const stored = await extensionApi.storage.local.get(["connection", "enabled", "connectionStatus"]);
  enabledInput.checked = Boolean(stored.enabled);
  paired.textContent = stored.connection ? "A connection is saved in this browser profile. Paste a new code only to replace it." : "No connection saved yet.";
  status.textContent = stored.connectionStatus || "";
}

async function rescan() {
  status.textContent = "Checking supported meeting pages…";
  try {
    await extensionApi.runtime.sendMessage({ type: "rescan" });
    const stored = await extensionApi.storage.local.get("connectionStatus");
    status.textContent = stored.connectionStatus || "No supported meeting page was found. Open one when you need NextCue.";
  } catch (_) {
    status.textContent = "The companion is unavailable. Reload the extension and try again.";
  }
}

document.getElementById("setup").addEventListener("submit", async (event) => {
  event.preventDefault();
  try {
    const stored = await extensionApi.storage.local.get("connection");
    const connection = connectionInput.value.trim() ? NextCueRoutes.parseConnection(connectionInput.value) : stored.connection;
    if (!connection) throw new Error("Paste the connection code copied from NextCue Settings.");
    await extensionApi.storage.local.set({ connection, enabled: enabledInput.checked, connectionStatus: "" });
    connectionInput.value = "";
    await refresh();
    if (enabledInput.checked) await rescan();
    else status.textContent = "Saved. Meeting-page hints are off in this browser.";
  } catch (error) {
    status.textContent = error instanceof SyntaxError ? "Paste the complete JSON connection code copied from NextCue." : error.message;
  }
});

document.getElementById("check").addEventListener("click", async () => {
  const stored = await extensionApi.storage.local.get(["connection", "enabled"]);
  if (!stored.connection || !stored.enabled) {
    status.textContent = "Save a connection and enable meeting-page hints first.";
    return;
  }
  await rescan();
});

document.getElementById("remove").addEventListener("click", async () => {
  await extensionApi.storage.local.clear();
  if (extensionApi.storage.session) await extensionApi.storage.session.clear();
  connectionInput.value = "";
  await refresh();
  status.textContent = "Connection removed. Meeting-page hints are off.";
});

refresh();
