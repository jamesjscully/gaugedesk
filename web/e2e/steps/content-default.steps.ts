/**
 * Step(s) for the content viewer's default surface (View unless a review is open).
 * Kept in its own file so it composes with the shared steps without editing them.
 */
import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";

const { Then } = createBdd();

Then("the content viewer is on the {string} tab", async ({ page }, tab: string) => {
    await expect(page.locator(`[data-viewer-tabs] .tab[data-tab="${tab}"]`)).toHaveClass(/\bactive\b/);
});
