/**
 * The **output catalog** (RF-E1 / m0-gate O-4): a listing of the produced
 * `output`-kind resources an engagement holds, read from the
 * `GET /chats/:id/resources` projection and filtered to outputs. Until now no
 * user-facing surface showed produced outputs — `ReviewShelf` is dev-only and the
 * `Shelf` shows only an activity timeline — so a finished deliverable had nowhere
 * to surface with its protection state. This catalog is available outside dev mode.
 *
 * Each output is shown with its availability/tombstone state and its review +
 * export **phase** (the protection gating it must pass before it can leave). The
 * decisions are the pure helpers in `workbench-ui/resource-catalog.ts` (re-exported
 * through the transitional `state/resource-catalog.ts` path); this component only
 * paints them and reads the review/export projections per output (handle +
 * metadata only, `INV-10`). It is mounted as a tab in the history `Shelf`.
 */

import { createResource, For, Show } from "solid-js";
import {
    type EngagementId,
    type ExportState,
    type ResourceView,
    type ReviewCommand,
    type ReviewState,
    type ScopeId,
    scopeId,
} from "@gaugewright/control-plane-client";
import {
    availabilityLabel,
    availabilityOf,
    outputProtectionLabel,
    outputs,
    resourceExportScope,
    resourceReviewScope,
    resourceTitle,
} from "./resource-catalog";
import { LoadError } from "./LoadError";

export interface OutputCatalogApi {
    getResources(id: EngagementId): Promise<ResourceView[]>;
    getReview(scope: ScopeId): Promise<ReviewState>;
    getExport(scope: ScopeId): Promise<ExportState>;
    /** Drive the review reducer (UX-11): a stakeholder consents to release a held output. */
    reviewCommand(scope: ScopeId, command: ReviewCommand): Promise<ReviewState>;
}

export function OutputCatalog(props: {
    api: OutputCatalogApi;
    /** The engagement whose outputs to catalog (the resource route is engagement-keyed). */
    id: EngagementId;
    refreshKey?: unknown;
}) {
    const [resources, { refetch }] = createResource(
        () => [props.id, props.refreshKey] as const,
        ([id]) => props.api.getResources(id),
    );
    const produced = () => outputs(resources() ?? []);

    return (
        <div class="output-catalog" data-output-catalog>
            <Show when={!resources.error} fallback={<LoadError what="the outputs" onRetry={() => void refetch()} />}>
            <Show when={resources()} fallback={<div class="status">loading…</div>}>
                <Show
                    when={produced().length}
                    fallback={
                        <div class="status">
                            No outputs yet. When the agent produces a deliverable it shows here with its review status.
                        </div>
                    }
                >
                    <div class="resource-list">
                        <For each={produced()}>
                            {(r) => <OutputRow api={props.api} engagement={props.id} resourceId={r.id} avail={availabilityOf(r)} title={resourceTitle(r)} tombstoned={r.tombstoned} />}
                        </For>
                    </div>
                </Show>
            </Show>
            </Show>
        </div>
    );
}

/** One output row: its availability plus the review/export phases that gate it.
 *  The lifecycle scopes are minted lazily (an output that was never proposed reads
 *  as `Init`), so a failed read just shows a dash rather than breaking the row. */
function OutputRow(props: {
    api: OutputCatalogApi;
    engagement: EngagementId;
    resourceId: string;
    avail: ReturnType<typeof availabilityOf>;
    title: string;
    tombstoned: boolean;
}) {
    const reviewScope = () => scopeId(resourceReviewScope(props.engagement, props.resourceId));
    const [review, { refetch: refetchReview }] = createResource(
        () => props.resourceId,
        async () => {
            try {
                return await props.api.getReview(reviewScope());
            } catch {
                return null;
            }
        },
    );
    // UX-11: a stakeholder party consents to release the held output. The provenance (which
    // parties have a stake) is the review's `required` set = the output's stakeholders (the
    // engagement taint). When every stakeholder consents, the reducer releases it.
    const consent = async (party: string) => {
        try {
            await props.api.reviewCommand(reviewScope(), { Consent: party });
        } finally {
            void refetchReview();
        }
    };
    // Once every stakeholder has consented (Cleared), the held output can be released.
    const release = async () => {
        try {
            await props.api.reviewCommand(reviewScope(), "Release");
        } finally {
            void refetchReview();
        }
    };
    const [exp] = createResource(
        () => props.resourceId,
        async (rid) => {
            try {
                return await props.api.getExport(scopeId(resourceExportScope(props.engagement, rid)));
            } catch {
                return null;
            }
        },
    );

    return (
        <div
            class="resource-row output-row"
            data-output={props.resourceId}
            data-availability={props.avail}
            classList={{ erased: props.tombstoned }}
        >
            <span class="resource-kind" data-resource-kind>output</span>
            <span class="resource-title">{props.title}</span>
            <span class="resource-availability" data-availability={props.avail} title="whether this output's contents are available to you">
                {availabilityLabel(props.avail)}
            </span>
            {/* Plain-language protection status — never raw "review: Init / export:
                Init" tokens (round-12 A). Hidden entirely until something happens;
                the raw phases stay on data- attributes for tests/automation. */}
            <Show when={outputProtectionLabel(review()?.phase, exp()?.phase)}>
                {(label) => (
                    <span
                        class="output-status"
                        data-output-review={review()?.phase ?? "—"}
                        data-output-export={exp()?.phase ?? "—"}
                        title="review &amp; sharing status"
                    >
                        {label()}
                    </span>
                )}
            </Show>
            <Show when={props.tombstoned}>
                <span class="resource-tombstone" data-tombstoned title="payload erased">erased</span>
            </Show>
            <Show when={review()?.phase === "Released"}>
                <span class="badge released" data-output-released title="every stakeholder consented — this output can leave">released</span>
            </Show>
            {/* UX-11: all stakeholders consented — the output is cleared and can be released. */}
            <Show when={review()?.phase === "Cleared"}>
                <div class="output-review-cleared" data-output-review-cleared={props.resourceId}>
                    <span class="hold-note">All stakeholders consented — ready to release</span>
                    <button type="button" class="link-btn" data-release onClick={() => void release()}>
                        release
                    </button>
                </div>
            </Show>
            {/* UX-11: a held output awaiting cross-party review. Show the stakeholder parties
                (its provenance — whose data it derives from) and each party's consent, with a
                consent-to-release affordance. Content stays gated until all consent. */}
            <Show when={review()?.phase === "Proposed"}>
                <div class="output-review-hold" data-output-review-hold={props.resourceId}>
                    <span class="hold-note" title="this output derives from other parties' data and is held until they consent to release">
                        Held for review — stakeholders must consent to release
                    </span>
                    <ul class="review-parties">
                        <For each={review()!.required}>
                            {(party) => {
                                const consented = () => (review()?.consented ?? []).includes(party);
                                return (
                                    <li class="review-party" data-review-party={party} classList={{ consented: consented() }}>
                                        <span class="party-id">{party}</span>
                                        <Show
                                            when={consented()}
                                            fallback={
                                                <button
                                                    type="button"
                                                    class="link-btn"
                                                    data-consent={party}
                                                    onClick={() => void consent(party)}
                                                >
                                                    consent
                                                </button>
                                            }
                                        >
                                            <span class="badge" data-consented={party}>consented ✓</span>
                                        </Show>
                                    </li>
                                );
                            }}
                        </For>
                    </ul>
                </div>
            </Show>
        </div>
    );
}
