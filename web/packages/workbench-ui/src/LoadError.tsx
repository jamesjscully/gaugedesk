/**
 * A consistent "couldn't load — retry" state for a failed projection fetch
 * (round-12 D). Panels back their content with `createResource`; on a fetch error
 * the accessor stays `undefined`, so a bare `<Show when={data()} fallback="loading…">`
 * hangs on "loading…" forever with no way out. Every such panel renders this
 * instead when the resource errored — the same honest, retryable state the chat
 * pane's freshness banner already shows, so failure reads the same everywhere.
 */

import { type JSX } from "solid-js";

export function LoadError(props: { onRetry: () => void; what?: string }): JSX.Element {
    return (
        <div class="status load-error" data-load-error>
            Couldn't load {props.what ?? "this"} — nothing current to show.{" "}
            <button class="link-button" data-load-retry onClick={() => props.onRetry()}>
                retry
            </button>
        </div>
    );
}
