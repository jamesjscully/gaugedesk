/**
 * Steps for the embedded panels (EMBED-2). The embed example page mounts the
 * `<gw-session>` + `<gw-chat>`/`<gw-viewer>`/`<gw-files>` custom elements against a
 * scoped control plane; it self-creates a fresh work chat when none is given, so
 * the page is self-contained. The panels render in **open** shadow roots, which
 * Playwright's selectors pierce automatically. The control-plane reset Before-hook
 * is shared from `steps.ts` (one global hook), so this scenario also starts clean.
 */
import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";

const { Given, When, Then } = createBdd();

Given("the embed example page is open", async ({ page }) => {
    await page.goto("/embed-example.html");
    // The composer appears once <gw-session> built the remote Session and <gw-chat>
    // rendered into its shadow root — proof the panel mounted against a non-desktop
    // session (the createEngagement quick-start + bind round-trip).
    await expect(page.locator("[data-embed-composer]")).toBeVisible({ timeout: 15_000 });
});

Then("the embedded chat shows a composer", async ({ page }) => {
    await expect(page.locator("[data-embed-send]")).toBeVisible();
});

When("I send {string} in the embedded chat", async ({ page }, msg: string) => {
    await page.locator("[data-embed-composer]").fill(msg);
    await page.locator("[data-embed-send]").click();
});

Then("the embedded transcript shows {string}", async ({ page }, text: string) => {
    // The optimistic echo lands the instant the turn starts — end-to-end proof that
    // the embedded composer drives the remote Session's send.
    await expect(page.locator("[data-embed-transcript]")).toContainText(text, { timeout: 15_000 });
});

Then("the embedded chat is themed by the workbench palette", async ({ page }) => {
    // The :host theme bridge defines the workbench palette inside the shadow root
    // (styles.css's :root block is inert there) — the default --gw-bg (#0f1115).
    await expect(page.locator("gw-chat")).toHaveCSS("background-color", "rgb(15, 17, 21)");
});

Then("a {string} override cascades into the panel's shadow root", async ({ page }, token: string) => {
    await page.locator("gw-session").evaluate((el, name) => {
        (el as HTMLElement).style.setProperty(name, "rgb(20, 0, 40)");
    }, token);
    // A consultant-set --gw-* token on the ancestor cascades across the shadow
    // boundary into the panel host (custom properties inherit through shadow DOM).
    await expect(page.locator("gw-chat")).toHaveCSS("background-color", "rgb(20, 0, 40)");
});
