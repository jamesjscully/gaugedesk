/**
 * The single **Devices** modal (FED-7 consolidation) — *everything* needed to manage
 * devices and connections, opened from **Settings ▸ Devices** (the gear in the Browse
 * tab's bottom bar). It folds the two formerly-separate surfaces into one, but keeps the
 * two **trust acts** clearly sectioned, because they are not the same thing:
 *
 * - **Your devices** — add one of *your own* phones/tablets (a device subkey under the
 *   *same* authority, fully trusted).
 * - **Connect a separate party** — pair a *different* authority's machine (the client),
 *   admission-gated; plus the **incoming-consent** queue (a project a peer wants to set
 *   up here) and standing auto-accept.
 *
 * Every mutation is a control-plane command; the modal renders projections and submits
 * (`INV-5`). Per-project hand-off / co-drive live in the project's `EngagementPane`.
 */

import { createResource, createSignal, For, onMount, Show, type JSX } from "solid-js";
import type {
    EnrollmentStatus,
    EnrollmentTicket,
    FederationPeer,
    HandoffStatus,
    IncomingHandoff,
    InviteAcceptResult,
    PairingTicket,
} from "@gaugewright/control-plane-client";
import { pairingTicket } from "./pairing";
import { qrSvg } from "./qr-code";

/** A short, URL-safe device id for a freshly-minted own-device ticket (browser-side).
 * The id is non-secret (it travels in plaintext / QR; the trust anchor is the signed
 * BridgeGrant), but we mint it from the CSPRNG so a scanner never flags `Math.random`
 * on an identity-adjacent path. */
function freshDeviceId(): string {
    const bytes = new Uint8Array(6);
    crypto.getRandomValues(bytes);
    const rand = Array.from(bytes, (b) => b.toString(36).padStart(2, "0")).join("").slice(0, 6);
    return `device:${rand}`;
}

export function DevicesModal(props: {
    api: DevicesModalApi;
    environment?: string;
    /** FED-7: seed the invite paste field from an OS-delivered `gaugewright://invite` deep link,
     *  so the consent preview renders as if the link had been pasted. */
    initialInviteLink?: string;
    onClose: () => void;
}): JSX.Element {
    const [status, setStatus] = createSignal("");
    // Own-device join ticket (a device subkey under this authority).
    const [deviceTicket, setDeviceTicket] = createSignal("");
    const [deviceCopied, setDeviceCopied] = createSignal(false);
    // Separate-party pairing ticket (a peer authority).
    const [peerTicket, setPeerTicket] = createSignal("");
    const [peerCopied, setPeerCopied] = createSignal(false);
    const [pasted, setPasted] = createSignal("");
    const [inviteLink, setInviteLink] = createSignal(props.initialInviteLink ?? "");

    // Device-enrollment handshake (ACCT-1). The holder hosts a session, shows the ticket,
    // then compares the 6-char SAS with the new device before confirming; the new device
    // pastes the ticket, shows its SAS, and waits. A relay-substituted subkey shows up as a
    // mismatched SAS the human catches — so the confirm is the load-bearing act.
    const [hostTicket, setHostTicket] = createSignal<EnrollmentTicket | null>(null);
    const [hostStatus, setHostStatus] = createSignal<EnrollmentStatus | null>(null);
    const [joinTicketText, setJoinTicketText] = createSignal("");
    const [joinStatus, setJoinStatus] = createSignal<EnrollmentStatus | null>(null);
    // The session currently being polled per role (a restart cancels the old loop).
    let hostSession = "";
    let joinSession = "";
    const terminal = (phase?: string) =>
        phase === "completed" || phase === "failed" || phase === "expired";
    const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

    const startHost = async () => {
        try {
            const ticket = await props.api.enrollHost();
            setHostTicket(ticket);
            setHostStatus(null);
            hostSession = ticket.session;
            void pollHost(ticket.session);
        } catch (e) {
            setStatus(`add-a-device failed: ${e}`);
        }
    };
    const pollHost = async (session: string) => {
        while (hostSession === session) {
            try {
                const s = await props.api.enrollHostStatus(session);
                if (hostSession !== session) return;
                setHostStatus(s);
                if (terminal(s.phase)) return;
            } catch {
                /* keep polling — a transient read is not fatal */
            }
            await sleep(1000);
        }
    };
    const confirmHost = async () => {
        const t = hostTicket();
        if (!t) return;
        try {
            await props.api.enrollAuthorize(t.session);
        } catch (e) {
            setStatus(`confirm failed: ${e}`);
        }
    };

    const startJoin = async () => {
        let ticket: EnrollmentTicket;
        try {
            ticket = JSON.parse(joinTicketText()) as EnrollmentTicket;
        } catch {
            setStatus("that does not look like a device ticket");
            return;
        }
        try {
            const session = await props.api.enrollJoin(ticket);
            setJoinStatus(null);
            joinSession = session;
            void pollJoin(session);
        } catch (e) {
            setStatus(`join failed: ${e}`);
        }
    };
    const pollJoin = async (session: string) => {
        while (joinSession === session) {
            try {
                const s = await props.api.enrollJoinStatus(session);
                if (joinSession !== session) return;
                setJoinStatus(s);
                if (terminal(s.phase)) return;
            } catch {
                /* keep polling */
            }
            await sleep(1000);
        }
    };

    const [peers, { refetch: refetchPeers }] = createResource(() => props.api.listPeers());
    const [incoming, { refetch: refetchIncoming }] = createResource(() =>
        props.api.handoffIncoming(),
    );

    const newDeviceCode = () => {
        setDeviceTicket(pairingTicket(props.environment ?? "local", freshDeviceId()));
        setDeviceCopied(false);
    };
    const mintPeerTicket = async () => {
        try {
            setPeerTicket(JSON.stringify(await props.api.mintPairingTicket()));
            setPeerCopied(false);
        } catch (e) {
            setStatus(`mint failed: ${e}`);
        }
    };
    onMount(() => {
        newDeviceCode();
        void mintPeerTicket();
    });

    const copyDevice = async () => {
        try {
            await navigator.clipboard?.writeText(deviceTicket());
            setDeviceCopied(true);
        } catch {
            /* selectable regardless */
        }
    };
    const copyPeer = async () => {
        try {
            await navigator.clipboard?.writeText(peerTicket());
            setPeerCopied(true);
        } catch {
            /* selectable regardless */
        }
    };
    const acceptPaste = async () => {
        try {
            const peer = await props.api.pair(JSON.parse(pasted()) as PairingTicket);
            setStatus(`paired with ${peer.authority}`);
            setPasted("");
            refetchPeers();
        } catch (e) {
            setStatus(`pair failed: ${e}`);
        }
    };
    const preauthPeer = async (peer: string) => {
        try {
            await props.api.handoffPreauth(peer, true);
            setStatus(`auto-accept handoffs from ${peer}: on`);
        } catch (e) {
            setStatus(`preauth failed: ${e}`);
        }
    };
    const acceptIncoming = async (project: string, source: string) => {
        try {
            const s = await props.api.handoffAccept(project, source);
            setStatus(`accepted ${project} from ${source}: ${s.phase}`);
            refetchIncoming();
        } catch (e) {
            setStatus(`accept failed: ${e}`);
        }
    };
    const declineIncoming = async (project: string, source: string) => {
        try {
            await props.api.handoffDecline(project, source);
            setStatus(`declined ${project} from ${source}`);
            refetchIncoming();
        } catch (e) {
            setStatus(`decline failed: ${e}`);
        }
    };
    const acceptAllIncoming = async () => {
        try {
            const accepted = await props.api.handoffAcceptAll();
            setStatus(`accepted ${accepted.length} handoff(s): ${accepted.join(", ")}`);
            refetchIncoming();
        } catch (e) {
            setStatus(`accept-all failed: ${e}`);
        }
    };

    // Decode an `gaugewright://invite?d=<hex>` link to its consent preview (origin, project,
    // confirm code) — never archetype bodies (INV-10), only the disclosed manifest.
    const decodedInvite = () => {
        const raw = inviteLink().trim();
        if (!raw.startsWith("gaugewright://invite")) return null;
        try {
            const hex = raw.split("d=").pop() ?? "";
            const bytes = hex.match(/.{1,2}/g)?.map((b) => parseInt(b, 16)) ?? [];
            const json = new TextDecoder().decode(new Uint8Array(bytes));
            return JSON.parse(json) as {
                project_name?: string;
                ticket?: { authority?: string };
                confirm_code?: string;
                manifest?: string[];
            };
        } catch {
            return null;
        }
    };
    const acceptInvite = async () => {
        try {
            const r = await props.api.inviteAccept(inviteLink().trim());
            if (r.ok) {
                setStatus(
                    `accepted — ${r.origin} set up "${r.project_name ?? r.project}" here · code ${r.confirm_code}`,
                );
                setInviteLink("");
                refetchPeers();
            } else {
                setStatus(`accept declined: ${r.reason ?? "verification failed"}`);
            }
        } catch (e) {
            setStatus(`accept failed: ${e}`);
        }
    };

    return (
        <div class="modal-overlay" data-devices-modal onClick={() => props.onClose()}>
            <div
                class="modal devices-modal"
                onClick={(e) => e.stopPropagation()}
                onKeyDown={(e) => e.key === "Escape" && props.onClose()}
            >
                <div class="modal-head">
                    <h3>Devices</h3>
                    <button type="button" onClick={() => props.onClose()}>close</button>
                </div>

                {/* Incoming consent — a separate party wants to set up a project here. */}
                <Show when={(incoming() ?? []).length > 0}>
                    <p class="status" style={{ margin: "0 0 4px" }}>
                        <strong>A device wants to set up a project here — your consent is required:</strong>
                        <Show when={(incoming() ?? []).length > 1}>
                            {" "}
                            <button type="button" class="tree-action" data-pd-accept-all onClick={() => void acceptAllIncoming()}>
                                accept all
                            </button>
                        </Show>
                    </p>
                    <ul class="fed-incoming" data-pd-incoming>
                        <For each={incoming() ?? []}>
                            {(h) => (
                                <li class="fed-incoming-item" data-pd-incoming-item={h.project}>
                                    <span>
                                        <strong>{h.source}</strong> wants to set up <strong>{h.project}</strong>{" "}
                                        on this device. It runs their agents on data you connect; nothing leaves
                                        without your approval.
                                    </span>
                                    <button type="button" class="tree-action" data-pd-accept={h.project} onClick={() => void acceptIncoming(h.project, h.source)}>
                                        accept
                                    </button>
                                    <button type="button" class="tree-action" data-pd-decline={h.project} onClick={() => void declineIncoming(h.project, h.source)}>
                                        decline
                                    </button>
                                </li>
                            )}
                        </For>
                    </ul>
                </Show>

                {/* Your devices — add one of YOUR OWN phones/tablets (scan-friendly). */}
                <h4>Your devices</h4>
                <p class="status" style={{ margin: "0 0 8px" }}>
                    Add your own phone or tablet to this workspace — scan this with its camera, or
                    enter the code:
                </p>
                <div class="pd-qr" data-pair-qr innerHTML={deviceTicket() ? qrSvg(deviceTicket()) : ""} />
                <code class="pair-ticket" data-pair-ticket>{deviceTicket()}</code>
                <div class="pair-device-actions">
                    <button type="button" class="tree-action" data-pair-copy onClick={() => void copyDevice()}>
                        {deviceCopied() ? "copied ✓" : "copy code"}
                    </button>
                    <button type="button" class="tree-action" data-pair-regenerate onClick={newDeviceCode}>
                        new code
                    </button>
                </div>

                {/* Secure enrollment handshake (ACCT-1): the holder shows a ticket + compares
                    the SAS before authorizing; the account key is transferred sealed. */}
                <h4 style={{ "margin-top": "16px" }}>Add a device securely</h4>
                <p class="status" style={{ margin: "0 0 8px" }}>
                    Enroll a new device into this account. Show it this ticket, then compare the
                    6-digit code on both screens before you confirm — it must match.
                </p>
                <Show
                    when={hostTicket()}
                    fallback={
                        <button type="button" class="tree-action" data-enroll-host-start onClick={() => void startHost()}>
                            add a device
                        </button>
                    }
                >
                    {(t) => (
                        <div data-enroll-host>
                            <div
                                class="pd-qr"
                                data-enroll-host-qr
                                innerHTML={qrSvg(JSON.stringify(t()))}
                            />
                            <code class="pair-ticket" data-enroll-host-ticket>{JSON.stringify(t())}</code>
                            <Show
                                when={hostStatus()?.sas}
                                fallback={
                                    <p class="status" data-enroll-host-waiting>
                                        Waiting for the new device to connect…
                                    </p>
                                }
                            >
                                {(sas) => (
                                    <div class="engagement-invite" data-enroll-host-sas>
                                        <Show
                                            when={hostStatus()?.phase === "completed"}
                                            fallback={
                                                <>
                                                    <p class="status">
                                                        Compare this code with the new device:{" "}
                                                        <strong data-enroll-host-code>{sas()}</strong>
                                                    </p>
                                                    <button
                                                        type="button"
                                                        class="tree-action"
                                                        data-enroll-host-confirm
                                                        onClick={() => void confirmHost()}
                                                    >
                                                        it matches — confirm &amp; authorize
                                                    </button>
                                                </>
                                            }
                                        >
                                            <p class="status" data-enroll-host-done>
                                                Device enrolled ✓
                                            </p>
                                        </Show>
                                    </div>
                                )}
                            </Show>
                        </div>
                    )}
                </Show>

                <h4 style={{ "margin-top": "16px" }}>Join this account</h4>
                <p class="status" style={{ margin: "0 0 8px" }}>
                    On a new device: paste the ticket from an already-enrolled device, then check the
                    6-digit code matches before the other device confirms.
                </p>
                <textarea
                    class="fed-paste"
                    data-enroll-join-ticket
                    rows={2}
                    value={joinTicketText()}
                    placeholder="paste a device ticket"
                    onInput={(e) => setJoinTicketText(e.currentTarget.value)}
                />
                <button type="button" class="tree-action" data-enroll-join-start onClick={() => void startJoin()}>
                    join
                </button>
                <Show when={joinStatus()?.sas}>
                    {(sas) => (
                        <div class="engagement-invite" data-enroll-join-sas>
                            <Show
                                when={joinStatus()?.phase === "completed"}
                                fallback={
                                    <p class="status">
                                        Your code: <strong data-enroll-join-code>{sas()}</strong> — check it
                                        matches the other device, then wait for it to confirm.
                                    </p>
                                }
                            >
                                <p class="status" data-enroll-join-done>
                                    Enrolled ✓ — this device now shares the account.
                                </p>
                            </Show>
                        </div>
                    )}
                </Show>

                <hr class="devices-sep" />

                {/* Connect a separate party — a different authority (the client). This is
                    a desktop↔desktop / invite-link flow: codes are shared, not scanned, so
                    there is no QR here (only "Your devices" above is scan-friendly). */}
                <h4>Connect a separate party</h4>

                <p class="status" style={{ margin: "0 0 4px" }}>
                    <strong>Have an invite link?</strong> Paste it to set up the project here in one step:
                </p>
                <textarea
                    class="fed-paste"
                    data-pd-invite-link
                    rows={2}
                    value={inviteLink()}
                    placeholder="paste an gaugewright://invite link"
                    onInput={(e) => setInviteLink(e.currentTarget.value)}
                />
                <Show when={decodedInvite()}>
                    {(d) => (
                        <div class="engagement-invite" data-pd-invite-consent>
                            <p class="status">
                                <strong>{d().ticket?.authority}</strong> wants to set up{" "}
                                <strong>{d().project_name}</strong> on this device. They can run their agents on
                                data you connect and see released results; they cannot read your files or take
                                anything off this machine unasked.
                            </p>
                            <Show when={(d().manifest ?? []).length > 0}>
                                <p class="status">Archetypes: {(d().manifest ?? []).join(", ")}</p>
                            </Show>
                            <p class="status">
                                Confirm code: <strong>{d().confirm_code}</strong> — check it matches what they
                                read you.
                            </p>
                        </div>
                    )}
                </Show>
                <button type="button" class="tree-action" data-pd-invite-accept onClick={() => void acceptInvite()}>
                    Accept &amp; set up
                </button>

                <p class="status" style={{ margin: "12px 0 4px" }}>
                    <strong>Or pair by code.</strong> Send them your code, or paste theirs (it pins your
                    key + cert):
                </p>
                <div class="pair-by-code">
                    <code class="pair-ticket" data-pd-ticket>{peerTicket()}</code>
                    <div class="pair-device-actions">
                        <button type="button" class="tree-action" data-pd-copy onClick={() => void copyPeer()}>
                            {peerCopied() ? "copied ✓" : "copy your code"}
                        </button>
                        <button type="button" class="tree-action" onClick={() => void mintPeerTicket()}>
                            new code
                        </button>
                    </div>
                </div>
                <textarea
                    class="fed-paste"
                    data-pd-paste
                    rows={2}
                    value={pasted()}
                    placeholder="paste their code"
                    onInput={(e) => setPasted(e.currentTarget.value)}
                />
                <button type="button" class="tree-action" data-pd-pair onClick={() => void acceptPaste()}>
                    pair
                </button>

                {/* Connected — paired separate parties + standing auto-accept. */}
                <p class="status" style={{ margin: "12px 0 4px" }}>Connected:</p>
                <ul class="fed-peers" data-pd-peers>
                    <For each={peers() ?? []} fallback={<li class="status">No device paired yet.</li>}>
                        {(p) => (
                            <li class="fed-peer" data-pd-peer={p.authority}>
                                <span class="fed-peer-name">{p.authority}</span>
                                <span class="fed-peer-grant" title={p.grant_id}>
                                    {p.active ? "paired" : "revoked"}
                                </span>
                                <button type="button" class="tree-action" data-pd-preauth={p.authority} title="Auto-accept handoffs from this device" onClick={() => void preauthPeer(p.authority)}>
                                    auto-accept
                                </button>
                            </li>
                        )}
                    </For>
                </ul>

                <p class="status" data-pd-status>{status()}</p>
            </div>
        </div>
    );
}

export interface DevicesModalApi {
    // Device-enrollment handshake (ACCT-1, ADR 0055): the holder hosts + authorizes on a
    // matched SAS; the new device joins and recovers the account key (sealed over the broker).
    enrollHost(): Promise<EnrollmentTicket>;
    enrollHostStatus(session: string): Promise<EnrollmentStatus>;
    enrollAuthorize(session: string): Promise<void>;
    enrollJoin(ticket: EnrollmentTicket): Promise<string>;
    enrollJoinStatus(session: string): Promise<EnrollmentStatus>;
    listPeers(): Promise<FederationPeer[]>;
    handoffIncoming(): Promise<IncomingHandoff[]>;
    mintPairingTicket(): Promise<PairingTicket>;
    pair(ticket: PairingTicket): Promise<FederationPeer>;
    handoffPreauth(peer: string, allow?: boolean): Promise<void>;
    handoffAccept(project: string, source: string): Promise<HandoffStatus>;
    handoffDecline(project: string, source: string): Promise<void>;
    handoffAcceptAll(): Promise<string[]>;
    inviteAccept(invite: string): Promise<InviteAcceptResult>;
}
