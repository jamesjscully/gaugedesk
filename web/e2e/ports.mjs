/**
 * The single source of truth for the e2e harness's ports (concurrency-safety).
 *
 * Every port the suite uses — the two open control planes, the enterprise control plane,
 * the rendezvous broker, and the Vite previews the browser loads (the workbench plus the
 * standalone admin-console app) — is read from the environment so that
 * **two runs never collide**
 * (a second worktree / a parallel agent picks a disjoint set). `e2e/run.mjs` resolves a free
 * set once per run and exports these vars to every child (vite build, bddgen, playwright);
 * when they're unset (e.g. a bare `playwright test`) we fall back to the historical defaults,
 * so direct invocation still works.
 *
 * Derived values (CP base URLs, the preview origin, per-run state dirs) live here too, so the
 * playwright config, the step files, and vite all agree by construction.
 */

const num = (name, dflt) => {
    const v = process.env[name];
    const n = v ? Number(v) : NaN;
    return Number.isInteger(n) && n > 0 && n < 65536 ? n : dflt;
};

export const ports = {
    alice: num("GW_E2E_ALICE", 7878),
    bob: num("GW_E2E_BOB", 7879),
    broker: num("GW_E2E_BROKER", 7900),
    preview: num("GW_E2E_PREVIEW", 4173),
    // The self-hosted enterprise composition (`gaugewright-enterprise-server`, ee/):
    // the /admin/* + SSO surface WITHOUT the managed planes — what the standalone
    // admin-console app drives (enterprise coverage must not require private code).
    enterprise: num("GW_E2E_ENTERPRISE", 7882),
    // Static preview of the standalone enterprise admin-console bundle (ee/web).
    adminApp: num("GW_E2E_ADMIN_APP", 4174),
};

export const aliceCP = `http://127.0.0.1:${ports.alice}`;
export const bobCP = `http://127.0.0.1:${ports.bob}`;
export const brokerAddr = `127.0.0.1:${ports.broker}`;
export const previewURL = `http://127.0.0.1:${ports.preview}`;
export const enterpriseCP = `http://127.0.0.1:${ports.enterprise}`;
/** The standalone enterprise admin-console app (ee/web's built bundle, served whole-dist). */
export const adminAppURL = `http://127.0.0.1:${ports.adminApp}/apps/admin-console/`;

/** Per-run control-plane state dirs (keyed by port so concurrent runs don't share state). */
export const aliceState = `/tmp/gw-e2e-state-${ports.alice}`;
export const bobState = `/tmp/gw-e2e-state-${ports.bob}`;
export const enterpriseState = `/tmp/gw-e2e-state-${ports.enterprise}`;
