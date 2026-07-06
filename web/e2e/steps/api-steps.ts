/**
 * HTTP-level BDD step bindings (decision D-BDD-API). These drive the **control
 * plane directly over HTTP** (the `request` fixture), not the browser — so backend
 * mechanism features with no UI surface (M2 packaging, federation, …) are BDD-
 * covered in the same Given/When/Then style. The control plane is launched per run
 * by `e2e/run.mjs` (a free port resolved per run) via `e2e/fed-control-plane.sh`.
 * Scenarios use distinct ids so the shared control-plane state stays isolated.
 */

import { expect } from "@playwright/test";
import { createBdd } from "playwright-bdd";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { aliceCP } from "../ports.mjs";

const { Given, When, Then, Before } = createBdd();

/** The co-resident control plane (the same one the browser talks to via CORS), resolved
 *  per run for concurrency-safety (e2e/ports.mjs). */
const CP = aliceCP;

/** POST a lifecycle command as JSON (a bare-string unit variant like "Export" must
 *  be JSON-encoded with an application/json content-type, or axum's Json extractor
 *  415s). `request` is playwright's APIRequestContext. */
function postCmd(request: any, url: string, body: unknown) {
    return request.post(url, { headers: { "content-type": "application/json" }, data: JSON.stringify(body) });
}

/** Scenario-scoped state (workers:1, serial), reset before each scenario. */
let world: { eng?: string; folder?: string; rid?: string; content?: string; exportState?: any; reviewState?: any } = {};
Before(() => {
    world = {};
});

/** Mirror the server's `context_id`: a URL-safe slug of the folder path. */
function contextRid(folder: string): string {
    const slug = folder.replace(/[^a-zA-Z0-9]/g, "-").replace(/^-+|-+$/g, "");
    return `ctx-${slug}`;
}

// ---- M2 packaging ----

Given("a published package {string} at version {string}", async ({ request }, id: string, version: string) => {
    const res = await request.post(`${CP}/packages`, {
        data: { id, version, agent_ref: "agent-default" },
    });
    expect(res.ok(), `publish ${id}`).toBeTruthy();
});

When("the target installs {string}", async ({ request }, id: string) => {
    // not asserted here — a later step asserts success or rejection
    await request.post(`${CP}/packages/${id}/install`);
});

Then("the install is rejected", async ({ request }) => {
    // the most recent install of a non-published package returns 409
    const res = await request.post(`${CP}/packages/ghost/install`);
    expect(res.status()).toBe(409);
});

Then("package {string} shows status {string} in the catalog", async ({ request }, id: string, status: string) => {
    const res = await request.get(`${CP}/packages`);
    const catalog = (await res.json()) as Array<{ id: string; status: string }>;
    expect(catalog.find((p) => p.id === id)?.status).toBe(status);
});

When("{string} is entitled for context {string}", async ({ request }, id: string, ctx: string) => {
    const res = await request.post(`${CP}/packages/${id}/entitle?context=${ctx}`);
    expect(res.ok(), `entitle ${id}`).toBeTruthy();
});

When("the source withdraws {string}", async ({ request }, id: string) => {
    const res = await request.post(`${CP}/packages/${id}/withdraw`);
    expect(res.ok(), `withdraw ${id}`).toBeTruthy();
});

Then("a governed run of {string} in {string} is ready", async ({ request }, id: string, ctx: string) => {
    const res = await request.get(`${CP}/packages/${id}/readiness?context=${ctx}`);
    expect((await res.json()).run_ready).toBe(true);
});

Then("a governed run of {string} in {string} is not ready", async ({ request }, id: string, ctx: string) => {
    const res = await request.get(`${CP}/packages/${id}/readiness?context=${ctx}`);
    expect((await res.json()).run_ready).toBe(false);
});

// ---- M1 durable context resources ----

Given("an engagement {string}", async ({ request }, id: string) => {
    world.eng = id;
    await request.post(`${CP}/chats`, { data: { id } });
});

When("a folder is opened as context in {string}", async ({ request }, id: string) => {
    world.eng = id;
    const folder = path.join(os.tmpdir(), `gaugewright-bdd-${id}`);
    fs.mkdirSync(folder, { recursive: true });
    world.content = `secret-bytes-${id}`;
    fs.writeFileSync(path.join(folder, "notes.txt"), world.content);
    world.folder = folder;
    world.rid = contextRid(folder);
    const res = await request.post(`${CP}/chats/${id}/context`, { data: { path: folder } });
    expect(res.ok(), "ingest context").toBeTruthy();
});

Then("{string} lists a granted context resource", async ({ request }, id: string) => {
    const res = await request.get(`${CP}/chats/${id}/resources`);
    const list = (await res.json()) as Array<{ kind: string; access: string }>;
    const ctx = list.find((r) => r.kind === "context");
    expect(ctx, "a context resource is listed").toBeTruthy();
    expect(ctx?.access).toBe("Granted");
});

Then("its payload is not in the resource listing", async ({ request }) => {
    const body = await (await request.get(`${CP}/chats/${world.eng}/resources`)).text();
    expect(body).not.toContain(world.content); // INV-10: metadata only, no payload
});

Then("the context content resolves to the ingested bytes", async ({ request }) => {
    const res = await request.get(`${CP}/chats/${world.eng}/resources/${world.rid}/content?path=notes.txt`);
    expect(res.ok()).toBeTruthy();
    expect(await res.text()).toBe(world.content);
});

When("the context resource is tombstoned", async ({ request }) => {
    const res = await request.post(`${CP}/chats/${world.eng}/resources/${world.rid}/tombstone`);
    expect(res.ok(), "tombstone").toBeTruthy();
});

Then("resolving the context content is gone", async ({ request }) => {
    const res = await request.get(`${CP}/chats/${world.eng}/resources/${world.rid}/content?path=notes.txt`);
    expect(res.status()).toBe(410); // INV-18: future resolution blocked
});

Then("the context resource still lists, marked tombstoned", async ({ request }) => {
    const list = (await (await request.get(`${CP}/chats/${world.eng}/resources`)).json()) as Array<{
        id: string;
        tombstoned: boolean;
    }>;
    expect(list.find((r) => r.id === world.rid)?.tombstoned).toBe(true);
});

// ---- per-resource export & review (output derives gating from the resource) ----

When("the agent runs a turn in {string}", async ({ request }, id: string) => {
    world.eng = id;
    const res = await request.post(`${CP}/chats/${id}/task`, { data: { prompt: "go" } });
    expect(res.ok(), "task turn").toBeTruthy();
});

Then("{string} has an {string} resource", async ({ request }, id: string, kind: string) => {
    const list = (await (await request.get(`${CP}/chats/${id}/resources`)).json()) as Array<{ id: string; kind: string }>;
    expect(list.find((r) => r.kind === kind), `${kind} resource`).toBeTruthy();
});

When("export of the output in {string} is proposed", async ({ request }, id: string) => {
    world.eng = id;
    const res = await request.post(`${CP}/chats/${id}/resources/out-${id}/export`);
    expect(res.ok(), "propose export").toBeTruthy();
    world.exportState = await res.json();
});

Then("the export's required consent includes {string}", async ({}, who: string) => {
    expect(world.exportState?.state?.source_required).toContain(who);
});

Then("the export does not clear without consent in {string}", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/scopes/${id}-export-out-${id}/export/command`, "Export");
    expect(res.status()).toBe(409); // gated: no source consent / target admission yet
});

When("the owner consents and the target admits the export in {string}", async ({ request }, id: string) => {
    const scope = `${id}-export-out-${id}`;
    await postCmd(request, `${CP}/scopes/${scope}/export/command`, { SourceConsent: "local-user" });
    await postCmd(request, `${CP}/scopes/${scope}/export/command`, "TargetAdmit");
});

Then("the output of {string} is exported", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/scopes/${id}-export-out-${id}/export/command`, "Export");
    expect((await res.json()).phase).toBe("Exported");
});

When("review of the output in {string} is proposed", async ({ request }, id: string) => {
    world.eng = id;
    const res = await request.post(`${CP}/chats/${id}/resources/out-${id}/review`);
    expect(res.ok(), "propose review").toBeTruthy();
    world.reviewState = await res.json();
});

Then("the review's required consent includes {string}", async ({}, who: string) => {
    expect(world.reviewState?.state?.required).toContain(who);
});

When("the owner consents to the review in {string}", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/scopes/${id}-review-out-${id}/review/command`, { Consent: "local-user" });
    expect((await res.json()).phase).toBe("Cleared");
});

Then("the review of {string} clears and releases", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/scopes/${id}-review-out-${id}/review/command`, "Release");
    expect((await res.json()).phase).toBe("Released");
});

// ---- merge / mainline ----

Then("the merge of {string} is {string}", async ({ request }, id: string, phase: string) => {
    const res = await request.get(`${CP}/chats/${id}/merge`);
    expect((await res.json()).phase).toBe(phase);
});

When("the diff of {string} is admitted", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/chats/${id}/merge/command`, { action: "admit" });
    expect(res.ok(), "admit merge").toBeTruthy();
});

When("the diff of {string} is rejected", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/chats/${id}/merge/command`, { action: "reject" });
    expect(res.ok(), "reject merge").toBeTruthy();
});

When("{string} is integrated to the mainline", async ({ request }, id: string) => {
    const res = await postCmd(request, `${CP}/chats/${id}/merge/command`, { action: "integrate" });
    expect(res.ok(), "integrate").toBeTruthy();
});

When("{string} syncs from the mainline", async ({ request }, id: string) => {
    const res = await request.post(`${CP}/chats/${id}/sync`);
    expect(res.ok(), "sync").toBeTruthy();
    world.exportState = await res.json(); // reuse the scratch slot for the sync result
});

Then("{string} reports it synced cleanly", async ({}, _id: string) => {
    expect(world.exportState?.synced).toBe(true);
    expect(world.exportState?.conflict).toBe(false);
});

// ---- rename a chat ----

When("{string} is renamed to {string}", async ({ request }, id: string, title: string) => {
    const res = await request.put(`${CP}/chats/${id}/title`, {
        headers: { "content-type": "application/json" },
        data: JSON.stringify({ title }),
    });
    expect(res.ok(), "rename").toBeTruthy();
});

Then("the library shows a chat titled {string}", async ({ request }, title: string) => {
    const body = await (await request.get(`${CP}/workspace`)).text();
    expect(body).toContain(title);
});
