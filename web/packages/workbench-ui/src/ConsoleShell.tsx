/**
 * **Console shell** (ADR 0077 §2/§7/§9): the hosted web account's home — *core*
 * (the account + the tenant switcher) plus one section per **facility** the current
 * scope holds. There is **no hardcoded section list**: the sections are whatever
 * facilities are attached, so different tenants render differently.
 *
 * This is hub *scaffolding*, not a workbench panel — the chat/viewer/files panels are
 * the shared workbench UI reached over the same control plane (the Console IS the
 * workbench UI over the Remote adapter); this shell is the surrounding surface that
 * only the hosted account has. A thin renderer over `/account/{tenants,facilities}`.
 */

import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import type { AccountFacility, AccountTenant, AttachFacilityInput } from "@gaugewright/control-plane-client";

/** The account/facility slice of the control plane the Console shell reads. */
export interface ConsoleApi {
    accountTenants(): Promise<AccountTenant[]>;
    accountFacilities(): Promise<AccountFacility[]>;
    accountAttachFacility(input: AttachFacilityInput): Promise<AccountFacility>;
    accountDetachFacility(id: string): Promise<void>;
}

/** Human labels for the known facility kinds (ADR 0077 §7); unknown kinds fall back
 *  to the raw kind so a new server-side kind still renders. */
const KIND_LABEL: Record<string, string> = {
    library_sync: "Library sync",
    cloud_backup: "Cloud backup",
    hosted_home_node: "Hosted home node",
    registered_host: "Registered host",
};

function kindLabel(kind: string): string {
    return KIND_LABEL[kind] ?? kind;
}

export function ConsoleShell(props: { api: ConsoleApi }): JSX.Element {
    const [tick, setTick] = createSignal(0);
    const refresh = () => setTick((t) => t + 1);
    const [status, setStatus] = createSignal("");

    const [tenants] = createResource(tick, () => props.api.accountTenants());
    const [facilities] = createResource(tick, () => props.api.accountFacilities());

    const hasLibrarySync = () =>
        (facilities() ?? []).some((f) => f.kind === "library_sync" && f.status === "active");

    const attachLibrarySync = async () => {
        setStatus("attaching Library sync…");
        try {
            await props.api.accountAttachFacility({
                id: "library_sync",
                kind: "library_sync",
                displayName: "Library sync",
            });
            setStatus("");
            refresh();
        } catch (e) {
            setStatus(`could not attach: ${e instanceof Error ? e.message : String(e)}`);
        }
    };

    const detach = async (id: string) => {
        setStatus("removing…");
        try {
            await props.api.accountDetachFacility(id);
            setStatus("");
            refresh();
        } catch (e) {
            setStatus(`could not remove: ${e instanceof Error ? e.message : String(e)}`);
        }
    };

    return (
        <div class="console-shell">
            <header class="console-header">
                <h1>Console</h1>
                {/* The tenant switcher (§9): the personal tenant-of-one is shown as your own
                    space, not "your org" — the `personal` flag, not a separate primitive. */}
                <nav class="tenant-switcher" aria-label="Tenants">
                    <Show when={(tenants() ?? []).length > 0} fallback={<span class="muted">No tenants yet</span>}>
                        <For each={tenants()}>
                            {(t) => (
                                <button class="tenant" type="button" title={`role: ${t.role}`}>
                                    {t.personal ? "Personal" : t.displayName || t.id}
                                </button>
                            )}
                        </For>
                    </Show>
                </nav>
            </header>

            <Show when={status()}>
                <p class="console-status" role="status">
                    {status()}
                </p>
            </Show>

            {/* One section per attached facility — no hardcoded list (§7). */}
            <section class="facilities" aria-label="Facilities">
                <div class="facilities-head">
                    <h2>Your facilities</h2>
                    <Show when={!hasLibrarySync()}>
                        <button class="btn" type="button" onClick={attachLibrarySync}>
                            Add Library sync
                        </button>
                    </Show>
                </div>
                <Show
                    when={(facilities() ?? []).length > 0}
                    fallback={<p class="muted">No facilities yet. Add one to extend what your account can do.</p>}
                >
                    <ul class="facility-list">
                        <For each={facilities()}>
                            {(f) => (
                                <li class="facility-card" classList={{ inactive: f.status !== "active" }}>
                                    <div class="facility-main">
                                        <span class="facility-title">{f.displayName || kindLabel(f.kind)}</span>
                                        <span class="facility-kind muted">{kindLabel(f.kind)}</span>
                                    </div>
                                    <span class="facility-status" data-status={f.status}>
                                        {f.status}
                                    </span>
                                    <button class="btn btn-quiet" type="button" onClick={() => void detach(f.id)}>
                                        Remove
                                    </button>
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>
            </section>
        </div>
    );
}
