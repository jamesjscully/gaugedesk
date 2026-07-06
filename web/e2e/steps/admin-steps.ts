/**
 * Admin console steps (M3, ADR 0043): drive the real org-facing UI. Post-split
 * (SPLIT-2) the console is a **standalone enterprise app**
 * (`ee/web/apps/admin-console`), not a workbench panel: it loads from its own
 * static preview and points at the self-hosted enterprise composition
 * (`gaugewright-enterprise-server`, ee/) via `?cp=` — the composition that mounts
 * the `/admin/*` control plane. The workbench keeps only the negative space: its
 * settings menu offers no org admin entry (the solo collapse, DEPLOY-7).
 */

import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";
import { adminAppURL, enterpriseCP } from "../ports.mjs";

const { Given, When, Then } = createBdd();

Given("the admin console app is open for a provisioned tenant", async ({ page, request }) => {
    // DEPLOY-7 (ADR 0059 §6): the org admin console is a tenant surface — a standalone
    // app pointed at a provisioned org control plane (`?cp=`), never part of the
    // workbench bundle. Start the tenant clean (the same test-only reset the workbench
    // suite applies to alice per scenario), then load the console against it.
    const res = await request.post(`${enterpriseCP}/test/reset`);
    if (!res.ok()) {
        throw new Error(`enterprise control-plane reset failed: ${res.status()} ${await res.text()}`);
    }
    await page.goto(`${adminAppURL}?cp=${encodeURIComponent(enterpriseCP)}`);
    await expect(page.locator("[data-admin-console]")).toBeVisible();
});

When("I open the settings menu", async ({ page }) => {
    await page.locator("[data-settings]").click();
    await expect(page.locator("[data-settings-menu]")).toBeVisible();
});

Then("the organization admin entry is not offered", async ({ page }) => {
    // The workbench never links the org admin console: it is a separate enterprise
    // app (SPLIT-2), and the solo collapse has nothing to offer ("you are not an org").
    await expect(page.locator("[data-settings-admin]")).toHaveCount(0);
});

When("I invite member {string} as {string}", async ({ page }, authority: string, role: string) => {
    await page.locator("[data-admin-invite-authority]").fill(authority);
    await page.locator(".admin-invite select").selectOption(role);
    await page.locator(".admin-invite button").click();
});

Then("the member {string} appears in the directory", async ({ page }, authority: string) => {
    await expect(page.locator(`[data-member="${authority}"]`)).toBeVisible();
});

Then("the audit log shows the {string} action", async ({ page }, action: string) => {
    await expect(page.locator("[data-audit-list]")).toContainText(action);
});

Then("the admin console offers SSO sign-in", async ({ page }) => {
    // Signed-out single-user desktop ⇒ the identity section shows the "Sign in with
    // SSO" affordance (ID-3, the OIDC shell's client half).
    await expect(page.locator("[data-admin-identity]")).toBeVisible();
    await expect(page.locator("[data-admin-signin]")).toBeVisible();
});

Then("the admin console shows the integration details", async ({ page }) => {
    // ONB-1: the SP-side values an admin pastes into their IdP, with copy buttons.
    const panel = page.locator("[data-admin-integration]");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("/auth/callback");
    await expect(panel).toContainText("/scim/v2");
});

When("I launch the SSO setup wizard", async ({ page }) => {
    await page.locator("[data-admin-sso-wizard]").click();
    await expect(page.locator("[data-sso-wizard]")).toBeVisible();
});

Then("the SSO wizard shows the connect step", async ({ page }) => {
    await expect(page.locator("[data-wizard-connect]")).toBeVisible();
    await expect(page.locator("[data-wizard-connect]")).toContainText("/auth/callback");
});

When("I advance the SSO wizard", async ({ page }) => {
    await page.locator("[data-wizard-next]").click();
});

Then("the SSO wizard shows the test step", async ({ page }) => {
    await expect(page.locator("[data-wizard-test]")).toBeVisible();
    await expect(page.locator("[data-wizard-test-btn]")).toBeVisible();
});

// ITGOV-2: the IT session roster is surfaced in the admin console.
Then("the admin console shows the active sessions roster", async ({ page }) => {
    const panel = page.locator("[data-sessions]");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("Active sessions");
});
