/**
 * The mobile **Pairing flow** (`mobile-client.md`, "Pairing", MOB-026): the
 * screen a device walks while binding itself to an environment — QR/code entry,
 * the approval wait, then success (or an explicit failure with a retry). It is the
 * surface the deep-link resolver routes a `needs-pairing` resolution to
 * (`deep-link-resolver.ts`, MOB-022) and the repair path the connection state
 * machine's `unpaired` status resolves into (`connection.ts`, MOB-018).
 *
 * Like {@link MobileContent} / {@link MobileFiles} this is a thin renderer: every
 * "what step are we on, and what may the user do?" decision is a pure function in
 * `pairing.ts` (`presentPairing`), so this component only wires Solid's `<Switch>`
 * onto the derived presentation and raises intents to the host. It owns no
 * transport — the host runs the two fetches (`POST /pairing-requests`, polling
 * `GET /pairing-status/:id`) and feeds their results back as pairing events.
 */

import { Show, Switch, Match, type JSX } from "solid-js";
import { presentPairing, type PairingState } from "./pairing";

export interface PairingFlowProps {
    /** The pairing flow's state (the host reduces `pairing.ts` over its events). */
    readonly state: PairingState;
    /** The raw QR/code payload the user scanned or typed; the host parses it into
     *  a ticket and raises `ticket-entered`. Called when the user submits entry. */
    readonly onSubmitTicket: (raw: string) => void;
    /** Start the flow over from a failed attempt (raises `retry`). */
    readonly onRetry: () => void;
    /** Dismiss a settled flow — hand off to the carousel on success, or back out
     *  of the flow. Offered once the flow is `paired` or `failed`. */
    readonly onDismiss: () => void;
}

export function PairingFlow(props: PairingFlowProps): JSX.Element {
    const view = () => presentPairing(props.state);
    let entryField: HTMLInputElement | undefined;

    return (
        <div class="pairing-flow" data-pairing={view().step}>
            <Switch>
                {/* Entry: scan a QR or type the short code. */}
                <Match when={view().step === "entry"}>
                    <form
                        class="pairing-entry"
                        data-pairing-entry
                        onSubmit={(e) => {
                            e.preventDefault();
                            const raw = entryField?.value ?? "";
                            if (raw.trim().length > 0) props.onSubmitTicket(raw);
                        }}
                    >
                        <div class="pairing-head">pair this device</div>
                        <div class="status">scan the QR code or enter the pairing code</div>
                        <input
                            ref={entryField}
                            type="text"
                            class="pairing-code-input"
                            data-pairing-code
                            placeholder="gaugewright-pair://…"
                            aria-label="pairing code"
                        />
                        <button type="submit" class="pairing-submit" data-pairing-submit>
                            pair
                        </button>
                    </form>
                </Match>

                {/* Submitting: the request is in flight (no status yet). */}
                <Match when={view().step === "submitting"}>
                    <div class="pairing-waiting" data-pairing-submitting role="status">
                        <div class="status">requesting pairing…</div>
                    </div>
                </Match>

                {/* Approval wait: bound; waiting for the owner to accept. */}
                <Match when={view().step === "awaiting-approval"}>
                    <div class="pairing-waiting" data-pairing-awaiting role="status">
                        <div class="pairing-head">waiting for approval</div>
                        <div class="status">the owner must accept this device on their workbench</div>
                    </div>
                </Match>

                {/* Success: the owner accepted; the grant is held. */}
                <Match when={view().step === "paired"}>
                    <div class="pairing-success" data-pairing-paired>
                        <div class="pairing-head">✓ paired</div>
                        <div class="status">this device is now bound to the environment</div>
                        <button type="button" class="pairing-dismiss" onClick={() => props.onDismiss()}>
                            continue
                        </button>
                    </div>
                </Match>

                {/* Failure: an explicit reason and a retry back to entry. */}
                <Match when={view().step === "failed"}>
                    <div class="pairing-failed" data-pairing-failed>
                        <div class="pairing-head">pairing failed</div>
                        <div class="status" data-pairing-error>
                            {view().error ?? "the pairing could not be completed"}
                        </div>
                        <Show when={view().canRetry}>
                            <button
                                type="button"
                                class="pairing-retry"
                                data-pairing-retry
                                onClick={() => props.onRetry()}
                            >
                                try again
                            </button>
                        </Show>
                        <button type="button" class="pairing-dismiss" onClick={() => props.onDismiss()}>
                            cancel
                        </button>
                    </div>
                </Match>
            </Switch>
        </div>
    );
}
