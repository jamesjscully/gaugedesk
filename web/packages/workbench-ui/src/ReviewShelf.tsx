/**
 * The review shelf — a DEV-ONLY driver for the verified `review` + `resource_export`
 * reducers (it posts raw scope-level commands). It is **not** the production
 * protection surface: the user-facing path is `post_resource_review` /
 * `post_resource_export`, which derive the required consenter set from the output
 * resource's stakeholders server-side (`stakeholders \ {recipient}`, INV-16/13) —
 * the client never authors it (see the `*_required_is_derived_from_the_resource_
 * stakeholders` tests). Here the consenter set is read back from the server
 * projection after propose; `SEED` is only this manual driver's initial input
 * (CONF-15). The client still never decides a *transition* — each step is a command.
 */

import { createResource, createSignal, For, Show } from "solid-js";
import {
    describeFailure,
    type ExportCommand,
    type ExportState,
    type ReviewCommand,
    type ReviewState,
    type ScopeId,
} from "@gaugewright/control-plane-client";

export interface ReviewShelfApi {
    getReview(scope: ScopeId): Promise<ReviewState>;
    getExport(scope: ScopeId): Promise<ExportState>;
    reviewCommand(scope: ScopeId, cmd: ReviewCommand): Promise<ReviewState>;
    exportCommand(scope: ScopeId, cmd: ExportCommand): Promise<ExportState>;
}

export function ReviewShelf(props: { api: ReviewShelfApi; scope: ScopeId }) {
    // A rejection (INV-2) is the expected "the reducer declined, here is why"
    // outcome — surface it rather than swallowing it into a silent refetch.
    const [err, setErr] = createSignal("");
    const [review, { mutate: setReview, refetch: refetchReview }] = createResource(
        () => props.scope,
        (s) => props.api.getReview(s),
    );
    const [exp, { mutate: setExp, refetch: refetchExp }] = createResource(
        () => props.scope,
        (s) => props.api.getExport(s),
    );

    const SEED = ["A", "B"]; // dev driver's initial Propose input only (CONF-15)
    // After propose, the required/consenter set is the server projection's truth,
    // not a client-authored list.
    const reviewRequired = () => (review()?.required?.length ? review()!.required : SEED);
    const exportRequired = () => {
        const e = exp() as ExportState | undefined;
        return e?.source_required?.length ? e.source_required : SEED;
    };

    async function review_(cmd: ReviewCommand) {
        try {
            setReview(await props.api.reviewCommand(props.scope, cmd));
            setErr("");
        } catch (e) {
            setErr(describeFailure("apply that review step", e));
            await refetchReview();
        }
    }
    async function export_(cmd: ExportCommand) {
        try {
            setExp(await props.api.exportCommand(props.scope, cmd));
            setErr("");
        } catch (e) {
            setErr(describeFailure("apply that export step", e));
            await refetchExp();
        }
    }

    return (
        <div class="shelf">
            <h3>Review</h3>
            <div class="status">
                phase: <span class="phase" data-review-phase>{review()?.phase ?? "—"}</span>
                <Show when={review()?.required?.length}>
                    {" · consented "}
                    {review()?.consented.join(",") || "none"} / {review()?.required.join(",")}
                </Show>
            </div>
            <div class="bar">
                <button data-testid="review-propose" onClick={() => review_({ Propose: { required: SEED } })}>
                    propose
                </button>
                <For each={reviewRequired()}>
                    {(s) => (
                        <button data-testid={`review-consent-${s}`} onClick={() => review_({ Consent: s })}>
                            consent {s}
                        </button>
                    )}
                </For>
                <button data-testid="review-release" onClick={() => review_("Release")}>release</button>
            </div>

            <h3 style={{ "margin-top": "14px" }}>Export</h3>
            <div class="status">
                phase: <span class="phase" data-export-phase>{exp()?.phase ?? "—"}</span>
                <Show when={exp()}>
                    {" · target "}
                    {(exp() as ExportState).target_admitted ? "admitted" : "pending"}
                </Show>
            </div>
            <div class="bar">
                <button
                    data-testid="export-propose"
                    onClick={() => export_({ ProposeExport: { source_required: SEED } })}
                >
                    propose
                </button>
                <For each={exportRequired()}>
                    {(s) => (
                        <button data-testid={`export-source-${s}`} onClick={() => export_({ SourceConsent: s })}>
                            source {s}
                        </button>
                    )}
                </For>
                <button data-testid="export-target-admit" onClick={() => export_("TargetAdmit")}>
                    target admit
                </button>
                <button data-testid="export-export" onClick={() => export_("Export")}>export</button>
            </div>

            <Show when={err()}>
                <p class="status" data-shelf-error>{err()}</p>
            </Show>
        </div>
    );
}
