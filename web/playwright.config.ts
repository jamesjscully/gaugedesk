import { defineConfig } from "@playwright/test";
import { defineBddConfig } from "playwright-bdd";
// Single source of truth for the harness ports (concurrency-safety). `e2e/run.mjs`
// resolves a free set per run and exports them; here we just read the resolved values.
import {
    adminAppURL,
    aliceState,
    bobState,
    brokerAddr,
    enterpriseCP,
    enterpriseState,
    ports,
    previewURL,
} from "./e2e/ports.mjs";

// Gherkin .feature files → generated Playwright specs (the "pickle" pipeline).
// Enterprise features live with their owning workspace (ee/web); this harness stays
// the one repo-wide runner until that package runs independently, so its discovery
// spans both roots (featuresRoot = the repo root).
const testDir = defineBddConfig({
    features: [
        "e2e/features/**/*.feature",
        "../ee/web/e2e/features/**/*.feature",
    ],
    featuresRoot: "..",
    steps: "e2e/steps/**/*.ts",
});

export default defineConfig({
    testDir,
    fullyParallel: false, // one shared control plane; scenarios share its state
    workers: 1,
    timeout: 60_000, // generous enough for an @live real-model WhippleScript turn
    expect: { timeout: 8_000 },
    reporter: [["list"]],
    use: {
        baseURL: previewURL,
        channel: "chrome", // system Google Chrome — no browser download
        headless: true,
        trace: "retain-on-failure",
    },
    webServer: [
        // The rendezvous broker both authorities dial out to (M8 federation).
        {
            command: "bash e2e/broker.sh",
            port: ports.broker,
            reuseExistingServer: false,
            timeout: 30_000,
            env: { BROKER_PORT: String(ports.broker) },
        },
        // The primary control plane (alice / `local-user`) — the one the existing
        // single-instance suite drives. Port-scoped launcher so the peer instance
        // survives (no blanket pkill). Its CORS allowlist blesses THIS run's preview
        // origin so the cross-origin client calls are admitted (FED-2).
        {
            command: "bash e2e/fed-control-plane.sh",
            url: `http://127.0.0.1:${ports.alice}/chats`,
            reuseExistingServer: false,
            timeout: 30_000,
            env: {
                FED_PORT: String(ports.alice),
                GAUGEWRIGHT_BROKER_ADDR: brokerAddr,
                GAUGEWRIGHT_E2E_STATE: aliceState,
                GAUGEWRIGHT_ALLOWED_ORIGINS: previewURL,
            },
        },
        // The federation peer (authority `bob`) — only the cross-machine scenarios use
        // it; it sits idle for the rest.
        {
            command: "bash e2e/fed-control-plane.sh",
            url: `http://127.0.0.1:${ports.bob}/chats`,
            reuseExistingServer: false,
            timeout: 30_000,
            env: {
                FED_PORT: String(ports.bob),
                GAUGEWRIGHT_AUTHORITY: "bob",
                GAUGEWRIGHT_BROKER_ADDR: brokerAddr,
                GAUGEWRIGHT_E2E_STATE: bobState,
                GAUGEWRIGHT_ALLOWED_ORIGINS: previewURL,
            },
        },
        {
            command: `npm run preview -- --port ${ports.preview} --strictPort`,
            url: previewURL,
            reuseExistingServer: false,
            timeout: 30_000,
        },
        // The SELF-HOSTED enterprise composition (`gaugewright-enterprise-server`,
        // ee/): the /admin/* + SSO surface without the managed planes. The
        // standalone admin-console scenarios point at it via `?cp=` — enterprise
        // coverage runs against ee code only, never the private cloud repo.
        {
            command: "bash e2e/enterprise-control-plane.sh",
            url: `${enterpriseCP}/chats`,
            reuseExistingServer: false,
            timeout: 30_000,
            env: {
                ENTERPRISE_PORT: String(ports.enterprise),
                GAUGEWRIGHT_E2E_STATE: enterpriseState,
                GAUGEWRIGHT_ALLOWED_ORIGINS: [
                    previewURL,
                    new URL(adminAppURL).origin,
                ].join(","),
            },
        },
        // Static preview of the standalone admin-console bundle (SPLIT-2), built by
        // e2e/run.mjs in its own workspace. The vite preview serves the whole dist,
        // so the app page sits at /apps/admin-console/ (the ports.mjs URL).
        {
            command: `npx vite preview --config apps/admin-console/vite.config.ts --port ${ports.adminApp} --strictPort`,
            cwd: "../ee/web",
            url: adminAppURL,
            reuseExistingServer: false,
            timeout: 30_000,
        },
    ],
});
