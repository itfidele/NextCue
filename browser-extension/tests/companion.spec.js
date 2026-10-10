"use strict";
const { test, expect, chromium } = require("@playwright/test");
const http = require("node:http");
const fs = require("node:fs/promises");
const os = require("node:os");
const path = require("node:path");

test("isolated Chromium extension forwards provider-only hints and obeys pairing removal", async () => {
  const requests = [];
  const token = "a".repeat(43);
  const server = http.createServer((request, response) => {
    const origin = request.headers.origin;
    if (origin) {
      response.setHeader("Access-Control-Allow-Origin", origin);
      response.setHeader("Vary", "Origin");
    }
    response.setHeader("Access-Control-Allow-Methods", "POST");
    response.setHeader("Access-Control-Allow-Headers", "Authorization, Content-Type");
    if (request.method === "OPTIONS") {
      response.writeHead(204).end();
      return;
    }
    let body = "";
    request.on("data", (chunk) => { body += chunk; });
    request.on("end", () => {
      requests.push({ method: request.method, path: request.url, authorization: request.headers.authorization, origin, body });
      response.setHeader("Content-Type", "application/json");
      response.end(JSON.stringify({ accepted: true }));
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), "nextcue-extension-profile-"));
  const extension = path.resolve(__dirname, "..");
  let context;
  try {
    context = await chromium.launchPersistentContext(profile, {
      channel: "chromium",
      executablePath: process.env.NEXTCUE_EXTENSION_BROWSER_PATH,
      headless: true,
      args: [`--disable-extensions-except=${extension}`, `--load-extension=${extension}`],
    });
    await context.route("https://meet.google.com/**", (route) => route.fulfill({ contentType: "text/html", body: "<title>Fake test meeting</title><p>PRIVATE_TRANSCRIPT must never be sent</p>" }));
    await context.route("https://us02web.zoom.us/**", (route) => route.fulfill({ contentType: "text/html", body: "<title>Fake Zoom page</title>" }));
    await context.route("https://meet.google.com.evil.test/**", (route) => route.fulfill({ contentType: "text/html", body: "<title>Lookalike host</title>" }));
    let worker = context.serviceWorkers()[0];
    if (!worker) worker = await context.waitForEvent("serviceworker");
    const identifier = new URL(worker.url()).hostname;
    const settings = await context.newPage();
    await settings.goto(`chrome-extension://${identifier}/options.html`);
    await settings.getByLabel("Connection code", { exact: true }).fill(JSON.stringify({ port, token }));
    await settings.getByLabel("Enable meeting-page hints in this browser").check();
    await settings.getByRole("button", { name: "Save connection", exact: true }).click();
    await expect(settings.locator("#paired")).toContainText("A connection is saved");

    const meeting = await context.newPage();
    await meeting.goto("https://meet.google.com/abc-defg-hij?private=MEETING_SECRET");
    await expect.poll(() => requests.length).toBe(1);
    const first = requests[0];
    expect(first.method).toBe("POST");
    expect(first.path).toBe("/v1/meeting");
    expect(first.authorization).toBe(`Bearer ${token}`);
    expect(first.origin === undefined || first.origin === `chrome-extension://${identifier}`).toBe(true);
    const body = JSON.parse(first.body);
    expect(Object.keys(body).sort()).toEqual(["app", "id", "kind"]);
    expect(body.app).toBe("Google Meet");
    expect(body.kind).toBe("browser-page");
    expect(body.id).toMatch(/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/);
    expect(first.body).not.toContain("MEETING_SECRET");
    expect(first.body).not.toContain("abc-defg-hij");
    expect(first.body).not.toContain("PRIVATE_TRANSCRIPT");
    await meeting.goto("https://meet.google.com.evil.test/abc-defg-hij");
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(requests).toHaveLength(1);

    await meeting.goto("https://us02web.zoom.us/wc/12345678901/join?pwd=SECRET");
    await expect.poll(() => requests.length).toBe(2);
    expect(JSON.parse(requests[1].body).app).toBe("Zoom");
    expect(requests[1].body).not.toContain("12345678901");

    await settings.getByRole("button", { name: "Remove connection", exact: true }).click();
    await expect(settings.locator("#status")).toContainText("Connection removed");
    await meeting.goto("https://meet.google.com/klm-nopq-rst");
    await new Promise((resolve) => setTimeout(resolve, 400));
    expect(requests).toHaveLength(2);
    console.log(`Provider-only hints verified; extension fetch Origin ${first.origin ? "present" : "omitted"}.`);
  } finally {
    if (context) await context.close();
    await new Promise((resolve) => server.close(resolve));
    await fs.rm(profile, { recursive: true, force: true });
  }
});
