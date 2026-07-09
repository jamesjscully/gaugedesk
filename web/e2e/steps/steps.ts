/**
 * Step definitions for the GaugeDesk user stories. The client is a thin
 * renderer over the control plane, so the steps drive the real UI (clicks +
 * reads) and assert on rendered projections — never on internal state.
 *
 * The model (ADR 0035/0036): the nav is **project-first**, facets
 * **Chats | Projects | Library** (default Chats). An **archetype** is the
 * reusable method (Library); a **placement** is an archetype installed on a
 * **project** (Projects) — what you chat with to do work. A chat's **kind** is
 * its ROOT, fixed at creation: rooted on an archetype ⇒ an *edit* chat; rooted on
 * a placement ⇒ a *work* chat. There is no mode toggle. The default "Personal"
 * project (and its default placement) is hidden, so a *work* chat is reached by
 * creating a project, placing an archetype on it, then opening a chat under that
 * placement.
 */

import { expect, type Page } from "@playwright/test";
import { createBdd } from "playwright-bdd";
import { aliceCP } from "../ports.mjs";

const { Given, When, Then, Before } = createBdd();

// Per-scenario clean slate. The whole suite shares ONE control plane, run serially,
// so without this the append-only store accumulates every prior scenario's projects,
// archetypes and chats — and later scenarios collide with the pile (a global
// `.first()` grabs a stale chat; a context menu opens off-screen on a tall tree).
// The test-only POST /test/reset route (gated by GAUGEWRIGHT_TEST_RESET, set in
// fed-control-plane.sh) stops live agents, wipes the state, and re-seeds — so every
// scenario starts from the same fresh workbench, pollution-proof by construction.
Before(async ({ request }) => {
    const res = await request.post(`${aliceCP}/test/reset`);
    if (!res.ok()) throw new Error(`control-plane reset failed: ${res.status()} ${await res.text()}`);
});

// A fresh, uniquely-named project per call: the control plane is shared across
// scenarios (serial, single worker), so unique names keep them isolated.
let seq = 0;
function freshProjectName(): string {
    return `e2e-proj-${Date.now()}-${seq++}`;
}

// The project whose placement the multi-agent scenario opens several chats under
// (round-13): shared across that scenario's steps so "another chat under that
// placement" targets the same placement.
let concProject = "";

/** Create a project, place the (default) archetype on it, and return the project
 *  group locator — the placement under it is what work chats are rooted on. */
async function placeArchetypeOnFreshProject(page: Page): Promise<string> {
    const name = freshProjectName();
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page.getByText("+ project", { exact: true }).click();
    await page.locator(".inline-edit").fill(name);
    await page.locator(".inline-edit").press("Enter");
    const group = page.locator(`.tree-group[data-project]`, { hasText: name });
    await expect(group.locator(".tree-node.project")).toBeVisible();
    // "add a method" opens the picker (#1); choose the first available method.
    await group.locator(".tree-node.project").click({ button: "right" });
    await page.locator(".menu-item", { hasText: "add an archetype" }).click();
    await pickFirstMethod(page);
    await expect(group.locator(".tree-subgroup[data-placement]").first()).toBeVisible();
    return name;
}

// The place picker (#1): pick the first listed method (an `[data-picker-archetype]`
// row, not the "+ create a new method" row).
async function pickFirstMethod(page: import("@playwright/test").Page) {
    await expect(page.locator("[data-place-picker]")).toBeVisible();
    await page.locator("[data-picker-archetype]").first().click();
}

// ---- navigation / setup ----

Given("the workbench is open", async ({ page }) => {
    await page.goto("/");
    // Chats is the default facet (WS-H): it gets `.active`.
    await expect(page.locator(".facet.active", { hasText: "Chats" })).toBeVisible();
});

// A new engagement is a usable WORK chat: a chat rooted on a placement (an
// archetype installed on a project), opened and ready to task.
Given("a new engagement", async ({ page }) => {
    await page.goto("/");
    const project = await placeArchetypeOnFreshProject(page);
    const group = page.locator(`.tree-group[data-project]`, { hasText: project });
    await group.locator(".tree-subgroup[data-placement] .action-row .create-btn").first().click();
    // selected → the chat-status badge carries the raw run phase as data-run-phase
    // (its visible text is the plain-language label).
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Init");
    // wait for the live SSE stream to connect before any task — the fake agent is
    // faster than the connection, so an early task would stream into the void.
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

// A new engagement whose archetype's policy blocks `bash`. We place an archetype
// on a fresh project, set the *placed* archetype's config to block bash (so the
// chat — seeded from the archetype config at creation — inherits the block), then
// open a work chat under that placement.
Given("a new engagement whose archetype blocks bash", async ({ page }) => {
    await page.goto("/");
    const project = await placeArchetypeOnFreshProject(page);
    const group = page.locator(`.tree-group[data-project]`, { hasText: project });
    const placement = group.locator(".tree-subgroup[data-placement]").first();
    // The placement's lineage carries the exact archetype id — configure *that*
    // archetype (by id), not by name (names can collide across the shared state).
    const archetypeId = await placement
        // The lineage id rides on the `data-lineage-archetype` attribute; round 6
        // moved it off a `.lineage` span onto `.node-label`, so match the attribute
        // itself, not the (reworded) class around it.
        .locator("[data-lineage-archetype]")
        .first()
        .getAttribute("data-lineage-archetype");

    // Open that archetype's settings from the Library and block bash.
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator(`.tree-group[data-archetype="${archetypeId}"]`)
        .locator(".tree-node.archetype")
        .first()
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^settings$/ }).click();
    await expect(page.locator("[data-config-editor]")).toBeVisible();
    // Block the shell via the plain form (round 5 #5): uncheck "let it run commands
    // on the computer" instead of hand-writing the block_tools JSON.
    await page.locator("[data-settings-shell]").uncheck();
    await page.locator("[data-settings-save]").click();
    await expect(page.locator("[data-config-status]").last()).toContainText("saved");
    await page.getByRole("button", { name: "close", exact: true }).click();

    // Now open a work chat under the placement — it seeds the blocking config.
    await page.locator(".facet", { hasText: "Projects" }).click();
    await group.locator(".tree-subgroup[data-placement] .action-row .create-btn").first().click();
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Init");
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

// ---- tasking the agent ----

When("I task the agent with {string}", async ({ page }, prompt: string) => {
    // Target the composer input structurally, not by placeholder: a work chat reads
    // "task the agent…" but an edit chat reads "Describe what to change about …", so
    // a placeholder match silently fails in edit chats (round10:6).
    await page.locator(".composer input").fill(prompt);
    await page.getByRole("button", { name: "send", exact: true }).click();
    // Fake agent returns instantly; a real (@live) turn can take ~20s. The turn's
    // completion shows on the chat-status badge reaching the terminal run phase.
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Completed", { timeout: 45_000 });
});

// ---- run lifecycle ----

Then("the run phase is {string}", async ({ page }, phase: string) => {
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", phase);
});

// ---- human task queue (top bar) ----

Then("the task bar shows a review", async ({ page }) => {
    await expect(page.locator("[data-testid=taskbar] [data-task]").first()).toBeVisible();
});

When("I complete the review from the task bar", async ({ page }) => {
    // the active task carries the keep control; the just-tasked chat is selected.
    // Keeping merges permanently, so the pill now arms on the first click and
    // commits on the second (round-6 #2 two-click guard).
    const keep = page.locator("[data-testid=taskbar] [data-task-keep]").first();
    await keep.click();
    await expect(keep).toHaveAttribute("data-arming", "1");
    await keep.click();
});

Then("the review is cleared from the task bar", async ({ page }) => {
    // The kept chat stays selected, so if its review were still pending it would
    // show the active keep control. No keep control ⇒ its review cleared. (The
    // queue is global, so other chats' pending reviews may legitimately remain.)
    await expect(page.locator("[data-testid=taskbar] [data-task-keep]")).toHaveCount(0);
});

Then("the review task carries its archetype tag", async ({ page }) => {
    // #22: the tab exposes its owning archetype as data-task-agent; the accent
    // colour is keyed off it. Asserting the attribute is present proves the colour
    // has a basis.
    await expect(page.locator("[data-testid=taskbar] [data-task-agent]").first()).toBeVisible();
});

// Only tool lines with additive detail (a command's full text / output, a file's
// contents) are expandable now; a file write is a tight non-expandable one-liner.
// So target the first *expandable* tool line, not merely the first tool line.
When("I expand the first tool line", async ({ page }) => {
    await page.locator('[data-testid="tool-line"] .tool-head.expandable').first().click();
});

Then("the first tool line is expanded", async ({ page }) => {
    await expect(
        page.locator('[data-testid="tool-line"] .tool-head.expandable .tool-caret').first(),
    ).toHaveText("▾");
});

// Round-8 #4: the expanded detail must be a plain sentence, never the raw
// `{"path":…}` JSON arg blob. The raw args are preserved on data-tool-args (for
// automation) but the visible text must not start with a "{".
Then("the expanded tool detail reads in plain language", async ({ page }) => {
    const detail = page.locator('[data-testid="tool-line"] .tool-detail-line').first();
    await expect(detail).toBeVisible();
    const text = (await detail.innerText()).trim();
    expect(text.startsWith("{")).toBe(false);
    expect(text).not.toMatch(/"path"|"command"/);
});

// ---- transcript ----

Then("the transcript shows {string}", async ({ page }, text: string) => {
    // A phrase may legitimately appear in more than one line — assert ≥1. Lifecycle
    // lines are shown in plain language but keep their raw phase on `data-line-text`,
    // so match either the visible text or the underlying raw line text.
    const byText = page.locator(".run .line", { hasText: text });
    const byRaw = page.locator(`.run .line[data-line-text="${text}"]`);
    await expect(byText.or(byRaw).first()).toBeVisible();
});

Then("a mediated tool line is shown", async ({ page }) => {
    // A boundary-mediated effect renders as a B4 tool line `▸ {tool} …` (run-chat.md);
    // blocked effects render separately. Its presence is the mediation made visible.
    await expect(page.locator('.run [data-testid="tool-line"]').first()).toBeVisible();
});

// ---- chat-log grouping + filter (view state over the transcript) ----

When("I collapse the first turn", async ({ page }) => {
    await page.locator('.run [data-testid="turn"] .turn-head').first().click();
});

Then("the first turn is collapsed", async ({ page }) => {
    const turn = page.locator('.run [data-testid="turn"]').first();
    await expect(turn).toHaveClass(/collapsed/);
    // The fold leaves a one-line gist in place of the turn body.
    await expect(turn.locator(".turn-summary")).toBeVisible();
});

Then("no tool line is shown", async ({ page }) => {
    await expect(page.locator('.run [data-testid="tool-line"]')).toHaveCount(0);
});

Then("no {string} tool line is shown", async ({ page }, category: string) => {
    await expect(page.locator(`.run [data-tool-category="${category}"]`)).toHaveCount(0);
});

When("I hide {string} tool calls from the chat log", async ({ page }, category: string) => {
    await page.locator("[data-transcript-filter]").click();
    await page.locator(`[data-filter-visible="${category}"]`).uncheck();
    // Dismiss the popover so the log is unobstructed for the following assertions.
    await page.locator(".popover-catcher").click();
});

Then("the chat log does not show {string}", async ({ page }, text: string) => {
    await expect(page.locator(".run .transcript")).not.toContainText(text);
});

When("I save the filter as default", async ({ page }) => {
    await page.locator("[data-transcript-filter]").click();
    await page.locator("[data-filter-save]").click();
    // The button acknowledges the persist before we dismiss the menu.
    await expect(page.locator("[data-filter-save]")).toHaveText(/Saved/);
    await page.locator(".popover-catcher").click();
});

When("I click the tool target {string}", async ({ page }, target: string) => {
    await page.locator(".run .tool-target", { hasText: target }).first().click();
});

Then("the content viewer shows {string}", async ({ page }, file: string) => {
    // Selecting a file only auto-switches to View when there's no pending review;
    // after a turn the viewer defaults to Changes (round-7 #3), so open View
    // explicitly rather than racing on whether the merge phase has loaded.
    await page.locator('[data-viewer-tabs] .tab[data-tab="view"]').click();
    await expect(page.locator("[data-file-view]")).toBeVisible();
    await expect(page.locator(".panel.content", { hasText: file })).toBeVisible();
});

// ---- shell / facets ----

Then("the facet {string} is active", async ({ page }, label: string) => {
    await expect(page.locator(".facet.active", { hasText: label })).toBeVisible();
});

Then("the facet {string} is present", async ({ page }, label: string) => {
    await expect(page.locator(".facet", { hasText: label })).toBeVisible();
});

When("I switch to the {string} facet", async ({ page }, label: string) => {
    await page.locator(".facet", { hasText: label }).click();
    await expect(page.locator(".facet.active", { hasText: label })).toBeVisible();
});

When("I search the facets for {string}", async ({ page }, q: string) => {
    await page.locator('[data-testid="facet-search"]').fill(q);
});

When("I clear the facet search", async ({ page }) => {
    await page.locator('[data-testid="facet-search"]').fill("");
});

// Content search (SEARCH-1): a chat whose transcript matches surfaces in the tree
// carrying a snippet of the hit, even when its title does not match the query.
Then("a chat surfaces with a content snippet", async ({ page }) => {
    const snippet = page.locator("[data-snippet]").first();
    await expect(snippet).toBeVisible();
    // The snippet sits inside a real chat row (it is why the row stayed).
    await expect(snippet.locator("xpath=ancestor::*[contains(@class,'chat-item')]")).toBeVisible();
});

Then("the archetype {string} is hidden", async ({ page }, name: string) => {
    await expect(
        page.locator("[data-archetype] .node-label", { hasText: new RegExp(`^${name}$`) }),
    ).toHaveCount(0);
});

// ---- the archetype / project library (facet browser CRUD) ----

// Archetypes live in the Library facet.
Then("I see the archetype {string}", async ({ page }, name: string) => {
    await expect(
        page.locator("[data-archetype] .node-label", { hasText: new RegExp(`^${name}$`) }),
    ).toBeVisible();
});

Then("I see the project {string}", async ({ page }, name: string) => {
    await expect(page.locator("[data-project] .node-label", { hasText: new RegExp(`^${name}$`) })).toBeVisible();
});

When("I create an archetype named {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page.getByText("+ archetype", { exact: true }).click();
    await page.locator(".inline-edit").fill(name);
    await page.locator(".inline-edit").press("Enter");
    await expect(
        page.locator("[data-archetype] .node-label", { hasText: new RegExp(`^${name}$`) }),
    ).toBeVisible();
});

When("I create a project named {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page.getByText("+ project", { exact: true }).click();
    await page.locator(".inline-edit").fill(name);
    await page.locator(".inline-edit").press("Enter");
});

// An EDIT chat is rooted on an archetype — opened from its right-click menu ("edit").
When("I add an edit chat under the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "edit" }).click();
});

// Place an archetype onto a project → a placement (the old "bind").
When("I place an archetype on the project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator("[data-project]", { hasText: name })
        .locator(".tree-node.project")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "add an archetype" }).click();
    await pickFirstMethod(page);
    await expect(
        page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup[data-placement]").first(),
    ).toBeVisible();
});

// A WORK chat is rooted on a placement (do the job).
When("I add a chat under the placement", async ({ page }) => {
    await page.locator("[data-placement]").first().getByRole("button", { name: "+ chat" }).click();
});

// The All-chats "just start typing" quick-start: roots on the hidden Personal
// default placement (ADR 0036), no project/method setup. Opens + selects the chat.
When("I start a new chat from All chats", async ({ page }) => {
    await page.getByTestId("new-default-chat").click();
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

// The just-created chat (active), rooted on a placement ⇒ a work chat (ADR 0035).
Then("the active chat is a work chat", async ({ page }) => {
    await expect(page.locator('[data-chat].active[data-kind="work"]')).toBeVisible();
});

Then("I see a chat in All chats", async ({ page }) => {
    await expect(page.locator(".facet.active", { hasText: "Chats" })).toBeVisible();
    await expect(page.locator(".chat-item").first()).toBeVisible();
});

// WS-H: start a workstream from a chat row. Right-click the chat → "new workstream" →
// name it. The control plane creates the line on the chat's own placement and joins it.
When("I create a workstream named {string} from that chat", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Chats" }).click();
    await page.locator(".chat-item").first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: "new workstream" }).click();
    await page.locator(".inline-edit").fill(name);
    await page.locator(".inline-edit").press("Enter");
});

// The chat is now a member of the named line: it renders inside that workstream's
// group (a non-empty, visible group — the bug this guards against was the create
// landing silently with nothing shown).
Then("the chat is on the workstream {string}", async ({ page }, name: string) => {
    const group = page.locator(".ws-group", {
        has: page.locator(".ws-label-name", { hasText: new RegExp(`^${name}$`) }),
    });
    await expect(group).toBeVisible();
    await expect(group.locator(".ws-members .chat-item").first()).toBeVisible();
});

// Start a chat directly in a project (WS-H): the project's built-in general placement
// is hidden, so a project-level "+ new chat" opens a work chat with no archetype choice.
When("I start a chat in project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page.locator("[data-project]", { hasText: name }).locator("[data-create='new-project-chat']").click();
});

// The chat appears directly under the project (in its general home), not under any
// placement node — proving the default placement is invisible plumbing.
Then("project {string} shows a chat", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(
        page.locator("[data-project]", { hasText: name }).locator("[data-project-home] .chat-item").first(),
    ).toBeVisible();
});

// Leave the line via the chat's own context menu (WS-F): the chat returns to the
// placement mainline; the now-empty line stays for others to join.
When("I remove that chat from its workstream", async ({ page }) => {
    await page.locator(".ws-group .ws-members .chat-item").first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: "leave workstream" }).click();
});

// An empty line is still shown — proving the create/leave landed and the line is
// joinable, not broken — but its member list is empty (no hint message; WS-H).
Then("the workstream {string} shows it has no chats yet", async ({ page }, name: string) => {
    const group = page.locator(".ws-group", {
        has: page.locator(".ws-label-name", { hasText: new RegExp(`^${name}$`) }),
    });
    await expect(group).toBeVisible();
    await expect(group.locator(".ws-members .chat-item")).toHaveCount(0);
});

// Archive the line via its label's context menu (WS-F): it re-homes members and is
// gone from the nav (only active lines render as groups). Archive is destructive, so
// the menu arms on the first click and runs on the confirming second (like delete).
When("I archive the workstream {string}", async ({ page }, name: string) => {
    await page.locator(".ws-label", { hasText: new RegExp(name) }).first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^archive$/ }).click(); // arm
    await page.locator(".menu-item.confirming").click(); // confirm
});

Then("there is no workstream {string}", async ({ page }, name: string) => {
    await expect(
        page.locator(".ws-group", { has: page.locator(".ws-label-name", { hasText: new RegExp(`^${name}$`) }) }),
    ).toHaveCount(0);
});

// Promote the line into the placement mainline via its label menu (WS-F). Not
// destructive (no confirm) — a single click runs it; the toast reports the result.
When("I promote the workstream {string}", async ({ page }, name: string) => {
    await page.locator(".ws-label", { hasText: new RegExp(name) }).first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: "promote into mainline" }).click();
});

// Join the most-recently-created (ungrouped, so last in document order) chat to an
// existing line via its context menu (WS-H join from the Chats facet).
When("I add the latest chat to the workstream {string}", async ({ page }, name: string) => {
    await page.locator(".chat-item").last().click({ button: "right" });
    await page.locator(".menu-item", { hasText: `join "${name}"` }).click();
});

Then("the workstream {string} has {int} chats", async ({ page }, name: string, n: number) => {
    const group = page.locator(".ws-group", {
        has: page.locator(".ws-label-name", { hasText: new RegExp(`^${name}$`) }),
    });
    await expect(group.locator(".ws-members .chat-item")).toHaveCount(n);
});

Then("the archetype {string} is gone", async ({ page }, name: string) => {
    await expect(
        page.locator("[data-archetype] .node-label", { hasText: new RegExp(`^${name}$`) }),
    ).toHaveCount(0);
});

When("I delete the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^delete$/ }).click(); // arm inline confirm
    await page.locator(".menu-item.confirming").click(); // confirm
});

// Tree collapse/expand (the node ▾/▸ icon).
When("I collapse the project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page.locator("[data-project]", { hasText: name }).locator(".tree-node.project .node-icon").first().click();
});

When("I add a work chat in project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup[data-placement] .action-row .create-btn").first().click();
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Init");
});

Then("the placement in project {string} shows a chat", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup[data-placement] .chat-item").first()).toBeVisible();
});

// Per-placement config-only customization (placement.md): right-click the placement →
// customize → set notes → save. Tweaks this client's placement without forking.
When("I customize the placement in project {string} with notes {string}", async ({ page }, project: string, notes: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator("[data-project]", { hasText: project })
        .locator(".tree-subgroup[data-placement] .tree-node.placement")
        .first()
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "customize" }).click();
    await page.locator("[data-cfg-notes]").fill(notes);
    await page.locator("[data-cfg-save]").click();
});

Then("the placement in project {string} shows it is customized", async ({ page }, project: string) => {
    await expect(
        page.locator("[data-project]", { hasText: project }).locator("[data-placement-customized]").first(),
    ).toBeVisible();
});

// Fork lineage (ADR 0038): the fork carries a "forked from <source>" line.
Then("an archetype is forked from {string}", async ({ page }, source: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await expect(page.locator(".fork-lineage", { hasText: `forked from ${source}` })).toBeVisible();
});

// Pull the source's improvements into the fork (the archetype carrying that lineage).
When("I pull updates into the fork of {string}", async ({ page }, source: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { has: page.locator(".fork-lineage", { hasText: `forked from ${source}` }) })
        .locator(".tree-node.archetype")
        .first()
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "pull updates from source" }).click();
});

When("I collapse the placement in project {string}", async ({ page }, name: string) => {
    await page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup[data-placement] .tree-node.placement .node-icon").first().click();
});

Then("the placement in project {string} hides its chats", async ({ page }, name: string) => {
    await expect(page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup[data-placement] .chat-item")).toHaveCount(0);
});

Then("the project {string} hides its placements", async ({ page }, name: string) => {
    await expect(page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup")).toHaveCount(0);
});

// C1 many-to-many: placing one archetype on N projects must REUSE it, not clone it —
// so the Library still lists a single archetype after two placements.
Then("the Library lists {int} archetype", async ({ page }, n: number) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await expect(page.locator("[data-archetype]")).toHaveCount(n);
});

Then("the project {string} shows its placements", async ({ page }, name: string) => {
    // The placement may have been made from the *Library* side (the reverse "place
    // on a project…" flow leaves you on Library); the project tree lives under the
    // Projects facet, so pivot there before reading it.
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(page.locator("[data-project]", { hasText: name }).locator(".tree-subgroup").first()).toBeVisible();
});

// Fork (ADR 0035/0038).
When("I fork the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .first()
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^fork$/ }).click();
});

Then("I see a forked copy of the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await expect(page.locator("[data-archetype] .node-label", { hasText: `${name} (fork)` }).first()).toBeVisible();
});

When("I fork the first chat", async ({ page }) => {
    await page.locator(".chat-item").first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^fork$/ }).click();
});

Then("I see a forked chat", async ({ page }) => {
    await expect(page.locator(".chat-item", { hasText: "(fork)" }).first()).toBeVisible();
});

// Round-8 #3: the forked row carries a quiet "copy of {source}" sublabel so its
// lineage is legible (it's not just a coincidental name-twin of its source).
Then("the forked chat shows it is a copy of its source", async ({ page }) => {
    const sub = page.locator(".chat-item", { hasText: "(fork)" }).first().locator(".leaf-sub");
    await expect(sub).toBeVisible();
    await expect(sub).toContainText("copy of");
});

// Round-8 #3: opening the fork shows a first-view note explaining the
// copy-semantics — files came along, the conversation starts fresh.
Then(
    "the chat shows it started as a copy with files but a fresh conversation",
    async ({ page }) => {
        await page.locator(".chat-item", { hasText: "(fork)" }).first().click();
        const note = page.locator("[data-fork-note]");
        await expect(note).toBeVisible();
        await expect(note).toContainText("files came along");
        await expect(note).toContainText("conversation starts fresh");
    },
);

// ---- edit vs work chats (chat rooting, ADR 0035) ----

// An edit chat is created under an archetype, via its context menu.
When("I create an edit chat under the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "edit" }).click();
});

// The chat's kind (its root) is shown read-only via the lineage header; there is
// no toggle and no composer caption. `data-kind` carries the kind ("edit"|"work").
Then("the chat pane kind is {string}", async ({ page }, kind: string) => {
    await expect(page.locator("[data-chat-lineage]")).toHaveAttribute("data-kind", kind);
});

Then("an edit chat is marked in the nav", async ({ page }) => {
    // The just-created chat (active), not the first edit chat in an accumulated tree.
    await expect(page.locator('[data-chat].active[data-kind="edit"]')).toBeVisible();
});

When("I reopen the chat", async ({ page }) => {
    await page.locator("[data-chat]").first().click();
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

When("I reload the workbench", async ({ page }) => {
    await page.reload();
    await expect(page.locator(".facet.active", { hasText: "Chats" })).toBeVisible();
});

// ---- content viewer (view / edit / diff) ----

When("I select the file {string} in the workspace", async ({ page }, file: string) => {
    await page.locator("[data-worktree] .file", { hasText: file }).click();
});

When("I replace the editor content with {string}", async ({ page }, text: string) => {
    await page.locator("[data-file-edit]").fill(text);
});

When("I save the file", async ({ page }) => {
    await page.locator("[data-file-save]").click();
    await expect(page.locator("[data-edit-status]")).toHaveText("saved");
});

Then("the file view shows {string}", async ({ page }, text: string) => {
    await expect(page.locator("[data-file-view]")).toContainText(text);
});

When("I open the {string} tab", async ({ page }, tab: string) => {
    // The visible label may be plain language ("changes" for the diff), so match
    // the stable data-tab mode value the feature passes (view/edit/diff).
    await page.locator(`[data-viewer-tabs] .tab[data-tab="${tab}"]`).click();
});

Then("the diff shows {string}", async ({ page }, text: string) => {
    await expect(page.locator(".diff", { hasText: text })).toBeVisible();
});

// The merge review: the human admits the turn's diff (advance main).
When("I keep the work", async ({ page }) => {
    await page.locator("[data-merge-admit]").click();
});

// ---- the review/audit shelf (an overlay surface) ----

When("I open the review shelf", async ({ page }) => {
    // The raw review/export state machine is engine-authoring material, gated
    // behind developer mode (round-6 #1). Tests of that lifecycle opt into dev
    // mode; the Shelf reads the flag fresh when it mounts, so no reload is needed.
    await page.evaluate(() => localStorage.setItem("ui.dev", "1"));
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.locator('.shelf-drawer .tab[data-tab="review"]').click();
});

When("I open the audit shelf", async ({ page }) => {
    await page.getByRole("button", { name: "History", exact: true }).click();
    // In the user-facing build the Activity list is the only thing in the drawer
    // (no tabs); the tab only exists under dev mode. Click it only if present.
    const auditTab = page.locator('.shelf-drawer .tab[data-tab="audit"]');
    if (await auditTab.count()) await auditTab.click();
});

// ---- review shelf ----

When("I propose review", async ({ page }) => {
    await page.getByTestId("review-propose").click();
});

When("the stakeholder {string} consents to review", async ({ page }, who: string) => {
    await page.getByTestId(`review-consent-${who}`).click();
});

When("I release the review", async ({ page }) => {
    await page.getByTestId("review-release").click();
});

Then("the review phase is {string}", async ({ page }, phase: string) => {
    await expect(page.locator("[data-review-phase]")).toHaveText(phase);
});

// ---- export gating ----

When("I propose export", async ({ page }) => {
    await page.getByTestId("export-propose").click();
});

When("the source {string} consents to export", async ({ page }, who: string) => {
    await page.getByTestId(`export-source-${who}`).click();
});

When("the target admits the export", async ({ page }) => {
    await page.getByTestId("export-target-admit").click();
});

When("I export", async ({ page }) => {
    await page.getByTestId("export-export").click();
});

Then("the export phase is {string}", async ({ page }, phase: string) => {
    await expect(page.locator("[data-export-phase]")).toHaveText(phase);
});

// ---- audit ----

Then("the audit timeline shows {string}", async ({ page }, text: string) => {
    // Raw engine event names (RunRequested, …) live behind the developer "raw
    // event log" toggle now (round-6 #1); the default view is plain-language
    // activity. The toggle only renders once the audit events have loaded, so wait
    // for it rather than a one-shot count() that can read 0 mid-fetch and skip the
    // reveal (the same race that bit config:13). Reveal only if not already shown.
    const toggle = page.locator("[data-raw-log-toggle]");
    await toggle.waitFor();
    if (!(await page.locator("[data-raw-log]").count())) await toggle.click();
    await expect(page.locator("[data-raw-log] .event", { hasText: text })).toBeVisible();
});

// ---- archetype settings (config) ----

When("I open the config editor", async ({ page }) => {
    // Settings live on the archetype now (ADR 0035): Library → right-click the
    // default archetype → settings.
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: "assistant" })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^settings$/ }).click();
    await expect(page.locator("[data-config-editor]")).toBeVisible();
});

When("I set the config to {string}", async ({ page }, json: string) => {
    // The raw settings text is now a collapsed "Advanced" surface (round 5 #5):
    // the plain form leads. Reveal Advanced, then edit the JSON. Click the toggle
    // directly (it auto-waits) rather than a one-shot isVisible() check, which can
    // read false before the modal paints and skip the reveal (config:13 race).
    await page.locator("[data-settings-advanced-toggle]").click();
    await page.locator("[data-config-text]").fill(json);
    await page.locator("[data-settings-save]").click();
});

Then("the config status shows {string}", async ({ page }, text: string) => {
    await expect(page.locator("[data-config-status]").last()).toContainText(text);
});

When("I close the config editor", async ({ page }) => {
    await page.getByRole("button", { name: "close", exact: true }).click();
    await expect(page.locator("[data-config-editor]")).toBeHidden();
});

// ---- context ingestion ----

When("I attach the context folder {string}", async ({ page }, path: string) => {
    await page.getByRole("button", { name: "Add files", exact: true }).click();
    // Use the stable data hooks, not the placeholder/label copy: the fallback's
    // wording is user-facing prose that gets reworded (it became "paste a folder
    // location…"), so binding to text makes the step brittle.
    await page.locator("[data-context-path]").fill(path);
    await page.locator("[data-context-attach]").click();
});

// ---- message attachments (composer paperclip, UX-14) ----

// Clip a file to the message being composed. The paperclip opens a plain file
// input ([data-attach-input]); its text is read client-side and rides the next
// prompt — no backend ingest. setInputFiles hands an in-memory file to that input.
When(
    "I attach the file {string} containing {string}",
    async ({ page }, name: string, content: string) => {
        await page.locator("[data-attach-input]").setInputFiles({
            name,
            mimeType: "text/plain",
            buffer: Buffer.from(content),
        });
        await expect(page.locator("[data-attachment]", { hasText: name })).toBeVisible();
    },
);

// A 1x1 transparent PNG — enough for the client to classify + base64 + chip it.
const TINY_PNG = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+M8AAAMBAQDJ/pLvAAAAAElFTkSuQmCC",
    "base64",
);

When("I attach a PNG image {string}", async ({ page }, name: string) => {
    await page.locator("[data-attach-input]").setInputFiles({
        name,
        mimeType: "image/png",
        buffer: TINY_PNG,
    });
});

Then("the composer shows an image attachment {string}", async ({ page }, name: string) => {
    await expect(page.locator(`[data-attachment][data-kind="image"]`, { hasText: name })).toBeVisible();
});

// PDF/Office aren't supported yet — attaching one is refused with a status message,
// nothing is added to the composer.
When(
    "I attach an unsupported file {string} of type {string}",
    async ({ page }, name: string, mimeType: string) => {
        await page.locator("[data-attach-input]").setInputFiles({
            name,
            mimeType,
            buffer: Buffer.from("%PDF-1.4 not really"),
        });
    },
);

Then("the composer has no pending attachments", async ({ page }) => {
    await expect(page.locator("[data-attachment]")).toHaveCount(0);
});

// ---- send queue & steering (composer dock) ----

// Start a turn without waiting for it to settle, so the queue can be exercised
// while the agent is busy. Pair with a `[slow]` prompt to widen that window.
When("I start tasking the agent with {string}", async ({ page }, prompt: string) => {
    await page.getByPlaceholder("task the agent…").fill(prompt);
    await page.getByRole("button", { name: "send", exact: true }).click();
});

Then("the agent is working", async ({ page }) => {
    await expect(page.getByTestId("agent-working")).toBeVisible();
});

// Type into the composer while busy and queue it (Enter == queue mid-turn).
When("I queue the message {string}", async ({ page }, text: string) => {
    await page.getByPlaceholder("task the agent…").fill(text);
    await page.getByPlaceholder("task the agent…").press("Enter");
    await expect(page.getByTestId("queue-item").filter({ hasText: text })).toBeVisible();
});

// Stop a running turn (run-chat.md): abort, the composer re-enables.
When("I stop the turn", async ({ page }) => {
    await page.getByTestId("stop-turn").click();
});
Then("the composer is ready to send again", async ({ page }) => {
    await expect(page.getByRole("button", { name: "send", exact: true })).toBeVisible();
});
// Panel header labels.
Then("the run pane is labelled {string}", async ({ page }, label) => {
    await expect(page.locator(".panel.run h2")).toContainText(label);
});
Then("the workspace pane is labelled {string}", async ({ page }, label) => {
    await expect(page.locator(".panel.workspace h2")).toContainText(label);
});

// Panel collapse (legacy 13-chrome-ui.md §6). Steps refer to a panel by its
// visible title; the grid class is the stable hook the UI exposes.
const PANEL_CLASS: Record<string, string> = { Browse: "nav", Chat: "run", Content: "content", Files: "workspace" };
function panelClass(title: string): string {
    const cls = PANEL_CLASS[title];
    if (!cls) throw new Error(`unknown panel "${title}"`);
    return cls;
}
When("I collapse the {string} panel", async ({ page }, title: string) => {
    await page.locator(`[data-collapse="${panelClass(title)}"]`).click();
});
When("I expand the {string} panel", async ({ page }, title: string) => {
    await page.locator(`[data-rail="${panelClass(title)}"]`).click();
});
Then("the {string} panel is folded", async ({ page }, title: string) => {
    const cls = panelClass(title);
    await expect(page.locator(`[data-rail="${cls}"]`)).toBeVisible();
    await expect(page.locator(`[data-collapse="${cls}"]`)).toHaveCount(0);
});
Then("the {string} panel is open", async ({ page }, title: string) => {
    const cls = panelClass(title);
    await expect(page.locator(`[data-collapse="${cls}"]`)).toBeVisible();
    await expect(page.locator(`[data-rail="${cls}"]`)).toHaveCount(0);
});

// The stage-gate (#24): stage messages without running them, then release. The
// control is a single inline chip left of `send` (⏸ hold / ▶ release) — always
// present when idle, no separate disclosure to reveal first.
When("I enable the stage-gate", async ({ page }) => {
    await page.locator("[data-queue-gate]").click();
    await expect(page.locator("[data-queue-gate].gated")).toBeVisible();
});
When("I release the stage-gate", async ({ page }) => {
    await page.locator("[data-queue-gate]").click();
});

// Steer: send now, interrupting the running turn (front of the queue).
When("I steer with {string}", async ({ page }, text: string) => {
    await page.getByPlaceholder("task the agent…").fill(text);
    await page.getByTestId("steer-turn").click();
});

Then(/^the queue shows (\d+) messages?$/, async ({ page }, n: string) => {
    await expect(page.getByTestId("queue-item")).toHaveCount(Number(n));
});

Then("queued message {int} is {string}", async ({ page }, pos: number, text: string) => {
    await expect(page.getByTestId("queue-item").nth(pos - 1)).toContainText(text);
});

When("I cancel the queued message {string}", async ({ page }, text: string) => {
    await page.getByTestId("queue-item").filter({ hasText: text }).locator(".queue-remove").click();
    await expect(page.getByTestId("queue-item").filter({ hasText: text })).toHaveCount(0);
});

When("I send now the queued message {string}", async ({ page }, text: string) => {
    await page.getByTestId("queue-item").filter({ hasText: text }).getByTestId("queue-send-now").click();
});

When("I edit the queued message {string} to {string}", async ({ page }, from: string, to: string) => {
    await page.getByTestId("queue-item").filter({ hasText: from }).locator(".queue-text").click();
    const edit = page.locator(".queue-edit");
    await edit.fill(to);
    await edit.press("Enter");
    await expect(page.getByTestId("queue-item").filter({ hasText: to })).toBeVisible();
});

When("I drag queued message {string} above {string}", async ({ page }, from: string, to: string) => {
    const src = page.getByTestId("queue-item").filter({ hasText: from }).locator(".queue-grip");
    const dst = page.getByTestId("queue-item").filter({ hasText: to });
    await src.dragTo(dst, { targetPosition: { x: 8, y: 2 } });
});

// The queue has drained and the agent is idle again.
Then("the agent finishes", async ({ page }) => {
    await expect(page.getByTestId("queue-stack")).toHaveCount(0, { timeout: 45_000 });
    await expect(page.getByTestId("agent-working")).toBeHidden({ timeout: 45_000 });
});

// The current turn has settled, without requiring the queue to be empty (a held
// queue can still hold messages after one is sent now).
Then("the agent is idle", async ({ page }) => {
    await expect(page.getByTestId("agent-working")).toBeHidden({ timeout: 45_000 });
});

// ---- round-1: on-ramps & plain language ----

// The chat-status badge (#4): a real coloured pill whose visible text is plain.
Then("the chat status badge reads {string}", async ({ page }, label: string) => {
    await expect(page.getByTestId("run-phase")).toContainText(label);
});

// Generic: a visible button with the given (plain-language) label exists.
Then("I see the button {string}", async ({ page }, label: string) => {
    await expect(page.getByRole("button", { name: label, exact: true })).toBeVisible();
});

// ---- the place picker (#1, round 2) ----

// Open the picker from the project's context menu. (A project is never empty now —
// it gets a default placement at creation — so "add an archetype" lives on the menu,
// not an empty-state CTA: it adds a *further* archetype alongside the default.)
When("I open the add-method picker for project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator("[data-project]", { hasText: name })
        .locator(".tree-node.project")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "add an archetype" }).click();
});

Then("the place picker is open", async ({ page }) => {
    await expect(page.locator("[data-place-picker]")).toBeVisible();
});

When("I choose the first method in the picker", async ({ page }) => {
    await pickFirstMethod(page);
});

// Use an archetype with no placement (ADR 0045): from the Library, its menu opens a
// work chat in the hidden Personal project directly — no place picker.
When("I use the archetype {string} from its menu", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    // The label lives in `.menu-item-label`; the `.menu-item` text also carries the
    // hint, so anchor on the label span (the click bubbles to the item).
    // The Library menu label is "test" (run the method to try it out) — same action
    // as the old "use": opens a work chat in the hidden Personal project.
    await page.locator(".menu-item-label", { hasText: /^test$/ }).click();
});

Then("a work chat opens", async ({ page }) => {
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Init");
    await expect(page.locator('[data-chat-lineage][data-kind="work"]')).toBeVisible();
});

// ---- auto-titling a new chat (#4, round 2) ----

Then("a chat titled {string} appears in the nav", async ({ page }, title: string) => {
    // The auto-titled chat is the one we just tasked (active); scope to it so an
    // identically-titled chat from another scenario can't mask a real failure.
    await expect(page.locator("[data-chat].active .leaf-label", { hasText: title })).toBeVisible();
});

// ---- the hold (stage) control (#24): a single inline chip beside send ----

Then("I can hold messages before running", async ({ page }) => {
    await expect(page.locator("[data-queue-gate]")).toBeVisible();
});

// ---- round 3: honest discard, settings link, grouped all-chats ----

// Discard the reviewed change (#1). The change must be reviewable (phase Clean).
When("I discard the work", async ({ page }) => {
    await page.locator("[data-merge-reject]").click();
});

// The honest end-state (#1): one plain "discarded" message, no "couldn't be kept"
// blame, and a clear "it's gone" state instead of a stale diff to keep again.
Then("the changes show an honest discarded state", async ({ page }) => {
    await expect(page.locator("[data-merge-phase]")).toContainText("discarded");
    await expect(page.locator(".merge-review")).not.toContainText("couldn't be kept");
    await expect(page.locator(".discarded-note")).toBeVisible();
});

// After discarding, the only follow-up is to start over — not "fix it up" (#1).
Then("I am offered to start over, not to fix it up", async ({ page }) => {
    await expect(page.getByRole("button", { name: "start over", exact: true })).toBeVisible();
    await expect(page.locator(".merge-review")).not.toContainText("fix it up");
});

// An archetype's settings open from its right-click menu (the empty-state hint
// link was removed — settings/edit now live in the context menu).
When("I click the settings link on the method {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: /^settings$/ }).click();
});

Then("the method settings modal is open", async ({ page }) => {
    await expect(page.locator("[data-config-editor]")).toBeVisible();
});

// All chats groups by archetype rather than repeating the label per row (#5).
Then("all chats are grouped under the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Chats" }).click();
    await expect(page.locator(`.chat-group[data-chat-group="${name}"]`).first()).toBeVisible();
});

// Un-started chats render as distinct "Untitled · N" rows, never two identical ones (#5).
Then("no two un-started chats read identically", async ({ page }) => {
    await page.locator(".facet", { hasText: "Chats" }).click();
    const untitled = page.locator("[data-chat] .leaf-label", { hasText: /^Untitled/ });
    const n = await untitled.count();
    const seen = new Set<string>();
    for (let i = 0; i < n; i++) {
        const t = (await untitled.nth(i).textContent())?.trim() ?? "";
        expect(seen.has(t)).toBe(false);
        seen.add(t);
    }
});

// The decorative "v0" version badge is gone (#6).
Then("placements carry no version badge", async ({ page }) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await expect(page.locator(".pinned-version")).toHaveCount(0);
});

// ---- round 4: config-edit safety, legible diffs, one canonical title ----

// The Files panel hides internal/config dotfiles behind a quiet toggle (#6 round-2);
// revealing them lets the test reach .agent-config.json directly.
When("I reveal the internal files", async ({ page }) => {
    await page.locator("[data-show-internal]").click();
});

// Editing the assistant's settings file directly is flagged as a safety surface (#1).
Then("the editor warns that this is the assistant's settings file", async ({ page }) => {
    await expect(page.locator("[data-config-edit-warning]")).toBeVisible();
});

// Saving must not assert "saved" — an invalid config save is rejected, not committed.
When("I try to save the file", async ({ page }) => {
    await page.locator("[data-file-save]").click();
});

// Corrupt config JSON is blocked with a plain sentence, never persisted (#1).
Then("the save is rejected with a plain-language message", async ({ page }) => {
    await expect(page.locator("[data-edit-status]")).toContainText("isn't valid");
    await expect(page.locator("[data-edit-status]")).not.toHaveText("saved");
});

// Split is illegible at the Content panel's default width, so it isn't offered (#2).
Then("the split diff toggle is not offered at the default panel width", async ({ page }) => {
    await expect(page.locator(".diff-toolbar")).toBeVisible();
    await expect(page.locator(".diff-mode")).toHaveCount(0);
});

// The TASKS bar shows one canonical title, never the raw "new chat" placeholder (#4).
Then("the task bar shows no chat literally titled {string}", async ({ page }, title: string) => {
    await expect(
        page.locator("[data-testid=taskbar] .task-title", { hasText: new RegExp(`^${title}$`) }),
    ).toHaveCount(0);
});

// ---- round 5 ----

// After a discard, the View tab must stop being silently contradictory (#1, the
// deepest honest-feedback violation: Changes said "thrown away" while View showed
// it present). Discard *isolates* the work (the backend keeps the engagement's
// files), so View honestly still shows the text — but it now says so explicitly
// rather than rendering it as if nothing happened.
Then("the View tab explains the discarded changes are still on the private copy", async ({ page }) => {
    await expect(page.locator("[data-view-discarded]")).toBeVisible();
    await expect(page.locator("[data-view-discarded]")).toContainText("won't be kept");
});

// The tree rows are real treeitems a keyboard/SR user can reach and activate (#4).
// Target the chat we just opened (the active row), not the first chat anywhere —
// the shared serial control plane accumulates older chats ahead of it, and opening
// one of those would land on a *completed* run rather than the fresh "Init" chat.
Then("the chat rows are keyboard-reachable", async ({ page }) => {
    const row = page.locator('[data-chat].active[role="treeitem"]');
    await expect(row).toHaveAttribute("tabindex", "0");
});

When("I open a chat by keyboard", async ({ page }) => {
    const row = page.locator('[data-chat].active[role="treeitem"]');
    await row.focus();
    await row.press("Enter");
});

// The settings modal leads with a plain-language form, with the raw JSON demoted (#5).
Then("the settings modal shows a plain-language form", async ({ page }) => {
    await expect(page.locator("[data-settings-form]")).toBeVisible();
    await expect(page.locator("[data-settings-posture=ask]")).toBeVisible();
    // The raw JSON is hidden until Advanced is expanded.
    await expect(page.locator("[data-config-text]")).toHaveCount(0);
});

When("I expand the advanced settings", async ({ page }) => {
    await page.locator("[data-settings-advanced-toggle]").click();
});

Then("the raw settings text is shown", async ({ page }) => {
    await expect(page.locator("[data-config-text]")).toBeVisible();
});

// Escape closes the settings modal (#6).
When("I press Escape", async ({ page }) => {
    await page.keyboard.press("Escape");
});

Then("the settings modal is closed", async ({ page }) => {
    await expect(page.locator("[data-config-editor]")).toBeHidden();
});

// Search has a clear control that resets the filter (#6).
When("I type {string} in the search box", async ({ page }, q: string) => {
    await page.getByTestId("facet-search").fill(q);
});

When("I clear the search", async ({ page }) => {
    await page.getByTestId("facet-search-clear").click();
});

Then("the search box is empty", async ({ page }) => {
    await expect(page.getByTestId("facet-search")).toHaveValue("");
});

// ---- round 6: plain-language history, keep guard, "method" wording ----

// The history "Activity" tab folds the raw event log into plain language (#1).
Then(
    "the history shows the plain activity for my request {string}",
    async ({ page }, prompt: string) => {
        await expect(page.locator("[data-audit] .activity-item", { hasText: prompt })).toBeVisible();
    },
);

// No raw engine type names leak into the user-facing activity list (#1).
Then("the history shows no raw engine event names", async ({ page }) => {
    // `data-audit` sits ON the `.activity` container (AuditTimeline), so match the
    // element itself — `[data-audit] .activity` would look for a non-existent child.
    const activity = page.locator("[data-audit]");
    await expect(activity).toBeVisible();
    // The raw log (with .event rows) must not be rendered until the dev toggle
    // is used — by default there are no raw event rows on screen.
    await expect(page.locator("[data-raw-log]")).toHaveCount(0);
    await expect(activity).not.toContainText("RunRequested");
    await expect(activity).not.toContainText("ObservationRecorded");
});

// The review/export state machine is gated behind dev mode, absent by default (#1).
Then("the history shows no review state-machine controls", async ({ page }) => {
    await expect(page.getByTestId("review-propose")).toHaveCount(0);
    await expect(page.getByTestId("export-target-admit")).toHaveCount(0);
});

When("I reveal the raw event log", async ({ page }) => {
    await page.locator("[data-raw-log-toggle]").click();
});

// The top-bar keep arms on the first click and commits on the second (#2).
When("I click keep on the task bar review", async ({ page }) => {
    await page.locator("[data-testid=taskbar] [data-task-keep]").first().click();
});

Then("the task bar keep is armed for confirmation", async ({ page }) => {
    await expect(page.locator("[data-testid=taskbar] [data-task-keep]").first()).toHaveAttribute(
        "data-arming",
        "1",
    );
});

Then("the review is still pending in the task bar", async ({ page }) => {
    await expect(page.locator("[data-testid=taskbar] [data-task]").first()).toBeVisible();
});

// The internal config file is folded out of the review by default (#4): the
// changed-files count reads "1 file changed" and only the deliverable shows.
Then("the changed-files review hides the internal settings file", async ({ page }) => {
    await expect(page.locator(".diff .diff-file-head", { hasText: ".agent-config.json" })).toHaveCount(0);
    await expect(page.locator(".diff-toolbar")).toContainText("1 file changed");
});

When("I reveal the internal settings file in the review", async ({ page }) => {
    await page.locator("[data-diff-internal-toggle]").click();
});

// The chat header leads with the chat's own name (#6) so two chats under one
// method are distinguishable.
Then("the chat header shows the title {string}", async ({ page }, title: string) => {
    await expect(page.locator("[data-chat-title]")).toContainText(title);
});

// Rename the currently-open chat from its nav row. The header reads a library
// projection, so it must reflect this live off the workspace event stream.
When("I rename the open chat to {string}", async ({ page }, name: string) => {
    await page.locator("[data-chat].active").click({ button: "right" });
    await page.locator(".menu-item-label", { hasText: /^rename$/ }).click();
    await page.locator(".inline-edit").fill(name);
    await page.locator(".inline-edit").press("Enter");
});

// ---- round 7: responsive layout, pinned composer, immediate feedback ----

// A narrow laptop width (round-7 #1): below the old assumption of a very wide
// window. The shell must not clip the primary `send` control — the chat lane has
// a hard min-width and the composer input shrinks instead of pushing send off.
When("the window is a narrow laptop size", async ({ page }) => {
    await page.setViewportSize({ width: 1040, height: 760 });
});

// A short frame (round-7 #2): the composer is pinned to the bottom of the chat
// panel, so it stays on screen regardless of transcript length or window height.
When("the window is a short frame", async ({ page }) => {
    await page.setViewportSize({ width: 1040, height: 520 });
});

// The single most important control must be fully within the viewport — not
// clipped off-panel and not scrolled below the fold.
Then("the send button is fully on screen", async ({ page }) => {
    const send = page.getByRole("button", { name: "send", exact: true });
    await expect(send).toBeVisible();
    const box = await send.boundingBox();
    const vp = page.viewportSize();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.y).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(vp!.width + 1);
    expect(box!.y + box!.height).toBeLessThanOrEqual(vp!.height + 1);
});

// (View auto-opening the single changed file (round-7 #3) reuses the existing
// "the file view shows {string}" step defined above — no new step needed.)

// Pressing send echoes the user's message into the transcript immediately
// (round-7 #6), so there's a visible reaction before the agent responds.
Then("the transcript echoes my message {string}", async ({ page }, text: string) => {
    await expect(page.locator(".run .transcript .line.user", { hasText: text })).toBeVisible();
});

// ---- round 9: honest banners, legible search, one honest improve entry ----

// Search highlight (#6 round-9): a surviving row marks the literal matched
// substring so a filtered list shows why each row stayed.
Then("the matched text {string} is highlighted in the results", async ({ page }, text: string) => {
    await expect(page.locator("mark.search-hit", { hasText: text }).first()).toBeVisible();
});

// The create affordance is hidden while a search is active so it can't read as a
// stray hit (#6 round-9).
Then("I can create a new method", async ({ page }) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await expect(page.getByText("+ archetype", { exact: true })).toBeVisible();
});
Then("I cannot create a new method", async ({ page }) => {
    await expect(page.getByText("+ archetype", { exact: true })).toHaveCount(0);
});

// Open the context menu on a named archetype (Library facet).
When("I open the context menu on the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await expect(page.locator(".context-menu")).toBeVisible();
});

// One honest edit entry, not two identically-behaving modes (#5 round-9). The
// action is now called "edit" (was "improve this method").
Then("the menu offers exactly one improve entry", async ({ page }) => {
    await expect(page.locator(".menu-item-label", { hasText: /^edit$/i })).toHaveCount(1);
});
Then("the menu does not promise working alongside it live", async ({ page }) => {
    await expect(page.locator(".context-menu", { hasText: "alongside it live" })).toHaveCount(0);
});

// The improve composer names the method and drops "archetype" / "the editor" (#4).
Then("the composer placeholder does not mention {string}", async ({ page }, word: string) => {
    const ph = await page.locator(".composer input").getAttribute("placeholder");
    expect(ph ?? "").not.toContain(word);
});

// Rename selects the existing name so it can be typed straight over (#smaller r9).
When("I start renaming the archetype {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Library" }).click();
    await page
        .locator("[data-archetype]", { hasText: name })
        .locator(".tree-node.archetype")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "rename" }).click();
    await expect(page.locator(".inline-edit")).toBeVisible();
});
Then("the rename field has the existing name selected", async ({ page }) => {
    const sel = await page.locator(".inline-edit").evaluate((el: HTMLInputElement) => ({
        start: el.selectionStart,
        end: el.selectionEnd,
        len: el.value.length,
    }));
    expect(sel.start).toBe(0);
    expect(sel.end).toBe(sel.len);
    expect(sel.len).toBeGreaterThan(0);
});

// ---- round 10: honest improve vocabulary, legible status, clearer review chrome ----

// A button that must NOT be present (the work-chat verb inside an improve chat).
Then("I do not see the button {string}", async ({ page }, label: string) => {
    await expect(page.getByRole("button", { name: label, exact: true })).toHaveCount(0);
});

// #4 — the per-chat status was a 10px grey whisper; it must read at a legible size.
Then("the status badge text is at least {int}px", async ({ page }, px: number) => {
    const size = await page
        .getByTestId("run-phase")
        .evaluate((el) => parseFloat(getComputedStyle(el as HTMLElement).fontSize));
    expect(size).toBeGreaterThanOrEqual(px);
});

// #6 — the hidden-config disclosure must not read like the changed-file count that
// sits directly above it (no second "N file(s)" phrase to be misread as a count).
Then("the internal-file toggle does not read like a changed-file count", async ({ page }) => {
    const toggle = page.locator("[data-diff-internal-toggle]");
    await expect(toggle).toBeVisible();
    await expect(toggle).not.toContainText("file changed");
    await expect(toggle).not.toContainText("internal file");
});

Then("the internal-file toggle reveals the hidden config files", async ({ page }) => {
    await page.locator("[data-diff-internal-toggle]").click();
    await expect(page.locator(".diff .diff-file-head", { hasText: ".agent-config.json" })).toBeVisible();
});


// ---- RF-E1 / O-1: context-sources panel -----------------------------------

When("I open the context sources panel", async ({ page }) => {
    await page.locator("[data-open-sources]").click();
    await expect(page.locator("[data-context-overlay]")).toBeVisible();
});

Then("the context sources panel lists a {string} source", async ({ page }, kind: string) => {
    await expect(page.locator(`[data-context-source][data-kind="${kind}"]`).first()).toBeVisible();
});

Then("the context source is marked {string}", async ({ page }, availability: string) => {
    await expect(
        page.locator(`[data-context-source][data-availability="${availability}"]`).first(),
    ).toBeVisible();
});

Then("the context sources panel shows no context sources", async ({ page }) => {
    // The empty-state copy renders in place of the list; assert no source rows.
    await expect(page.locator("[data-context-source]")).toHaveCount(0);
    await expect(page.locator(".context-drawer .status", { hasText: "No context" })).toBeVisible();
});

// ---- RF-E1 / O-4: output catalog -------------------------------------------

// A turn under a chat that holds context produces the engagement's output
// resource; we run one turn and let it settle so the catalog has an output.
When("I task the agent and let the turn settle", async ({ page }) => {
    await page.locator(".composer input").fill("produce something");
    await page.getByRole("button", { name: "send", exact: true }).click();
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Completed", { timeout: 45_000 });
});

When("I open the outputs catalog", async ({ page }) => {
    await page.getByRole("button", { name: "History", exact: true }).click();
    await page.locator('.shelf-drawer .tab[data-tab="outputs"]').click();
    await expect(page.locator("[data-output-catalog]")).toBeVisible();
});

Then("the outputs catalog lists an output", async ({ page }) => {
    await expect(page.locator("[data-output]").first()).toBeVisible();
});

Then("the output shows its review state", async ({ page }) => {
    await expect(page.locator("[data-output] [data-output-review]").first()).toBeVisible();
});

Then("the outputs catalog shows no outputs", async ({ page }) => {
    await expect(page.locator("[data-output]")).toHaveCount(0);
    await expect(page.locator(".output-catalog .status", { hasText: "No outputs" })).toBeVisible();
});

// ---- RF-E4: projection freshness + retry (error path) ----------------------

// Force the desktop projection loads to fail by aborting the diff/merge/run
// fetches at the network edge. The freshness reducer folds the failure into a
// `stale`/`stuck` status and the FreshnessBanner surfaces it; this is the only
// error-path that exercises a dropped projection call end to end in the browser.
// Fail every per-chat projection the desktop freshness signal folds — run, diff,
// AND merge (RF-E4, App.tsx markLoadOk/Fail). Aborting only some would let a
// successful load clear the staleness (the signal is shared), so a refresh could
// race back to `fresh`; failing all of them makes the banner deterministic. The
// merge review reads through the freshness carriage (UX-13), so its route is the
// carriage projection `/projections/:id/merge` (with a `?freshness=` query), not
// the bare `/chats/:id/merge`.
const FAILING_PROJECTIONS = /\/(chats\/[^/]+\/diff|scopes\/[^/]+\/run|projections\/[^/]+\/merge)(\?|$)/;
When("the projection refresh starts failing", async ({ page }) => {
    await page.route(FAILING_PROJECTIONS, (route) => route.abort());
});

When("the projection refresh recovers", async ({ page }) => {
    await page.unroute(FAILING_PROJECTIONS);
});

// WS-H removed the manual "pull in latest" button; the per-chat projections now
// re-fetch on chat selection, so reloading (the chat re-selects from the URL)
// re-runs them all against the failing routes — a deterministic projection refresh.
When("I trigger a projection refresh", async ({ page }) => {
    await page.reload();
});

When("I retry the projection refresh", async ({ page }) => {
    await page.locator("[data-freshness-retry]").click();
});

Then("the freshness banner is shown", async ({ page }) => {
    await expect(page.locator("[data-freshness-banner]")).toBeVisible();
});

Then("the freshness banner offers a retry", async ({ page }) => {
    await expect(page.locator("[data-freshness-retry]")).toBeVisible();
});

Then("the freshness banner clears", async ({ page }) => {
    await expect(page.locator("[data-freshness-banner]")).toHaveCount(0);
});

// ---- multi-agent concurrency (round-13): per-chat run state + Browse dots ----

Given("a placement I can open more chats under", async ({ page }) => {
    await page.goto("/");
    concProject = await placeArchetypeOnFreshProject(page);
    const group = page.locator(`.tree-group[data-project]`, { hasText: concProject });
    await group.locator(".tree-subgroup[data-placement] .action-row .create-btn").first().click();
    await expect(page.getByTestId("run-phase")).toHaveAttribute("data-run-phase", "Init");
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

When("I open another chat under that placement", async ({ page }) => {
    const group = page.locator(`.tree-group[data-project]`, { hasText: concProject });
    await group.locator(".tree-subgroup[data-placement] .action-row .create-btn").first().click();
    await expect(page.getByTestId("stream-ready")).toBeAttached();
});

Then("the open chat shows a working dot", async ({ page }) => {
    await expect(page.locator('[data-chat].active .status-gem[data-state="working"]')).toBeVisible();
});

Then("{int} chats show a working dot", async ({ page }, n: number) => {
    await expect(page.locator('.status-gem[data-state="working"]')).toHaveCount(n);
});

// The chat on screen must show its OWN turn, not a concurrently-running chat's —
// scope to the run pane (the nav rows legitimately carry both chats' titles).
Then("the chat pane shows {string} but not {string}", async ({ page }, mine: string, other: string) => {
    await expect(page.locator(".run .transcript")).toContainText(mine);
    await expect(page.locator(".run .transcript")).not.toContainText(other);
});

When("the running turns finish", async ({ page }) => {
    // The [slow] turns hold ~3.5s each; wait until no chat is working any more.
    await expect(page.locator('.status-gem[data-state="working"]')).toHaveCount(0, { timeout: 15_000 });
});

Then("no chat shows a working dot", async ({ page }) => {
    await expect(page.locator('.status-gem[data-state="working"]')).toHaveCount(0);
});

// ---- per-project model access (LLM-2) ----

// Open a project's model-access panel from its right-click context menu (the id comes
// from the node, never typed) — mirrors the "add an archetype" project-menu flow.
When("I open model access for project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator("[data-project]", { hasText: name })
        .locator(".tree-node.project")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "model access" }).click();
});

Then("the model-access panel is open", async ({ page }) => {
    await expect(page.locator("[data-project-model-access]")).toBeVisible();
});

// Pin a provider: select it, paste a throwaway token, and submit. The token is sealed
// server-side (SEC-4) and never read back — we only assert the pin appears.
When("I pin the provider {string} for this project", async ({ page }, provider: string) => {
    const panel = page.locator("[data-project-model-access]");
    await panel.locator("select").selectOption(provider);
    await panel.locator("[data-project-credential-token]").fill("sk-e2e-throwaway");
    await panel.locator("button", { hasText: /^pin$/ }).click();
});

Then("the project pins the provider {string}", async ({ page }, provider: string) => {
    await expect(
        page.locator("[data-project-model-access]").locator(`[data-pinned="${provider}"]`),
    ).toBeVisible();
});

When("I unpin the provider {string} for this project", async ({ page }, provider: string) => {
    await page
        .locator("[data-project-model-access]")
        .locator(`[data-pinned="${provider}"]`)
        .locator("button", { hasText: "unpin" })
        .click();
});

Then("the project has no provider pins", async ({ page }) => {
    await expect(
        page.locator("[data-project-model-access]").locator("[data-pinned]"),
    ).toHaveCount(0);
});

// ---- project home rollup (UX-2) ----

// Open a project's home panel from its right-click context menu (id from the node).
When("I open project home for project {string}", async ({ page }, name: string) => {
    await page.locator(".facet", { hasText: "Projects" }).click();
    await page
        .locator("[data-project]", { hasText: name })
        .locator(".tree-node.project")
        .click({ button: "right" });
    await page.locator(".menu-item", { hasText: "project home" }).click();
});

Then("the project-home panel is open", async ({ page }) => {
    await expect(page.locator("[data-project-home-panel]")).toBeVisible();
});

// A project created here gets a default placement, so the audit rollup counts >= 1.
Then("the project-home panel shows at least {int} placement", async ({ page }, n: number) => {
    const badge = page.locator("[data-project-home-panel] [data-audit-placements]");
    await expect(badge).toBeVisible();
    const text = (await badge.textContent()) ?? "0";
    expect(Number.parseInt(text, 10)).toBeGreaterThanOrEqual(n);
});

// ---- fork tree (UX-8) ----

When("I open the fork tree for the first chat", async ({ page }) => {
    await page.locator(".chat-item").first().click({ button: "right" });
    await page.locator(".menu-item", { hasText: "fork tree" }).click();
});

Then("the fork tree shows at least {int} chats", async ({ page }, n: number) => {
    await expect(page.locator("[data-fork-tree]")).toBeVisible();
    const nodes = page.locator("[data-fork-tree] [data-fork-node]");
    expect(await nodes.count()).toBeGreaterThanOrEqual(n);
});

// ---- UX-11: cross-party output review (held output + provenance + consent) ----

// Propose review on the current chat's produced output — keyed on the chat id from the
// UX-4 URL (?chat=<id>); the output resource is out-<chat>. This puts the output in the held
// (Proposed) state the catalog surfaces, with its stakeholder parties (the taint) as required.
When("review is proposed on this chat's output", async ({ page, request }) => {
    const chat = new URL(page.url()).searchParams.get("chat");
    if (!chat) throw new Error("no chat selected in the URL (UX-4 ?chat=)");
    const res = await request.post(`${aliceCP}/chats/${chat}/resources/out-${chat}/review`);
    expect(res.ok(), "propose review on the output").toBeTruthy();
});

Then("the held output shows stakeholder {string}", async ({ page }, party: string) => {
    await expect(page.locator("[data-output-review-hold]").first()).toBeVisible();
    await expect(page.locator(`[data-review-party="${party}"]`).first()).toBeVisible();
});

When("I consent to release the held output for {string}", async ({ page }, party: string) => {
    await page.locator(`[data-consent="${party}"]`).first().click();
});

Then("the held output is released", async ({ page }) => {
    await expect(page.locator("[data-output-released]").first()).toBeVisible();
});

Then("I release the held output", async ({ page }) => {
    await page.locator("[data-release]").first().click();
});
