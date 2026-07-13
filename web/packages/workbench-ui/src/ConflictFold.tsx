/**
 * The conflict fold (SUB-6): when a base-carrying save finds real divergence,
 * the server returns the merged/conflicted spans instead of writing anything.
 * This panel renders them in document order — merged spans as quiet context,
 * each conflict region as a side-by-side choice between the assistant's
 * version (`ours` = the file as it stands) and yours (`theirs` = your draft),
 * with a free-text override for hand-blending. Resolving composes the final
 * text locally and re-saves against the file's CURRENT body, so the resolve
 * itself is race-checked too. Nothing is chosen for you: resolve stays
 * disabled until every region has an answer.
 */

import { createSignal, For } from "solid-js";
import { type MergePiece, type RegionResolution } from "@gaugewright/control-plane-client";

export type RegionChoice =
    | { pick: "ours" }
    | { pick: "theirs" }
    | { pick: "custom"; text: string };

function choiceText(piece: Extract<MergePiece, { kind: "conflict" }>, choice: RegionChoice): string {
    if (choice.pick === "ours") return piece.ours_text;
    if (choice.pick === "theirs") return piece.theirs_text;
    return choice.text;
}

/** Compose the resolved document from the pieces and one choice per
 *  conflict region (in region order). Pure — unit-tested. */
export function composeResolution(pieces: MergePiece[], choices: RegionChoice[]): string {
    let regionIndex = 0;
    let out = "";
    for (const piece of pieces) {
        if (piece.kind === "merged") {
            out += piece.text;
            continue;
        }
        const choice = choices[regionIndex];
        regionIndex += 1;
        if (!choice) throw new Error(`no choice for conflict region ${regionIndex}`);
        out += choiceText(piece, choice);
    }
    return out;
}

/** The settled triples the resolve carries (§12.2): for each conflict
 *  region, the exact three texts the user saw plus the text they chose —
 *  the server records them as durable region-resolution memory, so the
 *  same divergence never re-asks. Pure — unit-tested. */
export function regionResolutions(
    pieces: MergePiece[],
    choices: RegionChoice[],
): RegionResolution[] {
    let regionIndex = 0;
    const out: RegionResolution[] = [];
    for (const piece of pieces) {
        if (piece.kind === "merged") continue;
        const choice = choices[regionIndex];
        regionIndex += 1;
        if (!choice) throw new Error(`no choice for conflict region ${regionIndex}`);
        out.push({
            base_text: piece.base_text,
            ours_text: piece.ours_text,
            theirs_text: piece.theirs_text,
            resolution_text: choiceText(piece, choice),
        });
    }
    return out;
}

export function conflictRegionCount(pieces: MergePiece[]): number {
    return pieces.filter((piece) => piece.kind === "conflict").length;
}

export function ConflictFold(props: {
    pieces: MergePiece[];
    onResolve: (resolved: string, resolutions: RegionResolution[]) => void;
    onCancel: () => void;
}) {
    const regions = () => conflictRegionCount(props.pieces);
    const [choices, setChoices] = createSignal<(RegionChoice | null)[]>(
        Array.from({ length: conflictRegionCount(props.pieces) }, () => null),
    );
    const choose = (index: number, choice: RegionChoice) =>
        setChoices((prev) => prev.map((existing, i) => (i === index ? choice : existing)));
    const ready = () => choices().every((choice) => choice !== null);
    const resolve = () => {
        const chosen = choices();
        if (!chosen.every((choice): choice is RegionChoice => choice !== null)) return;
        props.onResolve(
            composeResolution(props.pieces, chosen),
            regionResolutions(props.pieces, chosen),
        );
    };

    let regionCounter = -1;
    return (
        <div class="conflict-fold" data-conflict-fold>
            <div class="conflict-fold-head">
                <span class="status" data-conflict-summary>
                    The assistant changed this file while you were editing, and{" "}
                    {regions() === 1 ? "one part conflicts" : `${regions()} parts conflict`} with
                    your edit. Everything else merged. Pick a version for each part below.
                </span>
                <div class="editor-actions">
                    <button data-conflict-cancel onClick={() => props.onCancel()}>
                        back to editing
                    </button>
                    <button
                        class="save"
                        data-conflict-resolve
                        disabled={!ready()}
                        onClick={resolve}
                    >
                        save resolved
                    </button>
                </div>
            </div>
            <div class="conflict-fold-body">
                <For each={props.pieces}>
                    {(piece) => {
                        if (piece.kind === "merged") {
                            return (
                                <pre class="fold-context" data-fold-merged data-provenance={piece.provenance}>
                                    {piece.text}
                                </pre>
                            );
                        }
                        regionCounter += 1;
                        const index = regionCounter;
                        const picked = () => choices()[index];
                        return (
                            <div class="fold-region" data-fold-conflict>
                                <div class="fold-option" classList={{ picked: picked()?.pick === "ours" }}>
                                    <label>
                                        <input
                                            type="radio"
                                            name={`region-${index}`}
                                            data-fold-pick-ours
                                            onChange={() => choose(index, { pick: "ours" })}
                                        />
                                        the assistant's version
                                    </label>
                                    <pre>{piece.ours_text}</pre>
                                </div>
                                <div class="fold-option" classList={{ picked: picked()?.pick === "theirs" }}>
                                    <label>
                                        <input
                                            type="radio"
                                            name={`region-${index}`}
                                            data-fold-pick-theirs
                                            onChange={() => choose(index, { pick: "theirs" })}
                                        />
                                        your version
                                    </label>
                                    <pre>{piece.theirs_text}</pre>
                                </div>
                                <details class="fold-custom">
                                    <summary>write it yourself (starts from the original)</summary>
                                    <textarea
                                        data-fold-custom
                                        value={(() => {
                                            const choice = picked();
                                            return choice?.pick === "custom" ? choice.text : piece.base_text;
                                        })()}
                                        onInput={(e) =>
                                            choose(index, { pick: "custom", text: e.currentTarget.value })
                                        }
                                    />
                                </details>
                            </div>
                        );
                    }}
                </For>
            </div>
        </div>
    );
}
