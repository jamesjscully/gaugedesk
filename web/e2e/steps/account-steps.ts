/**
 * "Your account" steps (ACCT-1, ADR 0053): open the account panel from
 * Settings ▸ Your account and link an LLM provider account.
 */

import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";

const { When, Then } = createBdd();

When("I open my account", async ({ page }) => {
    await page.locator("[data-settings]").click();
    await page.locator("[data-settings-account]").click();
    await expect(page.locator("[data-account-panel]")).toBeVisible();
});

When("I link the {string} account with token {string}", async ({ page }, provider: string, token: string) => {
    const link = page.locator(".account-panel .admin-invite");
    await link.locator("select").selectOption(provider);
    await page.locator("[data-account-token]").fill(token);
    await link.getByRole("button", { name: "link", exact: true }).click();
});

Then("{string} shows as a linked account", async ({ page }, provider: string) => {
    await expect(page.locator(`[data-linked="${provider}"]`)).toBeVisible();
});
