//! gaugewright git workspace — the instance `main` + engagement worktrees.
//!
//! The deterministic shell over `git` (CLI shell-out, not `git2` — `backend-stack.md`)
//! that gives the system its substrate:
//! - an **instance** owns a git repo whose `main` is settled state (`instance.md`);
//! - each **engagement** is a `git worktree` off `main` on its own branch plus a
//!   persistent Pi thread (`engagement.md`); many engagements share one `main`;
//! - a **turn** auto-commits the engagement's worktree; keeping an output merges
//!   the engagement branch back into `main` (the lean auto-apply-with-diff).
//!
//! Git is the only side-effecting dependency here; the lifecycle *decisions*
//! (when to commit, when to merge) live in the pure core. This crate just makes
//! the bytes move, and reports faithfully when `git` fails.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// A monotonic suffix for temp bundle files, so concurrent bundle operations in one
/// process never collide on a path.
static BUNDLE_SEQ: AtomicU64 = AtomicU64::new(0);

/// A scratch path for a `git bundle` file (the on-disk handoff content carrier).
fn temp_bundle_path() -> PathBuf {
    let n = BUNDLE_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "gaugewright-handoff-{}-{n}.bundle",
        std::process::id()
    ))
}

/// Neutral seam error. The impl mints the full human-readable message
/// (this git impl produces byte-identical strings to the legacy `GitError`
/// Display: "could not run git: {e}" / "git {command} failed: {stderr}").
#[derive(Debug)]
pub struct WorkspaceError {
    pub message: String,
}

impl WorkspaceError {
    /// `git` could not be spawned at all — byte-identical to the legacy
    /// `GitError::Spawn` Display. Also minted by the filesystem steps of git
    /// operations (init/seed/bundle), which keep the legacy bytes deliberately;
    /// only the worktree-FS facet mints bare io messages ([`Self::io`]).
    fn git_spawn(e: std::io::Error) -> Self {
        Self {
            message: format!("could not run git: {e}"),
        }
    }

    /// `git` ran but exited non-zero — byte-identical to the legacy
    /// `GitError::Failed` Display (stderr trimmed).
    fn git_failed(command: &str, stderr: &str) -> Self {
        Self {
            message: format!("git {command} failed: {}", stderr.trim()),
        }
    }

    /// A worktree-FS facet failure (`read_file`/`write_file`/`tree`/`ingest`):
    /// the bare io message — filesystem errors never claim git ran.
    fn io(e: std::io::Error) -> Self {
        Self {
            message: e.to_string(),
        }
    }

    /// A non-io facet failure with an impl-minted message.
    fn msg(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for WorkspaceError {}

type Result<T> = std::result::Result<T, WorkspaceError>;

/// A `git -C <dir>` command with gaugewright's identity pinned per-invocation, so
/// commits work without any global git config. The shared base for [`git`] and
/// [`git_ok`].
///
/// Resolve the `git` executable (SELFHOST-1). A packaged desktop bundle **vendors
/// git on every OS** (the engine shells out to the git CLI by settled choice —
/// `backend-stack.md`) and points here via `GAUGEWRIGHT_GIT_BIN`, which the Tauri shell
/// sets from the bundle's `resource_dir()`; the dev/test build leaves it unset and
/// finds `git` on PATH. Env-override-then-default, mirroring `resolve_pi_bin`.
fn git_bin() -> String {
    git_bin_from(std::env::var("GAUGEWRIGHT_GIT_BIN").ok())
}

/// The pure resolution (pulled out so it is testable without touching the process
/// env): a non-empty override wins, else `git` on PATH.
fn git_bin_from(override_var: Option<String>) -> String {
    override_var
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "git".into())
}

fn git_cmd(dir: &Path) -> Command {
    let mut cmd = Command::new(git_bin());
    cmd.arg("-C").arg(dir);
    cmd.args([
        "-c",
        "user.name=gaugewright",
        "-c",
        "user.email=agent@gaugewright.local",
    ]);
    cmd
}

/// Run `git -C <dir> <args...>`, returning trimmed stdout or a faithful error.
fn git<I, S>(dir: &Path, args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = git_cmd(dir);
    let collected: Vec<S> = args.into_iter().collect();
    cmd.args(&collected);

    let out = cmd.output().map_err(WorkspaceError::git_spawn)?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    } else {
        let shown = collected
            .iter()
            .map(|s| s.as_ref().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        Err(WorkspaceError::git_failed(
            &shown,
            &String::from_utf8_lossy(&out.stderr),
        ))
    }
}

/// Run a git command where a non-zero exit is *meaningful* (e.g. a merge
/// conflict), not an error. Returns whether it succeeded plus trimmed stdout.
fn git_ok<I, S>(dir: &Path, args: I) -> Result<(bool, String)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = git_cmd(dir);
    cmd.args(args.into_iter().collect::<Vec<S>>());
    let out = cmd.output().map_err(WorkspaceError::git_spawn)?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).trim_end().to_string(),
    ))
}

/// One entry in the worktree file tree (relative path; dir or file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    pub is_dir: bool,
}

/// The outcome of merging an engagement branch into the standing ref.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeOutcome {
    /// Merges cleanly (no conflicts).
    Clean,
    /// Conflicts — the merge is not applied; the engagement stays isolated.
    Conflict,
}

/// Opaque revision id minted by the impl (git: the auto-commit's short hash; a
/// future impl mints its own, e.g. a cut id). Callers never parse it; it crosses
/// back out as the same bare string, so `ContentLocator::Workspace { commit }`
/// and the event-log bytes are unchanged.
#[derive(Debug, Clone, PartialEq)]
pub struct RevisionId(pub String);

impl std::fmt::Display for RevisionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Opaque, impl-specific full-content export (git: `git bundle --all` bytes).
///
/// CONTRACT: same-impl re-materialization, idempotent; there is NO
/// byte-round-trip promise — a future impl MAY erasure-filter, so tombstoned
/// content never travels (ADR 0071 §2).
pub struct WorkspaceExport(pub Vec<u8>);

/// Opaque cross-instance source token for fork/pull. Minted only by an impl
/// ([`Instance::peer_source`]); callers never look inside. (git: wraps the
/// source repo path.)
pub struct PeerSource(PathBuf);

/// An instance: the repo that owns settled `main`, plus where its engagement
/// worktrees are placed.
pub struct Instance {
    repo: PathBuf,
    worktrees: PathBuf,
}

impl Instance {
    /// Initialize a fresh instance repo with an empty initial commit on `main`,
    /// and a directory under which engagement worktrees will be placed.
    pub fn init(repo: impl Into<PathBuf>, worktrees: impl Into<PathBuf>) -> Result<Self> {
        let repo = repo.into();
        let worktrees = worktrees.into();
        std::fs::create_dir_all(&repo).map_err(WorkspaceError::git_spawn)?;
        std::fs::create_dir_all(&worktrees).map_err(WorkspaceError::git_spawn)?;
        git(&repo, ["init", "-q", "-b", "main"])?;
        // An empty root commit so `main` exists and worktrees can branch off it.
        git(
            &repo,
            ["commit", "-q", "--allow-empty", "-m", "init instance"],
        )?;
        Ok(Self { repo, worktrees })
    }

    /// Open an existing instance repo.
    pub fn open(repo: impl Into<PathBuf>, worktrees: impl Into<PathBuf>) -> Self {
        Self {
            repo: repo.into(),
            worktrees: worktrees.into(),
        }
    }

    /// Initialize a fresh instance under a single directory, with the standard
    /// `dir/repo` + `dir/worktrees` layout — the convention every instance on
    /// disk follows (one repo per `(agent, workspace)` binding).
    pub fn init_at(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        Self::init(dir.join("repo"), dir.join("worktrees"))
    }

    /// Open an existing instance from its `dir/repo` + `dir/worktrees` layout.
    pub fn open_at(dir: impl AsRef<Path>) -> Self {
        let dir = dir.as_ref();
        Self::open(dir.join("repo"), dir.join("worktrees"))
    }

    pub fn repo(&self) -> &Path {
        &self.repo
    }

    /// Serialize the instance's **entire object graph** — `main` plus every
    /// `engagement/<id>` branch, with all reachable commits/trees/blobs — into a single
    /// opaque [`WorkspaceExport`] (a `git bundle` byte buffer). This is the project's
    /// content bytes for a [`handoff`]: the bytes behind every relocated handle, shipped
    /// to the target so they can be re-materialized there before its home commits
    /// (`STATE_BEFORE_HOME`). An export is opaque to the relay (`INV-14`) — and to every
    /// caller: only a same-impl re-materialization may interpret it (see
    /// [`WorkspaceExport`]'s contract; no byte-round-trip promise).
    ///
    /// [`handoff`]: ../../gaugewright_core/handoff/index.html
    pub fn export(&self) -> Result<WorkspaceExport> {
        let path = temp_bundle_path();
        let path_str = path.to_string_lossy().to_string();
        // `--all` captures every ref (main + all engagement branches) and the full
        // object closure they reach — the complete content graph in one file.
        git(&self.repo, ["bundle", "create", "-q", &path_str, "--all"])?;
        let bytes = std::fs::read(&path).map_err(WorkspaceError::git_spawn)?;
        let _ = std::fs::remove_file(&path);
        Ok(WorkspaceExport(bytes))
    }

    /// The impl-stable id of this impl's [`export`](Self::export) payload format —
    /// the future guard against re-materializing an export under a different impl
    /// (mixed-substrate handoff is not supported; ADR 0071).
    pub fn export_format(&self) -> &'static str {
        "git-bundle-v1"
    }

    /// Re-materialize an instance on this machine from a `git bundle` produced by
    /// [`export`](Self::export) — the target side of a [`handoff`] content
    /// relocation. Lays down `dir/repo` + `dir/worktrees`, imports `main` and every
    /// `engagement/<id>` branch from the bundle, and checks out `main`. The caller then
    /// runs [`reconcile_engagements`](Self::reconcile_engagements) to rehydrate the
    /// engagement worktrees. The bytes never re-derive state — the log is authority; this
    /// only places the content the log's handles point at.
    ///
    /// [`handoff`]: ../../gaugewright_core/handoff/index.html
    pub fn from_bundle(
        repo: impl Into<PathBuf>,
        worktrees: impl Into<PathBuf>,
        bundle: &[u8],
    ) -> Result<Self> {
        let repo = repo.into();
        let worktrees = worktrees.into();
        std::fs::create_dir_all(&repo).map_err(WorkspaceError::git_spawn)?;
        std::fs::create_dir_all(&worktrees).map_err(WorkspaceError::git_spawn)?;
        // An empty repo whose checked-out branch is a scratch ref *not* in the bundle,
        // so the fetch can create `main` without git refusing to update the branch HEAD
        // is on. No initial commit here (unlike `init`): the bundle provides history.
        git(&repo, ["init", "-q", "-b", "gaugewright-import-scratch"])?;
        let path = temp_bundle_path();
        std::fs::write(&path, bundle).map_err(WorkspaceError::git_spawn)?;
        let path_str = path.to_string_lossy().to_string();
        // Import every branch from the bundle (force: local `main` is unborn).
        let fetch = git(
            &repo,
            ["fetch", "-q", &path_str, "+refs/heads/*:refs/heads/*"],
        );
        let _ = std::fs::remove_file(&path);
        fetch?;
        // Populate the repo working tree from the imported `main`.
        git(&repo, ["checkout", "-q", "-f", "main"])?;
        Ok(Self { repo, worktrees })
    }

    /// Initialize a target-side instance from a content bundle under the standard
    /// `dir/repo` + `dir/worktrees` layout (the [`from_bundle`](Self::from_bundle)
    /// counterpart to [`init_at`](Self::init_at)).
    pub fn from_bundle_at(dir: impl AsRef<Path>, bundle: &[u8]) -> Result<Self> {
        let dir = dir.as_ref();
        Self::from_bundle(dir.join("repo"), dir.join("worktrees"), bundle)
    }

    /// The opaque token another instance uses to fork from / pull from this one
    /// — the cross-instance source handle. Callers pass it around; only this impl
    /// looks inside.
    pub fn peer_source(&self) -> PeerSource {
        PeerSource(self.repo.clone())
    }

    /// Create this instance's repo as a **fork** of `source`: `main` is fetched from
    /// the source so the fork *shares the source's history*. That shared ancestry is what
    /// lets a fork later [`pull_from`](Self::pull_from) the source with a real 3-way merge
    /// — a plain file copy shares no ancestry and could only be overwritten (ADR 0038).
    pub fn fork_from(
        repo: impl Into<PathBuf>,
        worktrees: impl Into<PathBuf>,
        source: &PeerSource,
    ) -> Result<Self> {
        let repo = repo.into();
        let worktrees = worktrees.into();
        std::fs::create_dir_all(&repo).map_err(WorkspaceError::git_spawn)?;
        std::fs::create_dir_all(&worktrees).map_err(WorkspaceError::git_spawn)?;
        // Scratch HEAD so the fetch can create `main` (the same dance as `from_bundle`).
        git(&repo, ["init", "-q", "-b", "gaugewright-fork-scratch"])?;
        let src = source.0.to_string_lossy().to_string();
        git(
            &repo,
            ["fetch", "-q", &src, "+refs/heads/main:refs/heads/main"],
        )?;
        git(&repo, ["checkout", "-q", "-f", "main"])?;
        Ok(Self { repo, worktrees })
    }

    /// Initialize a fork instance under the standard `dir/repo` + `dir/worktrees` layout
    /// (the [`fork_from`](Self::fork_from) counterpart to [`init_at`](Self::init_at)).
    pub fn fork_from_at(dir: impl AsRef<Path>, source: &PeerSource) -> Result<Self> {
        let dir = dir.as_ref();
        Self::fork_from(dir.join("repo"), dir.join("worktrees"), source)
    }

    /// Pull the source archetype's current `main` into this fork's `main` — upstream
    /// improvements flow down via a real 3-way merge over the shared fork point. Clean,
    /// or Conflict (aborted cleanly so the fork's `main` is never left half-merged).
    pub fn pull_from(&self, source: &PeerSource) -> Result<MergeOutcome> {
        let src = source.0.to_string_lossy().to_string();
        git(
            &self.repo,
            ["fetch", "-q", &src, "+refs/heads/main:refs/heads/upstream"],
        )?;
        let (ok, _) = git_ok(
            &self.repo,
            [
                "merge",
                "--no-ff",
                "-q",
                "-m",
                "pull updates from source archetype",
                "upstream",
            ],
        )?;
        if ok {
            Ok(MergeOutcome::Clean)
        } else {
            let _ = git(&self.repo, ["merge", "--abort"]);
            Ok(MergeOutcome::Conflict)
        }
    }

    /// Whether the source archetype has commits this fork has not pulled yet — i.e. the
    /// source's `main` is ahead of what this fork last merged. Best-effort (false on any
    /// git hiccup), used only to surface a "pull available" hint.
    pub fn updates_available_from(&self, source_repo: &Path) -> bool {
        let src = source_repo.to_string_lossy().to_string();
        if git(
            &self.repo,
            ["fetch", "-q", &src, "+refs/heads/main:refs/heads/upstream"],
        )
        .is_err()
        {
            return false;
        }
        // Commits reachable from upstream but not from main ⇒ something to pull.
        match git(&self.repo, ["rev-list", "--count", "main..upstream"]) {
            Ok(out) => out.trim().parse::<u64>().map(|n| n > 0).unwrap_or(false),
            Err(_) => false,
        }
    }

    /// Seed files onto `main` and commit — used to lay down an agent's starter
    /// definition (`.pi/SYSTEM.md`, `AGENTS.md`) so engagement worktrees branched
    /// off `main` inherit it (ADR 0029). Each entry is `(relative path, content)`.
    pub fn seed_main(&self, files: &[(&str, &str)]) -> Result<()> {
        for (rel, content) in files {
            let path = self.repo.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(WorkspaceError::git_spawn)?;
            }
            std::fs::write(&path, content).map_err(WorkspaceError::git_spawn)?;
        }
        git(&self.repo, ["add", "-A"])?;
        git(&self.repo, ["commit", "-q", "-m", "seed agent definition"])?;
        Ok(())
    }

    /// Tear down one engagement: remove its worktree and delete its branch. The
    /// inverse of `create_engagement` — used when a chat is deleted. Best-effort
    /// (`--force`) so a dirty worktree still goes.
    pub fn remove_engagement(&self, id: &str) -> Result<()> {
        let path = self.worktrees.join(id);
        let path_str = path.to_string_lossy().into_owned();
        let _ = git(&self.repo, ["worktree", "remove", "--force", &path_str]);
        let _ = git(&self.repo, ["branch", "-D", &format!("engagement/{id}")]);
        Ok(())
    }

    /// **Crypto-erasure of git-blob workspace content (`SECAUD-6`).** After a deleted
    /// engagement's branch is removed, its commits and blobs become *unreachable* but linger
    /// in the object store (the reflog holds them, and `gc` keeps a grace window), so the
    /// content is still recoverable (`git fsck --lost-found` / `git cat-file`). Expire the
    /// reflog and `gc --prune=now` so the erased engagement's **unique** objects are gone;
    /// objects still reachable from `main` or another engagement branch are kept (only that
    /// engagement's data is erased). Called on the crypto-erasure path, not ordinary teardown
    /// (`gc` is heavy). Best-effort — a failure leaves the objects unreachable-but-present,
    /// which a later `gc` still collects.
    pub fn purge_unreachable_objects(&self) -> Result<()> {
        let _ = git(&self.repo, ["reflog", "expire", "--expire=now", "--all"]);
        let _ = git(&self.repo, ["gc", "--prune=now", "--quiet"]);
        Ok(())
    }

    /// Create an engagement: a worktree off `main` on branch `engagement/<id>`.
    pub fn create_engagement(&self, id: &str) -> Result<Engagement> {
        self.create_engagement_on(id, "main")
    }

    /// Create an engagement homed to a specific shared ref `target` — `main` (the
    /// placement mainline, the default) or a workstream main `workstream/<id>/main`
    /// (`WS-C`). The worktree branches off `target` and the engagement promotes/syncs
    /// against it.
    pub fn create_engagement_on(&self, id: &str, target: &str) -> Result<Engagement> {
        let branch = format!("engagement/{id}");
        let path = self.worktrees.join(id);
        let path_str = path.to_string_lossy().into_owned();
        git(
            &self.repo,
            ["worktree", "add", "-q", "-b", &branch, &path_str, target],
        )?;
        Ok(Engagement {
            repo: self.repo.clone(),
            path,
            branch,
            target: target.to_string(),
        })
    }

    /// The shared-ref name of a workstream's main line.
    pub fn workstream_ref(ws_id: &str) -> String {
        format!("workstream/{ws_id}/main")
    }

    /// Create a workstream's shared line: a `workstream/<id>/main` branch off the
    /// placement mainline (`WS-C`). No worktree — the stream main advances by
    /// ref-update (`merge-tree → commit-tree → update-ref`), not via a checkout.
    pub fn create_workstream(&self, ws_id: &str) -> Result<()> {
        git(&self.repo, ["branch", &Self::workstream_ref(ws_id), "main"])?;
        Ok(())
    }

    /// Integrate a workstream's main into the placement mainline — the boundary-gated
    /// `advanced → integrated` hop at the git level. Merges `workstream/<id>/main`
    /// into `main` (checked out in the repo worktree); a conflict aborts cleanly so
    /// mainline is never left half-merged (PARTIAL_MERGE_NOT_STANDING).
    pub fn promote_workstream_to_main(&self, ws_id: &str) -> Result<MergeOutcome> {
        let ws_ref = Self::workstream_ref(ws_id);
        let msg = format!("integrate {ws_ref}");
        let (ok, _) = git_ok(&self.repo, ["merge", "--no-ff", "-q", "-m", &msg, &ws_ref])?;
        if ok {
            Ok(MergeOutcome::Clean)
        } else {
            let _ = git(&self.repo, ["merge", "--abort"]);
            Ok(MergeOutcome::Conflict)
        }
    }

    /// Reconstruct the handle for an **existing** engagement worktree (no git op).
    /// Targets `main` by default; the caller re-homes it to a workstream main via
    /// [`Engagement::set_target`] from the workstream membership projection.
    pub fn open_engagement(&self, id: &str) -> Engagement {
        Engagement {
            repo: self.repo.clone(),
            path: self.worktrees.join(id),
            branch: format!("engagement/{id}"),
            target: "main".to_string(),
        }
    }

    /// Reconcile the engagements against **git's authoritative view** — so the
    /// workbench rehydrates after a restart (worktrees + event log are durable;
    /// only the in-memory map was lost). Engagements *are* the `engagement/*`
    /// branches: each is rehydrated to its live worktree, or — if its worktree
    /// dir was deleted by hand — **re-materialized** from the branch (the
    /// committed work is never lost). Returns `(id, handle)` pairs.
    pub fn reconcile_engagements(&self) -> Result<Vec<(String, Engagement)>> {
        let _ = git(&self.repo, ["worktree", "prune"]); // drop dead registrations

        // live worktrees: engagement id -> path (from `worktree list --porcelain`).
        let listing = git(&self.repo, ["worktree", "list", "--porcelain"]).unwrap_or_default();
        let mut live: std::collections::BTreeMap<String, PathBuf> = Default::default();
        let mut cur_path: Option<PathBuf> = None;
        for line in listing.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                cur_path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/engagement/") {
                if let Some(p) = cur_path.take() {
                    live.insert(b.to_string(), p);
                }
            }
        }

        // every engagement branch is an engagement, worktree or not.
        let branches = git(
            &self.repo,
            [
                "branch",
                "--list",
                "engagement/*",
                "--format=%(refname:short)",
            ],
        )
        .unwrap_or_default();
        let mut out = Vec::new();
        for branch in branches.lines().map(str::trim).filter(|s| !s.is_empty()) {
            let Some(id) = branch.strip_prefix("engagement/") else {
                continue;
            };
            let path = match live.get(id) {
                Some(p) => p.clone(),
                None => {
                    // re-materialize the orphaned branch's worktree.
                    let path = self.worktrees.join(id);
                    let path_str = path.to_string_lossy().into_owned();
                    git(&self.repo, ["worktree", "add", "-q", &path_str, branch])?;
                    path
                }
            };
            out.push((
                id.to_string(),
                Engagement {
                    repo: self.repo.clone(),
                    path,
                    branch: branch.to_string(),
                    target: "main".to_string(),
                },
            ));
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

/// An engagement worktree: where a single conversation's runs do their work,
/// isolated from its **target shared ref** until kept. The target is `main` (the
/// placement mainline) by default, or a workstream main `workstream/<id>/main` when
/// the chat is homed to a workstream (`WS-C`). Promote/sync/diff are all against the
/// target.
pub struct Engagement {
    repo: PathBuf,
    path: PathBuf,
    branch: String,
    target: String,
}

impl Engagement {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn branch(&self) -> &str {
        &self.branch
    }
    /// The shared ref this engagement promotes into / syncs from (`main` by default,
    /// or its workstream main).
    pub fn target(&self) -> &str {
        &self.target
    }
    /// Re-home this engagement onto a different shared ref — joining a workstream
    /// (`workstream/<id>/main`) or leaving it back to `main`. The branch and its
    /// commits are unchanged; only the ref it promotes/syncs against moves.
    pub fn set_target(&mut self, target: impl Into<String>) {
        self.target = target.into();
    }

    /// Auto-commit the worktree at the end of a turn. Returns the new revision's
    /// opaque id (git: the commit's short hash), or `None` if the turn changed
    /// nothing (no empty commits).
    pub fn commit_turn(&self, message: &str) -> Result<Option<RevisionId>> {
        git(&self.path, ["add", "-A"])?;
        // Nothing staged → nothing to commit (a no-op turn).
        if git(&self.path, ["status", "--porcelain"])?.is_empty() {
            return Ok(None);
        }
        git(&self.path, ["commit", "-q", "-m", message])?;
        Ok(Some(RevisionId(git(
            &self.path,
            ["rev-parse", "--short", "HEAD"],
        )?)))
    }

    /// The diff a reviewer sees: the engagement branch against its target shared ref
    /// (`main`, or its workstream main).
    pub fn diff_against_main(&self) -> Result<String> {
        git(&self.path, ["diff", &format!("{}...HEAD", self.target)])
    }

    /// Discard the engagement's work, restoring the worktree to its target shared ref
    /// (`main`) — the user-facing **revert** (UX-5). Hard-resets the engagement branch to
    /// `target` and removes any untracked files, so the engagement's commits and
    /// uncommitted edits since the baseline are dropped. **`main` itself is untouched**
    /// (the reset moves only this engagement's ref); the work is recoverable only by
    /// redoing it. After this, `diff_against_main` is empty.
    pub fn revert_to_main(&self) -> Result<()> {
        git(&self.path, ["reset", "--hard", &self.target])?;
        git(&self.path, ["clean", "-fd"])?;
        Ok(())
    }

    /// Ingest existing context into the engagement worktree: a **folder** copied
    /// recursively (skipping any nested `.git`), or a **single file** (UX-1) copied into the
    /// worktree root under its own name. Returns the number of files ingested. The next
    /// turn's auto-commit, or [`commit_turn`](Self::commit_turn), records it.
    pub fn ingest(&self, source: &Path) -> Result<usize> {
        if source.is_file() {
            let name = source.file_name().ok_or_else(|| {
                WorkspaceError::msg(format!(
                    "ingest {}: context path has no file name",
                    source.display()
                ))
            })?;
            std::fs::copy(source, self.path.join(name)).map_err(WorkspaceError::io)?;
            Ok(1)
        } else {
            copy_dir(source, &self.path)
        }
    }

    /// Ingest **uploaded** context (`ENTSEC-5`): each `(name, content)` is written into the
    /// worktree root under its **basename** (any directory part of `name` is dropped) and
    /// path-confined via [`safe_path`](Self::safe_path) — so an upload can never write outside
    /// the worktree, and a remote client never drives a server-side path read (the enterprise
    /// thin-client's context-in, where the client's disk is not the server's). Returns the
    /// number of files written; the caller commits.
    pub fn ingest_upload(&self, files: &[(String, String)]) -> Result<usize> {
        for (name, content) in files {
            let base = Path::new(name)
                .file_name()
                .ok_or_else(|| {
                    WorkspaceError::msg(format!(
                        "ingest upload: uploaded file has no name: {name:?}"
                    ))
                })?
                .to_string_lossy()
                .to_string();
            self.write_file(&base, content)?;
        }
        Ok(files.len())
    }

    /// The worktree's files, recursively (relative paths, `.git` skipped) — the
    /// workspace file tree (`navigation.md`). Sorted for stable rendering.
    pub fn tree(&self) -> Result<Vec<FileEntry>> {
        let mut out = Vec::new();
        walk_tree(&self.path, &self.path, &mut out).map_err(WorkspaceError::io)?;
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }

    /// Read a worktree file's text. The path must stay inside the worktree.
    pub fn read_file(&self, rel: &str) -> Result<String> {
        let abs = self.safe_path(rel)?;
        std::fs::read_to_string(&abs).map_err(WorkspaceError::io)
    }

    /// Read at most `max_bytes` of a worktree file as text for content search, or
    /// `None` when the file looks **binary** (a NUL byte in the scanned prefix).
    /// Path-confined exactly like [`read_file`](Self::read_file); the byte cap bounds
    /// a per-query content walk so a large blob is never fully materialized (SEARCH-2).
    pub fn read_file_capped(&self, rel: &str, max_bytes: usize) -> Result<Option<String>> {
        use std::io::Read;
        let abs = self.safe_path(rel)?;
        let file = std::fs::File::open(&abs).map_err(WorkspaceError::io)?;
        let mut buf = Vec::new();
        file.take(max_bytes as u64)
            .read_to_end(&mut buf)
            .map_err(WorkspaceError::io)?;
        // Null-byte sniff (how text tools detect non-text): a NUL in the scanned
        // prefix marks the file binary — skip it rather than match on noise.
        if buf.contains(&0) {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
    }

    /// Write a worktree file (the human's edit). Path-confined to the worktree;
    /// parent dirs are created. Does not commit — the caller decides when.
    pub fn write_file(&self, rel: &str, content: &str) -> Result<()> {
        let abs = self.safe_path(rel)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(WorkspaceError::io)?;
        }
        std::fs::write(&abs, content).map_err(WorkspaceError::io)
    }

    /// Resolve `rel` inside the worktree, rejecting any path that escapes it.
    fn safe_path(&self, rel: &str) -> Result<PathBuf> {
        let candidate = self.path.join(rel);
        // Reject `..` escapes without requiring the file to exist (lexical check).
        let escapes = candidate
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
            || rel.starts_with('/');
        if escapes {
            return Err(WorkspaceError::msg(format!(
                "path {rel} escapes the worktree"
            )));
        }
        Ok(candidate)
    }

    /// Probe whether the engagement branch would merge cleanly into its target shared
    /// ref — **without mutating anything** (`git merge-tree`). Drives the merge
    /// lifecycle's `GitClean`/`GitConflict` at turn end.
    pub fn merge_probe(&self) -> Result<MergeOutcome> {
        let (ok, _) = git_ok(
            &self.repo,
            ["merge-tree", "--write-tree", &self.target, &self.branch],
        )?;
        Ok(if ok {
            MergeOutcome::Clean
        } else {
            MergeOutcome::Conflict
        })
    }

    /// Advance the engagement's **target shared ref** by merging the engagement branch
    /// in (no fast-forward, so it is a visible point in history). On conflict the ref
    /// is left untouched (PARTIAL_MERGE_NOT_STANDING); the outcome is reported for the
    /// lifecycle to isolate + repair.
    ///
    /// The target is `main` (the placement mainline) by default, or a workstream main
    /// `workstream/<id>/main`. `main` is checked out in the repo worktree, so the merge
    /// runs there (keeping the repo tree in sync); a workstream main has **no worktree**,
    /// so the ref advances by `merge-tree → commit-tree → update-ref` — atomic, with
    /// nothing to abort on conflict.
    pub fn merge_into_main(&self) -> Result<MergeOutcome> {
        if self.target == "main" {
            let msg = format!("keep {}", self.branch);
            let (ok, _) = git_ok(
                &self.repo,
                ["merge", "--no-ff", "-q", "-m", &msg, &self.branch],
            )?;
            if ok {
                Ok(MergeOutcome::Clean)
            } else {
                let _ = git(&self.repo, ["merge", "--abort"]); // never leave a partial merge
                Ok(MergeOutcome::Conflict)
            }
        } else {
            // No worktree for a workstream main — advance the ref directly. A clean
            // merge-tree yields the merged tree; we commit it with both parents and
            // move the ref. A conflict mutates nothing, so there is no partial merge.
            let (ok, tree) = git_ok(
                &self.repo,
                ["merge-tree", "--write-tree", &self.target, &self.branch],
            )?;
            if !ok {
                return Ok(MergeOutcome::Conflict);
            }
            let msg = format!("keep {} into {}", self.branch, self.target);
            let commit = git(
                &self.repo,
                [
                    "commit-tree",
                    tree.trim(),
                    "-p",
                    &self.target,
                    "-p",
                    &self.branch,
                    "-m",
                    &msg,
                ],
            )?;
            git(
                &self.repo,
                [
                    "update-ref",
                    &format!("refs/heads/{}", self.target),
                    &commit,
                ],
            )?;
            Ok(MergeOutcome::Clean)
        }
    }

    /// Sync the settled target ref **into** this engagement's worktree — the within-a-
    /// workstream auto-sync hop (WC-1, ADR 0025): pick up work other member engagements
    /// promoted to the shared line (`main` or a workstream main). A conflict aborts
    /// cleanly (the worktree is unchanged, left for repair). The inverse direction of
    /// [`merge_into_main`](Self::merge_into_main).
    pub fn sync_from_main(&self) -> Result<MergeOutcome> {
        let msg = format!("sync from {}", self.target);
        let (ok, _) = git_ok(
            &self.path,
            ["merge", "--no-ff", "-q", "-m", &msg, &self.target],
        )?;
        if ok {
            Ok(MergeOutcome::Clean)
        } else {
            let _ = git(&self.path, ["merge", "--abort"]);
            Ok(MergeOutcome::Conflict)
        }
    }
}

/// The substrate seam over an instance's shared state: the neutral surface the
/// control plane holds (`Box<dyn Workspace>`), with git as today's only impl.
/// Ref tokens (`mainline`, workstream refs) are minted **and** parsed only here —
/// callers treat them as opaque strings.
pub trait Workspace: Send {
    /// The placement mainline's ref token (git: `"main"`).
    fn mainline(&self) -> &str;
    /// Mint the shared-ref token of a workstream's main line
    /// (git: `"workstream/<id>/main"`).
    fn workstream_ref(&self, ws_id: &str) -> String;
    /// The inverse of [`workstream_ref`](Self::workstream_ref): recover the
    /// workstream id from a ref token, or `None` when the token is not a
    /// workstream main (e.g. the mainline).
    fn workstream_id_of(&self, target: &str) -> Option<String>;
    /// Create an engagement homed to the mainline.
    fn create_engagement(&self, id: &str) -> Result<Box<dyn ChatWorkspace>>;
    /// Create an engagement homed to a specific shared ref `target`.
    fn create_engagement_on(&self, id: &str, target: &str) -> Result<Box<dyn ChatWorkspace>>;
    /// Tear down one engagement (best-effort; the inverse of `create_engagement`).
    fn remove_engagement(&self, id: &str) -> Result<()>;
    /// Erase removed content from the substrate's object store (`SECAUD-6`): after an
    /// engagement is removed, its now-unreachable content must stop being recoverable
    /// from this workspace. Best-effort for the git impl (reflog expire + gc); a future
    /// impl performs per-blob crypto-erasure with retained hashes (ADR 0071 §2).
    fn purge_unreachable_objects(&self) -> Result<()>;
    /// Rehydrate every engagement from the impl's authoritative view (restart
    /// recovery). Returns `(id, handle)` pairs, sorted by id.
    fn reconcile_engagements(&self) -> Result<Vec<(String, Box<dyn ChatWorkspace>)>>;
    /// Create a workstream's shared line off the mainline.
    fn create_workstream(&self, ws_id: &str) -> Result<()>;
    /// Integrate a workstream's main into the placement mainline.
    fn promote_workstream_to_main(&self, ws_id: &str) -> Result<MergeOutcome>;
    /// Seed `(relative path, content)` files onto the mainline as settled state.
    fn seed_main(&self, files: &[(&str, &str)]) -> Result<()>;
    /// Serialize the full content graph into an opaque [`WorkspaceExport`].
    /// CONTRACT: same-impl re-materialization, idempotent; there is NO
    /// byte-round-trip promise — an impl MAY erasure-filter (see
    /// [`WorkspaceExport`]).
    fn export(&self) -> Result<WorkspaceExport>;
    /// The impl-stable id of this impl's export payload format.
    fn export_format(&self) -> &'static str;
    /// The opaque token another instance uses to fork from / pull from this one.
    fn peer_source(&self) -> PeerSource;
    /// Pull the source's mainline into this instance's mainline (3-way merge over
    /// the shared fork point). Clean, or Conflict with nothing half-applied.
    fn pull_from(&self, src: &PeerSource) -> Result<MergeOutcome>;
}

/// The substrate seam over one chat's isolated working state.
pub trait ChatWorkspace: Send {
    /// CONTRACT: a real, materialized directory usable as the harness cwd for the
    /// life of the chat. Any impl (virtual working sets included) must honor this.
    fn path(&self) -> &Path;
    /// Opaque display/id label for this chat's line of work. Its format is
    /// impl-stable (pinned by clients), not semantic — callers never parse it.
    fn branch(&self) -> &str;
    /// The opaque shared-ref token this chat promotes into / syncs from. Only
    /// [`Workspace`] mints and parses ref tokens.
    fn target(&self) -> &str;
    /// Re-home this chat onto a different shared ref token. The git impl cannot
    /// fail (`Ok(())` after the field write); the error channel exists for impls
    /// that must re-home fail-closed.
    fn set_target(&mut self, target: &str) -> Result<()>;
    /// Record the turn's work as a new revision. `None` = this turn produced no
    /// content change since the last revision (git: no empty commits; an
    /// auto-cutting impl reports `None` when quiescent).
    fn commit_turn(&self, message: &str) -> Result<Option<RevisionId>>;
    /// CONTRACT (blessed): unified-diff text of this chat's work against its
    /// target. This is the review wire format the web client parses; any impl
    /// must render to it.
    fn diff_against_main(&self) -> Result<String>;
    /// Discard the chat's work, restoring its working state to the target.
    fn revert_to_main(&self) -> Result<()>;
    /// Sync integrates target changes available at call time. Impls MAY also
    /// integrate continuously between calls; callers must not assume a frozen
    /// base. A conflict leaves the working state unchanged.
    fn sync_from_main(&self) -> Result<MergeOutcome>;
    /// Probe whether the chat's work would merge cleanly into its target,
    /// mutating nothing.
    fn merge_probe(&self) -> Result<MergeOutcome>;
    /// Advance the target by merging the chat's work in; on conflict the target
    /// is left untouched.
    fn merge_into_main(&self) -> Result<MergeOutcome>;
    // The worktree-FS facet: path-confined to the chat's directory; the impl
    // skips its own metadata dir (git: `.git`).
    /// Copy a context folder (recursively) or single file into the working dir.
    fn ingest(&self, source: &Path) -> Result<usize>;
    /// Ingest uploaded `(name, content)` files (`ENTSEC-5`): the upload counterpart of
    /// [`Self::ingest`] for hosts whose client disk is not the server's. Base-name only,
    /// path-confined; the caller records the revision.
    fn ingest_upload(&self, files: &[(String, String)]) -> Result<usize>;
    /// The working dir's files, recursively (relative paths, sorted).
    fn tree(&self) -> Result<Vec<FileEntry>>;
    /// Read a file's text (path-confined).
    fn read_file(&self, rel: &str) -> Result<String>;
    /// Read up to `max_bytes` of a file as text (path-confined), or `None` when it
    /// looks binary (a NUL byte in the scanned prefix). The byte-capped, binary-
    /// skipping counterpart of [`read_file`](Self::read_file) — the read primitive a
    /// bounded per-query content walk stands on (SEARCH-2), never a persistent index.
    fn read_file_capped(&self, rel: &str, max_bytes: usize) -> Result<Option<String>>;
    /// Write a file, creating parents; does not record a revision (path-confined).
    fn write_file(&self, rel: &str, content: &str) -> Result<()>;
}

// Compile-time proof the seam traits stay object-safe — the app holds them as
// `Box<dyn Workspace>` / `Box<dyn ChatWorkspace>` / `Arc<dyn WorkspaceProvider>`.
const _: fn(&dyn Workspace, &dyn ChatWorkspace, &dyn WorkspaceProvider) = |_, _, _| {};

// Trait impls delegate to the inherent methods (which always win name
// resolution on the concrete types), so the git behavior lives in one place.
impl Workspace for Instance {
    fn mainline(&self) -> &str {
        "main"
    }
    fn workstream_ref(&self, ws_id: &str) -> String {
        Instance::workstream_ref(ws_id)
    }
    fn workstream_id_of(&self, target: &str) -> Option<String> {
        target
            .strip_prefix("workstream/")
            .and_then(|s| s.strip_suffix("/main"))
            .map(str::to_string)
    }
    fn create_engagement(&self, id: &str) -> Result<Box<dyn ChatWorkspace>> {
        Ok(Box::new(Instance::create_engagement(self, id)?))
    }
    fn create_engagement_on(&self, id: &str, target: &str) -> Result<Box<dyn ChatWorkspace>> {
        Ok(Box::new(Instance::create_engagement_on(self, id, target)?))
    }
    fn remove_engagement(&self, id: &str) -> Result<()> {
        Instance::remove_engagement(self, id)
    }
    fn purge_unreachable_objects(&self) -> Result<()> {
        Instance::purge_unreachable_objects(self)
    }
    fn reconcile_engagements(&self) -> Result<Vec<(String, Box<dyn ChatWorkspace>)>> {
        Ok(Instance::reconcile_engagements(self)?
            .into_iter()
            .map(|(id, e)| (id, Box::new(e) as Box<dyn ChatWorkspace>))
            .collect())
    }
    fn create_workstream(&self, ws_id: &str) -> Result<()> {
        Instance::create_workstream(self, ws_id)
    }
    fn promote_workstream_to_main(&self, ws_id: &str) -> Result<MergeOutcome> {
        Instance::promote_workstream_to_main(self, ws_id)
    }
    fn seed_main(&self, files: &[(&str, &str)]) -> Result<()> {
        Instance::seed_main(self, files)
    }
    fn export(&self) -> Result<WorkspaceExport> {
        Instance::export(self)
    }
    fn export_format(&self) -> &'static str {
        Instance::export_format(self)
    }
    fn peer_source(&self) -> PeerSource {
        Instance::peer_source(self)
    }
    fn pull_from(&self, src: &PeerSource) -> Result<MergeOutcome> {
        Instance::pull_from(self, src)
    }
}

impl ChatWorkspace for Engagement {
    fn path(&self) -> &Path {
        Engagement::path(self)
    }
    fn branch(&self) -> &str {
        Engagement::branch(self)
    }
    fn target(&self) -> &str {
        Engagement::target(self)
    }
    fn set_target(&mut self, target: &str) -> Result<()> {
        Engagement::set_target(self, target);
        Ok(())
    }
    fn commit_turn(&self, message: &str) -> Result<Option<RevisionId>> {
        Engagement::commit_turn(self, message)
    }
    fn diff_against_main(&self) -> Result<String> {
        Engagement::diff_against_main(self)
    }
    fn revert_to_main(&self) -> Result<()> {
        Engagement::revert_to_main(self)
    }
    fn sync_from_main(&self) -> Result<MergeOutcome> {
        Engagement::sync_from_main(self)
    }
    fn merge_probe(&self) -> Result<MergeOutcome> {
        Engagement::merge_probe(self)
    }
    fn merge_into_main(&self) -> Result<MergeOutcome> {
        Engagement::merge_into_main(self)
    }
    fn ingest(&self, source: &Path) -> Result<usize> {
        Engagement::ingest(self, source)
    }
    fn ingest_upload(&self, files: &[(String, String)]) -> Result<usize> {
        Engagement::ingest_upload(self, files)
    }
    fn tree(&self) -> Result<Vec<FileEntry>> {
        Engagement::tree(self)
    }
    fn read_file(&self, rel: &str) -> Result<String> {
        Engagement::read_file(self, rel)
    }
    fn read_file_capped(&self, rel: &str, max_bytes: usize) -> Result<Option<String>> {
        Engagement::read_file_capped(self, rel, max_bytes)
    }
    fn write_file(&self, rel: &str, content: &str) -> Result<()> {
        Engagement::write_file(self, rel, content)
    }
}

/// The construction seam: static workspace constructors (not object-safe on
/// [`Workspace`]) go behind a provider, registered per substrate id by the
/// host. A future substrate is a new provider, not new call sites.
pub trait WorkspaceProvider: Send + Sync {
    /// Initialize a fresh workspace under `dir` — the host-assigned state root
    /// for this instance (this git impl lays down the `dir/repo` +
    /// `dir/worktrees` layout; a future impl anchors its handle there).
    fn init_at(&self, dir: &Path) -> Result<Box<dyn Workspace>>;
    /// Open the existing workspace anchored at `dir`.
    fn open_at(&self, dir: &Path) -> Box<dyn Workspace>;
    /// Materialize a workspace under `dir` from a [`WorkspaceExport`]'s bytes
    /// (same-impl only; see [`Workspace::export_format`]).
    // Not a conversion of `self` — the provider constructs *from* the export.
    #[allow(clippy::wrong_self_convention)]
    fn from_export_at(&self, dir: &Path, export: &[u8]) -> Result<Box<dyn Workspace>>;
    /// Initialize a workspace under `dir` as a fork of `src`, sharing the
    /// source's history so later `pull_from` merges have a real ancestor.
    fn fork_from_at(&self, dir: &Path, src: &PeerSource) -> Result<Box<dyn Workspace>>;
}

/// The git impl behind [`WorkspaceProvider`].
pub struct GitWorkspaceProvider;

impl WorkspaceProvider for GitWorkspaceProvider {
    fn init_at(&self, dir: &Path) -> Result<Box<dyn Workspace>> {
        Ok(Box::new(Instance::init_at(dir)?))
    }
    fn open_at(&self, dir: &Path) -> Box<dyn Workspace> {
        Box::new(Instance::open_at(dir))
    }
    fn from_export_at(&self, dir: &Path, export: &[u8]) -> Result<Box<dyn Workspace>> {
        Ok(Box::new(Instance::from_bundle_at(dir, export)?))
    }
    fn fork_from_at(&self, dir: &Path, src: &PeerSource) -> Result<Box<dyn Workspace>> {
        Ok(Box::new(Instance::fork_from_at(dir, src)?))
    }
}

/// Recursively collect the worktree's entries relative to `root`, skipping `.git`.
fn walk_tree(root: &Path, dir: &Path, out: &mut Vec<FileEntry>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let p = entry.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        let ft = entry.file_type()?;
        if ft.is_dir() {
            out.push(FileEntry {
                path: rel,
                is_dir: true,
            });
            walk_tree(root, &p, out)?;
        } else if ft.is_file() {
            out.push(FileEntry {
                path: rel,
                is_dir: false,
            });
        }
    }
    Ok(())
}

/// Recursively copy `src` into `dst`, skipping `.git`. Returns the file count.
fn copy_dir(src: &Path, dst: &Path) -> Result<usize> {
    let mut count = 0;
    let entries = std::fs::read_dir(src).map_err(WorkspaceError::io)?;
    for entry in entries {
        let entry = entry.map_err(WorkspaceError::io)?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let from = entry.path();
        let to = dst.join(&name);
        let ft = entry.file_type().map_err(WorkspaceError::io)?;
        if ft.is_dir() {
            std::fs::create_dir_all(&to).map_err(WorkspaceError::io)?;
            count += copy_dir(&from, &to)?;
        } else if ft.is_file() {
            std::fs::copy(&from, &to).map_err(WorkspaceError::io)?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_bin_resolves_override_then_path_default() {
        // Unset / empty → the PATH default `git` (what every other test relies on).
        assert_eq!(git_bin_from(None), "git");
        assert_eq!(git_bin_from(Some(String::new())), "git");
        // A non-empty GAUGEWRIGHT_GIT_BIN (a bundle's vendored git) wins (SELFHOST-1).
        assert_eq!(
            git_bin_from(Some("/opt/gaugewright/bin/git".into())),
            "/opt/gaugewright/bin/git"
        );
    }

    #[test]
    fn workspace_error_display_is_byte_identical_to_the_legacy_git_error() {
        // The legacy `GitError::Spawn` Display bytes ("could not run git: {e}").
        let spawn = WorkspaceError::git_spawn(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No such file or directory (os error 2)",
        ));
        assert_eq!(
            spawn.to_string(),
            "could not run git: No such file or directory (os error 2)"
        );
        // The legacy `GitError::Failed` Display bytes ("git {command} failed: {stderr}",
        // stderr trimmed).
        let failed = WorkspaceError::git_failed("merge --abort", "fatal: no merge to abort\n");
        assert_eq!(
            failed.to_string(),
            "git merge --abort failed: fatal: no merge to abort"
        );
    }

    #[test]
    fn failing_git_op_mints_the_legacy_error_bytes() {
        let (_d, inst) = instance();
        let err = match inst.create_engagement_on("e-bad", "no-such-ref") {
            Ok(_) => panic!("creating an engagement on a missing ref must fail"),
            Err(e) => e,
        };
        let shown = err.to_string();
        assert!(
            shown.starts_with("git worktree add") && shown.contains(" failed: "),
            "git-subprocess failure keeps the legacy shape: {shown}"
        );
    }

    #[test]
    fn facet_fs_errors_do_not_claim_git_ran() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e-err").unwrap();
        // A missing file is a bare io message, not a "could not run git" claim.
        let err = eng.read_file("missing.txt").unwrap_err();
        assert!(
            !err.to_string().contains("git"),
            "facet io error must not claim git ran: {err}"
        );
        // The path-confinement rejection names the path, without a git claim.
        let err = eng.read_file("../escape").unwrap_err();
        assert_eq!(err.to_string(), "path ../escape escapes the worktree");
    }

    fn instance() -> (tempfile::TempDir, Instance) {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init(dir.path().join("repo"), dir.path().join("worktrees")).unwrap();
        (dir, inst)
    }
    fn write(p: &Path, name: &str, contents: &str) {
        std::fs::write(p.join(name), contents).unwrap();
    }
    fn read(p: &Path, name: &str) -> Option<String> {
        std::fs::read_to_string(p.join(name)).ok()
    }

    #[test]
    fn engagement_isolates_until_kept() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e1").unwrap();

        // a turn produces a file in the worktree, auto-committed
        write(eng.path(), "out.txt", "hello");
        let oid = eng.commit_turn("turn 1").unwrap();
        assert!(oid.is_some(), "a changed turn commits");

        // the diff shows the work; main is still clean (isolation)
        assert!(eng.diff_against_main().unwrap().contains("hello"));
        assert_eq!(
            read(inst.repo(), "out.txt"),
            None,
            "main untouched before keep"
        );

        // keeping merges it into main
        eng.merge_into_main().unwrap();
        assert_eq!(read(inst.repo(), "out.txt").as_deref(), Some("hello"));
    }

    #[test]
    fn bundle_round_trip_relocates_main_and_engagement_content() {
        // Origin: an instance with settled `main` content and an engagement whose
        // worktree holds work not yet on main.
        let (_d, origin) = instance();
        origin.seed_main(&[("AGENTS.md", "be helpful")]).unwrap();
        let eng = origin.create_engagement("e-bundle").unwrap();
        write(eng.path(), "draft.txt", "engagement work");
        eng.commit_turn("turn 1").unwrap();

        // Ship the content bytes as an opaque export and re-materialize on a fresh target.
        let export = origin.export().unwrap();
        assert!(!export.0.is_empty(), "a non-empty export is produced");
        assert_eq!(origin.export_format(), "git-bundle-v1");

        let target_dir = tempfile::tempdir().unwrap();
        let target = Instance::from_bundle_at(target_dir.path(), &export.0).unwrap();

        // The target's `main` carries the settled content (the bytes behind handles).
        assert_eq!(
            read(target.repo(), "AGENTS.md").as_deref(),
            Some("be helpful"),
            "main content materialized on the target"
        );

        // Reconcile rehydrates the engagement worktree, with its in-flight work intact.
        let engs = target.reconcile_engagements().unwrap();
        let (_id, t_eng) = engs
            .iter()
            .find(|(id, _)| id == "e-bundle")
            .expect("engagement branch relocated");
        assert_eq!(
            read(t_eng.path(), "draft.txt").as_deref(),
            Some("engagement work"),
            "engagement content materialized on the target"
        );
    }

    #[test]
    fn ingest_opens_a_folder_of_context_into_the_worktree() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e-ctx").unwrap();

        // an existing project folder with a nested file and a .git to skip
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("src")).unwrap();
        std::fs::create_dir_all(src.path().join(".git")).unwrap();
        write(src.path(), "README.md", "# project");
        write(&src.path().join("src"), "main.rs", "fn main() {}");
        std::fs::write(src.path().join(".git/HEAD"), "ref: x").unwrap();

        let n = eng.ingest(src.path()).unwrap();
        assert_eq!(n, 2, "two files ingested, .git skipped");
        assert_eq!(read(eng.path(), "README.md").as_deref(), Some("# project"));
        assert!(eng.path().join("src/main.rs").exists());
        assert!(!eng.path().join(".git/HEAD").exists() || eng.path().join(".git").is_dir());

        // the ingested context shows up as the turn's work once committed
        eng.commit_turn("ingest context").unwrap();
        assert!(eng.diff_against_main().unwrap().contains("README.md"));
    }

    #[test]
    fn purge_unreachable_erases_a_deleted_engagements_git_blobs() {
        // SECAUD-6: after a chat's engagement is torn down, purging makes its UNIQUE workspace
        // content (git blobs) unrecoverable, while content shared with main survives.
        let (dir, inst) = instance();
        let repo = dir.path().join("repo");
        inst.seed_main(&[("shared.txt", "SHARED-CONTENT")]).unwrap();

        let eng = inst.create_engagement("e-erase").unwrap();
        eng.write_file("secret.txt", "TOP-SECRET-PII-XYZZY")
            .unwrap();
        eng.commit_turn("add secret").unwrap();

        let secret = git(&repo, ["rev-parse", "engagement/e-erase:secret.txt"])
            .unwrap()
            .trim()
            .to_string();
        let shared = git(&repo, ["rev-parse", "main:shared.txt"])
            .unwrap()
            .trim()
            .to_string();
        assert!(
            git(&repo, ["cat-file", "-e", &secret]).is_ok(),
            "the secret blob exists"
        );

        // Tear down the engagement (branch deleted) — the object is now unreachable but still
        // present (recoverable) until pruned.
        inst.remove_engagement("e-erase").unwrap();
        assert!(
            git(&repo, ["cat-file", "-e", &secret]).is_ok(),
            "unreachable-but-present before purge (the gap SECAUD-6 closes)"
        );

        // Purge → the erased engagement's blob is gone; the shared main content survives.
        inst.purge_unreachable_objects().unwrap();
        assert!(
            git(&repo, ["cat-file", "-e", &secret]).is_err(),
            "the deleted chat's workspace blob is purged (unrecoverable)"
        );
        assert!(
            git(&repo, ["cat-file", "-e", &shared]).is_ok(),
            "content shared with main is NOT over-erased"
        );
    }

    #[test]
    fn ingest_upload_writes_files_and_confines_to_the_worktree() {
        // ENTSEC-5: uploaded context is written into the worktree, and a traversal-y name is
        // reduced to its basename so it can never escape the worktree root.
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e-upload").unwrap();

        let files = vec![
            ("notes.md".to_string(), "# uploaded notes".to_string()),
            ("../../etc/evil".to_string(), "nope".to_string()),
        ];
        let n = eng.ingest_upload(&files).unwrap();
        assert_eq!(n, 2, "both files written");
        assert_eq!(
            read(eng.path(), "notes.md").as_deref(),
            Some("# uploaded notes")
        );
        // the traversal-y name landed as its basename inside the worktree, not outside it.
        assert_eq!(read(eng.path(), "evil").as_deref(), Some("nope"));
        assert!(!eng.path().join("../../etc/evil").exists());

        eng.commit_turn("ingest upload").unwrap();
        assert!(eng.diff_against_main().unwrap().contains("notes.md"));
    }

    #[test]
    fn revert_to_main_discards_engagement_work() {
        // UX-5: a user-facing revert restores the worktree to main and drops the work.
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e-revert").unwrap();

        // Do work: a committed change plus an untracked file.
        write(eng.path(), "draft.md", "work in progress");
        eng.commit_turn("a turn of work").unwrap();
        write(eng.path(), "scratch.tmp", "untracked");
        assert!(eng.diff_against_main().unwrap().contains("draft.md"));

        eng.revert_to_main().unwrap();

        // The engagement now matches main: no diff, the committed + untracked work gone.
        assert_eq!(eng.diff_against_main().unwrap(), "");
        assert!(!eng.path().join("draft.md").exists());
        assert!(!eng.path().join("scratch.tmp").exists());
    }

    #[test]
    fn ingest_attaches_a_single_file_into_the_worktree() {
        // UX-1: single-file attach, not only folders.
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e-file").unwrap();

        let src = tempfile::tempdir().unwrap();
        write(src.path(), "notes.md", "# just one file");

        let n = eng.ingest(&src.path().join("notes.md")).unwrap();
        assert_eq!(n, 1, "exactly the one file ingested");
        assert_eq!(
            read(eng.path(), "notes.md").as_deref(),
            Some("# just one file")
        );

        eng.commit_turn("ingest single file").unwrap();
        assert!(eng.diff_against_main().unwrap().contains("notes.md"));
    }

    #[test]
    fn reconcile_rehydrates_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let wt = dir.path().join("wt");
        let inst = Instance::init(&repo, &wt).unwrap();
        inst.create_engagement("alpha").unwrap();
        inst.create_engagement("beta").unwrap();

        // a *fresh* Instance::open (as on a control-plane restart) rediscovers them.
        let reopened = Instance::open(&repo, &wt);
        let ids: Vec<String> = reopened
            .reconcile_engagements()
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["alpha".to_string(), "beta".to_string()]);
    }

    #[test]
    fn workstream_sync_one_engagement_promotes_another_picks_it_up() {
        // two engagements off one instance's main = a workstream of two (WC-1).
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init_at(dir.path()).unwrap();
        let a = inst.create_engagement("a").unwrap();
        let b = inst.create_engagement("b").unwrap();

        // A does work and promotes it to main.
        write(a.path(), "shared.txt", "from A");
        a.commit_turn("a's work").unwrap();
        assert_eq!(a.merge_into_main().unwrap(), MergeOutcome::Clean);

        // B doesn't have A's file yet…
        assert!(read(b.path(), "shared.txt").is_none());
        // …until B syncs from main (the within-workstream auto-sync hop).
        assert_eq!(b.sync_from_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(read(b.path(), "shared.txt").as_deref(), Some("from A"));
    }

    #[test]
    fn named_workstream_isolates_from_mainline_then_promotes() {
        // Two chats homed to a named workstream sync via the stream main, isolated from
        // the placement mainline; then the stream is explicitly promoted to mainline.
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init_at(dir.path()).unwrap();
        inst.create_workstream("ws1").unwrap();
        let ws_ref = Instance::workstream_ref("ws1");

        let a = inst.create_engagement_on("a", &ws_ref).unwrap();
        let b = inst.create_engagement_on("b", &ws_ref).unwrap();
        assert_eq!(a.target(), ws_ref);

        // A promotes its work — into the STREAM main, not the placement mainline.
        write(a.path(), "feat.txt", "stream work");
        a.commit_turn("a's work").unwrap();
        assert_eq!(a.merge_into_main().unwrap(), MergeOutcome::Clean);

        // The placement mainline (repo worktree) is untouched — isolation holds.
        assert_eq!(
            read(inst.repo(), "feat.txt"),
            None,
            "mainline untouched by an intra-stream promote"
        );

        // B, a member of the same stream, auto-syncs and picks A's work up.
        assert_eq!(b.sync_from_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(read(b.path(), "feat.txt").as_deref(), Some("stream work"));

        // Explicit, boundary-gated promotion of the stream into the placement mainline.
        assert_eq!(
            inst.promote_workstream_to_main("ws1").unwrap(),
            MergeOutcome::Clean
        );
        assert_eq!(
            read(inst.repo(), "feat.txt").as_deref(),
            Some("stream work"),
            "mainline carries the stream's work only after explicit promotion"
        );
    }

    #[test]
    fn rehoming_an_engagement_changes_its_target_ref() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init_at(dir.path()).unwrap();
        inst.create_workstream("ws1").unwrap();
        let ws_ref = Instance::workstream_ref("ws1");

        // A chat created on mainline, then joined to the workstream.
        let mut e = inst.create_engagement("c").unwrap();
        assert_eq!(e.target(), "main");
        e.set_target(&ws_ref);
        assert_eq!(e.target(), ws_ref);

        // Its promote now lands on the stream main, leaving mainline clean.
        write(e.path(), "x.txt", "joined work");
        e.commit_turn("work").unwrap();
        assert_eq!(e.merge_into_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(read(inst.repo(), "x.txt"), None, "mainline untouched");

        // Leaving (re-home to mainline) sends future promotes back to mainline.
        e.set_target("main");
        assert_eq!(e.merge_into_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(read(inst.repo(), "x.txt").as_deref(), Some("joined work"));
    }

    #[test]
    fn init_at_lays_out_repo_and_worktrees_then_remove_engagement_tears_down() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::init_at(dir.path()).unwrap();
        assert!(dir.path().join("repo/.git").exists());
        assert!(dir.path().join("worktrees").exists());

        let eng = inst.create_engagement("c1").unwrap();
        write(eng.path(), "f.txt", "hi");
        eng.commit_turn("t").unwrap();
        assert_eq!(inst.reconcile_engagements().unwrap().len(), 1);

        inst.remove_engagement("c1").unwrap();
        // worktree gone and branch deleted → nothing reconciles back.
        assert!(!dir.path().join("worktrees/c1").exists());
        assert_eq!(inst.reconcile_engagements().unwrap().len(), 0);
    }

    #[test]
    fn reconcile_rematerializes_an_orphaned_branch_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let wt = dir.path().join("wt");
        let inst = Instance::init(&repo, &wt).unwrap();
        let eng = inst.create_engagement("gamma").unwrap();
        write(eng.path(), "work.txt", "committed work");
        eng.commit_turn("turn").unwrap();

        // the worktree dir is deleted by hand, but the branch (and its commits) remain.
        std::fs::remove_dir_all(eng.path()).unwrap();

        // reconcile recovers it: re-materializes the worktree from the branch, so
        // the committed work is back.
        let reopened = Instance::open(&repo, &wt);
        let recovered = reopened.reconcile_engagements().unwrap();
        assert_eq!(recovered.len(), 1);
        let (id, handle) = &recovered[0];
        assert_eq!(id, "gamma");
        assert_eq!(
            read(handle.path(), "work.txt").as_deref(),
            Some("committed work")
        );
    }

    #[test]
    fn trait_objects_delegate_to_the_git_impl() {
        // The seam surface, driven purely through `dyn` — what the app holds.
        let dir = tempfile::tempdir().unwrap();
        let ws: Box<dyn Workspace> = Box::new(Instance::init_at(dir.path()).unwrap());
        assert_eq!(ws.mainline(), "main");
        assert_eq!(ws.workstream_ref("ws1"), "workstream/ws1/main");
        assert_eq!(
            ws.workstream_id_of("workstream/ws1/main").as_deref(),
            Some("ws1"),
            "workstream_id_of inverts workstream_ref"
        );
        assert_eq!(ws.workstream_id_of("main"), None);

        let mut chat: Box<dyn ChatWorkspace> = ws.create_engagement("e-dyn").unwrap();
        assert_eq!(chat.branch(), "engagement/e-dyn");
        assert_eq!(chat.target(), "main");
        chat.write_file("out.txt", "via the seam").unwrap();
        assert!(chat.commit_turn("turn").unwrap().is_some());
        assert!(chat.diff_against_main().unwrap().contains("via the seam"));
        assert_eq!(chat.merge_into_main().unwrap(), MergeOutcome::Clean);

        // AM-4: the git impl's re-home cannot fail.
        ws.create_workstream("ws1").unwrap();
        chat.set_target(&ws.workstream_ref("ws1")).unwrap();
        assert_eq!(chat.target(), "workstream/ws1/main");

        // Rehydration through the seam reports the same engagement set.
        let ids: Vec<String> = ws
            .reconcile_engagements()
            .unwrap()
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(ids, vec!["e-dyn".to_string()]);
    }

    #[test]
    fn noop_turn_makes_no_commit() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e2").unwrap();
        assert!(
            eng.commit_turn("empty").unwrap().is_none(),
            "no empty commits"
        );
    }

    #[test]
    fn many_engagements_share_one_main() {
        let (_d, inst) = instance();
        let a = inst.create_engagement("a").unwrap();
        let b = inst.create_engagement("b").unwrap();
        write(a.path(), "a.txt", "A");
        write(b.path(), "b.txt", "B");
        a.commit_turn("a work").unwrap();
        b.commit_turn("b work").unwrap();
        assert_eq!(a.merge_into_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(b.merge_into_main().unwrap(), MergeOutcome::Clean);
        assert_eq!(read(inst.repo(), "a.txt").as_deref(), Some("A"));
        assert_eq!(read(inst.repo(), "b.txt").as_deref(), Some("B"));
    }

    #[test]
    fn seed_main_commits_definition_and_new_engagements_inherit_it() {
        let (_d, inst) = instance();
        inst.seed_main(&[(".pi/SYSTEM.md", "PERSONA"), ("AGENTS.md", "CONVENTIONS")])
            .unwrap();
        // committed on main…
        assert_eq!(
            read(inst.repo(), ".pi/SYSTEM.md").as_deref(),
            Some("PERSONA")
        );
        // …and a worktree branched off main inherits the agent's definition.
        let e = inst.create_engagement("e").unwrap();
        assert_eq!(read(e.path(), ".pi/SYSTEM.md").as_deref(), Some("PERSONA"));
        assert_eq!(read(e.path(), "AGENTS.md").as_deref(), Some("CONVENTIONS"));
    }

    #[test]
    fn tree_read_write_round_trip_confined_to_worktree() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e1").unwrap();
        std::fs::create_dir_all(eng.path().join("src")).unwrap();
        write(eng.path(), "README.md", "# hi");
        write(&eng.path().join("src"), "main.rs", "fn main() {}");

        let tree = eng.tree().unwrap();
        let paths: Vec<&str> = tree.iter().map(|e| e.path.as_str()).collect();
        assert!(
            paths.contains(&"README.md")
                && paths.contains(&"src")
                && paths.contains(&"src/main.rs")
        );
        assert!(tree.iter().find(|e| e.path == "src").unwrap().is_dir);

        assert_eq!(eng.read_file("README.md").unwrap(), "# hi");
        eng.write_file("notes/todo.txt", "buy milk").unwrap(); // creates parent
        assert_eq!(eng.read_file("notes/todo.txt").unwrap(), "buy milk");

        // path escapes are rejected
        assert!(eng.read_file("../../etc/passwd").is_err());
        assert!(eng.write_file("/etc/x", "x").is_err());
    }

    #[test]
    fn read_file_capped_bounds_bytes_skips_binary_and_stays_confined() {
        let (_d, inst) = instance();
        let eng = inst.create_engagement("e2").unwrap();

        // A short text file reads back in full when the cap is generous.
        write(eng.path(), "note.txt", "hello world");
        assert_eq!(
            eng.read_file_capped("note.txt", 1024).unwrap().as_deref(),
            Some("hello world")
        );

        // The byte cap truncates: only the first `max_bytes` are returned.
        assert_eq!(
            eng.read_file_capped("note.txt", 5).unwrap().as_deref(),
            Some("hello")
        );

        // A NUL byte in the scanned prefix marks the file binary → None (skipped).
        eng.write_file("blob.bin", "abc\u{0}def").unwrap();
        assert_eq!(eng.read_file_capped("blob.bin", 1024).unwrap(), None);

        // Confinement is the same as `read_file`: an escaping path is rejected.
        assert!(eng.read_file_capped("../../etc/passwd", 1024).is_err());
    }

    #[test]
    fn conflicting_engagement_probes_and_merges_as_conflict_without_touching_main() {
        let (_d, inst) = instance();
        // both engagements edit the SAME file differently → a merge conflict.
        let a = inst.create_engagement("a").unwrap();
        let b = inst.create_engagement("b").unwrap();
        write(a.path(), "shared.txt", "from A");
        write(b.path(), "shared.txt", "from B");
        a.commit_turn("a").unwrap();
        b.commit_turn("b").unwrap();

        // a merges clean (main had no shared.txt)
        assert_eq!(a.merge_probe().unwrap(), MergeOutcome::Clean);
        assert_eq!(a.merge_into_main().unwrap(), MergeOutcome::Clean);

        // b now conflicts with main's shared.txt — probe says so, and the real
        // merge reports conflict and leaves main untouched (aborted).
        assert_eq!(b.merge_probe().unwrap(), MergeOutcome::Conflict);
        assert_eq!(b.merge_into_main().unwrap(), MergeOutcome::Conflict);
        assert_eq!(
            read(inst.repo(), "shared.txt").as_deref(),
            Some("from A"),
            "main untouched"
        );
    }
}
