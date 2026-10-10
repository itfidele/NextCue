const { defineConfig } = require("@playwright/test");
module.exports = defineConfig({
  testDir: __dirname,
  testMatch: "*.spec.js",
  fullyParallel: false,
  workers: 1,
  timeout: 45000,
  reporter: "list",
  outputDir: "/private/tmp/nextcue-extension-test-results",
});
