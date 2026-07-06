/**
 * Placement version-upgrade steps (UX-9, ADR 0063): publish a new archetype version from the
 * Library, then take the resulting "upgrade available" notice on a placement (manual default).
 */

import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";

const { When, Then } = createBdd();

When("I publish a new version of the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "publish a new version" }).click();
});

const placementBadge = (page: import("@playwright/test").Page, project: string) =>
    page.locator("[data-project]", { hasText: project }).locator("[data-upgrade-available]");

Then("the placement on {string} shows an upgrade is available", async ({ page }, project: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(placementBadge(page, project)).toBeVisible();
});

When("I upgrade the placement on {string}", async ({ page }, project: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await placementBadge(page, project).click();
});

Then("the placement on {string} is up to date", async ({ page }, project: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(placementBadge(page, project)).toHaveCount(0);
});
