import { defineConfig, devices } from "@playwright/test"

const URL_DEV = "http://127.0.0.1:1420"

export default defineConfig({
    testDir: "tests/e2e",
    fullyParallel: true,
    forbidOnly: !!process.env.CI,
    retries: process.env.CI ? 2 : 0,
    reporter: process.env.CI ? "github" : "list",
    use: {
        baseURL: URL_DEV,
        trace: "on-first-retry",
    },
    projects: [
        {
            name: "chromium",
            use: { ...devices["Desktop Chrome"] },
        },
    ],
    webServer: {
        command: "bun run dev --host 127.0.0.1",
        url: URL_DEV,
        reuseExistingServer: !process.env.CI,
        timeout: 120_000,
    },
})
