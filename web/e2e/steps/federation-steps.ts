/**
 * Cross-machine federation steps (M8 / FED-7). Drives TWO browser windows — alice at
 * the default backend (7878) and bob via `?cp=http://127.0.0.1:7879` — through the
 * reworked surface: the global **Paired devices** panel (pair a peer machine, co-drive,
 * auto-accept incoming) and the per-project **Engagement pane** (hand off a project's
 * home), opened from the project's node. Peer-side evidence is asserted through the
 * peer's projection, since each mutation crosses a real cert-pinned TLS leg through the
 * broker between two separate control-plane processes.
 */

import { expect, type Page } from "@playwright/test";
import { createBdd } from "playwright-bdd";
import { aliceCP, bobCP } from "../ports.mjs";

const { Given, When, Then } = createBdd();

const ALICE_CP = aliceCP;
const BOB_CP = bobCP;

// The peer (bob) browser window, opened in the same context as the primary page.
let bobPage: Page | null = null;

// Device management lives under Settings ▸ Devices (FED-7): the gear in the Browse
// bottom bar → the Devices option → the single Devices modal.
async function openPairedDevices(page: Page): Promise<void> {
    await page.locator("[data-settings]").click();
    await page.locator("[data-settings-devices]").click();
    await expect(page.locator("[data-devices-modal]")).toBeVisible();
    // The separate-party pairing ticket is minted async on mount; wait for it.
    await expect(page.locator("[data-pd-ticket]")).toHaveText(/.+/);
}

Given("the two federated workbenches are open", async ({ page, request }) => {
    // The global Before resets 7878; reset the peer (7879) too for a clean pair.
    const reset = await request.post(`${BOB_CP}/test/reset`);
    if (!reset.ok()) throw new Error(`peer reset failed: ${reset.status()}`);

    // Seed a real project in alice's library *before* she loads, so the Engagement
    // pane can be opened from its node later (no typed project id anywhere).
    const mk = await request.post(`${ALICE_CP}/projects`, { data: { name: "Acme Engagement" } });
    if (!mk.ok()) throw new Error(`project create failed: ${mk.status()}`);

    // Alice on the default backend; Bob via ?cp= on the same Vite preview.
    await page.goto("/");
    bobPage = await page.context().newPage();
    await bobPage.goto(`/?cp=${encodeURIComponent(BOB_CP)}`);

    await openPairedDevices(page);
    await openPairedDevices(bobPage);
});

When("the two authorities pair with each other", async ({ page }) => {
    const bob = bobPage!;
    const aliceTicket = await page.locator("[data-pd-ticket]").textContent();
    const bobTicket = await bob.locator("[data-pd-ticket]").textContent();
    expect(aliceTicket, "alice minted a ticket").toBeTruthy();
    expect(bobTicket, "bob minted a ticket").toBeTruthy();

    // Each accepts the other's ticket (TOFU pin), so both hold the other's grant.
    await bob.locator("[data-pd-paste]").fill(aliceTicket!.trim());
    await bob.locator("[data-pd-pair]").click();
    await page.locator("[data-pd-paste]").fill(bobTicket!.trim());
    await page.locator("[data-pd-pair]").click();

    // Both render the pairing (alice is `local-user`, bob is `bob`).
    await expect(page.locator('[data-pd-peer="bob"]')).toBeVisible();
    await expect(bob.locator('[data-pd-peer="local-user"]')).toBeVisible();

    // Give the peers' receiver tasks a moment to park on the broker.
    await page.waitForTimeout(800);
});

// The raw "cross a handle" / "remote run" controls were retired from the UI (co-drive
// is now the per-project Engagement-pane flow, FED-7); the FED-1 endpoints still back
// them, so these steps exercise the endpoints directly through the control plane.
let lastRun: { observations_admitted?: number } | null = null;

When("the owner crosses a handle to the peer", async ({ request }) => {
    const r = await request.post(`${ALICE_CP}/federation/cross`, {
        data: { peer: "bob", handle: "ctx-method-HANDLE", correlation: `xc-${Date.now()}` },
    });
    expect(r.ok(), "cross admitted").toBeTruthy();
});

Then("the handle appears in the peer's federation inbox", async ({ request }) => {
    await expect
        .poll(
            async () => {
                const r = await request.get(`${BOB_CP}/federation/inbox`);
                const j = (await r.json()) as { federated?: unknown[] };
                return (j.federated ?? []).length;
            },
            { timeout: 8_000 },
        )
        .toBeGreaterThan(0);
});

When("the owner places a remote run on the peer", async ({ request }) => {
    const r = await request.post(`${ALICE_CP}/federation/remote-run`, {
        data: { peer: "bob", run_scope: `run-${Date.now()}`, prompt: "go" },
    });
    expect(r.ok(), "remote run ok").toBeTruthy();
    lastRun = (await r.json()) as { observations_admitted?: number };
});

Then("the owner sees the peer's observations were admitted", async () => {
    expect(lastRun?.observations_admitted ?? 0).toBeGreaterThan(0);
});

// FED-6/7: relocate a project's home to the peer over the wire, driven from the
// per-project Engagement pane. Bob first pre-authorizes alice (in his Paired devices
// panel), so the relocate auto-accepts on the peer and commits deterministically: bob
// imports the log + becomes home, alice becomes the operator. (The pending-consent path
// is covered by the two-control-plane Rust test `handoff_relocation.rs`.)
When("the owner hands off a project's home", async ({ page }) => {
    const bob = bobPage!;
    await bob.locator('[data-pd-preauth="local-user"]').click();

    // Alice closes the Devices modal and opens the project's Engagement pane
    // from its node (no typed project id — the id comes from the node). The default
    // Browse facet is Chats (the WS-H nav default), so switch to Projects to reach
    // the project tree node.
    await page.locator("[data-devices-modal] .modal-head button").click();
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator(".tree-node.project", { hasText: "Acme Engagement" })
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "share & hand off" }).click();
    await expect(page.locator("[data-engagement-pane]")).toBeVisible();

    // One action — hand off to the (only) paired device, bob.
    await page.locator("[data-engagement-handoff]").click();
});

Then("the project's handoff is committed to the target", async ({ page }) => {
    await expect(page.locator("[data-engagement-feedback]")).toContainText("home is now there");
    await expect(page.locator("[data-engagement-phase]")).toContainText("committed");
});
