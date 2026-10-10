import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests/ui",
  fullyParallel: false,
  use: {
    baseURL: "http://127.0.0.1:5173",
    channel: "chrome",
    viewport: { width: 1440, height: 1000 },
  },
  reporter: "list",
});
