/**
 * The content viewer/editor (3rd column, `navigation.md`): renders the selected
 * worktree file, or the turn's diff. Toggle above the area: **View · Edit · Diff**.
 *
 * The **Edit** tab is a full-panel editor with the **save** button top-right
 * (`Ctrl/Cmd+S` also saves). Editing undo/redo is the textarea's native, in-buffer
 * undo (`Ctrl+Z`). Each save commits to the engagement branch, so **git is the
 * file's durable version history** — surfaced through the Diff / promote-to-main
 * surface below, not a separate per-file history.
 *
 * The **Diff** is the *review surface for the merge lifecycle* (D1): the human
 * reviews the engagement-vs-`main` diff and **admits** (advance main) or
 * **rejects** (isolate); a conflict surfaces with repair/retry.
 */

import { createEffect, createMemo, createResource, createSignal, lazy, on, Show, Suspense } from "solid-js";
import { useSession } from "./session-context";
import { changedUserFiles, diffHasFiles } from "./changed-files";
import { defaultContentMode, isSettledPhase, phaseLabel as phaseLabelFor, shouldShowViewOnSelect } from "./content-view";
import { readPolicyDiff } from "./policy-diff";

// The diff viewer pulls in @git-diff-view (+ highlight.js/lowlight, ~350 KB).
// Load that chunk only when the Diff tab is first opened, not on app boot.
const DiffView = lazy(() => import("./DiffView").then((m) => ({ default: m.DiffView })));

type Mode = "view" | "edit" | "diff";

export function ContentViewer() {
    const session = useSession();
    // Local accessors over the injected Session (EMBED-1, ADR 0051 §3): the panel
    // reads its addressing + projections here, never from a desktop global — so the
    // body is unchanged but the panel mounts portably (desktop or embedded session).
    // Round-10 #3 — keep/review vocabulary is scope-specific: a work chat merges into
    // the project's shared copy; an "edit" chat updates a reusable method that applies
    // everywhere it runs, so we phrase keep/kept in method terms (naming the method).
    const id = () => session.engagementId();
    const file = () => session.selectedFile();
    const diff = () => session.diff();
    const mergePhase = () => session.mergePhase();
    // A `Rejected` merge can mean two different things: the user discarded the work, or git
    // couldn't merge it (a conflict). The copy must not say "you discarded" for a conflict.
    const conflicted = () => session.mergeConflicted();
    const chatKind = () => session.chatKind();
    const methodName = () => session.methodName();

    // Open on the file View by default; the Changes (diff) review surface leads only
    // when this chat has a review open (a "Clean" merge phase awaiting keep/discard).
    const [mode, setMode] = createSignal<Mode>(defaultContentMode(mergePhase() ?? null));
    const [content, { refetch }] = createResource(
        () => {
            const i = id();
            const f = file();
            return i && f ? ([i, f] as const) : null;
        },
        ([i, f]) => session.api.getFile(i, f),
    );
    const [draft, setDraft] = createSignal<string | null>(null);
    const [msg, setMsg] = createSignal("");
    const text = () => draft() ?? content() ?? "";

    // The default surface (the request): open on the file View, and show the
    // Changes (diff) review surface only while a review is open — a "Clean" merge
    // phase, i.e. a finished turn awaiting keep/discard. Re-applied whenever the
    // chat OR its review-state changes (keyed on both, so it's never stale), so a
    // chat with nothing to review never inherits the previous chat's Changes tab,
    // and a turn that opens a review surfaces it. A manual tab pick persists until
    // the next such change.
    createEffect(
        on(
            () => [id(), mergePhase() ?? null] as const,
            ([curId, phase], prev) => {
                // Switching chats: open on the resting default surface for that chat.
                if (!prev || prev[0] !== curId) {
                    setMode(defaultContentMode(phase));
                    return;
                }
                // Same chat. Only act on a real phase *transition*, never on a mere
                // merge-resource re-emit (e.g. the `refetchMerge` after a manual save,
                // which would otherwise yank the user out of the Edit tab — round-4).
                if (phase === prev[1]) return;
                // A review newly opened (a finished turn → "Clean"): surface the Changes
                // tab. We do NOT switch on a transition into "Rejected" (discard/conflict):
                // the merge-review bar (its discarded/conflict copy + repair affordance)
                // lives on the Changes tab, so staying put shows the honest outcome on the
                // tab they acted from (round3/round5/merge-conflict). View stays a manual pick.
                if (phase === "Clean") setMode("diff");
            },
        ),
    );

    // Selecting a file switches the viewer to View — UNLESS there's a pending
    // change to review (round-7 #3). When a turn just modified one file, App
    // auto-selects it so View is populated rather than showing a "pick a file"
    // hint; but the default review surface is Changes, so we must not yank the
    // user off an active "needs review" diff onto View just because the file
    // became selected. A user-initiated pick (no pending review, or a different
    // file) still drops them into View as before.
    createEffect(
        on(
            () => file(),
            (f) => {
                if (!f) return;
                setDraft(null);
                setMsg("");
                if (shouldShowViewOnSelect(mergePhase() ?? null)) setMode("view");
            },
        ),
    );

    // Keep View in sync with the working copy when the merge phase settles (#1
    // round-5). The honesty fix is the in-tab banner below (discard isolates, it
    // doesn't erase — so View truthfully still shows the text); this refetch just
    // makes sure View isn't serving a stale read after a repair/retry actually
    // rewrites the worktree. The `content` resource is keyed only on (id, file), so
    // without this a phase change wouldn't re-read the file.
    createEffect(
        on(
            () => mergePhase() ?? null,
            (p) => {
                if (isSettledPhase(p) && file() && draft() === null) void refetch();
            },
        ),
    );

    // The assistant's settings file governs what it is *allowed* to do. Editing it
    // directly used to accept garbage JSON silently (#1 round-4) — corrupt content
    // was persisted with no feedback or chance to undo. Validate a config/JSON file
    // before save: block + explain on a parse error, in one plain sentence (never a
    // raw parser line/column dump). This is the same guard the Settings modal needs;
    // it must not be bypassable just because the same file is reachable here.
    const isJsonFile = (path: string) => path.toLowerCase().endsWith(".json");
    // Source of truth for the config file name: the `definition` module in
    // `crates/boundary` (CONFIG_PATH) — a cross-language copy, kept in sync by hand.
    const isConfigFile = (path: string) => path.split("/").pop() === ".agent-config.json";

    async function save() {
        const i = id();
        const f = file();
        if (!i || !f) return;
        if (isJsonFile(f)) {
            try {
                JSON.parse(text());
            } catch {
                setMsg("Not saved — this isn't valid settings text. Check for a stray character or a missing comma, bracket, or quote.");
                return;
            }
        }
        try {
            await session.api.putFile(i, f, text());
            setMsg("saved");
            setDraft(null);
            await refetch();
            session.onContentSaved();
        } catch (e) {
            setMsg(String(e));
        }
    }

    function onKeyDown(e: KeyboardEvent) {
        if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
            e.preventDefault();
            void save();
        }
    }

    const phase = () => mergePhase() ?? "Idle";

    // The merge phase is a property of the *engagement* (the turn's diff), not of
    // whichever file the user is browsing. The discard banner must therefore be
    // scoped to the file(s) the discarded change actually touched (#1 round-9):
    // opening an unrelated, unchanged file (e.g. AGENTS.md) used to keep the
    // "you discarded these changes — the file below shows them" banner pinned over
    // it, asserting a destructive state about a file nothing happened to. We only
    // claim "the file below shows your unkept changes" when the file below is in
    // fact one of the changed files.
    const fileWasTouched = createMemo(() => {
        const f = file();
        if (!f) return false;
        return changedUserFiles(diff()).includes(f);
    });

    // The merge phase in plain words (#5): "Advanced" / "Idle" / "Integrated" are
    // internal state-machine tokens. Show what they mean to the user; the raw token
    // stays on `data-merge-phase` for tests/automation.
    // Round-10 #3 — keep/kept vocabulary is scope-specific. In an improve ("edit")
    // chat the kept change updates a reusable method and applies wherever that
    // method runs — saying "the shared copy" there is a category error (that's
    // project mechanics). Name the method when we have it so the user grasps the
    // (much broader) scope of what they just kept.
    const isImprove = () => chatKind() === "edit";
    const method = () => (methodName() ?? "").trim();
    const phaseLabel = (): string => phaseLabelFor(phase(), chatKind(), methodName());

    // Plain tab labels (#3): "diff" is dev-speak for the review of what changed.
    const tabLabel: Record<Mode, string> = { view: "view", edit: "edit", diff: "changes" };

    // Plain-language reading of a security/permission change buried in the config
    // file (#3): a change that only touches `.agent-config.json` is otherwise shown
    // as raw JSON — exactly the wrong representation for "this lets the assistant
    // run shell commands". We summarise the safety meaning above the diff, and gate
    // a one-click keep on a *loosening* change behind an explicit confirm.
    const policy = createMemo(() => readPolicyDiff(diff()));
    const loosening = createMemo(() => policy().notes.some((n) => n.direction === "loosen"));
    // Is there actually a diff to review? The keep/discard prompt and the diff body
    // must both hinge on this — otherwise the panel says "Review what changed, then:
    // [keep][discard]" over an empty "no changes" body (round-11 #3). When there's
    // nothing to review we show a single honest empty state and no merge controls.
    const hasChanges = createMemo(() => diffHasFiles(diff()));
    const [confirmKeep, setConfirmKeep] = createSignal(false);
    // Reset the confirm gate whenever a different change comes up for review.
    createEffect(on(() => diff(), () => setConfirmKeep(false)));

    return (
        <div class="viewer">
            <div class="tabs" data-viewer-tabs>
                {(["view", "edit", "diff"] as Mode[]).map((m) => (
                    <span class="tab" classList={{ active: mode() === m }} data-tab={m} onClick={() => setMode(m)}>
                        {tabLabel[m]}
                    </span>
                ))}
                <Show when={file() && mode() !== "edit"}>
                    <span class="status viewer-filename" title={file() ?? ""}>{file()}</span>
                </Show>
            </div>

            <Show when={mode() === "view"}>
                <Show when={file()} fallback={<div class="status">Pick a file from the Files panel on the right to view it.</div>}>
                    {/* Honest View after a discard (#1 round-5). Discarding *isolates*
                        the work — it does NOT erase it from this chat's private copy
                        (the backend's Reject keeps the engagement's files, only holding
                        them back from the shared copy). So View legitimately still shows
                        the text. The dishonesty was leaving that implicit: Changes said
                        "thrown away" while View silently rendered it as if nothing
                        happened. Tell the truth — the file still shows these unkept
                        changes on your private copy — instead of pretending it reverted. */}
                    <Show when={phase() === "Rejected" && fileWasTouched()}>
                        <div class="discarded-note" data-view-discarded>
                            <Show
                                when={conflicted()}
                                fallback="You discarded these changes — they won't be kept into the shared copy. This is still your private copy, so the file below shows them until you send a new request."
                            >
                                This change conflicted with the shared copy and couldn't be merged. It's preserved on your private copy — repair it in the changes view and try again; the file below shows it.
                            </Show>
                        </div>
                    </Show>
                    <pre class="filebody" data-file-view>{content() ?? ""}</pre>
                </Show>
            </Show>

            <Show when={mode() === "edit"}>
                <Show when={file()} fallback={<div class="status">Pick a file from the Files panel on the right to edit it.</div>}>
                    {/* Editing the assistant's settings file directly is a safety
                        surface, not an ordinary text edit (#1 round-4): say so plainly
                        and point at the safer place to change behaviour. */}
                    <Show when={isConfigFile(file()!)}>
                        <div class="policy-callout loosen" data-config-edit-warning>
                            <div class="policy-callout-head">This is the assistant's settings file</div>
                            <div class="status">It controls what the assistant is allowed to do. Editing it by hand is for advanced users — to change how this assistant behaves, use its <strong>settings</strong> instead. Invalid text won't be saved.</div>
                        </div>
                    </Show>
                    <div class="editor">
                        <div class="editor-head">
                            <span class="status" data-edit-file>{file()}</span>
                            <span class="status" data-edit-status>{msg()}</span>
                            <div class="editor-actions">
                                <button class="save" data-file-save onClick={save}>save</button>
                            </div>
                        </div>
                        <textarea
                            class="editor-text"
                            data-file-edit
                            aria-label={`Edit ${file()}`}
                            spellcheck={false}
                            value={text()}
                            onInput={(e) => setDraft(e.currentTarget.value)}
                            onKeyDown={onKeyDown}
                        />
                    </div>
                </Show>
            </Show>

            <Show when={mode() === "diff"}>
                {/* The merge review in plain words (#3, MEMORY #10): the human keeps
                    the work into the shared copy, or discards it — no admit/reject/
                    promote/main jargon. The underlying merge actions are unchanged. */}
                {/* Plain-language safety summary for a permission/policy change (#3).
                    When a change touches the (hidden) config file, the raw JSON diff
                    can't tell a layperson "this lets the assistant run shell commands".
                    We say it in words; a *loosening* change makes keep slow down. */}
                <Show when={phase() === "Clean" && policy().touchesConfig && policy().notes.length}>
                    <div class="policy-callout" classList={{ loosen: loosening() }} data-policy-callout>
                        <div class="policy-callout-head">{loosening() ? "This changes what the assistant is allowed to do" : "This adjusts the assistant's permissions"}</div>
                        <ul class="policy-callout-list">
                            {policy().notes.map((n) => (
                                <li classList={{ loosen: n.direction === "loosen" }}>{n.text}</li>
                            ))}
                        </ul>
                        <Show when={policy().onlyConfig}>
                            <div class="status">Only the assistant's settings changed — review the details below before keeping.</div>
                        </Show>
                    </div>
                </Show>
                <div class="bar merge-review" style={{ "margin-bottom": "10px" }}>
                    <Show
                        when={phase() === "Clean" && hasChanges()}
                        fallback={
                            // The Rejected/Repairing phases get their own dedicated rows
                            // below (with the start-over / try-again buttons), so don't
                            // also print phaseLabel() here — that printed the discarded
                            // sentence twice (#6 round-5). Only show the generic label
                            // for the in-between phases that have no dedicated row. A
                            // "Clean" phase with an EMPTY diff shows nothing here — the
                            // diff body below carries the single "no changes yet" state,
                            // so we never pair a keep/discard prompt with an empty diff.
                            <Show when={phase() !== "Rejected" && phase() !== "Repairing" && phase() !== "Clean"}>
                                <span class="status">
                                    <span class="phase" data-merge-phase={phase()}>{phaseLabel()}</span>
                                </span>
                            </Show>
                        }
                    >
                        <span class="status" data-merge-phase>Review what changed, then:</span>
                        {/* A loosening permission change is the one place the review
                            slows down: a two-step confirm, never a single blind click. */}
                        <Show
                            when={loosening() && !confirmKeep()}
                            fallback={
                                <button
                                    class="keep-btn"
                                    data-merge-admit
                                    title={
                                        isImprove()
                                            ? `Save this change to the ${method() || "archetype"} — it will apply everywhere the archetype is used`
                                            : "Keep these changes into the shared copy"
                                    }
                                    onClick={() => session.merge("admit")}
                                >
                                    {loosening()
                                        ? "Yes, keep this permission change"
                                        : isImprove()
                                            ? "save to the archetype"
                                            : "keep this work"}
                                </button>
                            }
                        >
                            <button class="keep-btn warn" data-merge-confirm-keep title="This changes the assistant's permissions — confirm before keeping" onClick={() => setConfirmKeep(true)}>keep this work…</button>
                        </Show>
                        <button data-merge-reject title="Throw these changes away — the shared copy is untouched" onClick={() => session.merge("reject")}>discard</button>
                    </Show>
                    <Show when={phase() === "Rejected"}>
                        {/* A `Rejected` merge is one of two things, and the copy must be honest
                            about which (UX-7): a git **conflict** (the change couldn't be merged
                            into the shared copy — the candidate is preserved for repair), or a
                            user **discard** (they chose to throw the work away). Both isolate
                            with a repair context, so the affordance is the same; only the framing
                            differs — never tell someone they "discarded" work that actually
                            conflicted. */}
                        <Show
                            when={conflicted()}
                            fallback={
                                <>
                                    <span class="status" data-merge-phase>You discarded these changes — nothing was kept.</span>
                                    <button data-merge-repair onClick={() => session.merge("repair")}>start over</button>
                                </>
                            }
                        >
                            <span class="status" data-merge-phase data-merge-conflict>This change conflicted with the shared copy and couldn't be merged — it's preserved for you to repair.</span>
                            <button data-merge-repair onClick={() => session.merge("repair")}>repair it</button>
                        </Show>
                    </Show>
                    <Show when={phase() === "Repairing"}>
                        <button data-merge-retry onClick={() => session.merge("retry")}>try again</button>
                    </Show>
                </div>
                {/* Once discarded, the changes are gone — don't keep rendering the
                    stale diff as if it were still live to keep/discard (#1). */}
                <Show when={phase() !== "Rejected"} fallback={<div class="status discarded-note">These changes were discarded. Send a new request to try again.</div>}>
                    {/* One honest empty state when there's nothing to review — never an
                        empty diff sitting under a keep/discard prompt (round-11 #3). */}
                    <Show
                        when={hasChanges()}
                        fallback={<div class="status diff-empty" data-diff-empty>Nothing has changed yet. When the assistant edits files, you'll see what changed here — ready to keep or discard.</div>}
                    >
                        <Suspense fallback={<div class="status">loading diff…</div>}>
                            <DiffView diff={diff()} />
                        </Suspense>
                    </Show>
                </Show>
            </Show>
        </div>
    );
}
