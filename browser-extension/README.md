# NextCue Meeting Companion

This optional local extension sends NextCue a hint when a supported meeting page opens. It recognizes Google Meet, Zoom web, Teams web, Webex, Jitsi at `meet.jit.si`, and Whereby meeting routes. It cannot confirm that you joined a call, identify participants, or determine when a call ended. Unsupported hosts and ordinary provider home pages do not trigger hints.

Only `{ app, id, kind: "browser-page" }` is sent to the desktop app: a provider name and a random page ID. Meeting URLs and codes are examined locally to classify the route and are never sent or saved by the companion. It does not use page scripts, DOM inspection, audio, screenshots, cookies, browsing history, or third-party network endpoints. There is no `tabs` permission or content script; URL access is limited to the listed meeting-site host permissions. The connection code is kept in this browser profile's local extension storage, never browser sync storage. The desktop stores its pairing token in the OS keychain and only a public preferred port in its app-data directory.

## Chrome, Edge, Brave, and Chromium

1. Open `chrome://extensions` or `edge://extensions` and enable Developer mode.
2. Choose **Load unpacked** and select this `browser-extension` directory.
3. In NextCue, open **Settings → Background & meetings**, enable meeting hints, choose what NextCue should do, and copy the browser connection code.
4. Open the extension's options, paste the code, select **Enable meeting-page hints in this browser**, and save.

Saving a connection or clicking **Check open meeting pages** intentionally checks already-open matching tabs. NextCue's default meeting action is **Do nothing**. Listening, note taking, and answering start only when explicitly selected in NextCue or manually triggered. The browser companion does not grant microphone or system-audio permissions.

## Firefox

The included `firefox` folder contains the Firefox development variant. In Firefox 140 or newer, open `about:debugging#/runtime/this-firefox`, choose **Load Temporary Add-on**, and select `firefox/manifest.json` inside the companion folder. Pair it using the same steps above. Firefox temporary add-ons are removed on browser restart; permanent distribution requires Mozilla signing. No browser settings or extensions are installed automatically. Firefox's data-permission disclosure covers the provider-only hint and pairing credential sent to your own local NextCue app; nothing is sent to a remote server.

Developers can regenerate that folder with `node browser-extension/build-firefox.cjs` from the project directory.

## Connection changes

NextCue binds only `127.0.0.1` and requires the random bearer token for every meeting hint. It normally reuses the previous port across launches. If that port is occupied, NextCue chooses a new port and rotates the pairing token so the old code cannot authenticate to the new listener. Copy the new code and replace the saved connection if the port changes or you switch app/QA profiles. Disable hints in NextCue or in the extension to stop forwarding. **Remove connection** clears the browser's pairing code.

This bridge is a meeting-page hint integration, not a Zoom/Teams/Google Meet call-state integration. Browser page routing may change; unknown routes are ignored. Native Windows/Linux builds and Firefox browser-to-desktop integration still need platform testing.

Run the route and pairing-code tests with `node --test browser-extension/routes.test.js`.

An isolated Chromium development-extension smoke test is available with `npx playwright test --config browser-extension/tests/playwright.config.cjs`. It uses a temporary browser profile, synthetic intercepted meeting pages, and a mock localhost endpoint. It never logs into meeting providers or uses a personal browser profile.
