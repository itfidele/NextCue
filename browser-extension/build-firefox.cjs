"use strict";
const fs = require("node:fs");
const path = require("node:path");
const directory = __dirname;
const target = path.join(directory, "firefox");
const manifest = JSON.parse(fs.readFileSync(path.join(directory, "manifest.json"), "utf8"));
manifest.background = { scripts: ["routes.js", "background.js"] };
manifest.browser_specific_settings = {
  gecko: {
    id: "nextcue-meeting-companion@nextcue.local",
    strict_min_version: "140.0",
    // Hints and the local pairing credential leave the browser for the user's
    // own desktop app. Declare that transmission instead of claiming "none".
    data_collection_permissions: { required: ["browsingActivity", "authenticationInfo"] },
  },
};
fs.mkdirSync(target, { recursive: true });
fs.writeFileSync(path.join(target, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
for (const filename of ["routes.js", "background.js", "options.html", "options.js", "options.css"]) {
  fs.copyFileSync(path.join(directory, filename), path.join(target, filename));
}
process.stdout.write("Firefox development extension prepared in browser-extension/firefox.\n");
