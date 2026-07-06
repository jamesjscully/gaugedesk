/**
 * Open-source settings slot for the workbench build. It keeps local/free surfaces
 * reachable while excluding enterprise governance and private managed-service UI
 * modules from the open bundle.
 */

import { createEffect, createSignal, on, Show, type Accessor, type JSX } from "solid-js";
import { AccountPanel, type AccountPanelApi } from "./AccountPanel";
import { DevicesModal, type DevicesModalApi } from "./DevicesModal";

export interface SettingsMenuApi extends AccountPanelApi, DevicesModalApi {}

export function SettingsMenu(props: {
    api: SettingsMenuApi;
    environment?: string;
    /** A monotonically increasing counter; each increment opens the Account panel.
     *  Lets another surface (e.g. an in-chat "no model" prompt) open settings. */
    openAccount?: Accessor<number>;
}): JSX.Element {
    const [menuOpen, setMenuOpen] = createSignal(false);
    const [devicesOpen, setDevicesOpen] = createSignal(false);
    const [accountOpen, setAccountOpen] = createSignal(false);

    // Open the Account panel when an external request comes in (defer the initial run
    // so we never pop it open on mount).
    createEffect(
        on(
            () => props.openAccount?.() ?? 0,
            () => {
                setMenuOpen(false);
                setAccountOpen(true);
            },
            { defer: true },
        ),
    );

    return (
        <div class="settings-anchor">
            <button
                type="button"
                class="settings-gear"
                classList={{ active: menuOpen() }}
                data-settings
                title="Settings"
                aria-label="Settings"
                aria-haspopup="menu"
                aria-expanded={menuOpen()}
                onClick={() => setMenuOpen((o) => !o)}
            >
                ⚙
            </button>

            <Show when={menuOpen()}>
                <div class="popover-catcher" onClick={() => setMenuOpen(false)} />
                <div
                    class="settings-popover"
                    role="menu"
                    data-settings-menu
                    data-open-settings-menu
                    onKeyDown={(e) => e.key === "Escape" && setMenuOpen(false)}
                >
                    <button
                        type="button"
                        class="settings-option"
                        role="menuitem"
                        data-settings-devices
                        title="Your devices, and connecting a separate party"
                        onClick={() => {
                            setMenuOpen(false);
                            setDevicesOpen(true);
                        }}
                    >
                        Devices
                    </button>
                    <button
                        type="button"
                        class="settings-option"
                        role="menuitem"
                        data-settings-account
                        title="Your linked AI accounts, devices, and settings"
                        onClick={() => {
                            setMenuOpen(false);
                            setAccountOpen(true);
                        }}
                    >
                        Your account
                    </button>
                </div>
            </Show>

            <Show when={devicesOpen()}>
                <DevicesModal
                    api={props.api}
                    environment={props.environment}
                    onClose={() => setDevicesOpen(false)}
                />
            </Show>

            <Show when={accountOpen()}>
                <AccountPanel api={props.api} onClose={() => setAccountOpen(false)} />
            </Show>
        </div>
    );
}
